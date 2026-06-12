//! Synthetic-override oracle: validate the command-line `KEY=VALUE` override
//! layer ([`ResolveQuery::with_override`]) against `xcodebuild` captures forced
//! with those same overrides.
//!
//! These captures live under
//! `fixtures/<base>/xcode-<ver>/metadata/_synthetic/<override>/build-settings/`
//! and were produced (see `scripts/07_synthetic_overrides.py`) by running
//! `xcodebuild -showBuildSettings -json -scheme S ... KEY=VALUE` for high-value
//! flags that no real corpus project happens to set (library evolution, LTO,
//! arm64e, Swift 6, …). The override KEY=VALUE that produced each capture is
//! encoded only in the `_synthetic/<override>/` directory name, so we carry a
//! small label→overrides table that mirrors the script's `OVERRIDES` list and
//! feed it back through [`ResolveQuery::with_override`].
//!
//! Unlike the scheme-aggregated corpus oracle, these filename suffixes are
//! literal `-destination` slugs (`platform-iOS-Simulator_id-<uuid>`,
//! `generic-platform-iOS`) rather than the `Platform_OSx_Device` shape
//! [`parse_destination_suffix`] understands, so we drive the SDK/arch off the
//! captured `PLATFORM_NAME` / `NATIVE_ARCH_ACTUAL` (the same fields the corpus
//! oracle reads) and let the override carry any arch it forces (`ARCHS=arm64e`).

#![allow(clippy::too_many_lines)]

mod common;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use sweetpad::build_context::{BuildContext, ResolveQuery};
use sweetpad::xcspec;

use common::{
    CatalogCache, MismatchTally, Stats, capture_xcode_version, find_oracles,
    find_xcodeproj_between, print_summary, read_build_settings,
};

/// The `KEY=VALUE` overrides that produced each `_synthetic/<label>/` capture.
/// Mirrors the `OVERRIDES` table in `scripts/07_synthetic_overrides.py`; the
/// label is the directory component right after `_synthetic/`. Returns the
/// parsed `(key, value)` pairs, or `None` for a label we don't know about (so a
/// newly-captured override that we forgot to wire up surfaces as a skip, not a
/// silent pass).
fn overrides_for_label(label: &str) -> Option<&'static [(&'static str, &'static str)]> {
    Some(match label {
        "library-evolution" => &[("BUILD_LIBRARY_FOR_DISTRIBUTION", "YES")],
        "llvm-lto" => &[("LLVM_LTO", "YES")],
        "mergeable-library" => &[("MERGEABLE_LIBRARY", "YES")],
        "strict-concurrency-upcoming" => &[("SWIFT_UPCOMING_FEATURE_STRICT_CONCURRENCY", "YES")],
        "library-evolution+lto" => &[
            ("BUILD_LIBRARY_FOR_DISTRIBUTION", "YES"),
            ("LLVM_LTO", "YES"),
        ],
        "archs-arm64e" => &[("ARCHS", "arm64e")],
        "ldflags-quoted-whitespace" => &[(
            "OTHER_LDFLAGS",
            "-framework \"My Framework\" -Wl,-segalign,0x4000",
        )],
        "swift-version-6" => &[("SWIFT_VERSION", "6.0")],
        "ios-deployment-15" => &[("IPHONEOS_DEPLOYMENT_TARGET", "15.0")],
        "dead-code-stripping-off" => &[("DEAD_CODE_STRIPPING", "NO")],
        "swift-onone" => &[("SWIFT_OPTIMIZATION_LEVEL", "-Onone")],
        "gcc-optimization-s" => &[("GCC_OPTIMIZATION_LEVEL", "s")],
        "enable-bitcode-no" => &[("ENABLE_BITCODE", "NO")],
        _ => return None,
    })
}

/// The `_synthetic/<label>/` component of a synthetic-override oracle path.
fn override_label(oracle: &Path) -> Option<String> {
    let comps: Vec<&OsStr> = oracle.iter().collect();
    let idx = comps.iter().rposition(|c| *c == OsStr::new("_synthetic"))?;
    comps
        .get(idx + 1)
        .and_then(|c| c.to_str())
        .map(str::to_owned)
}

fn run_oracle(
    oracle_path: &Path,
    catalog: &xcspec::Catalog,
    mismatch_tally: &mut MismatchTally,
    canon_only_tally: &mut MismatchTally,
) -> Option<Stats> {
    let label = override_label(oracle_path)?;
    let overrides = overrides_for_label(&label)?;
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

        let xcodeproj = find_xcodeproj_between(oracle_path, "_synthetic", project_name)?;

        let ctx = BuildContext::open(&xcodeproj)
            .ok()?
            .with_xcspec(catalog.clone());
        let mut query = ResolveQuery::new(target, config, sdk, arch);
        for (k, v) in overrides {
            query = query.with_override(*k, *v);
        }
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
fn synthetic_override_oracle_coverage() {
    common::pin_capture_host();
    // `find_oracles()` enumerates every `build-settings/*.json`; the synthetic
    // captures are exactly those under a `_synthetic/` ancestor.
    let oracles: Vec<_> = find_oracles()
        .into_iter()
        .filter(|p| p.iter().any(|c| c == OsStr::new("_synthetic")))
        .collect();
    let mut catalogs = CatalogCache::new();

    let mut total = Stats::default();
    let mut per_label: BTreeMap<String, Stats> = BTreeMap::new();
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
        let label = override_label(path).unwrap_or_else(|| "unknown".into());
        let catalog = catalogs.get(&version);
        let Some(stats) = run_oracle(path, catalog, &mut mismatch_tally, &mut canon_only_tally)
        else {
            let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            if size < 16 {
                empty += 1;
            } else {
                skipped += 1;
            }
            continue;
        };
        total.merge(stats);
        per_label.entry(label).or_default().merge(stats);
        per_version.entry(version).or_default().merge(stats);
    }

    print_summary(
        "Synthetic-override oracle validation",
        &total,
        &mismatch_tally,
    );
    println!(
        "({empty} empty, {skipped} skipped — target/project lookup or unknown override label)"
    );

    println!("\n--- per override (exact / canonical / structural) ---");
    for (k, s) in &per_label {
        println!(
            "  {k:<28} files={:<3} shared={:<6} exact={}% canon={}% struct={}%",
            s.files,
            s.shared_keys,
            s.exact_pct(),
            s.canonical_pct(),
            s.structural_pct(),
        );
    }

    println!("\n--- top 20 canonical-only mismatches (structural matches, path-root drift) ---");
    let mut canon_entries: Vec<(&String, &u64)> = canon_only_tally.iter().collect();
    canon_entries.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in canon_entries.iter().take(20) {
        println!("  {n:<5} {k}");
    }

    // We expect all 13 overrides × 2 configs = 26 captures. Guard against the
    // walk silently finding nothing (e.g. a layout change).
    assert!(
        total.files >= 20,
        "expected to process at least 20 synthetic-override captures; got {} \
         ({empty} empty, {skipped} skipped)",
        total.files
    );

    // Floors set JUST UNDER the observed pass rate (87% exact / 98% canonical /
    // 99% structural). The override layer is the top-priority layer, so the
    // forced KEY=VALUE itself always lands; the residual exact gap is dominated
    // by `BUILD_ACTIVE_RESOURCES_ONLY` (ours=NO, oracle=YES) — a
    // single-concrete-simulator-destination default the resolver can't
    // synthesize here because the capture's `id=<uuid>` filename suffix isn't
    // the parseable `Platform_OSx_Device` shape `parse_destination_suffix`
    // needs. The rest is the same volatile-path / project-root drift the corpus
    // oracle sees (CCHROOT, DSTROOT, INSTALL_DIR, INSTALL_ROOT land in the
    // canonical/structural tiers).
    common::assert_version_floors("synthetic-override", &per_version, version_floor);
}

/// Per-version `(exact, canonical, structural)` floors for the synthetic forced
/// `KEY=VALUE` override captures. The override layer is top-priority so the
/// forced value always lands; the residual exact gap is the usual
/// volatile-path drift (`BUILD_ACTIVE_RESOURCES_ONLY` is now synthesized
/// from the simulator SDK itself, destination suffix or not). Set from the
/// first clean multi-version run minus a ~1pt margin.
fn version_floor(version: &str) -> Option<(u64, u64, u64)> {
    // Only 26.0.1 has synthetic-override captures today; a future version with no
    // entry gets the structural safety guard until its floor is codified.
    match version {
        "26.5.0" => Some((88, 100, 100)),
        _ => None,
    }
}
