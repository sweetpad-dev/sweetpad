//! The typed view over `project.xcproj`, read and edited against the
//! Xcode-authored fixtures in `fixtures/_xcproj`.

use std::fs;
use std::path::PathBuf;

use sweetpad_lib::schema_xcproj::{
    self as schema, Entry, Scope, Setting, configuration_condition, split_key,
};
use sweetpad_lib::xcproj;

fn fixture(name: &str) -> xcproj::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/_xcproj")
        .join(format!("{name}.xcodeproj/project.xcproj"));
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    xcproj::parse(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn setting(entries: &[Entry], key: &str) -> Option<String> {
    entries
        .iter()
        .find(|e| e.key() == key)
        .map(|e| e.value.display())
}

#[test]
fn reads_targets_and_configurations() {
    let doc = fixture("SweetpadCIApp");

    assert_eq!(
        schema::target_names(&doc),
        ["SweetpadCIApp", "SweetpadCIAppTests", "SweetpadCIMac"]
    );
    assert_eq!(schema::configuration_names(&doc), ["Debug", "Release"]);
    assert_eq!(schema::default_configuration(&doc), Some("Debug"));
}

/// A configuration is a bare string until it carries an xcconfig, and only
/// then does it become an object with a `name`.
#[test]
fn reads_configurations_in_either_form() {
    let doc = fixture("NetNewsWire");

    let names = schema::configuration_names(&doc);
    assert!(names.contains(&"Debug".to_string()), "{names:?}");
    assert!(names.contains(&"Release".to_string()), "{names:?}");
}

#[test]
fn reads_project_and_target_settings() {
    let doc = fixture("SweetpadCIApp");

    let project = schema::raw(&doc, &Scope::Project).unwrap();
    assert_eq!(setting(&project, "SWIFT_VERSION").as_deref(), Some("5.0"));

    let target = schema::raw(&doc, &Scope::Target("SweetpadCIMac".into())).unwrap();
    assert_eq!(setting(&target, "SDKROOT").as_deref(), Some("macosx"));
    // A list-valued setting keeps its elements.
    assert_eq!(
        target
            .iter()
            .find(|e| e.name == "LD_RUNPATH_SEARCH_PATHS")
            .unwrap()
            .value
            .elements(),
        ["$(inherited)", "@executable_path/../Frameworks"]
    );
}

/// What `pbxproj` spelled as one settings map per configuration, this format
/// spells as a condition on the key.
#[test]
fn per_configuration_settings_are_conditions_on_the_key() {
    let doc = fixture("SweetpadCIApp");
    let project = schema::raw(&doc, &Scope::Project).unwrap();

    let optimization: Vec<_> = project
        .iter()
        .filter(|e| e.name == "SWIFT_OPTIMIZATION_LEVEL")
        .map(|e| (e.condition.clone().unwrap_or_default(), e.value.display()))
        .collect();
    assert_eq!(
        optimization,
        [
            ("[config=Debug]".to_string(), "-Onone".to_string()),
            ("[config=Release]".to_string(), "-O".to_string()),
        ]
    );

    // An unconditional setting has no condition, and rejoining gives the key
    // back exactly.
    let version = project.iter().find(|e| e.name == "SWIFT_VERSION").unwrap();
    assert_eq!(version.condition, None);
    assert_eq!(version.key(), "SWIFT_VERSION");
}

#[test]
fn splits_keys_into_name_and_condition() {
    assert_eq!(split_key("SWIFT_VERSION"), ("SWIFT_VERSION", None));
    assert_eq!(
        split_key("SWIFT_OPTIMIZATION_LEVEL[config=Debug]"),
        ("SWIFT_OPTIMIZATION_LEVEL", Some("[config=Debug]"))
    );
    // Several clauses stay together, since they are one condition.
    assert_eq!(
        split_key("OTHER_LDFLAGS[sdk=iphoneos*][arch=arm64]"),
        ("OTHER_LDFLAGS", Some("[sdk=iphoneos*][arch=arm64]"))
    );
    // An unterminated bracket is part of the name rather than a condition.
    assert_eq!(split_key("WEIRD[oops"), ("WEIRD[oops", None));
}

#[test]
fn reads_the_xcconfig_a_configuration_is_based_on() {
    let doc = fixture("NetNewsWire");
    let target = Scope::Target("NetNewsWire".into());

    let file = schema::base_xcconfig(&doc, &target, "Debug")
        .expect("NetNewsWire specializes its Debug configuration");
    assert!(
        file.get("relative-path")
            .and_then(xcproj::Value::as_str)
            .is_some_and(|p| p.ends_with(".xcconfig")),
        "{file:?}"
    );

    assert_eq!(schema::base_xcconfig(&doc, &target, "Nonexistent"), None);
}

/// An edit changes the one key it names and leaves the document otherwise
/// byte-identical, which is the whole point of editing the tree in place.
#[test]
fn setting_a_value_touches_one_line() {
    let before = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/_xcproj/SweetpadCIApp.xcodeproj/project.xcproj"),
    )
    .unwrap();
    let mut doc = xcproj::parse(&before).unwrap();

    schema::set(
        &mut doc,
        &Scope::Target("SweetpadCIMac".into()),
        "SWEETPAD_PROBE",
        None,
        &Setting::String("1".into()),
    )
    .unwrap();
    let after = xcproj::serialize(&doc);

    let added: Vec<&str> = after
        .lines()
        .filter(|l| !before.lines().any(|b| b == *l))
        .collect();
    assert_eq!(added, ["        \"SWEETPAD_PROBE\": \"1\","]);
    assert_eq!(before.lines().count() + 1, after.lines().count());
}

/// Xcode keeps `build-settings` sorted, so a new key lands where Xcode would
/// have put it.
#[test]
fn a_new_key_lands_in_alphabetical_order() {
    let mut doc = fixture("SweetpadCIApp");
    let scope = Scope::Target("SweetpadCIMac".into());

    schema::set(
        &mut doc,
        &scope,
        "GENERATE_INFOPLIST_FILE",
        None,
        &Setting::String("NO".into()),
    )
    .unwrap();
    schema::set(
        &mut doc,
        &scope,
        "AAA_FIRST",
        None,
        &Setting::String("1".into()),
    )
    .unwrap();
    schema::set(
        &mut doc,
        &scope,
        "ZZZ_LAST",
        None,
        &Setting::String("1".into()),
    )
    .unwrap();

    let keys: Vec<String> = schema::raw(&doc, &scope)
        .unwrap()
        .iter()
        .map(Entry::key)
        .collect();
    assert_eq!(keys.first().map(String::as_str), Some("AAA_FIRST"));
    assert_eq!(keys.last().map(String::as_str), Some("ZZZ_LAST"));
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "insertion left the map unsorted");
    // Replacing an existing key keeps its place rather than adding a second.
    assert_eq!(
        keys.iter()
            .filter(|k| *k == "GENERATE_INFOPLIST_FILE")
            .count(),
        1
    );
    assert_eq!(
        setting(
            &schema::raw(&doc, &scope).unwrap(),
            "GENERATE_INFOPLIST_FILE"
        )
        .as_deref(),
        Some("NO")
    );
}

#[test]
fn a_conditional_setting_round_trips_through_set() {
    let mut doc = fixture("SweetpadCIApp");
    let scope = Scope::Project;
    let condition = configuration_condition("Release");

    schema::set(
        &mut doc,
        &scope,
        "OTHER_SWIFT_FLAGS",
        Some(&condition),
        &Setting::List(vec![
            "-warnings-as-errors".into(),
            "-strict-concurrency".into(),
        ]),
    )
    .unwrap();

    let entry = schema::raw(&doc, &scope)
        .unwrap()
        .into_iter()
        .find(|e| e.name == "OTHER_SWIFT_FLAGS")
        .unwrap();
    assert_eq!(entry.condition.as_deref(), Some("[config=Release]"));
    assert_eq!(
        entry.value.elements(),
        ["-warnings-as-errors", "-strict-concurrency"]
    );
    // A multi-element value stores as an array, a single one as a string.
    assert!(matches!(entry.value, Setting::List(_)));
}

#[test]
fn unset_removes_a_key_and_reports_whether_it_was_there() {
    let mut doc = fixture("SweetpadCIApp");
    let scope = Scope::Target("SweetpadCIMac".into());

    assert!(schema::unset(&mut doc, &scope, "SDKROOT", None).unwrap());
    assert!(!schema::unset(&mut doc, &scope, "SDKROOT", None).unwrap());
    assert_eq!(
        setting(&schema::raw(&doc, &scope).unwrap(), "SDKROOT"),
        None
    );

    assert!(
        schema::unset(
            &mut doc,
            &Scope::Project,
            "SWIFT_OPTIMIZATION_LEVEL",
            Some("[config=Debug]")
        )
        .unwrap()
    );
    let names: Vec<String> = schema::raw(&doc, &Scope::Project)
        .unwrap()
        .iter()
        .map(Entry::key)
        .collect();
    assert!(!names.contains(&"SWIFT_OPTIMIZATION_LEVEL[config=Debug]".to_string()));
    assert!(names.contains(&"SWIFT_OPTIMIZATION_LEVEL[config=Release]".to_string()));
}

#[test]
fn an_unknown_target_is_an_error_rather_than_a_silent_no_op() {
    let mut doc = fixture("SweetpadCIApp");
    let missing = Scope::Target("Nope".into());

    assert!(schema::raw(&doc, &missing).is_err());
    assert!(schema::unset(&mut doc, &missing, "SDKROOT", None).is_err());
    assert!(schema::set(&mut doc, &missing, "A", None, &Setting::String("1".into())).is_err());
}
