use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use sweetpad::project::{
    Target, is_self_buildable, open, target_dependencies, target_has_package_products,
    target_source_files, transitive_dependencies,
};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn xcodeproj_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        if matches!(
            p.file_name().and_then(OsStr::to_str),
            Some(".derived" | ".cache" | "DerivedData")
        ) {
            continue;
        }
        if p.extension() == Some(OsStr::new("xcodeproj")) {
            out.push(p);
        } else {
            walk(&p, out);
        }
    }
}

#[test]
fn opens_every_xcodeproj_in_corpus() {
    let projects = xcodeproj_dirs(&fixtures_root());
    assert!(
        !projects.is_empty(),
        "expected at least one .xcodeproj in fixtures/"
    );
    let mut failures: Vec<String> = Vec::new();
    let mut empty_target_count = 0;
    for path in &projects {
        match open(path) {
            Ok(project) => {
                if project.name.is_empty() {
                    failures.push(format!("{}: project name is empty", path.display()));
                }
                if project.targets.is_empty() {
                    empty_target_count += 1;
                }
            }
            Err(e) => {
                failures.push(format!("{}: {e}", path.display()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} projects failed to open:\n{}",
        failures.len(),
        projects.len(),
        failures.join("\n")
    );
    // Tuist fixtures and some package checkouts are legitimately target-less
    // (aggregate-only or empty). Just ensure the bulk have targets.
    assert!(
        empty_target_count * 2 < projects.len(),
        "more than half of projects ({empty_target_count}/{}) have no targets — likely a walking bug",
        projects.len()
    );
}

#[test]
fn alamofire_targets_include_main_library() {
    let path = fixtures_root().join("alamofire/xcode-26.5.0/raw/Alamofire.xcodeproj");
    let project = open(&path).unwrap();
    assert_eq!(project.name, "Alamofire");
    let target_names: Vec<&str> = project
        .targets
        .iter()
        .map(|t: &Target| t.name.as_str())
        .collect();
    assert!(
        target_names.iter().any(|n| n.starts_with("Alamofire")),
        "expected a target starting with 'Alamofire' in {target_names:?}"
    );
}

#[test]
fn netnewswire_configs_present() {
    let path = fixtures_root().join("netnewswire/xcode-26.5.0/raw/NetNewsWire.xcodeproj");
    let project = open(&path).unwrap();
    assert!(
        !project.configurations.is_empty(),
        "expected NetNewsWire to have configurations"
    );
    // NetNewsWire's targets should reference these configs too.
    for t in &project.targets {
        if t.configurations.is_empty() {
            // Some PBXAggregateTargets legitimately have an empty config list.
            continue;
        }
        // The set of names should be a subset of (or equal to) the project's.
        let project_set: std::collections::BTreeSet<&str> =
            project.configurations.iter().map(String::as_str).collect();
        for c in &t.configurations {
            assert!(
                project_set.contains(c.as_str()),
                "target {} config {c} not in project's set {project_set:?}",
                t.name
            );
        }
    }
}

#[test]
fn icecubes_shared_schemes_discovered() {
    // Scheme discovery from `xcshareddata/xcschemes/` (was covered via the CLI
    // `list` command).
    let path = fixtures_root().join("ice-cubes/xcode-26.5.0/raw/IceCubesApp.xcodeproj");
    let project = open(&path).unwrap();
    for expected in [
        "IceCubesActionExtension",
        "IceCubesApp",
        "IceCubesAppWidgetsExtensionExtension",
        "IceCubesNotifications",
        "IceCubesShareExtension",
    ] {
        assert!(
            project.schemes.iter().any(|s| s == expected),
            "expected scheme '{expected}' in {:?}",
            project.schemes
        );
    }
}

/// A target whose sources live in a `PBXFileSystemSynchronizedRootGroup`
/// (Xcode 16 "buildable folders") lists none of them in a build phase — they
/// are every compilable file physically under the folder. `target_source_files`
/// must walk that folder (recursively), include `.swift`/C-family sources, and
/// skip headers and other non-compiled files.
#[test]
fn synchronized_folder_sources_are_walked() {
    let root = std::env::temp_dir().join(format!("sweetpad-sync-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let xcodeproj = root.join("App.xcodeproj");
    let sources = root.join("App/Sources");
    fs::create_dir_all(xcodeproj.join("..")).unwrap();
    fs::create_dir_all(&xcodeproj).unwrap();
    fs::create_dir_all(sources.join("Nested")).unwrap();
    // The synchronized `Sources` folder nests under a `path = App` group, so its
    // members resolve to `<root>/App/Sources/...` — the same accumulation Xcode
    // applies. A naive `<root>/Sources` would miss them.
    fs::write(sources.join("Alpha.swift"), "let a = 1\n").unwrap();
    fs::write(sources.join("Beta.swift"), "let b = 2\n").unwrap();
    fs::write(sources.join("Nested/Gamma.swift"), "let g = 3\n").unwrap();
    fs::write(sources.join("Header.h"), "// not compiled\n").unwrap();
    fs::write(sources.join("Readme.md"), "# not compiled\n").unwrap();

    let pbxproj = "\
// !$*UTF8*$!
{
\tarchiveVersion = 1;
\tobjects = {
\t\tPROJ = { isa = PBXProject; mainGroup = MAIN; targets = (APP); };
\t\tMAIN = { isa = PBXGroup; sourceTree = \"<group>\"; children = (APPGRP); };
\t\tAPPGRP = { isa = PBXGroup; path = App; sourceTree = \"<group>\"; children = (SYNC); };
\t\tSYNC = { isa = PBXFileSystemSynchronizedRootGroup; path = Sources; sourceTree = \"<group>\"; };
\t\tAPP = { isa = PBXNativeTarget; name = App; buildPhases = (); fileSystemSynchronizedGroups = (SYNC); };
\t};
\trootObject = PROJ;
}
";
    fs::write(xcodeproj.join("project.pbxproj"), pbxproj).unwrap();

    let files = target_source_files(&xcodeproj, "App").unwrap();
    let mut names: Vec<&str> = files
        .iter()
        .filter_map(|p| p.file_name().and_then(OsStr::to_str))
        .collect();
    names.sort_unstable();
    let _ = fs::remove_dir_all(&root);

    assert_eq!(
        names,
        vec!["Alpha.swift", "Beta.swift", "Gamma.swift"],
        "expected the synchronized folder's compilable sources (recursively), no header/markdown"
    );
}

/// A file unchecked from a target's membership appears in the synchronized
/// folder's `PBXFileSystemSynchronizedBuildFileExceptionSet.membershipExceptions`
/// and must be dropped from that target's sources.
#[test]
fn synchronized_folder_membership_exception_is_excluded() {
    let root = std::env::temp_dir().join(format!("sweetpad-sync-exc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let xcodeproj = root.join("App.xcodeproj");
    let sources = root.join("App/Sources");
    fs::create_dir_all(&xcodeproj).unwrap();
    fs::create_dir_all(&sources).unwrap();
    fs::write(sources.join("Included.swift"), "let a = 1\n").unwrap();
    fs::write(sources.join("Excluded.swift"), "let b = 2\n").unwrap();

    // The exception set targets `App` and excludes `Excluded.swift` (group-relative).
    let pbxproj = "\
// !$*UTF8*$!
{
\tarchiveVersion = 1;
\tobjects = {
\t\tPROJ = { isa = PBXProject; mainGroup = MAIN; targets = (APP); };
\t\tMAIN = { isa = PBXGroup; sourceTree = \"<group>\"; children = (APPGRP); };
\t\tAPPGRP = { isa = PBXGroup; path = App; sourceTree = \"<group>\"; children = (SYNC); };
\t\tSYNC = { isa = PBXFileSystemSynchronizedRootGroup; path = Sources; sourceTree = \"<group>\"; exceptions = (EXC); };
\t\tEXC = { isa = PBXFileSystemSynchronizedBuildFileExceptionSet; target = APP; membershipExceptions = (\"Excluded.swift\"); };
\t\tAPP = { isa = PBXNativeTarget; name = App; buildPhases = (); fileSystemSynchronizedGroups = (SYNC); };
\t};
\trootObject = PROJ;
}
";
    fs::write(xcodeproj.join("project.pbxproj"), pbxproj).unwrap();

    let files = target_source_files(&xcodeproj, "App").unwrap();
    let mut names: Vec<&str> = files
        .iter()
        .filter_map(|p| p.file_name().and_then(OsStr::to_str))
        .collect();
    names.sort_unstable();
    let _ = fs::remove_dir_all(&root);

    assert_eq!(
        names,
        vec!["Included.swift"],
        "Excluded.swift should be dropped by the membership exception"
    );
}

/// True if the dependency graph (target → its same-project dependencies) has no
/// cycle reachable from `start` — a DFS with a recursion stack.
fn acyclic_from(
    start: &str,
    adj: &BTreeMap<String, Vec<String>>,
    stack: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
) -> bool {
    if done.contains(start) {
        return true;
    }
    if !stack.insert(start.to_string()) {
        return false; // back-edge: cycle
    }
    for dep in adj.get(start).into_iter().flatten() {
        if !acyclic_from(dep, adj, stack, done) {
            return false;
        }
    }
    stack.remove(start);
    done.insert(start.to_string());
    true
}

/// The BSP graph queries must hold across every real project in the corpus
/// (no source trees needed — this is pbxproj-level): every target resolves its
/// dependency, package, and source queries without error; each derived
/// dependency names a real same-project target; and the graph is acyclic. This
/// exercises the synchronized-group + dependency derivation against IceCubes /
/// NetNewsWire / Tuist structures the synthetic fixtures don't cover.
#[test]
fn corpus_dependency_graphs_are_sound() {
    let projects = xcodeproj_dirs(&fixtures_root());
    let mut checked = 0;
    for path in &projects {
        let Ok(project) = open(path) else {
            continue; // open() failures are covered by opens_every_xcodeproj_in_corpus
        };
        let names: BTreeSet<&str> = project.targets.iter().map(|t| t.name.as_str()).collect();
        let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for t in &project.targets {
            // None of the per-target queries may error for a project that opened.
            let deps = target_dependencies(path, &t.name).unwrap_or_else(|e| {
                panic!(
                    "{}: target_dependencies({}) failed: {e}",
                    path.display(),
                    t.name
                )
            });
            target_source_files(path, &t.name).unwrap_or_else(|e| {
                panic!(
                    "{}: target_source_files({}) failed: {e}",
                    path.display(),
                    t.name
                )
            });
            target_has_package_products(path, &t.name).unwrap_or_else(|e| {
                panic!(
                    "{}: target_has_package_products({}) failed: {e}",
                    path.display(),
                    t.name
                )
            });
            for d in &deps {
                assert!(
                    names.contains(d.as_str()),
                    "{}: target '{}' depends on '{d}', not a target of this project {names:?}",
                    path.display(),
                    t.name,
                );
            }
            adj.insert(t.name.clone(), deps);
        }
        let (mut stack, mut done) = (BTreeSet::new(), BTreeSet::new());
        for t in &project.targets {
            assert!(
                acyclic_from(&t.name, &adj, &mut stack, &mut done),
                "{}: dependency cycle reachable from '{}'",
                path.display(),
                t.name,
            );
        }
        checked += 1;
    }
    assert!(
        checked > 5,
        "expected to check several corpus projects, only {checked}"
    );
}

/// The v3 prepare gate: a pure-Swift target + its deps are self-buildable (so
/// modules can be emitted with `swiftc` directly), and the dependency closure is
/// reported deps-first.
#[test]
fn multimodule_closure_is_self_buildable() {
    let mm = fixtures_root().join("_synthetic-multimodule/project/MultiModule.xcodeproj");
    assert_eq!(
        transitive_dependencies(&mm, "ModuleB").unwrap(),
        vec!["ModuleA".to_string()],
        "ModuleB depends on ModuleA"
    );
    assert!(
        transitive_dependencies(&mm, "ModuleA").unwrap().is_empty(),
        "ModuleA has no deps"
    );
    assert!(is_self_buildable(&mm, "ModuleA").unwrap());
    assert!(is_self_buildable(&mm, "ModuleB").unwrap());
}

/// A target linking a Swift-package product can't be emitted by `swiftc` alone —
/// it must fall back to a real build.
#[test]
fn spm_target_is_not_self_buildable() {
    let spm = fixtures_root().join("_synthetic-spm/project/SpmApp.xcodeproj");
    assert!(
        !is_self_buildable(&spm, "SpmApp").unwrap(),
        "SpmApp links a package product, so it is not a pure-swiftc emit"
    );
}
