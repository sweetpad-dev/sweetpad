//! Reading and mutating raw build settings — `XCBuildConfiguration.buildSettings`
//! entries — in a parsed [`crate::pbxproj::Value`] tree.
//!
//! This is the *stored* layer, one input to the resolver: what the pbxproj
//! itself assigns, before xcconfig files, SDK defaults, and `$(inherited)`
//! chains are applied. `sweetpad settings set/unset` edits it and
//! `settings show --raw` prints it; the resolved view stays
//! [`crate::resolver`]'s job.
//!
//! Everything here is pure (no I/O): callers parse the file, mutate the tree,
//! and serialize/write it — the same contract as [`crate::spm_pbxproj`]. New
//! keys are spliced into `buildSettings` at their alphabetical position, the
//! order Xcode writes, so diffs stay minimal and Xcode-shaped.

use crate::pbxproj::{Dict, Value};

/// Which configurations to operate on: the project-level ones (inherited by
/// every target) or a single target's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Project,
    Target(String),
}

impl Scope {
    /// The target name, when target-scoped.
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        match self {
            Scope::Project => None,
            Scope::Target(name) => Some(name),
        }
    }
}

/// A raw setting value as stored in `buildSettings`: a plain string or an
/// array of strings. Xcode treats a whitespace-separated string and an array
/// as the same list at resolve time; the stored shape is preserved here so
/// reports show exactly what the file says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    String(String),
    List(Vec<String>),
}

impl Setting {
    /// The value as its element list — arrays as-is, strings whitespace-split
    /// (how xcodebuild consumes list-typed settings). `KEY += v` appends to
    /// this normalized form.
    #[must_use]
    pub fn elements(&self) -> Vec<String> {
        match self {
            Setting::String(s) => s.split_whitespace().map(str::to_string).collect(),
            Setting::List(items) => items.clone(),
        }
    }

    /// Human rendering: the string itself, or elements joined with a space.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Setting::String(s) => s.clone(),
            Setting::List(items) => items.join(" "),
        }
    }

    fn from_value(value: &Value) -> Result<Setting, String> {
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
            Value::Dict(_) => Err("dictionary-valued build setting".to_string()),
        }
    }

    /// The canonical stored form: a plain string for zero or one element, an
    /// array for more (the shape Xcode itself writes for multi-value settings).
    fn canonical(elements: &[String]) -> (Setting, Value) {
        match elements {
            [] => (Setting::String(String::new()), Value::String(String::new())),
            [one] => (Setting::String(one.clone()), Value::String(one.clone())),
            many => (
                Setting::List(many.to_vec()),
                Value::Array(many.iter().cloned().map(Value::String).collect()),
            ),
        }
    }
}

/// How one key changes. `Assign` replaces the value outright (its `Vec` is the
/// element list — a repeated `KEY=` on the command line builds it up);
/// `Append` extends the existing value's normalized element list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Assign(Vec<String>),
    Append(Vec<String>),
}

/// One folded `KEY=…`/`KEY+=…` request. The key is the exact `buildSettings`
/// key, conditional suffix included (`CODE_SIGN_IDENTITY[sdk=iphoneos*]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub key: String,
    pub op: Op,
}

/// One applied (or attempted) edit, for the report: what `key` was in
/// `configuration` before and after. `new: None` records an unset; an unset of
/// an absent key yields `old: None, new: None` (the no-op case callers note).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// `None` for project scope, the target name otherwise.
    pub target: Option<String>,
    pub configuration: String,
    pub key: String,
    pub old: Option<Setting>,
    pub new: Option<Setting>,
}

/// The stored settings of one configuration, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSettings {
    pub configuration: String,
    pub settings: Vec<(String, Setting)>,
}

/// Every target name declared in the project, in file order.
#[must_use]
pub fn target_names(root: &Value) -> Vec<String> {
    let Some(objects) = objects(root) else {
        return Vec::new();
    };
    objects
        .iter()
        .filter(|(_, o)| is_target_isa(isa(o)))
        .filter_map(|(_, o)| str_field(o, "name").map(str::to_string))
        .collect()
}

/// The configuration names of a scope, in list order.
///
/// # Errors
/// Returns a message when the tree is malformed or the target is missing.
pub fn configuration_names(root: &Value, scope: &Scope) -> Result<Vec<String>, String> {
    let objects = objects(root).ok_or("pbxproj has no objects dict")?;
    let configs = scope_configurations(root, objects, scope)?;
    Ok(configs.into_iter().map(|(name, _)| name).collect())
}

/// Apply `assignments` to every selected configuration of `scope`.
/// `configurations` empty means all of them; naming an unknown configuration
/// is an error (mutations don't guess). Returns one [`Change`] per
/// (configuration, key).
///
/// # Errors
/// Returns a message when the tree is malformed, the target/configuration is
/// missing, or an existing value has a shape a build setting can't have.
pub fn set(
    root: &mut Value,
    scope: &Scope,
    configurations: &[String],
    assignments: &[Assignment],
) -> Result<Vec<Change>, String> {
    let selected = selected_configurations(root, scope, configurations)?;
    let mut changes = Vec::new();
    for (config_name, config_guid) in &selected {
        for assignment in assignments {
            let settings = build_settings_mut(root, config_guid)?;
            let old = match settings.get(&assignment.key) {
                Some(v) => Some(Setting::from_value(v).map_err(|e| {
                    format!("{}: existing value of {}: {e}", config_name, assignment.key)
                })?),
                None => None,
            };
            let elements = match &assignment.op {
                Op::Assign(values) => values.clone(),
                Op::Append(values) => {
                    let mut merged = old.as_ref().map(Setting::elements).unwrap_or_default();
                    merged.extend(values.iter().cloned());
                    merged
                }
            };
            let (new, value) = Setting::canonical(&elements);
            insert_sorted(settings, &assignment.key, value);
            changes.push(Change {
                target: scope.target().map(str::to_string),
                configuration: config_name.clone(),
                key: assignment.key.clone(),
                old,
                new: Some(new),
            });
        }
    }
    Ok(changes)
}

/// Remove `keys` from every selected configuration of `scope` — true
/// inheritance, not `$(inherited)`. Exact key match only: conditional
/// variants (`KEY[sdk=…]`) are separate keys and are never implicitly swept.
/// An absent key is a recorded no-op (`old: None`), so re-run scripts stay
/// green.
///
/// # Errors
/// Returns a message when the tree is malformed or the target/configuration
/// is missing.
pub fn unset(
    root: &mut Value,
    scope: &Scope,
    configurations: &[String],
    keys: &[String],
) -> Result<Vec<Change>, String> {
    let selected = selected_configurations(root, scope, configurations)?;
    let mut changes = Vec::new();
    for (config_name, config_guid) in &selected {
        for key in keys {
            let settings = build_settings_mut(root, config_guid)?;
            let old = settings
                .remove(key)
                .map(|v| Setting::from_value(&v))
                .transpose()
                .map_err(|e| format!("{config_name}: existing value of {key}: {e}"))?;
            changes.push(Change {
                target: scope.target().map(str::to_string),
                configuration: config_name.clone(),
                key: key.clone(),
                old,
                new: None,
            });
        }
    }
    Ok(changes)
}

/// The stored `buildSettings` of every configuration of `scope`, in file
/// order — the `settings show --raw` payload.
///
/// # Errors
/// Returns a message when the tree is malformed, the target is missing, or a
/// value has a shape a build setting can't have.
pub fn raw(root: &Value, scope: &Scope) -> Result<Vec<ConfigSettings>, String> {
    let objects = objects(root).ok_or("pbxproj has no objects dict")?;
    let configs = scope_configurations(root, objects, scope)?;
    let mut out = Vec::with_capacity(configs.len());
    for (configuration, guid) in configs {
        let mut settings = Vec::new();
        if let Some(dict) = objects
            .get(&guid)
            .and_then(|c| c.get("buildSettings"))
            .and_then(Value::as_dict)
        {
            for (key, value) in dict {
                let setting = Setting::from_value(value)
                    .map_err(|e| format!("{configuration}: value of {key}: {e}"))?;
                settings.push((key.clone(), setting));
            }
        }
        out.push(ConfigSettings {
            configuration,
            settings,
        });
    }
    Ok(out)
}

/// The project-dir-relative xcconfig path backing each selected configuration
/// (`baseConfigurationReference`, or the Xcode-16 anchor + relative-path
/// pair), as `(configuration, path)` pairs — configurations without one are
/// omitted. Callers warn when an edited key is also assigned there: the
/// pbxproj layer outranks the xcconfig, so the edit silently shadows it.
///
/// # Errors
/// Returns a message when the tree is malformed or the target/configuration
/// is missing.
pub fn base_xcconfigs(
    root: &Value,
    scope: &Scope,
    configurations: &[String],
) -> Result<Vec<(String, String)>, String> {
    let objects = objects(root).ok_or("pbxproj has no objects dict")?;
    let selected = selected_configurations(root, scope, configurations)?;
    let mut out = Vec::new();
    for (name, guid) in selected {
        let Some(config) = objects.get(&guid) else {
            continue;
        };
        let path = if let Some(file_ref) = str_field(config, "baseConfigurationReference") {
            crate::project::group_dir(objects, file_ref, std::path::Path::new(""), 0)
        } else if let (Some(anchor), Some(relative)) = (
            str_field(config, "baseConfigurationReferenceAnchor"),
            str_field(config, "baseConfigurationReferenceRelativePath"),
        ) {
            crate::project::group_dir(objects, anchor, std::path::Path::new(""), 0).join(relative)
        } else {
            continue;
        };
        out.push((name, path.to_string_lossy().into_owned()));
    }
    Ok(out)
}

/// Resolve and validate the `(name, guid)` configuration selection for a
/// mutation: all of the scope's configurations, or the named subset.
fn selected_configurations(
    root: &Value,
    scope: &Scope,
    configurations: &[String],
) -> Result<Vec<(String, String)>, String> {
    let objects = objects(root).ok_or("pbxproj has no objects dict")?;
    let all = scope_configurations(root, objects, scope)?;
    if configurations.is_empty() {
        return Ok(all);
    }
    let mut selected = Vec::new();
    for wanted in configurations {
        if let Some(hit) = all.iter().find(|(name, _)| name == wanted) {
            selected.push(hit.clone())
        } else {
            let known: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
            return Err(format!(
                "no configuration named `{wanted}` (project has: {})",
                known.join(", ")
            ));
        }
    }
    Ok(selected)
}

/// The `(name, guid)` pairs of a scope's `XCConfigurationList`, in list order.
fn scope_configurations(
    root: &Value,
    objects: &Dict,
    scope: &Scope,
) -> Result<Vec<(String, String)>, String> {
    let owner_guid = match scope {
        Scope::Project => root
            .as_dict()
            .and_then(|d| d.get("rootObject"))
            .and_then(Value::as_str)
            .ok_or("pbxproj has no rootObject")?
            .to_string(),
        Scope::Target(name) => objects
            .iter()
            .find(|(_, o)| is_target_isa(isa(o)) && str_field(o, "name") == Some(name))
            .map(|(g, _)| g.clone())
            .ok_or_else(|| {
                let known = target_names(root);
                format!(
                    "no target named `{name}` (project has: {})",
                    known.join(", ")
                )
            })?,
    };
    let list_guid = objects
        .get(&owner_guid)
        .and_then(|o| o.get("buildConfigurationList"))
        .and_then(Value::as_str)
        .ok_or("owner has no buildConfigurationList")?;
    let config_guids = objects
        .get(list_guid)
        .and_then(|l| l.get("buildConfigurations"))
        .and_then(Value::as_array)
        .ok_or("configuration list has no buildConfigurations")?;
    let mut configs = Vec::new();
    for guid in config_guids {
        let Some(guid) = guid.as_str() else { continue };
        let Some(name) = objects.get(guid).and_then(|c| str_field(c, "name")) else {
            continue;
        };
        configs.push((name.to_string(), guid.to_string()));
    }
    Ok(configs)
}

/// Mutable access to a configuration's `buildSettings`, created (at its
/// alphabetical position in the configuration object) when absent.
fn build_settings_mut<'a>(root: &'a mut Value, config_guid: &str) -> Result<&'a mut Dict, String> {
    let objects = root
        .as_dict_mut()
        .and_then(|d| d.get_mut("objects"))
        .and_then(Value::as_dict_mut)
        .ok_or("pbxproj has no objects dict")?;
    let config = objects
        .get_mut(config_guid)
        .and_then(Value::as_dict_mut)
        .ok_or_else(|| format!("configuration {config_guid} not in objects"))?;
    if !config.contains_key("buildSettings") {
        insert_sorted(config, "buildSettings", Value::Dict(Dict::new()));
    }
    config
        .get_mut("buildSettings")
        .and_then(Value::as_dict_mut)
        .ok_or_else(|| format!("configuration {config_guid} buildSettings is not a dict"))
}

/// Insert `key` at its alphabetical position (the order Xcode writes object
/// keys), keeping `isa` first when present. An existing key updates in place.
pub(crate) fn insert_sorted(dict: &mut Dict, key: &str, value: Value) {
    if dict.contains_key(key) {
        dict.insert(key.to_string(), value);
        return;
    }
    let entries = std::mem::take(dict);
    let single_line = entries.is_single_line();
    let mut placed = false;
    for (existing_key, existing_value) in entries.iter() {
        if !placed && existing_key != "isa" && existing_key.as_str() > key {
            dict.insert(key.to_string(), value.clone());
            placed = true;
        }
        dict.insert(existing_key.clone(), existing_value.clone());
    }
    if !placed {
        dict.insert(key.to_string(), value);
    }
    dict.set_single_line(single_line);
}

fn objects(root: &Value) -> Option<&Dict> {
    root.as_dict()?.get("objects")?.as_dict()
}

fn isa(obj: &Value) -> &str {
    obj.get("isa").and_then(Value::as_str).unwrap_or("")
}

fn str_field<'a>(obj: &'a Value, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

fn is_target_isa(isa: &str) -> bool {
    matches!(
        isa,
        "PBXNativeTarget" | "PBXAggregateTarget" | "PBXLegacyTarget"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"// !$*UTF8*$!
{
	archiveVersion = 1;
	classes = {
	};
	objectVersion = 77;
	objects = {
		P1 /* Project object */ = {
			isa = PBXProject;
			buildConfigurationList = CLP;
			mainGroup = MG;
			targets = (
				T1,
			);
		};
		MG = {
			isa = PBXGroup;
			children = (
			);
			sourceTree = "<group>";
		};
		T1 /* App */ = {
			isa = PBXNativeTarget;
			buildConfigurationList = CLT;
			name = App;
			productType = "com.apple.product-type.application";
		};
		CLP = {
			isa = XCConfigurationList;
			buildConfigurations = (
				CPD,
				CPR,
			);
		};
		CLT = {
			isa = XCConfigurationList;
			buildConfigurations = (
				CTD,
				CTR,
			);
		};
		CPD = {
			isa = XCBuildConfiguration;
			buildSettings = {
				SDKROOT = macosx;
				SWIFT_VERSION = 5.0;
			};
			name = Debug;
		};
		CPR = {
			isa = XCBuildConfiguration;
			buildSettings = {
				SDKROOT = macosx;
				SWIFT_VERSION = 5.0;
			};
			name = Release;
		};
		CTD = {
			isa = XCBuildConfiguration;
			buildSettings = {
				CODE_SIGN_STYLE = Automatic;
				OTHER_LDFLAGS = "-ObjC -lz";
				PRODUCT_NAME = "$(TARGET_NAME)";
			};
			name = Debug;
		};
		CTR = {
			isa = XCBuildConfiguration;
			buildSettings = {
				CODE_SIGN_STYLE = Automatic;
				PRODUCT_NAME = "$(TARGET_NAME)";
			};
			name = Release;
		};
	};
	rootObject = P1 /* Project object */;
}
"#;

    fn parsed() -> Value {
        crate::pbxproj::parse(FIXTURE).expect("fixture parses")
    }

    fn assign(key: &str, values: &[&str]) -> Assignment {
        Assignment {
            key: key.to_string(),
            op: Op::Assign(values.iter().map(|v| (*v).to_string()).collect()),
        }
    }

    fn raw_of(root: &Value, scope: &Scope, config: &str, key: &str) -> Option<Setting> {
        raw(root, scope)
            .unwrap()
            .into_iter()
            .find(|c| c.configuration == config)?
            .settings
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    #[test]
    fn sets_project_level_on_all_configurations() {
        let mut root = parsed();
        let changes = set(
            &mut root,
            &Scope::Project,
            &[],
            &[assign("ENABLE_HARDENED_RUNTIME", &["YES"])],
        )
        .unwrap();
        assert_eq!(changes.len(), 2);
        assert!(
            changes
                .iter()
                .all(|c| c.target.is_none() && c.old.is_none())
        );
        for config in ["Debug", "Release"] {
            assert_eq!(
                raw_of(&root, &Scope::Project, config, "ENABLE_HARDENED_RUNTIME"),
                Some(Setting::String("YES".into()))
            );
        }
        // Target-level settings are untouched.
        assert_eq!(
            raw_of(
                &root,
                &Scope::Target("App".into()),
                "Debug",
                "ENABLE_HARDENED_RUNTIME"
            ),
            None
        );
    }

    #[test]
    fn narrows_to_named_configuration_and_rejects_unknown() {
        let mut root = parsed();
        set(
            &mut root,
            &Scope::Target("App".into()),
            &["Release".into()],
            &[assign("DEVELOPMENT_TEAM", &["ABC123"])],
        )
        .unwrap();
        assert_eq!(
            raw_of(
                &root,
                &Scope::Target("App".into()),
                "Debug",
                "DEVELOPMENT_TEAM"
            ),
            None
        );
        assert_eq!(
            raw_of(
                &root,
                &Scope::Target("App".into()),
                "Release",
                "DEVELOPMENT_TEAM"
            ),
            Some(Setting::String("ABC123".into()))
        );

        let err = set(
            &mut root,
            &Scope::Project,
            &["Nightly".into()],
            &[assign("A", &["1"])],
        )
        .unwrap_err();
        assert!(
            err.contains("Nightly") && err.contains("Debug, Release"),
            "{err}"
        );
    }

    #[test]
    fn unknown_target_errors_with_known_names() {
        let mut root = parsed();
        let err = set(
            &mut root,
            &Scope::Target("Nope".into()),
            &[],
            &[assign("A", &["1"])],
        )
        .unwrap_err();
        assert!(err.contains("Nope") && err.contains("App"), "{err}");
    }

    #[test]
    fn multi_value_assign_writes_an_array_and_single_stays_string() {
        let mut root = parsed();
        set(
            &mut root,
            &Scope::Target("App".into()),
            &["Debug".into()],
            &[assign(
                "LD_RUNPATH_SEARCH_PATHS",
                &["$(inherited)", "@executable_path/Frameworks"],
            )],
        )
        .unwrap();
        assert_eq!(
            raw_of(
                &root,
                &Scope::Target("App".into()),
                "Debug",
                "LD_RUNPATH_SEARCH_PATHS"
            ),
            Some(Setting::List(vec![
                "$(inherited)".into(),
                "@executable_path/Frameworks".into()
            ]))
        );
    }

    #[test]
    fn append_splits_a_string_value_into_elements() {
        let mut root = parsed();
        let changes = set(
            &mut root,
            &Scope::Target("App".into()),
            &["Debug".into()],
            &[Assignment {
                key: "OTHER_LDFLAGS".into(),
                op: Op::Append(vec!["-framework".into(), "Metal".into()]),
            }],
        )
        .unwrap();
        assert_eq!(changes[0].old, Some(Setting::String("-ObjC -lz".into())));
        assert_eq!(
            raw_of(
                &root,
                &Scope::Target("App".into()),
                "Debug",
                "OTHER_LDFLAGS"
            ),
            Some(Setting::List(vec![
                "-ObjC".into(),
                "-lz".into(),
                "-framework".into(),
                "Metal".into()
            ]))
        );
    }

    #[test]
    fn append_to_absent_key_assigns() {
        let mut root = parsed();
        set(
            &mut root,
            &Scope::Project,
            &["Debug".into()],
            &[Assignment {
                key: "OTHER_SWIFT_FLAGS".into(),
                op: Op::Append(vec!["-DFOO".into()]),
            }],
        )
        .unwrap();
        assert_eq!(
            raw_of(&root, &Scope::Project, "Debug", "OTHER_SWIFT_FLAGS"),
            Some(Setting::String("-DFOO".into()))
        );
    }

    #[test]
    fn conditional_keys_pass_through_and_unset_matches_exactly() {
        let mut root = parsed();
        let scope = Scope::Target("App".into());
        set(
            &mut root,
            &scope,
            &[],
            &[
                assign("CODE_SIGN_IDENTITY", &["Apple Development"]),
                assign(
                    "CODE_SIGN_IDENTITY[sdk=iphoneos*]",
                    &["iPhone Distribution"],
                ),
            ],
        )
        .unwrap();
        // Unsetting the base key leaves the conditional variant alone.
        let changes = unset(&mut root, &scope, &[], &["CODE_SIGN_IDENTITY".into()]).unwrap();
        assert!(changes.iter().all(|c| c.old.is_some()));
        assert_eq!(raw_of(&root, &scope, "Debug", "CODE_SIGN_IDENTITY"), None);
        assert_eq!(
            raw_of(&root, &scope, "Debug", "CODE_SIGN_IDENTITY[sdk=iphoneos*]"),
            Some(Setting::String("iPhone Distribution".into()))
        );
    }

    #[test]
    fn unset_of_absent_key_is_a_recorded_noop() {
        let mut root = parsed();
        let before = crate::pbxproj_writer::serialize(&root, "Fix");
        let changes = unset(&mut root, &Scope::Project, &[], &["NOT_THERE".into()]).unwrap();
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c.old.is_none() && c.new.is_none()));
        let after = crate::pbxproj_writer::serialize(&root, "Fix");
        assert_eq!(before, after, "a no-op unset must not touch the file");
    }

    #[test]
    fn new_keys_land_in_alphabetical_position() {
        let mut root = parsed();
        set(
            &mut root,
            &Scope::Target("App".into()),
            &["Debug".into()],
            &[assign("INFOPLIST_FILE", &["App/Info.plist"])],
        )
        .unwrap();
        let keys: Vec<String> = raw(&root, &Scope::Target("App".into()))
            .unwrap()
            .into_iter()
            .find(|c| c.configuration == "Debug")
            .unwrap()
            .settings
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(
            keys,
            vec![
                "CODE_SIGN_STYLE",
                "INFOPLIST_FILE",
                "OTHER_LDFLAGS",
                "PRODUCT_NAME"
            ]
        );
    }

    #[test]
    fn mutated_tree_round_trips_through_the_writer() {
        let mut root = parsed();
        set(
            &mut root,
            &Scope::Target("App".into()),
            &[],
            &[
                assign("ENABLE_HARDENED_RUNTIME", &["YES"]),
                assign("CODE_SIGN_IDENTITY[sdk=macosx*]", &["-"]),
                assign(
                    "LD_RUNPATH_SEARCH_PATHS",
                    &["$(inherited)", "@executable_path/../Frameworks"],
                ),
            ],
        )
        .unwrap();
        let text = crate::pbxproj_writer::serialize(&root, "Fix");
        let reparsed = crate::pbxproj::parse(&text).expect("mutated pbxproj parses");
        let again = crate::pbxproj_writer::serialize(&reparsed, "Fix");
        assert_eq!(text, again, "serialize → parse → serialize is stable");
        assert!(text.contains("ENABLE_HARDENED_RUNTIME = YES;"));
        assert!(text.contains("\"CODE_SIGN_IDENTITY[sdk=macosx*]\" = \"-\";"));
    }

    #[test]
    fn raw_reports_stored_shapes() {
        let root = parsed();
        assert_eq!(
            raw_of(
                &root,
                &Scope::Target("App".into()),
                "Debug",
                "OTHER_LDFLAGS"
            ),
            Some(Setting::String("-ObjC -lz".into()))
        );
        assert_eq!(
            Setting::String("-ObjC -lz".into()).elements(),
            vec!["-ObjC".to_string(), "-lz".to_string()]
        );
    }
}
