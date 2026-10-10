//! `sweetpad build --engine swift-build` end to end, against a stub `swbuild`
//! that checks it was handed a PIF and build parameters, then replays a canned
//! message stream. Nothing compiles, and no Swift Build or Darwin SDK is
//! needed: on macOS the engine takes the SDKs from Xcode.

#![cfg(target_os = "macos")]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::TempDir;
use serde_json::Value;

fn tmp(tag: &str) -> TempDir {
    let dir = TempDir::new(&format!("sweetpad-swbuild-{tag}"));
    // Stop walk-up discovery at this directory.
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

/// A committed project with a shared `SweetpadCIMac` scheme, so target
/// resolution settles from flags alone.
fn project() -> PathBuf {
    Path::new(env!("SWEETPAD_LIB_DIR"))
        .join("fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj")
}

/// A `swbuild` in `dir` that fails unless its parameters file and PIF exist
/// and it was asked for the scheme's target, then prints `messages` the way
/// swbuild frames them (byte count, then the JSON) and exits with `status`.
fn write_stub(dir: &Path, messages: &[&str], status: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let framed = messages.iter().fold(String::new(), |mut out, m| {
        use std::fmt::Write;
        let _ = write!(out, "{}\n{m}\n", m.len());
        out
    });
    let transcript = dir.join("messages.log");
    std::fs::write(&transcript, framed).unwrap();
    let stub = dir.join("swbuild");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\n\
             params=; target=; last=\n\
             while [ $# -gt 0 ]; do\n\
             case \"$1\" in\n\
             --buildParametersFile) params=\"$2\"; shift;;\n\
             --target) target=\"$2\"; shift;;\n\
             esac\n\
             last=\"$1\"; shift\n\
             done\n\
             [ -f \"$params\" ] || {{ echo \"error: no parameters file '$params'\"; exit 64; }}\n\
             [ -f \"$last\" ] || {{ echo \"error: no PIF '$last'\"; exit 64; }}\n\
             [ \"$target\" = SweetpadCIMac ] || {{ echo \"error: target '$target'\"; exit 64; }}\n\
             cat '{}'\nexit {status}\n",
            transcript.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    stub
}

/// `sweetpad build --engine swift-build` for the fixture against a stub
/// replaying `messages`.
fn build(tag: &str, messages: &[&str], status: i32, extra: &[&str]) -> Output {
    let home = tmp(&format!("{tag}-home"));
    let cwd = tmp(&format!("{tag}-cwd"));
    let stub = write_stub(&cwd, messages, status);
    let developer_dir = cwd.join("Developer");
    std::fs::create_dir_all(&developer_dir).unwrap();
    let project = project();
    let mut args = vec![
        "build",
        "--engine",
        "swift-build",
        "--project",
        project.to_str().unwrap(),
        "--scheme",
        "SweetpadCIMac",
        "--configuration",
        "Debug",
        "--destination",
        "platform=macOS",
        "--non-interactive",
    ];
    args.extend_from_slice(extra);
    Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(&args)
        .current_dir(&cwd)
        .env("HOME", &home)
        .env("CFFIXED_USER_HOME", &home)
        .env("XDG_STATE_HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("XDG_CACHE_HOME", &home)
        .env("DEVELOPER_DIR", &developer_dir)
        .env("SWEETPAD_SWBUILD", &stub)
        .env_remove("NO_COLOR")
        .output()
        .expect("failed to run the sweetpad binary")
}

const SUCCEEDED: &[&str] = &[
    r#"{"kind":"buildStarted","baseDirectory":"/src","derivedDataPath":"/dd"}"#,
    r#"{"kind":"taskStarted","id":1,"ruleInfo":"SwiftCompile normal arm64 /src/App.swift","executionDescription":"Compile App.swift (arm64)"}"#,
    r#"{"kind":"warning","message":"variable 'x' was never used","location":{"locationType":"path","path":"/src/App.swift","line":3,"column":9},"locationContext":{"locationType":"task","taskID":1,"targetID":0},"childDiagnostics":[]}"#,
    r#"{"kind":"taskComplete","id":1,"result":"success"}"#,
    r#"{"kind":"taskStarted","id":2,"ruleInfo":"Ld /dd/App normal","executionDescription":"Link App (arm64)"}"#,
    r#"{"kind":"buildCompleted","result":"ok"}"#,
];

const FAILED: &[&str] = &[
    r#"{"kind":"taskStarted","id":1,"ruleInfo":"SwiftCompile normal arm64 /src/App.swift","executionDescription":"Compile App.swift (arm64)"}"#,
    r#"{"kind":"error","message":"cannot find 'y' in scope","location":{"locationType":"path","path":"/src/App.swift","line":4,"column":5},"locationContext":{"locationType":"task","taskID":1,"targetID":0},"childDiagnostics":[]}"#,
    r#"{"kind":"taskComplete","id":1,"result":"failed"}"#,
    r#"{"kind":"buildCompleted","result":"failed"}"#,
];

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_successful_build_reports_like_an_xcodebuild_one() {
    let output = build("ok-json", SUCCEEDED, 0, &["--json"]);
    assert!(output.status.success(), "{output:?}");
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["warnings"], 1);
    assert_eq!(envelope["data"]["errors"], 0);
    assert_eq!(envelope["data"]["scheme"], "SweetpadCIMac");
}

#[test]
fn ndjson_streams_tasks_and_diagnostics_then_one_result() {
    let output = build("ok-ndjson", SUCCEEDED, 0, &["-o", "ndjson"]);
    assert!(output.status.success(), "{output:?}");
    let events: Vec<Value> = stdout(&output)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(events[0]["event"], "task");
    assert_eq!(events[0]["kind"], "compile");
    assert_eq!(events[0]["name"], "App.swift");
    assert_eq!(events[1]["event"], "diagnostic");
    assert_eq!(events[1]["location"], "/src/App.swift:3:9");
    assert_eq!(events[2]["kind"], "link");
    let results: Vec<&Value> = events.iter().filter(|e| e["event"] == "result").collect();
    assert_eq!(results.len(), 1);
    assert_eq!(events.last().unwrap()["event"], "result");
}

#[test]
fn a_failed_build_exits_3_with_its_errors() {
    let output = build("failed", FAILED, 1, &[]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let out = stdout(&output);
    assert!(out.contains("/src/App.swift:4:5"), "{out}");
    assert!(out.contains("cannot find 'y' in scope"), "{out}");
    assert!(out.contains("Build failed"), "{out}");
}

#[test]
fn show_command_names_the_inputs_without_building() {
    let output = build("show", SUCCEEDED, 0, &["--show-command"]);
    assert!(output.status.success(), "{output:?}");
    let out = stdout(&output);
    assert!(out.starts_with("swbuild build"), "{out}");
    assert!(out.contains("--buildParametersFile"), "{out}");
    assert!(out.contains("--target SweetpadCIMac"), "{out}");
    assert!(out.contains("workspace.pif.json"), "{out}");
}

#[test]
fn xcodebuild_flags_after_the_double_dash_are_refused() {
    let output = build(
        "refused",
        SUCCEEDED,
        0,
        &["--", "-allowProvisioningUpdates"],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(
        err.contains("'-allowProvisioningUpdates' is an xcodebuild flag"),
        "{err}"
    );
}
