use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use sweetpad::project::{Target, open, target_source_files};

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
