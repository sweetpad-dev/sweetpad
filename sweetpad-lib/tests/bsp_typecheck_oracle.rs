//! Layer 0 of the BSP measurement loop (see `PLAN_BSP.md`): does the editor's
//! front end accept our generated arguments and **resolve every imported
//! module**? This is the search-path/module-input surface the compiler-args
//! oracle deliberately excludes as geometry — yet it's exactly what makes
//! completion and navigation work.
//!
//! Method: build the synthetic multi-module fixture hermetically (into a
//! throwaway `-derivedDataPath`), then `swiftc -typecheck` each target's sources
//! with the args we generate (pointed at that same DerivedData). The headline
//! metric is the count of **module-resolution errors** (`no such module`, …) →
//! it must be zero, including `ModuleB`'s cross-module `import ModuleA`.
//!
//! Opt-in: this builds with `xcodebuild`, so it only runs when `BSP_ORACLE=1`
//! (and Xcode 26.5 is installed). ⚠️ Pinned to Xcode 26.5 for now — expand to
//! 15.4 / 16.4 later (see `PLAN_BSP.md`).

use std::path::{Path, PathBuf};
use std::process::Command;

use sweetpad::build_settings::{self, BuildSettingsOptions};
use sweetpad::compiler_args::TargetCompilerArguments;

// ⚠️ Xcode 26.5 only for now (PLAN_BSP.md "expand later").
const XCODE: &str = "/Applications/Xcode-26.5.0.app";

fn developer_dir() -> String {
    format!("{XCODE}/Contents/Developer")
}

fn swiftc() -> String {
    format!("{}/Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc", developer_dir())
}

fn fixture_project() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/_synthetic-multimodule/project/MultiModule.xcodeproj")
}

/// Flags that carry a value in the next token; we keep both. The set is the
/// module-resolution surface — search paths, sysroot/target, importer flags,
/// module name, language mode, features.
const PAIR_FLAGS: &[&str] = &[
    "-sdk",
    "-target",
    "-module-name",
    "-swift-version",
    "-I",
    "-F",
    "-Xcc",
    "-import-objc-header",
    "-isystem",
    "-iframework",
    "-fmodule-map-file",
    "-enable-experimental-feature",
    "-enable-upcoming-feature",
    "-resource-dir",
];

/// Reduce a build invocation to a `-typecheck` one: keep only the flags that
/// affect module resolution / parsing, drop the build actions and
/// explicit-module plumbing a standalone front-end run can't satisfy.
fn typecheck_args(build_args: &[String]) -> Vec<String> {
    let mut out = vec!["-typecheck".to_string()];
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
        {
            out.push(a.clone());
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Diagnostic lines that mean a module/header couldn't be resolved — the defect
/// Layer 0 hunts. (A plain type error would be a fixture bug, tallied separately.)
fn module_errors(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter(|l| {
            let l = l.to_lowercase();
            l.contains("no such module")
                || l.contains("could not build module")
                || l.contains("missing required module")
                || l.contains("cannot load module")
                || l.contains("unable to load standard library")
                || l.contains("'.h' file not found")
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

    let project = fixture_project();
    let dd = std::env::temp_dir().join(format!("sweetpad-bsp-{}", std::process::id()));

    // Build hermetically so ModuleA.swiftmodule exists where ModuleB's args point.
    let build = Command::new("xcodebuild")
        .env("DEVELOPER_DIR", developer_dir())
        .args(["build", "-project"])
        .arg(&project)
        .args([
            "-scheme", "ModuleB", "-configuration", "Debug", "-destination", "platform=macOS",
            "-derivedDataPath",
        ])
        .arg(&dd)
        .arg("CODE_SIGNING_ALLOWED=NO")
        .output()
        .expect("run xcodebuild");
    assert!(
        build.status.success(),
        "fixture build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    // Each target: generate args, reduce to a typecheck run, count module errors.
    let mut total_module_errors = 0usize;
    let mut module_b_errors = Vec::new();
    for (target, sources) in [("ModuleA", &["ModuleA/a.swift"][..]), ("ModuleB", &["ModuleB/b.swift"][..])] {
        let inv = resolve_target(&project, target, &dd);
        let swift = inv.swift.unwrap_or_else(|| panic!("{target} has no swift invocation"));
        let mut args = typecheck_args(&swift.arguments);
        args.extend(swift.input_files.clone());

        let out = Command::new(swiftc())
            .env("DEVELOPER_DIR", developer_dir())
            .args(&args)
            .output()
            .expect("run swiftc -typecheck");
        let stderr = String::from_utf8_lossy(&out.stderr);
        let errs = module_errors(&stderr);
        eprintln!(
            "[{target}] typecheck exit={} module-errors={} (sources: {sources:?})",
            out.status.code().unwrap_or(-1),
            errs.len()
        );
        for e in &errs {
            eprintln!("    {e}");
        }
        total_module_errors += errs.len();
        if target == "ModuleB" {
            module_b_errors = errs;
        }
    }

    let _ = std::fs::remove_dir_all(&dd);

    // The headline invariant: the cross-module import resolves cleanly.
    assert!(
        module_b_errors.is_empty(),
        "ModuleB failed to resolve its cross-module import: {module_b_errors:?}"
    );
    assert_eq!(total_module_errors, 0, "module-resolution errors across the fixture");
}
