//! Custom-`SYMROOT` oracle: a project whose xcconfig relocates the build
//! products (`SYMROOT = $(SRCROOT)/../build/products`) must resolve its product
//! directories to follow that `SYMROOT`, exactly as `xcodebuild` does — never to
//! the default `<DerivedData>/Build/Products` layout.
//!
//! Regression for sweetpad issue #292: the build wrote `Hello.app` under the
//! custom `SYMROOT`, but the launcher computed `TARGET_BUILD_DIR` from the
//! hard-wired DerivedData `BUILD_DIR` and reported `App path does not exist`.
//! Ground truth is a real `xcodebuild -xcconfig Custom.xcconfig
//! -showBuildSettings` capture (`fixtures/_synthetic-symroot-override/`): with a
//! custom `SYMROOT`, xcodebuild makes `BUILD_DIR`, `BUILD_ROOT`,
//! `CONFIGURATION_BUILD_DIR`, `BUILT_PRODUCTS_DIR`, and `TARGET_BUILD_DIR` all
//! follow it, while `OBJROOT` stays under DerivedData.
//!
//! The fixture lives at a path that differs per checkout, so both sides are
//! lexically normalized (collapse `.`/`..`, then replace the prefix through the
//! `/fixtures/<slug>/` marker) before comparison.

mod common;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use sweetpad::build_context::{BuildContext, ResolveQuery};

use common::{CatalogCache, capture_xcode_version, fixtures_root, read_build_settings};

const FIXTURE_DIR: &str = "_synthetic-symroot-override";

/// The product-directory keys that must follow a custom `SYMROOT`.
const PRODUCT_DIR_KEYS: &[&str] = &[
    "SYMROOT",
    "BUILD_DIR",
    "BUILD_ROOT",
    "CONFIGURATION_BUILD_DIR",
    "BUILT_PRODUCTS_DIR",
    "TARGET_BUILD_DIR",
];

/// Every per-config capture under
/// `fixtures/_synthetic-symroot-override/*/captures/` (excluding `meta.json`).
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

/// The project + its baseConfiguration-relocating xcconfig live at
/// `<ver>/project/Hello/…`, two directories above `captures/`.
fn project_and_xcconfig(capture: &Path) -> Option<(PathBuf, PathBuf)> {
    let ver_root = capture.parent()?.parent()?; // captures/ -> <ver>/
    let proj = ver_root.join("project/Hello/Hello.xcodeproj");
    let xcconfig = ver_root.join("project/Hello/Custom.xcconfig");
    (proj.is_dir() && xcconfig.is_file()).then_some((proj, xcconfig))
}

/// Lexically collapse `.`/`..` segments, then replace the absolute prefix up to
/// and including `/fixtures/<slug>/` with `<FIX>/`, so a value captured at one
/// checkout compares equal to the resolver's output at another. xcodebuild
/// already standardizes the path; the resolver may leave a `..` from the
/// `$(SRCROOT)/..` xcconfig, so collapse both sides the same way.
fn normalize(value: &str) -> String {
    let collapsed = lexically_normalize(value);
    let marker = format!("/fixtures/{FIXTURE_DIR}/");
    match collapsed.find(&marker) {
        Some(idx) => format!("<FIX>/{}", &collapsed[idx + marker.len()..]),
        None => collapsed,
    }
}

/// Pure-lexical path cleanup (no filesystem access): drop `.` segments and
/// resolve `..` against the preceding segment, preserving a leading `/`.
fn lexically_normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut stack: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if matches!(stack.last(), Some(&s) if s != "..") {
                    stack.pop();
                } else if !absolute {
                    stack.push("..");
                }
            }
            other => stack.push(other),
        }
    }
    let joined = stack.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

#[test]
fn custom_symroot_relocates_product_directories() {
    common::pin_capture_host();
    let captures = capture_files();
    assert!(
        !captures.is_empty(),
        "no {FIXTURE_DIR} captures found — fixture missing?"
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

        let (xcodeproj, xcconfig) = project_and_xcconfig(capture)
            .unwrap_or_else(|| panic!("no project/xcconfig for {}", capture.display()));
        let catalog = catalogs.get(&version).clone();
        let resolved = BuildContext::open(&xcodeproj)
            .unwrap_or_else(|e| panic!("open {}: {e}", xcodeproj.display()))
            .with_xcspec(catalog)
            .with_extra_xcconfig(&xcconfig)
            .unwrap_or_else(|e| panic!("layer xcconfig {}: {e}", xcconfig.display()))
            .resolve(&ResolveQuery::new(target, config, sdk, arch))
            .unwrap_or_else(|e| panic!("resolve: {e}"))
            .settings;

        // Every product-directory key must match the real xcodebuild capture
        // (which placed them all under the custom SYMROOT, not DerivedData).
        for key in PRODUCT_DIR_KEYS {
            let ours = resolved
                .get(*key)
                .unwrap_or_else(|| panic!("resolver missing {key}"));
            let oracle = bs
                .get(*key)
                .unwrap_or_else(|| panic!("oracle missing {key}"));
            assert_eq!(
                normalize(ours),
                normalize(oracle),
                "{key}: resolver {ours:?} != oracle {oracle:?} \
                 — a custom SYMROOT must relocate the product directory (issue #292)"
            );
            assert!(
                !ours.contains("/DerivedData/"),
                "{key} must follow the custom SYMROOT, not DerivedData; got {ours:?}"
            );
        }

        // OBJROOT is independent of SYMROOT: xcodebuild keeps intermediates under
        // DerivedData even when the products move. Pin that asymmetry so a fix
        // that naively repoints *everything* at SYMROOT would be caught.
        let objroot = resolved.get("OBJROOT").expect("resolver missing OBJROOT");
        assert!(
            objroot.contains("/DerivedData/"),
            "OBJROOT must stay under DerivedData even with a custom SYMROOT; got {objroot:?}"
        );
    }
}
