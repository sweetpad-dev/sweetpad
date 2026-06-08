//! Structural invariants on the BSP editor arguments — the cheapest net, needing
//! no build and no captured oracle. For every committed fixture target it
//! resolves the editor `swiftc` args for every `(platform, configuration)` and
//! asserts properties that must hold for *any* correct invocation, independently
//! of what Xcode would emit:
//!
//! - **one real `-sdk`** — exactly one, a recognizable SDK, never the `auto`
//!   sentinel or an unexpanded `$(…)`;
//! - **`-target` agrees with `-sdk`** — the triple's platform family + simulator
//!   bit match the SDK's (a `-target arm64-apple-macos` under `-sdk iphoneos`
//!   is the mismatch that makes the compiler refuse to load the stdlib);
//! - **no leaked build variables** — no argument carries an unexpanded `$(…)`;
//! - **a valid `-module-name`**.
//!
//! The platform sweep is the point: a multiplatform `SDKROOT = auto` target is
//! resolved once per platform in its `SUPPORTED_PLATFORMS`, so a regression that
//! stops binding `auto` to a concrete SDK fails here on the very first iOS pass —
//! the SDKROOT=auto class of bug, caught two independent ways (`-sdk` and
//! `-target`) with no fixture capture required.
//!
//! Committed `_synthetic-*` fixtures are an asserted gate (zero violations). When
//! a real corpus is present (`corpus/<slug>/`) it is swept too, but only
//! *reported* — a real app may exhibit a corner we haven't closed, and surfacing
//! it is the hunting half; it must not break `cargo test`.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::{CatalogCache, fixtures_root};
use sweetpad::build_context::{BuildContext, ResolveQuery};
use sweetpad::{compiler_args, project};

/// The Xcode whose xcspec defaults + compiler options drive resolution here —
/// the installed toolchain the corpus is captured against.
const XCODE_VERSION: &str = "26.5.0";

/// SDK names the editor can resolve against. The platform sweep intersects a
/// target's `SUPPORTED_PLATFORMS` with this set so an unknown token can't pull a
/// resolve toward a catalog SDK that doesn't exist.
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

/// A platform family, paired with whether it's the simulator variant — the unit
/// the `-sdk` and `-target` checks compare on.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Platform {
    family: &'static str,
    simulator: bool,
}

/// Classify an `-sdk` value (a path like `…/iPhoneSimulator26.5.sdk` or a bare
/// name like `iphonesimulator`) into its platform. `None` for an unrecognizable
/// value — `auto`, empty, or a leaked variable — which the caller flags.
fn sdk_platform(sdk: &str) -> Option<Platform> {
    let s = sdk.to_lowercase();
    let simulator = s.contains("simulator");
    let family = if s.contains("iphone") {
        "ios"
    } else if s.contains("appletv") {
        "tvos"
    } else if s.contains("watch") {
        "watchos"
    } else if s.contains("xr") {
        "xros"
    } else if s.contains("macos") {
        "macos"
    } else {
        return None;
    };
    Some(Platform { family, simulator })
}

/// Classify a `-target` triple (`arm64-apple-ios26.5-simulator`,
/// `arm64-apple-macos26.5`, `…-macabi`) into its platform. Mac Catalyst
/// (`-macabi`) builds against the macOS SDK, so it classifies as `macos`.
fn triple_platform(triple: &str) -> Option<Platform> {
    let t = triple.to_lowercase();
    if t.contains("macabi") {
        return Some(Platform {
            family: "macos",
            simulator: false,
        });
    }
    let simulator = t.contains("simulator");
    let family = if t.contains("tvos") {
        "tvos"
    } else if t.contains("watchos") {
        "watchos"
    } else if t.contains("xros") {
        "xros"
    } else if t.contains("macos") {
        "macos"
    } else if t.contains("ios") {
        "ios"
    } else {
        return None;
    };
    Some(Platform { family, simulator })
}

/// Mirror of `bsp::editor_sdk_for`: the SDK the editor picks for a target from
/// its resolved `SDKROOT` + `SUPPORTED_PLATFORMS`, mapping device platforms to
/// their simulator and never yielding `auto`. Kept in sync with the shipped
/// chooser so the sweep covers the platform the editor would actually pick (the
/// `bsp::editor_sdk_for` unit tests guard the mapping itself).
fn editor_sdk_for(sdkroot: &str, supported_platforms: &str) -> &'static str {
    let sdkroot = sdkroot.trim().to_lowercase();
    let platform = if sdkroot.is_empty() || sdkroot == "auto" {
        supported_platforms.to_lowercase()
    } else {
        sdkroot
    };
    if platform.contains("iphone") {
        "iphonesimulator"
    } else if platform.contains("appletv") {
        "appletvsimulator"
    } else if platform.contains("watch") {
        "watchsimulator"
    } else if platform.contains("xr") {
        "xrsimulator"
    } else {
        "macosx"
    }
}

fn is_valid_module_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The single value following `flag` in `args` (e.g. the path after `-sdk`),
/// and how many times `flag` appears — a flag that must be unique reports its
/// count so a duplicate (the "last `-sdk` wins" hazard) is itself a violation.
fn flag_value<'a>(args: &'a [String], flag: &str) -> (Option<&'a str>, usize) {
    let mut value = None;
    let mut count = 0;
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag {
            count += 1;
            if value.is_none() {
                value = args.get(i + 1).map(String::as_str);
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    (value, count)
}

/// Every value carried by any occurrence of `flag` — for the path flags that
/// repeat (`-I`, `-F`).
fn all_values<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == flag {
            out.push(args[i + 1].as_str());
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

/// Check one resolved editor argument vector, returning a violation string per
/// broken invariant (empty = clean).
fn check_invariants(args: &[String]) -> Vec<String> {
    let mut v = Vec::new();

    // No argument may carry an unexpanded build variable.
    for a in args {
        if a.contains("$(") {
            v.push(format!("unexpanded build variable in argument: {a:?}"));
        }
    }

    // Exactly one real -sdk.
    let (sdk, sdk_n) = flag_value(args, "-sdk");
    let sdk_plat = match (sdk, sdk_n) {
        (_, 0) => {
            v.push("no -sdk emitted".into());
            None
        }
        (_, n) if n > 1 => {
            v.push(format!(
                "-sdk emitted {n} times (the last one wins, masking the others)"
            ));
            sdk.and_then(sdk_platform)
        }
        (Some(s), _) if s == "auto" || s.is_empty() => {
            v.push(format!(
                "-sdk is the unbound sentinel {s:?} (no standard library loads)"
            ));
            None
        }
        (Some(s), _) => {
            let p = sdk_platform(s);
            if p.is_none() {
                v.push(format!("-sdk value is not a recognizable SDK: {s:?}"));
            }
            p
        }
        (None, _) => None,
    };

    // Exactly one -target, agreeing with the -sdk's platform.
    let (target, target_n) = flag_value(args, "-target");
    match (target, target_n) {
        (_, 0) => v.push("no -target emitted".into()),
        (_, n) if n > 1 => v.push(format!("-target emitted {n} times")),
        (Some(t), _) => match (triple_platform(t), sdk_plat) {
            (None, _) => v.push(format!("-target triple not recognizable: {t:?}")),
            (Some(tp), Some(sp)) if tp != sp => v.push(format!(
                "-target {t:?} (platform {tp:?}) disagrees with -sdk (platform {sp:?})"
            )),
            _ => {}
        },
        (None, _) => {}
    }

    // A valid -module-name.
    match flag_value(args, "-module-name").0 {
        None => v.push("no -module-name emitted".into()),
        Some(m) if !is_valid_module_name(m) => {
            v.push(format!("-module-name is not a valid identifier: {m:?}"));
        }
        Some(_) => {}
    }

    // Search-path / plugin flags must be absolute and variable-free (existence is
    // a build-tier concern; structure is checkable without one).
    for flag in ["-I", "-F", "-import-objc-header"] {
        for path in all_values(args, flag) {
            if !path.starts_with('/') && !path.is_empty() {
                v.push(format!("{flag} path is not absolute: {path:?}"));
            }
        }
    }

    v
}

/// One target's editor args for a `(platform, configuration)`, or `None` when the
/// target can't be resolved for that platform (skipped, not a violation).
fn editor_args(
    ctx: &BuildContext,
    swift_opts: &[sweetpad::xcspec::CompilerOption],
    xcodeproj: &Path,
    target: &str,
    platform: &str,
    config: &str,
) -> Option<Vec<String>> {
    let query = ResolveQuery::new(target, config, platform, "arm64");
    let resolved = ctx.resolve(&query).ok()?;
    let has_pkg = project::target_has_package_products(xcodeproj, target).unwrap_or(false);
    Some(compiler_args::swift_arguments(
        &resolved.settings,
        "arm64",
        swift_opts,
        XCODE_VERSION,
        has_pkg,
        &[],
    ))
}

/// The platforms to sweep for a target: every `SUPPORTED_PLATFORMS` entry the
/// catalog knows, plus the SDK the editor would actually pick (which may be a
/// simulator absent from `SUPPORTED_PLATFORMS`).
fn platforms_for(ctx: &BuildContext, target: &str, config: &str) -> Vec<String> {
    let Ok(probe) = ctx.resolve(&ResolveQuery::new(target, config, "macosx", "arm64")) else {
        return vec!["macosx".into()];
    };
    let supported = probe
        .settings
        .get("SUPPORTED_PLATFORMS")
        .cloned()
        .unwrap_or_default();
    let sdkroot = probe.settings.get("SDKROOT").cloned().unwrap_or_default();

    let mut set: BTreeSet<String> = supported
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|p| KNOWN_SDKS.contains(&p.as_str()))
        .collect();
    set.insert(editor_sdk_for(&sdkroot, &supported).to_string());
    set.into_iter().collect()
}

/// A violation with enough context to localize it.
struct Violation {
    slug: String,
    target: String,
    platform: String,
    config: String,
    message: String,
}

/// Sweep one project's non-test targets across `(platform, config)` and collect
/// every invariant violation.
fn sweep_project(
    slug: &str,
    xcodeproj: &Path,
    swift_opts: &[sweetpad::xcspec::CompilerOption],
    catalog: &sweetpad::xcspec::Catalog,
) -> Vec<Violation> {
    let mut out = Vec::new();
    let Ok(ctx) = BuildContext::open(xcodeproj).map(|c| c.with_xcspec(catalog.clone())) else {
        return out;
    };
    let configs = if ctx.project.configurations.is_empty() {
        vec!["Debug".to_string()]
    } else {
        ctx.project.configurations.clone()
    };
    for target in &ctx.project.targets {
        if project::is_test_bundle_product_type(target.product_type.as_deref()) {
            continue;
        }
        for config in &configs {
            for platform in platforms_for(&ctx, &target.name, config) {
                let Some(args) =
                    editor_args(&ctx, swift_opts, xcodeproj, &target.name, &platform, config)
                else {
                    continue;
                };
                for message in check_invariants(&args) {
                    out.push(Violation {
                        slug: slug.to_string(),
                        target: target.name.clone(),
                        platform: platform.clone(),
                        config: config.clone(),
                        message,
                    });
                }
            }
        }
    }
    out
}

/// The committed `_synthetic-*` fixtures that carry a generated `project/*.xcodeproj`.
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
        let project_dir = entry.path().join("project");
        if let Ok(inner) = std::fs::read_dir(&project_dir) {
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

/// Top-level corpus clones (`corpus/<slug>/<X>.xcodeproj`), present only on a
/// developer checkout — swept opportunistically and reported, never asserted.
fn corpus_projects() -> Vec<(String, PathBuf)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return out;
    };
    for entry in entries.flatten() {
        let slug = entry.file_name().to_string_lossy().into_owned();
        if slug.starts_with('_') {
            continue; // synthetic mirrors live under fixtures/
        }
        if let Ok(inner) = std::fs::read_dir(entry.path()) {
            for f in inner.flatten() {
                if f.path().extension().and_then(|e| e.to_str()) == Some("xcodeproj") {
                    out.push((slug.clone(), f.path()));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn bsp_editor_args_satisfy_invariants() {
    let mut cache = CatalogCache::new();
    let catalog = cache.get(XCODE_VERSION).clone();
    let swift_opts = catalog
        .compiler_options
        .get("com.apple.xcode.tools.swift.compiler")
        .map_or(&[][..], Vec::as_slice)
        .to_vec();

    // Asserted gate: the committed fixtures must be flawless on every platform.
    let fixtures = fixture_projects();
    assert!(
        !fixtures.is_empty(),
        "no _synthetic-*/project/*.xcodeproj fixtures found"
    );
    let mut violations = Vec::new();
    let mut swept = 0;
    for (slug, xcodeproj) in &fixtures {
        let found = sweep_project(slug, xcodeproj, &swift_opts, &catalog);
        swept += 1;
        violations.extend(found);
    }

    eprintln!("\n===== BSP arg invariants: swept {swept} committed fixture project(s) =====");
    if violations.is_empty() {
        eprintln!("  all fixtures clean across every (platform, configuration)");
    } else {
        for x in &violations {
            eprintln!(
                "  ✗ {} / {} [{}/{}]: {}",
                x.slug, x.target, x.platform, x.config, x.message
            );
        }
    }

    // Hunting half: report (never fail on) any corner the real corpus surfaces.
    let corpus = corpus_projects();
    if !corpus.is_empty() {
        let mut corpus_violations = Vec::new();
        for (slug, xcodeproj) in &corpus {
            corpus_violations.extend(sweep_project(slug, xcodeproj, &swift_opts, &catalog));
        }
        eprintln!(
            "----- corpus sweep ({} project(s), reported only) -----",
            corpus.len()
        );
        if corpus_violations.is_empty() {
            eprintln!("  corpus clean");
        } else {
            for x in &corpus_violations {
                eprintln!(
                    "  · {} / {} [{}/{}]: {}",
                    x.slug, x.target, x.platform, x.config, x.message
                );
            }
        }
    }
    eprintln!("================================================================\n");

    assert!(
        violations.is_empty(),
        "{} invariant violation(s) on committed fixtures (listed above)",
        violations.len()
    );
}
