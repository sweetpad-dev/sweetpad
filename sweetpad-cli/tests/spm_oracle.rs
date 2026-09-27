//! SwiftPM CLI oracle — ground sweetpad's manifest-derived project view against
//! real xcodebuild/swift for a bare `Package.swift`.
//!
//! sweetpad reads a package's structure from `swift package dump-package`
//! ([`sweetpad_cli::cli::swiftpm`]), never xcodebuild. This oracle proves that view
//! matches what xcodebuild actually synthesizes, and that the `swift`-driven
//! build/test path succeeds on a real package:
//!
//! - **fixture mode** (default): compare our `scheme_names()` (parsed from a
//!   captured `dump-package.json`) against the captured `xcodebuild -list`
//!   schemes, and check the captured `swift build`/`swift test` succeeded.
//!   Skips cleanly when no captures exist (e.g. on a non-macOS host), so it
//!   never fails a Linux/CI run — capture with `scripts/22_spm_cli_oracle.py`.
//! - **live mode** (`SPM_LIVE_ORACLE=1`): run our `swiftpm::schemes()` and
//!   `xcodebuild -list -json` against the sample package in real time and
//!   compare. Requires macOS + Xcode + the Swift toolchain.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::TempDir;
use sweetpad_cli::cli::resolve::Container;
use sweetpad_cli::cli::swiftpm::{self, Manifest};

fn fixtures_root() -> PathBuf {
    Path::new(env!("SWEETPAD_LIB_DIR")).join("fixtures/_synthetic-spm-cli")
}

/// Capture directories that have both the xcodebuild and dump-package captures.
fn capture_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Ok(entries) = std::fs::read_dir(fixtures_root()) else {
        return dirs;
    };
    for e in entries.flatten() {
        let captures = e.path().join("captures");
        if captures.is_dir() {
            dirs.push(captures);
        }
    }
    dirs
}

/// Schemes from an `xcodebuild -list -json` payload — a package is reported
/// under the `workspace` key, a bare project under `project`.
fn xcodebuild_list_schemes(json: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(json).expect("list json is valid");
    v.get("workspace")
        .or_else(|| v.get("project"))
        .and_then(|b| b.get("schemes"))
        .and_then(|s| s.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn our_schemes_from_dump(dump: &str) -> Vec<String> {
    let manifest: Manifest = serde_json::from_str(dump).expect("dump-package json deserializes");
    manifest.scheme_names()
}

#[test]
fn schemes_match_captured_xcodebuild_list() {
    let dirs: Vec<PathBuf> = capture_dirs()
        .into_iter()
        .filter(|d| d.join("list.json").exists() && d.join("dump-package.json").exists())
        .collect();
    if dirs.is_empty() {
        eprintln!(
            "skipping: no SPM captures under {} — run scripts/22_spm_cli_oracle.py on macOS",
            fixtures_root().display()
        );
        return;
    }

    let mut failures = Vec::new();
    for dir in &dirs {
        let list = std::fs::read_to_string(dir.join("list.json")).unwrap();
        let dump = std::fs::read_to_string(dir.join("dump-package.json")).unwrap();
        let theirs: BTreeSet<String> = xcodebuild_list_schemes(&list).into_iter().collect();
        let ours: BTreeSet<String> = our_schemes_from_dump(&dump).into_iter().collect();
        if ours != theirs {
            failures.push(format!(
                "{}: ours={ours:?} xcodebuild={theirs:?} (missing={:?}, extra={:?})",
                dir.display(),
                theirs.difference(&ours).collect::<Vec<_>>(),
                ours.difference(&theirs).collect::<Vec<_>>(),
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "SPM scheme oracle diverged from xcodebuild:\n{}",
        failures.join("\n")
    );
}

#[test]
fn captured_configurations_are_debug_release() {
    // sweetpad reports a package's configurations as Debug/Release (SwiftPM has
    // no others). When xcodebuild's -list block carries configurations, ground
    // that assumption against it.
    let dirs: Vec<PathBuf> = capture_dirs()
        .into_iter()
        .filter(|d| d.join("list.json").exists())
        .collect();
    if dirs.is_empty() {
        eprintln!("skipping: no SPM captures — run scripts/22_spm_cli_oracle.py on macOS");
        return;
    }

    let allowed: BTreeSet<String> = ["Debug".to_string(), "Release".to_string()]
        .into_iter()
        .collect();
    for dir in &dirs {
        let list = std::fs::read_to_string(dir.join("list.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&list).unwrap();
        let configs: BTreeSet<String> = v
            .get("workspace")
            .or_else(|| v.get("project"))
            .and_then(|b| b.get("configurations"))
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // Packages often report no configurations at all; only assert when present.
        assert!(
            configs.is_empty() || configs.is_subset(&allowed),
            "{}: xcodebuild reports configurations {configs:?} outside Debug/Release",
            dir.display()
        );
    }
}

#[test]
fn captured_build_and_test_succeeded() {
    // Grounds that the `swift build` / `swift test` invocations sweetpad models
    // actually succeed on a real package (status captured by the script).
    let dirs = capture_dirs();
    let mut checked = 0;
    for dir in &dirs {
        for action in ["build.json", "test.json"] {
            let path = dir.join(action);
            if !path.exists() {
                continue;
            }
            let v: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(
                v.get("ok").and_then(serde_json::Value::as_bool),
                Some(true),
                "{}: {action} did not succeed: {v}",
                dir.display()
            );
            checked += 1;
        }
    }
    if checked == 0 {
        eprintln!(
            "skipping: no SPM build/test captures — run scripts/22_spm_cli_oracle.py on macOS"
        );
    }
}

#[test]
fn live_schemes_match_xcodebuild() {
    if std::env::var("SPM_LIVE_ORACLE").is_err() {
        eprintln!(
            "skipping: set SPM_LIVE_ORACLE=1 to run the live SPM oracle (needs macOS + Xcode + swift)"
        );
        return;
    }
    let manifest_path = fixtures_root().join("project/Package.swift");
    if !manifest_path.exists() {
        eprintln!(
            "skipping: sample package missing at {}",
            manifest_path.display()
        );
        return;
    }
    let project_dir = manifest_path.parent().unwrap().to_path_buf();
    let container = Container::SwiftPackage(manifest_path);

    // Ours: our `schemes()` runs `swift package dump-package` via the real toolchain.
    let ours: BTreeSet<String> = swiftpm::schemes(&container)
        .expect("our schemes() should run swift package dump-package")
        .into_iter()
        .collect();

    // Theirs: xcodebuild -list -json from the package dir.
    let out = Command::new("xcodebuild")
        .args(["-list", "-json"])
        .current_dir(&project_dir)
        .output()
        .expect("run xcodebuild -list");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json = &stdout[stdout.find('{').expect("xcodebuild -list emits JSON")..];
    let theirs: BTreeSet<String> = xcodebuild_list_schemes(json).into_iter().collect();

    assert_eq!(
        ours, theirs,
        "live SPM scheme mismatch: ours={ours:?} xcodebuild={theirs:?}"
    );
}

/// Reading a package's manifest leaves nothing behind. `swift package
/// dump-package` makes a `.build/` in the package, and the manifest compile a
/// `TemporaryDirectory.*` and a lock file in `$TMPDIR`, unless it runs with a
/// scratch path and `TMPDIR` of its own.
#[test]
fn reading_a_manifest_leaves_nothing_behind() {
    let root = TempDir::new("sweetpad-spm-manifest");
    let probe = root.join("probe-tmp");
    std::fs::create_dir_all(&probe).unwrap();
    let have_swift = Command::new("swift")
        .arg("--version")
        .env("TMPDIR", &probe)
        .output()
        .is_ok_and(|out| out.status.success());
    if !have_swift {
        eprintln!("skipping: needs the Swift toolchain to evaluate a manifest");
        return;
    }
    let package = root.join("Dumped");
    let temp = root.join("tmp");
    std::fs::create_dir_all(package.join(".git")).unwrap();
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::write(
        package.join("Package.swift"),
        "// swift-tools-version:5.9\nimport PackageDescription\n\
         let package = Package(name: \"Dumped\")\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(["project", "info", "--json"])
        .current_dir(&package)
        .env("XDG_STATE_HOME", &root)
        .env("XDG_CONFIG_HOME", &root)
        .env("XDG_CACHE_HOME", &root)
        .env("TMPDIR", &temp)
        .output()
        .expect("failed to run the sweetpad binary");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let info: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(info["data"]["name"], "Dumped", "{info}");

    assert!(!package.join(".build").exists(), "a .build/ in the package");
    let left: Vec<_> = std::fs::read_dir(&temp)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "left in TMPDIR: {left:?}");
}

/// A package's manifest is read with the Xcode its `sweetpad.toml` pins, also
/// when the package is found below the working directory, where nothing else
/// loads that file before the read. The pinned Xcode is a stub whose `xcrun`,
/// which the `swift` shim in `/usr/bin` hands every call to, answers
/// `dump-package` with a manifest of its own.
#[test]
fn a_manifest_is_read_with_the_xcode_its_sweetpad_toml_pins() {
    use std::os::unix::fs::PermissionsExt;

    if !Path::new("/usr/bin/swift").exists() {
        eprintln!("skipping: needs the /usr/bin/swift shim that follows DEVELOPER_DIR");
        return;
    }
    let root = TempDir::new("sweetpad-spm-pinned-xcode");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let xcrun = root.join("Pinned.app/Contents/Developer/usr/bin/xcrun");
    std::fs::create_dir_all(xcrun.parent().unwrap()).unwrap();
    std::fs::write(
        &xcrun,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *dump-package*) echo '{\"name\": \"FromPinnedXcode\", \"dependencies\": \
         [{\"fileSystem\": [{\"identity\": \"pinned-dep\", \"path\": \"/pinned/dep\"}]}]}' ;;\n\
         *) exit 1 ;;\n\
         esac\n",
    )
    .unwrap();
    std::fs::set_permissions(&xcrun, std::fs::Permissions::from_mode(0o755)).unwrap();
    let package = root.join("packages/Dumped");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("Package.swift"),
        "// swift-tools-version:5.9\nimport PackageDescription\n\
         let package = Package(name: \"FromDefaultXcode\")\n",
    )
    .unwrap();
    std::fs::write(
        package.join("sweetpad.toml"),
        format!(
            "developer_dir = \"{}\"\n",
            root.join("Pinned.app/Contents/Developer").display()
        ),
    )
    .unwrap();
    let temp = root.join("tmp");
    std::fs::create_dir_all(&temp).unwrap();

    let sweetpad = |args: &[&str]| -> serde_json::Value {
        let out = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
            .args(args)
            .current_dir(&*root)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env_remove("DEVELOPER_DIR")
            .env("XDG_STATE_HOME", &*root)
            .env("XDG_CONFIG_HOME", &*root)
            .env("XDG_CACHE_HOME", &*root)
            .env("TMPDIR", &temp)
            .output()
            .expect("failed to run the sweetpad binary");
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };

    let info = sweetpad(&["project", "info", "--json"]);
    assert_eq!(info["data"]["name"], "FromPinnedXcode", "{info}");
    let deps = sweetpad(&["dependency", "list", "--json"]);
    assert_eq!(
        deps["data"]["direct"][0]["identity"], "pinned-dep",
        "{deps}"
    );
}
