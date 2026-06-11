//! Compiler-argument oracle: score generated `swiftc`/`clang`/`ld` argv against
//! the literal commands a real build executed (captured under
//! `fixtures/<slug>/xcode-<ver>/compiler-args/`).
//!
//! Phase 1 builds the comparator and proves it on the captured oracle itself
//! (identity → 100%, injected defects classify correctly). The generate-and-
//! score pass lands in later phases once the Swift generator exists; until then
//! this guards the scoring core and the captured fixtures' integrity.

#![allow(
    clippy::too_many_lines,
    clippy::case_sensitive_file_extension_comparisons
)]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::argv::{self, ArgvStats, compare_argv, parse_argv, print_argv_summary};
use common::{
    CatalogCache, MismatchTally, canonicalize_value, capture_xcode_version,
    find_compiler_args_oracles, fixtures_root,
};
use sweetpad::build_context::{BuildContext, ResolveQuery};
use sweetpad::{compiler_args, project, scheme};

// ----- comparator unit tests -----------------------------------------------

fn argv(tokens: &[&str]) -> Vec<String> {
    tokens.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn parse_argv_pairs_attached_and_standalone() {
    let items = parse_argv(&argv(&[
        "-module-name",
        "Alamofire",
        "-Onone",
        "-DDEBUG",
        "-I/inc",
        "-g",
    ]));
    // (-module-name Alamofire) pair, (-Onone) standalone, (-D DEBUG) split,
    // (-I /inc) split, (-g) standalone.
    assert_eq!(items.len(), 5);
    assert_eq!(items[0].flag, "-module-name");
    assert_eq!(items[0].value.as_deref(), Some("Alamofire"));
    assert_eq!(items[1].flag, "-Onone");
    assert_eq!(items[1].value, None);
    assert_eq!(items[2].flag, "-D");
    assert_eq!(items[2].value.as_deref(), Some("DEBUG"));
    assert_eq!(items[3].flag, "-I");
    assert_eq!(items[3].value.as_deref(), Some("/inc"));
}

#[test]
fn identity_scores_all_exact() {
    let a = argv(&[
        "-module-name",
        "Alamofire",
        "-Onone",
        "-g",
        "-swift-version",
        "5",
    ]);
    let (mut miss, mut extra) = (MismatchTally::new(), MismatchTally::new());
    let st = compare_argv(&a, &a, &mut miss, &mut extra);
    assert_eq!(st.oracle_items, st.exact, "every item should be byte-exact");
    assert_eq!(st.missing, 0);
    assert_eq!(st.extra, 0);
    assert_eq!(st.structural_pct(), 100);
    assert!(miss.is_empty() && extra.is_empty());
}

#[test]
fn detects_missing_and_extra() {
    let oracle = argv(&["-module-name", "Alamofire", "-Onone", "-enable-testing"]);
    // Drop -enable-testing (missing), add -Osize (extra), keep the rest.
    let ours = argv(&["-module-name", "Alamofire", "-Onone", "-Osize"]);
    let (mut miss, mut extra) = (MismatchTally::new(), MismatchTally::new());
    let st = compare_argv(&oracle, &ours, &mut miss, &mut extra);
    assert_eq!(st.missing, 1, "the dropped -enable-testing");
    assert_eq!(st.extra, 1, "the spurious -Osize");
    assert_eq!(miss.get("-enable-testing"), Some(&1));
    assert_eq!(extra.get("-Osize"), Some(&1));
}

#[test]
fn geometry_is_counted_not_scored() {
    // -o and -output-file-map are pure geometry; .hmap-bearing -Xcc too.
    let oracle = argv(&[
        "-module-name",
        "Alamofire",
        "-o",
        "/dd/out.o",
        "-output-file-map",
        "/dd/ofm.json",
        "-Xcc",
        "-I/dd/Build/Intermediates.noindex/x.hmap",
    ]);
    let ours = argv(&["-module-name", "Alamofire"]);
    let (mut miss, mut extra) = (MismatchTally::new(), MismatchTally::new());
    let st = compare_argv(&oracle, &ours, &mut miss, &mut extra);
    // Only -module-name is scored; the three geometry items are excluded so
    // omitting them is not a "missing" defect.
    assert_eq!(st.oracle_items, 1);
    assert_eq!(st.geometry_oracle, 3);
    assert_eq!(st.missing, 0);
    assert_eq!(st.structural_pct(), 100);
}

#[test]
fn structural_credits_divergent_abs_paths() {
    // Same search-path flag, paths anchored at different roots: the canonical
    // tier fails (different roots) but the structural tier credits "both abs".
    let oracle = argv(&[
        "-I",
        "/Users/ci/corpus/alamofire/.work/dd/Build/Products/Debug",
    ]);
    let ours = argv(&[
        "-I",
        "/Users/dev/Library/Developer/Xcode/DerivedData/Alamofire-aaaaaaaaaaaaaaaaaaaaaaaaaaaa/Build/Products/Debug",
    ]);
    let (mut miss, mut extra) = (MismatchTally::new(), MismatchTally::new());
    let st = compare_argv(&oracle, &ours, &mut miss, &mut extra);
    assert_eq!(st.structural, 1);
    assert_eq!(st.exact, 0);
    assert_eq!(st.missing, 0);
    assert_eq!(st.extra, 0);
}

#[test]
fn canonical_credits_home_drift() {
    // -sdk to the same SDK under two different users/Xcodes: canonicalizes equal.
    let oracle = argv(&[
        "-sdk",
        "/Applications/Xcode-26.5.0.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.5.sdk",
    ]);
    let ours = argv(&[
        "-sdk",
        "/Applications/Xcode-26.0.1.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.0.sdk",
    ]);
    let (mut miss, mut extra) = (MismatchTally::new(), MismatchTally::new());
    let st = compare_argv(&oracle, &ours, &mut miss, &mut extra);
    assert_eq!(st.canonical, 1, "same SDK modulo Xcode dir + SDK version");
    assert_eq!(st.exact, 0);
}

// ----- pilot oracle integrity ----------------------------------------------

/// Every captured oracle must self-score 100% structural with zero missing /
/// extra — a comparator + capture sanity check (an argv always matches itself),
/// and it exercises the reader on the real committed JSON.
#[test]
fn captured_oracles_self_score_100() {
    let oracles = find_compiler_args_oracles();
    assert!(
        !oracles.is_empty(),
        "no compiler-args oracle captured yet under fixtures/*/compiler-args/"
    );
    for path in &oracles {
        let oracle = argv::read_compiler_args(path)
            .unwrap_or_else(|| panic!("failed to read compiler-args oracle {}", path.display()));
        println!(
            "{}: {} target(s)",
            path.file_name().unwrap().to_string_lossy(),
            oracle.targets.len()
        );
        for t in &oracle.targets {
            let self_check = |label: &str, a: &[String]| {
                let (mut miss, mut extra) = (MismatchTally::new(), MismatchTally::new());
                let st = compare_argv(a, a, &mut miss, &mut extra);
                assert_eq!(
                    st.missing, 0,
                    "[{} {}] self-compare produced {} missing",
                    t.target, label, st.missing
                );
                assert_eq!(
                    st.extra, 0,
                    "[{} {}] self-compare produced extra",
                    t.target, label
                );
                assert_eq!(
                    st.exact, st.oracle_items,
                    "[{} {}] self-compare not byte-exact",
                    t.target, label
                );
            };
            if let Some(sw) = &t.swift {
                self_check("swift", &sw.arguments);
                self_check("swift.inputFiles", &sw.input_files);
                assert!(
                    !sw.input_files.is_empty(),
                    "[{}] swift invocation captured no input files",
                    t.target
                );
            }
            if let Some(cl) = &t.clang {
                self_check("clang.common", &cl.common_arguments);
            }
            if let Some(ln) = &t.link {
                self_check("link", &ln.arguments);
            }
        }
    }
}

/// Sanity: the pilot Alamofire macOS oracle has the shape later phases rely on —
/// a Swift module invocation carrying the semantic flags, 43 source inputs, and
/// a link step. Guards against a capture regression silently emptying it.
#[test]
fn alamofire_pilot_oracle_is_complete() {
    let path = common::fixtures_root()
        .join("alamofire/xcode-26.5.0/compiler-args/Alamofire-macOS__Debug__macOS.json");
    if !path.exists() {
        // Tolerate absence on a checkout that hasn't captured the pilot.
        eprintln!("pilot oracle absent ({}); skipping", path.display());
        return;
    }
    let oracle = argv::read_compiler_args(&path).expect("read pilot oracle");
    assert_eq!(oracle.sdk, "macosx");
    assert_eq!(oracle.arch, "arm64");
    let t = oracle
        .targets
        .iter()
        .find(|t| t.target == "Alamofire macOS")
        .expect("Alamofire macOS target");
    let sw = t.swift.as_ref().expect("swift invocation");
    assert!(sw.arguments.iter().any(|a| a == "-module-name"));
    assert!(sw.arguments.iter().any(|a| a == "Alamofire"));
    assert!(sw.arguments.iter().any(|a| a == "-Onone"));
    assert!(sw.arguments.iter().any(|a| a == "-swift-version"));
    assert_eq!(sw.input_files.len(), 43, "Alamofire's .swift source count");
    let ln = t.link.as_ref().expect("link invocation");
    assert_eq!(ln.tool.as_deref(), Some("clang"));
    assert!(ln.arguments.iter().any(|a| a == "-dynamiclib"));
}

// ----- Phase 2: source-file extraction -------------------------------------

/// `project::target_source_files` must yield exactly the target's `.swift`
/// inputs — the same set the oracle's `SwiftFileList` carries (compared
/// canonically, since our raw fixture and the captured checkout sit at
/// different roots).
#[test]
fn alamofire_source_files_match_oracle() {
    let oracle_path = fixtures_root()
        .join("alamofire/xcode-26.5.0/compiler-args/Alamofire-macOS__Debug__macOS.json");
    let xcodeproj = fixtures_root().join("alamofire/xcode-26.5.0/raw/Alamofire.xcodeproj");
    if !oracle_path.exists() || !xcodeproj.exists() {
        eprintln!("pilot oracle or raw fixture absent; skipping");
        return;
    }
    let oracle = argv::read_compiler_args(&oracle_path).expect("read oracle");
    let t = oracle
        .targets
        .iter()
        .find(|t| t.target == "Alamofire macOS")
        .expect("Alamofire macOS target");
    let sw = t.swift.as_ref().expect("swift invocation");

    let mut want: Vec<String> = sw
        .input_files
        .iter()
        .map(|p| canonicalize_value(p))
        .collect();
    want.sort();

    let resolved =
        project::target_source_files(&xcodeproj, "Alamofire macOS").expect("resolve sources");
    let mut got: Vec<String> = resolved
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|p| p.ends_with(".swift"))
        .map(|p| canonicalize_value(&p))
        .collect();
    got.sort();

    let missing: Vec<&String> = want.iter().filter(|w| !got.contains(w)).collect();
    let extra: Vec<&String> = got.iter().filter(|g| !want.contains(g)).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "source-file mismatch\n  missing ({}): {:?}\n  extra ({}): {:?}",
        missing.len(),
        missing,
        extra.len(),
        extra
    );
    assert_eq!(got.len(), 43, "Alamofire macOS .swift source count");
}

// ----- Phase 3: Swift generator vs oracle ----------------------------------

/// The `raw/` checkout sibling of a compiler-args oracle (…/xcode-<ver>/raw).
fn raw_root_for(oracle: &Path) -> PathBuf {
    oracle
        .parent() // compiler-args/
        .and_then(Path::parent) // xcode-<ver>/
        .map(|p| p.join("raw"))
        .unwrap_or_default()
}

/// Whether the named scheme instruments **all** built targets for coverage — a
/// scheme-level fact that forces `CLANG_COVERAGE_MAPPING=YES` (hence the build's
/// `-profile-generate` / `-profile-coverage-mapping`). It applies when
/// `TestAction.codeCoverageEnabled` is set AND coverage is not scoped to a
/// specific target list (`<CodeCoverageTargets>`); a scoped scheme (Alamofire's
/// empty list) leaves the framework uninstrumented. Targeting a target inside a
/// non-empty scope list is a tracked gap (no corpus case yet).
fn scheme_coverage(raw: &Path, scheme_name: &str) -> bool {
    let Some(path) = common::find_file_named(raw, &format!("{scheme_name}.xcscheme")) else {
        return false;
    };
    let enabled = scheme::parse_file(&path)
        .ok()
        .and_then(|s| s.test_action)
        .is_some_and(|ta| ta.code_coverage_enabled);
    let scoped = std::fs::read_to_string(&path).is_ok_and(|t| t.contains("<CodeCoverageTargets"));
    enabled && !scoped
}

/// Find the `.xcodeproj` under `raw` that declares `target`.
fn project_with_target(raw: &Path, target: &str) -> Option<PathBuf> {
    let mut projects = Vec::new();
    common::walk(raw, &mut projects, &|p, out| {
        if p.extension() == Some(std::ffi::OsStr::new("xcodeproj")) {
            out.push(p.to_path_buf());
        }
    });
    projects.into_iter().find(|proj| {
        BuildContext::open(proj)
            .is_ok_and(|ctx| ctx.project.targets.iter().any(|t| t.name == target))
    })
}

/// Per-version structural floors `(swift, clang, link)`, from the clean run minus
/// a small margin. The corpus spans pure-Swift frameworks (Alamofire, Kingfisher),
/// a real ObjC target (KingfisherTests' vendored Nocilla `.m`), and a Release app
/// (executable link + whole-module). Swift is near-exact; clang is language-gated
/// against the xcspec `FileTypes`/`Architectures`; link now scores at or near
/// 100% structural everywhere — the residuals are the visionOS scheme-coverage
/// capture gap (`-debug_variant` / `-fprofile-instr-generate`) and the
/// multi-arch Release capture's second `-target` triple. Judged by
/// structural % + the tally, never the geometry-capped exact %. Each Xcode runs
/// its own Swift driver and each platform its own SDK/flags, so every
/// (version, platform) cell is guarded at its own baseline.
#[allow(clippy::match_same_arms)] // distinct cells may share a baseline
fn version_floor(version: &str, sdk: &str) -> (u64, u64, u64) {
    match (version, sdk) {
        // (swift, clang, link), each = the clean run minus a ~2pt margin.
        ("26.5.0", "macosx") => (97, 92, 97),
        ("16.4.0", "macosx") => (97, 92, 98),
        ("15.4.0", "macosx") => (97, 91, 98),
        ("26.5.0", "iphoneos") => (97, 92, 98),
        ("26.5.0", "iphonesimulator") => (97, 91, 98),
        ("26.5.0", "appletvos") => (97, 92, 98),
        ("26.5.0", "watchos") => (97, 90, 98),
        ("26.5.0", "xros") => (92, 90, 90),
        // Other (version, platform) cells: calibrated once captured.
        _ => (90, 85, 55),
    }
}

/// Per-version precision floors `(swift, clang, link)` = the share of what we
/// emit (geometry excluded) that the oracle also has. Modeling the xcspec
/// `Condition` field keeps confident-wrong extras out (a sanitizer sub-setting
/// resolving `YES` no longer emits `-fsanitize=…` when the parent sanitizer is
/// off), so precision sits at 90–100%. These floors sit a few points below the
/// clean run and guard against a regression that reintroduces spurious flags.
#[allow(clippy::match_same_arms)] // distinct cells may share a baseline
fn precision_floor(version: &str, sdk: &str) -> (u64, u64, u64) {
    match (version, sdk) {
        ("26.5.0", "macosx") => (96, 96, 97),
        ("16.4.0", "macosx") => (95, 96, 98),
        ("15.4.0", "macosx") => (95, 95, 98),
        ("26.5.0", "iphoneos") => (97, 97, 97),
        ("26.5.0", "iphonesimulator") => (97, 95, 98),
        ("26.5.0", "appletvos") => (97, 97, 97),
        ("26.5.0", "watchos") => (97, 95, 97),
        ("26.5.0", "xros") => (97, 97, 97),
        _ => (88, 88, 78),
    }
}

/// Condition-gated flags whose sub-setting can resolve `YES` while the parent
/// gate is off — emitting them is the confident-wrong bug the `Condition`
/// modeling fixes. None may appear as a clang extra in any cell.
const NEVER_LEAK: &[&str] = &["-fsanitize=integer", "-fsanitize=nullability"];

/// One tool's accumulated score plus its split missing/extra tallies.
#[derive(Default)]
struct ToolScore {
    stats: ArgvStats,
    miss: MismatchTally,
    extra: MismatchTally,
    targets: u64,
}

impl ToolScore {
    fn record(
        &mut self,
        key: &str,
        slug: &str,
        target: &str,
        label: &str,
        oracle: &[String],
        ours: &[String],
    ) {
        let st = compare_argv(oracle, ours, &mut self.miss, &mut self.extra);
        println!(
            "  {key:<17} {slug:<11} {target:<22} {label:<5} struct={}% precision={}% (oracle={} ours={} missing={} extra={} geom_o={})",
            st.structural_pct(),
            st.precision_pct(),
            st.oracle_items,
            st.our_items,
            st.missing,
            st.extra,
            st.geometry_oracle,
        );
        self.stats.merge(st);
        self.targets += 1;
    }
}

/// Per-Xcode-version tool scores — the same resolver/generator is run for every
/// version, but each is scored and floored on its own (different Swift drivers).
#[derive(Default)]
struct VersionScores {
    swift: ToolScore,
    clang: ToolScore,
    link: ToolScore,
}

/// Resolve each captured target, generate its `swiftc`/`clang`/`ld` argv, and
/// score each against the oracle. The systematic missing/extra tally per tool is
/// the deliverable; the per-tool floors guard against a generation regression.
#[test]
fn compiler_args_oracle_coverage() {
    let oracles = find_compiler_args_oracles();
    assert!(!oracles.is_empty(), "no compiler-args oracles captured");
    let mut catalogs = CatalogCache::new();
    // Keyed by (Xcode version, SDK/platform): each cell is scored and floored on
    // its own, so e.g. an iOS oracle never dilutes the macOS aggregate.
    let mut by_key: BTreeMap<(String, String), VersionScores> = BTreeMap::new();

    for path in &oracles {
        let Some(version) = capture_xcode_version(path) else {
            continue;
        };
        let Some(oracle) = argv::read_compiler_args(path) else {
            continue;
        };
        let raw = raw_root_for(path);
        let catalog = catalogs.get(&version).clone();
        let swift_opts = catalog
            .compiler_options
            .get("com.apple.xcode.tools.swift.compiler")
            .map_or(&[][..], Vec::as_slice);
        let clang_opts = catalog
            .compiler_options
            .get("com.apple.compilers.llvm.clang.1_0")
            .map_or(&[][..], Vec::as_slice);

        for t in &oracle.targets {
            let Some(xcodeproj) = project_with_target(&raw, &t.target) else {
                eprintln!(
                    "no raw project for target {} under {}",
                    t.target,
                    raw.display()
                );
                continue;
            };
            let Ok(ctx) = BuildContext::open(&xcodeproj) else {
                continue;
            };
            let ctx = ctx.with_xcspec(catalog.clone());
            let mut query =
                ResolveQuery::new(&t.target, &oracle.configuration, &oracle.sdk, &oracle.arch);
            if scheme_coverage(&raw, &oracle.scheme) {
                query = query.with_code_coverage_enabled(true);
            }
            let Ok(resolved) = ctx.resolve(&query) else {
                eprintln!("resolve failed for {}", t.target);
                continue;
            };
            let settings = &resolved.settings;
            let dump = std::env::var("ARGV_DUMP").is_ok();
            let key = format!("{version} {}", oracle.sdk);
            let scores = by_key
                .entry((version.clone(), oracle.sdk.clone()))
                .or_default();

            if let Some(sw) = &t.swift {
                let has_pkg =
                    project::target_has_package_products(&xcodeproj, &t.target).unwrap_or(false);
                let ours = compiler_args::swift_arguments(
                    settings,
                    &oracle.arch,
                    swift_opts,
                    &version,
                    has_pkg,
                    &[],
                );
                if dump {
                    eprintln!(
                        "--- ORACLE swift {key} {} ---\n{}",
                        t.target,
                        sw.arguments.join("\n")
                    );
                    eprintln!("--- OURS swift {key} {} ---\n{}", t.target, ours.join("\n"));
                }
                scores
                    .swift
                    .record(&key, &oracle.slug, &t.target, "swift", &sw.arguments, &ours);
            }
            if let Some(cl) = &t.clang {
                let files: Vec<String> = cl.files.iter().map(|f| f.file.clone()).collect();
                let langs = compiler_args::clang_languages(&files);
                let ours =
                    compiler_args::clang_arguments(settings, &oracle.arch, clang_opts, &langs);
                if dump {
                    eprintln!(
                        "--- ORACLE clang {key} {} ---\n{}",
                        t.target,
                        cl.common_arguments.join("\n")
                    );
                    eprintln!("--- OURS clang {key} {} ---\n{}", t.target, ours.join("\n"));
                }
                scores.clang.record(
                    &key,
                    &oracle.slug,
                    &t.target,
                    "clang",
                    &cl.common_arguments,
                    &ours,
                );
            }
            if let Some(ln) = &t.link {
                let ours = if ln.tool.as_deref() == Some("libtool") {
                    compiler_args::static_lib_arguments(settings, &oracle.arch)
                } else {
                    let fws = project::target_linked_frameworks(&xcodeproj, &t.target)
                        .unwrap_or_default();
                    // The version-stamp compile (`<Product>_vers.c`,
                    // VERSIONING_SYSTEM=apple-generic) doesn't count as a
                    // C-family participant: pure-Swift targets carry it yet
                    // their links stay ARC-free.
                    let has_clang_sources = t
                        .clang
                        .as_ref()
                        .is_some_and(|c| c.files.iter().any(|f| !f.file.ends_with("_vers.c")));
                    let libs =
                        project::target_linked_libraries(&xcodeproj, &t.target).unwrap_or_default();
                    compiler_args::link_arguments(
                        settings,
                        &oracle.arch,
                        &fws,
                        &libs,
                        has_clang_sources,
                    )
                };
                if dump {
                    eprintln!(
                        "--- ORACLE link {key} {} ---\n{}",
                        t.target,
                        ln.arguments.join("\n")
                    );
                    eprintln!("--- OURS link {key} {} ---\n{}", t.target, ours.join("\n"));
                }
                scores
                    .link
                    .record(&key, &oracle.slug, &t.target, "link", &ln.arguments, &ours);
            }
        }
    }

    for ((version, sdk), scores) in &by_key {
        print_argv_summary(
            &format!("[{version} {sdk}] swift"),
            &scores.swift.stats,
            &scores.swift.miss,
            &scores.swift.extra,
        );
        print_argv_summary(
            &format!("[{version} {sdk}] clang"),
            &scores.clang.stats,
            &scores.clang.miss,
            &scores.clang.extra,
        );
        print_argv_summary(
            &format!("[{version} {sdk}] link"),
            &scores.link.stats,
            &scores.link.miss,
            &scores.link.extra,
        );
    }
    assert!(
        by_key.values().any(|s| s.swift.targets > 0),
        "no swift targets scored"
    );

    if std::env::var("ARGV_DIAGNOSTIC").is_ok() {
        return;
    }
    // Condition-gated flags must never leak: their sub-setting can resolve `YES`
    // while the parent gate is off, so emitting them unconditionally is the exact
    // confident-wrong bug the xcspec `Condition` modeling fixes. Guard the whole
    // matrix against a regression that drops the gate.
    for ((version, sdk), scores) in &by_key {
        for flag in NEVER_LEAK {
            let n = scores.clang.extra.get(*flag).copied().unwrap_or(0);
            assert_eq!(
                n, 0,
                "[{version} {sdk}] clang leaked condition-gated {flag} ×{n}"
            );
        }
    }
    for ((version, sdk), scores) in &by_key {
        let (swift_floor, clang_floor, link_floor) = version_floor(version, sdk);
        let (swift_prec, clang_prec, link_prec) = precision_floor(version, sdk);
        for (label, score, floor, pfloor) in [
            ("swift", &scores.swift, swift_floor, swift_prec),
            ("clang", &scores.clang, clang_floor, clang_prec),
            ("link", &scores.link, link_floor, link_prec),
        ] {
            if score.targets == 0 {
                continue;
            }
            let pct = score.stats.structural_pct();
            assert!(
                pct >= floor,
                "[{version} {sdk}] {label} structural {pct}% < floor {floor}% (tally above)"
            );
            let prec = score.stats.precision_pct();
            assert!(
                prec >= pfloor,
                "[{version} {sdk}] {label} precision {prec}% < floor {pfloor}% (tally above)"
            );
        }
    }
}

// Keep ArgvStats referenced for the dead-code lint when only unit tests build.
const _: fn() -> ArgvStats = ArgvStats::default;
