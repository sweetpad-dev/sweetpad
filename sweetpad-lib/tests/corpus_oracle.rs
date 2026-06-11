//! Corpus-wide validation: run the full resolver pipeline against every
//! non-empty `xcodebuild -showBuildSettings` JSON in the captured fixtures and
//! report how close our output gets, per project and per platform.
//!
//! This test is the main "does the whole pipeline actually work across the
//! real world" guard. The Scratch fixture has zero conditionals; everything
//! interesting (workspace builds, iOS/tvOS/watchOS/visionOS, multi-target
//! schemes) lives in `metadata/schemes/.../build-settings/`.
//!
//! The JSON reader, canonicalizer, corpus walk, project lookup, [`Stats`], and
//! the per-key comparison core live in [`common`] and are shared with the four
//! per-source oracle tests; this file holds only the scheme-aggregated
//! specifics (destination-suffix parsing + scheme code-coverage lookup).

#![allow(clippy::too_many_lines, clippy::needless_lifetimes, dead_code)]

mod common;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use sweetpad::build_context::{BuildContext, ResolveQuery};
use sweetpad::destination::parse_destination_suffix;
use sweetpad::scheme;
use sweetpad::xcspec;

use common::{
    CatalogCache, MismatchTally, Stats, capture_xcode_version, find_oracles,
    find_xcodeproj_for_oracle, read_build_settings,
};

/// Whether the scheme that produced this oracle gathers code coverage.
///
/// The oracle path embeds the scheme name as the component right after
/// `schemes/`. We re-derive the `raw/` sub-fixture root (same logic as
/// [`find_xcodeproj_for_oracle`]) and search it for `<scheme>.xcscheme`,
/// then read the TestAction's `codeCoverageEnabled`. Schemes can sit under
/// either an `.xcodeproj` or an `.xcworkspace`, so we search the whole
/// sub-fixture rather than guessing the container.
fn scheme_code_coverage_enabled(oracle: &Path) -> bool {
    let comps: Vec<&OsStr> = oracle.iter().collect();
    let Some(metadata_idx) = comps.iter().rposition(|c| *c == OsStr::new("metadata")) else {
        return false;
    };
    let Some(schemes_idx) = comps.iter().rposition(|c| *c == OsStr::new("schemes")) else {
        return false;
    };
    let Some(scheme_name) = comps.get(schemes_idx + 1).and_then(|c| c.to_str()) else {
        return false;
    };
    let mut root = PathBuf::new();
    for (i, c) in comps.iter().enumerate() {
        if i < metadata_idx {
            root.push(c);
        } else if i == metadata_idx {
            root.push("raw");
        } else if i > metadata_idx && i < schemes_idx {
            root.push(c);
        }
    }
    let file_name = format!("{scheme_name}.xcscheme");
    let Some(scheme_path) = common::find_file_named(&root, &file_name) else {
        return false;
    };
    scheme::parse_file(&scheme_path)
        .ok()
        .and_then(|s| s.test_action)
        .is_some_and(|ta| ta.code_coverage_enabled)
}

fn run_oracle(
    oracle_path: &Path,
    catalog: &xcspec::Catalog,
    project_mismatch_tally: &mut MismatchTally,
    project_canon_only_tally: &mut MismatchTally,
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

        let xcodeproj = find_xcodeproj_for_oracle(oracle_path, project_name)?;

        // Pull the run destination out of the oracle filename suffix so the
        // resolver can synthesize destination-aware defaults (`ARCHS`,
        // `ONLY_ACTIVE_ARCH`, `__IS_NOT_SIMULATOR`, etc.).
        let destination = oracle_path
            .file_stem()
            .and_then(OsStr::to_str)
            .and_then(|stem| stem.split_once("__").map(|(_, rest)| rest))
            .and_then(parse_destination_suffix);

        let ctx = BuildContext::open(&xcodeproj)
            .ok()?
            .with_xcspec(catalog.clone());
        let mut query = ResolveQuery::new(target, config, sdk, arch);
        if let Some(d) = destination {
            query = query.with_destination(d);
        }
        // Code coverage is a scheme-level fact (TestAction.codeCoverageEnabled)
        // that the per-target pbxproj can't carry, so the harness reads it from
        // the scheme named in the oracle path and feeds it to the resolver.
        if scheme_code_coverage_enabled(oracle_path) {
            query = query.with_code_coverage_enabled(true);
        }
        let resolved = ctx.resolve(&query).ok()?.settings;

        stats.merge(common::compare(
            &resolved,
            bs,
            oracle_path,
            project_mismatch_tally,
            project_canon_only_tally,
        ));
    }
    Some(stats)
}

fn key_for_grouping(path: &Path, group: &str) -> String {
    // path looks like fixtures/<project>/xcode-<ver>/metadata/schemes/<scheme>/build-settings/<config>__<dest>.json
    let comps: Vec<&str> = path.iter().filter_map(OsStr::to_str).collect();
    match group {
        "project" => comps
            .iter()
            .position(|c| *c == "fixtures")
            .and_then(|i| comps.get(i + 1).copied())
            .unwrap_or("unknown")
            .to_string(),
        "platform" => {
            // Sniff from the filename suffix between `__` and `.json`.
            let stem = path.file_stem().and_then(OsStr::to_str).unwrap_or_default();
            stem.split_once("__")
                .map_or_else(|| "unknown".into(), |(_, rest)| rest.to_string())
        }
        _ => "all".to_string(),
    }
}

#[test]
fn full_corpus_oracle_coverage() {
    common::pin_capture_host();
    let oracles = find_oracles();
    let only = common::only_version();
    let mut catalogs = CatalogCache::new();

    let mut total = Stats::default();
    let mut per_project: BTreeMap<String, Stats> = BTreeMap::new();
    let mut per_platform: BTreeMap<String, Stats> = BTreeMap::new();
    let mut per_version: BTreeMap<String, Stats> = BTreeMap::new();
    let mut per_project_mismatch: BTreeMap<String, MismatchTally> = BTreeMap::new();
    let mut per_project_canon_only: BTreeMap<String, MismatchTally> = BTreeMap::new();
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
        let project = key_for_grouping(path, "project");
        let platform = key_for_grouping(path, "platform");
        let project_tally = per_project_mismatch.entry(project.clone()).or_default();
        let project_canon_tally = per_project_canon_only.entry(project.clone()).or_default();
        let catalog = catalogs.get(&version);

        let Some(stats) = run_oracle(path, catalog, project_tally, project_canon_tally) else {
            // Could be empty `[]` (lots of those) or unresolvable target.
            let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            if size < 16 {
                empty += 1;
            } else {
                skipped += 1;
            }
            continue;
        };
        total.merge(stats);
        per_project.entry(project).or_default().merge(stats);
        per_platform.entry(platform).or_default().merge(stats);
        per_version.entry(version).or_default().merge(stats);
    }

    println!("=== Full corpus oracle validation ===");
    println!(
        "files: {} processed, {} empty, {} skipped (target/project lookup failed)",
        total.files, empty, skipped
    );
    println!(
        "oracle keys total: {}, our keys total: {}",
        total.oracle_keys, total.our_keys
    );
    if total.shared_keys > 0 {
        println!(
            "shared: {}, exact: {} ({}%), canonical: {} ({}%), structural: {} ({}%)",
            total.shared_keys,
            total.exact_matches,
            total.exact_pct(),
            total.canonical_matches,
            total.canonical_pct(),
            total.structural_matches,
            total.structural_pct(),
        );
    }
    println!("\n--- per project (exact / canonical / structural) ---");
    for (k, s) in &per_project {
        println!(
            "  {k:<20} files={:<4} shared={:<7} exact={}% canon={}% struct={}%",
            s.files,
            s.shared_keys,
            s.exact_pct(),
            s.canonical_pct(),
            s.structural_pct(),
        );
    }
    println!("\n--- per platform (exact / canonical / structural) ---");
    for (k, s) in &per_platform {
        println!(
            "  {k:<60} files={:<4} exact={}% canon={}% struct={}%",
            s.files,
            s.exact_pct(),
            s.canonical_pct(),
            s.structural_pct(),
        );
    }
    println!("\n--- top 30 systematic mismatches (key, # of oracles it fails in) ---");
    // Global tally is the sum of the per-project tallies `compare` filled.
    let mut mismatch_tally: MismatchTally = BTreeMap::new();
    for tally in per_project_mismatch.values() {
        common::merge_tally(&mut mismatch_tally, tally);
    }
    let mut entries: Vec<(&String, &u64)> = mismatch_tally.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in entries.iter().take(30) {
        println!("  {n:<5} {k}");
    }

    println!("\n--- top 20 mismatches per project ---");
    for (project, tally) in &per_project_mismatch {
        if tally.is_empty() {
            continue;
        }
        let mut entries: Vec<(&String, &u64)> = tally.iter().collect();
        entries.sort_by(|a, b| b.1.cmp(a.1));
        println!("  [{project}]");
        for (k, n) in entries.iter().take(20) {
            println!("    {n:<5} {k}");
        }
    }
    println!("\n--- top 20 canonical-only mismatches per project (structural matches) ---");
    for (project, tally) in &per_project_canon_only {
        if tally.is_empty() {
            continue;
        }
        let mut entries: Vec<(&String, &u64)> = tally.iter().collect();
        entries.sort_by(|a, b| b.1.cmp(a.1));
        println!("  [{project}]");
        for (k, n) in entries.iter().take(20) {
            println!("    {n:<5} {k}");
        }
    }

    // Diagnostic mode (ORACLE_ONLY_VERSION set): single-version run, floors
    // don't apply — the printed tally is the deliverable.
    if only.is_some() {
        return;
    }

    // Coverage floors are per Xcode version (see `assert_version_floors`): a
    // single blended floor across versions drifts as majors are added and masks
    // a per-version regression. exact/canonical are geometry-capped per version;
    // structural is the geometry-independent correctness signal.
    assert!(
        total.files > 30,
        "expected to process more than 30 oracles; only got {}",
        total.files
    );
    common::assert_version_floors("corpus", &per_version, version_floor);
}

/// Per-version `(exact, canonical, structural)` floors for the full corpus
/// scheme captures. Set from the first clean multi-version run minus a ~1pt
/// margin so a real value regression (a whole key family) trips while run-to-run
/// noise doesn't. The exact ceiling is dominated by tuist-fixtures geometry
/// (~76% of all keys, resolved against `raw/`), so it sits ~85% — judge
/// correctness by structural + the systematic-mismatch tally, not exact%.
fn version_floor(version: &str) -> Option<(u64, u64, u64)> {
    match version {
        "26.5.0" => Some(CORPUS_FLOOR_2650),
        "16.4.0" => Some(CORPUS_FLOOR_1640),
        "15.4.0" => Some(CORPUS_FLOOR_1540),
        _ => None,
    }
}

const CORPUS_FLOOR_2650: (u64, u64, u64) = (87, 95, 98);
const CORPUS_FLOOR_1640: (u64, u64, u64) = (87, 98, 98);
// 15.4 structural sits ~97% (vs 99% on 16+) because that Xcode reports host/arch
// settings the resolver can't derive from project inputs and that newer Xcodes
// normalize away (NATIVE_ARCH/HOST_ARCH=arm64e, concrete CURRENT_ARCH on the
// no-destination path, VALID_ARCHS ordering) — documented irreducibles, not
// resolver bugs (the real 15.x parse bugs, PACKAGE_TYPE/BUNDLE_FORMAT
// undomained-clobber, are fixed). Floor accordingly.
const CORPUS_FLOOR_1540: (u64, u64, u64) = (85, 94, 96);
