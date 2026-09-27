//! Referential integrity in a parsed `project.pbxproj`: which objects name a
//! given object, so that an edit deleting one can take every name with it or
//! refuse.
//!
//! An object is named by its id wherever another object stores that id: a
//! group's `children`, a build file's `fileRef`, a configuration's
//! `baseConfigurationReference`, a target's `productReference`, the project's
//! `mainGroup`. The format has no schema saying which values are ids, so a
//! string equal to a key of `objects` counts as naming that object, wherever
//! it sits inside another object. `buildSettings` is the one place skipped:
//! its values are settings, and one that happens to spell an id names nothing.
//!
//! A group listing is the one name a delete removes on its own, from every
//! group that lists the node: a group naming a missing object is a corrupt
//! file, and a node listed in two groups is one Xcode 27.2 refuses to open.
//! Every other name is a reason to refuse the delete, or, for a verb that
//! only tidies up after itself, a reason to keep the object.

use crate::pbxproj::{Dict, Value};

/// The kinds of object whose `children` is a navigator listing.
const GROUP_ISAS: [&str; 3] = ["PBXGroup", "PBXVariantGroup", "XCVersionGroup"];

/// One place an object names another: the naming object and the top-level
/// key under which the id sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Referrer {
    pub owner: String,
    pub key: String,
}

/// Every place outside the navigator's group listings that names `id`, in
/// object order.
pub(crate) fn referrers(objects: &Dict, id: &str) -> Vec<Referrer> {
    let mut out = Vec::new();
    for (owner, object) in objects.iter() {
        let Some(fields) = object.as_dict() else {
            continue;
        };
        let group = GROUP_ISAS.contains(&isa(object));
        for (key, value) in fields.iter() {
            if key == "buildSettings" || (group && key == "children") {
                continue;
            }
            if names(value, id) {
                out.push(Referrer {
                    owner: owner.clone(),
                    key: key.clone(),
                });
            }
        }
    }
    out
}

/// Whether `value` holds `id` as a string anywhere inside it.
fn names(value: &Value, id: &str) -> bool {
    match value {
        Value::String(s) => s == id,
        Value::Array(items) => items.iter().any(|v| names(v, id)),
        Value::Dict(dict) => dict.values().any(|v| names(v, id)),
    }
}

/// The groups whose `children` list `id`, in object order.
pub(crate) fn listings(objects: &Dict, id: &str) -> Vec<String> {
    objects
        .iter()
        .filter(|(_, o)| GROUP_ISAS.contains(&isa(o)))
        .filter(|(_, o)| {
            o.get("children")
                .and_then(Value::as_array)
                .is_some_and(|c| c.iter().any(|v| v.as_str() == Some(id)))
        })
        .map(|(guid, _)| guid.clone())
        .collect()
}

/// Take `id` out of every group that lists it, returning those groups.
pub(crate) fn unlist(objects: &mut Dict, id: &str) -> Vec<String> {
    let groups = listings(objects, id);
    for group in &groups {
        if let Some(children) = objects
            .get_mut(group)
            .and_then(|g| g.get_mut("children"))
            .and_then(Value::as_array_mut)
        {
            children.retain(|v| v.as_str() != Some(id));
        }
    }
    groups
}

/// The refusal for deleting `id` while `referrers` still name it, or `None`
/// when nothing does.
pub(crate) fn still_named(objects: &Dict, id: &str, referrers: &[Referrer]) -> Option<String> {
    if referrers.is_empty() {
        return None;
    }
    let what: Vec<String> = referrers.iter().map(|r| describe(objects, r)).collect();
    Some(crate::tree::still_named_refusal(id, &what))
}

/// How a referrer reads in a sentence, after "is still".
fn describe(objects: &Dict, referrer: &Referrer) -> String {
    let owner = objects.get(&referrer.owner);
    let owner_isa = owner.map_or("", isa);
    let name = |o: Option<&Value>| {
        o.and_then(|o| o.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match (owner_isa, referrer.key.as_str()) {
        ("XCBuildConfiguration", "baseConfigurationReference") => format!(
            "the xcconfig that {} is based on",
            configuration_label(objects, &referrer.owner)
        ),
        ("XCBuildConfiguration", "baseConfigurationReferenceAnchor") => format!(
            "the folder holding the xcconfig that {} is based on",
            configuration_label(objects, &referrer.owner)
        ),
        (_, "productReference") => format!("the product of target '{}'", name(owner)),
        ("PBXProject", "mainGroup") => "the project's navigator root".to_string(),
        ("PBXProject", "productRefGroup") => "the project's Products group".to_string(),
        ("PBXProject", "projectReferences") => "part of a reference to another project".to_string(),
        ("PBXContainerItemProxy", "containerPortal") => {
            "the project a dependency on another project points into".to_string()
        }
        ("PBXBuildFile", "fileRef") => "built by a build file".to_string(),
        (_, "fileSystemSynchronizedGroups") => {
            format!("a synchronized folder of target '{}'", name(owner))
        }
        ("XCVersionGroup", "currentVersion") => format!(
            "the current version of {}",
            owner
                .and_then(|o| o.get("path"))
                .and_then(Value::as_str)
                .unwrap_or("a versioned model")
        ),
        (isa, key) => format!("named by the '{key}' of {isa} {}", referrer.owner),
    }
}

/// `the 'Debug' configuration of target 'App'`, or `of the project`.
fn configuration_label(objects: &Dict, configuration: &str) -> String {
    let name = objects
        .get(configuration)
        .and_then(|c| c.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let holds = |o: &Value, key: &str, id: &str| {
        o.get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|v| v.as_str() == Some(id)))
    };
    let list = objects
        .iter()
        .find(|(_, o)| {
            isa(o) == "XCConfigurationList" && holds(o, "buildConfigurations", configuration)
        })
        .map(|(guid, _)| guid.as_str());
    let owner = list.and_then(|list| {
        objects
            .values()
            .find(|o| o.get("buildConfigurationList").and_then(Value::as_str) == Some(list))
    });
    match owner {
        Some(o) if isa(o) == "PBXProject" => format!("the '{name}' configuration of the project"),
        Some(o) => format!(
            "the '{name}' configuration of target '{}'",
            o.get("name").and_then(Value::as_str).unwrap_or_default()
        ),
        None => format!("the '{name}' configuration"),
    }
}

fn isa(object: &Value) -> &str {
    object
        .get("isa")
        .and_then(Value::as_str)
        .unwrap_or_default()
}
