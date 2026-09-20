//! A typed view over a parsed `project.xcproj` document: targets,
//! configurations, and the stored build-settings layer, read and edited in
//! place.
//!
//! This is a view rather than a decoded model, which is how the rest of the
//! crate treats project files — [`crate::pbxproj`] parses to a generic tree and
//! [`crate::settings_pbxproj`] and friends read and edit it. Two things follow.
//! Writing back is byte-exact by construction, because everything the view did
//! not touch is still the tree the parser produced. And a document carrying a
//! key this module has never heard of survives an edit, which matters while the
//! published schema is at 0.1.0.
//!
//! **Where the same information lives, compared with `pbxproj`.** A
//! configuration is a name in the project's `configurations` list, not an
//! `XCBuildConfiguration` object per target, and there is exactly one
//! `build-settings` map per scope. What used to be one map per configuration is
//! now a condition on the key — `SWIFT_OPTIMIZATION_LEVEL[config=Debug]` — the
//! same spelling `.xcconfig` files have always used, which is why
//! [`crate::condition`] reads both. Verified against `xcodebuild
//! -showBuildSettings` on Xcode 27: that key resolves under Debug and is absent
//! under Release.

use crate::xcproj::{Object, Value};

pub use crate::stored_settings::{Scope, Setting};

/// Read a stored value out of a document.
pub(crate) fn setting_from_value(value: &Value) -> Result<Setting, String> {
    match value {
        Value::String(s) => Ok(Setting::String(s.clone())),
        Value::Array(items) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "array setting has a non-string element".to_string())
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Setting::List),
        other => Err(format!("build setting is {}", shape_of(other))),
    }
}

/// The stored form Xcode writes: a plain string for one element, an array for
/// more.
pub(crate) fn setting_to_value(setting: &Setting) -> Value {
    match setting {
        Setting::String(s) => Value::String(s.clone()),
        Setting::List(items) => Value::Array(
            items
                .iter()
                .cloned()
                .map(Value::String)
                .collect::<Vec<_>>()
                .into(),
        ),
    }
}

fn shape_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// One stored entry: the setting's name, the condition attached to it, if any,
/// and its value.
///
/// `SWIFT_OPTIMIZATION_LEVEL[config=Debug]` splits into name
/// `SWIFT_OPTIMIZATION_LEVEL` and condition `[config=Debug]`; the condition is
/// kept verbatim, since it can carry several clauses and wildcards
/// (`[sdk=iphoneos*][arch=arm64]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub condition: Option<String>,
    pub value: Setting,
}

impl Entry {
    /// The key as stored, name and condition rejoined.
    #[must_use]
    pub fn key(&self) -> String {
        match &self.condition {
            Some(c) => format!("{}{c}", self.name),
            None => self.name.clone(),
        }
    }
}

/// Split a stored key into its name and its condition suffix.
#[must_use]
pub fn split_key(key: &str) -> (&str, Option<&str>) {
    match key.find('[') {
        Some(i) if key.ends_with(']') => (&key[..i], Some(&key[i..])),
        _ => (key, None),
    }
}

/// The names of the project's build configurations, in document order.
///
/// A configuration is a bare string until it needs an xcconfig or an id, at
/// which point it becomes an object with a `name`.
#[must_use]
pub fn configuration_names(root: &Value) -> Vec<String> {
    root.get("configurations")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(configuration_name)
        .collect()
}

fn configuration_name(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Object(_) => value
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

/// The configuration a build uses when none is named.
#[must_use]
pub fn default_configuration(root: &Value) -> Option<&str> {
    root.get("default-configuration").and_then(Value::as_str)
}

/// Target names, in document order.
#[must_use]
pub fn target_names(root: &Value) -> Vec<String> {
    targets(root)
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn targets(root: &Value) -> &[Value] {
    root.get("targets")
        .and_then(Value::as_array)
        .unwrap_or_default()
}

/// The stored settings for a scope, in document order.
pub fn raw(root: &Value, scope: &Scope) -> Result<Vec<Entry>, String> {
    let Some(settings) = settings_object(root, scope)? else {
        return Ok(Vec::new());
    };
    settings
        .iter()
        .map(|(key, value)| {
            let (name, condition) = split_key(key);
            Ok(Entry {
                name: name.to_string(),
                condition: condition.map(str::to_string),
                value: setting_from_value(value).map_err(|e| format!("{key}: {e}"))?,
            })
        })
        .collect()
}

pub(crate) fn settings_object<'a>(
    root: &'a Value,
    scope: &Scope,
) -> Result<Option<&'a Object>, String> {
    let owner = scope_object(root, scope)?;
    Ok(owner.get("build-settings").and_then(Value::as_object))
}

pub(crate) fn scope_object<'a>(root: &'a Value, scope: &Scope) -> Result<&'a Value, String> {
    match scope {
        Scope::Project => Ok(root),
        Scope::Target(name) => targets(root)
            .iter()
            .find(|t| t.get("name").and_then(Value::as_str) == Some(name.as_str()))
            .ok_or_else(|| format!("no target named {name}")),
    }
}

/// The xcconfig a configuration is based on in this scope, as the stored
/// `{ anchor, relative-path }` pair.
///
/// The project lists every configuration; a target lists only the ones it
/// specializes, under `specialized-configurations`.
#[must_use]
pub fn base_xcconfig<'a>(root: &'a Value, scope: &Scope, configuration: &str) -> Option<&'a Value> {
    let owner = scope_object(root, scope).ok()?;
    let key = match scope {
        Scope::Project => "configurations",
        Scope::Target(_) => "specialized-configurations",
    };
    owner
        .get(key)
        .and_then(Value::as_array)?
        .iter()
        .find(|c| configuration_name(c).as_deref() == Some(configuration))
        .and_then(|c| c.get("file"))
}

/// The condition that scopes a setting to one build configuration.
#[must_use]
pub fn configuration_condition(configuration: &str) -> String {
    format!("[config={configuration}]")
}

/// Assign a stored setting, replacing any existing value for the same key.
///
/// `condition` is the suffix as written, e.g. `[config=Debug]`; pass `None` for
/// a setting that applies everywhere. A scope with no `build-settings` yet
/// grows one.
pub fn set(
    root: &mut Value,
    scope: &Scope,
    name: &str,
    condition: Option<&str>,
    value: &Setting,
) -> Result<(), String> {
    let key = match condition {
        Some(c) => format!("{name}{c}"),
        None => name.to_string(),
    };
    let stored = setting_to_value(value);
    let owner = scope_object_mut(root, scope)?;
    let owner = owner
        .as_object_mut()
        .ok_or_else(|| "scope is not an object".to_string())?;
    if owner.get("build-settings").is_none() {
        owner.insert_sorted("build-settings".to_string(), Value::Object(Object::new()));
    }
    owner
        .get_mut("build-settings")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "build-settings is not an object".to_string())?
        .insert_sorted(key, stored);
    Ok(())
}

/// Remove a stored setting. Returns whether it was there. An emptied
/// `build-settings` is dropped, since Xcode omits empty collections.
pub fn unset(
    root: &mut Value,
    scope: &Scope,
    name: &str,
    condition: Option<&str>,
) -> Result<bool, String> {
    let key = match condition {
        Some(c) => format!("{name}{c}"),
        None => name.to_string(),
    };
    let owner = scope_object_mut(root, scope)?
        .as_object_mut()
        .ok_or_else(|| "scope is not an object".to_string())?;
    let Some(settings) = owner
        .get_mut("build-settings")
        .and_then(Value::as_object_mut)
    else {
        return Ok(false);
    };
    let removed = settings.remove(&key).is_some();
    if settings.is_empty() {
        owner.remove("build-settings");
    }
    Ok(removed)
}

pub(crate) fn scope_object_mut<'a>(
    root: &'a mut Value,
    scope: &Scope,
) -> Result<&'a mut Value, String> {
    match scope {
        Scope::Project => Ok(root),
        Scope::Target(name) => match root.get_mut("targets") {
            Some(Value::Array(items)) => items
                .iter_mut()
                .find(|t| t.get("name").and_then(Value::as_str) == Some(name.as_str()))
                .ok_or_else(|| format!("no target named {name}")),
            _ => Err(format!("no target named {name}")),
        },
    }
}
