use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use sweetpad::xcscheme::{Element, parse};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn find_xcscheme_files(root: &Path) -> Vec<PathBuf> {
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
        } else if p.extension() == Some(OsStr::new("xcscheme")) {
            out.push(p);
        }
    }
}

#[test]
fn parses_every_xcscheme_in_corpus() {
    let files = find_xcscheme_files(&fixtures_root());
    assert!(
        !files.is_empty(),
        "expected at least one xcscheme fixture, found none"
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
        match parse(&s) {
            Ok(root) => {
                if root.name != "Scheme" {
                    failures.push(format!(
                        "{}: root is <{}>, expected <Scheme>",
                        path.display(),
                        root.name
                    ));
                }
            }
            Err(e) => {
                failures.push(format!("{}: parse error: {e}", path.display()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} xcscheme fixtures failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

fn read_fixture(rel: &str) -> Element {
    let s = fs::read_to_string(fixtures_root().join(rel)).unwrap();
    parse(&s).unwrap()
}

#[test]
fn kingfisher_scheme_has_expected_actions() {
    let root = read_fixture(
        "kingfisher/xcode-26.5.0/raw/Kingfisher.xcodeproj/xcshareddata/xcschemes/Kingfisher.xcscheme",
    );
    assert_eq!(root.name, "Scheme");
    assert!(root.child("BuildAction").is_some());
    let test_action = root
        .child("TestAction")
        .expect("Kingfisher scheme should have TestAction");
    assert_eq!(test_action.attr("buildConfiguration"), Some("Debug"));
    let launch = root.child("LaunchAction").expect("LaunchAction");
    assert_eq!(launch.attr("buildConfiguration"), Some("Debug"));
}

#[test]
fn kingfisher_buildable_references_present() {
    let root = read_fixture(
        "kingfisher/xcode-26.5.0/raw/Kingfisher.xcodeproj/xcshareddata/xcschemes/Kingfisher.xcscheme",
    );
    let refs = root.descendants_named("BuildableReference");
    assert!(
        !refs.is_empty(),
        "expected at least one BuildableReference in the Kingfisher scheme"
    );
    let first = refs[0];
    assert!(first.attr("BlueprintIdentifier").is_some());
    assert!(first.attr("BlueprintName").is_some());
    assert!(first.attr("BuildableName").is_some());
    assert!(first.attr("ReferencedContainer").is_some());
}
