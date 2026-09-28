//! Destination-platform oracle: the SDK one run destination builds each
//! target of a scheme for. `-destination platform=macOS` builds an iOS app
//! that doesn't support Mac Catalyst "Designed for iPad", on the `iphoneos`
//! SDK into `Debug-iphoneos`, and an app that does for Mac Catalyst.
//! xcodebuild picks one destination for the scheme from its app, so the
//! framework both schemes build follows the app either way, and a macOS app
//! beside a Designed-for-iPad app builds as it would for an iOS device. A
//! target that can't run on the destination builds for its own platform: an
//! iPhone simulator destination builds the macOS app of a mixed scheme for
//! `macosx`.
//!
//! The corpus oracles feed the resolver the SDK a capture reports, so they
//! never check which SDK a destination binds. This one resolves the way the
//! CLI does, from the scheme and the destination alone. Ground truth is a real
//! `xcodebuild -showBuildSettings -json -scheme <S> -configuration Debug
//! -destination <D>` capture of
//! `fixtures/_synthetic-destination-platforms/xcode-<ver>/project/DestPlatforms`,
//! named `<S>__Debug__<D>.json` (a simulator capture is made for a concrete
//! device and named for its platform).

mod common;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use sweetpad_core::build_settings::{BuildSettingsOptions, resolve_build_settings};
use sweetpad_lib::destination::parse_destination_arg;

use common::{
    capture_xcode_version, fixtures_root, read_build_settings, sdksettings_root_for,
    xcspec_root_for,
};

const FIXTURE_DIR: &str = "_synthetic-destination-platforms";

/// The keys that say which platform and SDK variant a target builds for, and
/// the ones a destination the target can't run on changes.
const PLATFORM_KEYS: &[&str] = &[
    "PLATFORM_NAME",
    "EFFECTIVE_PLATFORM_NAME",
    "SWIFT_PLATFORM_TARGET_PREFIX",
    "LLVM_TARGET_TRIPLE_OS_VERSION",
    "LLVM_TARGET_TRIPLE_SUFFIX",
    "IS_MACCATALYST",
    "SUPPORTED_PLATFORMS",
    "ARCHS",
    "ONLY_ACTIVE_ARCH",
    "BUILD_ACTIVE_RESOURCES_ONLY",
    "__IS_NOT_SIMULATOR",
];

/// Every `<scheme>__<config>__<destination>.json` capture under
/// `fixtures/_synthetic-destination-platforms/*/captures/`.
fn capture_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(versions) = std::fs::read_dir(fixtures_root().join(FIXTURE_DIR)) else {
        return out;
    };
    for version in versions.flatten() {
        let Ok(entries) = std::fs::read_dir(version.path().join("captures")) else {
            continue;
        };
        out.extend(
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension() == Some(OsStr::new("json"))),
        );
    }
    out.sort();
    out
}

/// The scheme, configuration and destination a capture file is named for.
fn capture_request(capture: &Path) -> (String, String, String) {
    let stem = capture.file_stem().and_then(OsStr::to_str).unwrap();
    let mut parts = stem.splitn(3, "__");
    let mut next = || parts.next().unwrap().to_string();
    (next(), next(), next())
}

/// The part of a products path below `Build/Products/`: `Debug-iphoneos`,
/// `Debug-maccatalyst`.
fn products_leaf(path: &str) -> &str {
    path.rsplit_once("/Build/Products/")
        .map_or(path, |(_, leaf)| leaf)
}

#[test]
fn a_destination_binds_each_target_to_the_sdk_xcodebuild_builds_it_with() {
    common::pin_capture_host();
    let captures = capture_files();
    assert!(
        !captures.is_empty(),
        "no {FIXTURE_DIR} captures found — fixture missing?"
    );
    for capture in &captures {
        let version = capture_xcode_version(capture).unwrap();
        let (scheme, configuration, destination) = capture_request(capture);
        let root = capture.parent().unwrap().parent().unwrap();
        let opts = BuildSettingsOptions {
            project: Some(root.join("project/DestPlatforms/DestPlatforms.xcodeproj")),
            scheme: Some(scheme.clone()),
            configuration,
            destination: parse_destination_arg(&destination),
            xcspec_root: Some(xcspec_root_for(&version)),
            sdksettings_root: Some(sdksettings_root_for(&version)),
            ..BuildSettingsOptions::default()
        };
        let resolved = resolve_build_settings(&opts)
            .unwrap_or_else(|e| panic!("{scheme} [{destination}]: {e}"));
        let entries = read_build_settings(capture).unwrap();
        for theirs in &entries {
            let target = &theirs["TARGET_NAME"];
            let ours = &resolved
                .iter()
                .find(|t| t.target == *target)
                .unwrap_or_else(|| panic!("{scheme}: no {target} resolved"))
                .settings;
            for key in PLATFORM_KEYS {
                // Where xcodebuild leaves a Catalyst key unset, the resolver
                // reports it off. Any other key xcodebuild doesn't report is
                // out of scope.
                let catalyst = matches!(*key, "IS_MACCATALYST" | "LLVM_TARGET_TRIPLE_SUFFIX");
                if !catalyst && !theirs.contains_key(*key) {
                    continue;
                }
                let theirs = theirs.get(*key).map_or("", String::as_str);
                let ours = ours.get(*key).map_or("", String::as_str);
                let ours = if theirs.is_empty() && ours == "NO" {
                    ""
                } else {
                    ours
                };
                assert_eq!(ours, theirs, "{scheme}/{target} [{destination}] {key}");
            }
            assert_eq!(
                products_leaf(&ours["TARGET_BUILD_DIR"]),
                products_leaf(&theirs["TARGET_BUILD_DIR"]),
                "{scheme}/{target} [{destination}] TARGET_BUILD_DIR"
            );
        }
    }
}
