//! The stand-in against what Xcode's xcstringstool wrote for the probe
//! catalogs in `fixtures/xcstringstool` (`regen.py` records it).

use std::path::{Path, PathBuf};

use super::*;
use crate::cli::testdir::TempDir;

/// The probe catalogs, by table name.
const CATALOGS: &[&str] = &["Invalid", "Localizable", "Probe", "Symbols", "my-table"];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/xcstringstool")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn catalog(table: &str) -> Catalog {
    Catalog::parse(&read(&fixtures().join(format!("{table}.xcstrings")))).unwrap()
}

/// Problems as the `.errors` files list them: sorted, the catalog in $(DIR).
fn lines(problems: &[Problem], table: &str) -> String {
    let path = PathBuf::from(format!("$(DIR)/{table}.xcstrings"));
    let mut lines: Vec<String> = problems.iter().map(|p| p.line(&path)).collect();
    lines.sort();
    lines.iter().map(|l| l.clone() + "\n").collect()
}

/// The first line where `actual` and `expected` part, for a readable failure.
fn first_difference(actual: &str, expected: &str) -> String {
    for (i, (a, e)) in actual.lines().zip(expected.lines()).enumerate() {
        if a != e {
            return format!("line {}:\n  got:      {a:?}\n  expected: {e:?}", i + 1);
        }
    }
    format!(
        "one is a prefix of the other ({} vs {} lines, {} vs {} bytes)",
        actual.lines().count(),
        expected.lines().count(),
        actual.len(),
        expected.len()
    )
}

fn assert_same(actual: &str, expected: &str, what: &str) {
    assert!(
        actual == expected,
        "{what} differs from xcstringstool's: {}",
        first_difference(actual, expected)
    );
}

/// Every file under `dir`, relative to it, sorted.
fn files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                out.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn args(force_all: bool) -> Args {
    Args {
        subcommand: "compile".into(),
        force_all,
        ..Args::default()
    }
}

#[test]
fn dry_runs_list_what_xcstringstool_lists() {
    for table in CATALOGS {
        for (mode, force_all) in [("compile", false), ("forced", true)] {
            let expected = fixtures().join(format!("expected/{table}/{mode}.dry-run"));
            if !expected.exists() {
                continue;
            }
            let options = compile::Options {
                languages: Vec::new(),
                force_all,
            };
            let mut listed = Vec::new();
            for (language, strings, dict) in compile::planned(&catalog(table), &options) {
                for (has, extension) in [(strings, "strings"), (dict, "stringsdict")] {
                    if has {
                        listed.push(format!("{language}.lproj/{table}.{extension}\n"));
                    }
                }
            }
            listed.sort();
            assert_same(
                &listed.concat(),
                &read(&expected),
                &format!("{table} {mode} dry run"),
            );
        }
    }
}

#[test]
fn compiles_write_what_xcstringstool_writes() {
    let dir = TempDir::new("xcstrings-compile");
    for table in CATALOGS {
        for (mode, force_all) in [("compile", false), ("forced", true)] {
            let expected = fixtures().join(format!("expected/{table}/{mode}"));
            let errors = fixtures().join(format!("expected/{table}/{mode}.errors"));
            if !errors.exists() {
                continue;
            }
            let out = dir.join(table).join(mode);
            let problems = compile_catalog(&catalog(table), &args(force_all), table, &out).unwrap();
            assert_same(
                &lines(&problems, table),
                &read(&errors),
                &format!("{table} {mode} errors"),
            );
            if !expected.exists() {
                continue;
            }
            assert_eq!(files(&out), files(&expected), "{table} {mode} files");
            for file in files(&expected) {
                assert_same(
                    &read(&out.join(&file)),
                    &read(&expected.join(&file)),
                    &format!("{table} {mode} {}", file.display()),
                );
            }
        }
    }
}

#[test]
fn a_language_with_an_error_is_not_written() {
    let dir = TempDir::new("xcstrings-invalid");
    let problems = compile_catalog(&catalog("Invalid"), &args(false), "Invalid", &dir).unwrap();
    assert!(!problems.is_empty());
    assert_eq!(
        files(&dir),
        [
            PathBuf::from("uk.lproj/Invalid.strings"),
            PathBuf::from("uk.lproj/Invalid.stringsdict")
        ]
    );
}

#[test]
fn symbols_match_xcstringstool() {
    for table in CATALOGS {
        let (swift, problems) = symbols::generate(&catalog(table), table);
        let expected = fixtures().join(format!(
            "expected/{table}/GeneratedStringSymbols_{table}.swift"
        ));
        assert_same(&swift, &read(&expected), &format!("{table} symbols"));
        assert_same(
            &lines(&problems, table),
            &read(&fixtures().join(format!("expected/{table}/symbols.errors"))),
            &format!("{table} symbol errors"),
        );
    }
}

#[test]
fn languages_narrow_a_compile() {
    let options = compile::Options {
        languages: vec!["uk".into()],
        force_all: false,
    };
    let planned: Vec<String> = compile::planned(&catalog("Probe"), &options)
        .into_iter()
        .map(|(language, ..)| language)
        .collect();
    assert_eq!(planned, ["uk"]);
}

#[test]
fn a_catalog_that_isnt_one_says_why() {
    assert_eq!(
        Catalog::parse("{\"sourceLanguage\":").unwrap_err(),
        "The data couldn’t be read because it isn’t in the correct format."
    );
    assert_eq!(
        Catalog::parse("{\"strings\":{}}").unwrap_err(),
        "Missing required key 'version'"
    );
}

#[test]
fn the_command_line_takes_what_swift_build_passes() {
    let parsed = Args::parse(&[
        "compile".into(),
        "--dry-run".into(),
        "-l".into(),
        "uk".into(),
        "--force-build-all-strings".into(),
        "--output-directory".into(),
        "/out".into(),
        "/in/Localizable.xcstrings".into(),
    ])
    .unwrap();
    assert!(parsed.dry_run && parsed.force_all);
    assert_eq!(parsed.languages, ["uk"]);
    assert_eq!(parsed.output.as_deref(), Some(Path::new("/out")));
    assert_eq!(
        parsed.input.as_deref(),
        Some(Path::new("/in/Localizable.xcstrings"))
    );
    assert!(
        Args::parse(&[
            "compile".into(),
            "--format".into(),
            "stringsdictOnly".into()
        ])
        .is_err()
    );
}
