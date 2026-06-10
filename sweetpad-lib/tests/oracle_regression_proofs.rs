//! Per-key regression proofs pinned directly to captured oracle values.
//!
//! Each test replays ONE corpus capture (same harness as `corpus_oracle.rs`:
//! pinned host, destination parsed from the filename, the capture's own
//! xcspec catalog) and asserts a SPECIFIC key equals the captured
//! `xcodebuild -showBuildSettings` value byte-for-byte. They were written
//! red — every assertion failed against the resolver rules that predated
//! them — to prove these systematic divergences from xcodebuild:
//!
//! 1. `ASSETCATALOG_FILTER_FOR_DEVICE_OS_VERSION` came from a live `sw_vers`
//!    call, so it was empty on any non-mac host (118 oracle files).
//! 2. `ENABLE_DEBUG_DYLIB` was gated on the configuration name; xcodebuild
//!    keeps the product-type default `YES` in Release and only forces `NO`
//!    when the target authors `ENABLE_PREVIEWS` truthy and the build is
//!    optimized (16 files). The old code called this split "irreducible".
//! 3. `ENABLE_HARDENED_RUNTIME` ignored the previews coupling: with previews
//!    effective (authored `YES` + Debug) off-macOS, xcodebuild forces `NO`
//!    over the user's authored `YES` (10 files).
//! 4. `STRIP_INSTALLED_PRODUCT` is `YES` in every Xcode 15.x capture, Debug
//!    included; the Debug→`NO` rule only holds on 16+ (7 files).
//! 5. Catalyst clamps the reported `IPHONEOS_DEPLOYMENT_TARGET` to the 13.1
//!    Catalyst floor; we passed the user's 10.0/13.0 through (4 files).
//! 6. `DEBUG_INFORMATION_FORMAT`: (a) an EDD-bearing target without
//!    previews defaults to `dwarf-with-dsym` even in Debug with a bound
//!    destination; (b) Catalyst keeps the `dwarf` default even in Release;
//!    (c) non-mac test bundles get the no-destination dSYM default under a
//!    macOS (non-runnable) destination too (20 files).
//! 7. `SWIFT_INCLUDE_PATHS` has a synthesized `$(BUILT_PRODUCTS_DIR) `
//!    default that user `$(inherited)` values append to (8 files).

mod common;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use sweetpad::build_context::{BuildContext, ResolveQuery};
use sweetpad::destination::parse_destination_suffix;

use common::{CatalogCache, capture_xcode_version, find_xcodeproj_for_oracle, read_build_settings};

/// Resolve `target` exactly the way `corpus_oracle.rs` replays the capture at
/// `oracle_rel` (relative to `fixtures/`), returning `(ours, oracle)` for that
/// target's entry.
fn resolve_against_oracle(
    oracle_rel: &str,
    target: &str,
) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    common::pin_capture_host();
    let oracle_path = common::fixtures_root().join(oracle_rel);
    let entries = read_build_settings(&oracle_path)
        .unwrap_or_else(|| panic!("unreadable oracle {}", oracle_path.display()));
    let oracle = entries
        .into_iter()
        .find(|bs| bs.get("TARGET_NAME").map(String::as_str) == Some(target))
        .unwrap_or_else(|| panic!("no entry for target {target} in {oracle_rel}"));

    let version = capture_xcode_version(&oracle_path).expect("oracle path carries xcode-<ver>");
    let mut catalogs = CatalogCache::new();
    let catalog = catalogs.get(&version);

    let project_name = oracle.get("PROJECT_NAME").expect("PROJECT_NAME");
    let config = oracle.get("CONFIGURATION").expect("CONFIGURATION");
    let sdk = oracle.get("PLATFORM_NAME").expect("PLATFORM_NAME");
    let arch = oracle
        .get("NATIVE_ARCH_ACTUAL")
        .or_else(|| oracle.get("HOST_ARCH"))
        .map_or("arm64", String::as_str);
    let destination = oracle_path
        .file_stem()
        .and_then(OsStr::to_str)
        .and_then(|stem| stem.split_once("__").map(|(_, rest)| rest))
        .and_then(parse_destination_suffix);

    let xcodeproj: PathBuf = find_xcodeproj_for_oracle(&oracle_path, project_name)
        .unwrap_or_else(|| panic!("no {project_name}.xcodeproj near {oracle_rel}"));
    let ctx = BuildContext::open(&xcodeproj)
        .expect("open project")
        .with_xcspec(catalog.clone());
    let mut query = ResolveQuery::new(target, config, sdk, arch);
    if let Some(d) = destination {
        query = query.with_destination(d);
    }
    let resolved = ctx.resolve(&query).expect("resolve").settings;
    (resolved, oracle)
}

fn assert_key_matches_oracle(oracle_rel: &str, target: &str, key: &str) {
    let (ours, oracle) = resolve_against_oracle(oracle_rel, target);
    let want = oracle
        .get(key)
        .unwrap_or_else(|| panic!("oracle {oracle_rel} has no {key}"));
    assert_eq!(
        ours.get(key),
        Some(want),
        "{key} for {target} must match the captured xcodebuild value in {oracle_rel}"
    );
}

// --- 1. host-derived asset-catalog OS filter ---------------------------------

/// An iOS-natural target under a macOS run destination reports the HOST's
/// macOS version as the asset-thinning OS filter (the capture host ran
/// macOS 26.5 — `corpus/manifest.json`). The value must come through the
/// pinned host override, not a live `sw_vers` probe that only exists on macs.
#[test]
fn assetcatalog_os_filter_under_macos_destination_uses_pinned_host_version() {
    assert_key_matches_oracle(
        "alamofire/xcode-26.5.0/metadata/schemes/iOS Example/build-settings/Debug__macOS.json",
        "iOS Example",
        "ASSETCATALOG_FILTER_FOR_DEVICE_OS_VERSION",
    );
}

// --- 2. ENABLE_DEBUG_DYLIB is previews-gated, not configuration-gated --------

/// Release build of an application that does NOT author `ENABLE_PREVIEWS`:
/// the DarwinProductTypes.xcspec default `YES` survives into Release.
#[test]
fn enable_debug_dylib_stays_yes_in_release_without_authored_previews() {
    assert_key_matches_oracle(
        "kingfisher/xcode-26.5.0/metadata/schemes/Kingfisher-Demo/build-settings/Release__iOS-Simulator_OS26.5_iPad-A16.json",
        "Kingfisher-Demo",
        "ENABLE_DEBUG_DYLIB",
    );
}

/// Release build of an application that AUTHORS `ENABLE_PREVIEWS = YES`
/// (tuist-generated): previews can't run optimized, so xcodebuild forces
/// `ENABLE_DEBUG_DYLIB = NO`.
#[test]
fn enable_debug_dylib_forced_no_in_release_when_previews_authored() {
    assert_key_matches_oracle(
        "tuist-fixtures/xcode-26.5.0/metadata/examples_xcode_generated_app_with_custom_scheme/schemes/App/build-settings/Release__iOS-Simulator_OS26.5_iPad-A16.json",
        "App",
        "ENABLE_DEBUG_DYLIB",
    );
}

/// A tvOS application keeps the product-type `YES` in Release too — the old
/// per-config rule reported `NO` here.
#[test]
fn enable_debug_dylib_yes_for_tvos_app_in_release() {
    assert_key_matches_oracle(
        "kingfisher/xcode-26.5.0/metadata/schemes/Kingfisher-tvOS-Demo/build-settings/Release__tvOS-Simulator_OS26.5_Apple-TV.json",
        "Kingfisher-tvOS-Demo",
        "ENABLE_DEBUG_DYLIB",
    );
}

// --- 3. previews force the hardened runtime off (non-macOS) ------------------

/// IceCubesApp authors BOTH `ENABLE_HARDENED_RUNTIME = YES` and
/// `ENABLE_PREVIEWS = YES`. In Debug on a simulator destination previews are
/// effective, and xcodebuild forces the hardened runtime OFF over the user's
/// authored YES.
#[test]
fn hardened_runtime_forced_off_when_previews_effective_off_macos() {
    assert_key_matches_oracle(
        "ice-cubes/xcode-26.5.0/metadata/schemes/IceCubesApp/build-settings/Debug__iOS-Simulator_OS26.5_iPad-A16.json",
        "IceCubesApp",
        "ENABLE_HARDENED_RUNTIME",
    );
}

/// Same target in Release: previews are not effective (optimized build), so
/// the authored `YES` survives — the override must not over-fire.
#[test]
fn hardened_runtime_keeps_user_yes_in_release() {
    assert_key_matches_oracle(
        "ice-cubes/xcode-26.5.0/metadata/schemes/IceCubesApp/build-settings/Release__iOS-Simulator_OS26.5_iPad-A16.json",
        "IceCubesApp",
        "ENABLE_HARDENED_RUNTIME",
    );
}

// --- 4. STRIP_INSTALLED_PRODUCT on Xcode 15.x ---------------------------------

/// Every Xcode 15.4 capture reports `STRIP_INSTALLED_PRODUCT = YES`, Debug
/// included — the Debug→NO default only appears on Xcode 16+.
#[test]
fn strip_installed_product_is_yes_in_debug_on_xcode_15() {
    assert_key_matches_oracle(
        "kingfisher/xcode-15.4.0/metadata/schemes/Kingfisher/build-settings/Debug__macOS.json",
        "Kingfisher",
        "STRIP_INSTALLED_PRODUCT",
    );
}

// --- 5. Catalyst clamps IPHONEOS_DEPLOYMENT_TARGET ---------------------------

/// The Alamofire framework authors `IPHONEOS_DEPLOYMENT_TARGET = 10.0`; under
/// Mac Catalyst xcodebuild reports the 13.1 Catalyst floor (the same floor the
/// derived MACOSX/SWIFT deployment targets are already computed from).
#[test]
fn catalyst_reports_ios_deployment_target_clamped_to_13_1() {
    assert_key_matches_oracle(
        "alamofire/xcode-26.5.0/metadata/schemes/Alamofire iOS/build-settings/Debug__macOS.json",
        "Alamofire iOS",
        "IPHONEOS_DEPLOYMENT_TARGET",
    );
}

// --- 6. DEBUG_INFORMATION_FORMAT couplings ------------------------------------

/// An application with the debug dylib active but previews NOT effective
/// (nothing authored) defaults to `dwarf-with-dsym` even in Debug with a
/// bound, runnable simulator destination.
#[test]
fn debug_dylib_without_previews_defaults_debug_dif_to_dsym() {
    assert_key_matches_oracle(
        "alamofire/xcode-26.5.0/metadata/schemes/iOS Example/build-settings/Debug__iOS-Simulator_OS26.5_iPad-A16.json",
        "iOS Example",
        "DEBUG_INFORMATION_FORMAT",
    );
}

/// With previews effective (ice-cubes authors `ENABLE_PREVIEWS = YES`), Debug
/// keeps the authored/xcspec `dwarf` — the dSYM default must not over-fire.
#[test]
fn previews_effective_debug_keeps_dwarf() {
    assert_key_matches_oracle(
        "ice-cubes/xcode-26.5.0/metadata/schemes/IceCubesApp/build-settings/Debug__iOS-Simulator_OS26.5_iPad-A16.json",
        "IceCubesApp",
        "DEBUG_INFORMATION_FORMAT",
    );
}

/// Catalyst Release keeps the xcspec `dwarf` default — the unconditional
/// Release→dwarf-with-dsym override doesn't apply under Catalyst.
#[test]
fn catalyst_release_keeps_dwarf_default() {
    assert_key_matches_oracle(
        "kingfisher/xcode-26.5.0/metadata/schemes/Kingfisher-Demo/build-settings/Release__macOS.json",
        "Kingfisher-Demo",
        "DEBUG_INFORMATION_FORMAT",
    );
}

/// A non-mac unit-test bundle under a macOS destination is in the
/// no-runnable-destination view, where xcodebuild applies the same Debug
/// dSYM default it uses with no destination at all.
#[test]
fn test_bundle_under_macos_destination_gets_dsym_default() {
    assert_key_matches_oracle(
        "tuist-fixtures/xcode-26.5.0/metadata/examples_xcode_generated_app_with_framework_and_tests/schemes/App-Workspace/build-settings/Debug__macOS.json",
        "AppTests",
        "DEBUG_INFORMATION_FORMAT",
    );
}

/// The same test bundle under a bound iOS-Simulator destination keeps the
/// documented Debug `dwarf` — the broadened gate must not leak into runnable
/// destinations.
#[test]
fn test_bundle_under_simulator_destination_keeps_dwarf() {
    assert_key_matches_oracle(
        "tuist-fixtures/xcode-26.5.0/metadata/examples_xcode_generated_app_with_framework_and_tests/schemes/App-Workspace/build-settings/Debug__iOS-Simulator_OS26.5_iPad-A16.json",
        "AppTests",
        "DEBUG_INFORMATION_FORMAT",
    );
}

// --- 7. SWIFT_INCLUDE_PATHS synthesized default --------------------------------

/// xcodebuild's defaults define `SWIFT_INCLUDE_PATHS = $(BUILT_PRODUCTS_DIR) `
/// (trailing space), so a target authoring `$(inherited) <path>` resolves to
/// `<BUILT_PRODUCTS_DIR>  <path>`. The two sides anchor at different roots
/// (fixture raw/ vs the capture checkout), so compare the SHAPE on both:
/// first token == that side's own BUILT_PRODUCTS_DIR, identical double-space
/// separator, and the same project-relative tail.
#[test]
fn swift_include_paths_inherits_built_products_dir_default() {
    let (ours, oracle) = resolve_against_oracle(
        "tuist-fixtures/xcode-26.5.0/metadata/examples_xcode_generated_ios_app_with_static_libraries/schemes/A/build-settings/Debug__iOS-Simulator_OS26.5_iPad-A16.json",
        "A",
    );
    let tail = "/Modules/A/../C/prebuilt/C";
    for (label, map) in [("ours", &ours), ("oracle", &oracle)] {
        let value = map
            .get("SWIFT_INCLUDE_PATHS")
            .unwrap_or_else(|| panic!("{label}: SWIFT_INCLUDE_PATHS missing"));
        let bpd = map.get("BUILT_PRODUCTS_DIR").expect("BUILT_PRODUCTS_DIR");
        let want_prefix = format!("{bpd}  ");
        assert!(
            value.starts_with(&want_prefix),
            "{label}: SWIFT_INCLUDE_PATHS must start with its own \
             BUILT_PRODUCTS_DIR + double space; got {value:?}"
        );
        assert!(
            value.ends_with(tail),
            "{label}: SWIFT_INCLUDE_PATHS must keep the authored tail {tail:?}; got {value:?}"
        );
    }
}

/// Guard for targets that DON'T author the key: the synthesized default must
/// resolve to exactly `<BUILT_PRODUCTS_DIR> ` (trailing space), matching the
/// shape xcodebuild's other synthesized search paths take.
#[test]
fn swift_include_paths_default_is_built_products_dir_when_unauthored() {
    let (ours, _) = resolve_against_oracle(
        "kingfisher/xcode-26.5.0/metadata/schemes/Kingfisher/build-settings/Debug__iOS-Simulator_OS26.5_iPad-A16.json",
        "Kingfisher",
    );
    let bpd = ours.get("BUILT_PRODUCTS_DIR").expect("BUILT_PRODUCTS_DIR");
    assert_eq!(
        ours.get("SWIFT_INCLUDE_PATHS"),
        Some(&format!("{bpd} ")),
        "unauthored SWIFT_INCLUDE_PATHS must resolve to the synthesized default"
    );
}
