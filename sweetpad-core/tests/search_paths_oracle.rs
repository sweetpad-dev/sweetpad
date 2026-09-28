//! Products-dir search-path oracle: xcodebuild puts the products dir in front
//! of `FRAMEWORK_SEARCH_PATHS`, `HEADER_SEARCH_PATHS` (as `…/include`),
//! `LIBRARY_SEARCH_PATHS` and `REZ_SEARCH_PATHS` above every user layer, so a
//! target that authors them without `$(inherited)` still finds the frameworks
//! and headers its dependencies build. `Tool` authors all three that way and
//! imports the `Kit` framework it depends on: its Swift arguments need `-F`
//! for the products dir, where `Kit.framework` lands.
//!
//! Ground truth is a real `xcodebuild -showBuildSettings -json -scheme Tool
//! -configuration Debug -destination platform=macOS` capture of
//! `fixtures/_synthetic-search-paths/xcode-<ver>/project/SearchPaths`. Paths
//! are compared with each side's own `BUILT_PRODUCTS_DIR` and `SRCROOT`
//! replaced by placeholders, since the capture was taken in another checkout.

mod common;

use std::ffi::OsStr;
use std::path::PathBuf;

use sweetpad_core::build_settings::{
    BuildSettingsOptions, resolve_build_settings, resolve_file_arguments,
};
use sweetpad_lib::destination::parse_destination_arg;

use common::{
    capture_xcode_version, fixtures_root, read_build_settings, sdksettings_root_for,
    xcspec_root_for,
};

const FIXTURE_DIR: &str = "_synthetic-search-paths";

const SEARCH_PATH_KEYS: &[&str] = &[
    "FRAMEWORK_SEARCH_PATHS",
    "HEADER_SEARCH_PATHS",
    "LIBRARY_SEARCH_PATHS",
    "REZ_SEARCH_PATHS",
];

/// Every capture under `fixtures/_synthetic-search-paths/*/captures/`.
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

/// `value` with `settings`' own products dir and source root spelled
/// `<PRODUCTS>` and `<SRCROOT>`.
fn placeholders(value: &str, settings: &std::collections::BTreeMap<String, String>) -> String {
    value
        .replace(&settings["BUILT_PRODUCTS_DIR"], "<PRODUCTS>")
        .replace(&settings["SRCROOT"], "<SRCROOT>")
}

#[test]
fn authored_search_paths_keep_the_products_dir_in_front() {
    common::pin_capture_host();
    let captures = capture_files();
    assert!(
        !captures.is_empty(),
        "no {FIXTURE_DIR} captures found — fixture missing?"
    );
    for capture in &captures {
        let version = capture_xcode_version(capture).unwrap();
        let root = capture.parent().unwrap().parent().unwrap();
        let project = root.join("project/SearchPaths/SearchPaths.xcodeproj");
        let opts = BuildSettingsOptions {
            project: Some(project.clone()),
            scheme: Some("Tool".into()),
            configuration: "Debug".into(),
            destination: parse_destination_arg("platform=macOS"),
            xcspec_root: Some(xcspec_root_for(&version)),
            sdksettings_root: Some(sdksettings_root_for(&version)),
            ..BuildSettingsOptions::default()
        };
        let resolved = resolve_build_settings(&opts).unwrap();
        for theirs in &read_build_settings(capture).unwrap() {
            let target = &theirs["TARGET_NAME"];
            let ours = &resolved
                .iter()
                .find(|t| t.target == *target)
                .unwrap_or_else(|| panic!("no {target} resolved"))
                .settings;
            for key in SEARCH_PATH_KEYS {
                assert_eq!(
                    placeholders(&ours[*key], ours),
                    placeholders(&theirs[*key], theirs),
                    "{target} {key}"
                );
            }
        }

        // The editor's arguments for the file that imports `Kit`.
        let tool = resolved.iter().find(|t| t.target == "Tool").unwrap();
        let products = &tool.settings["BUILT_PRODUCTS_DIR"];
        let main = project.parent().unwrap().join("Sources/App/main.swift");
        let file_opts = BuildSettingsOptions {
            scheme: None,
            target: Some("Tool".into()),
            ..opts
        };
        let args = resolve_file_arguments(&file_opts, &main).unwrap().arguments;
        assert!(
            args.windows(2).any(|w| w[0] == "-F" && w[1] == *products),
            "no -F {products} in {args:?}"
        );
    }
}
