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
//! Each target is compared on the platforms its own `SUPPORTED_PLATFORMS`
//! names, read the way a plain `-showBuildSettings` reads it: under the SDK
//! its `SDKROOT` names. A device or macOS platform is bound with `-sdk`. A
//! simulator platform is bound to a concrete simulator, through the scheme
//! that builds the target, because that is how every simulator build runs and
//! what the resolver models: with no device to single out, `-sdk
//! iphonesimulator` alone keeps the full `ARCHS` list that a Debug build for a
//! simulator collapses to the active arch. A simulator cell with no scheme or
//! no simulator to bind falls back to `-sdk` and leaves the run-destination
//! keys out of the comparison.
//!
//! For each key the resolver produces, it compares our value to xcodebuild's
//! (canonicalized to absorb `$HOME` / DerivedData / Xcode-dir drift). The editor-
//! critical keys that drive `-sdk`/`-target` (`SDKROOT`, `PLATFORM_NAME`, `ARCHS`,
//! the triple inputs) and the compilation conditions are asserted for committed
//! fixtures; everything else is reported, since this is a discovery sweep, not a
//! byte-for-byte gate.
//!
//! Opt-in (`BSP_LIVE_DIFF=1`): shells out to `xcodebuild` per (target, platform,
//! config), so it's slow. It runs the Xcode `BSP_ORACLE_XCODE` names, else the
//! selected one ([`oracle_xcode`]), with a `TMPDIR` of the test's own, and
//! resolves against that Xcode's own specs. `BSP_LIVE_DIFF_ONLY=<slug>` scopes it.

mod common;
mod oracle_xcode;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::{canonicalize_value, fixtures_root};
use oracle_xcode::OracleXcode;
use serde_json::Value;
use sweetpad_core::build_context::{BuildContext, ResolveQuery};
use sweetpad_core::scratch::ScratchDir;
use sweetpad_lib::destination::{self, RunDestination};
use sweetpad_lib::{project, xcspec};

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
/// loading), or changes what a file type-checks against — asserted to match
/// for the committed fixtures.
const CRITICAL_KEYS: &[&str] = &[
    "SDKROOT",
    "PLATFORM_NAME",
    "ARCHS",
    "SWIFT_PLATFORM_TARGET_PREFIX",
    "IS_MACCATALYST",
    "LLVM_TARGET_TRIPLE_OS_VERSION",
    "LLVM_TARGET_TRIPLE_SUFFIX",
    "SWIFT_ACTIVE_COMPILATION_CONDITIONS",
    "GCC_PREPROCESSOR_DEFINITIONS",
    "SWIFT_VERSION",
];

/// The keys only a run destination decides: with no simulator bound,
/// xcodebuild's simulator view keeps the full arch list and no active
/// resources, which no simulator build uses.
const RUN_DESTINATION_KEYS: &[&str] = &["ARCHS", "BUILD_ACTIVE_RESOURCES_ONLY", "ONLY_ACTIVE_ARCH"];

/// The Xcode the diff runs, the `TMPDIR` it runs in, and the simulators it
/// can bind a simulator cell to.
struct Live {
    xcode: OracleXcode,
    tmp: ScratchDir,
    simulators: Vec<Simulator>,
}

/// One available simulator: its runtime's platform SDK and OS version, its
/// name and its UDID.
struct Simulator {
    sdk: &'static str,
    os: (u32, u32),
    name: String,
    udid: String,
}

impl Simulator {
    /// The run destination a build for this simulator resolves under.
    fn destination(&self) -> Option<RunDestination> {
        let label = match self.sdk {
            "iphonesimulator" => "iOS Simulator",
            "appletvsimulator" => "tvOS Simulator",
            "watchsimulator" => "watchOS Simulator",
            "xrsimulator" => "visionOS Simulator",
            _ => return None,
        };
        destination::parse_destination_arg(&format!(
            "platform={label},OS={}.{},name={}",
            self.os.0, self.os.1, self.name
        ))
    }
}

/// The simulators `simctl` lists as available for this Xcode, newest runtime
/// first. Empty when `simctl` can't answer.
fn available_simulators(xcode: &OracleXcode, tmp: &Path) -> Vec<Simulator> {
    let Ok(out) = xcode
        .command("xcrun", tmp)
        .args(["simctl", "list", "devices", "available", "-j"])
        .output()
    else {
        return Vec::new();
    };
    let json: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    let mut sims = Vec::new();
    for (runtime, devices) in json
        .get("devices")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        // `com.apple.CoreSimulator.SimRuntime.iOS-27-0`
        let Some((family, version)) = runtime.rsplit('.').next().and_then(|r| r.split_once('-'))
        else {
            continue;
        };
        let sdk = match family {
            "iOS" => "iphonesimulator",
            "tvOS" => "appletvsimulator",
            "watchOS" => "watchsimulator",
            "xrOS" => "xrsimulator",
            _ => continue,
        };
        let mut parts = version.split('-').map(|p| p.parse::<u32>().unwrap_or(0));
        let os = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        for device in devices.as_array().into_iter().flatten() {
            if let (Some(name), Some(udid)) = (
                device.get("name").and_then(Value::as_str),
                device.get("udid").and_then(Value::as_str),
            ) {
                sims.push(Simulator {
                    sdk,
                    os,
                    name: name.to_string(),
                    udid: udid.to_string(),
                });
            }
        }
    }
    sims.sort_by(|a, b| b.os.cmp(&a.os).then_with(|| a.name.cmp(&b.name)));
    sims
}

/// The defaults catalog of the Xcode under test, stamped with its location and
/// version the way the resolver's own `--xcode` loading stamps it, so ours and
/// xcodebuild's settings come from the same install.
fn catalog(xcode: &OracleXcode) -> xcspec::Catalog {
    let layout = &xcode.layout;
    let mut catalog = xcspec::load_catalog(&layout.xcspec_root, Some(&layout.sdksettings_root))
        .unwrap_or_else(|e| {
            panic!(
                "failed to load the xcspec catalog from {}: {e}",
                layout.xcspec_root.display()
            )
        });
    catalog.developer_dir = Some(layout.developer_dir.to_string_lossy().into_owned());
    catalog.xcode_version = Some(layout.short_version.clone()).filter(|v| !v.is_empty());
    catalog.product_build_version = Some(layout.build_version.clone()).filter(|v| !v.is_empty());
    catalog
}

/// How xcodebuild is asked for one cell's settings.
enum Binding<'a> {
    /// `-target <target> -sdk <sdk>`.
    Sdk(&'a str),
    /// `-scheme <scheme> -destination id=<udid>`, read for the target.
    Simulator { scheme: String, udid: &'a str },
}

/// Real `buildSettings` for `target` in `config` under `binding`, via
/// `-showBuildSettings -json`, or `None` if xcodebuild fails (e.g. the target
/// can't resolve for that SDK).
fn xcodebuild_settings(
    live: &Live,
    xcodeproj: &Path,
    target: &str,
    config: &str,
    binding: &Binding<'_>,
) -> Option<BTreeMap<String, String>> {
    let mut cmd = live.xcode.command("xcodebuild", &live.tmp);
    cmd.arg("-showBuildSettings")
        .arg("-json")
        .arg("-project")
        .arg(xcodeproj)
        .args(["-configuration", config]);
    match binding {
        Binding::Sdk(sdk) => cmd.args(["-target", target, "-sdk", sdk]),
        Binding::Simulator { scheme, udid } => cmd
            .args(["-scheme", scheme])
            .args(["-destination", &format!("id={udid}")]),
    };
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let json: Value = serde_json::from_slice(&out.stdout).ok()?;
    let entry = json
        .as_array()?
        .iter()
        .find(|e| e.get("target").and_then(Value::as_str) == Some(target))?;
    let settings = entry.get("buildSettings")?.as_object()?;
    Some(
        settings
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
    )
}

/// The platforms `target` supports, read from `SUPPORTED_PLATFORMS` under the
/// SDK its own `SDKROOT` names, as a plain `-showBuildSettings` resolves it.
/// Probing under a fixed SDK instead reads that SDK's defaults: an iOS-only
/// target probed under `macosx` reports `macosx`, and gets diffed on a
/// platform it doesn't build for.
fn platforms_for(ctx: &BuildContext, target: &str, config: &str) -> Vec<String> {
    let Ok(probe) = ctx.resolve(&ResolveQuery::new(target, config, "", "")) else {
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

/// One compared cell: its mismatches, and whether a simulator platform fell
/// back to the destination-less view.
struct Cell {
    diffs: Vec<Diff>,
    unbound_simulator: bool,
}

/// Compare the resolver against live xcodebuild for one `(target, platform,
/// config)`, returning a mismatch per resolver key whose canonicalized value
/// differs from xcodebuild's. Keys xcodebuild doesn't emit are skipped (we model
/// some it derives differently); keys we don't emit are out of scope.
fn diff_target(
    live: &Live,
    ctx: &BuildContext,
    xcodeproj: &Path,
    target: &str,
    platform: &str,
    config: &str,
) -> Option<Cell> {
    let simulator = platform
        .ends_with("simulator")
        .then(|| {
            let scheme = project::scheme_for_target(xcodeproj, target)?;
            let sim = live.simulators.iter().find(|s| s.sdk == platform)?;
            Some((scheme, sim, sim.destination()?))
        })
        .flatten();
    let unbound_simulator = platform.ends_with("simulator") && simulator.is_none();
    let (theirs, query) = match &simulator {
        Some((scheme, sim, dest)) => (
            xcodebuild_settings(
                live,
                xcodeproj,
                target,
                config,
                &Binding::Simulator {
                    scheme: scheme.clone(),
                    udid: &sim.udid,
                },
            )?,
            ResolveQuery::new(target, config, platform, dest.arch.clone())
                .with_destination(dest.clone()),
        ),
        None => (
            xcodebuild_settings(live, xcodeproj, target, config, &Binding::Sdk(platform))?,
            ResolveQuery::new(target, config, platform, ""),
        ),
    };
    let ours = ctx.resolve(&query).ok()?.settings;
    let mut diffs = Vec::new();
    for (key, our_val) in &ours {
        if unbound_simulator && RUN_DESTINATION_KEYS.contains(&key.as_str()) {
            continue;
        }
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
    Some(Cell {
        diffs,
        unbound_simulator,
    })
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
    let Some(xcode) = oracle_xcode::find() else {
        return;
    };
    let only = std::env::var("BSP_LIVE_DIFF_ONLY").ok();
    let catalog = catalog(&xcode);
    let tmp = ScratchDir::new("sweetpad-live-diff-tmp").unwrap();
    let simulators = available_simulators(&xcode, &tmp);
    let live = Live {
        xcode,
        tmp,
        simulators,
    };

    let mut critical_failures = Vec::new();
    let mut compared = 0;

    eprintln!("\n===== live -showBuildSettings differential =====");
    for (slug, xcodeproj) in fixture_projects() {
        if only.as_deref().is_some_and(|o| o != slug) {
            continue;
        }
        // The comparison is against this machine's xcodebuild, so follow the
        // Xcode location settings it obeys. `canonicalize_value` only rewrites
        // the hash inside a literal `DerivedData/` segment, so a custom
        // location would otherwise diff on every path-valued key.
        let Ok(ctx) = BuildContext::open(&xcodeproj)
            .map(|c| c.with_xcspec(catalog.clone()).with_xcode_locations())
        else {
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
                    let Some(cell) =
                        diff_target(&live, &ctx, &xcodeproj, &target.name, &platform, config)
                    else {
                        continue;
                    };
                    compared += 1;
                    let crit: Vec<&Diff> = cell
                        .diffs
                        .iter()
                        .filter(|d| CRITICAL_KEYS.contains(&d.key.as_str()))
                        .collect();
                    eprintln!(
                        "  {slug} / {} [{platform}/{config}]: {} mismatch(es){}{}",
                        target.name,
                        cell.diffs.len(),
                        if cell.unbound_simulator {
                            "  (no simulator bound: run-destination keys not compared)"
                        } else {
                            ""
                        },
                        if crit.is_empty() {
                            ""
                        } else {
                            "  ⚠ critical"
                        }
                    );
                    for d in &cell.diffs {
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
