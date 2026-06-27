//! Enforces the `--json` envelope contract across the non-streaming commands.
//!
//! Under `--json`, a command's stdout must be EITHER one `{schema, ok:true, data}`
//! value, or empty with a `{schema, ok:false, error:{code}}` envelope on stderr.
//! A stray `println!`, a forgotten payload, or a bare (un-enveloped) value would
//! all break the single-value parse below — so this is the regression net for the
//! "render once, centrally" design.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Run the `sweetpad` binary with an isolated XDG/HOME so the test never reads
/// the developer's real config/state and DerivedData resolution is deterministic.
fn sweetpad(args: &[&str], cwd: &Path, home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home)
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("SWEETPAD_NONINTERACTIVE")
        .output()
        .expect("failed to run the sweetpad binary")
}

fn tmp(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sweetpad-json-{tag}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Parse stdout as exactly one JSON value — fails if there is any non-JSON or
/// trailing junk, which IS the "nothing leaked to stdout" check.
fn parse_stdout(out: &Output, args: &[&str]) -> Value {
    let stdout = String::from_utf8(out.stdout.clone()).unwrap();
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("{args:?}: stdout is not one JSON value ({e}):\n{stdout:?}"))
}

fn parse_stderr_error(out: &Output, args: &[&str]) -> Value {
    let stderr = String::from_utf8(out.stderr.clone()).unwrap();
    let v: Value = serde_json::from_str(stderr.trim()).unwrap_or_else(|e| {
        panic!("{args:?}: stderr is not a JSON error envelope ({e}):\n{stderr:?}")
    });
    assert_eq!(v["schema"], 1, "{args:?}: error envelope schema");
    assert_eq!(v["ok"], Value::Bool(false), "{args:?}: error envelope ok");
    assert!(
        v["error"]["code"].is_string(),
        "{args:?}: error envelope needs a string code"
    );
    v
}

/// Commands that produce a success payload with no real project or Xcode — a
/// `--project` flag pointing anywhere is enough (these don't read the project).
#[test]
fn success_payloads_are_enveloped() {
    let home = tmp("ok-home");
    let cwd = tmp("ok-cwd");
    let proj = "/tmp/sweetpad-does-not-exist.xcodeproj";
    let commands: &[&[&str]] = &[
        &[
            "context",
            "show",
            "--project",
            proj,
            "--json",
            "--non-interactive",
        ],
        &[
            "derived-data",
            "path",
            "--project",
            proj,
            "--json",
            "--non-interactive",
        ],
        &[
            "derived-data",
            "size",
            "--project",
            proj,
            "--json",
            "--non-interactive",
        ],
    ];
    for args in commands {
        let out = sweetpad(args, &cwd, &home);
        assert!(out.status.success(), "{args:?}: expected exit 0");
        let v = parse_stdout(&out, args);
        assert_eq!(v["schema"], 1, "{args:?}");
        assert_eq!(v["ok"], Value::Bool(true), "{args:?}");
        assert!(
            v.get("data").is_some(),
            "{args:?}: success envelope needs data"
        );
    }
}

/// Tool-backed commands shell to `simctl`/`xcrun`/`devicectl`. With Xcode they
/// succeed; without it they emit a `tool_missing` error envelope. Either way the
/// invariant holds: stdout is one success value, or empty with an error on stderr.
#[test]
fn tool_backed_commands_are_enveloped_or_error() {
    let home = tmp("tool-home");
    let cwd = tmp("tool-cwd"); // empty — no project needed for these
    let commands: &[&[&str]] = &[
        &["simulator", "list", "--json", "--non-interactive"],
        &["device", "list", "--json", "--non-interactive"],
        &["destination", "list", "--json", "--non-interactive"],
        &["doctor", "--json", "--non-interactive"],
    ];
    for args in commands {
        let out = sweetpad(args, &cwd, &home);
        let stdout = String::from_utf8(out.stdout.clone()).unwrap();
        if stdout.trim().is_empty() {
            parse_stderr_error(&out, args); // errored → must be an error envelope
        } else {
            let v = parse_stdout(&out, args);
            assert_eq!(v["schema"], 1, "{args:?}");
            assert_eq!(v["ok"], Value::Bool(true), "{args:?}");
            assert!(v.get("data").is_some(), "{args:?}");
        }
    }
}

/// An unresolved target under `--json --non-interactive` is the canonical error
/// path: empty stdout, a `target_resolution` error envelope on stderr, exit 4.
#[test]
fn unresolved_target_is_an_error_envelope() {
    let home = tmp("err-home");
    let cwd = tmp("err-cwd"); // empty dir → no project to discover
    let args: &[&str] = &["scheme", "list", "--json", "--non-interactive"];
    let out = sweetpad(args, &cwd, &home);
    let stdout = String::from_utf8(out.stdout.clone()).unwrap();
    assert!(
        stdout.trim().is_empty(),
        "error path must not write to stdout, got {stdout:?}"
    );
    let err = parse_stderr_error(&out, args);
    assert_eq!(err["error"]["code"], "target_resolution");
    assert_eq!(out.status.code(), Some(4), "target resolution → exit 4");
}

/// `app run` streams a live session, so it deliberately rejects `--json` rather
/// than emit a degenerate payload — it must never produce a success envelope.
#[test]
fn app_run_rejects_json() {
    let home = tmp("apprun-home");
    let cwd = tmp("apprun-cwd");
    let args: &[&str] = &["app", "run", "--json"];
    let out = sweetpad(args, &cwd, &home);
    let stdout = String::from_utf8(out.stdout.clone()).unwrap();
    assert!(
        stdout.trim().is_empty(),
        "app run --json must not emit a success payload, got {stdout:?}"
    );
    assert!(!out.status.success(), "app run --json must exit non-zero");
    parse_stderr_error(&out, args);
}
