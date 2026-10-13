//! A data model as its `contents` XML describes it: the entities, their
//! attributes and relationships, and what the model says around them.

use sweetpad_lib::xcscheme::Element;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Language {
    Swift,
    ObjectiveC,
}

#[derive(Debug)]
pub(super) struct Model {
    pub language: Language,
    /// `userDefinedModelVersionIdentifier`, empty when it has none.
    pub version_identifier: String,
    pub entities: Vec<Entity>,
    /// Each configuration's name and its entities' names.
    pub configurations: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Codegen {
    /// Written by hand.
    Manual,
    /// The class and its properties.
    Class,
    /// Only the properties, in an extension of a class written by hand.
    Category,
}

#[derive(Debug)]
pub(super) struct Entity {
    pub name: String,
    pub class_name: Option<String>,
    pub parent: Option<String>,
    pub is_abstract: bool,
    pub version_hash_modifier: Option<String>,
    pub renaming_identifier: Option<String>,
    pub codegen: Codegen,
    pub user_info: Vec<(String, String)>,
    pub attributes: Vec<Attribute>,
    pub relationships: Vec<Relationship>,
    pub fetch_indexes: Vec<FetchIndex>,
    /// Each uniqueness constraint's property names.
    pub uniqueness: Vec<Vec<String>>,
}

/// What attributes and relationships share.
#[derive(Debug)]
pub(super) struct Property {
    pub name: String,
    pub optional: bool,
    pub transient: bool,
    pub version_hash_modifier: Option<String>,
    pub renaming_identifier: Option<String>,
    pub user_info: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AttributeType {
    Integer16,
    Integer32,
    Integer64,
    Decimal,
    Double,
    Float,
    String,
    Boolean,
    Date,
    Binary,
    Uuid,
    Uri,
    Transformable,
    ObjectId,
}

impl AttributeType {
    fn parse(name: &str) -> Option<AttributeType> {
        Some(match name {
            "Integer 16" => AttributeType::Integer16,
            "Integer 32" => AttributeType::Integer32,
            "Integer 64" => AttributeType::Integer64,
            "Decimal" => AttributeType::Decimal,
            "Double" => AttributeType::Double,
            "Float" => AttributeType::Float,
            "String" => AttributeType::String,
            "Boolean" => AttributeType::Boolean,
            "Date" => AttributeType::Date,
            "Binary" => AttributeType::Binary,
            "UUID" => AttributeType::Uuid,
            "URI" => AttributeType::Uri,
            "Transformable" => AttributeType::Transformable,
            "ObjectID" => AttributeType::ObjectId,
            _ => return None,
        })
    }

    /// The number `NSAttributeType` gives the type.
    pub fn code(self) -> u32 {
        match self {
            AttributeType::Integer16 => 100,
            AttributeType::Integer32 => 200,
            AttributeType::Integer64 => 300,
            AttributeType::Decimal => 400,
            AttributeType::Double => 500,
            AttributeType::Float => 600,
            AttributeType::String => 700,
            AttributeType::Boolean => 800,
            AttributeType::Date => 900,
            AttributeType::Binary => 1000,
            AttributeType::Uuid => 1100,
            AttributeType::Uri => 1200,
            AttributeType::Transformable => 1800,
            AttributeType::ObjectId => 2000,
        }
    }

    pub fn is_integer(self) -> bool {
        matches!(
            self,
            AttributeType::Integer16 | AttributeType::Integer32 | AttributeType::Integer64
        )
    }
}

// One flag for each of the model's YES/NO settings.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug)]
pub(super) struct Attribute {
    pub property: Property,
    /// `None` for a type the model doesn't name, which momc rejects.
    pub kind: Option<AttributeType>,
    pub default: Option<String>,
    /// A date's default, in seconds since 2001.
    pub default_date: Option<String>,
    pub scalar: bool,
    pub external_storage: bool,
    pub preserve_after_deletion: bool,
    pub spotlight: bool,
    pub stored_in_truth_file: bool,
    pub cloud_encryption: bool,
    pub transformer: Option<String>,
    pub custom_class: Option<String>,
    pub min: Option<String>,
    pub max: Option<String>,
    pub regex: Option<String>,
    pub indexed: bool,
}

#[derive(Debug)]
pub(super) struct Relationship {
    pub property: Property,
    pub destination: Option<String>,
    pub inverse: Option<String>,
    pub to_many: bool,
    pub ordered: bool,
    pub min_count: u32,
    pub max_count: u32,
    /// No Action 0, Nullify 1, Cascade 2, Deny 3.
    pub delete_rule: u32,
}

#[derive(Debug)]
pub(super) struct FetchIndex {
    pub name: String,
    /// Each element's property, type (Binary 0, R-Tree 1) and whether it
    /// sorts ascending.
    pub elements: Vec<(String, u32, bool)>,
}

impl Model {
    pub fn entity(&self, name: &str) -> Option<&Entity> {
        self.entities.iter().find(|e| e.name == name)
    }

    /// The entity's ancestors, nearest first.
    pub fn ancestors<'a>(&'a self, entity: &'a Entity) -> impl Iterator<Item = &'a Entity> {
        std::iter::successors(Some(entity), |e| {
            e.parent.as_deref().and_then(|p| self.entity(p))
        })
        .skip(1)
        .take(self.entities.len())
    }
}

fn yes(element: &Element, key: &str) -> bool {
    element.attr(key) == Some("YES")
}

fn text(element: &Element, key: &str) -> Option<String> {
    element.attr(key).map(str::to_string)
}

fn user_info(element: &Element) -> Vec<(String, String)> {
    element
        .children_named("userInfo")
        .flat_map(|info| info.children_named("entry"))
        .filter_map(|entry| {
            Some((
                text(entry, "key")?,
                text(entry, "value").unwrap_or_default(),
            ))
        })
        .collect()
}

/// The model in `root`, the `contents` file's `model` element, or what in
/// it this stand-in doesn't compile.
pub(super) fn parse(root: &Element) -> Result<Model, String> {
    if root.name != "model" {
        return Err("the contents file isn't a data model".to_string());
    }
    for (name, what) in [
        ("fetchRequest", "fetch request templates"),
        ("fetchedProperty", "fetched properties"),
    ] {
        if !root.descendants_named(name).is_empty() {
            return Err(format!(
                "the model has {what}, which SweetPad's momc doesn't compile yet"
            ));
        }
    }
    let entities = root
        .children_named("entity")
        .map(entity)
        .collect::<Result<_, _>>()?;
    let configurations = root
        .children_named("configuration")
        .map(|c| {
            let name = text(c, "name").unwrap_or_default();
            let members = c
                .children_named("memberEntity")
                .filter_map(|m| text(m, "name"))
                .collect();
            (name, members)
        })
        .collect();
    Ok(Model {
        language: match root.attr("sourceLanguage") {
            Some("Objective-C") => Language::ObjectiveC,
            _ => Language::Swift,
        },
        version_identifier: text(root, "userDefinedModelVersionIdentifier").unwrap_or_default(),
        entities,
        configurations,
    })
}

fn entity(element: &Element) -> Result<Entity, String> {
    let name = text(element, "name").unwrap_or_default();
    if element.child("compoundIndexes").is_some() {
        return Err(format!(
            "{name} has compound indexes, which SweetPad's momc doesn't compile yet; use fetch indexes instead"
        ));
    }
    let attributes: Vec<Attribute> = element
        .children_named("attribute")
        .map(|a| attribute(&name, a))
        .collect::<Result<_, _>>()?;
    let mut fetch_indexes: Vec<FetchIndex> = element
        .children_named("fetchIndex")
        .map(|index| fetch_index(&name, index))
        .collect::<Result<_, _>>()?;
    // An attribute marked indexed gets a fetch index of its own.
    fetch_indexes.extend(attributes.iter().filter(|a| a.indexed).map(|a| FetchIndex {
        name: format!("by{}Index", capitalized(&a.property.name)),
        elements: vec![(a.property.name.clone(), 0, true)],
    }));
    Ok(Entity {
        class_name: text(element, "representedClassName"),
        parent: text(element, "parentEntity"),
        is_abstract: yes(element, "isAbstract"),
        version_hash_modifier: text(element, "versionHashModifier"),
        renaming_identifier: text(element, "elementID"),
        codegen: match element.attr("codeGenerationType") {
            Some("class") => Codegen::Class,
            Some("category") => Codegen::Category,
            _ => Codegen::Manual,
        },
        user_info: user_info(element),
        relationships: element
            .children_named("relationship")
            .map(relationship)
            .collect(),
        attributes,
        fetch_indexes,
        uniqueness: element
            .children_named("uniquenessConstraints")
            .flat_map(|u| u.children_named("uniquenessConstraint"))
            .map(|u| {
                u.children_named("constraint")
                    .filter_map(|c| text(c, "value"))
                    .collect()
            })
            .collect(),
        name,
    })
}

fn property(element: &Element) -> Property {
    Property {
        name: text(element, "name").unwrap_or_default(),
        optional: yes(element, "optional"),
        transient: yes(element, "transient"),
        version_hash_modifier: text(element, "versionHashModifier"),
        renaming_identifier: text(element, "elementID"),
        user_info: user_info(element),
    }
}

fn attribute(entity: &str, element: &Element) -> Result<Attribute, String> {
    let property = property(element);
    let unsupported = |what: &str| {
        Err(format!(
            "{entity}.{} is {what}, which SweetPad's momc doesn't compile yet",
            property.name
        ))
    };
    let type_name = element.attr("attributeType").unwrap_or_default();
    if type_name == "Composite" {
        return unsupported("a composite attribute");
    }
    if yes(element, "derived") {
        return unsupported("a derived attribute");
    }
    let kind = AttributeType::parse(type_name);
    let bounded = element.attr("minValueString").is_some()
        || element.attr("maxValueString").is_some()
        || element.attr("minDateTimeInterval").is_some()
        || element.attr("maxDateTimeInterval").is_some();
    let checked = matches!(
        kind,
        Some(
            AttributeType::Integer16
                | AttributeType::Integer32
                | AttributeType::Integer64
                | AttributeType::Double
                | AttributeType::Float
                | AttributeType::String
        )
    );
    if bounded && !checked {
        return unsupported(&format!(
            "a {type_name} attribute with a minimum or maximum"
        ));
    }
    Ok(Attribute {
        kind,
        default: text(element, "defaultValueString"),
        default_date: text(element, "defaultDateTimeInterval"),
        scalar: element.attr("usesScalarValueType") != Some("NO"),
        external_storage: yes(element, "allowsExternalBinaryDataStorage"),
        preserve_after_deletion: yes(element, "preserveAfterDeletion"),
        spotlight: yes(element, "spotlightIndexingEnabled"),
        stored_in_truth_file: yes(element, "storedInTruthFile"),
        cloud_encryption: yes(element, "allowsCloudEncryption"),
        transformer: text(element, "valueTransformerName"),
        custom_class: text(element, "customClassName"),
        min: text(element, "minValueString"),
        max: text(element, "maxValueString"),
        regex: text(element, "regularExpressionString"),
        indexed: yes(element, "indexed"),
        property,
    })
}

fn relationship(element: &Element) -> Relationship {
    let to_many = yes(element, "toMany");
    let count = |key: &str| element.attr(key).and_then(|n| n.parse().ok());
    Relationship {
        property: property(element),
        destination: text(element, "destinationEntity"),
        inverse: text(element, "inverseName"),
        to_many,
        ordered: yes(element, "ordered"),
        min_count: count("minCount").unwrap_or(0),
        max_count: count("maxCount").unwrap_or(u32::from(!to_many)),
        delete_rule: match element.attr("deletionRule") {
            Some("Nullify") => 1,
            Some("Cascade") => 2,
            Some("Deny") => 3,
            _ => 0,
        },
    }
}

fn fetch_index(entity: &str, element: &Element) -> Result<FetchIndex, String> {
    let name = text(element, "name").unwrap_or_default();
    if element.attr("partialIndexPredicate").is_some() {
        return Err(format!(
            "{entity}'s fetch index {name} has a predicate, which SweetPad's momc doesn't compile yet"
        ));
    }
    let elements = element
        .children_named("fetchIndexElement")
        .map(|e| match text(e, "property") {
            Some(property) => Ok((
                property,
                u32::from(e.attr("type") == Some("R-Tree")),
                e.attr("order") != Some("descending"),
            )),
            None => Err(format!(
                "{entity}'s fetch index {name} indexes an expression, which SweetPad's momc doesn't compile yet"
            )),
        })
        .collect::<Result<_, _>>()?;
    Ok(FetchIndex { name, elements })
}

/// `name` with its first letter upper case, as accessor names spell it.
pub(super) fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
