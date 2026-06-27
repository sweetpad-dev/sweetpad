//! Round-trip verification for the serializers: parse every fixture file
//! (`project.pbxproj`, `*.xcscheme`, `contents.xcworkspacedata`,
//! `*.xcconfig`), re-serialize it, and compare against the raw bytes.
//!
//! Two tiers:
//!
//! * **Byte-exact** — the serialized output must equal the file on disk for
//!   every file *not* in the allowlist below. Everything Xcode (or Tuist)
//!   wrote round-trips byte-for-byte; the allowlisted files are hand-written
//!   synthetic fixtures whose formatting (4-space indentation, one-line XML
//!   tags) is not what Xcode would ever emit, and the writers intentionally
//!   produce Xcode-canonical formatting.
//! * **Semantic** — for *every* file, including allowlisted ones, parsing the
//!   serialized output must yield the same data as parsing the original.
//!
//! The allowlist is exact: a file that regresses *or* an allowlisted file
//! that becomes byte-exact fails the test, so the list always reflects
//! reality.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use sweetpad_lib::pbxproj::{self, Value};
use sweetpad_lib::{pbxproj_writer, xcconfig, xcscheme};

/// Fixture files whose source formatting is not Xcode-canonical (hand-written
/// synthetic projects), with the reason they can't be byte-exact.
const NOT_BYTE_EXACT: &[(&str, &str)] = &[
    (
        "_synthetic-custom-config/xcode-15.4.0/project/Scratch.xcodeproj/project.pbxproj",
        "hand-written: 4-space indentation instead of tabs",
    ),
    (
        "_synthetic-custom-config/xcode-16.4.0/project/Scratch.xcodeproj/project.pbxproj",
        "hand-written: 4-space indentation instead of tabs",
    ),
    (
        "_synthetic-custom-config/xcode-26.5.0/project/Scratch.xcodeproj/project.pbxproj",
        "hand-written: 4-space indentation instead of tabs",
    ),
    (
        "_synthetic-multimodule/project/MultiModule.xcodeproj/project.pbxproj",
        "hand-written: 4-space indentation instead of tabs",
    ),
    (
        "_synthetic-multimodule/project/MultiModule.xcodeproj/xcshareddata/xcschemes/ModuleB.xcscheme",
        "hand-written: attributes inline in the open tag",
    ),
    (
        "_synthetic-objc-headers/project/ObjCHeaders.xcodeproj/project.pbxproj",
        "hand-written: 4-space indentation instead of tabs",
    ),
    (
        "_synthetic-objc-headers/project/ObjCHeaders.xcodeproj/xcshareddata/xcschemes/ObjCHeaders.xcscheme",
        "hand-written: attributes inline in the open tag",
    ),
    (
        "_synthetic-rich/xcode-26.5.0/raw/Scratch.xcodeproj/project.pbxproj",
        "hand-written: 4-space indentation instead of tabs",
    ),
    (
        "_synthetic-rich/xcode-26.5.0/raw/Scratch.xcodeproj/xcshareddata/xcschemes/Scratch.xcscheme",
        "hand-written: attributes inline in the open tag",
    ),
    (
        "_synthetic-staticlib/xcode-26.5.0/raw/Scratch.xcodeproj/project.pbxproj",
        "hand-written: 4-space indentation instead of tabs",
    ),
    (
        "_synthetic-staticlib/xcode-26.5.0/raw/Scratch.xcodeproj/xcshareddata/xcschemes/Scratch.xcscheme",
        "hand-written: attributes inline in the open tag",
    ),
    (
        "_synthetic-xcconfigs/xcode-15.4.0/project/Scratch.xcodeproj/project.pbxproj",
        "hand-written: 4-space indentation instead of tabs",
    ),
    (
        "_synthetic-xcconfigs/xcode-16.4.0/project/Scratch.xcodeproj/project.pbxproj",
        "hand-written: 4-space indentation instead of tabs",
    ),
    (
        "_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj/project.pbxproj",
        "hand-written: 4-space indentation instead of tabs",
    ),
];

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn collect_files(ext_or_name: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(&fixtures_root(), ext_or_name, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, keep: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            // Skip transitive SwiftPM checkouts pulled in during builds.
            if matches!(
                p.file_name().and_then(OsStr::to_str),
                Some(".derived" | ".cache" | "DerivedData")
            ) {
                continue;
            }
            walk(&p, keep, out);
        } else if keep(&p) {
            out.push(p);
        }
    }
}

fn rel(path: &Path) -> String {
    path.strip_prefix(fixtures_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

fn first_diff_line(raw: &str, serialized: &str) -> (usize, String, String) {
    let mut raw_lines = raw.lines();
    let mut ser_lines = serialized.lines();
    let mut line_no = 0usize;
    loop {
        line_no += 1;
        match (raw_lines.next(), ser_lines.next()) {
            (Some(raw_line), Some(ser_line)) if raw_line == ser_line => {}
            (raw_line, ser_line) => {
                return (
                    line_no,
                    raw_line.unwrap_or("<eof>").to_string(),
                    ser_line.unwrap_or("<eof>").to_string(),
                );
            }
        }
    }
}

/// Compare byte-exactness across `files`, asserting the mismatch set equals
/// the allowlisted subset for this extension and reporting a readable diff
/// for anything unexpected.
fn assert_byte_exact(files: &[PathBuf], serialize: &dyn Fn(&Path, &str) -> String) {
    let mut unexpected = Vec::new();
    let mut exact = 0usize;
    let mut allowlisted_now_exact: Vec<String> = NOT_BYTE_EXACT
        .iter()
        .map(|(p, _)| (*p).to_string())
        .filter(|p| files.iter().any(|f| rel(f) == *p))
        .collect();
    for f in files {
        let raw = fs::read_to_string(f).unwrap_or_else(|e| panic!("read {}: {e}", f.display()));
        let serialized = serialize(f, &raw);
        let relative = rel(f);
        if serialized == raw {
            exact += 1;
            assert!(
                !allowlisted_now_exact.contains(&relative),
                "{relative} is allowlisted as not byte-exact but now round-trips exactly; \
                 remove it from NOT_BYTE_EXACT"
            );
        } else if allowlisted_now_exact.contains(&relative) {
            allowlisted_now_exact.retain(|p| *p != relative);
        } else {
            let (line, want, got) = first_diff_line(&raw, &serialized);
            unexpected.push(format!(
                "{relative}: line {line}\n  raw: {want:?}\n  ser: {got:?}"
            ));
        }
    }
    assert!(
        unexpected.is_empty(),
        "{} file(s) no longer byte-exact:\n{}",
        unexpected.len(),
        unexpected.join("\n")
    );
    assert!(exact > 0, "no files were compared");
}

/// Order-insensitive structural equality for pbxproj values: dict key order
/// is formatting (the writer groups objects into sorted isa sections), so
/// semantic comparison treats dicts as maps.
fn semantically_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(i, j)| semantically_equal(i, j))
        }
        (Value::Dict(x), Value::Dict(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| semantically_equal(v, w)))
        }
        _ => false,
    }
}

/// `Foo.xcodeproj/project.pbxproj` → `Foo` (what Xcode embeds in
/// `Build configuration list for PBXProject "Foo"` annotations).
fn project_name(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_stem)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[test]
fn pbxproj_round_trips() {
    let files = collect_files(&|p| p.file_name() == Some(OsStr::new("project.pbxproj")));
    assert!(files.len() > 50, "expected a substantial corpus");
    assert_byte_exact(&files, &|f, raw| {
        let parsed = pbxproj::parse(raw).unwrap_or_else(|e| panic!("parse {}: {e}", f.display()));
        pbxproj_writer::serialize(&parsed, &project_name(f))
    });
    // Semantic tier: serialized output parses back to the same data, even
    // for files whose original formatting was not Xcode-canonical.
    for f in &files {
        let raw = fs::read_to_string(f).unwrap();
        let parsed = pbxproj::parse(&raw).unwrap();
        let serialized = pbxproj_writer::serialize(&parsed, &project_name(f));
        let reparsed = pbxproj::parse(&serialized)
            .unwrap_or_else(|e| panic!("reparse of serialized {}: {e}", rel(f)));
        assert!(
            semantically_equal(&parsed, &reparsed),
            "serialized {} parses to different data",
            rel(f)
        );
    }
}

#[test]
fn xcscheme_round_trips() {
    let files = collect_files(&|p| p.extension() == Some(OsStr::new("xcscheme")));
    assert!(files.len() > 100, "expected a substantial corpus");
    assert_byte_exact(&files, &|f, raw| {
        let parsed = xcscheme::parse(raw).unwrap_or_else(|e| panic!("parse {}: {e}", f.display()));
        xcscheme::serialize(&parsed)
    });
    for f in &files {
        let raw = fs::read_to_string(f).unwrap();
        let parsed = xcscheme::parse(&raw).unwrap();
        let reparsed = xcscheme::parse(&xcscheme::serialize(&parsed))
            .unwrap_or_else(|e| panic!("reparse of serialized {}: {e}", rel(f)));
        // Element equality is exact (attribute order is preserved data).
        assert_eq!(parsed, reparsed, "serialized {} parses differently", rel(f));
    }
}

#[test]
fn xcworkspacedata_round_trips() {
    let files = collect_files(&|p| p.extension() == Some(OsStr::new("xcworkspacedata")));
    assert!(files.len() > 50, "expected a substantial corpus");
    assert_byte_exact(&files, &|f, raw| {
        let parsed = xcscheme::parse(raw).unwrap_or_else(|e| panic!("parse {}: {e}", f.display()));
        xcscheme::serialize(&parsed)
    });
    for f in &files {
        let raw = fs::read_to_string(f).unwrap();
        let parsed = xcscheme::parse(&raw).unwrap();
        let reparsed = xcscheme::parse(&xcscheme::serialize(&parsed))
            .unwrap_or_else(|e| panic!("reparse of serialized {}: {e}", rel(f)));
        assert_eq!(parsed, reparsed, "serialized {} parses differently", rel(f));
    }
}

#[test]
fn xcconfig_round_trips() {
    let files = collect_files(&|p| p.extension() == Some(OsStr::new("xcconfig")));
    assert!(files.len() > 10, "expected a substantial corpus");
    assert_byte_exact(&files, &|f, raw| {
        let parsed = xcconfig::parse(raw).unwrap_or_else(|e| panic!("parse {}: {e}", f.display()));
        xcconfig::serialize(&parsed)
    });
    for f in &files {
        let raw = fs::read_to_string(f).unwrap();
        let parsed = xcconfig::parse(&raw).unwrap();
        let reparsed = xcconfig::parse(&xcconfig::serialize(&parsed))
            .unwrap_or_else(|e| panic!("reparse of serialized {}: {e}", rel(f)));
        assert_eq!(
            parsed.entries,
            reparsed.entries,
            "serialized {} parses differently",
            rel(f)
        );
    }
}
