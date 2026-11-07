//! Core Data mapping models for portable builds. The output is the same
//! NSMappingModel keyed archive used by Core Data's migration manager.
#![allow(clippy::print_stderr)]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use base64::Engine as _;
use sweetpad_lib::pbxproj::Value;
use sweetpad_lib::xcscheme::Element;

use super::keyed_archive::{ARRAY, Archiver, MUTABLE_ARRAY, NULL, Plist};

#[must_use]
pub fn main(args: &[String]) -> ExitCode {
    let result = run(args);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: mapc: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let mut paths = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mapc-no-warnings" => {}
            "--sdkroot" | "--module" => {
                args.next()
                    .ok_or_else(|| format!("'{arg}' takes a value"))?;
            }
            flag if flag.starts_with("--") && flag.ends_with("-deployment-target") => {
                args.next()
                    .ok_or_else(|| format!("'{arg}' takes a value"))?;
            }
            flag if flag.starts_with('-') => return Err(format!("unsupported option '{flag}'")),
            path => paths.push(Path::new(path)),
        }
    }
    let [source, output] = paths.as_slice() else {
        return Err("expected a .xcmappingmodel and an output .cdm path".into());
    };
    let root = sweetpad_lib::xcscheme::parse_file(&source.join("xcmapping.xml"))
        .map_err(|e| e.to_string())?;
    let compiled = compile(&root)?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(output, compiled).map_err(|e| e.to_string())
}

fn attribute<'a>(object: &'a Element, name: &str) -> Option<&'a str> {
    object
        .children_named("attribute")
        .find(|a| a.attr("name") == Some(name))
        .map(|a| a.text.as_str())
}

fn relation<'a>(object: &'a Element, name: &str) -> Vec<&'a str> {
    object
        .children_named("relationship")
        .find(|a| a.attr("name") == Some(name))
        .and_then(|a| a.attr("idrefs"))
        .unwrap_or_default()
        .split_whitespace()
        .collect()
}

fn compile(root: &Element) -> Result<Vec<u8>, String> {
    let objects: BTreeMap<&str, &Element> = root
        .children_named("object")
        .filter_map(|o| Some((o.attr("id")?, o)))
        .collect();
    let model = objects
        .values()
        .find(|o| o.attr("type") == Some("XDDEVMAPPINGMODEL"))
        .ok_or("no mapping model in xcmapping.xml")?;
    let snapshots = |name| {
        attribute(model, name)
            .map(parse_snapshot)
            .transpose()
            .map(Option::unwrap_or_default)
    };
    let source = snapshots("sourcemodeldata")?;
    let destination = snapshots("destinationmodeldata")?;
    let mut a = Archiver::new();
    let mut entities = Vec::new();
    let mut by_name = Vec::new();
    for id in relation(model, "entitymappings") {
        let mapping = objects
            .get(id)
            .ok_or_else(|| format!("missing entity mapping '{id}'"))?;
        for name in ["userinfo", "userInfo"] {
            if !relation(mapping, name).is_empty() {
                return Err("mapping userInfo is not supported yet".into());
            }
        }
        let src = attribute(mapping, "sourcename");
        let dst = attribute(mapping, "destinationname");
        let policy = attribute(mapping, "migrationpolicyclassname")
            .or_else(|| attribute(mapping, "entitymigrationpolicyclassname"))
            .filter(|s| !s.is_empty());
        let kind = mapping_kind(mapping, &source, &destination)?;
        let name = attribute(mapping, "name").map_or_else(
            || format!("{}To{}", src.unwrap_or_default(), dst.unwrap_or_default()),
            str::to_string,
        );
        let name_ref = a.string(&name);
        let source_name = a.optional_string(src);
        let destination_name = a.optional_string(dst);
        let source_hash = model_hash(&mut a, src, &source.hashes)?;
        let destination_hash = model_hash(&mut a, dst, &destination.hashes)?;
        if let Some(expression) = attribute(mapping, "sourceexpression").filter(|e| !e.is_empty()) {
            return Err(format!(
                "custom source expression is not supported yet: {expression}"
            ));
        }
        let expression = src.map_or(NULL, |name| source_fetch(&mut a, name));
        let attributes = property_mappings(
            &mut a,
            &objects,
            mapping,
            "attributemappings",
            &source,
            &destination,
        )?;
        let relationships = property_mappings(
            &mut a,
            &objects,
            mapping,
            "relationshipmappings",
            &source,
            &destination,
        )?;
        let policy = a.optional_string(policy);
        let entity = a.object(
            None,
            &["NSEntityMapping", "NSObject"],
            vec![
                ("NSDestinationEntityName", destination_name),
                ("NSSourceEntityName", source_name),
                ("NSMappingName", name_ref.clone()),
                ("NSDestinationEntityVersionHash", destination_hash),
                ("NSUserInfo", NULL),
                ("NSSourceEntityVersionHash", source_hash),
                ("NSSourceExpression", expression),
                ("NSRelationshipMappings", relationships),
                ("NSMappingType", Plist::Int(kind)),
                ("NSEntityMigrationPolicyClassName", policy),
                ("NSAttributeMappings", attributes),
            ],
        );
        entities.push(entity.clone());
        by_name.push((name_ref, entity));
    }
    let entities = a.collection(ARRAY, entities);
    let by_name = a.dictionary(true, by_name);
    let root = a.object(
        None,
        &["NSMappingModel", "NSObject"],
        vec![
            ("NSEntityMappingsByName", by_name),
            ("NSEntityMappings", entities),
        ],
    );
    Ok(a.finish(root))
}

fn model_hash(
    a: &mut Archiver,
    name: Option<&str>,
    hashes: &BTreeMap<String, Vec<u8>>,
) -> Result<Plist, String> {
    name.map(|n| {
        hashes
            .get(n)
            .map(|h| a.value(Plist::Data(h.clone())))
            .ok_or_else(|| format!("model snapshot has no entity '{n}'"))
    })
    .transpose()
    .map(|v| v.unwrap_or(NULL))
}

fn mapping_kind(
    mapping: &Element,
    source: &ModelSnapshot,
    destination: &ModelSnapshot,
) -> Result<i64, String> {
    let src = attribute(mapping, "sourcename");
    let dst = attribute(mapping, "destinationname");
    let policy = attribute(mapping, "migrationpolicyclassname")
        .or_else(|| attribute(mapping, "entitymigrationpolicyclassname"))
        .filter(|s| !s.is_empty());
    let kind = match attribute(mapping, "mappingtypename").unwrap_or("Undefined") {
        "Undefined" if policy.is_some() => 1,
        "Undefined"
            if src.zip(dst).is_some_and(|(src, dst)| {
                source
                    .attribute_shapes
                    .get(src)
                    .is_some_and(|shape| destination.attribute_shapes.get(dst) == Some(shape))
            }) =>
        {
            4
        }
        "Undefined" | "Transform" => 5,
        other => return Err(format!("unsupported entity mapping kind '{other}'")),
    };
    Ok(kind)
}

fn property_mappings(
    a: &mut Archiver,
    objects: &BTreeMap<&str, &Element>,
    mapping: &Element,
    relationship: &str,
    source: &ModelSnapshot,
    destination: &ModelSnapshot,
) -> Result<Plist, String> {
    let mut out = Vec::new();
    let mut properties = relation(mapping, relationship)
        .into_iter()
        .map(|id| {
            objects
                .get(id)
                .copied()
                .ok_or_else(|| format!("missing property mapping '{id}'"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    properties.sort_by_key(|p| attribute(p, "name").unwrap_or_default());
    for p in properties {
        let name = attribute(p, "name").ok_or("property mapping has no name")?;
        let explicit = attribute(p, "valueexpression").filter(|e| !e.is_empty());
        let expression = explicit
            .filter(|e| !e.is_empty())
            .map_or_else(|| format!("$source.{name}"), str::to_string);
        if attribute(p, "valueexpression").is_none()
            && attribute(mapping, "autogenerateexpression") != Some("1")
        {
            return Err(format!(
                "property '{name}' has no automatic or explicit mapping expression"
            ));
        }
        let Some(path) = expression.strip_prefix("$source.") else {
            return Err(format!("unsupported mapping expression '{expression}'"));
        };
        let property = attribute(mapping, "sourcename")
            .and_then(|entity| source.properties.get(entity))
            .and_then(|p| p.get(name));
        let value = if explicit.is_none() && property.is_none() {
            NULL
        } else if explicit.is_none() && relationship == "relationshipmappings" {
            let source_entity = property
                .and_then(Option::as_deref)
                .ok_or("source relationship has no destination")?;
            let destination_entity = attribute(mapping, "destinationname")
                .and_then(|entity| destination.properties.get(entity))
                .and_then(|p| p.get(name))
                .and_then(Option::as_deref)
                .ok_or("destination relationship has no destination")?;
            let candidates: Vec<_> = objects
                .values()
                .filter(|m| {
                    m.attr("type") == Some("XDDEVENTITYMAPPING")
                        && attribute(m, "sourcename") == Some(source_entity)
                        && attribute(m, "destinationname") == Some(destination_entity)
                })
                .collect();
            if candidates.len() != 1 {
                return Err(format!(
                    "relationship '{name}' needs one mapping from {source_entity} to {destination_entity}"
                ));
            }
            let target = attribute(candidates[0], "name").map_or_else(
                || format!("{source_entity}To{destination_entity}"),
                str::to_string,
            );
            relationship_expression(a, &target, path)
        } else {
            key_path(a, "source", path)
        };
        let name = a.string(name);
        out.push(a.object(
            None,
            &["NSPropertyMapping", "NSObject"],
            vec![
                ("NSValueExpression", value),
                ("NSUserInfo", NULL),
                ("NSPropertyTransforms", NULL),
                ("NSDestinationPropertyName", name),
            ],
        ));
    }
    Ok(a.collection(ARRAY, out))
}

fn relationship_expression(a: &mut Archiver, mapping: &str, path: &str) -> Plist {
    let manager = variable(a, "manager");
    let mapping = constant(a, mapping);
    let source = key_path(a, "source", path);
    let arguments = a.collection(MUTABLE_ARRAY, vec![mapping, source]);
    let selector = a.string("destinationInstancesForEntityMappingNamed:sourceInstances:");
    a.object(
        None,
        &["NSFunctionExpression", "NSExpression", "NSObject"],
        vec![
            ("NSOperand", manager),
            ("NSSelectorName", selector),
            ("NSExpressionType", Plist::Int(4)),
            ("NSArguments", arguments),
        ],
    )
}

fn variable(a: &mut Archiver, name: &str) -> Plist {
    let name = a.string(name);
    a.object(
        None,
        &["NSVariableExpression", "NSExpression", "NSObject"],
        vec![("NSVariable", name), ("NSExpressionType", Plist::Int(2))],
    )
}

fn constant(a: &mut Archiver, value: &str) -> Plist {
    let value = a.string(value);
    a.object(
        None,
        &["NSConstantValueExpression", "NSExpression", "NSObject"],
        vec![
            ("NSConstantValue", value),
            ("NSExpressionType", Plist::Int(0)),
        ],
    )
}

fn key_path(a: &mut Archiver, owner: &str, path: &str) -> Plist {
    let owner = variable(a, owner);
    let path = a.string(path);
    let path = a.object(
        None,
        &["NSKeyPathSpecifierExpression", "NSExpression", "NSObject"],
        vec![("NSExpressionType", Plist::Int(10)), ("NSKeyPath", path)],
    );
    let arguments = a.collection(MUTABLE_ARRAY, vec![path]);
    let selector = a.string("valueForKeyPath:");
    a.object(
        None,
        &[
            "NSKeyPathExpression",
            "NSFunctionExpression",
            "NSExpression",
            "NSObject",
        ],
        vec![
            ("NSOperand", owner),
            ("NSSelectorName", selector),
            ("NSExpressionType", Plist::Int(4)),
            ("NSArguments", arguments),
        ],
    )
}

fn source_fetch(a: &mut Archiver, name: &str) -> Plist {
    let owner = variable(a, "manager");
    let entity = constant(a, name);
    let predicate = constant(a, "TRUEPREDICATE");
    let arguments = a.collection(ARRAY, vec![entity, predicate]);
    let selector = a.string("fetchRequestForSourceEntityNamed:predicateString:");
    let request = a.object(
        None,
        &["NSFunctionExpression", "NSExpression", "NSObject"],
        vec![
            ("NSOperand", owner),
            ("NSSelectorName", selector),
            ("NSExpressionType", Plist::Int(4)),
            ("NSArguments", arguments),
        ],
    );
    let context = key_path(a, "manager", "sourceContext");
    a.object(
        None,
        &["NSFetchRequestExpression", "NSExpression", "NSObject"],
        vec![
            ("NSMOCExpression", context),
            ("NSCountOnlyFlag", Plist::Bool(false)),
            ("NSExpressionType", Plist::Int(50)),
            ("NSFRExpression", request),
        ],
    )
}

struct Snapshot {
    objects: Vec<Value>,
}

impl Snapshot {
    fn field<'a>(object: &'a Value, name: &str) -> Option<&'a Value> {
        object.as_dict()?.get(name)
    }
    fn reference<'a>(&'a self, value: &Value) -> Option<&'a Value> {
        self.objects.get(value.as_str()?.parse::<usize>().ok()?)
    }
    fn text<'a>(&'a self, object: &Value, name: &str) -> Option<&'a str> {
        self.reference(Snapshot::field(object, name)?)?
            .as_str()
            .filter(|s| *s != "$null")
    }
    fn dictionary<'a>(&'a self, value: &Value) -> Vec<(&'a str, &'a Value)> {
        let Some(object) = self.reference(value) else {
            return Vec::new();
        };
        let keys = Snapshot::field(object, "NS.keys")
            .and_then(Value::as_array)
            .unwrap_or_default();
        let values = Snapshot::field(object, "NS.objects")
            .and_then(Value::as_array)
            .unwrap_or_default();
        keys.iter()
            .zip(values)
            .filter_map(|(k, v)| Some((self.reference(k)?.as_str()?, self.reference(v)?)))
            .collect()
    }
    fn defaults<'a>(&'a self, object: &Value) -> BTreeMap<&'a str, &'a Value> {
        let mut result = BTreeMap::new();
        if let Some(stereotypes) = Snapshot::field(object, "XDBucketForStereotypesstorage") {
            for (_, stereotype) in self.dictionary(stereotypes) {
                if let Some(properties) =
                    Snapshot::field(stereotype, "XDBucketForOwnedAttributesstorage")
                {
                    for (name, property) in self.dictionary(properties) {
                        if let Some(value) =
                            Snapshot::field(property, "_default").and_then(|v| self.reference(v))
                        {
                            result.insert(name, value);
                        }
                    }
                }
            }
        }
        result
    }

    fn properties(&self, object: &Value) -> BTreeMap<String, String> {
        self.defaults(object)
            .into_iter()
            .filter_map(|(name, value)| {
                Some((
                    name.to_string(),
                    value.as_str().filter(|s| *s != "$null")?.to_string(),
                ))
            })
            .collect()
    }

    fn collection_is_empty(value: &Value) -> bool {
        Snapshot::field(value, "NS.objects")
            .and_then(Value::as_array)
            .is_some_and(<[Value]>::is_empty)
    }
}

#[derive(Default)]
struct ModelSnapshot {
    hashes: BTreeMap<String, Vec<u8>>,
    properties: BTreeMap<String, BTreeMap<String, Option<String>>>,
    attribute_shapes: BTreeMap<String, Vec<Element>>,
}

fn parse_snapshot(encoded: &str) -> Result<ModelSnapshot, String> {
    let encoded: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| e.to_string())?;
    let archive = sweetpad_lib::bplist::parse(&bytes).map_err(|e| e.to_string())?;
    let objects = archive
        .as_dict()
        .and_then(|d| d.get("$objects"))
        .and_then(Value::as_array)
        .ok_or("model snapshot has no objects")?;
    let snapshot = Snapshot {
        objects: objects.to_vec(),
    };
    let mut model = Element {
        name: "model".into(),
        attributes: Vec::new(),
        children: Vec::new(),
        text: String::new(),
    };
    for object in &snapshot.objects {
        let class = Snapshot::field(object, "$class")
            .and_then(|v| snapshot.reference(v))
            .and_then(|c| Snapshot::field(c, "$classname"))
            .and_then(Value::as_str);
        if class != Some("XDPMEntity") {
            continue;
        }
        model.children.push(snapshot_entity(&snapshot, object)?);
    }
    let properties = model
        .children
        .iter()
        .map(|entity| {
            let properties = entity
                .children
                .iter()
                .filter_map(|property| {
                    Some((
                        property.attr("name")?.to_string(),
                        property.attr("destinationEntity").map(str::to_string),
                    ))
                })
                .collect();
            (
                entity.attr("name").unwrap_or_default().to_string(),
                properties,
            )
        })
        .collect();
    let attribute_shapes = model
        .children
        .iter()
        .map(|entity| {
            let mut attributes: Vec<_> = entity
                .children
                .iter()
                .filter(|p| p.name == "attribute")
                .cloned()
                .collect();
            attributes.sort_by(|a, b| a.attr("name").cmp(&b.attr("name")));
            (
                entity.attr("name").unwrap_or_default().to_string(),
                attributes,
            )
        })
        .collect();
    super::momc::entity_version_hashes(&model).map(|hashes| ModelSnapshot {
        hashes: hashes
            .into_iter()
            .map(|(name, hash)| (name, hash.to_vec()))
            .collect(),
        properties,
        attribute_shapes,
    })
}

fn snapshot_entity(snapshot: &Snapshot, object: &Value) -> Result<Element, String> {
    let name = snapshot
        .text(object, "_name")
        .ok_or("snapshot entity has no name")?;
    if let Some(parents) = Snapshot::field(object, "XDBucketForGeneralizationsstorage")
        .and_then(|v| snapshot.reference(v))
        && !Snapshot::collection_is_empty(parents)
    {
        return Err(format!(
            "mapping snapshots with entity inheritance are not supported yet: {name}"
        ));
    }
    if let Some(constraints) = snapshot.defaults(object).get("XDPMUniquenessConstraints")
        && !Snapshot::collection_is_empty(constraints)
    {
        return Err(format!(
            "mapping snapshots with uniqueness constraints are not supported yet: {name}"
        ));
    }
    let properties = snapshot.properties(object);
    let mut entity = Element {
        name: "entity".into(),
        attributes: vec![("name".into(), name.into())],
        children: Vec::new(),
        text: String::new(),
    };
    for (key, xml) in [
        ("XD_PSK_isAbstract", "isAbstract"),
        ("XD_PSK_versionHashModifier", "versionHashModifier"),
    ] {
        if let Some(value) = properties.get(key) {
            entity.attributes.push((xml.into(), value.clone()));
        }
    }
    if let Some(attributes) = Snapshot::field(object, "XDBucketForOwnedAttributesstorage") {
        for (name, value) in snapshot.dictionary(attributes) {
            if Snapshot::field(value, "$class")
                .and_then(|v| snapshot.reference(v))
                .and_then(|v| Snapshot::field(v, "$classname"))
                .and_then(Value::as_str)
                == Some("XDPMRelationship")
            {
                entity
                    .children
                    .push(snapshot_relationship(snapshot, name, value)?);
                continue;
            }
            let properties = snapshot.properties(value);
            let kind = properties.get("XD_PAttK_attributeType").ok_or_else(|| {
                format!("snapshot property '{name}' is not a supported attribute")
            })?;
            let kind = match kind.as_str() {
                "100" => "Integer 16",
                "200" => "Integer 32",
                "300" => "Integer 64",
                "400" => "Decimal",
                "500" => "Double",
                "600" => "Float",
                "700" => "String",
                "800" => "Boolean",
                "900" => "Date",
                "1000" => "Binary",
                "1100" => "UUID",
                "1200" => "URI",
                "1800" => "Transformable",
                "2000" => "ObjectID",
                _ => return Err(format!("unsupported snapshot attribute type '{kind}'")),
            };
            let mut attribute = Element {
                name: "attribute".into(),
                attributes: vec![
                    ("name".into(), name.into()),
                    ("attributeType".into(), kind.into()),
                ],
                children: Vec::new(),
                text: String::new(),
            };
            for (key, xml) in [
                ("XD_PPSK_isOptional", "optional"),
                ("XD_PPSK_isTransient", "transient"),
                ("XD_PPSK_versionHashModifier", "versionHashModifier"),
                (
                    "XD_PAttK_allowsExternalBinaryDataStorage",
                    "allowsExternalBinaryDataStorage",
                ),
                (
                    "XD_PAttK_preservesValueInHistoryOnDeletion",
                    "preserveAfterDeletion",
                ),
            ] {
                if let Some(value) = properties.get(key) {
                    attribute.attributes.push((xml.into(), value.clone()));
                }
            }
            entity.children.push(attribute);
        }
    }
    Ok(entity)
}

fn snapshot_relationship(
    snapshot: &Snapshot,
    name: &str,
    object: &Value,
) -> Result<Element, String> {
    let defaults = snapshot.defaults(object);
    let mut relationship = Element {
        name: "relationship".into(),
        attributes: vec![("name".into(), name.into())],
        children: Vec::new(),
        text: String::new(),
    };
    for (key, xml) in [
        ("XD_PPSK_isOptional", "optional"),
        ("XD_PPSK_isTransient", "transient"),
        ("XD_PPSK_versionHashModifier", "versionHashModifier"),
        ("XD_PRK_isToMany", "toMany"),
        ("XD_PRK_ordered", "ordered"),
        ("XD_PRK_minCount", "minCount"),
        ("XD_PRK_maxCount", "maxCount"),
    ] {
        if let Some(value) = defaults
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|s| *s != "$null")
        {
            relationship.attributes.push((xml.into(), value.into()));
        }
    }
    for (key, xml) in [
        ("XD_PRK_destination", "destinationEntity"),
        ("XD_PRK_inverseRelationship", "inverseName"),
    ] {
        if let Some(value) = defaults.get(key) {
            let name = snapshot
                .text(value, "_name")
                .ok_or_else(|| format!("relationship '{name}' has no {xml}"))?;
            relationship.attributes.push((xml.into(), name.into()));
        }
    }
    Ok(relationship)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_mapping_archive_matches_xcodes_capture() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/mapc");
        for name in ["Users", "Relationships"] {
            let root = sweetpad_lib::xcscheme::parse_file(
                &dir.join(format!("{name}.xcmappingmodel/xcmapping.xml")),
            )
            .unwrap();
            let got = compile(&root).unwrap();
            let want = std::fs::read(dir.join(format!("{name}.cdm"))).unwrap();
            let got = super::super::momc::tests::canonical(&got);
            let want = super::super::momc::tests::canonical(&want);
            let difference = got
                .lines()
                .zip(want.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b);
            assert!(
                got == want,
                "{name}: first difference {difference:?}; line counts {} vs {}",
                got.lines().count(),
                want.lines().count()
            );
        }
    }
    #[test]
    fn invalid_snapshot_data_is_an_error() {
        assert!(parse_snapshot("bad base64").is_err());
    }
}
