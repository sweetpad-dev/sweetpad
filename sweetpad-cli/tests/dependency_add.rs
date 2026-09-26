//! `dependency add` on a Swift package, driven against stub `swift` scripts
//! that log their argv: the argv shows how a local dependency reaches SwiftPM
//! and which product gets linked, and a stub that fails every package command
//! exercises the early exits that must not leave the crash-safe manifest
//! backup behind.

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sweetpad-dep-add-{tag}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    // Stop walk-up discovery at this directory.
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

fn sweetpad(args: &[&str], cwd: &Path, home: &Path, bin: &Path) -> Output {
    sweetpad_command(args, cwd, home, bin)
        .output()
        .expect("failed to run the sweetpad binary")
}

fn sweetpad_command(args: &[&str], cwd: &Path, home: &Path, bin: &Path) -> Command {
    let path_env = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sweetpad"));
    cmd.args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home)
        .env("PATH", path_env);
    cmd
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

/// Run `cmd` with a pty for its stdin, stdout and stderr, typing `keys` once
/// `prompt` shows. Returns the exit status and everything the child wrote.
fn answer_on_pty(mut cmd: Command, prompt: &str, keys: &[u8]) -> (ExitStatus, String) {
    use std::io::{Read, Write};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    let (mut master, slave) = pty();
    cmd.stdin(Stdio::from(slave.try_clone().unwrap()))
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

#[test]
fn a_failed_local_add_keeps_no_backup_and_names_a_path() {
    use std::os::unix::fs::PermissionsExt;

    let home = tmp("home");
    let root = tmp("root");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = root.join("swift.log");
    std::fs::write(
        bin.join("swift"),
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = --version ]; then echo 'Apple Swift version 6.1'; exit 0; fi\n\
             echo \"$*\" >> '{}'\n\
             exit 1\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(bin.join("swift"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let app = root.join("App");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(root.join("Dep")).unwrap();
    let manifest = "// swift-tools-version: 6.0\n";
    std::fs::write(app.join("Package.swift"), manifest).unwrap();
    let backup = app.join("Package.swift.sweetpad-rollback");

    for dependency in ["../Dep", "../Missing"] {
        let args = [
            "dep",
            "add",
            dependency,
            "--product",
            "Dep",
            "--target",
            "App",
            "--non-interactive",
        ];
        let out = sweetpad(&args, &app, &home, &bin);
        assert!(!out.status.success(), "{args:?}: expected a failure");
        assert!(
            !backup.exists(),
            "{args:?}: left {} behind",
            backup.display()
        );
        assert_eq!(
            std::fs::read_to_string(app.join("Package.swift")).unwrap(),
            manifest,
            "{args:?}: manifest changed"
        );
    }
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "package add-dependency ../Dep --type path\n",
        "only the existing directory reaches swift, as a path"
    );
}

/// `swift --version` writes the driver's version to stderr with no trailing
/// newline (Swift 6.4). Under `--json` that must not reach sweetpad's stderr,
/// where it would run into the error envelope.
#[test]
fn json_stderr_holds_only_the_error_envelope() {
    use std::os::unix::fs::PermissionsExt;

    let home = tmp("envelope-home");
    let root = tmp("envelope-root");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        bin.join("swift"),
        "#!/bin/sh\n\
         if [ \"$1\" = --version ]; then\n\
         echo 'Apple Swift version 6.4 (swiftlang-6.4.0.34.1 clang-2100.3.34.1)'\n\
         printf 'swift-driver version: 1.168.6 ' >&2\n\
         exit 0\n\
         fi\n\
         exit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(bin.join("swift"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let app = root.join("App");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(root.join("Dep")).unwrap();
    std::fs::write(app.join("Package.swift"), "// swift-tools-version: 6.0\n").unwrap();

    let args = [
        "dep",
        "add",
        "../Dep",
        "--product",
        "Dep",
        "--target",
        "App",
        "--json",
    ];
    let out = sweetpad(&args, &app, &home, &bin);
    assert!(!out.status.success(), "expected the stub's add to fail");
    let stderr = String::from_utf8(out.stderr).unwrap();
    let envelope: serde_json::Value = serde_json::from_str(&stderr)
        .unwrap_or_else(|e| panic!("stderr is not one JSON document ({e}): {stderr:?}"));
    assert_eq!(envelope["ok"], serde_json::Value::Bool(false));
    assert!(envelope["error"]["code"].is_string(), "{envelope}");
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&root);
}

/// An interactive local add with no `--product` offers the products the local
/// package's own manifest declares: SwiftPM reads a path dependency in place,
/// so `.build` holds no checkout of it. Run on a pty so the picker comes up,
/// then answered with space (select the one product) and return.
#[test]
fn an_interactive_local_add_picks_from_the_packages_manifest() {
    use std::os::unix::fs::PermissionsExt;

    let home = tmp("picker-home");
    let root = tmp("picker-root");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = root.join("swift.log");
    let dep_dump = r#"{"name":"Dep","products":[{"name":"DepKit","type":{"library":["automatic"]},"targets":["DepKit"]}],"targets":[{"name":"DepKit","type":"regular"}]}"#;
    let app_dump = r#"{"name":"App","products":[],"targets":[{"name":"App","type":"regular"}]}"#;
    std::fs::write(
        bin.join("swift"),
        format!(
            "#!/bin/sh\n\
             case \"$*\" in\n\
             --version) echo 'Apple Swift version 6.4' ;;\n\
             'package dump-package --package-path ../Dep') echo '{dep_dump}' ;;\n\
             'package dump-package') echo '{app_dump}' ;;\n\
             *) echo \"$*\" >> '{}' ;;\n\
             esac\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(bin.join("swift"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let app = root.join("App");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(root.join("Dep")).unwrap();
    std::fs::write(app.join("Package.swift"), "// swift-tools-version: 6.0\n").unwrap();

    let mut cmd = sweetpad_command(
        &["dep", "add", "../Dep", "--target", "App"],
        &app,
        &home,
        &bin,
    );
    cmd.env_remove("CI").env_remove("SWEETPAD_NONINTERACTIVE");
    let (status, shown) = answer_on_pty(cmd, "Select product(s)", b" \r");
    assert!(status.success(), "{status}:\n{shown}");
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains("package add-target-dependency DepKit App --package Dep\n"),
        "{calls}"
    );
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&root);
}
