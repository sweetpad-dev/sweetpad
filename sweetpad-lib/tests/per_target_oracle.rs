//! Per-target oracle: validate the resolver against the isolated single-target
//! `xcodebuild -showBuildSettings -json -project P -target T -configuration C`
//! captures in `metadata/<sub>/_per_target/<Proj>/<target>__<config>.json`.
//!
//! Unlike `corpus_oracle.rs` (which feeds the scheme-aggregated captures and
//! has to reconstruct a destination + scheme code-coverage flag), these were
//! captured with NO scheme and NO destination — the cleanest analog to a bare
//! `ResolveQuery::new(target, config, sdk, arch)`. That makes this test a
//! direct exercise of the per-target layer stack (project + target xcconfig +
//! inline buildSettings), a path the scheme oracle can mask through
//! aggregation. The capture method is documented in
//! `scripts/09_per_project_settings.py`.
//!
//! Shares the JSON reader, canonicalizer, project lookup, [`Stats`], and the
//! three-tier [`common::compare`] classifier with the other oracle tests.

#![allow(clippy::too_many_lines)]

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use sweetpad::build_context::{BuildContext, ResolveQuery};
use sweetpad::xcspec;

use common::{
    CatalogCache, MismatchTally, Stats, capture_xcode_version, find_capture_files,
    find_xcodeproj_between, read_build_settings,
};

/// Resolve one per-target capture and score it against the resolver output.
///
/// Each capture is a one-element `-showBuildSettings` array for a single
/// (target, config). We pull `(target, config, sdk, arch)` straight out of the
/// captured `buildSettings` — no destination, no scheme aggregation — and
/// compare against a bare query.
fn run_oracle(
    oracle_path: &Path,
    catalog: &xcspec::Catalog,
    mismatch_tally: &mut MismatchTally,
    canon_only_tally: &mut MismatchTally,
) -> Option<Stats> {
    let entries = read_build_settings(oracle_path)?;
    let mut stats = Stats::default();
    for bs in &entries {
        let project_name = bs.get("PROJECT_NAME")?;
        let target = bs.get("TARGET_NAME")?;
        let config = bs.get("CONFIGURATION")?;
        let sdk = bs.get("PLATFORM_NAME")?;
        let arch = bs
            .get("NATIVE_ARCH_ACTUAL")
            .or_else(|| bs.get("HOST_ARCH"))
            .map_or("arm64", String::as_str);

        let xcodeproj = find_xcodeproj_between(oracle_path, "_per_target", project_name)?;

        let ctx = BuildContext::open(&xcodeproj)
            .ok()?
            .with_xcspec(catalog.clone());
        let query = ResolveQuery::new(target, config, sdk, arch);
        let resolved = ctx.resolve(&query).ok()?.settings;

        stats.merge(common::compare(
            &resolved,
            bs,
            oracle_path,
            mismatch_tally,
            canon_only_tally,
        ));
    }
    Some(stats)
}

#[test]
fn per_target_oracle_coverage() {
    common::pin_capture_host();
    let oracles = find_capture_files("_per_target");
    let only = common::only_version();
    let mut catalogs = CatalogCache::new();

    let mut total = Stats::default();
    let mut per_version: BTreeMap<String, Stats> = BTreeMap::new();
    let mut mismatch_tally: MismatchTally = BTreeMap::new();
    let mut canon_only_tally: MismatchTally = BTreeMap::new();
    let mut skipped: u64 = 0;
    let mut empty: u64 = 0;

    for path in &oracles {
        let Some(version) = capture_xcode_version(path) else {
            skipped += 1;
            continue;
        };
        if only.as_deref().is_some_and(|v| v != version) {
            continue;
        }
        let catalog = catalogs.get(&version);
        let Some(stats) = run_oracle(path, catalog, &mut mismatch_tally, &mut canon_only_tally)
        else {
            let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            if size < 16 {
                empty += 1;
            } else {
                skipped += 1;
            }
            continue;
        };
        total.merge(stats);
        per_version.entry(version).or_default().merge(stats);
    }

    common::print_summary("Per-target oracle validation", &total, &mismatch_tally);
    println!(
        "files: {} discovered, {empty} empty, {skipped} skipped (project lookup failed)",
        oracles.len()
    );
    println!("\n--- top 30 canonical-only mismatches (structural matches, path-root drift) ---");
    let mut canon: Vec<(&String, &u64)> = canon_only_tally.iter().collect();
    canon.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in canon.iter().take(30) {
        println!("  {n:<5} {k}");
    }

    // Diagnostic mode (ORACLE_ONLY_VERSION set): we scored a single version in
    // isolation, so the corpus-wide floors don't apply — the printed tally is
    // the deliverable. Skip the assertions.
    if only.is_some() {
        return;
    }

    // Per-version floors (see `assert_version_floors`). The canonical tier sits
    // below corpus_oracle's because the isolated single-target view surfaces
    // real resolver gaps (the documented-irreducible ENABLE_DEBUG_DYLIB
    // Release-app heuristic and the 15.x host/arch family — see the
    // systematic-mismatch tally) that scheme aggregation otherwise masks.
    // Structural still lands at 99%: those misses are genuine value
    // disagreements, not path-root drift.
    assert!(
        total.files >= 100,
        "expected to process ≥100 per-target captures; only got {}",
        total.files
    );
    common::assert_version_floors("per-target", &per_version, version_floor);
}

/// Per-version `(exact, canonical, structural)` floors for the no-destination
/// per-target captures — the cleanest resolver oracle. Set from the first clean
/// multi-version run minus a ~1pt margin. Arms are kept per-version even when two
/// versions' floors currently coincide, since they track independent baselines.
#[allow(clippy::match_same_arms)]
fn version_floor(version: &str) -> Option<(u64, u64, u64)> {
    match version {
        "26.5.0" => Some((88, 89, 100)),
        "16.4.0" => Some((87, 89, 100)),
        // The 15.x host/arch reporting family (NATIVE_ARCH/HOST_ARCH=arm64e,
        // concrete CURRENT_ARCH = last of resolved ARCHS, the legacy
        // VALID_ARCHS lists, empty LOCROOT/LOCSYMROOT) is now modelled as
        // version-gated rules, so 15.4 scores match the modern majors.
        "15.4.0" => Some((89, 97, 100)),
        _ => None,
    }
}
