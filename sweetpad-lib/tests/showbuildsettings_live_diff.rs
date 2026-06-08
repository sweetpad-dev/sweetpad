//! Live `xcodebuild -showBuildSettings` differential — the long-tail hunter.
//!
//! The committed settings oracles compare the resolver against *pre-captured*
//! `-showBuildSettings` JSON. That misses two things: settings/combinations no
//! capture exists for, and — crucially — a multiplatform `SDKROOT = auto` target,
//! whose *unbound* capture reports the literal `auto` (so the oracle skips it).
//! This test runs `-showBuildSettings` live and **bound to a concrete `-sdk`**, so
//! `auto` resolves to a real SDK path and every platform of a multiplatform target
//! gets a ground-truth row the pre-captured oracle never had.
//!
//! For each key the resolver produces, it compares our value to xcodebuild's
//! (canonicalized to absorb `$HOME` / DerivedData / Xcode-dir drift). The editor-
//! critical keys that drive `-sdk`/`-target` (`SDKROOT`, `PLATFORM_NAME`, `ARCHS`,
//! the triple inputs) are asserted for committed fixtures; everything else is
//! reported, since this is a discovery sweep, not a byte-for-byte gate.
//!
//! Opt-in (`BSP_LIVE_DIFF=1`): shells out to `xcodebuild` per (target, platform,
//! config), so it's slow and needs Xcode 26.5. `BSP_LIVE_DIFF_ONLY=<slug>` scopes it.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{CatalogCache, canonicalize_value, fixtures_root};
use serde_json::Value;
use sweetpad::build_context::{BuildContext, ResolveQuery};
use sweetpad::project;

const XCODE: &str = "/Applications/Xcode-26.5.0.app";
const XCODE_VERSION: &str = "26.5.0";

const KNOWN_SDKS: &[&str] = &[
    "macosx",
    "iphoneos",
    "iphonesimulator",
    "appletvos",
    "appletvsimulator",
    "watchos",
    "watchsimulator",
    "xros",
    "xrsimulator",
];

/// Keys whose mismatch corrupts the editor `-sdk`/`-target` (and thus stdlib
/// loading) — asserted to match for the committed fixtures.
const CRITICAL_KEYS: &[&str] = &[
    "SDKROOT",
    "PLATFORM_NAME",
    "ARCHS",
    "SWIFT_PLATFORM_TARGET_PREFIX",
];

fn developer_dir() -> String {
    format!("{XCODE}/Contents/Developer")
}

/// Real `buildSettings` for `(target, config, sdk)` via a bound
/// `-showBuildSettings -json`, or `None` if xcodebuild fails (e.g. the target
/// can't resolve for that SDK).
fn xcodebuild_settings(
    xcodeproj: &Path,
    target: &str,
    config: &str,
    sdk: &str,
) -> Option<BTreeMap<String, String>> {
    let out = Command::new("xcodebuild")
        .env("DEVELOPER_DIR", developer_dir())
        .arg("-showBuildSettings")
        .arg("-json")
        .arg("-project")
        .arg(xcodeproj)
        .args(["-target", target, "-configuration", config, "-sdk", sdk])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let json: Value = serde_json::from_slice(&out.stdout).ok()?;
    let settings = json.get(0)?.get("buildSettings")?.as_object()?;
    Some(
        settings
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
    )
}

fn platforms_for(ctx: &BuildContext, target: &str, config: &str) -> Vec<String> {
    let Ok(probe) = ctx.resolve(&ResolveQuery::new(target, config, "macosx", "arm64")) else {
        return vec!["macosx".into()];
    };
    let supported = probe
        .settings
        .get("SUPPORTED_PLATFORMS")
        .cloned()
        .unwrap_or_default();
    let mut set: std::collections::BTreeSet<String> = supported
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|p| KNOWN_SDKS.contains(&p.as_str()))
        .collect();
    if set.is_empty() {
        set.insert("macosx".into());
    }
    set.into_iter().collect()
}

struct Diff {
    key: String,
    ours: String,
    theirs: String,
}

/// Compare the resolver against live xcodebuild for one `(target, platform,
/// config)`, returning a mismatch per resolver key whose canonicalized value
/// differs from xcodebuild's. Keys xcodebuild doesn't emit are skipped (we model
/// some it derives differently); keys we don't emit are out of scope.
fn diff_target(
    ctx: &BuildContext,
    xcodeproj: &Path,
    target: &str,
    platform: &str,
    config: &str,
) -> Option<Vec<Diff>> {
    let theirs = xcodebuild_settings(xcodeproj, target, config, platform)?;
    let ours = ctx
        .resolve(&ResolveQuery::new(target, config, platform, "arm64"))
        .ok()?
        .settings;
    let mut diffs = Vec::new();
    for (key, our_val) in &ours {
        if let Some(their_val) = theirs.get(key)
            && canonicalize_value(our_val) != canonicalize_value(their_val)
        {
            diffs.push(Diff {
                key: key.clone(),
                ours: our_val.clone(),
                theirs: their_val.clone(),
            });
        }
    }
    Some(diffs)
}

fn fixture_projects() -> Vec<(String, PathBuf)> {
    let root = fixtures_root();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("_synthetic-") {
            continue;
        }
        if let Ok(inner) = std::fs::read_dir(entry.path().join("project")) {
            for f in inner.flatten() {
                if f.path().extension().and_then(|e| e.to_str()) == Some("xcodeproj") {
                    out.push((name.clone(), f.path()));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn resolver_matches_live_showbuildsettings() {
    if std::env::var("BSP_LIVE_DIFF").is_err() {
        eprintln!("skipping: set BSP_LIVE_DIFF=1 to run the live -showBuildSettings differential");
        return;
    }
    if !Path::new(XCODE).exists() {
        eprintln!("skipping: {XCODE} not found");
        return;
    }
    let only = std::env::var("BSP_LIVE_DIFF_ONLY").ok();
    let mut cache = CatalogCache::new();
    let catalog = cache.get(XCODE_VERSION).clone();

    let mut critical_failures = Vec::new();
    let mut compared = 0;

    eprintln!("\n===== live -showBuildSettings differential =====");
    for (slug, xcodeproj) in fixture_projects() {
        if only.as_deref().is_some_and(|o| o != slug) {
            continue;
        }
        let Ok(ctx) = BuildContext::open(&xcodeproj).map(|c| c.with_xcspec(catalog.clone())) else {
            continue;
        };
        let configs = if ctx.project.configurations.is_empty() {
            vec!["Debug".to_string()]
        } else {
            ctx.project.configurations.clone()
        };
        for target in ctx.project.targets.clone() {
            if project::is_test_bundle_product_type(target.product_type.as_deref()) {
                continue;
            }
            for config in &configs {
                for platform in platforms_for(&ctx, &target.name, config) {
                    let Some(diffs) =
                        diff_target(&ctx, &xcodeproj, &target.name, &platform, config)
                    else {
                        continue;
                    };
                    compared += 1;
                    let crit: Vec<&Diff> = diffs
                        .iter()
                        .filter(|d| CRITICAL_KEYS.contains(&d.key.as_str()))
                        .collect();
                    eprintln!(
                        "  {slug} / {} [{platform}/{config}]: {} mismatch(es){}",
                        target.name,
                        diffs.len(),
                        if crit.is_empty() {
                            ""
                        } else {
                            "  ⚠ critical"
                        }
                    );
                    for d in &diffs {
                        let mark = if CRITICAL_KEYS.contains(&d.key.as_str()) {
                            "⚠"
                        } else {
                            "·"
                        };
                        eprintln!(
                            "      {mark} {}: ours={:?} xcodebuild={:?}",
                            d.key, d.ours, d.theirs
                        );
                    }
                    for d in crit {
                        critical_failures.push(format!(
                            "{slug}/{} [{platform}/{config}] {}: ours={:?} xcodebuild={:?}",
                            target.name, d.key, d.ours, d.theirs
                        ));
                    }
                }
            }
        }
    }
    eprintln!("  compared {compared} (target, platform, config) cell(s)");
    eprintln!("================================================\n");

    assert!(
        compared > 0,
        "no cells compared — xcodebuild failed everywhere (setup fault)"
    );
    assert!(
        critical_failures.is_empty(),
        "editor-critical settings disagree with live xcodebuild:\n  {}",
        critical_failures.join("\n  ")
    );
}
