//! A scheme builds with the configuration its own `.xcscheme` names, the way
//! Xcode does, and a remembered configuration belongs to one scheme (#342).
//!
//! The fixture is a project with three configurations and no `Debug`, so
//! nothing here can pass by falling through to the `Debug` default.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::TempDir;
use serde_json::Value;

const DESTINATION: &str = "platform=iOS Simulator,name=iPhone 17";

const PBXPROJ: &str = r#"// !$*UTF8*$!
{
	archiveVersion = 1;
	classes = {
	};
	objectVersion = 77;
	objects = {
		C1 = {isa = XCBuildConfiguration; buildSettings = {}; name = "TEST-Debug"; };
		C2 = {isa = XCBuildConfiguration; buildSettings = {}; name = UAT; };
		C3 = {isa = XCBuildConfiguration; buildSettings = {}; name = Release; };
		L1 = {isa = XCConfigurationList; buildConfigurations = (C1, C2, C3, ); defaultConfigurationIsVisible = 0; defaultConfigurationName = Release; };
		G1 = {isa = PBXGroup; children = (); sourceTree = "<group>"; };
		P1 = {isa = PBXProject; buildConfigurationList = L1; mainGroup = G1; targets = (); };
	};
	rootObject = P1;
}
"#;

/// A project whose schemes each name their own Run and Test configurations.
fn project() -> TempDir {
    let dir = TempDir::new("sweetpad-scheme-configuration");
    // A `.git` marker stops walk-up discovery at this directory.
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let schemes = dir.join("App.xcodeproj/xcshareddata/xcschemes");
    std::fs::create_dir_all(&schemes).unwrap();
    std::fs::write(dir.join("App.xcodeproj/project.pbxproj"), PBXPROJ).unwrap();
    for (name, run, test) in [
        ("App TEST", "TEST-Debug", "TEST-Debug"),
        ("App UAT", "UAT", "UAT"),
        ("App Prod", "Release", "TEST-Debug"),
        ("App Stale", "Staging", "Staging"),
    ] {
        std::fs::write(
            schemes.join(format!("{name}.xcscheme")),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="1600" version="1.7">
   <BuildAction/>
   <TestAction buildConfiguration="{test}"/>
   <LaunchAction buildConfiguration="{run}"/>
</Scheme>
"#
            ),
        )
        .unwrap();
    }
    std::fs::create_dir_all(dir.join("home")).unwrap();
    dir
}

/// Run the binary in `dir` with an isolated home, so state stays in the fixture.
fn sweetpad(dir: &Path, args: &[&str]) -> Output {
    let home = dir.join("home");
    Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(args)
        .current_dir(dir)
        .env("HOME", &home)
        .env("CFFIXED_USER_HOME", &home)
        .env("XDG_STATE_HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("XDG_CACHE_HOME", &home)
        .env("TMPDIR", &home)
        .env_remove("SWEETPAD_NONINTERACTIVE")
        .output()
        .expect("failed to run the sweetpad binary")
}

/// The `-configuration` a dry-run `build` or `test` would pass to xcodebuild.
fn previewed_configuration(dir: &Path, command: &str, scheme: &str) -> String {
    preview(dir, &[command, "--scheme", scheme])
}

fn preview(dir: &Path, args: &[&str]) -> String {
    let mut args = args.to_vec();
    args.extend(["--show-command", "--destination", DESTINATION]);
    let out = sweetpad(dir, &args);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let words: Vec<&str> = stdout.split_whitespace().collect();
    let at = words
        .iter()
        .position(|w| *w == "-configuration")
        .unwrap_or_else(|| panic!("no -configuration in {stdout}"));
    words[at + 1].trim_matches('\'').to_string()
}

/// `status --json`'s configuration row for the scheme.
fn status_configuration(dir: &Path, scheme: &str) -> (Value, Value) {
    let out = sweetpad(dir, &["status", "--json", "--scheme", scheme]);
    let json: Value = serde_json::from_slice(&out.stdout).expect("status --json");
    let row = &json["data"]["context"]["configuration"];
    (row["value"].clone(), row["source"].clone())
}

#[test]
fn each_scheme_builds_with_its_own_configuration() {
    let dir = project();
    assert_eq!(
        previewed_configuration(&dir, "build", "App TEST"),
        "TEST-Debug"
    );
    assert_eq!(previewed_configuration(&dir, "build", "App UAT"), "UAT");
    assert_eq!(
        previewed_configuration(&dir, "build", "App Prod"),
        "Release"
    );
    // Tests take the scheme's Test action, not its Run action.
    assert_eq!(
        previewed_configuration(&dir, "test", "App Prod"),
        "TEST-Debug"
    );
    assert_eq!(
        status_configuration(&dir, "App UAT"),
        (Value::from("UAT"), Value::from("scheme"))
    );
}

#[test]
fn a_remembered_configuration_belongs_to_one_scheme() {
    let dir = project();
    // With no scheme there is nothing to remember it for.
    let out = sweetpad(&dir, &["context", "set", "configuration", "Release"]);
    assert!(!out.status.success());

    assert!(
        sweetpad(&dir, &["context", "set", "scheme", "App UAT"])
            .status
            .success()
    );
    assert!(
        sweetpad(&dir, &["context", "set", "configuration", "Release"])
            .status
            .success()
    );
    assert_eq!(previewed_configuration(&dir, "build", "App UAT"), "Release");
    assert_eq!(
        status_configuration(&dir, "App UAT"),
        (Value::from("Release"), Value::from("remembered"))
    );
    // Another scheme keeps its own.
    assert_eq!(
        previewed_configuration(&dir, "build", "App TEST"),
        "TEST-Debug"
    );

    assert!(
        sweetpad(&dir, &["context", "remove", "configuration"])
            .status
            .success()
    );
    assert_eq!(previewed_configuration(&dir, "build", "App UAT"), "UAT");
}

/// The single project-wide configuration older versions remembered is the one
/// that pinned every scheme, so it is no longer read.
#[test]
fn the_project_wide_configuration_older_versions_remembered_is_ignored() {
    let dir = project();
    let key = std::fs::canonicalize(dir.join("App.xcodeproj")).unwrap();
    let state = dir.join("home/sweetpad");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(
        state.join("state.toml"),
        format!(
            "[projects.{key:?}]\nconfiguration = \"TEST-Debug\"\n",
            key = key.display().to_string()
        ),
    )
    .unwrap();
    assert_eq!(previewed_configuration(&dir, "build", "App UAT"), "UAT");
}

/// A scheme naming a configuration the project lacks can't be built with it,
/// so it doesn't count; with no `Debug` either, a non-interactive run says a
/// configuration is needed instead of guessing.
#[test]
fn a_stale_scheme_configuration_is_skipped() {
    let dir = project();
    let out = sweetpad(
        &dir,
        &[
            "build",
            "--show-command",
            "--scheme",
            "App Stale",
            "--destination",
            DESTINATION,
        ],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no configuration specified"));
    // A flag still names one.
    assert_eq!(
        preview(
            &dir,
            &["build", "--scheme", "App Stale", "--configuration", "UAT"]
        ),
        "UAT"
    );
}
