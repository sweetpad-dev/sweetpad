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
    let home = tmp(&format!("{tag}-home"));
    let cwd = tmp(&format!("{tag}-cwd"));
    write_stub(&cwd, transcript, status);
    let cmd = command_in(&home, &cwd, args);
    (cmd, home, cwd)
}

/// Put an xcodebuild in `cwd/bin` that prints `transcript` and exits with
/// `status`, replacing the one already there.
fn write_stub(cwd: &Path, transcript: &str, status: i32) {
    use std::os::unix::fs::PermissionsExt;

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
}

/// `sweetpad <args>` in `cwd`, with `home` as its home and state dir and the
/// stub [`write_stub`] put in `cwd` ahead of the real xcodebuild.
fn command_in(home: &Path, cwd: &Path, args: &[&str]) -> Command {
    let bin = cwd.join("bin");
    // An empty developer dir keeps the `productPath` lookup from loading the
    // installed Xcode's specs, which costs seconds and is not under test.
    let developer_dir = cwd.join("Developer");
    std::fs::create_dir_all(&developer_dir).unwrap();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sweetpad"));
    cmd.args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home)
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
    cmd
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

/// The build arguments [`build_with_stub`] passes, for a caller that needs
/// the command itself.
fn build_args(project: &Path) -> Vec<&str> {
    vec![
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
    ]
}

/// '--mac' on 'build', 'test' and 'test build' is '--on mac': each builds for
/// 'platform=macOS', and an exported destination yields to it as it would to
/// any typed flag.
#[test]
fn mac_names_the_mac_on_build_and_test() {
    let project = project();
    for verb in [&["build"][..], &["test"], &["test", "build"]] {
        let mut args = verb.to_vec();
        args.extend([
            "--project",
            project.to_str().unwrap(),
            "--scheme",
            "SweetpadCIMac",
            "--configuration",
            "Debug",
            "--mac",
            "--show-command",
            "-o",
            "json",
            "--non-interactive",
        ]);
        let (mut cmd, _home, _cwd) = stub_command("mac", "", 0, &args);
        let out = cmd
            .env("SWEETPAD_DESTINATION", "platform=iOS Simulator,name=Nope")
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
        let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
        let command: Vec<&str> = envelope["data"]["command"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        let destination = command
            .windows(2)
            .find(|pair| pair[0] == "-destination")
            .map(|pair| pair[1]);
        assert_eq!(destination, Some("platform=macOS"), "{args:?}: {command:?}");
    }
}

/// Run `cmd` with stdout and stderr in one file under `dir`, the way a
/// terminal or a `2>&1` capture holds them. Returns the exit code and what
/// the file holds.
fn in_one_file(mut cmd: Command, dir: &Path) -> (Option<i32>, String) {
    let transcript = dir.join("one-file.txt");
    let file = std::fs::File::create(&transcript).unwrap();
    let status = cmd
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .status()
        .unwrap();
    (status.code(), std::fs::read_to_string(&transcript).unwrap())
}

/// The streamed log already names the compile error and closes on `✗ Build
/// failed`, so where stderr shares its file, output ends there; the exit code
/// still says it failed.
#[test]
fn a_streamed_compile_error_ends_human_output_at_the_banner() {
    let project = project();
    let (cmd, _home, cwd) = stub_command("broken", BROKEN, 65, &build_args(&project));
    let (code, shown) = in_one_file(cmd, &cwd);
    assert_eq!(code, Some(3), "{shown}");
    assert_eq!(shown.lines().last(), Some("✗ Build failed"), "{shown}");
    assert!(
        shown.contains("error: /src/App/ContentView.swift:17:19: cannot find"),
        "{shown}"
    );
    assert_eq!(
        shown.matches("cannot find 'undefinedSymbol'").count(),
        1,
        "{shown}"
    );
    assert!(!shown.contains("error: building"), "{shown}");
}

/// Every verb that builds streams its errors to stdout, and with stderr in
/// one file beside it none of them repeats the errors after the banner.
#[test]
fn no_build_verb_repeats_errors_the_log_just_showed() {
    let project = project();
    let project = project.to_str().unwrap();
    let target = [
        "--project",
        project,
        "--scheme",
        "SweetpadCIMac",
        "--configuration",
        "Debug",
    ];
    let with = |verb: &[&'static str], rest: &[&'static str]| -> Vec<&str> {
        let mut args: Vec<&str> = verb.to_vec();
        args.extend_from_slice(&target);
        args.extend_from_slice(rest);
        args.push("--non-interactive");
        args
    };
    let mac = ["--destination", "platform=macOS"];
    for (tag, args) in [
        ("once-test", with(&["test"], &mac)),
        ("once-test-build", with(&["test", "build"], &mac)),
        ("once-session", with(&["app", "run", "--hot", "--mac"], &[])),
    ] {
        let (mut cmd, _home, cwd) = stub_command(tag, BROKEN, 65, &args);
        // The hot session checks for an injection client before it builds.
        let client = cwd.join("client.dylib");
        std::fs::write(&client, b"").unwrap();
        cmd.env("SWEETPAD_HOTRELOAD_DYLIB", &client);
        let (code, shown) = in_one_file(cmd, &cwd);
        assert_eq!(code, Some(3), "{tag}: {shown}");
        assert_eq!(
            shown.matches("cannot find 'undefinedSymbol'").count(),
            1,
            "{tag}: {shown}"
        );
        assert!(!shown.contains("error: building"), "{tag}: {shown}");
        assert!(!shown.contains("error: running"), "{tag}: {shown}");
    }
}

/// With stderr in a file of its own (`2>err.log`), the streamed errors are
/// not in front of the error, so it repeats them and names the command that
/// reads them back.
#[test]
fn a_redirected_stderr_gets_the_errors_the_log_showed() {
    let out = build_with_stub("redirected", BROKEN, 65, &[]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().last(), Some("✗ Build failed"), "{stdout}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.ends_with(
            "error: building the project\n  \
             xcodebuild exited with a non-zero status:\n  \
             error: /src/App/ContentView.swift:17:19: cannot find 'undefinedSymbol' in scope\n\
             tip: run 'sweetpad build diagnostics' to see every error and warning\n"
        ),
        "{stderr}"
    );
}

/// Past the first few errors, the repeat counts the rest and names the
/// command that reads them all back, after a failed `test` build too.
#[test]
fn a_redirected_stderr_counts_the_errors_it_leaves_out() {
    let broken = (1..=5)
        .map(|n| format!("/src/App/ContentView.swift:{n}:9: error: cannot find 'x{n}' in scope\n"))
        .chain(["** BUILD FAILED **\n".to_string()])
        .collect::<Vec<_>>()
        .concat();
    let first_three = (1..=3)
        .map(|n| {
            format!("  error: /src/App/ContentView.swift:{n}:9: cannot find 'x{n}' in scope\n")
        })
        .collect::<Vec<_>>()
        .concat();

    let build = build_with_stub("many-build", &broken, 65, &[]);
    let stderr = String::from_utf8(build.stderr).unwrap();
    assert!(
        stderr.ends_with(&format!(
            "xcodebuild exited with a non-zero status:\n{first_three}  \
             and 2 more error(s)\n\
             tip: run 'sweetpad build diagnostics' to see every error and warning\n"
        )),
        "{stderr}"
    );

    let project = project();
    let mut args = build_args(&project);
    args[0] = "test";
    let test = sweetpad_with_stub("many-test", &broken, 65, &args);
    assert_eq!(test.status.code(), Some(3), "{test:?}");
    let stderr = String::from_utf8(test.stderr).unwrap();
    assert!(
        stderr.ends_with(&format!(
            "error: running the tests\n  \
             xcodebuild test failed before any test ran:\n{first_three}  \
             and 2 more error(s)\n\
             tip: run 'sweetpad build diagnostics' to see every error and warning\n"
        )),
        "{stderr}"
    );
}

/// A `test` whose build fails is the project's last build, so 'build
/// diagnostics' reads that failure back, in place of the build before it.
#[test]
fn build_diagnostics_reads_back_a_failed_test_build() {
    let project = project();
    let args = build_args(&project);
    let (mut build, home, cwd) = stub_command("test-record", WARNED, 0, &args);
    assert!(build.output().unwrap().status.success());

    let mut test_args = args.clone();
    test_args[0] = "test";
    for mode in [&[][..], &["-o", "json"], &["-o", "ndjson"]] {
        write_stub(&cwd, BROKEN, 65);
        let run = [&test_args[..], mode].concat();
        let out = command_in(&home, &cwd, &run).output().unwrap();
        assert_eq!(out.status.code(), Some(3), "{run:?}: {out:?}");

        let read = ["build", "diagnostics", "--project", args[2], "-o", "json"];
        let out = command_in(&home, &cwd, &read).output().unwrap();
        assert!(out.status.success(), "{mode:?}: {out:?}");
        let record: Value = serde_json::from_slice(&out.stdout).unwrap();
        let record = &record["data"];
        assert_eq!(record["ok"], false, "{mode:?}: {record}");
        assert_eq!(record["scheme"], "SweetpadCIMac", "{mode:?}: {record}");
        assert_eq!(record["errors"], 1, "{mode:?}: {record}");
        assert_eq!(
            record["diagnostics"][0]["message"], "cannot find 'undefinedSymbol' in scope",
            "{mode:?}: {record}"
        );

        // A passing build in between puts the record back to its own.
        write_stub(&cwd, WARNED, 0);
        assert!(
            command_in(&home, &cwd, &args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
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

    // A stderr of its own repeats the destination error, and the device's
    // tip still closes it.
    let human = build("device-human", &[]);
    assert_eq!(human.status.code(), Some(3), "{human:?}");
    let stderr = String::from_utf8(human.stderr).unwrap();
    assert_eq!(
        stderr.lines().last(),
        Some(format!("tip: {tip}").as_str()),
        "{stderr}"
    );
    assert!(
        stderr.contains("  error: xcodebuild: Timed out waiting for all destinations"),
        "{stderr}"
    );
    assert!(!stderr.contains("build diagnostics"), "{stderr}");

    let (cmd, _home, cwd) = stub_command(
        "device-one-file",
        DESTINATION_TIMEOUT,
        70,
        &[
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
        ],
    );
    let (code, shown) = in_one_file(cmd, &cwd);
    assert_eq!(code, Some(3), "{shown}");
    assert_eq!(
        shown.lines().last(),
        Some(format!("tip: {tip}").as_str()),
        "{shown}"
    );
    assert!(!shown.contains("error: building"), "{shown}");

    for mode in [["-o", "json"], ["-o", "ndjson"]] {
        let out = build(&format!("device-{}", mode[1]), &mode);
        assert_eq!(out.status.code(), Some(3), "{mode:?}: {out:?}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        let envelope: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
        assert_eq!(envelope["error"]["tip"], tip, "{mode:?}");
    }
}

/// Xcode 27 given a device id that matches nothing (cut to a few entries):
/// it waits about a minute, then fails with no `** BUILD FAILED **`.
const DESTINATION_NOT_FOUND: &str = "\
xcodebuild: error: Unable to find a device matching the provided destination specifier:
\t\t{ platform:iOS, id:00008110-000A1B2C3D4E5F60 }

\tThe requested device could not be found because no available devices matched the request.

\tDestinations compatible with the \"SweetpadCIApp\" scheme:
\t\t{ platform:iOS, id:dvtdevice-DVTiPhonePlaceholder-iphoneos:placeholder, name:Any iOS Device }
\t\t{ platform:iOS Simulator, arch:arm64, id:F13C004A-0824-4870-B4F2-29AAEE36636E, OS:27.0, name:iPhone 17 }
";

/// With no banner of xcodebuild's own, a destination error still closes on
/// the `✗` line every other failed run ends on, for `build` and for `test`,
/// and the tip that follows it on stderr stays the last word.
#[test]
fn a_destination_error_closes_on_the_failure_banner() {
    let project = project();
    let tip = "tip: run 'sweetpad device info 00008110-000A1B2C3D4E5F60' to see why the \
               device isn't ready";
    for (verb, banner) in [("build", "✗ Build failed"), ("test", "✗ Tests failed")] {
        let out = sweetpad_with_stub(
            &format!("not-found-{verb}"),
            DESTINATION_NOT_FOUND,
            70,
            &[
                verb,
                "--project",
                project.to_str().unwrap(),
                "--scheme",
                "SweetpadCIMac",
                "--configuration",
                "Debug",
                "--destination",
                "platform=iOS,id=00008110-000A1B2C3D4E5F60",
                "--non-interactive",
            ],
        );
        assert_eq!(out.status.code(), Some(3), "{verb}: {out:?}");
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert!(
            stdout.contains("Unable to find a device matching"),
            "{verb}: {stdout}"
        );
        assert_eq!(stdout.lines().last(), Some(banner), "{verb}: {stdout}");
        assert_eq!(
            stdout.matches(banner).count(),
            1,
            "{verb}: one banner: {stdout}"
        );
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert_eq!(stderr.lines().last(), Some(tip), "{verb}: {stderr}");
        assert!(
            stderr.contains(
                "  error: xcodebuild: Unable to find a device matching the provided \
                 destination specifier\n"
            ),
            "{verb}: {stderr}"
        );
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
/// anything. It checks for an injection client before that build, and a
/// sweetpad built from source may bundle none, so a stand-in file serves as
/// the client.
fn session_with_stub(tag: &str, transcript: &str, status: i32) -> Output {
    let (mut cmd, _home, cwd) = hot_session_command(tag, transcript, status);
    let client = cwd.join("client.dylib");
    std::fs::write(&client, b"").unwrap();
    cmd.env("SWEETPAD_HOTRELOAD_DYLIB", &client)
        .output()
        .expect("failed to run the sweetpad binary")
}

/// The `app run --hot --mac` command [`session_with_stub`] runs, with its
/// directories as [`stub_command`] returns them.
fn hot_session_command(tag: &str, transcript: &str, status: i32) -> (Command, TempDir, TempDir) {
    let project = project();
    stub_command(
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

/// `--hot` looks for its injection client before it builds, so a run with
/// none fails at once, exits as a missing tool, and says where it looked.
#[test]
fn a_hot_run_with_no_injection_client_fails_before_building() {
    let (mut cmd, _home, cwd) = hot_session_command("no-client", BROKEN, 65);
    let missing = cwd.join("no-such-client.dylib");
    let out = cmd
        .env("SWEETPAD_HOTRELOAD_DYLIB", &missing)
        .output()
        .expect("failed to run the sweetpad binary");
    assert_eq!(out.status.code(), Some(5), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(!stdout.contains("Compiling"), "{stdout}");
    assert!(!stdout.contains("Build failed"), "{stdout}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains(&format!(
            "SWEETPAD_HOTRELOAD_DYLIB points at {}, which doesn't exist",
            missing.display()
        )),
        "{stderr}"
    );
    assert!(!stderr.contains("building"), "{stderr}");
}

/// `app run <flags>` on a simulator in a project whose sweetpad.toml sets
/// `[run] hot = true`, against stubs: xcodebuild fails the build, `xcrun`
/// lists one booted simulator and answers everything else, and `open` does
/// nothing, so no Simulator window comes up.
fn hot_default_run(tag: &str, flags: &[&str], json: bool) -> Output {
    use std::os::unix::fs::PermissionsExt;

    let cwd = tmp(&format!("{tag}-project"));
    let copied = std::process::Command::new("cp")
        .arg("-R")
        .arg(project())
        .arg(&*cwd)
        .status()
        .unwrap();
    assert!(copied.success());
    std::fs::write(cwd.join("sweetpad.toml"), "[run]\nhot = true\n").unwrap();
    let project = cwd.join("SweetpadCIApp.xcodeproj");
    let mut args = vec![
        "app",
        "run",
        "--project",
        project.to_str().unwrap(),
        "--scheme",
        "SweetpadCIApp",
        "--configuration",
        "Debug",
        "--destination",
        "platform=iOS Simulator,id=AAAAAAAA-0000-0000-0000-000000000000",
        "--non-interactive",
    ];
    args.extend_from_slice(flags);
    if json {
        args.push("--json");
    }
    let (mut cmd, _home, stub) = stub_command(tag, BROKEN, 65, &args);
    let executable = |name: &str, body: &str| {
        let path = stub.join("bin").join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    executable(
        "xcrun",
        "#!/bin/sh\n\
         if [ \"$2\" = list ]; then\n\
         echo '{\"devices\":{\"com.apple.CoreSimulator.SimRuntime.iOS-26-0\":[{\"udid\":\
         \"AAAAAAAA-0000-0000-0000-000000000000\",\"name\":\"iPhone 17\",\"state\":\"Booted\",\
         \"isAvailable\":true}]}}'\n\
         fi\n",
    );
    executable("open", "#!/bin/sh\n");
    let client = stub.join("client.dylib");
    std::fs::write(&client, b"").unwrap();
    cmd.env("SWEETPAD_HOTRELOAD_DYLIB", &client)
        .output()
        .expect("failed to run the sweetpad binary")
}

/// A committed `[run] hot = true` yields to the flags that ask for a run a
/// hot session can't be, with a note, so the agent-facing `--no-logs` builds
/// in any project instead of being refused. A typed '--hot' still refuses.
#[test]
fn the_hot_default_yields_to_no_logs_detach_and_wait_for_debugger() {
    for (tag, flags) in [
        ("yield-no-logs", &["--no-logs"][..]),
        ("yield-detach", &["--detach"][..]),
        ("yield-debugger", &["--wait-for-debugger"][..]),
    ] {
        let out = hot_default_run(tag, flags, false);
        // The run went on to build, which the stub fails.
        assert_eq!(out.status.code(), Some(3), "{tag}: {out:?}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(
            stderr.contains(&format!(
                "hot reload off for this run: the '[run] hot = true' default yields to '{}'",
                flags[0]
            )),
            "{tag}: {stderr}"
        );
        assert!(!stderr.contains("isn't supported"), "{tag}: {stderr}");
        assert!(!stderr.contains("hot reload on"), "{tag}: {stderr}");
    }

    // The machine form carries no note, just the build's own error.
    let out = hot_default_run("yield-json", &["--no-logs"], true);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!stderr.contains("hot reload off"), "{stderr}");
    assert!(stderr.contains("\"code\":\"build_failure\""), "{stderr}");

    let out = hot_default_run("typed-hot", &["--hot", "--no-logs"], false);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("--no-logs isn't supported with --hot"),
        "{stderr}"
    );
}

/// The session's build closes on the same banner as `build`'s, whether
/// xcodebuild printed one (a compile error) or not (a destination error).
#[test]
fn the_run_sessions_build_ends_on_the_banner_too() {
    for (tag, transcript, status) in [
        ("session", BROKEN, 65),
        ("session-destination", DESTINATION_NOT_FOUND, 70),
    ] {
        let out = session_with_stub(tag, transcript, status);
        assert_eq!(out.status.code(), Some(3), "{tag}: {out:?}");
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert_eq!(
            stdout.lines().last(),
            Some("✗ Build failed"),
            "{tag}: {stdout}"
        );
        assert_eq!(
            stdout.matches("✗ Build failed").count(),
            1,
            "{tag}: {stdout}"
        );
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(
            stderr.contains(
                "error: building the app\n  xcodebuild exited with a non-zero status:\n  error: "
            ),
            "{tag}: {stderr}"
        );
    }
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

/// What to type on a pty: each `(prompt, keys)` pair's keys once its prompt
/// shows.
type Steps<'a> = [(&'a str, &'a str)];

/// Run `cmd` on a pty for its stdin, stdout and stderr, typing each step's
/// keys once its prompt shows in what the child wrote after the previous
/// step's keys. Returns the exit status and everything the child wrote. CI's
/// own variables are dropped, since they make every run non-interactive.
fn on_pty(mut cmd: Command, steps: &Steps) -> (ExitStatus, String) {
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
    let mut steps = steps.iter();
    let mut next = steps.next();
    let mut seen = 0;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("still running after a minute:\n{}", shown());
        }
        if let Some((prompt, keys)) = next {
            let so_far = shown();
            if so_far[seen..].contains(prompt) {
                master.write_all(keys.as_bytes()).unwrap();
                seen = so_far.len();
                next = steps.next();
            }
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
    let (status, shown) = on_pty(
        session,
        &[("q quit", "h"), ("q quit (terminate the app)", "q")],
    );
    assert_eq!(status.code(), Some(3), "{shown}");
    assert!(
        shown.contains("the last build failed, so nothing was launched"),
        "{shown}"
    );
    // With nothing launched there is no log stream, so no level keys to offer.
    assert!(!shown.contains("log level"), "{shown}");
    let tip = "(this tip shows once)";
    assert!(!shown.contains(tip), "{shown}");

    let (mut help, _help_home, _help_cwd) = stub_command("quit-help", "", 0, &["help"]);
    help.env("XDG_STATE_HOME", &*home);
    let (status, shown) = on_pty(help, &[]);
    assert!(status.success(), "{shown}");
    assert!(shown.contains(tip), "{shown}");
}

/// An `app run --mac` session against a stub xcodebuild whose builds from the
/// `slow_from`th on compile until interrupted, while earlier ones succeed at
/// once and leave a product that stays up: a script standing in for the app,
/// in the DerivedData the session is pointed at.
fn launchable_session(tag: &str, slow_from: u32) -> (Command, [TempDir; 3]) {
    use std::os::unix::fs::PermissionsExt;

    let project = project();
    let derived = tmp(&format!("{tag}-dd"));
    let (session, home, cwd) = stub_command(
        tag,
        "",
        0,
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
            "--",
            "-derivedDataPath",
            derived.to_str().unwrap(),
        ],
    );
    let executable = |path: &Path, body: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    executable(
        &cwd.join("bin/xcodebuild"),
        &format!(
            "#!/bin/sh\n\
             n=$(( $(cat \"$0.builds\" 2>/dev/null || echo 0) + 1 ))\n\
             echo $n > \"$0.builds\"\n\
             if [ $n -ge {slow_from} ]; then\n\
             echo \"CompileSwift normal arm64 /src/App/ContentView.swift (in target 'SweetpadCIMac' from project 'SweetpadCIApp')\"\n\
             exec sleep 30\n\
             fi\n\
             echo '** BUILD SUCCEEDED **'\n"
        ),
    );
    executable(
        &derived.join("Build/Products/Debug/SweetpadCIMac.app/Contents/MacOS/SweetpadCIMac"),
        "#!/bin/sh\nexec sleep 60\n",
    );
    (session, [home, cwd, derived])
}

/// An `app run --mac` session plus `extra` flags, whose build succeeds at once
/// and whose product writes AppKit's note about `-ApplePersistenceIgnoreState`
/// to stderr, as a real app launched with it does, then `stderr marker` on
/// the same pipe, and stays up.
fn persistence_note_session(tag: &str, extra: &[&str]) -> (Command, [TempDir; 3]) {
    use std::os::unix::fs::PermissionsExt;

    let project = project();
    let derived = tmp(&format!("{tag}-dd"));
    let mut args = vec![
        "app",
        "run",
        "--mac",
        "--project",
        project.to_str().unwrap(),
        "--scheme",
        "SweetpadCIMac",
        "--configuration",
        "Debug",
    ];
    args.extend_from_slice(extra);
    args.extend_from_slice(&["--", "-derivedDataPath", derived.to_str().unwrap()]);
    let (session, home, cwd) = stub_command(tag, "** BUILD SUCCEEDED **\n", 0, &args);
    let app = derived.join("Build/Products/Debug/SweetpadCIMac.app/Contents/MacOS/SweetpadCIMac");
    std::fs::create_dir_all(app.parent().unwrap()).unwrap();
    std::fs::write(
        &app,
        "#!/bin/sh\n\
         echo '2026-09-27 00:01:39.396 SweetpadCIMac[34229:21532494] ApplePersistenceIgnoreState: \
         Existing state will not be touched. New state will be written to \
         /var/folders/T/dev.sweetpad.ci.mac.savedState' >&2\n\
         echo 'stderr marker' >&2\n\
         exec sleep 60\n",
    )
    .unwrap();
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();
    (session, [home, cwd, derived])
}

/// AppKit's note that `-ApplePersistenceIgnoreState YES` took effect is about
/// sweetpad's launch when sweetpad added the argument, so the session leaves
/// it out of the app's output. Passed by the caller, the argument is the
/// caller's, and so is the note.
#[test]
fn the_session_hides_appkits_note_about_the_argument_sweetpad_added() {
    let note = "Existing state will not be touched";

    let (session, _dirs) = persistence_note_session("note-ours", &[]);
    let (status, shown) = on_pty(session, &[("stderr marker", "q")]);
    assert_eq!(status.code(), Some(0), "{shown}");
    assert!(!shown.contains(note), "{shown}");

    let (session, _dirs) = persistence_note_session(
        "note-theirs",
        &["--arg", "-ApplePersistenceIgnoreState", "--arg", "YES"],
    );
    let (status, shown) = on_pty(session, &[("stderr marker", "q")]);
    assert_eq!(status.code(), Some(0), "{shown}");
    assert!(shown.contains(note), "{shown}");
}

/// Copy the directory tree at `from` to `to`.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A copy of [`project`] beside a `sweetpad.toml`, and a stub xcodebuild
/// that records its arguments, one per line, in `argv.txt`. The stub fails
/// a second '-xcconfig' with the message xcodebuild gives it and otherwise
/// succeeds.
struct RecordingProject {
    home: TempDir,
    cwd: TempDir,
}

impl RecordingProject {
    fn new(tag: &str, sweetpad_toml: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let home = tmp(&format!("{tag}-home"));
        let cwd = tmp(&format!("{tag}-cwd"));
        copy_dir(&project(), &cwd.join("SweetpadCIApp.xcodeproj"));
        std::fs::write(cwd.join("sweetpad.toml"), sweetpad_toml).unwrap();
        std::fs::create_dir_all(cwd.join("bin")).unwrap();
        std::fs::create_dir_all(cwd.join("Developer")).unwrap();
        let stub = cwd.join("bin/xcodebuild");
        std::fs::write(
            &stub,
            format!(
                "#!/bin/sh\n\
                 printf '%s\\n' \"$@\" > '{}'\n\
                 n=0\n\
                 for a in \"$@\"; do [ \"$a\" = -xcconfig ] && n=$((n + 1)); done\n\
                 if [ $n -gt 1 ]; then\n\
                 echo \"xcodebuild: error: option '-xcconfig' may only be provided once\" >&2\n\
                 exit 64\n\
                 fi\n\
                 echo '** BUILD SUCCEEDED **'\n",
                cwd.join("argv.txt").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { home, cwd }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_sweetpad"))
            .args(args)
            .current_dir(&self.cwd)
            .env("HOME", &self.home)
            .env("XDG_STATE_HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.home)
            .env("XDG_CACHE_HOME", &self.home)
            .env("DEVELOPER_DIR", self.cwd.join("Developer"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.cwd.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .output()
            .unwrap()
    }

    /// The arguments the stub xcodebuild was last run with.
    fn argv(&self) -> Vec<String> {
        std::fs::read_to_string(self.cwd.join("argv.txt"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// xcodebuild fails a second '-xcconfig' with "option '-xcconfig' may only be
/// provided once", which the stub replays for any repeat. The one typed after
/// '--' replaces the one in `sweetpad.toml`, the rest of the file's arguments
/// still reach the build, and '-v' says what was left out.
#[test]
fn a_typed_xcconfig_replaces_the_one_in_sweetpad_toml() {
    let project = RecordingProject::new(
        "single-use",
        "[xcodebuild]\nargs = [\"-xcconfig\", \"a.xcconfig\", \"-skipMacroValidation\"]\n",
    );
    let build = |mode: &str| {
        project.run(&[
            "build",
            "--scheme",
            "SweetpadCIMac",
            "--configuration",
            "Debug",
            "--destination",
            "platform=macOS",
            "--non-interactive",
            mode,
            "--",
            "-xcconfig",
            "b.xcconfig",
        ])
    };

    let out = build("--json");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    let passed = project.argv();
    let at = passed.iter().position(|a| a == "-xcconfig").unwrap();
    assert_eq!(passed[at + 1], "b.xcconfig", "{passed:?}");
    assert_eq!(passed.iter().filter(|a| *a == "-xcconfig").count(), 1);
    assert!(
        passed.iter().any(|a| a == "-skipMacroValidation"),
        "{passed:?}"
    );
    assert!(!stderr.contains("leaving out"), "{stderr}");

    let out = build("-v");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(
        stderr.contains(
            "leaving out sweetpad.toml's '-xcconfig a.xcconfig': the '-xcconfig' typed after \
             '--' replaces it, and xcodebuild takes '-xcconfig' only once"
        ),
        "{stderr}"
    );
}

/// `settings show` takes a `--` tail the way a build does: a typed setting
/// wins over the file's, and a typed `-xcconfig` replaces the file's.
#[test]
fn settings_show_previews_a_typed_tail() {
    let project = RecordingProject::new(
        "settings-tail",
        "[xcodebuild]\nargs = [\"PRODUCT_NAME=Alpha\", \"-xcconfig\", \"a.xcconfig\"]\n",
    );
    std::fs::write(project.cwd.join("a.xcconfig"), "SWEETPAD_FROM = a\n").unwrap();
    std::fs::write(project.cwd.join("b.xcconfig"), "SWEETPAD_FROM = b\n").unwrap();
    let show = |key: &str, tail: &[&str]| {
        let mut args = vec![
            "settings",
            "show",
            "--scheme",
            "SweetpadCIMac",
            "--configuration",
            "Debug",
            "--non-interactive",
            "--key",
            key,
        ];
        if !tail.is_empty() {
            args.push("--");
            args.extend_from_slice(tail);
        }
        let out = project.run(&args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    assert_eq!(show("PRODUCT_NAME", &[]), "Alpha");
    assert_eq!(show("PRODUCT_NAME", &["PRODUCT_NAME=Beta"]), "Beta");
    assert_eq!(show("SWEETPAD_FROM", &[]), "a");
    assert_eq!(show("SWEETPAD_FROM", &["-xcconfig", "b.xcconfig"]), "b");
}

/// `clean` takes the file's arguments, which can move the products, and
/// leaves out a flag `xcodebuild clean` fails on ("The flag
/// -enableCodeCoverage is only supported when testing").
#[test]
fn clean_takes_the_arguments_in_sweetpad_toml() {
    let project = RecordingProject::new(
        "clean-args",
        "[xcodebuild]\nargs = [\"SYMROOT=build\", \"-enableCodeCoverage\", \"YES\", \
         \"-xcconfig\", \"ci.xcconfig\"]\n",
    );
    let out = project.run(&[
        "clean",
        "--scheme",
        "SweetpadCIMac",
        "--configuration",
        "Debug",
        "--non-interactive",
        "--json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let passed = project.argv();
    assert_eq!(passed[0], "clean", "{passed:?}");
    assert!(passed.iter().any(|a| a == "SYMROOT=build"), "{passed:?}");
    assert!(
        passed.windows(2).any(|w| w == ["-xcconfig", "ci.xcconfig"]),
        "{passed:?}"
    );
    assert!(
        !passed
            .iter()
            .any(|a| a == "-enableCodeCoverage" || a == "YES"),
        "{passed:?}"
    );
}

/// A session exits by how it ends. Ctrl-C while a build runs cancels it,
/// exit 6, whether or not the app ran before; a quit at the prompt, by 'q'
/// or by Ctrl-C, exits 0 once the app has run.
#[test]
fn a_session_exits_by_how_it_ended() {
    let endings: [(&str, u32, &Steps, i32); 4] = [
        ("end-q", 2, &[("h keys", "q")], 0),
        ("end-ctrl-c", 2, &[("h keys", "\u{3}")], 0),
        (
            "end-rebuild",
            2,
            &[("h keys", "r"), ("Compiling", "\u{3}")],
            6,
        ),
        ("end-first-build", 1, &[("Compiling", "\u{3}")], 6),
    ];
    for (tag, slow_from, steps, code) in endings {
        let (session, _dirs) = launchable_session(tag, slow_from);
        let (status, shown) = on_pty(session, steps);
        assert_eq!(status.code(), Some(code), "{tag}: {shown}");
        if code == 6 {
            assert!(shown.contains("Build cancelled"), "{tag}: {shown}");
        } else {
            assert!(shown.contains("Launched in"), "{tag}: {shown}");
        }
    }
}
