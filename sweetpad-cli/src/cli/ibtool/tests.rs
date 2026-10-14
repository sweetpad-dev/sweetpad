//! The stand-in against what Xcode's ibtool made of the storyboards in
//! `fixtures/ibtool` (`regen.py` records it).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::nib::read::{self, Read};
use super::*;
use crate::cli::keyed_archive::{self, Plist};
use crate::cli::testdir::TempDir;

/// The fixtures, or `IBTOOL_FIXTURES`, a folder laid out the same way, to
/// hold other storyboards up against Xcode's ibtool.
fn fixtures() -> PathBuf {
    std::env::var_os("IBTOOL_FIXTURES").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/ibtool"),
        PathBuf::from,
    )
}

fn storyboards() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "storyboard"))
        .collect();
    found.sort();
    found
}

/// The arrays whose order comes from a hash table in Xcode's ibtool, so
/// isn't the document's or anything this can reproduce.
const UNORDERED: &[&str] = &["UINibObjectsKey", "UIViewAutolayoutConstraints"];

fn scalar(value: &Read) -> String {
    match value {
        Read::Data(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) if !text.starts_with('\u{7}') => format!("{text:?}"),
            _ => bytes.iter().fold("data ".to_string(), |mut out, b| {
                let _ = write!(out, "{b:02x}");
                out
            }),
        },
        other => format!("{other:?}"),
    }
}

/// What an object sorts by in an unordered array: its class and values, and
/// those of the objects it refers to, a level down.
fn signature(objects: &[read::Object], index: usize, depth: usize) -> String {
    let object = &objects[index];
    let mut values: Vec<&(String, Read)> = object.values.iter().collect();
    values.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = object.class.clone();
    for (key, value) in values {
        let text = match value {
            Read::Object(child) if depth > 0 => {
                format!("({})", signature(objects, *child, depth - 1))
            }
            Read::Object(child) => objects[*child].class.clone(),
            other => scalar(other),
        };
        let _ = write!(out, " {key}={text}");
    }
    out
}

/// A nib as a tree that doesn't depend on the order objects or keys are
/// written in: each object's values sorted by key, the unordered arrays
/// sorted, and every object numbered by when a walk in that order meets it.
fn canonical(data: &[u8]) -> String {
    let objects = read::parse(data);
    let mut out = String::new();
    let mut seen = BTreeMap::new();
    walk(&objects, 0, 0, "", false, &mut seen, &mut out);
    out
}

fn walk(
    objects: &[read::Object],
    index: usize,
    indent: usize,
    label: &str,
    unordered: bool,
    seen: &mut BTreeMap<usize, usize>,
    out: &mut String,
) {
    let pad = "  ".repeat(indent);
    let object = &objects[index];
    if let Some(n) = seen.get(&index) {
        let _ = writeln!(out, "{pad}{label}<{} @{n}>", object.class);
        return;
    }
    let n = seen.len();
    seen.insert(index, n);
    let _ = writeln!(out, "{pad}{label}{} @{n}", object.class);
    let mut values: Vec<&(String, Read)> = object.values.iter().collect();
    values.sort_by(|a, b| a.0.cmp(&b.0));
    if unordered {
        values.sort_by_cached_key(|(key, value)| {
            let order = match value {
                Read::Object(child) => signature(objects, *child, 2),
                other => scalar(other),
            };
            (key.clone(), order)
        });
    }
    for (key, value) in values {
        match value {
            Read::Object(child) => walk(
                objects,
                *child,
                indent + 1,
                &format!("{key}: "),
                UNORDERED.contains(&key.as_str()),
                seen,
                out,
            ),
            other => {
                let _ = writeln!(out, "{pad}  {key}: {}", scalar(other));
            }
        }
    }
}

fn first_difference(actual: &str, expected: &str) -> String {
    for (i, (a, e)) in actual.lines().zip(expected.lines()).enumerate() {
        if a != e {
            return format!("line {}:\n  got:      {a:?}\n  expected: {e:?}", i + 1);
        }
    }
    format!(
        "one is a prefix of the other ({} vs {} lines)",
        actual.lines().count(),
        expected.lines().count()
    )
}

fn sorted(value: Plist) -> Plist {
    match value {
        Plist::Dict(entries) => {
            let mut entries: Vec<(String, Plist)> =
                entries.into_iter().map(|(k, v)| (k, sorted(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Plist::Dict(entries)
        }
        other => other,
    }
}

fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect()
}

#[test]
fn each_storyboard_compiles_as_xcode_compiles_it() {
    let mut failures = Vec::new();
    for storyboard in storyboards() {
        let name = storyboard
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let out = TempDir::new("sweetpad-ibtool");
        let args = Args {
            compilation_directory: Some(out.to_path_buf()),
            ..Args::default()
        };
        if let Err(e) = compile(&storyboard, &args) {
            failures.push(format!("{name}: {e}"));
            continue;
        }
        let ours = files(&out.join(format!("{name}.storyboardc")));
        let xcode = files(
            &fixtures()
                .join("expected")
                .join(format!("{name}.storyboardc")),
        );
        if ours.keys().ne(xcode.keys()) {
            failures.push(format!(
                "{name} compiles to {:?}, Xcode to {:?}",
                ours.keys().collect::<Vec<_>>(),
                xcode.keys().collect::<Vec<_>>()
            ));
            continue;
        }
        for (file, data) in &ours {
            if file == "Info.plist" {
                let (got, want) = (
                    sorted(keyed_archive::read::parse(data)),
                    sorted(keyed_archive::read::parse(&xcode[file])),
                );
                if got != want {
                    failures.push(format!(
                        "{name} {file}:\n  got:      {got:?}\n  expected: {want:?}"
                    ));
                }
                continue;
            }
            let (got, want) = (canonical(data), canonical(&xcode[file]));
            if let Some(dump) = std::env::var_os("IBTOOL_DUMP") {
                let dump = Path::new(&dump);
                std::fs::write(dump.join(format!("{name}-{file}.got")), &got).unwrap();
                std::fs::write(dump.join(format!("{name}-{file}.want")), &want).unwrap();
            }
            if got != want {
                failures.push(format!("{name} {file}, {}", first_difference(&got, &want)));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_arguments_swift_build_passes_parse() {
    let args: Vec<String> = [
        "--errors",
        "--warnings",
        "--notices",
        "--module",
        "App",
        "--output-partial-info-plist",
        "/out/Launch-SBPartialInfo.plist",
        "--auto-activate-custom-fonts",
        "--target-device",
        "iphone",
        "--target-device",
        "ipad",
        "--minimum-deployment-target",
        "18.0",
        "--output-format",
        "human-readable-text",
        "/src/Launch.storyboard",
        "--compilation-directory",
        "/out",
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    let parsed = parse_args(&args).unwrap();
    assert_eq!(parsed.compilation_directory, Some(PathBuf::from("/out")));
    assert_eq!(parsed.inputs, vec![PathBuf::from("/src/Launch.storyboard")]);
    assert!(parse_args(&["--frobnicate".to_string()]).is_err());
}

#[test]
fn linking_keeps_the_lproj_folder() {
    let dir = TempDir::new("sweetpad-ibtool-link");
    let compiled = dir.join("build/Base.lproj/Main.storyboardc");
    std::fs::create_dir_all(&compiled).unwrap();
    std::fs::write(compiled.join("Info.plist"), b"x").unwrap();
    let product = dir.join("App.app");
    link(&compiled, &product).unwrap();
    assert!(
        product
            .join("Base.lproj/Main.storyboardc/Info.plist")
            .is_file()
    );
}

#[test]
fn a_view_it_doesnt_know_is_refused_by_name() {
    let document = sweetpad_lib::xcscheme::parse(
        r#"<document type="com.apple.InterfaceBuilder3.CocoaTouch.Storyboard.XIB" initialViewController="a">
        <scenes><scene sceneID="s"><objects>
            <viewController id="a" sceneMemberID="viewController">
                <view key="view" contentMode="scaleToFill" id="v">
                    <rect key="frame" x="0" y="0" width="10" height="10"/>
                    <subviews><datePicker id="d"><rect key="frame" x="0" y="0" width="1" height="1"/></datePicker></subviews>
                </view>
            </viewController>
        </objects></scene></scenes></document>"#,
    )
    .unwrap();
    let Err(error) = storyboard::compile(&document) else {
        panic!("compiled");
    };
    assert!(error.contains("<datePicker>"), "{error}");
}
