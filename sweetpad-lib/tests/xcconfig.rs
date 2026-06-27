use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use sweetpad_lib::xcconfig::{Assignment, Entry, Include, parse};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn find_xcconfig_files(root: &Path) -> Vec<PathBuf> {
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
        if p.is_dir() {
            if matches!(
                p.file_name().and_then(OsStr::to_str),
                Some(".derived" | ".cache" | "DerivedData")
            ) {
                continue;
            }
            walk(&p, out);
        } else if p.extension() == Some(OsStr::new("xcconfig")) {
            out.push(p);
        }
    }
}

#[test]
fn parses_every_xcconfig_in_corpus() {
    let files = find_xcconfig_files(&fixtures_root());
    assert!(
        !files.is_empty(),
        "expected at least one xcconfig fixture, found none"
    );

    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let s = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{}: read error: {e}", path.display()));
                continue;
            }
        };
        if let Err(e) = parse(&s) {
            failures.push(format!("{}: parse error: {e}", path.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} xcconfig fixtures failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

fn read_fixture(rel: &str) -> String {
    fs::read_to_string(fixtures_root().join(rel)).unwrap()
}

fn synthetic(name: &str) -> String {
    read_fixture(&format!(
        "_synthetic-xcconfigs/xcode-26.5.0/xcconfigs/{name}.xcconfig"
    ))
}

#[test]
fn synthetic_conditional_sdk_has_four_assignments() {
    let c = parse(&synthetic("conditional-sdk")).unwrap();
    let assigns: Vec<&Assignment> = c
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Assignment(a) => Some(a),
            Entry::Include(_) => None,
        })
        .collect();
    assert_eq!(assigns.len(), 4);
    assert_eq!(assigns[0].key, "FOO");
    assert!(assigns[0].conditions.is_empty());
    assert_eq!(assigns[0].value, "base");

    for a in &assigns[1..] {
        assert_eq!(a.key, "FOO");
        assert_eq!(a.conditions.len(), 1);
        assert_eq!(a.conditions[0].key, "sdk");
    }
}

#[test]
fn synthetic_conditional_arch_resolves() {
    let c = parse(&synthetic("conditional-arch")).unwrap();
    assert_eq!(c.entries.len(), 4);
    // Verify the arch-conditional entries exist.
    let arches: Vec<&str> = c
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Assignment(a) if !a.conditions.is_empty() => {
                Some(a.conditions[0].value.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(arches, vec!["arm64", "arm64e", "x86_64"]);
}

#[test]
fn synthetic_include_directive() {
    let c = parse(&synthetic("include-directive")).unwrap();
    assert_eq!(c.entries.len(), 2);
    match &c.entries[0] {
        Entry::Include(Include { path, optional }) => {
            assert_eq!(path, "conditional-sdk.xcconfig");
            assert!(!optional);
        }
        Entry::Assignment(_) => panic!("expected Include, got {:?}", c.entries[0]),
    }
    match &c.entries[1] {
        Entry::Assignment(a) => {
            assert_eq!(a.key, "EXTRA");
            assert_eq!(a.value, "layered");
        }
        Entry::Include(_) => panic!("expected Assignment"),
    }
}

#[test]
fn synthetic_multi_line_continuation() {
    let c = parse(&synthetic("multi-line-continuation")).unwrap();
    assert_eq!(c.entries.len(), 1);
    match &c.entries[0] {
        Entry::Assignment(a) => {
            assert_eq!(a.key, "QUUX");
            assert_eq!(a.value, "first_part second_part third_part");
        }
        Entry::Include(_) => panic!("expected Assignment"),
    }
}

#[test]
fn synthetic_inherited() {
    let c = parse(&synthetic("inherited")).unwrap();
    assert_eq!(c.entries.len(), 2);
    for entry in &c.entries {
        let Entry::Assignment(a) = entry else {
            panic!("expected Assignment");
        };
        assert!(
            a.value.contains("$(inherited)"),
            "expected $(inherited) in value: {}",
            a.value
        );
    }
}

#[test]
fn real_world_icecubes_release() {
    let c = parse(&read_fixture(
        "ice-cubes/xcode-26.5.0/raw/IceCubesApp-release.xcconfig",
    ))
    .unwrap();
    assert_eq!(c.entries.len(), 2);
    let assigns: Vec<&Assignment> = c
        .entries
        .iter()
        .filter_map(|e| match e {
            Entry::Assignment(a) => Some(a),
            Entry::Include(_) => None,
        })
        .collect();
    assert_eq!(assigns[0].key, "DEVELOPMENT_TEAM");
    assert_eq!(assigns[1].key, "BUNDLE_ID_PREFIX");
}
