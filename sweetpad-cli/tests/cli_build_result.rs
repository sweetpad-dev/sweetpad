//! What `sweetpad build` reports once xcodebuild exits, per output mode. A stub
//! xcodebuild replays a canned transcript, so the result is checked end to end
//! without compiling anything.

mod common;

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

use common::TempDir;
use serde_json::Value;

fn tmp(tag: &str) -> TempDir {
    let dir = TempDir::new(&format!("sweetpad-build-{tag}"));
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
    sweetpad_with_stub(tag, transcript, status, &args)
}

/// `sweetpad <args>` against a stub xcodebuild that prints `transcript` and
/// exits with `status`.
fn sweetpad_with_stub(tag: &str, transcript: &str, status: i32, args: &[&str]) -> Output {
    let (mut cmd, _home, _cwd) = stub_command(tag, transcript, status, args);
    cmd.output().expect("failed to run the sweetpad binary")
}

/// The command [`sweetpad_with_stub`] runs, the directory it uses as home and
/// state dir, and the working directory that holds the stub. Both directories
/// go when their guards drop, so a caller keeps them until the command ends.
fn stub_command(
    tag: &str,
    transcript: &str,
    status: i32,
    args: &[&str],
) -> (Command, TempDir, TempDir) {
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

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sweetpad"));
    cmd.args(args)
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
        .env_remove("CLICOLOR_FORCE");
    (cmd, home, cwd)
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

/// Xcode 27 building for a locked iPhone, from `buildlog`'s captured case.
const DESTINATION_TIMEOUT: &str = "\
xcodebuild: error: Timed out waiting for all destinations matching the provided destination specifier to become available


\tDestinations compatible with the \"SweetpadCIApp\" scheme:
\t\t{ platform:iOS, arch:arm64, id:00008110-000559182E90401E, name:Iphone 13, error:Iphone 13 needs to be unlocked to enable development services Please unlock the device. }
";

/// The destination listing is the diagnostic's detail, so `error.message`
/// stays one line and ends on the log it names.
#[test]
fn a_destination_errors_headline_is_one_line() {
    let out = build_with_stub("destination", DESTINATION_TIMEOUT, 70, &["-o", "json"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    let envelope: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    let message = envelope["error"]["message"].as_str().unwrap();
    assert!(!message.contains('\n'), "{message}");
    assert!(
        message.starts_with(
            "building the project: xcodebuild exited with a non-zero status: xcodebuild: \
             Timed out waiting for all destinations matching the provided destination \
             specifier to become available; full log: "
        ),
        "{message}"
    );
    let diagnostic = envelope["error"]["diagnostics"][0]["message"]
        .as_str()
        .unwrap();
    assert!(
        diagnostic.contains("needs to be unlocked to enable development services"),
        "{diagnostic}"
    );
}

/// A destination error on a physical device closes on the command that says
/// why the device isn't ready, in every mode.
#[test]
fn a_device_destination_error_ends_on_a_device_info_tip() {
    let project = project();
    let build = |tag: &str, mode: &[&str]| {
        let mut args = vec![
            "build",
            "--project",
            project.to_str().unwrap(),
            "--scheme",
            "SweetpadCIMac",
            "--configuration",
            "Debug",
            "--destination",
            "platform=iOS,id=00008110-000559182E90401E",
            "--non-interactive",
        ];
        args.extend_from_slice(mode);
        sweetpad_with_stub(tag, DESTINATION_TIMEOUT, 70, &args)
    };
    let tip = "run 'sweetpad device info 00008110-000559182E90401E' to see why the device \
               isn't ready";

    let human = build("device-human", &[]);
    assert_eq!(human.status.code(), Some(3), "{human:?}");
    let stderr = String::from_utf8(human.stderr).unwrap();
    assert_eq!(
        stderr.lines().last(),
        Some(format!("tip: {tip}").as_str()),
        "{stderr}"
    );
    assert!(!stderr.contains("error:"), "{stderr}");

    for mode in [["-o", "json"], ["-o", "ndjson"]] {
        let out = build(&format!("device-{}", mode[1]), &mode);
        assert_eq!(out.status.code(), Some(3), "{mode:?}: {out:?}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        let envelope: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
        assert_eq!(envelope["error"]["tip"], tip, "{mode:?}");
    }
}

/// The same error on a destination that is not a physical device has no
/// device to ask about.
#[test]
fn a_destination_error_off_a_device_gets_no_tip() {
    let out = build_with_stub("mac", DESTINATION_TIMEOUT, 70, &["-o", "json"]);
    let stderr = String::from_utf8(out.stderr).unwrap();
    let envelope: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    assert!(envelope["error"].get("tip").is_none(), "{envelope}");
}

/// `app run`'s session build against a stub xcodebuild. The session builds
/// through its own runner rather than `build`'s, and `--hot --mac` reaches that
/// runner without a terminal, since the hot session builds before it launches
/// anything.
fn session_with_stub(tag: &str, transcript: &str, status: i32) -> Output {
    let project = project();
    sweetpad_with_stub(
        tag,
        transcript,
        status,
        &[
            "app",
            "run",
            "--hot",
            "--mac",
            "--project",
            project.to_str().unwrap(),
            "--scheme",
            "SweetpadCIMac",
            "--configuration",
            "Debug",
            "--non-interactive",
        ],
    )
}

/// The session's build closes on the same banner as `build`'s.
#[test]
fn the_run_sessions_build_ends_on_the_banner_too() {
    let out = session_with_stub("session", BROKEN, 65);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().last(), Some("✗ Build failed"), "{stdout}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains("error:"), "{stderr}");
}

/// Xcode 26 failing on a package build-tool plugin nobody has approved, which
/// it names only in the list of failed commands.
const BLOCKED: &str = "\
Prepare packages
Validate plug-in \u{201c}SwiftLintPlugin\u{201d} in package \u{201c}swiftlint\u{201d}
** BUILD FAILED **

The following build commands failed:
\tValidate plug-in \u{201c}SwiftLintPlugin\u{201d} in package \u{201c}swiftlint\u{201d}
\tBuilding workspace SweetpadCIApp with scheme SweetpadCIMac and configuration Debug
(2 failures)
";

/// No diagnostic explains a build blocked on plugin approval, so both `build`
/// and the run session end on the flag that gets past it.
#[test]
fn a_blocked_build_names_the_flag_in_the_run_session_too() {
    let build = build_with_stub("blocked-build", BLOCKED, 65, &[]);
    let session = session_with_stub("blocked-session", BLOCKED, 65);
    for out in [build, session] {
        assert_eq!(out.status.code(), Some(3), "{out:?}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(
            stderr.contains(
                "the build is blocked, not broken: SwiftLintPlugin build-tool plugin must be \
                 approved before it can run"
            ),
            "{stderr}"
        );
        assert!(
            stderr.contains("retry with '-- -skipPackagePluginValidation'"),
            "{stderr}"
        );
    }
}

/// A 24x80 pty: the master end, and the slave end a child takes as its
/// terminal.
fn pty() -> (std::fs::File, OwnedFd) {
    use std::os::fd::FromRawFd;

    let (mut master, mut slave) = (0, 0);
    let mut size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // Safety: openpty(3) writes two fresh fds on success, each owned by the
    // wrapper built from it below.
    let rc = unsafe {
        libc::openpty(
            &raw mut master,
            &raw mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut size,
        )
    };
    assert_eq!(rc, 0, "openpty: {}", std::io::Error::last_os_error());
    unsafe {
        (
            std::fs::File::from_raw_fd(master),
            OwnedFd::from_raw_fd(slave),
        )
    }
}

/// Run `cmd` on a pty for its stdin, stdout and stderr, typing `keys` once
/// `prompt` shows. Returns the exit status and everything the child wrote.
/// CI's own variables are dropped, since they make every run non-interactive.
fn on_pty(mut cmd: Command, prompt: &str, keys: &[u8]) -> (ExitStatus, String) {
    use std::io::{Read, Write};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    let (mut master, slave) = pty();
    cmd.env_remove("CI")
        .env_remove("SWEETPAD_NONINTERACTIVE")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave));
    let mut child = cmd.spawn().unwrap();
    // The master reads end of file only once no slave end is open, and `cmd`
    // still holds the parent's copies.
    drop(cmd);
    let transcript = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let transcript = Arc::clone(&transcript);
        let mut master = master.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n @ 1..) = master.read(&mut buf) {
                transcript.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        })
    };
    let shown = || String::from_utf8_lossy(&transcript.lock().unwrap()).into_owned();

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut typed = false;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("still running after a minute:\n{}", shown());
        }
        if !typed && shown().contains(prompt) {
            master.write_all(keys).unwrap();
            typed = true;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    reader.join().unwrap();
    (status, shown())
}

/// Quitting a session whose only build failed exits 3, the code `build` gives
/// the same failure. The one-time tip waits for a command that succeeds rather
/// than printing under the error.
#[test]
fn quitting_a_session_whose_build_failed_exits_as_a_failed_build() {
    let project = project();
    let (session, home, _cwd) = stub_command(
        "quit",
        BROKEN,
        65,
        &[
            "app",
            "run",
            "--mac",
            "--project",
            project.to_str().unwrap(),
            "--scheme",
            "SweetpadCIMac",
            "--configuration",
            "Debug",
        ],
    );
    let (status, shown) = on_pty(session, "q quit", b"q");
    assert_eq!(status.code(), Some(3), "{shown}");
    assert!(
        shown.contains("the last build failed, so nothing was launched"),
        "{shown}"
    );
    let tip = "(this tip shows once)";
    assert!(!shown.contains(tip), "{shown}");

    let (mut help, _help_home, _help_cwd) = stub_command("quit-help", "", 0, &["help"]);
    help.env("XDG_STATE_HOME", &*home);
    let (status, shown) = on_pty(help, "", b"");
    assert!(status.success(), "{shown}");
    assert!(shown.contains(tip), "{shown}");
}
