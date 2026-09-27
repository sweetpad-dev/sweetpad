//! Layer 0 of the BSP measurement loop (see `DOCS.md` §8 (BSP server)): does the editor's
//! front end accept our generated arguments and **resolve every imported
//! module / header**? This is the search-path/module-input surface the
//! compiler-args oracle excludes as geometry — yet it's exactly what makes
//! completion and navigation work.
//!
//! Method: build a synthetic fixture hermetically (into a throwaway
//! `-derivedDataPath`), then run the front end (`swiftc -typecheck` /
//! `clang -fsyntax-only`) on each target's sources with the args we generate
//! (pointed at that same DerivedData). The headline metric is the count of
//! **resolution errors** (`no such module`, `'foo.h' file not found`, …) → it
//! must be zero, covering the Swift cross-module import (multi-module fixture),
//! the ObjC header search path (objc-headers fixture), and the imports that only
//! Xcode's header maps and generated-sources dirs resolve — a sibling
//! directory's header, a mixed target's `-Swift.h`, another target's public
//! framework header (headermaps fixture, GitHub #238).
//!
//! A second test closes the loop from the other side: every search path Xcode
//! itself passes and that exists on disk must have a counterpart in ours, so a
//! path class we've never thought about shows up as a failure rather than as
//! completion that quietly doesn't work.
//!
//! Opt-in: builds with `xcodebuild`, so it only runs when `BSP_ORACLE=1`. It
//! builds with the Xcode `BSP_ORACLE_XCODE` names, else the selected one
//! ([`oracle_xcode`]), and gives the build and the front ends a `TMPDIR` of
//! the test's own.

mod oracle_xcode;

use std::path::{Path, PathBuf};

use oracle_xcode::OracleXcode;
use sweetpad_core::build_settings::{self, BuildSettingsOptions};
use sweetpad_core::scratch::ScratchDir;
use sweetpad_lib::compiler_args::TargetCompilerArguments;

/// The Xcode the oracle builds with and the `TMPDIR` its tools run in.
struct Oracle {
    xcode: OracleXcode,
    tmp: ScratchDir,
}

/// The oracle, when `BSP_ORACLE` is set and there is an Xcode to build with;
/// prints why when there isn't.
fn gated(what: &str) -> Option<Oracle> {
    if std::env::var("BSP_ORACLE").is_err() {
        eprintln!("skipping: set BSP_ORACLE=1 to run the {what}");
        return None;
    }
    Some(Oracle {
        xcode: oracle_xcode::find()?,
        tmp: ScratchDir::new("sweetpad-bsp-typecheck-tmp").unwrap(),
    })
}

fn fixture(name: &str, proj: &str) -> PathBuf {
    PathBuf::from(env!("SWEETPAD_LIB_DIR")).join(format!("fixtures/{name}/project/{proj}"))
}

/// Flags carrying a value in the next token — the module-resolution surface
/// (search paths, sysroot/target, importer flags, module name, language mode).
/// Superset for both front ends; `-sdk` is swift, `-isysroot` is clang.
const PAIR_FLAGS: &[&str] = &[
    "-sdk",
    "-isysroot",
    "-target",
    "-x",
    "-module-name",
    "-swift-version",
    "-I",
    "-F",
    "-Xcc",
    "-import-objc-header",
    "-isystem",
    "-iquote",
    "-iframework",
    "-fmodule-map-file",
    "-include",
    "-resource-dir",
    "-enable-experimental-feature",
    "-enable-upcoming-feature",
];

/// Reduce a build invocation to a syntax-only one: keep the flags that affect
/// resolution / parsing, drop build actions and explicit-module plumbing a
/// standalone front-end run can't satisfy.
fn syntax_args(build_args: &[String], action: &str) -> Vec<String> {
    let mut out = vec![action.to_string()];
    let mut i = 0;
    while i < build_args.len() {
        let a = &build_args[i];
        if PAIR_FLAGS.contains(&a.as_str()) {
            out.push(a.clone());
            if i + 1 < build_args.len() {
                out.push(build_args[i + 1].clone());
            }
            i += 2;
        } else if a.starts_with("-D")
            || a.starts_with("-I")
            || a.starts_with("-F")
            || a.starts_with("-isystem")
            || a.starts_with("-std")
        {
            out.push(a.clone());
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Diagnostic lines that mean a module/header couldn't be resolved.
fn resolution_errors(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter(|l| {
            let l = l.to_lowercase();
            l.contains("no such module")
                || l.contains("could not build module")
                || l.contains("missing required module")
                || l.contains("cannot load module")
                || l.contains("unable to load standard library")
                || l.contains("file not found")
        })
        .map(ToString::to_string)
        .collect()
}

fn resolve_target(
    oracle: &Oracle,
    project: &Path,
    target: &str,
    dd: &Path,
) -> TargetCompilerArguments {
    let opts = BuildSettingsOptions {
        project: Some(project.to_path_buf()),
        workspace: None,
        scheme: None,
        target: Some(target.to_string()),
        configuration: "Debug".into(),
        sdk: "macosx".into(),
        arch: "arm64".into(),
        destination: None,
        xcconfig: None,
        xcode: Some(oracle.xcode.path.clone()),
        xcspec_root: None,
        sdksettings_root: None,
        catalog_cache: None,
        derived_data_path: Some(dd.to_path_buf()),
        overrides: Vec::new(),
        // The fixture pins its own DerivedData; reading the runner's Xcode
        // configuration would make the expectations machine-dependent.
        read_xcode_locations: false,
        keys: None,
    };
    let mut all = build_settings::resolve_compiler_arguments(&opts)
        .unwrap_or_else(|e| panic!("resolve {target}: {e}"));
    all.retain(|t| t.target == target);
    all.pop()
        .unwrap_or_else(|| panic!("no args for target {target}"))
}

/// Build a fixture hermetically into `dd`, returning xcodebuild's transcript
/// (which carries the `CompileC` lines the search-path check reads back).
fn build_fixture(oracle: &Oracle, project: &Path, scheme: &str, dd: &Path) -> String {
    let build = oracle
        .xcode
        .command("xcodebuild", &oracle.tmp)
        .args(["build", "-project"])
        .arg(project)
        .args([
            "-scheme",
            scheme,
            "-configuration",
            "Debug",
            "-destination",
            "platform=macOS",
            "-derivedDataPath",
        ])
        .arg(dd)
        .arg("CODE_SIGNING_ALLOWED=NO")
        .output()
        .expect("run xcodebuild");
    assert!(
        build.status.success(),
        "fixture build failed ({scheme}):\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    String::from_utf8_lossy(&build.stdout).into_owned()
}

/// Run a front end on a target's sources, returning resolution errors.
fn check_target(
    oracle: &Oracle,
    project: &Path,
    target: &str,
    dd: &Path,
    swift: bool,
) -> Vec<String> {
    let inv = resolve_target(oracle, project, target, dd);
    let (tool, action, build_args, files) = if swift {
        let s = inv
            .swift
            .unwrap_or_else(|| panic!("{target} has no swift invocation"));
        (
            oracle.xcode.tool("swiftc"),
            "-typecheck",
            s.arguments,
            s.input_files,
        )
    } else {
        let c = inv
            .clang
            .unwrap_or_else(|| panic!("{target} has no clang invocation"));
        (
            oracle.xcode.tool("clang"),
            "-fsyntax-only",
            c.arguments,
            c.input_files,
        )
    };
    let mut args = syntax_args(&build_args, action);
    args.extend(files);
    let out = oracle
        .xcode
        .command(&tool, &oracle.tmp)
        .args(&args)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", tool.display()));
    let errs = resolution_errors(&String::from_utf8_lossy(&out.stderr));
    eprintln!(
        "[{target}] {} exit={} resolution-errors={}",
        if swift { "swift" } else { "clang" },
        out.status.code().unwrap_or(-1),
        errs.len()
    );
    for e in &errs {
        eprintln!("    {e}");
    }
    errs
}

#[test]
fn bsp_typecheck_oracle() {
    let Some(oracle) = gated("BSP type-check oracle") else {
        return;
    };

    let mut errors = Vec::new();

    // Swift cross-module: ModuleB imports ModuleA.
    let multimodule = fixture("_synthetic-multimodule", "MultiModule.xcodeproj");
    let dd1 = ScratchDir::new("sweetpad-bsp-mm").unwrap();
    let _ = build_fixture(&oracle, &multimodule, "ModuleB", &dd1);
    errors.extend(check_target(&oracle, &multimodule, "ModuleA", &dd1, true));
    errors.extend(check_target(&oracle, &multimodule, "ModuleB", &dd1, true));
    drop(dd1);

    // ObjC header search path: widget.m #imports include/widget.h via HEADER_SEARCH_PATHS.
    let objc = fixture("_synthetic-objc-headers", "ObjCHeaders.xcodeproj");
    let dd2 = ScratchDir::new("sweetpad-bsp-objc").unwrap();
    let _ = build_fixture(&oracle, &objc, "ObjCHeaders", &dd2);
    errors.extend(check_target(&oracle, &objc, "ObjCHeaders", &dd2, false));
    drop(dd2);

    // Swift Package product: SpmApp imports `Dep` from a local package, whose
    // module Xcode builds into the products dir / PackageFrameworks.
    let spm = fixture("_synthetic-spm", "SpmApp.xcodeproj");
    let dd3 = ScratchDir::new("sweetpad-bsp-spm").unwrap();
    let _ = build_fixture(&oracle, &spm, "SpmApp", &dd3);
    errors.extend(check_target(&oracle, &spm, "SpmApp", &dd3, true));
    drop(dd3);

    // Header maps + generated sources: none of Widget.m's imports is reachable
    // through HEADER_SEARCH_PATHS, which the fixture doesn't set at all.
    let hmaps = fixture("_synthetic-headermaps", "HeaderMaps.xcodeproj");
    let dd4 = ScratchDir::new("sweetpad-bsp-hmap").unwrap();
    let _ = build_fixture(&oracle, &hmaps, "HeaderMaps", &dd4);
    errors.extend(check_target(&oracle, &hmaps, "HeaderMapsCore", &dd4, false));
    errors.extend(check_target(&oracle, &hmaps, "HeaderMaps", &dd4, false));
    drop(dd4);

    assert!(
        errors.is_empty(),
        "module/header resolution failures: {errors:?}"
    );
}

/// Split a build-log command line into tokens, expanding the `@file` response
/// arguments Xcode 26 puts most of a `CompileC` invocation behind. Quoting is
/// naive (fixture paths carry no spaces); only the search-path flags are read
/// back, and those are never quoted.
fn command_tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in line.split_whitespace() {
        let token = raw.trim_matches('\'');
        match token.strip_prefix('@') {
            Some(resp) => match std::fs::read_to_string(resp) {
                Ok(body) => out.extend(
                    body.split_whitespace()
                        .map(|t| t.trim_matches('\'').to_string()),
                ),
                Err(e) => panic!("read response file {resp}: {e}"),
            },
            None => out.push(token.to_string()),
        }
    }
    out
}

/// The clang command line xcodebuild logged for the first `CompileC` in
/// `target`. A `CompileC` header line names the target it belongs to; the
/// invocation is the `…/clang` line inside the block that follows.
fn xcode_clang_line(log: &str, target: &str) -> Vec<String> {
    let mut in_block = false;
    for line in log.lines() {
        if line.starts_with("CompileC ") {
            in_block = line.contains(&format!("(in target '{target}' from project"));
        } else if in_block && line.trim_start().starts_with('/') && line.contains("/clang ") {
            return command_tokens(line);
        }
    }
    panic!("no CompileC invocation for target {target} in the build log");
}

/// The `-I` / `-iquote` values in an argv, in either spelling (`-I<path>` joined,
/// as Xcode writes it, or `-I <path>` separated, as we do).
fn search_paths(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        for flag in ["-I", "-iquote"] {
            if a == flag {
                if let Some(v) = args.get(i + 1) {
                    out.push(v.clone());
                }
            } else if let Some(v) = a.strip_prefix(flag)
                && !v.is_empty()
            {
                out.push(v.to_string());
            }
        }
        i += 1;
    }
    out
}

/// Every header search path Xcode passes **and that exists** must have a
/// counterpart in ours.
///
/// The existence filter is the whole point of the comparison: Xcode names its
/// generated-sources dirs whether or not a build created them, and we only name
/// what's there, so an unqualified subset check would fail on paths that hold
/// nothing. What it does catch is the #238 shape — a class of search path Xcode
/// relies on and we've never emitted — which the arg-oracle comparator can't,
/// since it scores header maps and `Intermediates.noindex` paths as geometry.
#[test]
fn bsp_clang_search_paths_cover_xcodes() {
    let Some(oracle) = gated("BSP search-path coverage check") else {
        return;
    };

    let project = fixture("_synthetic-headermaps", "HeaderMaps.xcodeproj");
    let dd = ScratchDir::new("sweetpad-bsp-cover").unwrap();
    let log = build_fixture(&oracle, &project, "HeaderMaps", &dd);

    let mut missing = Vec::new();
    for target in ["HeaderMapsCore", "HeaderMaps"] {
        let ours = search_paths(
            &resolve_target(&oracle, &project, target, &dd)
                .clang
                .unwrap_or_else(|| panic!("{target} has no clang invocation"))
                .arguments,
        );
        let theirs = search_paths(&xcode_clang_line(&log, target));
        let live: Vec<&String> = theirs
            .iter()
            .filter(|p| Path::new(p).exists())
            .filter(|p| !ours.contains(p))
            .collect();
        eprintln!(
            "[{target}] xcode search paths={} (live) ours={} missing={}",
            theirs.iter().filter(|p| Path::new(p).exists()).count(),
            ours.len(),
            live.len()
        );
        for p in &live {
            eprintln!("    missing {p}");
            missing.push(format!("{target}: {p}"));
        }
    }
    drop(dd);

    assert!(
        missing.is_empty(),
        "search paths xcode passes and we don't: {missing:?}"
    );
}
