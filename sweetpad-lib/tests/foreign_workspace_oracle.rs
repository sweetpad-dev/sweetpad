//! Foreign-workspace DerivedData oracle: a bare `.xcodeproj` nested under an
//! UNRELATED `.xcworkspace` (one it is not a member of) must key its DerivedData
//! folder by the PROJECT — exactly as `xcodebuild -project` does — never by the
//! neighbouring workspace.
//!
//! Regression for the `app run` failure where the build wrote the `.app` to
//! `Hello-<hash>/…` but the install looked under the foreign `Pulse-<hash>/…`
//! and reported the bundle missing. Ground truth is a real
//! `xcodebuild -scheme Hello -showBuildSettings` capture
//! (`fixtures/_synthetic-foreign-workspace/`) taken with `Pulse.xcworkspace`
//! sitting in the project's grandparent directory. The DerivedData hash is
//! path-derived (so it differs at this checkout), but the folder *name* and the
//! whole build-dir geometry must match the capture once canonicalized.

mod common;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use sweetpad::build_context::{BuildContext, ResolveQuery};

use common::{
    CatalogCache, canonicalize_value, capture_xcode_version, fixtures_root, read_build_settings,
};

const FIXTURE_DIR: &str = "_synthetic-foreign-workspace";

/// Every per-config capture under `fixtures/_synthetic-foreign-workspace/*/captures/`
/// (excluding `meta.json`).
fn capture_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let root = fixtures_root().join(FIXTURE_DIR);
    let Ok(versions) = std::fs::read_dir(&root) else {
        return out;
    };
    for ver in versions.flatten() {
        let captures = ver.path().join("captures");
        let Ok(entries) = std::fs::read_dir(&captures) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension() == Some(OsStr::new("json"))
                && p.file_name() != Some(OsStr::new("meta.json"))
            {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The nested project sits at `<ver>/project/Hello/Hello.xcodeproj` — under the
/// foreign `<ver>/project/Pulse.xcworkspace`, two directories up.
fn project_for(capture: &Path) -> Option<PathBuf> {
    let ver_root = capture.parent()?.parent()?; // captures/ -> <ver>/
    let proj = ver_root.join("project/Hello/Hello.xcodeproj");
    proj.is_dir().then_some(proj)
}

#[test]
fn foreign_workspace_keys_derived_data_by_the_project() {
    common::pin_capture_host();
    let captures = capture_files();
    assert!(
        !captures.is_empty(),
        "no _synthetic-foreign-workspace captures found"
    );

    let mut catalogs = CatalogCache::new();
    for capture in &captures {
        let version = capture_xcode_version(capture)
            .unwrap_or_else(|| panic!("capture path lacks xcode-<ver>: {}", capture.display()));
        let entries = read_build_settings(capture)
            .unwrap_or_else(|| panic!("unreadable: {}", capture.display()));
        let bs = entries
            .first()
            .unwrap_or_else(|| panic!("empty capture: {}", capture.display()));

        let target = bs.get("TARGET_NAME").expect("TARGET_NAME");
        let config = bs.get("CONFIGURATION").expect("CONFIGURATION");
        let sdk = bs.get("PLATFORM_NAME").expect("PLATFORM_NAME");
        let arch = bs
            .get("NATIVE_ARCH_ACTUAL")
            .or_else(|| bs.get("HOST_ARCH"))
            .map_or("arm64", String::as_str);

        let xcodeproj =
            project_for(capture).unwrap_or_else(|| panic!("no project for {}", capture.display()));
        let catalog = catalogs.get(&version).clone();
        let ctx = BuildContext::open(&xcodeproj)
            .unwrap_or_else(|e| panic!("open {}: {e}", xcodeproj.display()))
            .with_xcspec(catalog);
        let resolved = ctx
            .resolve(&ResolveQuery::new(target, config, sdk, arch))
            .unwrap_or_else(|e| panic!("resolve: {e}"))
            .settings;

        // The DerivedData-anchored build dirs must match the real capture once
        // the path-derived hash and host root are canonicalized away — which
        // keeps the folder *name*. That name is the project (`Hello`), never the
        // foreign workspace (`Pulse`): the bug was the resolver adopting `Pulse`.
        for key in ["TARGET_BUILD_DIR", "BUILD_DIR"] {
            let ours = resolved
                .get(key)
                .unwrap_or_else(|| panic!("resolver missing {key}"));
            let oracle = bs
                .get(key)
                .unwrap_or_else(|| panic!("oracle missing {key}"));
            assert_eq!(
                canonicalize_value(ours),
                canonicalize_value(oracle),
                "{key}: resolver {ours:?} != oracle {oracle:?} (canonicalized)"
            );
            assert!(
                ours.contains("/DerivedData/Hello-"),
                "{key} must be keyed under the project (Hello-<hash>); got {ours}"
            );
            assert!(
                !ours.contains("/DerivedData/Pulse-"),
                "{key} must NOT be keyed under the foreign workspace (Pulse): {ours}"
            );
        }
    }
}
