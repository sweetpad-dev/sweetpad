//! Custom-configuration oracle: prove the resolver selects and resolves a build
//! configuration *named* something other than `Debug`/`Release` — a path no real
//! corpus project exercises (they all ship only the two stock configs).
//!
//! The fixture (`scripts/15_custom_configuration.py`) is a minimal macOS tool
//! with a third configuration `Profile`, captured no-destination per config so
//! each capture is a bare `ResolveQuery::new(target, "Profile", sdk, arch)` —
//! the same clean layer stack as `per_target_oracle.rs`.
//!
//! `Profile` is marked two ways, one per resolution layer. `PBXPROJ_MARKER` is a
//! per-config inline `buildSettings` value (tests config-name selection of the
//! right `XCBuildConfiguration`); `XCCONFIG_MARKER` is a `[config=Profile]` entry
//! in the project's `baseConfigurationReference` xcconfig (tests config-conditional
//! xcconfig resolution firing under a non-stock config name). Both must resolve to
//! `profile` only under `Profile`, and to their base values under `Debug`/`Release`
//! — explicit assertions below, not just an aggregate %.

mod common;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use sweetpad::build_context::{BuildContext, ResolveQuery};

use common::{
    CatalogCache, MismatchTally, Stats, assert_version_floors, capture_xcode_version,
    fixtures_root, read_build_settings,
};

const FIXTURE_DIR: &str = "_synthetic-custom-config";

/// Every per-config capture under `fixtures/_synthetic-custom-config/*/captures/`
/// (excluding `meta.json`). One file per (project, configuration).
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

/// The generated `Scratch.xcodeproj` sits at `<ver>/project/` — one level up from
/// the `captures/` dir the oracle file lives in.
fn project_for(capture: &Path) -> Option<PathBuf> {
    let ver_root = capture.parent()?.parent()?; // captures/ -> <ver>/
    let proj = ver_root.join("project").join("Scratch.xcodeproj");
    proj.is_dir().then_some(proj)
}

/// Resolve one per-config capture, score every shared key, and assert the
/// config-name-driven markers resolved correctly. Returns the capture's Xcode
/// version and the scoring [`Stats`].
fn run_capture(
    capture: &Path,
    catalogs: &mut CatalogCache,
    mismatch: &mut MismatchTally,
    canon_only: &mut MismatchTally,
) -> (String, Stats) {
    let version = capture_xcode_version(capture)
        .unwrap_or_else(|| panic!("capture path lacks xcode-<ver>: {}", capture.display()));
    let entries =
        read_build_settings(capture).unwrap_or_else(|| panic!("unreadable: {}", capture.display()));
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
        .unwrap_or_else(|e| panic!("resolve {config}: {e}"))
        .settings;

    // The headline of this fixture: a config named neither Debug nor Release is
    // selected by name, its inline per-config setting wins, and a
    // `[config=<name>]` xcconfig entry fires — each checked against the real
    // captured value so the assertion can't drift from the oracle.
    for key in ["CONFIGURATION", "PBXPROJ_MARKER", "XCCONFIG_MARKER"] {
        assert_eq!(
            resolved.get(key).map(String::as_str),
            bs.get(key).map(String::as_str),
            "[{config}] {key}: resolver {:?} != oracle {:?}",
            resolved.get(key),
            bs.get(key),
        );
    }

    let stats = common::compare(&resolved, bs, capture, mismatch, canon_only);
    (version, stats)
}

/// Per-version `(exact, canonical, structural)` floors. 26.x/16.x land ~99%
/// structural — we resolve against the very project the oracle was captured from
/// (same path, same Xcode), so even path-anchored keys match. 15.4 sits lower for
/// the same irreducible reason as `per_target_oracle.rs`: the 15.x host/arch
/// reporting (NATIVE_ARCH/HOST_ARCH/CURRENT_ARCH/VALID_ARCHS) the resolver can't
/// derive from inputs. Signing keys (CODE_SIGN_IDENTITY) are out of scope on
/// every version. Set from the first clean run minus a ~1pt margin.
///
/// The strict floors only hold *in place*: this fixture was captured inside the
/// repo itself, so when the checkout sits at the capture path even
/// path-anchored keys byte-match. At any other checkout (another machine, CI)
/// the same fixed set of path-embedding keys can only structurally match, so
/// the exact/canonical ceilings drop by that set's share — `in_place: false`
/// returns floors calibrated for that mode (first off-path clean run minus a
/// ~1pt margin). Structural is geometry-independent and shared by both modes.
#[allow(clippy::match_same_arms)]
fn version_floor(version: &str, in_place: bool) -> Option<(u64, u64, u64)> {
    match (version, in_place) {
        ("26.5.0", true) => Some((85, 85, 98)),
        ("26.5.0", false) => Some((83, 84, 98)),
        ("16.4.0", true) => Some((84, 85, 98)),
        ("16.4.0", false) => Some((83, 83, 98)),
        // 15.4 byte-matches more keys (simpler SDK geometry) but loses structural
        // to the irreducible 15.x arch family — the inverse of the newer majors.
        ("15.4.0", true) => Some((93, 93, 95)),
        ("15.4.0", false) => Some((81, 92, 95)),
        _ => None,
    }
}

/// Whether this checkout sits at the very absolute path the capture was taken
/// at, by comparing the captured `SRCROOT` against the fixture project's real
/// location. Selects which floor calibration applies (see [`version_floor`]).
fn checkout_in_capture_place(capture: &Path) -> bool {
    let Some(entries) = read_build_settings(capture) else {
        return false;
    };
    let captured_srcroot = entries.first().and_then(|bs| bs.get("SRCROOT").cloned());
    let local_srcroot =
        project_for(capture).and_then(|p| p.parent().map(|d| d.display().to_string()));
    captured_srcroot.is_some() && captured_srcroot == local_srcroot
}

#[test]
fn custom_configuration_oracle_coverage() {
    common::pin_capture_host();
    let captures = capture_files();
    assert!(
        captures.len() >= 3,
        "expected ≥3 custom-config captures (Debug/Release/Profile); got {}",
        captures.len()
    );

    let mut catalogs = CatalogCache::new();
    let mut total = Stats::default();
    let mut per_version: BTreeMap<String, Stats> = BTreeMap::new();
    let mut mismatch: MismatchTally = BTreeMap::new();
    let mut canon_only: MismatchTally = BTreeMap::new();
    let mut saw_profile = false;
    let mut in_place = true;

    for capture in &captures {
        if capture.file_name() == Some(OsStr::new("Scratch__Profile.json")) {
            saw_profile = true;
        }
        in_place &= checkout_in_capture_place(capture);
        let (version, stats) = run_capture(capture, &mut catalogs, &mut mismatch, &mut canon_only);
        total.merge(stats);
        per_version.entry(version).or_default().merge(stats);
    }

    assert!(
        saw_profile,
        "the custom `Profile` config capture is missing"
    );
    common::print_summary("Custom-configuration oracle", &total, &mismatch);
    if !in_place {
        println!("(checkout differs from the capture path — applying off-path floor calibration)");
    }

    // The marker assertions in `run_capture` are the real proof that a custom
    // config resolves; these per-version floors guard the rest of the resolved
    // dictionary against regressions (see `version_floor`).
    assert_version_floors("custom-config", &per_version, |v| {
        version_floor(v, in_place)
    });
}
