//! Layer 0 of the BSP measurement loop (see `PLAN_BSP.md`): does the editor's
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
//! must be zero, covering both the Swift cross-module import (multi-module
//! fixture) and the ObjC header search path (objc-headers fixture).
//!
//! Opt-in: builds with `xcodebuild`, so it only runs when `BSP_ORACLE=1` (and
//! Xcode 26.5 is installed). ⚠️ Pinned to Xcode 26.5 — expand later (PLAN_BSP.md).

use std::path::{Path, PathBuf};
use std::process::Command;

use sweetpad::build_settings::{self, BuildSettingsOptions};
use sweetpad::compiler_args::TargetCompilerArguments;

// ⚠️ Xcode 26.5 only for now (PLAN_BSP.md "expand later").
const XCODE: &str = "/Applications/Xcode-26.5.0.app";

fn developer_dir() -> String {
    format!("{XCODE}/Contents/Developer")
}

fn toolchain_bin(tool: &str) -> String {
    format!("{}/Toolchains/XcodeDefault.xctoolchain/usr/bin/{tool}", developer_dir())
}

fn fixture(name: &str, proj: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/{name}/project/{proj}"))
}

/// Flags carrying a value in the next token — the module-resolution surface
/// (search paths, sysroot/target, importer flags, module name, language mode).
/// Superset for both front ends; `-sdk` is swift, `-isysroot` is clang.
const PAIR_FLAGS: &[&str] = &[
    "-sdk", "-isysroot", "-target", "-x", "-module-name", "-swift-version", "-I", "-F", "-Xcc",
    "-import-objc-header", "-isystem", "-iquote", "-iframework", "-fmodule-map-file", "-include",
    "-resource-dir", "-enable-experimental-feature", "-enable-upcoming-feature",
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

fn resolve_target(project: &Path, target: &str, dd: &Path) -> TargetCompilerArguments {
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
        xcode: Some(PathBuf::from(XCODE)),
        xcspec_root: None,
        sdksettings_root: None,
        catalog_cache: None,
        derived_data_path: Some(dd.to_path_buf()),
        keys: None,
    };
    let mut all = build_settings::resolve_compiler_arguments(&opts)
        .unwrap_or_else(|e| panic!("resolve {target}: {e}"));
    all.retain(|t| t.target == target);
    all.pop().unwrap_or_else(|| panic!("no args for target {target}"))
}

fn build_fixture(project: &Path, scheme: &str, dd: &Path) {
    let build = Command::new("xcodebuild")
        .env("DEVELOPER_DIR", developer_dir())
        .args(["build", "-project"])
        .arg(project)
        .args(["-scheme", scheme, "-configuration", "Debug", "-destination", "platform=macOS", "-derivedDataPath"])
        .arg(dd)
        .arg("CODE_SIGNING_ALLOWED=NO")
        .output()
        .expect("run xcodebuild");
    assert!(
        build.status.success(),
        "fixture build failed ({scheme}):\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
}

/// Run a front end on a target's sources, returning resolution errors.
fn check_target(project: &Path, target: &str, dd: &Path, swift: bool) -> Vec<String> {
    let inv = resolve_target(project, target, dd);
    let (tool, action, build_args, files) = if swift {
        let s = inv.swift.unwrap_or_else(|| panic!("{target} has no swift invocation"));
        (toolchain_bin("swiftc"), "-typecheck", s.arguments, s.input_files)
    } else {
        let c = inv.clang.unwrap_or_else(|| panic!("{target} has no clang invocation"));
        (toolchain_bin("clang"), "-fsyntax-only", c.arguments, c.input_files)
    };
    let mut args = syntax_args(&build_args, action);
    args.extend(files);
    let out = Command::new(&tool)
        .env("DEVELOPER_DIR", developer_dir())
        .args(&args)
        .output()
        .unwrap_or_else(|e| panic!("run {tool}: {e}"));
    let errs = resolution_errors(&String::from_utf8_lossy(&out.stderr));
    eprintln!("[{target}] {} exit={} resolution-errors={}", if swift { "swift" } else { "clang" }, out.status.code().unwrap_or(-1), errs.len());
    for e in &errs {
        eprintln!("    {e}");
    }
    errs
}

#[test]
fn bsp_typecheck_oracle() {
    if std::env::var("BSP_ORACLE").is_err() {
        eprintln!("skipping: set BSP_ORACLE=1 to run the BSP type-check oracle");
        return;
    }
    if !Path::new(XCODE).exists() {
        eprintln!("skipping: {XCODE} not installed");
        return;
    }

    let mut errors = Vec::new();

    // Swift cross-module: ModuleB imports ModuleA.
    let multimodule = fixture("_synthetic-multimodule", "MultiModule.xcodeproj");
    let dd1 = std::env::temp_dir().join(format!("sweetpad-bsp-mm-{}", std::process::id()));
    build_fixture(&multimodule, "ModuleB", &dd1);
    errors.extend(check_target(&multimodule, "ModuleA", &dd1, true));
    errors.extend(check_target(&multimodule, "ModuleB", &dd1, true));
    let _ = std::fs::remove_dir_all(&dd1);

    // ObjC header search path: widget.m #imports include/widget.h via HEADER_SEARCH_PATHS.
    let objc = fixture("_synthetic-objc-headers", "ObjCHeaders.xcodeproj");
    let dd2 = std::env::temp_dir().join(format!("sweetpad-bsp-objc-{}", std::process::id()));
    build_fixture(&objc, "ObjCHeaders", &dd2);
    errors.extend(check_target(&objc, "ObjCHeaders", &dd2, false));
    let _ = std::fs::remove_dir_all(&dd2);

    assert!(errors.is_empty(), "module/header resolution failures: {errors:?}");
}
