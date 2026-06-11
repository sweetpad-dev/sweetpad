//! Project-defaults oracle: validate the resolver against the
//! `xcodebuild -showBuildSettings -json -project P` captures (NO `-scheme`,
//! NO `-target`) under `metadata/<sub>/_project_defaults/<Proj>/project-only*.json`.
//!
//! `xcodebuild` with no target still resolves a concrete view: it picks the
//! project's default target and emits its fully-expanded `buildSettings`
//! (one entry, with `TARGET_NAME`/`PROJECT_NAME`/`CONFIGURATION`/`PLATFORM_NAME`
//! all populated). So the resolver doesn't need a separate "project-only"
//! path — each capture maps to the same per-target [`ResolveQuery`] the corpus
//! oracle uses, keyed off the target xcodebuild chose. This exercises the
//! project layer before any scheme aggregation or destination selection.
//!
//! The JSON reader, canonicalizer, [`Stats`], and [`common::compare`] tiering
//! are shared with `corpus_oracle.rs` via [`common`].

#![allow(clippy::too_many_lines)]

mod common;

use std::collections::BTreeMap;

use sweetpad::build_context::{BuildContext, ResolveQuery};
use sweetpad::xcspec;

use common::{
    CatalogCache, MismatchTally, Stats, capture_xcode_version, find_capture_files,
    find_xcodeproj_between, read_build_settings,
};

fn run_capture(
    capture_path: &std::path::Path,
    catalog: &xcspec::Catalog,
    mismatch_tally: &mut MismatchTally,
    canon_only_tally: &mut MismatchTally,
) -> Option<Stats> {
    let entries = read_build_settings(capture_path)?;
    let mut stats = Stats::default();
    for bs in &entries {
        let project_name = bs.get("PROJECT_NAME")?;
        let target = bs.get("TARGET_NAME")?;
        let config = bs.get("CONFIGURATION")?;
        // `SDKROOT=auto` projects (e.g. multi-platform apps) leave PLATFORM_NAME
        // empty in the no-target capture; fall back to macosx so resolution
        // still proceeds — the platform-anchored keys then land in the
        // structural tier rather than dropping the file.
        let sdk = bs.get("PLATFORM_NAME").map_or("macosx", String::as_str);
        let arch = bs
            .get("NATIVE_ARCH_ACTUAL")
            .or_else(|| bs.get("HOST_ARCH"))
            .map_or("arm64", String::as_str);

        let xcodeproj = find_xcodeproj_between(capture_path, "_project_defaults", project_name)?;

        let ctx = BuildContext::open(&xcodeproj)
            .ok()?
            .with_xcspec(catalog.clone());
        // No scheme aggregation and no destination: this is the bare project
        // default-target view, so the query carries only (target, config, sdk,
        // arch) — the same minimal binding `xcodebuild -project P` resolves.
        let query = ResolveQuery::new(target, config, sdk, arch);
        let resolved = ctx.resolve(&query).ok()?.settings;

        stats.merge(common::compare(
            &resolved,
            bs,
            capture_path,
            mismatch_tally,
            canon_only_tally,
        ));
    }
    Some(stats)
}

#[test]
fn project_defaults_oracle_coverage() {
    common::pin_capture_host();
    let captures = find_capture_files("_project_defaults");
    let only = common::only_version();
    let mut catalogs = CatalogCache::new();

    let mut total = Stats::default();
    let mut per_version: BTreeMap<String, Stats> = BTreeMap::new();
    let mut mismatch_tally: MismatchTally = BTreeMap::new();
    let mut canon_only_tally: MismatchTally = BTreeMap::new();
    let mut skipped: u64 = 0;

    for path in &captures {
        let Some(version) = capture_xcode_version(path) else {
            skipped += 1;
            continue;
        };
        if only.as_deref().is_some_and(|v| v != version) {
            continue;
        }
        let catalog = catalogs.get(&version);
        let Some(stats) = run_capture(path, catalog, &mut mismatch_tally, &mut canon_only_tally)
        else {
            // One known skip: ice-cubes' `IceCubesApp.xcodeproj` Debug config
            // names a base xcconfig (`IceCubesApp.xcconfig`) that wasn't part
            // of the captured `raw/` tree, so the project fails to open for
            // that config. A missing-fixture-file gap, not a resolver miss.
            skipped += 1;
            continue;
        };
        total.merge(stats);
        per_version.entry(version).or_default().merge(stats);
    }

    common::print_summary(
        "Project-defaults oracle validation",
        &total,
        &mismatch_tally,
    );
    println!(
        "files: {} processed, {} skipped (target/project lookup failed) of {} captures",
        total.files,
        skipped,
        captures.len()
    );
    println!("\n--- top 30 canonical-only mismatches (structural matches, path-root drift) ---");
    let mut canon_entries: Vec<(&String, &u64)> = canon_only_tally.iter().collect();
    canon_entries.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in canon_entries.iter().take(30) {
        println!("  {n:<5} {k}");
    }

    // Diagnostic mode (ORACLE_ONLY_VERSION set): single-version run, floors
    // don't apply — the printed tally is the deliverable.
    if only.is_some() {
        return;
    }

    // Floors set just under the observed pass rate (data-driven):
    // 87% exact / 88% canonical / 99% structural.
    //
    // Canonical here (88%) runs well below the corpus oracle's 96% on purpose:
    // the no-target `-showBuildSettings -project P` capture roots its build
    // output at a project-relative `build/` directory next to the project,
    // while our resolver computes the usual `~/Library/Developer/Xcode/
    // DerivedData/<hash>` root. That's a path-root drift on the whole
    // `BUILD_DIR`/`OBJROOT`/`*_SEARCH_PATHS` family (the 83-deep canonical-only
    // tally above), so those keys land in the structural tier rather than
    // canonical — hence the high structural (99%) but lower canonical. The
    // residual real misses (ENABLE_DEBUG_DYLIB=YES, ARCHS=`arm64 x86_64`) are
    // xcodebuild's default-target behaviour in this no-destination mode and
    // are left as documented gaps, not forced.
    assert!(
        total.files >= 80,
        "expected to process at least 80 project-defaults captures; only got {}",
        total.files
    );
    common::assert_version_floors("project-defaults", &per_version, version_floor);
}

/// Per-version `(exact, canonical, structural)` floors for the no-target
/// `-showBuildSettings -project P` captures. Canonical runs below the corpus
/// oracle's by design (the no-target capture roots build output at a
/// project-relative `build/` dir while we compute the DerivedData root, pushing
/// the whole `BUILD_DIR`/`*_SEARCH_PATHS` family into the structural tier). Set
/// from the first clean multi-version run minus a ~1pt margin.
fn version_floor(version: &str) -> Option<(u64, u64, u64)> {
    match version {
        "26.5.0" => Some((86, 87, 98)),
        "16.4.0" => Some((86, 88, 98)),
        // 15.4 structural ~97%: irreducible 15.x host/arch reporting (arm64e
        // NATIVE_ARCH/HOST_ARCH, concrete no-destination CURRENT_ARCH,
        // VALID_ARCHS ordering); the real 15.x parse bugs are fixed.
        "15.4.0" => Some((85, 93, 96)),
        _ => None,
    }
}
