//! The stand-in against what Xcode's ibtool made of the storyboards in
//! `fixtures/ibtool` (`regen.py` records it).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::nib::read::{self, Read};
use super::*;
use crate::cli::keyed_archive::{self, Plist};
use crate::cli::testdir::TempDir;
mod normalize;
mod operations;

/// The fixtures, or `IBTOOL_FIXTURES`, a folder laid out the same way, to
/// hold other storyboards up against Xcode's ibtool.
fn fixtures() -> PathBuf {
    std::env::var_os("IBTOOL_FIXTURES").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/ibtool"),
        PathBuf::from,
    )
}

fn documents() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .flat_map(|e| {
            let path = e.unwrap().path();
            if path.is_dir() && path.extension().is_some_and(|e| e == "lproj") {
                std::fs::read_dir(path)
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .collect()
            } else {
                vec![path]
            }
        })
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e == "storyboard" || e == "xib")
        })
        .collect();
    found.sort();
    found
}

/// The arrays whose order comes from a hash table in Xcode's ibtool, so
/// isn't the document's or anything this can reproduce.
const UNORDERED: &[&str] = &[
    "NSConnections",
    "NSViewConstraints",
    "UINibObjectsKey",
    "UINibConnectionsKey",
    "UIViewAutolayoutConstraints",
    "UIStoryboardSegueTemplates",
    "UINibKeyValuePairsKey",
];

fn scalar(value: &Read) -> String {
    match value {
        Read::Data(bytes) if bytes.starts_with(b"canonical nested nib:\n") => {
            String::from_utf8(bytes.clone()).unwrap()
        }
        Read::Data(bytes) if bytes.starts_with(b"NIBArchive") => {
            format!("nested nib:\n{}", canonical(bytes))
        }
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
    if matches!(
        object.class.as_str(),
        "NSDictionary" | "NSMutableDictionary"
    ) {
        let entries: Vec<_> = object
            .values
            .iter()
            .filter(|(key, _)| key == "UINibEncoderEmptyKey")
            .collect();
        let mut pairs: Vec<_> = entries.chunks(2).collect();
        pairs.sort_by_cached_key(|pair| match &pair[0].1 {
            Read::Object(key) => signature(objects, *key, 1),
            other => scalar(other),
        });
        values.retain(|(key, _)| key != "UINibEncoderEmptyKey");
        for pair in pairs {
            values.extend(pair.iter().copied());
        }
    }
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
    canonical_with(data, &BTreeMap::new())
}
fn canonical_with(data: &[u8], names: &normalize::Names) -> String {
    canonical_with_color_precision(data, names, false)
}

fn canonical_with_color_precision(
    data: &[u8],
    names: &normalize::Names,
    calibrated: bool,
) -> String {
    let mut objects = read::parse(data);
    if calibrated {
        // ICC implementations differ below one thousandth of a channel.
        // Preserve every archive field and compare converted color channels
        // at that precision; all other fixture values remain exact.
        for object in &mut objects {
            if object.class != "UIColor" {
                continue;
            }
            for (key, value) in &mut object.values {
                if ["UIWhite", "UIRed", "UIGreen", "UIBlue"].contains(&key.as_str())
                    && let Read::Float(channel) = value
                {
                    *channel = (*channel * 1000.0).round() / 1000.0;
                }
            }
        }
    }
    normalize::prepare(&mut objects, names);
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
    if matches!(
        object.class.as_str(),
        "NSDictionary" | "NSMutableDictionary"
    ) {
        let entries: Vec<_> = object
            .values
            .iter()
            .filter(|(key, _)| key == "UINibEncoderEmptyKey")
            .collect();
        let mut pairs: Vec<_> = entries.chunks(2).collect();
        pairs.sort_by_cached_key(|pair| match &pair[0].1 {
            Read::Object(key) => signature(objects, *key, 1),
            other => scalar(other),
        });
        values.retain(|(key, _)| key != "UINibEncoderEmptyKey");
        for pair in pairs {
            values.extend(pair.iter().copied());
        }
    }
    if unordered {
        values.sort_by_cached_key(|(key, value)| {
            let order = match value {
                Read::Object(child) => signature(objects, *child, 2),
                other => scalar(other),
            };
            let previous = match value {
                Read::Object(child) => seen.get(child).copied().unwrap_or(usize::MAX),
                _ => usize::MAX,
            };
            (key.clone(), order, previous)
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
            return format!(
                "line {}:\n  got:      {:?}\n  expected: {:?}",
                i + 1,
                a.chars().take(200).collect::<String>(),
                e.chars().take(200).collect::<String>()
            );
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
        Plist::Array(entries) => Plist::Array(entries.into_iter().map(sorted).collect()),
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
    if dir.is_file() {
        return BTreeMap::from([("document.nib".to_string(), std::fs::read(dir).unwrap())]);
    }
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

fn dump_native_fixture(compiled_name: &str, name: &str) {
    if let Some(dump) = std::env::var_os("IBTOOL_DUMP") {
        let expected = fixtures().join("expected").join(compiled_name);
        if expected.exists() {
            for (file, bytes) in files(&expected) {
                if bytes.starts_with(b"NIBArchive") {
                    std::fs::write(
                        Path::new(&dump).join(format!("{name}-{file}.native")),
                        canonical(&bytes),
                    )
                    .unwrap();
                }
            }
        }
    }
}

#[test]
fn each_document_compiles_as_xcode_compiles_it() {
    let mut failures = Vec::new();
    for storyboard in documents() {
        let name = storyboard
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let compiled_name = format!(
            "{name}.{}",
            if storyboard.extension().is_some_and(|e| e == "xib") {
                "nib"
            } else {
                "storyboardc"
            }
        );
        let out = TempDir::new("sweetpad-ibtool");
        dump_native_fixture(&compiled_name, &name);
        let args = Args {
            compilation_directory: Some(out.to_path_buf()),
            module: "App".to_string(),
            ..Args::default()
        };
        // A corpus sweep can compare the actual Linux compiler's output,
        // preserving source-path context such as Base.lproj localization.
        let actual = if let Some(actual) = std::env::var_os("IBTOOL_ACTUAL") {
            PathBuf::from(actual)
        } else {
            if let Err(e) = compile(&storyboard, &args) {
                failures.push(format!("{name}: {e}"));
                continue;
            }
            out.to_path_buf()
        };
        let ours = files(&actual.join(&compiled_name));
        let xcode = files(&fixtures().join("expected").join(&compiled_name));
        if ours.keys().ne(xcode.keys()) {
            failures.push(format!(
                "{name} compiles to {:?}, Xcode to {:?}",
                ours.keys().collect::<Vec<_>>(),
                xcode.keys().collect::<Vec<_>>()
            ));
            continue;
        }
        let ours_names = normalize::upstreams(&ours);
        let xcode_names = normalize::upstreams(&xcode);
        for (file, data) in &ours {
            if Path::new(file)
                .extension()
                .is_some_and(|extension| extension == "plist")
            {
                let (got, want) = (
                    sorted(normalize::metadata(data)),
                    sorted(normalize::metadata(&xcode[file])),
                );
                if let Some(dump) = std::env::var_os("IBTOOL_DUMP") {
                    let dump = Path::new(&dump);
                    std::fs::write(dump.join(format!("{name}-{file}.got")), format!("{got:#?}"))
                        .unwrap();
                    std::fs::write(
                        dump.join(format!("{name}-{file}.want")),
                        format!("{want:#?}"),
                    )
                    .unwrap();
                }
                if got != want {
                    failures.push(format!(
                        "{name} {file}:\n  got:      {got:?}\n  expected: {want:?}"
                    ));
                }
                continue;
            }
            let empty = BTreeMap::new();
            let (got, want) = (
                canonical_with_color_precision(
                    data,
                    ours_names.get(file).unwrap_or(&empty),
                    name.starts_with("Calibrated"),
                ),
                canonical_with_color_precision(
                    &xcode[file],
                    xcode_names.get(file).unwrap_or(&empty),
                    name.starts_with("Calibrated"),
                ),
            );
            if let Some(dump) = std::env::var_os("IBTOOL_DUMP") {
                let dump = Path::new(&dump);
                std::fs::write(dump.join(format!("{name}-{file}.got.nib")), data).unwrap();
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
fn the_version_probe_matches_xcode() {
    let args = ["--version", "--output-format", "xml1"].map(str::to_string);
    assert!(parse_args(&args).unwrap().version);
    assert_eq!(
        String::from_utf8(output::encode(output::Format::Xml, &output::version_data())).unwrap(),
        include_str!("../../../fixtures/ibtool/expected/version.plist")
    );
}

#[test]
fn static_sections_are_connected_before_a_table_queries_its_data_source() {
    fn outlet_labels(data: &[u8]) -> Vec<String> {
        let objects = read::parse(data);
        let reference = |object: usize, key: &str| {
            objects[object].values.iter().find_map(|(k, v)| match v {
                Read::Object(id) if k == key => Some(*id),
                _ => None,
            })
        };
        let connections = reference(0, "UINibConnectionsKey").unwrap();
        objects[connections]
            .values
            .iter()
            .filter_map(|(_, value)| match value {
                Read::Object(id) => reference(*id, "UILabel"),
                _ => None,
            })
            .filter_map(|label| {
                objects[label].values.iter().find_map(|(key, value)| {
                    if let Read::Data(bytes) = value
                        && key == "NS.bytes"
                    {
                        return String::from_utf8(bytes.clone()).ok();
                    }
                    None
                })
            })
            .collect()
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/ibtool");
    let out = TempDir::new("sweetpad-static-table-order");
    compile(
        &fixtures.join("AlamofireMain.storyboard"),
        &Args {
            compilation_directory: Some(out.to_path_buf()),
            module: "App".into(),
            ..Args::default()
        },
    )
    .unwrap();
    let expected = files(&fixtures.join("expected/AlamofireMain.storyboardc"));
    let actual = files(&out.join("AlamofireMain.storyboardc"));
    let mut checked = 0;
    for (name, bytes) in expected.iter().filter(|(name, _)| {
        Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("nib"))
    }) {
        let labels = outlet_labels(bytes);
        if let Some(static_source) = labels.iter().position(|label| label == "staticDataSource") {
            let data_source = labels
                .iter()
                .position(|label| label == "dataSource")
                .unwrap();
            assert!(static_source < data_source, "Xcode's outlet order changed");
            let labels = outlet_labels(&actual[name]);
            assert!(
                labels
                    .iter()
                    .position(|label| label == "staticDataSource")
                    .unwrap()
                    < labels
                        .iter()
                        .position(|label| label == "dataSource")
                        .unwrap(),
                "{name}: connecting dataSource first caches empty section headers"
            );
            checked += 1;
        }
    }
    assert!(checked > 0);
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
    let Err(error) = storyboard::compile(&document, &Options::default()) else {
        panic!("compiled");
    };
    assert!(error.contains("<datePicker>"), "{error}");
}

/// Renumbering a loading boundary is harmless; changing the destination of
/// a connection must still be caught by the corpus comparison.
#[test]
fn proxy_normalization_preserves_connection_destinations() {
    fn archives(reverse_names: bool, reverse_targets: bool) -> BTreeMap<String, Vec<u8>> {
        let mut controller = nib::Graph::default();
        let object = controller.object("UIViewController");
        let name = controller.string("Example-view");
        controller.set(object, "UINibName", name);
        let mut entries = Vec::new();
        for (index, identifier) in ["first", "second"].into_iter().enumerate() {
            let template = controller.object("UIStoryboardShowSegueTemplate");
            let identifier = controller.string(identifier);
            controller.set(template, "UIIdentifier", identifier);
            let number = if reverse_names { 2 - index } else { index + 1 };
            let key = controller.string(&format!("UpstreamPlaceholder-{number}"));
            entries.push((key, template));
        }
        let table = controller.dictionary("NSDictionary", &entries);
        controller.set(object, "UIExternalObjectsTableForViewLoading", table);
        let root = storyboard::nib_root(&mut controller, &[object], &[object], &[], &[]);
        let mut view = nib::Graph::default();
        let mut top = Vec::new();
        for number in 1..=2 {
            top.push(storyboard::proxy(
                &mut view,
                &format!("UpstreamPlaceholder-{number}"),
            ));
        }
        let mut connections = Vec::new();
        let mut objects = top.clone();
        for index in 0..2 {
            let button = view.object("UIButton");
            view.set(
                button,
                "UIContentHorizontalAlignment",
                i64::try_from(index).unwrap(),
            );
            objects.push(button);
            let target = if reverse_targets { 1 - index } else { index };
            connections.push(storyboard::outlet(
                &mut view,
                "selectionSegueTemplate",
                button,
                top[target],
            ));
        }
        let root_view = storyboard::nib_root(&mut view, &top, &objects, &connections, &[]);
        BTreeMap::from([
            ("Example.nib".to_string(), controller.write(root)),
            ("Example-view.nib".to_string(), view.write(root_view)),
        ])
    }
    fn normalized_view(files: &BTreeMap<String, Vec<u8>>) -> String {
        let names = normalize::upstreams(files);
        canonical_with(&files["Example-view.nib"], &names["Example-view.nib"])
    }
    let original = normalized_view(&archives(false, false));
    assert_eq!(original, normalized_view(&archives(true, true)));
    assert_ne!(original, normalized_view(&archives(true, false)));
}
