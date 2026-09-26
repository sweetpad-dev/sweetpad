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

fn sweetpad(args: &[&str], home: &Path) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home)
        .output()
        .expect("failed to run the sweetpad binary");
    assert!(out.status.success(), "{args:?}: {out:?}");
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
    envelope["data"].clone()
}

/// Both verbs report the child as `address` and the group by its address,
/// even when '--group' named it by directory.
#[test]
fn attach_and_detach_report_the_node_by_address() {
    let dir = TempDir::new("sweetpad-group-link");
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
    let project = project.to_str().unwrap();

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
