//! Enforces the `--json` envelope contract across the non-streaming commands.
//!
//! Under `--json`, a command's stdout must be EITHER one `{schema, ok:true, data}`
//! value, or empty with a `{schema, ok:false, error:{code}}` envelope on stderr.
//! A stray `println!`, a forgotten payload, or a bare (un-enveloped) value would
//! all break the single-value parse below — so this is the regression net for the
//! "render once, centrally" design.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::TempDir;
use serde_json::Value;

/// Run the `sweetpad` binary with an isolated XDG/HOME so the test never reads
/// the developer's real config/state and DerivedData resolution is deterministic.
/// `TMPDIR` points into the home too, so whatever a spawned tool leaves there
/// goes with the home.
fn sweetpad(args: &[&str], cwd: &Path, home: &Path) -> Output {
    sweetpad_with_tmpdir(args, cwd, home, home)
}

fn sweetpad_with_tmpdir(args: &[&str], cwd: &Path, home: &Path, tmpdir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home)
        .env("TMPDIR", tmpdir)
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("SWEETPAD_NONINTERACTIVE")
        .output()
        .expect("failed to run the sweetpad binary")
}

fn tmp(tag: &str) -> TempDir {
    let dir = TempDir::new(&format!("sweetpad-json-{tag}"));
    // A `.git` marker stops walk-up discovery at this directory — without it
    // the CLI would walk into the shared temp root, where concurrently-running
    // tests drop `.xcodeproj` fixtures.
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

/// Parse stdout as exactly one JSON value — fails if there is any non-JSON or
/// trailing junk, which IS the "nothing leaked to stdout" check.
fn parse_stdout(out: &Output, args: &[&str]) -> Value {
    let stdout = String::from_utf8(out.stdout.clone()).unwrap();
    serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("{args:?}: stdout is not one JSON value ({e}):\n{stdout:?}"))
}

/// The error envelope is the *last* line of stderr, compact by design — earlier
/// lines may legitimately be warnings (config typos, ambiguity notes), which
/// the documented contract keeps out of the machine-parsed envelope itself.
fn parse_stderr_error(out: &Output, args: &[&str]) -> Value {
    let stderr = String::from_utf8(out.stderr.clone()).unwrap();
    let last = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default();
    let v: Value = serde_json::from_str(last.trim()).unwrap_or_else(|e| {
        panic!("{args:?}: stderr's last line is not a JSON error envelope ({e}):\n{stderr:?}")
    });
    assert_eq!(v["schema"], 1, "{args:?}: error envelope schema");
    assert_eq!(v["ok"], Value::Bool(false), "{args:?}: error envelope ok");
    assert!(
        v["error"]["code"].is_string(),
        "{args:?}: error envelope needs a string code"
    );
    v
}

/// Commands that produce a success payload with no Xcode — these don't read
/// the project's contents, but an explicit `--project` must name a path that
/// exists (a nonexistent one is a target-resolution error by design), so a
/// bare `.xcodeproj` directory is scaffolded.
#[test]
fn success_payloads_are_enveloped() {
    let home = tmp("ok-home");
    let cwd = tmp("ok-cwd");
    let proj_dir = cwd.join("Fixture.xcodeproj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    let proj = proj_dir.to_str().unwrap();
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

/// `doctor` runs `swift --version`, and the Swift driver leaves a
/// `TemporaryDirectory.*` in `$TMPDIR` on every run unless it is handed a
/// `TMPDIR` of its own.
#[test]
fn doctor_leaves_nothing_in_tmpdir() {
    let home = tmp("doctor-home");
    let cwd = tmp("doctor-cwd");
    let tmpdir = TempDir::new("sweetpad-json-doctor-tmp");
    let args: &[&str] = &["doctor", "--json", "--non-interactive"];
    let out = sweetpad_with_tmpdir(args, &cwd, &home, &tmpdir);
    assert!(out.status.code().is_some(), "{args:?}: {out:?}");
    let left: Vec<_> = std::fs::read_dir(&*tmpdir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "doctor left {left:?} in TMPDIR");
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

/// Bare `sweetpad --json` outside a project must speak JSON like everything
/// else: an error envelope and exit 4 — never the human help wall on stdout.
#[test]
fn bare_json_outside_a_project_is_an_error_envelope() {
    let home = tmp("bare-home");
    let cwd = tmp("bare-cwd"); // empty dir → no project to discover
    let args: &[&str] = &["--json"];
    let out = sweetpad(args, &cwd, &home);
    let stdout = String::from_utf8(out.stdout.clone()).unwrap();
    assert!(
        stdout.trim().is_empty(),
        "bare --json must not write text to stdout, got {stdout:?}"
    );
    let err = parse_stderr_error(&out, args);
    assert_eq!(err["error"]["code"], "target_resolution");
    assert_eq!(out.status.code(), Some(4));
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

/// A command line clap parses but the command refuses on its own is a usage
/// error: a flag on a verb it means nothing to (a run flag on 'test build', a
/// start flag on 'build diagnostics'), an argument or flag value no project
/// could make valid. Exit 2, the code clap's own usage errors get, and a
/// 'usage_error' envelope under '--json'. Each is refused before any project
/// is looked for, so no fixture is needed.
#[test]
fn a_refused_flag_is_a_usage_error() {
    let home = tmp("usage-home");
    let cwd = tmp("usage-cwd");
    let refused: &[&[&str]] = &[
        &["test", "build", "--failed"],
        &["test", "attachments", "--junit", "x.xml"],
        &["test", "output", "--coverage"],
        &["build", "diagnostics", "--clean"],
        &[
            "pbxproj",
            "membership",
            "add",
            "--target",
            "App",
            "--phase",
            "sources",
        ],
        &["pbxproj", "settings", "set", "NO_EQUALS_SIGN"],
        &["pbxproj", "settings", "unset", "SWIFT_VERSION=5.0"],
        &["archive", "--on", "toaster"],
        &["app", "ui", "click", "--label", "Save", "--nth", "0"],
        &["app", "screenshot", "--window", "0"],
        &["context", "alias", "mac", "iPhone 17"],
        &["context", "set", "sdk", "iphoneos", "--testing"],
        &["context", "select", "sdk", "--testing"],
        &["context", "remove", "target"],
        &["project", "new", "my app", "--no-git"],
        &[
            "project",
            "new",
            "App",
            "--bundle-id",
            "not a bundle id",
            "--no-git",
        ],
        &[
            "project",
            "new",
            "App",
            "--deployment-target",
            "latest",
            "--no-git",
        ],
        &["help", "no-such-topic"],
        // A '--' tail naming what sweetpad passes xcodebuild itself would
        // fail inside xcodebuild ("may only be provided once"), so the flag
        // that sets it is named instead.
        &["build", "--", "-scheme", "App"],
        &["test", "build", "--", "-configuration", "Release"],
        &["test", "--", "-resultBundlePath", "r.xcresult"],
        &["test", "--coverage", "--", "-enableCodeCoverage", "NO"],
        &["test", "--retry-flaky", "2", "--", "-test-iterations", "3"],
        &["archive", "--", "-archivePath", "App.xcarchive"],
        &["app", "run", "--mac", "--", "-sdk", "macosx"],
        &["app", "install", "--", "-project", "Other.xcodeproj"],
        &["settings", "show", "--", "-workspace", "Other.xcworkspace"],
    ];
    for args in refused {
        let human = sweetpad(args, &cwd, &home);
        assert_eq!(human.status.code(), Some(2), "{args:?}: {human:?}");
        let stderr = String::from_utf8(human.stderr).unwrap();
        assert!(stderr.contains("error:"), "{args:?}: {stderr}");

        // Ahead of any '--' tail, which would take it as xcodebuild's.
        let tail = args.iter().position(|a| *a == "--").unwrap_or(args.len());
        let json_args = [&args[..tail], &["--json"], &args[tail..]].concat();
        let out = sweetpad(&json_args, &cwd, &home);
        assert_eq!(out.status.code(), Some(2), "{json_args:?}: {out:?}");
        let err = parse_stderr_error(&out, &json_args);
        assert_eq!(err["error"]["code"], "usage_error", "{json_args:?}");
    }

    // A flag that conflicts with the output mode is refused the same way.
    for args in [
        &["build", "--gh-annotations", "--json"][..],
        &["app", "debug", "--batch", "--json"],
    ] {
        let out = sweetpad(args, &cwd, &home);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
        assert_eq!(
            parse_stderr_error(&out, args)["error"]["code"],
            "usage_error",
            "{args:?}"
        );
    }
    // A refused scaffold wrote nothing into the working directory.
    let left: Vec<_> = std::fs::read_dir(&*cwd)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|name| name != ".git")
        .collect();
    assert!(left.is_empty(), "a refused command wrote {left:?}");
}

/// 'build', 'test' and 'test build' take '--mac' as '--on mac', so a typed
/// '--on' or '--destination' beside it is a usage error, found before any
/// project is looked for.
#[test]
fn a_typed_destination_beside_mac_is_a_usage_error_on_build_and_test() {
    let home = tmp("dest-mac-home");
    let cwd = tmp("dest-mac-cwd");
    for verb in [&["build"][..], &["test"], &["test", "build"]] {
        for (flag, value) in [
            ("--on", "mac"),
            ("--on", "iPhone 17"),
            ("--destination", "platform=macOS"),
        ] {
            let args = [verb, &[flag, value, "--mac"]].concat();
            let out = sweetpad(&args, &cwd, &home);
            assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
            let stderr = String::from_utf8(out.stderr).unwrap();
            let expected = format!("{flag} and --mac are mutually exclusive; pass one");
            assert!(stderr.contains(&expected), "{args:?}: {stderr}");
        }
    }
}

/// Every `app` verb that takes the mode flags checks a typed '--on' or
/// '--destination' against them the way 'app run' does, before any project
/// is looked for: both typed is a usage error, whichever destination the
/// other flag names.
#[test]
fn a_typed_on_or_destination_beside_a_mode_flag_is_a_usage_error_on_every_app_verb() {
    let home = tmp("on-mac-home");
    let cwd = tmp("on-mac-cwd");
    let verbs = [
        "run",
        "install",
        "launch",
        "debug",
        "diagnose",
        "uninstall",
        "logs",
        "stop",
        "container",
    ];
    for verb in verbs {
        for (flag, value, mode) in [
            ("--on", "mac", "--mac"),
            ("--on", "iPhone 17", "--mac"),
            ("--destination", "platform=iOS Simulator,name=Nope", "--mac"),
            ("--destination", "platform=macOS", "--mac"),
            ("--destination", "platform=iOS Simulator", "--device"),
        ] {
            let args = ["app", verb, flag, value, mode];
            let out = sweetpad(&args, &cwd, &home);
            assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
            let stderr = String::from_utf8(out.stderr).unwrap();
            let expected = format!("{flag} and {mode} are mutually exclusive; pass one");
            assert!(stderr.contains(&expected), "{args:?}: {stderr}");
        }
    }
}
