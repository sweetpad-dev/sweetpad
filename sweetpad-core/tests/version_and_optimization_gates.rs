//! Oracle-grounded regression tests for two families of resolution bugs found
//! by auditing the resolver against the per-target / custom-configuration
//! captures:
//!
//! 1. **Xcode-version gates.** Several built-in rules were calibrated on the
//!    16.4/26.5 corpus and silently misfired on Xcode 15.4, whose captures
//!    disagree: no synthesized `$(BUILT_PRODUCTS_DIR)` search paths, no
//!    device-platform bitcode strip, `STRIP_INSTALLED_PRODUCT = YES` even in
//!    Debug, simulator-first `SUPPORTED_PLATFORMS` pair ordering for the
//!    iPhone/Watch families, `ENABLE_PREVIEWS = YES` in Release, an
//!    `ONLY_ACTIVE_ARCH` ARCHS collapse even with no destination, no `armv7k`
//!    in ARCHS, and no swift-testing `-plugin-path`.
//!
//! 2. **The "debug build" gate is the optimization level, not the config
//!    name.** `STRIP_INSTALLED_PRODUCT`, `GCC_SYMBOLS_PRIVATE_EXTERN`,
//!    `ENABLE_PREVIEWS`, `LD_EXPORT_GLOBAL_SYMBOLS`, and the effective
//!    `ONLY_ACTIVE_ARCH` default key on the resolved
//!    `GCC_OPTIMIZATION_LEVEL = 0`, NOT on the configuration being named
//!    `Debug`: the `_synthetic-custom-config` fixture's template-less `Debug`
//!    gets the optimized (Release-shaped) values from xcodebuild on every
//!    captured version.
//!
//! Plus one cross-version gap: XCTest bundles gain `$(inherited)
//! $(TEST_LIBRARY_SEARCH_PATHS)` on `LIBRARY_SEARCH_PATHS` *above* the user
//! layers, on every captured version.
//!
//! Every assertion below is the captured `xcodebuild -showBuildSettings`
//! value for that exact (project, target, configuration, sdk).

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use sweetpad_core::build_context::{BuildContext, ResolveQuery};
use sweetpad_lib::xcspec;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("SWEETPAD_LIB_DIR")).join("fixtures")
}

/// Resolve `(target, config, sdk)` in `project_rel` (under `fixtures/`)
/// against the cached xcspec catalog for `version`, with the host pinned to
/// the corpus capture machine (arm64).
fn resolve(
    version: &str,
    project_rel: &str,
    target: &str,
    config: &str,
    sdk: &str,
) -> BTreeMap<String, String> {
    common::pin_capture_host();
    let catalog = xcspec::load_catalog(
        &common::xcspec_root_for(version),
        Some(&common::sdksettings_root_for(version)),
    )
    .unwrap();
    let ctx = BuildContext::open(&fixtures_root().join(project_rel))
        .unwrap()
        .with_xcspec(catalog);
    ctx.resolve(&ResolveQuery::new(target, config, sdk, "arm64"))
        .unwrap()
        .settings
}

/// The captured `buildSettings` map of a single-entry per-target capture.
fn oracle(capture_rel: &str) -> BTreeMap<String, String> {
    common::read_build_settings(&fixtures_root().join(capture_rel))
        .unwrap()
        .remove(0)
}

fn get<'a>(map: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    map.get(key).map_or("", String::as_str)
}

/// Xcode 15.4, tuist watchapp2 `App` (iphoneos application): the four
/// version-gated defaults the 16+-calibrated rules used to get wrong, checked
/// against the capture, plus the 26.5 capture of the same fixture to guard
/// against overcorrecting the modern path.
#[test]
fn xcode15_device_defaults_match_the_captures() {
    let proj = "tuist-fixtures/xcode-15.4.0/raw/examples_xcode_generated_ios_app_with_watchapp2/App.xcodeproj";
    let cap = "tuist-fixtures/xcode-15.4.0/metadata/examples_xcode_generated_ios_app_with_watchapp2/_per_target/App";

    let ours = resolve("15.4.0", proj, "App", "Debug", "iphoneos");
    let want = oracle(&format!("{cap}/App__Debug.json"));
    for key in [
        // 15.4 has no device bitcode strip (CoreBuildSystem default NO).
        "STRIP_BITCODE_FROM_COPIED_FILES",
        // 15.4 reports the plain YES default even for an unoptimized build.
        "STRIP_INSTALLED_PRODUCT",
        // 15.4 orders the iPhone pair simulator-first.
        "SUPPORTED_PLATFORMS",
        // 15.4 collapses ARCHS to the build machine's arch under
        // ONLY_ACTIVE_ARCH=YES even with no destination bound.
        "ARCHS",
    ] {
        assert_eq!(get(&ours, key), get(&want, key), "15.4 Debug {key}");
    }

    // 15.4 keeps ENABLE_PREVIEWS=YES in the optimized Release config.
    let ours = resolve("15.4.0", proj, "App", "Release", "iphoneos");
    let want = oracle(&format!("{cap}/App__Release.json"));
    assert_eq!(
        get(&ours, "ENABLE_PREVIEWS"),
        get(&want, "ENABLE_PREVIEWS"),
        "15.4 Release ENABLE_PREVIEWS"
    );
    // No synthesized $(BUILT_PRODUCTS_DIR) search paths on 15.4: the capture
    // carries no FRAMEWORK/HEADER_SEARCH_PATHS for this target, and ours must
    // not invent a Build/Products entry.
    for key in ["FRAMEWORK_SEARCH_PATHS", "HEADER_SEARCH_PATHS"] {
        assert!(
            !get(&ours, key).contains("Build/Products"),
            "15.4 must not synthesize a products-dir entry in {key}; got {:?}",
            get(&ours, key)
        );
    }

    // The same fixture under 26.5 keeps the modern values.
    let proj26 = "tuist-fixtures/xcode-26.5.0/raw/examples_xcode_generated_ios_app_with_watchapp2/App.xcodeproj";
    let cap26 = "tuist-fixtures/xcode-26.5.0/metadata/examples_xcode_generated_ios_app_with_watchapp2/_per_target/App";
    let ours = resolve("26.5.0", proj26, "App", "Debug", "iphoneos");
    let want = oracle(&format!("{cap26}/App__Debug.json"));
    for key in [
        "STRIP_BITCODE_FROM_COPIED_FILES", // YES on a 16+ device platform
        "STRIP_INSTALLED_PRODUCT",         // NO for an unoptimized build
        "SUPPORTED_PLATFORMS",             // device-first on 16+
        "ARCHS",                           // full standard list, no destination
    ] {
        assert_eq!(get(&ours, key), get(&want, key), "26.5 Debug {key}");
    }
}

/// Xcode 15.4, Kingfisher's watch demo (`WATCHOS_DEPLOYMENT_TARGET = 6.0`):
/// `ARCHS_STANDARD` keeps the pre-watchOS-9 `armv7k`, but 15.4's ARCHS view
/// drops it — Release reports `arm64 arm64_32` — and Debug collapses to the
/// build machine's arch with no destination bound.
#[test]
fn xcode15_archs_drop_armv7k_and_collapse_without_destination() {
    let proj = "kingfisher/xcode-15.4.0/raw/Demo/Kingfisher-Demo.xcodeproj";
    let cap = "kingfisher/xcode-15.4.0/metadata/_per_target/Demo_Kingfisher-Demo";

    let ours = resolve(
        "15.4.0",
        proj,
        "Kingfisher-watchOS-Demo",
        "Release",
        "watchos",
    );
    let want = oracle(&format!("{cap}/Kingfisher-watchOS-Demo__Release.json"));
    assert_eq!(get(&ours, "ARCHS"), get(&want, "ARCHS"), "Release ARCHS");
    assert_eq!(
        get(&ours, "ARCHS_STANDARD"),
        get(&want, "ARCHS_STANDARD"),
        "Release ARCHS_STANDARD"
    );

    let ours = resolve(
        "15.4.0",
        proj,
        "Kingfisher-watchOS-Demo",
        "Debug",
        "watchos",
    );
    let want = oracle(&format!("{cap}/Kingfisher-watchOS-Demo__Debug.json"));
    assert_eq!(get(&ours, "ARCHS"), get(&want, "ARCHS"), "Debug ARCHS");
}

/// The `_synthetic-custom-config` Scratch tool authors *no* optimization
/// settings, so even its `Debug` configuration is an optimized build —
/// xcodebuild reports the Release-shaped values for every "debug flip" key.
/// The old config-name-keyed rules got all of these wrong.
#[test]
fn unoptimized_flag_not_config_name_drives_the_debug_flips() {
    let proj = "_synthetic-custom-config/xcode-26.5.0/project/Scratch.xcodeproj";
    let cap = "_synthetic-custom-config/xcode-26.5.0/captures";

    for config in ["Debug", "Profile", "Release"] {
        let ours = resolve("26.5.0", proj, "Scratch", config, "macosx");
        let want = oracle(&format!("{cap}/Scratch__{config}.json"));
        for key in [
            // YES in every config: no GCC_OPTIMIZATION_LEVEL=0 anywhere.
            "GCC_SYMBOLS_PRIVATE_EXTERN",
            "STRIP_INSTALLED_PRODUCT",
            // The effective ONLY_ACTIVE_ARCH default follows the same gate,
            // so ARCHS never collapses for this project.
            "ARCHS",
            // A macOS tool with no team/no identity signs ad-hoc.
            "CODE_SIGN_IDENTITY",
            // Tools are exempt from the optimized-build dwarf-with-dsym
            // force: the capture reports plain `dwarf` in every config.
            "DEBUG_INFORMATION_FORMAT",
        ] {
            assert_eq!(get(&ours, key), get(&want, key), "26.5 {config} {key}");
        }
    }
}

/// XCTest bundles gain `$(inherited) $(TEST_LIBRARY_SEARCH_PATHS)` on
/// `LIBRARY_SEARCH_PATHS` — above the user layers — on every captured Xcode
/// version. On 16+/26 that lands after the synthesized products dir; on 15.4
/// (no synthesized base) the value is exactly the platform lib dir with the
/// empty-inherited double leading space.
#[test]
fn test_bundles_gain_the_platform_library_search_path() {
    // 26.5: products dir first, then the platform's Developer/usr/lib.
    let ours = resolve(
        "26.5.0",
        "kingfisher/xcode-26.5.0/raw/Kingfisher.xcodeproj",
        "KingfisherTests",
        "Debug",
        "macosx",
    );
    let v = get(&ours, "LIBRARY_SEARCH_PATHS");
    let tokens: Vec<&str> = v.split_whitespace().collect();
    assert_eq!(tokens.len(), 2, "products dir + platform lib dir: {v:?}");
    assert!(
        tokens[0].ends_with("/Build/Products/Debug"),
        "first entry is the products dir: {v:?}"
    );
    assert!(
        tokens[1].ends_with("/Platforms/MacOSX.platform/Developer/usr/lib"),
        "second entry is the platform lib dir: {v:?}"
    );

    // 15.4: no synthesized products entry — just the platform lib dir, with
    // the capture's two-leading-space shape ("$(inherited) $(TEST_…)" over an
    // empty inherited and the SDK default's own leading space).
    let ours = resolve(
        "15.4.0",
        "kingfisher/xcode-15.4.0/raw/Kingfisher.xcodeproj",
        "KingfisherTests",
        "Debug",
        "macosx",
    );
    let v = get(&ours, "LIBRARY_SEARCH_PATHS");
    assert!(v.starts_with("  /"), "two leading spaces: {v:?}");
    let tokens: Vec<&str> = v.split_whitespace().collect();
    assert_eq!(tokens.len(), 1, "platform lib dir only: {v:?}");
    assert!(
        tokens[0].ends_with("/Platforms/MacOSX.platform/Developer/usr/lib"),
        "the platform lib dir: {v:?}"
    );
}

/// Test bundles get the swift-testing `-plugin-path` appended to
/// `OTHER_SWIFT_FLAGS` on Xcode 16+ only — 15.4 predates the toolchain
/// plugin and its captures carry the bare user flags.
#[test]
fn swift_testing_plugin_path_is_xcode16_plus_only() {
    let ours = resolve(
        "15.4.0",
        "tuist-fixtures/xcode-15.4.0/raw/examples_xcode_generated_ios_app_with_custom_configuration/App/App.xcodeproj",
        "AppTests",
        "debug",
        "iphoneos",
    );
    let want = oracle(
        "tuist-fixtures/xcode-15.4.0/metadata/examples_xcode_generated_ios_app_with_custom_configuration/_per_target/App_App/AppTests__debug.json",
    );
    assert_eq!(
        get(&ours, "OTHER_SWIFT_FLAGS"),
        get(&want, "OTHER_SWIFT_FLAGS"),
        "15.4 carries the user flags only"
    );

    let ours = resolve(
        "26.5.0",
        "tuist-fixtures/xcode-26.5.0/raw/examples_xcode_generated_ios_app_with_custom_configuration/App/App.xcodeproj",
        "AppTests",
        "debug",
        "iphoneos",
    );
    assert!(
        get(&ours, "OTHER_SWIFT_FLAGS").contains("-plugin-path"),
        "26.5 appends the swift-testing plugin path: {:?}",
        get(&ours, "OTHER_SWIFT_FLAGS")
    );
}
