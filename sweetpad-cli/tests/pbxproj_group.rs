//! `pbxproj group attach`/`detach` name the node they touched the way the
//! rest of the fileref and group verbs do, so one command's output feeds the
//! next without a rename in between.

mod common;

use std::path::Path;
use std::process::Command;

use common::TempDir;
use serde_json::Value;

/// `ContentView.swift`'s reference, and the group listing it, in the
/// committed classic fixture.
const CONTENT_VIEW: &str = "85BB78F9ECC9184F5BA8114B";
const SOURCES_APP: &str = "71376D09ABE451C1E73CAAE7";

fn run(args: &[&str], home: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home)
        .output()
        .expect("failed to run the sweetpad binary")
}

fn sweetpad(args: &[&str], home: &Path) -> Value {
    let out = run(args, home);
    assert!(out.status.success(), "{args:?}: {out:?}");
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
    envelope["data"].clone()
}

/// A scratch copy of the committed classic fixture, and its project path.
fn fixture(name: &str) -> (TempDir, String) {
    let dir = TempDir::new(name);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let project = dir.join("SweetpadCIApp.xcodeproj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::copy(
        Path::new(env!("SWEETPAD_LIB_DIR")).join(
            "fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj/project.pbxproj",
        ),
        project.join("project.pbxproj"),
    )
    .unwrap();
    let project = project.to_str().unwrap().to_string();
    (dir, project)
}

/// Both verbs report the child as `address` and the group by its address,
/// even when '--group' named it by directory.
#[test]
fn attach_and_detach_report_the_node_by_address() {
    let (dir, project) = fixture("sweetpad-group-link");
    let project = project.as_str();

    let refs = sweetpad(
        &["pbxproj", "fileref", "list", "--project", project, "--json"],
        &dir,
    );
    assert!(
        refs["refs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["address"] == CONTENT_VIEW && r["group"] == SOURCES_APP),
        "{refs}"
    );

    for (verb, changed) in [("detach", true), ("detach", false), ("attach", true)] {
        let data = sweetpad(
            &[
                "pbxproj",
                "group",
                verb,
                CONTENT_VIEW,
                "--group",
                "Sources/App",
                "--project",
                project,
                "--json",
            ],
            &dir,
        );
        assert_eq!(
            data,
            serde_json::json!({
                "action": verb,
                "address": CONTENT_VIEW,
                "group": SOURCES_APP,
                "changed": changed,
            }),
            "{verb}"
        );
    }
}

/// 'group list' prints the navigator path beside the id and the directory, so
/// every spelling a group argument takes is on the row, and the miss hint that
/// sends people there is true.
#[test]
fn group_list_prints_each_groups_navigator_path() {
    let (dir, project) = fixture("sweetpad-group-list");
    let project = project.as_str();

    let data = sweetpad(
        &["pbxproj", "group", "list", "--project", project, "--json"],
        &dir,
    );
    let groups = data["groups"].as_array().unwrap();
    let navigator_path = |address: &str| {
        groups
            .iter()
            .find(|g| g["address"] == address)
            .map(|g| g["navigatorPath"].clone())
    };
    assert_eq!(
        navigator_path(SOURCES_APP),
        Some(Value::from("Sources/App")),
        "{data}"
    );
    // An organizational group resolves to the project directory, so only its
    // navigator path tells it apart from the root.
    let recovered = groups
        .iter()
        .find(|g| g["name"] == "Recovered References")
        .unwrap();
    assert_eq!(recovered["navigatorPath"], "Recovered References");
    assert_eq!(recovered["resolved"], "");

    let out = run(&["pbxproj", "group", "list", "--project", project], &dir);
    assert!(out.status.success(), "{out:?}");
    let human = String::from_utf8(out.stdout).unwrap();
    assert!(
        human.contains(&format!(
            "{SOURCES_APP}  Sources/App  [Sources/App, 2 child(ren)]"
        )),
        "{human}"
    );
    assert!(
        human.contains("  (navigator root)  [(project root), "),
        "{human}"
    );

    let out = run(
        &[
            "pbxproj",
            "group",
            "attach",
            CONTENT_VIEW,
            "--group",
            "Sources/Nope",
            "--project",
            project,
        ],
        &dir,
    );
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains(
            "no group with id, navigator path, or directory Sources/Nope; \
             'pbxproj group list' shows all three"
        ),
        "{stderr}"
    );
}
