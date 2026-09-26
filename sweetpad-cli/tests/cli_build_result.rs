//! What `sweetpad build` reports once xcodebuild exits, per output mode. A stub
//! xcodebuild replays a canned transcript, so the result is checked end to end
//! without compiling anything.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn tmp(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sweetpad-build-{tag}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
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

/// `sweetpad build` in `mode` against a stub xcodebuild that prints
/// `transcript` and exits with `status`.
fn build_with_stub(tag: &str, transcript: &str, status: i32, mode: &[&str]) -> Output {
    use std::os::unix::fs::PermissionsExt;

    let home = tmp(&format!("{tag}-home"));
    let cwd = tmp(&format!("{tag}-cwd"));
    let bin = cwd.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = cwd.join("transcript.log");
    std::fs::write(&log, transcript).unwrap();
    let stub = bin.join("xcodebuild");
    std::fs::write(
        &stub,
        format!("#!/bin/sh\ncat '{}'\nexit {status}\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    // An empty developer dir keeps the `productPath` lookup from loading the
    // installed Xcode's specs, which costs seconds and is not under test.
    let developer_dir = cwd.join("Developer");
    std::fs::create_dir_all(&developer_dir).unwrap();

    let project = project();
    let mut args = vec![
        "build",
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
    args.extend_from_slice(mode);
    Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(&args)
        .current_dir(&cwd)
        .env("HOME", &home)
        .env("XDG_STATE_HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("XDG_CACHE_HOME", &home)
        .env("DEVELOPER_DIR", &developer_dir)
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .output()
        .expect("failed to run the sweetpad binary")
}

const WARNED: &str = "\
CompileSwift normal arm64 /src/App/ContentView.swift (in target 'SweetpadCIMac' from project 'SweetpadCIApp')
/src/App/ContentView.swift:3:9: warning: variable 'x' was never used
/src/App/ContentView.swift:4:9: warning: variable 'y' was never used
** BUILD SUCCEEDED **
";

/// `-o json` and `-o ndjson` render the same `BuildReport`, so a caller
/// switching modes gets the same fields, counts included.
#[test]
fn json_and_ndjson_report_the_same_build_result() {
    let json = build_with_stub("json", WARNED, 0, &["-o", "json"]);
    assert!(json.status.success(), "{json:?}");
    let envelope: Value = serde_json::from_slice(&json.stdout).unwrap();
    let from_json = &envelope["data"];

    let ndjson = build_with_stub("ndjson", WARNED, 0, &["-o", "ndjson"]);
    assert!(ndjson.status.success(), "{ndjson:?}");
    let stdout = String::from_utf8(ndjson.stdout).unwrap();
    let result: Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    assert_eq!(result["event"], "result");
    let from_ndjson = &result["data"];

    let keys = |v: &Value| -> Vec<String> { v.as_object().unwrap().keys().cloned().collect() };
    assert_eq!(keys(from_json), keys(from_ndjson));
    for data in [from_json, from_ndjson] {
        assert_eq!(data["errors"], 0, "{data}");
        assert_eq!(data["warnings"], 2, "{data}");
        assert!(data["durationMs"].is_u64(), "{data}");
    }
}

const BROKEN: &str = "\
CompileSwift normal arm64 /src/App/ContentView.swift (in target 'SweetpadCIMac' from project 'SweetpadCIApp')
/src/App/ContentView.swift:17:19: error: cannot find 'undefinedSymbol' in scope
** BUILD FAILED **

The following build commands failed:
\tCompileSwift normal arm64 /src/App/ContentView.swift (in target 'SweetpadCIMac' from project 'SweetpadCIApp')
(1 failure)
";

/// The streamed log already names the compile error and closes on `✗ Build
/// failed`, so human output ends there; the exit code still says it failed.
#[test]
fn a_streamed_compile_error_ends_human_output_at_the_banner() {
    let out = build_with_stub("broken", BROKEN, 65, &[]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().last(), Some("✗ Build failed"), "{stdout}");
    assert!(
        stdout.contains("error: /src/App/ContentView.swift:17:19: cannot find"),
        "{stdout}"
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains("error:"), "{stderr}");
}

/// With nothing on the stream to explain it, the trailing error is the only
/// account of the failure and stays.
#[test]
fn a_failure_with_no_parsed_error_keeps_the_trailing_error() {
    let transcript = "Command PhaseScriptExecution failed with a nonzero exit code\n\
                      ** BUILD FAILED **\n";
    let out = build_with_stub("unexplained", transcript, 65, &[]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("error: building the project"), "{stderr}");
    assert!(
        stderr.contains("xcodebuild exited with a non-zero status"),
        "{stderr}"
    );
}

/// The machine-readable modes keep their error object whatever the terminal
/// would have shown.
#[test]
fn a_streamed_compile_error_still_reaches_the_machine_modes() {
    for mode in [["-o", "json"], ["-o", "ndjson"]] {
        let out = build_with_stub(mode[1], BROKEN, 65, &mode);
        assert_eq!(out.status.code(), Some(3), "{mode:?}: {out:?}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        let envelope: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
        assert_eq!(envelope["ok"], false, "{mode:?}");
        assert_eq!(envelope["error"]["code"], "build_failure", "{mode:?}");
        assert!(
            envelope["error"]["message"]
                .as_str()
                .unwrap()
                .starts_with("building the project: xcodebuild exited with a non-zero status"),
            "{mode:?}: {envelope}"
        );
        assert_eq!(
            envelope["error"]["diagnostics"][0]["message"],
            "cannot find 'undefinedSymbol' in scope",
            "{mode:?}"
        );
    }
}
