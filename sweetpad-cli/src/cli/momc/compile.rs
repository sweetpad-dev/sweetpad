//! A model compiled: the `.mom` file, an `NSManagedObjectModel` in a keyed
//! archive, with the objects and keys Core Data reads back, and the version
//! hashes `VersionInfo.plist` records for each entity.

use sweetpad_lib::xcode_hash::sha256;

use super::model::{Attribute, AttributeType, Entity, Model, Property, Relationship};
use crate::cli::keyed_archive::{Archiver, Class, MUTABLE_ARRAY, NULL, Plist, SET};

const MODEL: Class = &["NSManagedObjectModel", "NSObject"];
const ENTITY: Class = &["NSEntityDescription", "NSObject"];
const ATTRIBUTE: Class = &[
    "NSAttributeDescription",
    "NSPropertyDescription",
    "NSObject",
];
const RELATIONSHIP: Class = &[
    "NSRelationshipDescription",
    "NSPropertyDescription",
    "NSObject",
];
const FETCH_INDEX: Class = &["NSFetchIndexDescription", "NSObject"];
const FETCH_INDEX_ELEMENT: Class = &["NSFetchIndexElementDescription", "NSObject"];
const DATE: Class = &["NSDate", "NSObject"];
const DECIMAL: Class = &[
    "NSDecimalNumberPlaceholder",
    "NSDecimalNumber",
    "NSNumber",
    "NSValue",
    "NSObject",
];
const COMPARISON: Class = &["NSComparisonPredicate", "NSPredicate", "NSObject"];
const COMPARISON_OPERATOR: Class = &[
    "NSComparisonPredicateOperator",
    "NSPredicateOperator",
    "NSObject",
];
const MATCHING_OPERATOR: Class = &[
    "NSMatchingPredicateOperator",
    "NSStringPredicateOperator",
    "NSPredicateOperator",
    "NSObject",
];
const SELF_EXPRESSION: Class = &["NSSelfExpression", "NSExpression", "NSObject"];
const CONSTANT_EXPRESSION: Class = &["NSConstantValueExpression", "NSExpression", "NSObject"];
const KEY_PATH_EXPRESSION: Class = &[
    "NSKeyPathExpression",
    "NSFunctionExpression",
    "NSExpression",
    "NSObject",
];
const KEY_PATH_SPECIFIER: Class = &["NSKeyPathSpecifierExpression", "NSExpression", "NSObject"];

/// The version of momc the archive says compiled it, Xcode 27's.
const TOOL_VERSION: f64 = 25249.0;
const XCODE_VERSION: i64 = 270_000;

/// A problem with the model, at an entity or one of its properties.
#[derive(Debug, PartialEq)]
pub(super) struct Diagnostic {
    pub error: bool,
    /// `Entity` or `Entity.property`.
    pub location: String,
    pub message: String,
    /// The category momc prints in brackets after the message.
    pub category: u32,
}

/// Which warnings to leave out, as momc's options say.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Quiet {
    pub all: bool,
    pub inverse: bool,
    pub delete_rule: bool,
}

/// What momc reports about `model` before compiling it, in the order of
/// the properties it's about. Errors stop it.
pub(super) fn check(model: &Model, quiet: Quiet) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut report = |error: bool, location: &str, message: String, category: u32| {
        if error || !quiet.all {
            out.push(Diagnostic {
                error,
                location: location.to_string(),
                message,
                category,
            });
        }
    };
    for entity in &model.entities {
        if let Some(parent) = &entity.parent
            && model.entity(parent).is_none()
        {
            report(
                true,
                &entity.name,
                format!(
                    "{} has a parent entity, {parent}, that isn't in the model",
                    entity.name
                ),
                2,
            );
        }
        for attribute in &entity.attributes {
            let at = format!("{}.{}", entity.name, attribute.property.name);
            match attribute.kind {
                None | Some(AttributeType::ObjectId) if !attribute.property.transient => {
                    report(true, &at, format!("{at} must have a defined type"), 2);
                }
                Some(AttributeType::Transformable) if attribute.transformer.is_none() => {
                    report(
                        false,
                        &at,
                        format!(
                            "{at} is using a nil or insecure value transformer. Please switch to \
                             NSSecureUnarchiveFromDataTransformerName or a custom NSValueTransformer \
                             subclass of NSSecureUnarchiveFromDataTransformer"
                        ),
                        2,
                    );
                }
                _ => {}
            }
            if attribute.stored_in_truth_file {
                report(
                    false,
                    &at,
                    format!(
                        "{at} is set to be stored using a Spotlight external record file, which is \
                         deprecated. Use Core Spotlight integration instead."
                    ),
                    5,
                );
            }
        }
        for relationship in &entity.relationships {
            let at = format!("{}.{}", entity.name, relationship.property.name);
            let destination = relationship
                .destination
                .as_deref()
                .and_then(|d| model.entity(d));
            if destination.is_none() {
                report(true, &at, format!("{at} must have a destination entity"), 2);
            }
            if relationship.delete_rule == 0 && !quiet.delete_rule {
                report(
                    false,
                    &at,
                    format!(
                        "Setting the No Action Delete Rule on {at} may result in relationships to deleted objects"
                    ),
                    7,
                );
            }
            if relationship.inverse.is_none() && !quiet.inverse {
                report(false, &at, format!("{at} should have an inverse"), 2);
            }
        }
    }
    out
}

/// The model as a `.mom` file.
#[allow(clippy::too_many_lines)]
pub(super) fn archive(model: &Model) -> Vec<u8> {
    let mut a = Archiver::new();
    let root = a.reserve();
    let entities: Vec<u64> = model.entities.iter().map(|_| a.reserve()).collect();
    let uid_of = |name: &str| {
        model
            .entities
            .iter()
            .position(|e| e.name == name)
            .map(|i| entities[i])
    };
    let properties: Vec<Vec<u64>> = model
        .entities
        .iter()
        .map(|e| {
            (0..e.attributes.len() + e.relationships.len())
                .map(|_| a.reserve())
                .collect()
        })
        .collect();
    let property_uid = |entity: &str, name: &str| {
        let i = model.entities.iter().position(|e| e.name == entity)?;
        let e = &model.entities[i];
        let at = e
            .attributes
            .iter()
            .map(|a| &a.property.name)
            .chain(e.relationships.iter().map(|r| &r.property.name))
            .position(|n| n == name)?;
        Some(properties[i][at])
    };
    let mut empty_user_info = None;
    let mut self_expression = None;

    for (i, entity) in model.entities.iter().enumerate() {
        let me = Plist::Uid(entities[i]);
        let mut fields = Vec::new();
        for (j, attribute) in entity.attributes.iter().enumerate() {
            let fields = attribute_fields(&mut a, attribute, &me, &mut self_expression);
            a.object(Some(properties[i][j]), ATTRIBUTE, fields);
        }
        for (j, relationship) in entity.relationships.iter().enumerate() {
            let destination = relationship.destination.as_deref().unwrap_or_default();
            let inverse = relationship
                .inverse
                .as_deref()
                .and_then(|inverse| property_uid(destination, inverse));
            let fields = relationship_fields(
                &mut a,
                relationship,
                &me,
                uid_of(destination).map_or(NULL, Plist::Uid),
                inverse.map_or(NULL, Plist::Uid),
            );
            a.object(
                Some(properties[i][entity.attributes.len() + j]),
                RELATIONSHIP,
                fields,
            );
        }
        let names = entity
            .attributes
            .iter()
            .map(|a| &a.property.name)
            .chain(entity.relationships.iter().map(|r| &r.property.name));
        let entries = names
            .zip(&properties[i])
            .map(|(name, &uid)| (a.string(name), Plist::Uid(uid)))
            .collect();
        let own = a.dictionary(true, entries);
        let subentities = model
            .entities
            .iter()
            .zip(&entities)
            .filter(|(e, _)| e.parent.as_deref() == Some(entity.name.as_str()))
            .map(|(e, &uid)| (a.string(&e.name), Plist::Uid(uid)))
            .collect();
        let subentities = a.dictionary(true, subentities);
        let user_info = if entity.user_info.is_empty() {
            empty_user_info
                .get_or_insert_with(|| a.dictionary(false, Vec::new()))
                .clone()
        } else {
            user_info_dictionary(&mut a, &entity.user_info)
        };
        fields.extend([
            (
                "NSClassNameForEntity",
                a.optional_string(entity.class_name.as_deref()),
            ),
            ("NSEntityName", a.string(&entity.name)),
            ("NSManagedObjectModel", Plist::Uid(root)),
            ("NSProperties", own),
            (
                "NSRenamingIdentifier",
                a.string(
                    entity
                        .renaming_identifier
                        .as_deref()
                        .unwrap_or(&entity.name),
                ),
            ),
            ("NSSubentities", subentities),
            (
                "NSSuperentity",
                entity
                    .parent
                    .as_deref()
                    .and_then(uid_of)
                    .map_or(NULL, Plist::Uid),
            ),
            ("NSUserInfo", user_info),
            (
                "NSVersionHashModifier",
                a.optional_string(entity.version_hash_modifier.as_deref()),
            ),
        ]);
        if entity.is_abstract {
            fields.push(("NSIsAbstract", Plist::Bool(true)));
        }
        if !entity.fetch_indexes.is_empty() {
            let indexes = entity
                .fetch_indexes
                .iter()
                .map(|index| {
                    let at = a.reserve();
                    let elements = index
                        .elements
                        .iter()
                        .map(|(property, kind, ascending)| {
                            let property = a.string(property);
                            a.object(
                                None,
                                FETCH_INDEX_ELEMENT,
                                vec![
                                    ("NSAscending", Plist::Bool(*ascending)),
                                    ("NSFetchIndexDescription", Plist::Uid(at)),
                                    ("NSFetchIndexElementType", Plist::Int(i64::from(*kind))),
                                    ("NSPropertyName", property),
                                ],
                            )
                        })
                        .collect();
                    let elements = a.collection(crate::cli::keyed_archive::ARRAY, elements);
                    let name = a.string(&index.name);
                    a.object(
                        Some(at),
                        FETCH_INDEX,
                        vec![
                            ("NSEntity", me.clone()),
                            ("NSIndexElements", elements),
                            ("NSIndexName", name),
                            ("NSPartialIndexPredicate", NULL),
                        ],
                    )
                })
                .collect();
            let indexes = a.collection(MUTABLE_ARRAY, indexes);
            // The indexes of more than one property, again in the form
            // compound indexes had before fetch indexes.
            let compound = entity
                .fetch_indexes
                .iter()
                .filter(|index| index.elements.len() > 1 && index.elements.iter().all(|e| e.1 == 0))
                .map(|index| {
                    let names = index.elements.iter().map(|e| a.string(&e.0)).collect();
                    a.collection(MUTABLE_ARRAY, names)
                })
                .collect();
            let compound = a.collection(MUTABLE_ARRAY, compound);
            fields.push(("NSFetchIndexDescriptions", indexes));
            fields.push(("NSCompoundIndexes", compound));
        }
        if !entity.uniqueness.is_empty() {
            let tuples = entity
                .uniqueness
                .iter()
                .map(|tuple| {
                    let names = tuple.iter().map(|n| a.string(n)).collect();
                    a.collection(MUTABLE_ARRAY, names)
                })
                .collect();
            let tuples = a.collection(MUTABLE_ARRAY, tuples);
            fields.push(("NSUniqueTuples", tuples));
        }
        a.object(Some(entities[i]), ENTITY, fields);
    }

    let entries = model
        .entities
        .iter()
        .zip(&entities)
        .map(|(e, &uid)| (a.string(&e.name), Plist::Uid(uid)))
        .collect();
    let all = a.dictionary(true, entries);
    let identifier = a.string(&model.version_identifier);
    let identifiers = a.collection(SET, vec![identifier]);
    let mut fields = vec![
        ("CDMCompilerToolVersion", Plist::Real(TOOL_VERSION)),
        ("CDMCompilerXcodeVersion", Plist::Int(XCODE_VERSION)),
        ("NSEntities", all),
        ("NSFetchRequestTemplates", NULL),
        ("NSVersionIdentifiers", identifiers),
    ];
    if !model.configurations.is_empty() {
        let configurations = model
            .configurations
            .iter()
            .map(|(name, members)| {
                let members = members
                    .iter()
                    .filter_map(|m| uid_of(m))
                    .map(Plist::Uid)
                    .collect();
                let set = a.collection(SET, members);
                (a.string(name), set)
            })
            .collect();
        fields.push(("NSConfigurations", a.dictionary(true, configurations)));
    }
    let root = a.object(Some(root), MODEL, fields);
    a.finish(root)
}

fn user_info_dictionary(a: &mut Archiver, entries: &[(String, String)]) -> Plist {
    let entries = entries
        .iter()
        .map(|(k, v)| (a.string(k), a.string(v)))
        .collect();
    a.dictionary(false, entries)
}

/// The bits of `_P`, which say which of a property's keys hold something
/// other than their default, so Core Data can skip reading the rest.
fn property_bits(property: &Property, validated: bool) -> i64 {
    let mut bits = 0xf1;
    if property.optional {
        bits |= 0x02;
    }
    if property.transient {
        bits |= 0x08;
    }
    if validated {
        bits &= !0x30;
    }
    if !property.user_info.is_empty() {
        bits &= !0x40;
    }
    if property.version_hash_modifier.is_some() {
        bits &= !0x80;
    }
    bits
}

/// The keys every property has, and those it has only when they're set.
fn property_fields(
    a: &mut Archiver,
    property: &Property,
    entity: &Plist,
) -> Vec<(&'static str, Plist)> {
    let mut fields = vec![
        ("NSEntity", entity.clone()),
        ("NSPropertyName", a.string(&property.name)),
        (
            "NSRenamingIdentifier",
            a.string(
                property
                    .renaming_identifier
                    .as_deref()
                    .unwrap_or(&property.name),
            ),
        ),
    ];
    if property.optional {
        fields.push(("NSIsOptional", Plist::Bool(true)));
    }
    if property.transient {
        fields.push(("NSIsTransient", Plist::Bool(true)));
    }
    if let Some(modifier) = &property.version_hash_modifier {
        fields.push(("NSVersionHashModifier", a.string(modifier)));
    }
    if !property.user_info.is_empty() {
        let info = user_info_dictionary(a, &property.user_info);
        fields.push(("NSUserInfo", info));
    }
    fields
}

fn attribute_fields(
    a: &mut Archiver,
    attribute: &Attribute,
    entity: &Plist,
    self_expression: &mut Option<Plist>,
) -> Vec<(&'static str, Plist)> {
    let kind = attribute.kind;
    let mut fields = property_fields(a, &attribute.property, entity);
    let class_name = match kind {
        Some(AttributeType::Transformable) => attribute.custom_class.as_deref(),
        Some(kind) => value_class(kind),
        None => None,
    };
    let default = default_value(a, attribute);
    let mut flags = 0;
    if attribute.external_storage {
        flags |= 4;
    }
    if attribute.preserve_after_deletion {
        flags |= 16;
    }
    let (predicates, warnings) = validation(a, attribute, self_expression);
    let mut bits = property_bits(&attribute.property, !predicates.is_empty());
    fields.extend([
        (
            "NSAttributeType",
            Plist::Int(i64::from(kind.map_or(0, AttributeType::code))),
        ),
        ("NSAttributeValueClassName", a.optional_string(class_name)),
        ("NSDefaultValue", default),
        ("NSFlagsKey", Plist::Int(flags)),
        (
            "NSValueTransformerName",
            a.optional_string(attribute.transformer.as_deref()),
        ),
    ]);
    if attribute.spotlight {
        bits |= 0x100;
        fields.push(("NSIndexedBySpotlight", Plist::Bool(true)));
    }
    if attribute.stored_in_truth_file {
        bits |= 0x200;
        fields.push(("NSIsStoredInTruth", Plist::Bool(true)));
    }
    if attribute.cloud_encryption {
        bits |= 0x800;
        fields.push(("NSAllowsCloudEncryption", Plist::Bool(true)));
    }
    if !predicates.is_empty() {
        let predicates = a.collection(crate::cli::keyed_archive::ARRAY, predicates);
        let warnings = warnings
            .into_iter()
            .map(|w| a.value(Plist::Int(w)))
            .collect();
        let warnings = a.collection(crate::cli::keyed_archive::ARRAY, warnings);
        fields.push(("NSValidationPredicates", predicates));
        fields.push(("NSValidationWarnings", warnings));
    }
    fields.push(("_P", Plist::Int(bits)));
    fields
}

fn relationship_fields(
    a: &mut Archiver,
    relationship: &Relationship,
    entity: &Plist,
    destination: Plist,
    inverse: Plist,
) -> Vec<(&'static str, Plist)> {
    let mut fields = property_fields(a, &relationship.property, entity);
    let mut bits = property_bits(&relationship.property, false);
    fields.extend([
        (
            "NSDeleteRule",
            Plist::Int(i64::from(relationship.delete_rule)),
        ),
        ("NSDestinationEntity", destination),
        ("NSInverseRelationship", inverse),
        ("NSMaxCount", Plist::Int(i64::from(relationship.max_count))),
        ("NSMinCount", Plist::Int(i64::from(relationship.min_count))),
        (
            "_NSDestinationEntityName",
            a.optional_string(relationship.destination.as_deref()),
        ),
        (
            "_NSInverseRelationshipName",
            a.optional_string(relationship.inverse.as_deref()),
        ),
    ]);
    if relationship.ordered {
        bits |= 0x400;
        fields.push(("NSIsOrdered", Plist::Bool(true)));
    }
    fields.push(("_P", Plist::Int(bits)));
    fields
}

/// The class of an attribute's values, by its type.
fn value_class(kind: AttributeType) -> Option<&'static str> {
    Some(match kind {
        AttributeType::Integer16
        | AttributeType::Integer32
        | AttributeType::Integer64
        | AttributeType::Double
        | AttributeType::Float
        | AttributeType::Boolean => "NSNumber",
        AttributeType::Decimal => "NSDecimalNumber",
        AttributeType::String => "NSString",
        AttributeType::Date => "NSDate",
        AttributeType::Binary => "NSData",
        AttributeType::Uuid => "NSUUID",
        AttributeType::Uri => "NSURL",
        AttributeType::Transformable | AttributeType::ObjectId => return None,
    })
}

/// An attribute's default as an object of its type. momc keeps none for a
/// UUID, URI, binary or transformable attribute, and gives a date whose
/// default isn't a whole number of seconds the reference date itself.
fn default_value(a: &mut Archiver, attribute: &Attribute) -> Plist {
    let Some(kind) = attribute.kind else {
        return NULL;
    };
    if kind == AttributeType::Date {
        let seconds = match (&attribute.default_date, &attribute.default) {
            (Some(seconds), _) => seconds.trim().parse::<i64>().unwrap_or(0),
            (None, Some(_)) => 0,
            (None, None) => return NULL,
        };
        #[allow(clippy::cast_precision_loss)]
        let time = Plist::Real(seconds as f64);
        return a.object(None, DATE, vec![("NS.time", time)]);
    }
    let Some(text) = attribute.default.as_deref() else {
        return NULL;
    };
    let value = match kind {
        kind if kind.is_integer() => Plist::Int(leading_integer(text)),
        AttributeType::Double => Plist::Real(text.trim().parse().unwrap_or(0.0)),
        AttributeType::Float => Plist::Real32(text.trim().parse().unwrap_or(0.0)),
        AttributeType::Boolean => Plist::Bool(bool_value(text)),
        AttributeType::String => return a.string(text),
        AttributeType::Decimal => return decimal(a, text),
        _ => return NULL,
    };
    a.value(value)
}

/// The integer `text` starts with, as `NSString.longLongValue` reads it.
fn leading_integer(text: &str) -> i64 {
    let text = text.trim_start();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let magnitude = digits
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0i64, |n, d| {
            n.saturating_mul(10).saturating_add(i64::from(d - b'0'))
        });
    if negative { -magnitude } else { magnitude }
}

/// Whether `text` is true, as `NSString.boolValue` reads it.
fn bool_value(text: &str) -> bool {
    let text = text.trim_start();
    let text = text.strip_prefix(['+', '-']).unwrap_or(text);
    let text = text.trim_start_matches('0');
    text.starts_with([
        'Y', 'y', 'T', 't', '1', '2', '3', '4', '5', '6', '7', '8', '9',
    ])
}

/// A decimal number, as `NSDecimalNumber` archives itself: its mantissa as
/// eight little-endian 16-bit words, of which `NS.length` are in use, and
/// a power of ten, with trailing zeros moved from the one to the other.
fn decimal(a: &mut Archiver, text: &str) -> Plist {
    let (negative, mut mantissa, mut exponent) = parse_decimal(text);
    let zero = mantissa == 0;
    if zero {
        exponent = 0;
    }
    while !zero && mantissa % 10 == 0 {
        mantissa /= 10;
        exponent += 1;
    }
    let bytes = mantissa.to_le_bytes().to_vec();
    let length = (128 - mantissa.leading_zeros()).div_ceil(16);
    a.object(
        None,
        DECIMAL,
        vec![
            ("NS.compact", Plist::Bool(!zero)),
            ("NS.exponent", Plist::Int(i64::from(exponent))),
            ("NS.length", Plist::Int(i64::from(length))),
            ("NS.mantissa", Plist::Data(bytes)),
            ("NS.mantissa.bo", Plist::Int(1)),
            ("NS.negative", Plist::Bool(negative && !zero)),
        ],
    )
}

/// The sign, digits and power of ten of a decimal number's text: digits,
/// a point and more digits, and an exponent. The 38 digits a mantissa
/// holds are kept; the rest go to the exponent.
fn parse_decimal(text: &str) -> (bool, u128, i32) {
    let text = text.trim();
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (number, power) = match text.find(['e', 'E']) {
        Some(at) => (&text[..at], text[at + 1..].parse::<i32>().unwrap_or(0)),
        None => (text, 0),
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let mut mantissa: u128 = 0;
    let mut exponent = power;
    let mut digits = 0;
    for (d, in_fraction) in whole
        .bytes()
        .map(|d| (d, false))
        .chain(fraction.bytes().map(|d| (d, true)))
    {
        if !d.is_ascii_digit() {
            break;
        }
        if digits < 38 {
            mantissa = mantissa * 10 + u128::from(d - b'0');
            if mantissa > 0 {
                digits += 1;
            }
            if in_fraction {
                exponent -= 1;
            }
        } else if !in_fraction {
            exponent += 1;
        }
    }
    (negative, mantissa, exponent)
}

/// The predicates that check an attribute's minimum, maximum and pattern,
/// and the error code each one fails with.
fn validation(
    a: &mut Archiver,
    attribute: &Attribute,
    self_expression: &mut Option<Plist>,
) -> (Vec<Plist>, Vec<i64>) {
    let Some(kind) = attribute.kind else {
        return (Vec::new(), Vec::new());
    };
    let mut predicates = Vec::new();
    let mut warnings = Vec::new();
    let string = kind == AttributeType::String;
    let bounds = [
        (&attribute.min, 3, if string { 1670 } else { 1620 }),
        (&attribute.max, 1, if string { 1660 } else { 1610 }),
    ];
    for (bound, operator, warning) in bounds {
        let Some(bound) = bound else { continue };
        let me = self_of(a, self_expression);
        let left = if string { length_of(a, me) } else { me };
        let constant = match kind {
            AttributeType::Double | AttributeType::Float => {
                Plist::Real(bound.trim().parse().unwrap_or(0.0))
            }
            _ => Plist::Int(leading_integer(bound)),
        };
        let constant = a.value(constant);
        let right = constant_expression(a, constant);
        let operator = a.object(
            None,
            COMPARISON_OPERATOR,
            vec![
                ("NSModifier", Plist::Int(0)),
                ("NSOperatorType", Plist::Int(operator)),
                ("NSOptions", Plist::Int(0)),
                ("NSVariant", Plist::Int(operator)),
            ],
        );
        predicates.push(comparison(a, left, operator, right));
        warnings.push(warning);
    }
    if let (Some(pattern), true) = (&attribute.regex, string) {
        let left = self_of(a, self_expression);
        let pattern = a.string(pattern);
        let right = constant_expression(a, pattern);
        let operator = a.object(
            None,
            MATCHING_OPERATOR,
            vec![
                ("NSFlags", Plist::Int(0)),
                ("NSModifier", Plist::Int(0)),
                ("NSOperatorType", Plist::Int(6)),
            ],
        );
        predicates.push(comparison(a, left, operator, right));
        warnings.push(1680);
    }
    (predicates, warnings)
}

fn self_of(a: &mut Archiver, cached: &mut Option<Plist>) -> Plist {
    cached
        .get_or_insert_with(|| {
            a.object(
                None,
                SELF_EXPRESSION,
                vec![("NSExpressionType", Plist::Int(1))],
            )
        })
        .clone()
}

/// `SELF.length`, as a key path expression.
fn length_of(a: &mut Archiver, me: Plist) -> Plist {
    let key = a.string("length");
    let specifier = a.object(
        None,
        KEY_PATH_SPECIFIER,
        vec![("NSExpressionType", Plist::Int(10)), ("NSKeyPath", key)],
    );
    let arguments = a.collection(MUTABLE_ARRAY, vec![specifier]);
    let selector = a.string("valueForKey:");
    a.object(
        None,
        KEY_PATH_EXPRESSION,
        vec![
            ("NSArguments", arguments),
            ("NSExpressionType", Plist::Int(3)),
            ("NSOperand", me),
            ("NSSelectorName", selector),
        ],
    )
}

fn constant_expression(a: &mut Archiver, value: Plist) -> Plist {
    a.object(
        None,
        CONSTANT_EXPRESSION,
        vec![
            ("NSConstantValue", value),
            ("NSExpressionType", Plist::Int(0)),
        ],
    )
}

fn comparison(a: &mut Archiver, left: Plist, operator: Plist, right: Plist) -> Plist {
    a.object(
        None,
        COMPARISON,
        vec![
            ("NSLeftExpression", left),
            ("NSPredicateOperator", operator),
            ("NSRightExpression", right),
        ],
    )
}

/// Each entity's version hash, by name, in name order. Core Data compares
/// them with a store's to tell whether the store fits the model.
pub(super) fn version_hashes(model: &Model) -> Hashes {
    let mut names: Vec<&str> = model.entities.iter().map(|e| e.name.as_str()).collect();
    names.sort_unstable();
    names
        .into_iter()
        .filter_map(|name| Some((name.to_string(), entity_hash(model, model.entity(name)?))))
        .collect()
}

/// The hash of every entity's version hash, in name order, base64.
pub(super) fn checksum(hashes: &[(String, [u8; 32])]) -> String {
    let all: Vec<u8> = hashes.iter().flat_map(|(_, h)| *h).collect();
    sweetpad_lib::xcode_hash::base64(&sha256(&all))
}

/// The hash of an entity's name, whether it's abstract, its parent's hash,
/// the hashes of the properties it stores, its own and inherited, in name
/// order, its version hash modifier, and its own uniqueness constraints:
/// each one's properties joined with dots, in sorted order.
fn entity_hash(model: &Model, entity: &Entity) -> [u8; 32] {
    let mut out = entity.name.as_bytes().to_vec();
    out.push(u8::from(entity.is_abstract));
    let lineage: Vec<&Entity> = std::iter::once(entity)
        .chain(model.ancestors(entity))
        .collect();
    if let Some(parent) = lineage.get(1) {
        out.extend(entity_hash(model, parent));
    }
    let mut hashes: Vec<(&str, [u8; 32])> = Vec::new();
    for e in lineage.iter().rev() {
        for attribute in e.attributes.iter().filter(|a| !a.property.transient) {
            hashes.push((&attribute.property.name, attribute_hash(attribute)));
        }
        for relationship in e.relationships.iter().filter(|r| !r.property.transient) {
            hashes.push((&relationship.property.name, relationship_hash(relationship)));
        }
    }
    hashes.sort_by(|a, b| a.0.cmp(b.0));
    for (_, hash) in hashes {
        out.extend(hash);
    }
    if let Some(modifier) = &entity.version_hash_modifier {
        out.extend(modifier.as_bytes());
    }
    let mut constraints: Vec<String> = entity.uniqueness.iter().map(|c| c.join(".")).collect();
    constraints.sort();
    out.extend(constraints.concat().as_bytes());
    sha256(&out)
}

fn property_hash(property: &Property) -> [u8; 32] {
    let mut out = property.name.as_bytes().to_vec();
    out.push(u8::from(property.optional));
    out.push(u8::from(property.transient));
    if let Some(modifier) = &property.version_hash_modifier {
        out.extend(modifier.as_bytes());
    }
    sha256(&out)
}

fn attribute_hash(attribute: &Attribute) -> [u8; 32] {
    let mut out = property_hash(&attribute.property).to_vec();
    out.extend(attribute.kind.map_or(0, AttributeType::code).to_le_bytes());
    if attribute.external_storage {
        out.push(1);
    }
    if attribute.preserve_after_deletion {
        out.push(1);
    }
    sha256(&out)
}

fn relationship_hash(relationship: &Relationship) -> [u8; 32] {
    let mut out = property_hash(&relationship.property).to_vec();
    out.extend(
        relationship
            .destination
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    out.extend(
        relationship
            .inverse
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    out.extend(relationship.max_count.to_le_bytes());
    out.extend(relationship.min_count.to_le_bytes());
    if relationship.ordered {
        out.extend([1, 0]);
    }
    sha256(&out)
}

/// Each entity's name and version hash.
pub(super) type Hashes = Vec<(String, [u8; 32])>;

/// `VersionInfo.plist` for a `.momd` of `versions`, each a name and its
/// entities' hashes, `current` the one `.xccurrentversion` names.
pub(super) fn version_info(versions: &[(String, Hashes)], current: Option<&str>) -> Vec<u8> {
    let mut fields = Vec::new();
    if let Some(current) = current {
        fields.push((
            "NSManagedObjectModel_CurrentVersionName".to_string(),
            Plist::String(current.to_string()),
        ));
    }
    fields.push((
        "NSManagedObjectModel_VersionChecksums".to_string(),
        Plist::Dict(
            versions
                .iter()
                .map(|(name, hashes)| (name.clone(), Plist::String(checksum(hashes))))
                .collect(),
        ),
    ));
    fields.push((
        "NSManagedObjectModel_VersionHashes".to_string(),
        Plist::Dict(
            versions
                .iter()
                .map(|(name, hashes)| {
                    let hashes = hashes
                        .iter()
                        .map(|(entity, hash)| (entity.clone(), Plist::Data(hash.to_vec())))
                        .collect();
                    (name.clone(), Plist::Dict(hashes))
                })
                .collect(),
        ),
    ));
    crate::cli::keyed_archive::write(&Plist::Dict(fields))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decimal_moves_its_trailing_zeros_to_the_exponent() {
        assert_eq!(parse_decimal("-12.345"), (true, 12345, -3));
        assert_eq!(parse_decimal("1.50"), (false, 150, -2));
        assert_eq!(parse_decimal("1e3"), (false, 1, 3));
        assert_eq!(parse_decimal("0"), (false, 0, 0));
    }

    #[test]
    fn numbers_read_the_way_foundation_reads_them() {
        assert_eq!(leading_integer(" -7"), -7);
        assert_eq!(leading_integer("3.7"), 3);
        assert_eq!(leading_integer("abc"), 0);
        assert!(bool_value("YES"));
        assert!(bool_value("true"));
        assert!(bool_value("01"));
        assert!(!bool_value("NO"));
        assert!(!bool_value("0"));
    }
}
