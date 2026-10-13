use std::path::Path;

use serde_json::{Value, json};

use super::catalog::Catalog;
use super::symbols::{self, Frameworks, Options};
use super::vector::tests::pdf;
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
    let report = compile::Report::of(
        &Catalog::read(&catalog),
        Some("AppIcon"),
        false,
        &std::collections::HashSet::new(),
        &[],
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
/// a namespaced star, a PDF glyph, and a JPEG photo, which the stand-in
/// can't read.
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
    std::fs::write(
        glyph.join("glyph.pdf"),
        pdf([0.0, 0.0, 14.0, 16.0], None, 0),
    )
    .unwrap();
    write_json(
        &glyph.join("Contents.json"),
        &json!({"images": [{"filename": "glyph.pdf", "idiom": "universal"}], "properties": {"preserves-vector-representation": true}}),
    );
    let photo = catalog.join("Photo.imageset");
    std::fs::create_dir_all(&photo).unwrap();
    std::fs::write(photo.join("photo.jpg"), [0xff, 0xd8, 0xff]).unwrap();
    write_json(
        &photo.join("Contents.json"),
        &json!({"images": [{"filename": "photo.jpg", "idiom": "universal"}]}),
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
    // The photo stays out; the star keeps its namespace.
    assert_eq!(
        car_names(&out.join("Assets.car")),
        ["Brand", "Glyph", "Icons/Star", "Logo"]
    );
    let data = std::fs::read(out.join("Assets.car")).unwrap();
    let (renditions, _) = super::car::tests::Store::read(&data).tree("RENDITIONS");
    // Logo: 2x, 3x, iPad 2x and dark 2x; Brand: any and dark; Star: 1x;
    // Glyph: 1x, 2x, 3x and its PDF.
    assert_eq!(renditions.len(), 11);
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
    std::fs::remove_dir_all(catalog.join("Glyph.imageset")).unwrap();
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
            "{prefix}1 image that isn't a PNG, PDF or SVG is missing from the app."
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
            "{prefix}the app icon, 1 color and 3 images that aren't PNGs, PDFs or SVGs are missing from the app."
        ))
    );
}

#[test]
fn the_image_catalog_reports_its_photo() {
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
        .filter(|a| a.name != "Photo" && a.kind == super::catalog::Kind::Image)
        .map(|a| a.dir.as_path())
        .collect();
    let report = compile::Report::of(&read, Some("AppIcon"), false, &compiled, &[]);
    assert_eq!(report.images, 1);
    assert_eq!(report.colors, 0);
}

/// A rendition's key attributes by id, its layout and its size.
type Rendition = (std::collections::BTreeMap<u32, u16>, u16, (u32, u32));

/// Each rendition in a compiled `Assets.car`.
fn renditions(path: &Path) -> Vec<Rendition> {
    let data = std::fs::read(path).unwrap();
    let store = super::car::tests::Store::read(&data);
    let format = store.variable("KEYFORMAT");
    let le = |data: &[u8], at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    let attributes: Vec<u32> = (0..le(format, 8) as usize)
        .map(|i| le(format, 12 + 4 * i))
        .collect();
    let (renditions, _) = store.tree("RENDITIONS");
    renditions
        .iter()
        .map(|(key, value)| {
            let values = key.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
            let key = attributes.iter().copied().zip(values).collect();
            let layout = u16::from_le_bytes([value[36], value[37]]);
            (key, layout, (le(value, 12), le(value, 16)))
        })
        .collect()
}

/// The arguments for compiling `catalog` into `out` for `platform`.
fn platform_args(catalog: &Path, out: &Path, platform: &str) -> Args {
    let command_line: Vec<String> = [
        catalog.display().to_string(),
        "--compile".into(),
        out.display().to_string(),
        "--platform".into(),
        platform.into(),
    ]
    .into();
    Args::parse(&command_line)
}

#[test]
fn vector_images_are_drawn_at_each_scale_the_platform_has() {
    let dir = TempDir::new("sweetpad-actool-vectors");
    let catalog = template_catalog(&dir);
    let set = |name: &str, files: &[(&str, Vec<u8>)], images: Value| {
        let folder = catalog.join(format!("{name}.imageset"));
        std::fs::create_dir_all(&folder).unwrap();
        for (file, bytes) in files {
            std::fs::write(folder.join(file), bytes).unwrap();
        }
        write_json(&folder.join("Contents.json"), &json!({ "images": images }));
    };
    let dark = json!([{"appearance": "luminosity", "value": "dark"}]);
    set(
        "Plain",
        &[
            ("plain.pdf", pdf([0.0, 0.0, 10.0, 6.0], None, 0)),
            ("dark.pdf", pdf([0.0, 0.0, 10.0, 6.0], None, 0)),
        ],
        json!([
            {"filename": "plain.pdf", "idiom": "universal"},
            {"appearances": dark, "filename": "dark.pdf", "idiom": "universal"},
        ]),
    );
    set(
        "Scaled",
        &[
            ("star.pdf", pdf([0.0, 0.0, 13.0, 13.0], None, 0)),
            ("star@2x.pdf", pdf([0.0, 0.0, 26.0, 26.0], None, 0)),
        ],
        json!([
            {"filename": "star.pdf", "idiom": "universal", "scale": "1x"},
            {"filename": "star@2x.pdf", "idiom": "universal", "scale": "2x"},
            {"idiom": "universal", "scale": "3x"},
        ]),
    );
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20.5" height="8"><rect width="20" height="8"/></svg>"#;
    set(
        "Drawing",
        &[("drawing.svg", svg.to_vec())],
        json!([{"filename": "drawing.svg", "idiom": "universal"}]),
    );
    let out = dir.join("ios");
    compile::compile(&platform_args(&catalog, &out, "iphonesimulator")).unwrap();
    let ids: Vec<String> = car_names(&out.join("Assets.car"));
    assert_eq!(ids, ["Drawing", "Plain", "Scaled"]);
    // What Xcode 27's actool wrote: a bitmap per scale, the 3x star from
    // the 2x file, and each set's file for no scale, as its sets don't
    // preserve vector data.
    // Identifier, appearance, scale, layout and size.
    let mut summary: Vec<_> = renditions(&out.join("Assets.car"))
        .into_iter()
        .map(|(key, layout, size)| (key[&17], key[&7], key[&12], layout, size))
        .collect();
    summary.sort_unstable();
    let (drawing, plain, star) = (1, 2, 3);
    assert_eq!(
        summary,
        [
            (drawing, 0, 0, 9, (0, 0)),
            (drawing, 0, 1, 12, (21, 8)),
            (drawing, 0, 2, 12, (41, 16)),
            (drawing, 0, 3, 12, (62, 24)),
            (plain, 0, 0, 9, (0, 0)),
            (plain, 0, 1, 12, (10, 6)),
            (plain, 0, 2, 12, (20, 12)),
            (plain, 0, 3, 12, (30, 18)),
            (plain, 1, 0, 9, (0, 0)),
            (plain, 1, 1, 12, (10, 6)),
            (plain, 1, 2, 12, (20, 12)),
            (plain, 1, 3, 12, (30, 18)),
            (star, 0, 0, 9, (0, 0)),
            (star, 0, 1, 12, (13, 13)),
            (star, 0, 2, 12, (26, 26)),
            (star, 0, 3, 12, (39, 39)),
        ]
    );
    // A Mac has no 3x screens.
    let out = dir.join("mac");
    compile::compile(&platform_args(&catalog, &out, "macosx")).unwrap();
    let scales: std::collections::BTreeSet<u16> = renditions(&out.join("Assets.car"))
        .iter()
        .map(|(key, ..)| key[&12])
        .collect();
    assert_eq!(scales, [0, 1, 2].into());
}

#[test]
fn a_symbol_set_goes_in_as_the_masters_its_template_draws() {
    let dir = TempDir::new("sweetpad-actool-symbol");
    let catalog = template_catalog(&dir);
    let folder = catalog.join("Mark.symbolset");
    std::fs::create_dir_all(&folder).unwrap();
    let template = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/actool/symbols/legacy.svg");
    std::fs::copy(template, folder.join("legacy.svg")).unwrap();
    write_json(
        &folder.join("Contents.json"),
        &json!({"symbols": [{"filename": "legacy.svg", "idiom": "universal"}]}),
    );
    let out = dir.join("out");
    compile::compile(&platform_args(&catalog, &out, "iphonesimulator")).unwrap();
    assert_eq!(car_names(&out.join("Assets.car")), ["Mark"]);
    // Regular at each size, as glyphs at 2019's deployment target.
    let glyphs: Vec<(u16, u16, u16, u16, u16)> = renditions(&out.join("Assets.car"))
        .into_iter()
        .map(|(key, layout, _)| (key[&2], key[&26], key[&27], key[&25], layout))
        .collect();
    assert_eq!(
        glyphs,
        [
            (59, 4, 1, 5, 1017),
            (59, 4, 2, 5, 1017),
            (59, 4, 3, 5, 1017)
        ]
    );
}

#[test]
fn a_set_that_preserves_vector_data_says_so_on_its_bitmaps() {
    let dir = TempDir::new("sweetpad-actool-preserved");
    let catalog = template_catalog(&dir);
    let folder = catalog.join("Logo.imageset");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("logo.png"), png_of(2, [9, 9, 9, 255])).unwrap();
    write_json(
        &folder.join("Contents.json"),
        &json!({"images": [{"filename": "logo.png", "idiom": "universal", "scale": "1x"}], "properties": {"preserves-vector-representation": true}}),
    );
    let out = dir.join("out");
    compile::compile(&platform_args(&catalog, &out, "iphonesimulator")).unwrap();
    let data = std::fs::read(out.join("Assets.car")).unwrap();
    let (renditions, _) = super::car::tests::Store::read(&data).tree("RENDITIONS");
    let flags = u32::from_le_bytes(renditions[0].1[8..12].try_into().unwrap());
    // Bit 8 as Xcode sets it, whatever the set's files are, and the
    // automatic rendering, 0x10.
    assert_eq!(flags, 0x110);
}

#[test]
fn app_icons_go_in_the_car_numbered_by_size() {
    let dir = TempDir::new("sweetpad-actool-car-icons");
    let catalog = template_catalog(&dir);
    let icon = |name: &str, images: &[(&str, &str, &str, u32)]| {
        let set = catalog.join(format!("{name}.appiconset"));
        let mut entries = Vec::new();
        for (idiom, size, scale, pixels) in images {
            let file = format!("{idiom}-{size}@{scale}.png");
            std::fs::create_dir_all(&set).unwrap();
            std::fs::write(set.join(&file), png_of(*pixels, [9, 9, 9, 255])).unwrap();
            entries.push(json!({"filename": file, "idiom": idiom, "size": size, "scale": scale}));
        }
        write_json(&set.join("Contents.json"), &json!({ "images": entries }));
    };
    icon(
        "AppIcon",
        &[
            ("iphone", "60x60", "3x", 180),
            ("ipad", "83.5x83.5", "2x", 167),
            ("ios-marketing", "1024x1024", "1x", 1024),
        ],
    );
    icon("Alt", &[("iphone", "60x60", "2x", 120)]);
    let out = dir.join("out");
    let mut args = compile_args(&catalog, &out, &dir);
    args.flags
        .push(("--alternate-app-icon".into(), Some("Alt".into())));
    compile::compile(&args).unwrap();

    // Idiom 15, subtype 16, scale 12, size index 9; part 220 is an icon
    // image and 218 the list of an idiom's sizes.
    let mut icons: Vec<(u16, u16, u16, u16, u32)> = renditions(&out.join("Assets.car"))
        .into_iter()
        .filter(|(key, _, _)| key[&2] == 220)
        .map(|(key, _, (width, _))| (key[&15], key[&16], key[&12], key[&9], width))
        .collect();
    icons.sort_unstable();
    assert_eq!(
        icons,
        [
            (1, 0, 2, 1, 120),
            (1, 0, 3, 1, 180),
            (1, 1792, 2, 3, 180),
            (2, 0, 2, 2, 167),
            (6, 0, 1, 4, 1024),
        ]
    );
    let lists = renditions(&out.join("Assets.car"))
        .into_iter()
        .filter(|(key, layout, _)| key[&2] == 218 && *layout == 1010)
        .count();
    // AppIcon's iPhone, iPhone XR, iPad and App Store lists, and Alt's.
    assert_eq!(lists, 5);
    let partial = std::fs::read_to_string(dir.join("partial.plist")).unwrap();
    for key in [
        "CFBundlePrimaryIcon",
        "CFBundleAlternateIcons",
        "<key>Alt</key>",
    ] {
        assert_eq!(partial.matches(key).count(), 2, "{key} in {partial}");
    }
}

#[test]
fn a_mac_icon_goes_in_the_car_and_an_icns_file() {
    let dir = TempDir::new("sweetpad-actool-mac-icon");
    let catalog = template_catalog(&dir);
    let set = catalog.join("AppIcon.appiconset");
    std::fs::write(set.join("512.png"), png_of(1024, [9, 9, 9, 255])).unwrap();
    std::fs::write(set.join("16.png"), png_of(16, [9, 9, 9, 255])).unwrap();
    write_json(
        &set.join("Contents.json"),
        &json!({"images": [
            {"filename": "512.png", "idiom": "mac", "scale": "2x", "size": "512x512"},
            {"filename": "16.png", "idiom": "mac", "scale": "1x", "size": "16x16"},
        ]}),
    );
    let out = dir.join("out");
    let mut args = platform_args(&catalog, &out, "macosx");
    args.flags.push((
        "--output-partial-info-plist".into(),
        Some(dir.join("partial.plist").display().to_string()),
    ));
    args.flags
        .push(("--app-icon".into(), Some("AppIcon".into())));
    compile::compile(&args).unwrap();
    let icns = std::fs::read(out.join("AppIcon.icns")).unwrap();
    let kinds: Vec<&[u8]> = [b"ic13", b"ic11", b"ic04", b"ic07"]
        .iter()
        .filter(|kind| icns.windows(4).any(|w| w == kind.as_slice()))
        .map(|kind| kind.as_slice())
        .collect();
    assert_eq!(kinds.len(), 4);
    let partial = std::fs::read_to_string(dir.join("partial.plist")).unwrap();
    for key in ["CFBundleIconFile", "CFBundleIconName"] {
        assert!(partial.contains(key), "{key} in {partial}");
    }
    let sizes: Vec<(u16, u32)> = renditions(&out.join("Assets.car"))
        .into_iter()
        .filter(|(key, _, _)| key[&2] == 220)
        .map(|(key, _, (width, _))| (key[&9], width))
        .collect();
    assert_eq!(sizes.len(), 2);
    assert!(
        sizes.contains(&(1, 16)) && sizes.contains(&(2, 1024)),
        "{sizes:?}"
    );
}
