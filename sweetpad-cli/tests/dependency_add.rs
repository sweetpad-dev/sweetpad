//! `dependency add` on a Swift package, driven against a stub `swift` that
//! logs its argv and fails every package command: the argv shows how a local
//! dependency reaches SwiftPM, and the failures exercise the early exits that
//! must not leave the crash-safe manifest backup behind.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
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
    let path_env = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home)
        .env("PATH", path_env)
        .output()
        .expect("failed to run the sweetpad binary")
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
