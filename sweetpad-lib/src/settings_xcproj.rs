//! Reading and mutating the stored build settings of a parsed
//! [`crate::xcproj::Value`] document — the `project.xcproj` counterpart of
//! [`crate::settings_pbxproj`], speaking the same
//! [`crate::stored_settings`] vocabulary.
//!
//! Everything here is pure (no I/O): callers parse the file, mutate the tree,
//! and serialize/write it. Keys are spliced in at their byte-sorted position,
//! the order Xcode writes, so diffs stay minimal and Xcode-shaped.
//!
//! **How a configuration reaches a setting.** A scope holds one
//! `build-settings` map, and a key can carry the configuration as a
//! `[config=Debug]` clause. Measured against `xcodebuild -showBuildSettings`
//! on Xcode 27, three rules govern it:
//!
//! * `KEY[config=Debug]` applies under Debug and nowhere else.
//! * `KEY` with no clause applies everywhere and **shadows** every
//!   `KEY[config=…]` in the same map, whichever comes first in the file. This
//!   is the one place the format departs from xcconfig's more-specific-wins
//!   rule, and it is why an edit here rewrites every form of a key at once
//!   rather than adding one.
//! * Other clauses (`[sdk=…]`, `[arch=…]`) stay ordinary conditionals and
//!   resolve by specificity, combining with `[config=…]` in either order.
//!
//! So an edit computes the value each configuration should end up with, then
//! writes that picture back the way Xcode does: one plain key when every
//! configuration agrees, one `[config=…]` key each when they don't. Xcode
//! never writes both forms of a key — 0 of the 161 `build-settings` maps in
//! the converted corpus do — and this keeps that true.

use std::collections::BTreeMap;

use crate::schema_xcproj::{self as schema, setting_from_value, setting_to_value};
use crate::stored_settings::{Assignment, Change, ConfigSettings, Op, Scope, Setting};
use crate::xcproj::{Object, Value};

/// Every target name declared in the project, in file order.
#[must_use]
pub fn target_names(root: &Value) -> Vec<String> {
    schema::target_names(root)
}

/// The configuration names a scope builds. Every scope uses the project's
/// list; a target's `specialized-configurations` only overrides their
/// xcconfigs.
///
/// # Errors
/// Returns a message when the named target is missing.
pub fn configuration_names(root: &Value, scope: &Scope) -> Result<Vec<String>, String> {
    schema::scope_object(root, scope)?;
    Ok(schema::configuration_names(root))
}

/// The stored settings of every configuration of `scope`, in file order — the
/// `settings show` payload, with each key reported under the configurations it
/// actually applies to and its `[config=…]` clause dropped.
///
/// # Errors
/// Returns a message when the target is missing or a value has a shape a build
/// setting can't have.
pub fn raw(root: &Value, scope: &Scope) -> Result<Vec<ConfigSettings>, String> {
    let configurations = configuration_names(root, scope)?;
    let stored = stored_entries(root, scope)?;
    let mut out = Vec::with_capacity(configurations.len());
    for configuration in configurations {
        let settings = stored
            .iter()
            .filter_map(|entry| {
                entry
                    .value_for(&configuration)
                    .map(|v| (entry.key.clone(), v.clone()))
            })
            .collect();
        out.push(ConfigSettings {
            configuration,
            settings,
        });
    }
    Ok(out)
}

/// Apply `assignments` to every selected configuration of `scope`.
/// `configurations` empty means all of them; naming an unknown configuration
/// is an error (mutations don't guess). Returns one [`Change`] per
/// (configuration, key).
///
/// # Errors
/// Returns a message when the target or configuration is missing, or an
/// existing value has a shape a build setting can't have.
pub fn set(
    root: &mut Value,
    scope: &Scope,
    configurations: &[String],
    assignments: &[Assignment],
) -> Result<Vec<Change>, String> {
    edit(root, scope, configurations, |selected, current| {
        assignments
            .iter()
            .map(|assignment| {
                let values = selected
                    .iter()
                    .map(|configuration| {
                        let old = current(&assignment.key, configuration);
                        let elements = match &assignment.op {
                            Op::Assign(values) => values.clone(),
                            Op::Append(values) => {
                                let mut merged =
                                    old.as_ref().map(Setting::elements).unwrap_or_default();
                                merged.extend(values.iter().cloned());
                                merged
                            }
                        };
                        (
                            configuration.clone(),
                            old,
                            Some(Setting::canonical(&elements)),
                        )
                    })
                    .collect();
                (assignment.key.clone(), values)
            })
            .collect()
    })
}

/// Remove `keys` from every selected configuration of `scope` — true
/// inheritance, not `$(inherited)`. Exact key match only: conditional variants
/// (`KEY[sdk=…]`) are separate keys and are never implicitly swept. An absent
/// key is a recorded no-op (`old: None`), so re-run scripts stay green.
///
/// # Errors
/// Returns a message when the target or configuration is missing.
pub fn unset(
    root: &mut Value,
    scope: &Scope,
    configurations: &[String],
    keys: &[String],
) -> Result<Vec<Change>, String> {
    edit(root, scope, configurations, |selected, current| {
        keys.iter()
            .map(|key| {
                let values = selected
                    .iter()
                    .map(|configuration| (configuration.clone(), current(key, configuration), None))
                    .collect();
                (key.clone(), values)
            })
            .collect()
    })
}

/// The project-dir-relative xcconfig backing each selected configuration, as
/// `(configuration, path)` pairs — configurations without one are omitted.
/// Callers warn when an edited key is also assigned there: the stored layer
/// outranks the xcconfig, so the edit silently shadows it.
///
/// # Errors
/// Returns a message when the target or configuration is missing.
pub fn base_xcconfigs(
    root: &Value,
    scope: &Scope,
    configurations: &[String],
) -> Result<Vec<(String, String)>, String> {
    let selected = selected_configurations(root, scope, configurations)?;
    Ok(selected
        .into_iter()
        .filter_map(|configuration| {
            let file = schema::base_xcconfig(root, scope, &configuration)?;
            let relative = file
                .get("relative-path")
                .or(Some(file))
                .and_then(Value::as_str)?;
            Some((configuration, relative.to_string()))
        })
        .collect())
}

/// One stored setting, gathered across the forms its key can take: the plain
/// key's value, and a value per configuration named in a `[config=…]` clause.
struct Entry {
    /// The key with any `[config=…]` clause removed — how it is reported and
    /// how a caller names it.
    key: String,
    plain: Option<Setting>,
    per_configuration: BTreeMap<String, Setting>,
}

impl Entry {
    /// The value this key has under `configuration`. A plain key shadows the
    /// conditional forms.
    fn value_for(&self, configuration: &str) -> Option<&Setting> {
        self.plain
            .as_ref()
            .or_else(|| self.per_configuration.get(configuration))
    }
}

/// Every stored key of a scope, in file order, with its forms gathered.
fn stored_entries(root: &Value, scope: &Scope) -> Result<Vec<Entry>, String> {
    let Some(settings) = schema::settings_object(root, scope)? else {
        return Ok(Vec::new());
    };
    let mut order: Vec<String> = Vec::new();
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    for (stored, value) in settings {
        let (key, configuration) = split_configuration(stored);
        let setting = setting_from_value(value).map_err(|e| format!("value of {stored}: {e}"))?;
        let entry = entries.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Entry {
                key,
                plain: None,
                per_configuration: BTreeMap::new(),
            }
        });
        match configuration {
            None => entry.plain = Some(setting),
            Some(name) => {
                entry.per_configuration.insert(name, setting);
            }
        }
    }
    Ok(order
        .into_iter()
        .filter_map(|key| entries.remove(&key))
        .collect())
}

/// The per-key edit a caller asks for: the key, and for each selected
/// configuration its old value and the value it should end up with.
type Edits = Vec<(String, Vec<(String, Option<Setting>, Option<Setting>)>)>;

/// Run an edit: gather the current picture, let `plan` decide the new values
/// for the selected configurations, then write every touched key back.
fn edit(
    root: &mut Value,
    scope: &Scope,
    configurations: &[String],
    plan: impl FnOnce(&[String], &dyn Fn(&str, &str) -> Option<Setting>) -> Edits,
) -> Result<Vec<Change>, String> {
    let all = configuration_names(root, scope)?;
    let selected = selected_configurations(root, scope, configurations)?;
    let stored = stored_entries(root, scope)?;

    let current = |key: &str, configuration: &str| -> Option<Setting> {
        stored
            .iter()
            .find(|e| e.key == key)
            .and_then(|e| e.value_for(configuration))
            .cloned()
    };
    let edits = plan(&selected, &current);

    let mut changes = Vec::new();
    for (key, per_configuration) in edits {
        // A configuration nobody selected keeps whatever it had, so the value
        // written back is the whole picture, not just the edited part.
        let mut picture: Vec<(String, Option<Setting>)> =
            all.iter().map(|c| (c.clone(), current(&key, c))).collect();
        for (configuration, old, new) in per_configuration {
            if let Some(slot) = picture.iter_mut().find(|(c, _)| *c == configuration) {
                slot.1.clone_from(&new);
            }
            changes.push(Change {
                target: scope.target().map(str::to_string),
                configuration,
                key: key.clone(),
                old,
                new,
            });
        }
        write_key(root, scope, &key, &picture)?;
    }
    Ok(changes)
}

/// Replace every stored form of `key` in `scope` with the picture given: one
/// plain key when every configuration agrees, one `[config=…]` key each when
/// they don't, and nothing at all when no configuration has a value.
fn write_key(
    root: &mut Value,
    scope: &Scope,
    key: &str,
    picture: &[(String, Option<Setting>)],
) -> Result<(), String> {
    let owner = schema::scope_object_mut(root, scope)?
        .as_object_mut()
        .ok_or_else(|| "scope is not an object".to_string())?;
    if owner.get("build-settings").is_none() {
        owner.insert_sorted("build-settings".to_string(), Value::Object(Object::new()));
    }
    let settings = owner
        .get_mut("build-settings")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "build-settings is not an object".to_string())?;

    let stale: Vec<String> = settings
        .iter()
        .map(|(stored, _)| stored.to_string())
        .filter(|stored| split_configuration(stored).0 == key)
        .collect();
    for stored in stale {
        settings.remove(&stored);
    }

    let uniform = picture
        .first()
        .is_some_and(|(_, first)| picture.iter().all(|(_, v)| v == first));
    if uniform {
        if let Some((_, Some(value))) = picture.first() {
            settings.insert_sorted(key.to_string(), setting_to_value(value));
        }
    } else {
        for (configuration, value) in picture {
            if let Some(value) = value {
                settings.insert_sorted(
                    format!("{key}[config={configuration}]"),
                    setting_to_value(value),
                );
            }
        }
    }

    if settings.is_empty() {
        owner.remove("build-settings");
    }
    Ok(())
}

/// The configurations an edit touches: the ones named, or all of them. An
/// unknown name is an error — mutations don't guess.
fn selected_configurations(
    root: &Value,
    scope: &Scope,
    configurations: &[String],
) -> Result<Vec<String>, String> {
    let all = configuration_names(root, scope)?;
    if configurations.is_empty() {
        return Ok(all);
    }
    for wanted in configurations {
        if !all.contains(wanted) {
            return Err(format!(
                "no configuration named {wanted} (have: {})",
                all.join(", ")
            ));
        }
    }
    Ok(configurations.to_vec())
}

/// Split a stored key into the key as a caller names it and the configuration
/// its `[config=…]` clause selects, if any. Other clauses stay on the key.
fn split_configuration(stored: &str) -> (String, Option<String>) {
    let Some(start) = stored.find('[') else {
        return (stored.to_string(), None);
    };
    let (name, clauses) = stored.split_at(start);
    let mut kept = String::new();
    let mut configuration = None;
    for clause in clauses.split_inclusive(']') {
        match clause
            .strip_prefix("[config=")
            .and_then(|c| c.strip_suffix(']'))
        {
            Some(value) if configuration.is_none() => configuration = Some(value.to_string()),
            _ => kept.push_str(clause),
        }
    }
    (format!("{name}{kept}"), configuration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_clause_splits_off_and_the_others_stay() {
        assert_eq!(split_configuration("SDKROOT"), ("SDKROOT".into(), None));
        assert_eq!(
            split_configuration("SDKROOT[config=Debug]"),
            ("SDKROOT".into(), Some("Debug".into()))
        );
        assert_eq!(
            split_configuration("OTHER_LDFLAGS[sdk=iphoneos*][config=Debug]"),
            ("OTHER_LDFLAGS[sdk=iphoneos*]".into(), Some("Debug".into()))
        );
        assert_eq!(
            split_configuration("OTHER_LDFLAGS[config=Debug][sdk=iphoneos*]"),
            ("OTHER_LDFLAGS[sdk=iphoneos*]".into(), Some("Debug".into()))
        );
        assert_eq!(
            split_configuration("OTHER_LDFLAGS[sdk=iphoneos*]"),
            ("OTHER_LDFLAGS[sdk=iphoneos*]".into(), None)
        );
    }
}
