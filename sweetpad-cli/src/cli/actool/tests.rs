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

fn compile_args(catalog: &Path, out: &Path, dir: &Path) -> Args {
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
        "--target-device".into(),
        "iphone".into(),
        "--target-device".into(),
        "ipad".into(),
        "--platform".into(),
        "iphonesimulator".into(),
    ]
    .into();
    Args::parse(&command_line)
}

fn png_of(width: u32, rgba: [u8; 4]) -> Vec<u8> {
    raster::encode(&raster::Rgba {
        width,
        height: width,
        pixels: rgba.repeat((width * width) as usize),
    })
    .unwrap()
}

fn pixel_size(path: &Path) -> (u32, u32) {
    let image = raster::decode(&std::fs::read(path).unwrap()).unwrap();
    (image.width, image.height)
}

#[test]
fn the_template_catalog_has_nothing_missing() {
    let dir = TempDir::new("sweetpad-actool-template");
    let catalog = template_catalog(&dir);
    let args = compile_args(&catalog, &dir.join("out"), &dir);
    let report = compile::Report::of(
        &Catalog::read(&catalog),
        Some("AppIcon"),
        false,
        &std::collections::HashSet::new(),
        &args,
    );
    assert_eq!(report, compile::Report::default());
    assert_eq!(report.warning(), None);
    let generated = symbols::generate(
        &[Catalog::read(&catalog)],
        &options(true, Frameworks::default()),
    );
    assert!(
        !generated.swift.contains("static let"),
        "{}",
        generated.swift
    );
    assert!(!generated.objc.contains("ACBundleID"), "{}", generated.objc);
}

#[test]
fn a_compile_writes_what_swift_build_links() {
    let dir = TempDir::new("sweetpad-actool-compile");
    let catalog = template_catalog(&dir);
    let out = dir.join("out");
    let args = compile_args(&catalog, &out, &dir);
    assert_eq!(args.inputs, vec![catalog.clone()]);
    assert_eq!(args.value("--enable-on-demand-resources"), Some("YES"));
    compile::compile(&args).unwrap();
    assert!(out.is_dir());
    let partial = std::fs::read_to_string(dir.join("partial.plist")).unwrap();
    assert!(partial.contains("<dict/>"), "{partial}");
    let deps = std::fs::read(dir.join("deps")).unwrap();
    assert_eq!(deps[0], 0x00);
    let input = [&[0x10][..], catalog.display().to_string().as_bytes(), &[0]].concat();
    assert!(deps.windows(input.len()).any(|w| w == input));
}

#[test]
fn the_app_icon_goes_in_at_both_ios_sizes() {
    let dir = TempDir::new("sweetpad-actool-icon");
    let catalog = template_catalog(&dir);
    let set = catalog.join("AppIcon.appiconset");
    std::fs::write(set.join("icon.png"), png_of(1024, [240, 120, 30, 255])).unwrap();
    write_json(
        &set.join("Contents.json"),
        &json!({"images": [
            {"filename": "icon.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"},
            {"appearances": [{"appearance": "luminosity", "value": "dark"}], "idiom": "universal", "platform": "ios", "size": "1024x1024"},
        ]}),
    );
    let out = dir.join("out");
    compile::compile(&compile_args(&catalog, &out, &dir)).unwrap();
    assert_eq!(pixel_size(&out.join("AppIcon60x60@2x.png")), (120, 120));
    assert_eq!(
        pixel_size(&out.join("AppIcon76x76@2x~ipad.png")),
        (152, 152)
    );
    let partial = std::fs::read_to_string(dir.join("partial.plist")).unwrap();
    for key in [
        "CFBundleIcons",
        "CFBundleIcons~ipad",
        "CFBundleIconName",
        "AppIcon60x60",
        "AppIcon76x76",
    ] {
        assert!(partial.contains(key), "{key} in {partial}");
    }
}

#[test]
fn an_icon_of_the_exact_size_goes_in_as_it_is() {
    let dir = TempDir::new("sweetpad-actool-icon-sizes");
    let catalog = template_catalog(&dir);
    let set = catalog.join("AppIcon.appiconset");
    let (iphone, ipad) = (png_of(120, [1, 2, 3, 255]), png_of(152, [4, 5, 6, 255]));
    std::fs::write(set.join("60.png"), &iphone).unwrap();
    std::fs::write(set.join("76.png"), &ipad).unwrap();
    write_json(
        &set.join("Contents.json"),
        &json!({"images": [
            {"filename": "60.png", "idiom": "iphone", "scale": "2x", "size": "60x60"},
            {"filename": "76.png", "idiom": "ipad", "scale": "2x", "size": "76x76"},
        ]}),
    );
    let out = dir.join("out");
    compile::compile(&compile_args(&catalog, &out, &dir)).unwrap();
    assert_eq!(
        std::fs::read(out.join("AppIcon60x60@2x.png")).unwrap(),
        iphone
    );
    assert_eq!(
        std::fs::read(out.join("AppIcon76x76@2x~ipad.png")).unwrap(),
        ipad
    );
}

/// A catalog with a logo at two scales and an iPad variant, a dark variant,
/// a namespaced star and a PDF glyph.
fn image_catalog(dir: &Path) -> PathBuf {
    let catalog = template_catalog(dir);
    let logo = catalog.join("Logo.imageset");
    std::fs::create_dir_all(&logo).unwrap();
    for name in ["2x.png", "3x.png", "pad.png", "dark.png"] {
        std::fs::write(logo.join(name), png_of(2, [9, 9, 9, 255])).unwrap();
    }
    write_json(
        &logo.join("Contents.json"),
        &json!({"images": [
            {"idiom": "universal", "scale": "1x"},
            {"filename": "2x.png", "idiom": "universal", "scale": "2x"},
            {"filename": "3x.png", "idiom": "universal", "scale": "3x"},
            {"filename": "pad.png", "idiom": "ipad", "scale": "2x"},
            {"appearances": [{"appearance": "luminosity", "value": "dark"}], "filename": "dark.png", "idiom": "universal", "scale": "2x"},
        ]}),
    );
    write_json(
        &catalog.join("Icons/Contents.json"),
        &json!({"properties": {"provides-namespace": true}}),
    );
    let star = catalog.join("Icons/Star.imageset");
    std::fs::create_dir_all(&star).unwrap();
    std::fs::write(star.join("star.PNG"), png_of(2, [7, 7, 7, 255])).unwrap();
    write_json(
        &star.join("Contents.json"),
        &json!({"images": [{"filename": "star.PNG", "idiom": "universal", "scale": "1x"}]}),
    );
    let glyph = catalog.join("Glyph.imageset");
    std::fs::create_dir_all(&glyph).unwrap();
    std::fs::write(glyph.join("glyph.pdf"), b"%PDF-1.4").unwrap();
    write_json(
        &glyph.join("Contents.json"),
        &json!({"images": [{"filename": "glyph.pdf", "idiom": "universal"}], "properties": {"preserves-vector-representation": true}}),
    );
    catalog
}

/// The names of the assets in a compiled `Assets.car`.
fn car_names(path: &Path) -> Vec<String> {
    let data = std::fs::read(path).unwrap();
    let (facets, _) = super::car::tests::Store::read(&data).tree("FACETKEYS");
    facets
        .iter()
        .map(|(name, _)| String::from_utf8(name.to_vec()).unwrap())
        .collect()
}

#[test]
fn colors_and_images_go_in_assets_car() {
    let dir = TempDir::new("sweetpad-actool-images");
    let catalog = image_catalog(&dir);
    let brand = catalog.join("Brand.colorset");
    std::fs::create_dir_all(&brand).unwrap();
    write_json(
        &brand.join("Contents.json"),
        &json!({"colors": [
            {"color": {"color-space": "srgb", "components": {"red": "0.9", "green": "0x33", "blue": "102", "alpha": "1.000"}}, "idiom": "universal"},
            {"appearances": [{"appearance": "luminosity", "value": "dark"}], "color": {"color-space": "display-p3", "components": {"red": "0.1", "green": "0.2", "blue": "0.3", "alpha": "1"}}, "idiom": "universal"},
        ]}),
    );
    let out = dir.join("out");
    let args = compile_args(&catalog, &out, &dir);
    compile::compile(&args).unwrap();
    let mut files: Vec<String> = walk(&out)
        .iter()
        .map(|f| f.strip_prefix(&out).unwrap().display().to_string())
        .collect();
    files.sort();
    assert_eq!(files, ["Assets.car"]);
    // The PDF glyph stays out; the star keeps its namespace.
    assert_eq!(
        car_names(&out.join("Assets.car")),
        ["Brand", "Icons/Star", "Logo"]
    );
    let data = std::fs::read(out.join("Assets.car")).unwrap();
    let (renditions, _) = super::car::tests::Store::read(&data).tree("RENDITIONS");
    // Logo: 2x, 3x, iPad 2x and dark 2x; Brand: any and dark; Star: 1x.
    assert_eq!(renditions.len(), 7);
}

#[test]
fn a_components_spelling_decides_how_it_reads() {
    let srgb = |components: Value| {
        compile::color(&json!({"color-space": "srgb", "components": components}))
    };
    let (space, values) =
        srgb(json!({"red": "0.1", "green": "0x80", "blue": "255", "alpha": "1"})).unwrap();
    assert_eq!(space, super::car::ColorSpace::Srgb);
    assert_eq!(values, [f64::from(0.1f32), 128.0 / 255.0, 1.0, 1.0]);
    let gray = compile::color(
        &json!({"color-space": "extended-gray", "components": {"white": "1.5", "alpha": "0.5"}}),
    );
    assert_eq!(
        gray,
        Some((super::car::ColorSpace::ExtendedGray, vec![1.5, 0.5]))
    );
    assert_eq!(
        compile::color(&json!({"platform": "ios", "reference": "systemBlueColor"})),
        None
    );
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn a_second_compile_takes_out_what_the_catalog_dropped() {
    let dir = TempDir::new("sweetpad-actool-recompile");
    let catalog = image_catalog(&dir);
    let out = dir.join("out");
    let args = compile_args(&catalog, &out, &dir);
    compile::compile(&args).unwrap();
    std::fs::remove_dir_all(catalog.join("Icons")).unwrap();
    compile::compile(&args).unwrap();
    assert_eq!(car_names(&out.join("Assets.car")), ["Logo"]);
    std::fs::remove_dir_all(catalog.join("Logo.imageset")).unwrap();
    compile::compile(&args).unwrap();
    assert!(!out.join("Assets.car").exists());
}

#[test]
fn the_warning_says_what_the_app_goes_without() {
    use compile::Report;
    let text = |report: Report| report.warning();
    let prefix = "Linux builds don't compile everything in an asset catalog yet, so ";
    assert_eq!(text(Report::default()), None);
    assert_eq!(
        text(Report {
            images: 1,
            ..Report::default()
        }),
        Some(format!(
            "{prefix}1 image that isn't a PNG is missing from the app."
        ))
    );
    assert_eq!(
        text(Report {
            colors: 2,
            ..Report::default()
        }),
        Some(format!("{prefix}2 colors are missing from the app."))
    );
    assert_eq!(
        text(Report {
            icon_missing: true,
            colors: 1,
            images: 3,
            ..Report::default()
        }),
        Some(format!(
            "{prefix}the app icon, 1 color and 3 images that aren't PNGs are missing from the app."
        ))
    );
}

#[test]
fn the_image_catalog_reports_its_vector_image() {
    let dir = TempDir::new("sweetpad-actool-report");
    let catalog = image_catalog(&dir);
    let out = dir.join("out");
    let args = compile_args(&catalog, &out, &dir);
    compile::compile(&args).unwrap();
    let read = Catalog::read(&catalog);
    let compiled: std::collections::HashSet<&Path> = read
        .assets()
        .into_iter()
        .map(|(_, a)| a)
        .filter(|a| a.name != "Glyph" && a.kind == super::catalog::Kind::Image)
        .map(|a| a.dir.as_path())
        .collect();
    let report = compile::Report::of(&read, Some("AppIcon"), false, &compiled, &args);
    assert_eq!(report.images, 1);
    assert_eq!(report.colors, 0);
}
