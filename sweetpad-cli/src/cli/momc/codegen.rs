//! The classes momc writes for a model's entities when their code
//! generation is Class Definition or Category/Extension, in Swift or
//! Objective-C as the model's language says, file for file as momc
//! writes them.

use std::fmt::Write as _;

use super::model::{
    Attribute, AttributeType, Codegen, Entity, Language, Model, Relationship, capitalized,
};

/// The files momc generates for `model`, named `name` (the model file's
/// name), by file name, in the order it lists them.
pub(super) fn files(model: &Model, name: &str, header: &Header) -> Vec<(String, String)> {
    let entities: Vec<&Entity> = model
        .entities
        .iter()
        .filter(|e| e.codegen != Codegen::Manual && e.class_name.is_some())
        .collect();
    let mut out = Vec::new();
    if entities.is_empty() {
        return out;
    }
    match model.language {
        Language::Swift => {
            for entity in &entities {
                let class = class_name(entity);
                if entity.codegen == Codegen::Class {
                    let file = format!("{class}+CoreDataClass.swift");
                    let text = swift_class(model, entity);
                    out.push((file.clone(), header.render(&file) + &text));
                }
                let file = format!("{class}+CoreDataProperties.swift");
                let text = swift_properties(model, entity);
                out.push((file.clone(), header.render(&file) + &text));
            }
            let file = format!("{name}+CoreDataModel.swift");
            let text = format!(
                "public import Foundation\npublic import CoreData\n\n\
                 public typealias {name}FoundationFrameworkSet = NSSet\n\
                 public typealias {name}CoreDataFrameworkManagedObject = NSManagedObject\n\n\n"
            );
            out.push((file.clone(), header.render(&file) + &text));
        }
        Language::ObjectiveC => {
            for entity in &entities {
                let class = class_name(entity);
                if entity.codegen == Codegen::Class {
                    for (file, text) in [
                        (
                            format!("{class}+CoreDataClass.h"),
                            objc_class_header(model, entity),
                        ),
                        (
                            format!("{class}+CoreDataClass.m"),
                            format!(
                                "#import \"{class}+CoreDataClass.h\"\n\n@implementation {class}\n\n@end\n"
                            ),
                        ),
                    ] {
                        out.push((file.clone(), header.render(&file) + &text));
                    }
                }
                for (file, text) in [
                    (
                        format!("{class}+CoreDataProperties.h"),
                        objc_properties_header(model, entity),
                    ),
                    (
                        format!("{class}+CoreDataProperties.m"),
                        objc_properties_implementation(entity),
                    ),
                ] {
                    out.push((file.clone(), header.render(&file) + &text));
                }
            }
            let mut imports = String::new();
            for entity in &entities {
                let _ = writeln!(imports, "#import \"{}\"", imported_header(entity));
            }
            for (file, text) in [
                (
                    format!("{name}+CoreDataModel.h"),
                    format!(
                        "#import <Foundation/Foundation.h>\n#import <CoreData/CoreData.h>\n\n{imports}\n\n\n\n"
                    ),
                ),
                (
                    format!("{name}+CoreDataModel.m"),
                    format!("#import \"{name}+CoreDataModel.h\"\n\n\n"),
                ),
            ] {
                out.push((file.clone(), header.render(&file) + &text));
            }
        }
    }
    out
}

/// What the comment at the top of each file says: who generated it, and
/// when.
pub(super) struct Header {
    pub author: String,
    pub date: String,
}

impl Header {
    fn render(&self, file: &str) -> String {
        format!(
            "//\n//  {file}\n//  \n//\n//  Created by {} on {}.\n//\n\
             //  This file was automatically generated and should not be edited.\n//\n\n",
            self.author, self.date
        )
    }
}

fn class_name(entity: &Entity) -> &str {
    entity.class_name.as_deref().unwrap_or("NSManagedObject")
}

/// The class an entity's objects are, or `NSManagedObject` when it names
/// none.
fn class_of<'a>(model: &'a Model, entity_name: Option<&str>) -> &'a str {
    entity_name
        .and_then(|name| model.entity(name))
        .and_then(|e| e.class_name.as_deref())
        .unwrap_or("NSManagedObject")
}

fn parent_class<'a>(model: &'a Model, entity: &Entity) -> &'a str {
    class_of(model, entity.parent.as_deref())
}

/// The header that declares an entity's generated class: its class file,
/// or for a category its properties, beside a class written by hand.
fn imported_header(entity: &Entity) -> String {
    let class = class_name(entity);
    match entity.codegen {
        Codegen::Class => format!("{class}+CoreDataClass.h"),
        _ => format!("{class}+CoreDataProperties.h"),
    }
}

fn has_subentities(model: &Model, entity: &Entity) -> bool {
    model
        .entities
        .iter()
        .any(|e| e.parent.as_deref() == Some(entity.name.as_str()))
}

fn swift_class(model: &Model, entity: &Entity) -> String {
    let class = class_name(entity);
    format!(
        "public import Foundation\npublic import CoreData\n\n\
         public typealias {class}CoreDataClassSet = NSSet\n\n\
         @objc({class})\npublic class {class}: {} {{\n\n}}\n",
        parent_class(model, entity)
    )
}

/// The Swift type a property's accessor has.
fn swift_attribute_type(attribute: &Attribute) -> String {
    let Some(kind) = attribute.kind else {
        return "NSObject?".to_string();
    };
    let scalar = attribute.scalar;
    match kind {
        AttributeType::Integer16 if scalar => "Int16",
        AttributeType::Integer32 if scalar => "Int32",
        AttributeType::Integer64 if scalar => "Int64",
        AttributeType::Double if scalar => "Double",
        AttributeType::Float if scalar => "Float",
        AttributeType::Boolean if scalar => "Bool",
        AttributeType::Date if scalar => "TimeInterval",
        AttributeType::Integer16
        | AttributeType::Integer32
        | AttributeType::Integer64
        | AttributeType::Double
        | AttributeType::Float
        | AttributeType::Boolean => "NSNumber?",
        AttributeType::Decimal => "NSDecimalNumber?",
        AttributeType::String => "String?",
        AttributeType::Date => "Date?",
        AttributeType::Binary => "Data?",
        AttributeType::Uuid => "UUID?",
        AttributeType::Uri => "URL?",
        AttributeType::Transformable => {
            return format!(
                "{}?",
                attribute.custom_class.as_deref().unwrap_or("NSObject")
            );
        }
        AttributeType::ObjectId => "NSObject?",
    }
    .to_string()
}

fn swift_relationship_type(model: &Model, relationship: &Relationship) -> String {
    match (relationship.to_many, relationship.ordered) {
        (false, _) => format!("{}?", class_of(model, relationship.destination.as_deref())),
        (true, false) => "NSSet?".to_string(),
        (true, true) => "NSOrderedSet?".to_string(),
    }
}

fn swift_properties(model: &Model, entity: &Entity) -> String {
    let class = class_name(entity);
    let mut out = format!(
        "public import Foundation\npublic import CoreData\n\n\n\
         public typealias {class}CoreDataPropertiesSet = NSSet\n\n\
         extension {class} {{\n\n    \
         @nonobjc public class func fetchRequest() -> NSFetchRequest<{class}> {{\n        \
         return NSFetchRequest<{class}>(entityName: \"{}\")\n    }}\n\n",
        entity.name
    );
    for attribute in &entity.attributes {
        let _ = writeln!(
            out,
            "    @NSManaged nonisolated public var {}: {}",
            attribute.property.name,
            swift_attribute_type(attribute)
        );
    }
    for relationship in &entity.relationships {
        let _ = writeln!(
            out,
            "    @NSManaged nonisolated public var {}: {}",
            relationship.property.name,
            swift_relationship_type(model, relationship)
        );
    }
    out.push_str("\n}\n");
    for relationship in entity.relationships.iter().filter(|r| r.to_many) {
        let name = capitalized(&relationship.property.name);
        let item = class_of(model, relationship.destination.as_deref());
        let _ = write!(
            out,
            "\n// MARK: Generated accessors for {}\nextension {class} {{\n\n",
            relationship.property.name
        );
        let mut accessors: Vec<(String, String)> = Vec::new();
        let set = if relationship.ordered {
            accessors.extend([
                (
                    format!("insertObject:in{name}AtIndex:"),
                    format!("insertInto{name}(_ value: {item}, at idx: Int)"),
                ),
                (
                    format!("removeObjectFrom{name}AtIndex:"),
                    format!("removeFrom{name}(at idx: Int)"),
                ),
                (
                    format!("insert{name}:atIndexes:"),
                    format!("insertInto{name}(_ values: [{item}], at indexes: NSIndexSet)"),
                ),
                (
                    format!("remove{name}AtIndexes:"),
                    format!("removeFrom{name}(at indexes: NSIndexSet)"),
                ),
                (
                    format!("replaceObjectIn{name}AtIndex:withObject:"),
                    format!("replace{name}(at idx: Int, with value: {item})"),
                ),
                (
                    format!("replace{name}AtIndexes:with{name}:"),
                    format!("replace{name}(at indexes: NSIndexSet, with values: [{item}])"),
                ),
            ]);
            "NSOrderedSet"
        } else {
            "NSSet"
        };
        accessors.extend([
            (
                format!("add{name}Object:"),
                format!("addTo{name}(_ value: {item})"),
            ),
            (
                format!("remove{name}Object:"),
                format!("removeFrom{name}(_ value: {item})"),
            ),
            (
                format!("add{name}:"),
                format!("addTo{name}(_ values: {set})"),
            ),
            (
                format!("remove{name}:"),
                format!("removeFrom{name}(_ values: {set})"),
            ),
        ]);
        for (selector, signature) in accessors {
            let _ = write!(
                out,
                "    @objc({selector})\n    @NSManaged nonisolated public func {signature}\n\n"
            );
        }
        out.push_str("}\n");
    }
    if entity.parent.is_none() {
        let _ = write!(out, "\nextension {class} : Identifiable {{\n\n}}\n");
    }
    out
}

/// An Objective-C property's attributes and type, and the class it names
/// that needs a forward declaration.
fn objc_attribute(attribute: &Attribute) -> (&'static str, String, Option<String>) {
    const VALUE: &str = "nonatomic";
    const COPY: &str = "nullable, nonatomic, copy";
    const RETAIN: &str = "nullable, nonatomic, retain";
    let Some(kind) = attribute.kind else {
        return (
            RETAIN,
            "NSObject *".to_string(),
            Some("NSObject".to_string()),
        );
    };
    let scalar = attribute.scalar;
    let (attributes, type_name) = match kind {
        AttributeType::Integer16 if scalar => (VALUE, "int16_t "),
        AttributeType::Integer32 if scalar => (VALUE, "int32_t "),
        AttributeType::Integer64 if scalar => (VALUE, "int64_t "),
        AttributeType::Double if scalar => (VALUE, "double "),
        AttributeType::Float if scalar => (VALUE, "float "),
        AttributeType::Boolean if scalar => (VALUE, "BOOL "),
        AttributeType::Date if scalar => (VALUE, "NSTimeInterval "),
        AttributeType::Integer16
        | AttributeType::Integer32
        | AttributeType::Integer64
        | AttributeType::Double
        | AttributeType::Float
        | AttributeType::Boolean => (COPY, "NSNumber *"),
        AttributeType::Decimal => (COPY, "NSDecimalNumber *"),
        AttributeType::String => (COPY, "NSString *"),
        AttributeType::Date => (COPY, "NSDate *"),
        AttributeType::Uuid => (COPY, "NSUUID *"),
        AttributeType::Uri => (COPY, "NSURL *"),
        AttributeType::Binary => (RETAIN, "NSData *"),
        AttributeType::Transformable | AttributeType::ObjectId => {
            let class = attribute
                .custom_class
                .clone()
                .filter(|_| kind == AttributeType::Transformable)
                .unwrap_or_else(|| "NSObject".to_string());
            return (RETAIN, format!("{class} *"), Some(class));
        }
    };
    (attributes, type_name.to_string(), None)
}

fn objc_relationship_type(model: &Model, relationship: &Relationship) -> String {
    let item = class_of(model, relationship.destination.as_deref());
    match (relationship.to_many, relationship.ordered) {
        (false, _) => format!("{item} *"),
        (true, false) => format!("NSSet<{item} *> *"),
        (true, true) => format!("NSOrderedSet<{item} *> *"),
    }
}

fn objc_class_header(model: &Model, entity: &Entity) -> String {
    let class = class_name(entity);
    let parent = entity.parent.as_deref().and_then(|p| model.entity(p));
    let parent_import = match parent {
        Some(p) if p.class_name.is_some() => match p.codegen {
            Codegen::Class => format!("\"{}+CoreDataClass.h\"", class_name(p)),
            _ => format!("\"{}.h\"", class_name(p)),
        },
        _ => "<CoreData/CoreData.h>".to_string(),
    };
    let mut forward: Vec<String> = entity
        .attributes
        .iter()
        .filter_map(|a| objc_attribute(a).2)
        .chain(
            entity
                .relationships
                .iter()
                .map(|r| class_of(model, r.destination.as_deref()).to_string()),
        )
        .collect();
    // The parent's class comes with its header.
    let parent_name = parent_class(model, entity);
    forward.retain(|class| class != parent_name);
    forward.sort();
    forward.dedup();
    let forward = if forward.is_empty() {
        String::new()
    } else {
        format!("@class {};\n\n", forward.join(", "))
    };
    format!(
        "#import <Foundation/Foundation.h>\n#import {parent_import}\n\n{forward}\
         NS_ASSUME_NONNULL_BEGIN\n\n@interface {class} : {}\n\n@end\n\n\
         NS_ASSUME_NONNULL_END\n\n#import \"{class}+CoreDataProperties.h\"\n",
        parent_class(model, entity)
    )
}

fn objc_properties_header(model: &Model, entity: &Entity) -> String {
    let class = class_name(entity);
    let import = match entity.codegen {
        Codegen::Class => format!("{class}+CoreDataClass.h"),
        _ => format!("{class}.h"),
    };
    // Swift sees a subclass's fetchRequest() as an override of its
    // parent's, so momc names only the leaves' for Swift.
    let swift_name = if has_subentities(model, entity) {
        ""
    } else {
        " NS_SWIFT_NAME(fetchRequest())"
    };
    let mut out = format!(
        "#import \"{import}\"\n\n\nNS_ASSUME_NONNULL_BEGIN\n\n\
         @interface {class} (CoreDataProperties)\n\n\
         + (NSFetchRequest<{class} *> *)fetchRequest{swift_name};\n\n"
    );
    for attribute in &entity.attributes {
        let (attributes, type_name, _) = objc_attribute(attribute);
        let _ = writeln!(
            out,
            "@property ({attributes}) {type_name}{};",
            attribute.property.name
        );
    }
    for relationship in &entity.relationships {
        let _ = writeln!(
            out,
            "@property (nullable, nonatomic, retain) {}{};",
            objc_relationship_type(model, relationship),
            relationship.property.name
        );
    }
    out.push_str("\n@end\n");
    let to_many: Vec<&Relationship> = entity.relationships.iter().filter(|r| r.to_many).collect();
    if !to_many.is_empty() {
        let _ = write!(out, "\n@interface {class} (CoreDataGeneratedAccessors)\n\n");
        let groups: Vec<String> = to_many
            .iter()
            .map(|relationship| {
                let name = capitalized(&relationship.property.name);
                let item = class_of(model, relationship.destination.as_deref());
                let mut lines = Vec::new();
                let set = if relationship.ordered {
                    lines.extend([
                        format!("- (void)insertObject:({item} *)value in{name}AtIndex:(NSUInteger)idx;"),
                        format!("- (void)removeObjectFrom{name}AtIndex:(NSUInteger)idx;"),
                        format!("- (void)insert{name}:(NSArray<{item} *> *)value atIndexes:(NSIndexSet *)indexes;"),
                        format!("- (void)remove{name}AtIndexes:(NSIndexSet *)indexes;"),
                        format!("- (void)replaceObjectIn{name}AtIndex:(NSUInteger)idx withObject:({item} *)value;"),
                        format!("- (void)replace{name}AtIndexes:(NSIndexSet *)indexes with{name}:(NSArray<{item} *> *)values;"),
                    ]);
                    "NSOrderedSet"
                } else {
                    "NSSet"
                };
                lines.extend([
                    format!("- (void)add{name}Object:({item} *)value;"),
                    format!("- (void)remove{name}Object:({item} *)value;"),
                    format!("- (void)add{name}:({set}<{item} *> *)values;"),
                    format!("- (void)remove{name}:({set}<{item} *> *)values;"),
                ]);
                lines.join("\n") + "\n"
            })
            .collect();
        out.push_str(&groups.join("\n"));
        out.push_str("\n@end\n");
    }
    out.push_str("\nNS_ASSUME_NONNULL_END\n");
    out
}

fn objc_properties_implementation(entity: &Entity) -> String {
    let class = class_name(entity);
    let mut out = format!(
        "#import \"{class}+CoreDataProperties.h\"\n\n\
         @implementation {class} (CoreDataProperties)\n\n\
         + (NSFetchRequest<{class} *> *)fetchRequest {{\n\
         \treturn [NSFetchRequest fetchRequestWithEntityName:@\"{}\"];\n}}\n\n",
        entity.name
    );
    let names = entity
        .attributes
        .iter()
        .map(|a| &a.property.name)
        .chain(entity.relationships.iter().map(|r| &r.property.name));
    for name in names {
        let _ = writeln!(out, "@dynamic {name};");
    }
    out.push_str("\n@end\n");
    out
}
