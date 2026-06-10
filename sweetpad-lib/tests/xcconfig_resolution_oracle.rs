//! Real-xcconfig oracle: validate the resolver's `.xcconfig` layering against
//! `xcodebuild -xcconfig FILE -showBuildSettings` captured on the corpus's own
//! `.xcconfig` files.
//!
//! `scripts/10_xcconfig_resolution.py` ran, for each real `.xcconfig` in the
//! corpus, `xcodebuild -showBuildSettings -json -xcconfig <FILE> -project P
//! -scheme S -configuration C -destination D`. The `-xcconfig` flag layers that
//! file at the top of the normal resolution chain, after xcodebuild interprets
//! its `#include`s, conditionals, and `$(inherited)` / modifier syntax. Each
//! capture sits beside a `*.meta.json` that names the source xcconfig (relative
//! to the corpus slug root) plus the project / scheme / configuration /
//! destination it was captured under.
//!
//! Our analog is [`BuildContext::with_extra_xcconfig`], which calls
//! [`sweetpad::resolver::flatten_xcconfig`] (the very path this oracle is meant
//! to exercise) and pushes it as the top layer. So this test resolves the same
//! (target, config, sdk, arch) tuple the buildSettings advertise, with the real
//! on-disk xcconfig layered on, and scores it with the shared three-tier
//! [`common::compare`].

#![allow(clippy::too_many_lines)]

mod common;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use sweetpad::build_context::{BuildContext, ResolveQuery};
use sweetpad::destination::parse_destination_suffix;
use sweetpad::xcspec;

use common::{
    CatalogCache, MismatchTally, Stats, capture_xcode_version, find_capture_files,
    find_xcodeproj_between, parse_json, read_build_settings,
};

/// Read the `*.meta.json` next to a capture and return the source xcconfig path
/// (relative to the corpus slug root) plus the captured `-destination` string.
fn read_meta(capture: &Path) -> Option<(String, Option<String>)> {
    let meta_path = capture.with_extension("meta.json");
    let json = parse_json(&fs::read_to_string(meta_path).ok()?).ok()?;
    let obj = json.as_object()?;
    let xcconfig = obj.get("xcconfig")?.as_string()?.to_string();
    let destination = obj
        .get("destination")
        .and_then(common::JsonValue::as_string)
        .map(str::to_string);
    Some((xcconfig, destination))
}

/// Resolve the source xcconfig's on-disk location under the fixture `raw/`
/// tree. The meta stores it relative to the corpus slug root; the slug root
/// maps to the `raw/` sibling of the project's `.xcodeproj` (which itself sits
/// under the sub-fixture root we re-derive via [`find_xcodeproj_between`]).
fn locate_xcconfig(xcodeproj: &Path, rel: &str) -> Option<std::path::PathBuf> {
    // The .xcodeproj lives at <raw-root>/<...>/<Proj>.xcodeproj; the corpus
    // slug root *is* <raw-root> (for flat fixtures the project is at the top).
    // Walk up to the `raw/` ancestor and join the meta-relative path.
    let mut dir = xcodeproj.parent()?;
    loop {
        if dir.file_name() == Some(OsStr::new("raw")) {
            break;
        }
        dir = dir.parent()?;
    }
    let candidate = dir.join(rel);
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

fn run_capture(
    capture: &Path,
    catalog: &xcspec::Catalog,
    mismatch_tally: &mut MismatchTally,
    canon_only_tally: &mut MismatchTally,
) -> Option<Stats> {
    let entries = read_build_settings(capture)?;
    let (xcconfig_rel, destination) = read_meta(capture)?;

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

        let xcodeproj = find_xcodeproj_between(capture, "_xcconfig_resolution", project_name)?;
        let xcconfig = locate_xcconfig(&xcodeproj, &xcconfig_rel)?;

        let ctx = BuildContext::open(&xcodeproj)
            .ok()?
            .with_xcspec(catalog.clone())
            .with_extra_xcconfig(&xcconfig)
            .ok()?;

        let mut query = ResolveQuery::new(target, config, sdk, arch);
        // The meta records the run destination as `platform=macOS`; strip the
        // `platform=` prefix and feed it through the same suffix parser the
        // scheme oracle uses, so destination-aware defaults match.
        if let Some(d) = destination
            .as_deref()
            .and_then(|d| d.strip_prefix("platform="))
            .and_then(parse_destination_suffix)
        {
            query = query.with_destination(d);
        }
        let resolved = ctx.resolve(&query).ok()?.settings;

        stats.merge(common::compare(
            &resolved,
            bs,
            capture,
            mismatch_tally,
            canon_only_tally,
        ));
    }
    Some(stats)
}

#[test]
fn xcconfig_resolution_oracle_coverage() {
    common::pin_capture_host();
    let captures = find_capture_files("_xcconfig_resolution");
    let mut catalogs = CatalogCache::new();

    let mut total = Stats::default();
    let mut per_version: BTreeMap<String, Stats> = BTreeMap::new();
    let mut mismatch_tally: MismatchTally = BTreeMap::new();
    let mut canon_only_tally: MismatchTally = BTreeMap::new();
    let mut skipped: u64 = 0;

    for capture in &captures {
        let Some(version) = capture_xcode_version(capture) else {
            skipped += 1;
            continue;
        };
        let catalog = catalogs.get(&version);
        let Some(stats) = run_capture(capture, catalog, &mut mismatch_tally, &mut canon_only_tally)
        else {
            // The lone ice-cubes capture skips because the captured `raw/` tree
            // is missing the project's own baseConfiguration xcconfig
            // (`IceCubesApp.xcconfig`) that `IceCubesApp.xcodeproj` references —
            // a fixture-capture gap, not a resolver gap. All 23 netnewswire
            // captures resolve cleanly.
            skipped += 1;
            continue;
        };
        total.merge(stats);
        per_version.entry(version).or_default().merge(stats);
    }

    common::print_summary("real-xcconfig oracle", &total, &mismatch_tally);
    println!(
        "(captures found: {}, processed: {}, skipped: {})",
        captures.len(),
        total.files,
        skipped
    );
    println!("\n--- top 30 canonical-only (structural-match) keys ---");
    let mut canon: Vec<(&String, &u64)> = canon_only_tally.iter().collect();
    canon.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in canon.iter().take(30) {
        println!("  {n:<5} {k}");
    }

    // Floors are set JUST UNDER the observed pass rate (88% exact / 99%
    // canonical / 99% structural over the 23 resolvable netnewswire captures)
    // so the test is a regression guard, not an over-fit. The minimum file
    // count guards against the walk silently finding nothing.
    assert!(
        total.files >= 20,
        "expected to process at least 20 xcconfig-resolution captures; got {}",
        total.files
    );
    common::assert_version_floors("real-xcconfig", &per_version, version_floor);
}

/// Per-version `(exact, canonical, structural)` floors for the real-xcconfig
/// captures (netnewswire's `.xcconfig`-driven targets). Set from the first clean
/// multi-version run minus a ~1pt margin.
fn version_floor(version: &str) -> Option<(u64, u64, u64)> {
    // Only 26.0.1 has real-xcconfig captures today (netnewswire wasn't captured
    // for 16.4); a future version with no entry gets the structural safety guard.
    match version {
        "26.5.0" => Some((87, 98, 98)),
        _ => None,
    }
}
