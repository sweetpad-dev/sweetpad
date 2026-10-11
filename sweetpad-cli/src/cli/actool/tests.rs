use std::path::Path;

use serde_json::{Value, json};

use super::catalog::Catalog;
use super::symbols::{self, Frameworks, Options};
use super::*;
use crate::cli::testdir::TempDir;

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/actool")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn write_json(path: &Path, value: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

/// The catalog `probe.json` describes, built as `regen.py` builds it.
/// Images name a file without one: the stand-in only reads `Contents.json`.
fn probe_catalog(dir: &Path) -> Catalog {
    let spec: Value = serde_json::from_str(&fixture("probe.json")).unwrap();
    let catalog = dir.join("Probe.xcassets");
    let info = json!({"info": {"author": "xcode", "version": 1}});
    let namespaces: Vec<&str> = spec["namespaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    write_json(&catalog.join("Contents.json"), &info);
    for asset in spec["assets"].as_array().unwrap() {
        let path = asset["path"].as_str().unwrap();
        let parts: Vec<&str> = path.split('/').collect();
        for depth in 1..parts.len() {
            let folder = parts[..depth].join("/");
            let mut contents = info.clone();
            if namespaces.contains(&folder.as_str()) {
                contents["properties"] = json!({"provides-namespace": true});
            }
            write_json(&catalog.join(&folder).join("Contents.json"), &contents);
        }
        let idioms: Vec<&str> = asset["idioms"].as_array().map_or_else(
            || vec!["universal"],
            |a| a.iter().map(|i| i.as_str().unwrap()).collect(),
        );
        let empty = asset["empty"].as_bool().unwrap_or(false);
        let (folder, key, entries): (String, &str, Vec<Value>) = match asset["kind"]
            .as_str()
            .unwrap()
        {
            "color" => (
                format!("{path}.colorset"),
                "colors",
                idioms
                    .iter()
                    .map(|i| {
                        if empty {
                            json!({"idiom": i})
                        } else {
                            json!({"color": {"color-space": "srgb"}, "idiom": i})
                        }
                    })
                    .collect(),
            ),
            "image" => (
                format!("{path}.imageset"),
                "images",
                idioms
                    .iter()
                    .map(|i| {
                        if empty {
                            json!({"idiom": i, "scale": "1x"})
                        } else {
                            json!({"filename": "img.png", "idiom": i, "scale": "1x"})
                        }
                    })
                    .collect(),
            ),
            "data" => (
                format!("{path}.dataset"),
                "data",
                vec![json!({"filename": "blob.bin", "idiom": "universal"})],
            ),
            "appicon" => (
                format!("{path}.appiconset"),
                "images",
                vec![
                    json!({"filename": "icon.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"}),
                ],
            ),
            other => panic!("unknown asset kind {other}"),
        };
        let mut contents = info.clone();
        contents[key] = Value::Array(entries);
        write_json(&catalog.join(folder).join("Contents.json"), &contents);
    }
    Catalog::read(&catalog)
}

fn options(extensions: bool, frameworks: Frameworks) -> Options {
    Options {
        bundle_id: "dev.sweetpad.Probe".into(),
        extensions,
        frameworks,
        target_devices: vec!["iphone".into(), "ipad".into()],
    }
}

/// The first line where `actual` and `expected` part, for a readable failure.
fn first_difference(actual: &str, expected: &str) -> String {
    for (i, (a, e)) in actual.lines().zip(expected.lines()).enumerate() {
        if a != e {
            return format!("line {}:\n  got:      {a}\n  expected: {e}", i + 1);
        }
    }
    format!(
        "one is a prefix of the other ({} vs {} lines)",
        actual.lines().count(),
        expected.lines().count()
    )
}

fn assert_same(actual: &str, expected: &str, what: &str) {
    assert!(
        actual == expected,
        "{what} differs from actool's: {}",
        first_difference(actual, expected)
    );
}

#[test]
fn symbols_match_actool_in_every_mode() {
    let dir = TempDir::new("sweetpad-actool-probe");
    let catalog = probe_catalog(&dir);
    let catalogs = [catalog];
    let swiftui = Frameworks {
        appkit: false,
        uikit: false,
        swiftui: true,
    };
    for (mode, opts) in [
        ("extensions", options(true, Frameworks::default())),
        ("no-extensions", options(false, Frameworks::default())),
        ("swiftui", options(true, swiftui)),
    ] {
        let generated = symbols::generate(&catalogs, &opts);
        assert_same(
            &generated.swift,
            &fixture(&format!("expected/{mode}.swift")),
            &format!("{mode}.swift"),
        );
        assert_same(
            &generated.objc,
            &fixture("expected/symbols.h"),
            &format!("{mode} header"),
        );
    }
}

#[test]
fn the_index_matches_actool() {
    let dir = TempDir::new("sweetpad-actool-index");
    let catalog = probe_catalog(&dir);
    let path = catalog.path.display().to_string();
    let generated = symbols::generate(&[catalog], &options(true, Frameworks::default()));
    let mut expected: Value = serde_json::from_str(&fixture("expected/index.json")).unwrap();
    for entries in expected.as_object_mut().unwrap().values_mut() {
        for entry in entries.as_array_mut().unwrap() {
            entry["catalogPath"] = json!(path);
        }
    }
    for key in ["colors", "images", "symbols"] {
        assert_eq!(generated.index[key], expected[key], "{key}");
    }
}

/// Xcode's app template: an AccentColor without a value and an AppIcon
/// without images, which actool compiles to nothing.
fn template_catalog(dir: &Path) -> PathBuf {
    let catalog = dir.join("Assets.xcassets");
    let info = json!({"info": {"author": "xcode", "version": 1}});
    write_json(&catalog.join("Contents.json"), &info);
    write_json(
        &catalog.join("AccentColor.colorset/Contents.json"),
        &json!({"colors": [{"idiom": "universal"}], "info": info["info"]}),
    );
    write_json(
        &catalog.join("AppIcon.appiconset/Contents.json"),
        &json!({"images": [{"idiom": "universal", "platform": "ios", "size": "1024x1024"}], "info": info["info"]}),
    );
    catalog
}

#[test]
fn the_template_catalog_has_nothing_missing() {
    let dir = TempDir::new("sweetpad-actool-template");
    let catalog = Catalog::read(&template_catalog(&dir));
    assert_eq!(missing(&catalog), None);
    let generated = symbols::generate(&[catalog], &options(true, Frameworks::default()));
    assert!(
        !generated.swift.contains("static let"),
        "{}",
        generated.swift
    );
    assert!(!generated.objc.contains("ACBundleID"), "{}", generated.objc);
}

#[test]
fn a_catalog_with_assets_names_what_the_app_goes_without() {
    let dir = TempDir::new("sweetpad-actool-missing");
    let catalog = probe_catalog(&dir);
    let (text, plural) = missing(&catalog).unwrap();
    assert_eq!(
        text,
        "the app icon, 167 colors, 23 images and 1 other asset"
    );
    assert!(plural);
}

#[test]
fn a_compile_writes_what_swift_build_links() {
    let dir = TempDir::new("sweetpad-actool-compile");
    let catalog = template_catalog(&dir);
    let out = dir.join("out");
    let command_line: Vec<String> = [
        catalog.display().to_string(),
        "--compile".into(),
        out.display().to_string(),
        "--output-format".into(),
        "human-readable-text".into(),
        "--notices".into(),
        "--warnings".into(),
        "--export-dependency-info".into(),
        dir.join("deps").display().to_string(),
        "--output-partial-info-plist".into(),
        dir.join("partial.plist").display().to_string(),
        "--app-icon".into(),
        "AppIcon".into(),
        "--compress-pngs".into(),
        "--enable-on-demand-resources".into(),
        "YES".into(),
    ]
    .into();
    let args = Args::parse(&command_line);
    assert_eq!(args.inputs, vec![catalog.clone()]);
    assert_eq!(args.value("--enable-on-demand-resources"), Some("YES"));
    compile(&args).unwrap();
    assert!(out.is_dir());
    let partial = std::fs::read_to_string(dir.join("partial.plist")).unwrap();
    assert!(partial.contains("<dict/>"), "{partial}");
    let deps = std::fs::read(dir.join("deps")).unwrap();
    assert_eq!(deps[0], 0x00);
    let input = [&[0x10][..], catalog.display().to_string().as_bytes(), &[0]].concat();
    assert!(deps.windows(input.len()).any(|w| w == input));
}
