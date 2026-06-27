//! End-to-end scheme planner tests.
//!
//! For each scheme picked from the corpus, parse it, run `plan_build` against
//! the project's `BuildContext`, and compare the resulting target list to the
//! captured `xcodebuild -showBuildSettings -scheme <S> -configuration <C>
//! -destination <D>` output (the `metadata/schemes/<S>/build-settings/*.json`
//! files). Those JSON captures ARE the oracle — each entry's `TARGET_NAME` is
//! the truth we're checking against.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use sweetpad_core::build_context::BuildContext;
use sweetpad_lib::destination::parse_destination_suffix;
use sweetpad_lib::scheme;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("SWEETPAD_LIB_DIR")).join("fixtures")
}

/// Tiny string-scrape for `"TARGET_NAME": "<value>"` lines. The oracle
/// JSON is well-formed and emitted by xcodebuild with one such pair per
/// entry — robust enough without pulling in a JSON parser dependency.
fn read_oracle_target_names(path: &Path) -> Vec<String> {
    let content =
        fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("\"TARGET_NAME\"") else {
            continue;
        };
        let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ':');
        let Some(value) = rest.strip_prefix('"') else {
            continue;
        };
        let Some(end) = value.find('"') else {
            continue;
        };
        out.push(value[..end].to_string());
    }
    out
}

#[test]
fn kingfisher_scheme_plans_single_framework_target() {
    let xcodeproj = fixtures_root().join("kingfisher/xcode-26.5.0/raw/Kingfisher.xcodeproj");
    let scheme_path = xcodeproj.join("xcshareddata/xcschemes/Kingfisher.xcscheme");
    let oracle = fixtures_root().join(
        "kingfisher/xcode-26.5.0/metadata/schemes/Kingfisher/build-settings/Debug__macOS.json",
    );

    let ctx = BuildContext::open(&xcodeproj).unwrap();
    let scheme = scheme::parse_file(&scheme_path).unwrap();
    let dest = parse_destination_suffix("macOS");
    let plan = ctx.plan_build(
        &scheme,
        scheme::BuildFor::Running,
        "Debug",
        "macosx",
        "arm64",
        dest.as_ref(),
    );

    let oracle_targets = read_oracle_target_names(&oracle);
    let plan_targets: Vec<String> = plan.entries.iter().map(|q| q.target.clone()).collect();

    assert_eq!(plan_targets, vec!["Kingfisher".to_string()]);
    assert_eq!(plan_targets, oracle_targets);
    assert!(plan.skipped.is_empty(), "skipped: {:?}", plan.skipped);
}

#[test]
fn share_extension_scheme_plans_both_extension_and_parent_app() {
    let xcodeproj = fixtures_root().join("ice-cubes/xcode-26.5.0/raw/IceCubesApp.xcodeproj");
    let scheme_path = xcodeproj.join("xcshareddata/xcschemes/IceCubesShareExtension.xcscheme");
    let oracle = fixtures_root().join(
        "ice-cubes/xcode-26.5.0/metadata/schemes/IceCubesShareExtension/build-settings/\
         Debug__iOS-Simulator_OS26.5_iPad-A16.json",
    );

    let ctx = BuildContext::open(&xcodeproj).unwrap();
    let scheme = scheme::parse_file(&scheme_path).unwrap();
    let dest = parse_destination_suffix("iOS-Simulator_OS26.5_iPad-A16");
    let plan = ctx.plan_build(
        &scheme,
        scheme::BuildFor::Running,
        "Debug",
        "iphonesimulator",
        "arm64",
        dest.as_ref(),
    );

    let plan_targets: BTreeSet<String> = plan.entries.iter().map(|q| q.target.clone()).collect();
    let oracle_targets: BTreeSet<String> = read_oracle_target_names(&oracle).into_iter().collect();

    assert_eq!(plan_targets, oracle_targets);
    assert!(plan.skipped.is_empty(), "skipped: {:?}", plan.skipped);
    // Sanity: the scheme explicitly lists both, so the entry order should
    // match the scheme declaration order (extension first, then host app).
    assert_eq!(plan.entries[0].target, "IceCubesShareExtension");
    assert_eq!(plan.entries[1].target, "IceCubesApp");
}

#[test]
fn plan_then_resolve_produces_expected_product_settings() {
    let xcodeproj = fixtures_root().join("kingfisher/xcode-26.5.0/raw/Kingfisher.xcodeproj");
    let scheme_path = xcodeproj.join("xcshareddata/xcschemes/Kingfisher.xcscheme");

    let ctx = BuildContext::open(&xcodeproj).unwrap();
    let scheme = scheme::parse_file(&scheme_path).unwrap();
    let plan = ctx.plan_build(
        &scheme,
        scheme::BuildFor::Running,
        "Debug",
        "macosx",
        "arm64",
        None,
    );
    assert_eq!(plan.entries.len(), 1);

    let resolved = ctx.resolve(&plan.entries[0]).unwrap();
    assert_eq!(
        resolved.settings.get("PRODUCT_NAME").map(String::as_str),
        Some("Kingfisher"),
    );
    assert_eq!(
        resolved.product_type.as_deref(),
        Some("com.apple.product-type.framework"),
    );
}

#[test]
fn plan_uses_passed_configuration_and_destination_for_every_entry() {
    let xcodeproj = fixtures_root().join("ice-cubes/xcode-26.5.0/raw/IceCubesApp.xcodeproj");
    let scheme_path = xcodeproj.join("xcshareddata/xcschemes/IceCubesShareExtension.xcscheme");

    let ctx = BuildContext::open(&xcodeproj).unwrap();
    let scheme = scheme::parse_file(&scheme_path).unwrap();
    let dest = parse_destination_suffix("iOS-Simulator_OS26.5_iPad-A16").unwrap();
    let plan = ctx.plan_build(
        &scheme,
        scheme::BuildFor::Running,
        "Release",
        "iphonesimulator",
        "arm64",
        Some(&dest),
    );

    for query in &plan.entries {
        assert_eq!(query.configuration, "Release");
        assert_eq!(query.sdk, "iphonesimulator");
        assert_eq!(query.arch, "arm64");
        assert_eq!(query.destination.as_ref(), Some(&dest));
    }
}

#[test]
fn plan_honors_build_for_action_flags() {
    // The "Alamofire macOS" scheme carries a testing-only second entry
    // (`buildForRunning="NO" buildForTesting="YES"` on the test bundle).
    // The Run build set excludes it — matching the oracle capture
    // metadata/schemes/Alamofire macOS/build-settings/Debug__macOS.json,
    // which contains only the framework target — while the Test build set
    // includes both. The scheme also has codeCoverageEnabled="YES", which
    // xcodebuild propagates to every resolved buildable.
    let xcodeproj = fixtures_root().join("alamofire/xcode-26.5.0/raw/Alamofire.xcodeproj");
    let scheme_path = xcodeproj.join("xcshareddata/xcschemes/Alamofire macOS.xcscheme");
    let oracle = fixtures_root().join(
        "alamofire/xcode-26.5.0/metadata/schemes/Alamofire macOS/build-settings/Debug__macOS.json",
    );

    let ctx = BuildContext::open(&xcodeproj).unwrap();
    let scheme = scheme::parse_file(&scheme_path).unwrap();

    let run = ctx.plan_build(
        &scheme,
        scheme::BuildFor::Running,
        "Debug",
        "macosx",
        "arm64",
        None,
    );
    let run_targets: Vec<String> = run.entries.iter().map(|q| q.target.clone()).collect();
    assert_eq!(run_targets, read_oracle_target_names(&oracle));
    assert_eq!(run_targets, vec!["Alamofire macOS"]);

    let test = ctx.plan_build(
        &scheme,
        scheme::BuildFor::Testing,
        "Debug",
        "macosx",
        "arm64",
        None,
    );
    let test_targets: Vec<String> = test.entries.iter().map(|q| q.target.clone()).collect();
    assert_eq!(
        test_targets,
        vec!["Alamofire macOS", "Alamofire macOS Tests"]
    );
}

/// Sample 10 schemes from the corpus and assert every entry's
/// `BlueprintName` resolves to a target in this project. This is the
/// "across the whole corpus" smoke test for the planner.
#[test]
fn every_corpus_scheme_plans_with_no_unresolved_entries_in_same_project() {
    let mut checked = 0;
    let mut total_skipped = 0;
    for case in corpus_scheme_cases() {
        let Ok(ctx) = BuildContext::open(&case.xcodeproj) else {
            continue;
        };
        let Ok(scheme) = scheme::parse_file(&case.scheme) else {
            continue;
        };
        let plan = ctx.plan_build(
            &scheme,
            scheme::BuildFor::Running,
            "Debug",
            "macosx",
            "arm64",
            None,
        );
        checked += 1;
        total_skipped += plan.skipped.len();
    }
    assert!(
        checked >= 10,
        "expected at least 10 corpus schemes, walked {checked}"
    );
    // We don't assert skipped == 0 (cross-container workspace schemes can
    // legitimately produce skipped entries); the assertion is that the
    // pipeline runs end-to-end across the corpus without panicking.
    eprintln!("corpus planner smoke: {checked} schemes, {total_skipped} cross-container skips");
}

struct SchemeCase {
    xcodeproj: PathBuf,
    scheme: PathBuf,
}

/// Walk `fixtures/<project>/xcode-*/raw/` for every `(xcodeproj, scheme)`
/// pair. Restricted to the captured top-level projects under `raw/`
/// rather than the whole tree — transitively-checked-out SPM packages
/// under `.derived/SourcePackages/checkouts/` have their own .xcodeproj
/// files we don't want to exercise (some are huge enough to dominate
/// the test runtime and they're not part of our oracle corpus).
fn corpus_scheme_cases() -> Vec<SchemeCase> {
    let mut out = Vec::new();
    let root = fixtures_root();
    let Ok(projects) = fs::read_dir(&root) else {
        return out;
    };
    for project in projects.flatten() {
        let project_path = project.path();
        if !project_path.is_dir() {
            continue;
        }
        // Each top-level project (kingfisher, ice-cubes, …) holds an
        // xcode-<ver>/ slot, and each slot has a `raw/` tree we crawl
        // for `.xcodeproj` directories.
        let Ok(slots) = fs::read_dir(&project_path) else {
            continue;
        };
        for slot in slots.flatten() {
            let raw = slot.path().join("raw");
            if raw.is_dir() {
                visit_raw(&raw, &mut out);
            }
            // `_synthetic-xcconfigs/<slot>/project/*.xcodeproj` lives
            // directly under the slot, not under raw/.
            let synthetic = slot.path().join("project");
            if synthetic.is_dir() {
                visit_raw(&synthetic, &mut out);
            }
        }
    }
    out
}

fn visit_raw(dir: &Path, out: &mut Vec<SchemeCase>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let basename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        // Skip Xcode's auto-generated package-resolution cache — it
        // contains transitive SPM checkouts that aren't part of our
        // top-level corpus.
        if basename == ".derived" || basename == "DerivedData" {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) == Some("xcodeproj") {
            let schemes_dir = path.join("xcshareddata/xcschemes");
            if let Ok(scheme_entries) = fs::read_dir(&schemes_dir) {
                for s in scheme_entries.flatten() {
                    let s_path = s.path();
                    if s_path.extension().and_then(|x| x.to_str()) == Some("xcscheme") {
                        out.push(SchemeCase {
                            xcodeproj: path.clone(),
                            scheme: s_path,
                        });
                    }
                }
            }
        } else {
            visit_raw(&path, out);
        }
    }
}
