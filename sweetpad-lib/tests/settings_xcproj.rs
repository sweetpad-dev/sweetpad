//! Editing the stored settings layer of a `project.xcproj` document, and the
//! synchronized-folder exception a custom Info.plist needs.
//!
//! The rules these pin were measured against `xcodebuild -showBuildSettings`
//! on Xcode 27: `KEY[config=Debug]` applies under Debug alone, a plain `KEY`
//! shadows every `[config=…]` spelling of itself, and other clauses stay
//! ordinary conditionals. See [`sweetpad_lib::settings_xcproj`].

use sweetpad_lib::settings_xcproj as settings;
use sweetpad_lib::stored_settings::{Assignment, Op, Scope, Setting};
use sweetpad_lib::sync_xcproj::{self, ExcludeOutcome};
use sweetpad_lib::xcproj;

fn document(source: &str) -> xcproj::Value {
    xcproj::parse(source).unwrap_or_else(|e| panic!("parse: {e}"))
}

fn project() -> xcproj::Value {
    document(
        r#"{
  "configurations": [ "Debug", "Release" ],
  "build-settings": {
    "SDKROOT": "iphoneos",
    "SWIFT_OPTIMIZATION_LEVEL[config=Debug]": "-Onone",
    "SWIFT_OPTIMIZATION_LEVEL[config=Release]": "-O",
  },
  "targets": [
    { "name": "App", "product-type": "application" },
  ],
}
"#,
    )
}

fn assign(key: &str, value: &str) -> Assignment {
    Assignment {
        key: key.to_string(),
        op: Op::Assign(vec![value.to_string()]),
    }
}

fn stored(doc: &xcproj::Value, scope: &Scope) -> Vec<String> {
    doc.get("build-settings")
        .or_else(|| {
            scope.target().and_then(|name| {
                doc.get("targets")?
                    .as_array()?
                    .iter()
                    .find(|t| t.get("name").and_then(xcproj::Value::as_str) == Some(name))?
                    .get("build-settings")
            })
        })
        .and_then(xcproj::Value::as_object)
        .map(|o| o.iter().map(|(k, _)| k.to_string()).collect())
        .unwrap_or_default()
}

fn value_of(doc: &xcproj::Value, scope: &Scope, configuration: &str, key: &str) -> Option<String> {
    settings::raw(doc, scope)
        .unwrap()
        .into_iter()
        .find(|c| c.configuration == configuration)?
        .settings
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.display())
}

#[test]
fn a_key_every_configuration_agrees_on_is_stored_without_a_clause() {
    let mut doc = project();
    let changes = settings::set(
        &mut doc,
        &Scope::Project,
        &[],
        &[assign("SWIFT_VERSION", "6.0")],
    )
    .unwrap();

    assert_eq!(changes.len(), 2, "one change per configuration");
    assert!(stored(&doc, &Scope::Project).contains(&"SWIFT_VERSION".to_string()));
    assert_eq!(
        value_of(&doc, &Scope::Project, "Release", "SWIFT_VERSION").as_deref(),
        Some("6.0")
    );
}

/// Naming one configuration has to keep the others on the value they had, and
/// the plain key cannot stay — it would shadow the new conditional one.
#[test]
fn naming_one_configuration_expands_a_plain_key() {
    let mut doc = project();
    settings::set(
        &mut doc,
        &Scope::Project,
        &["Debug".to_string()],
        &[assign("SDKROOT", "macosx")],
    )
    .unwrap();

    let keys = stored(&doc, &Scope::Project);
    assert!(!keys.contains(&"SDKROOT".to_string()), "{keys:?}");
    assert_eq!(
        value_of(&doc, &Scope::Project, "Debug", "SDKROOT").as_deref(),
        Some("macosx")
    );
    assert_eq!(
        value_of(&doc, &Scope::Project, "Release", "SDKROOT").as_deref(),
        Some("iphoneos")
    );
}

/// The reverse: giving every configuration the same value collapses the
/// per-configuration keys back into one.
#[test]
fn agreeing_on_a_split_key_collapses_it() {
    let mut doc = project();
    settings::set(
        &mut doc,
        &Scope::Project,
        &[],
        &[assign("SWIFT_OPTIMIZATION_LEVEL", "-O")],
    )
    .unwrap();

    let keys = stored(&doc, &Scope::Project);
    assert!(
        keys.contains(&"SWIFT_OPTIMIZATION_LEVEL".to_string()),
        "{keys:?}"
    );
    assert!(
        !keys
            .iter()
            .any(|k| k.starts_with("SWIFT_OPTIMIZATION_LEVEL[")),
        "{keys:?}"
    );
}

#[test]
fn unsetting_one_configuration_leaves_the_others() {
    let mut doc = project();
    let changes = settings::unset(
        &mut doc,
        &Scope::Project,
        &["Debug".to_string()],
        &["SWIFT_OPTIMIZATION_LEVEL".to_string()],
    )
    .unwrap();

    assert_eq!(
        changes[0].old,
        Some(Setting::String("-Onone".into())),
        "the report names what was there"
    );
    assert_eq!(
        value_of(&doc, &Scope::Project, "Debug", "SWIFT_OPTIMIZATION_LEVEL"),
        None
    );
    assert_eq!(
        value_of(&doc, &Scope::Project, "Release", "SWIFT_OPTIMIZATION_LEVEL").as_deref(),
        Some("-O")
    );
}

#[test]
fn unsetting_an_absent_key_is_a_recorded_no_op() {
    let mut doc = project();
    let changes =
        settings::unset(&mut doc, &Scope::Project, &[], &["NOT_THERE".to_string()]).unwrap();
    assert!(changes.iter().all(|c| c.old.is_none() && c.new.is_none()));
}

/// An append folds into the value that configuration already has, so a split
/// key stays split when the two sides differ afterwards.
#[test]
fn appending_extends_each_configuration_from_its_own_value() {
    let mut doc = project();
    settings::set(
        &mut doc,
        &Scope::Project,
        &[],
        &[Assignment {
            key: "SWIFT_OPTIMIZATION_LEVEL".into(),
            op: Op::Append(vec!["-extra".into()]),
        }],
    )
    .unwrap();

    assert_eq!(
        value_of(&doc, &Scope::Project, "Debug", "SWIFT_OPTIMIZATION_LEVEL").as_deref(),
        Some("-Onone -extra")
    );
    assert_eq!(
        value_of(&doc, &Scope::Project, "Release", "SWIFT_OPTIMIZATION_LEVEL").as_deref(),
        Some("-O -extra")
    );
}

/// A clause that isn't `config=` names a different setting, not a
/// configuration of the same one.
#[test]
fn another_clause_stays_part_of_the_key() {
    let mut doc = project();
    settings::set(
        &mut doc,
        &Scope::Project,
        &["Debug".to_string()],
        &[assign("OTHER_LDFLAGS[sdk=iphoneos*]", "-lz")],
    )
    .unwrap();

    let keys = stored(&doc, &Scope::Project);
    assert!(
        keys.contains(&"OTHER_LDFLAGS[sdk=iphoneos*][config=Debug]".to_string()),
        "{keys:?}"
    );
}

#[test]
fn a_target_scope_edits_only_that_target() {
    let mut doc = project();
    settings::set(
        &mut doc,
        &Scope::Target("App".into()),
        &[],
        &[assign("PRODUCT_NAME", "App")],
    )
    .unwrap();

    assert_eq!(
        value_of(&doc, &Scope::Target("App".into()), "Debug", "PRODUCT_NAME").as_deref(),
        Some("App")
    );
    assert_eq!(
        value_of(&doc, &Scope::Project, "Debug", "PRODUCT_NAME"),
        None
    );
}

#[test]
fn an_unknown_configuration_or_target_is_an_error() {
    let mut doc = project();
    let err = settings::set(
        &mut doc,
        &Scope::Project,
        &["Beta".to_string()],
        &[assign("SDKROOT", "macosx")],
    )
    .unwrap_err();
    assert!(err.contains("no configuration named Beta"), "{err}");

    let err = settings::set(
        &mut doc,
        &Scope::Target("Nope".into()),
        &[],
        &[assign("SDKROOT", "macosx")],
    )
    .unwrap_err();
    assert!(err.contains("no target named Nope"), "{err}");
}

#[test]
fn an_info_plist_inside_a_synchronized_folder_is_excepted() {
    let mut doc = document(
        r#"{
  "configurations": [ "Debug" ],
  "files": [
    {
      "kind": "group",
      "path": "Sources",
      "children": [
        { "kind": "folder", "path": "Shared", "target-membership": [ "App" ] },
      ],
    },
  ],
  "targets": [
    { "name": "App", "product-type": "application" },
  ],
}
"#,
    );

    let outcome =
        sync_xcproj::ensure_infoplist_exception(&mut doc, "App", "Sources/Shared/Info.plist")
            .unwrap();
    assert_eq!(
        outcome,
        Some(ExcludeOutcome::Added {
            root_dir: "Sources/Shared".into(),
            exception: "Info.plist".into(),
        })
    );

    // Re-running says so rather than adding it twice.
    let again =
        sync_xcproj::ensure_infoplist_exception(&mut doc, "App", "Sources/Shared/Info.plist")
            .unwrap();
    assert_eq!(
        again,
        Some(ExcludeOutcome::AlreadyExcluded {
            root_dir: "Sources/Shared".into(),
            exception: "Info.plist".into(),
        })
    );

    // A plist outside every folder of the target needs no exception.
    assert_eq!(
        sync_xcproj::ensure_infoplist_exception(&mut doc, "App", "Elsewhere/Info.plist").unwrap(),
        None
    );
}
