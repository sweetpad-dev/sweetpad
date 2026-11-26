use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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
        api: symbols::ResourceApi::System,
        mac_catalyst: false,
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
    let back_deployed: Value =
        serde_json::from_str(&fixture("expected/back-deployed-sha256.json")).unwrap();
    for (mode, mut opts) in [
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
        opts.api = symbols::ResourceApi::BackDeployed;
        let swift = symbols::generate(&catalogs, &opts).swift;
        assert_eq!(
            format!("{:x}", Sha256::digest(swift.as_bytes())),
            back_deployed[mode].as_str().unwrap(),
            "iOS 15.6 {mode} output differs from Xcode's actool",
        );
    }
}

#[test]
fn catalyst_keeps_ipad_only_assets_as_thinnable_resources() {
    let dir = TempDir::new("sweetpad-actool-catalyst");
    let catalog = probe_catalog(&dir);
    let mut opts = options(true, Frameworks::default());
    opts.target_devices = vec!["mac".into()];
    opts.mac_catalyst = true;
    for api in [
        symbols::ResourceApi::BackDeployed,
        symbols::ResourceApi::System,
    ] {
        opts.api = api;
        let swift = symbols::generate(std::slice::from_ref(&catalog), &opts).swift;
        assert!(swift.contains(r#"ImageResource(thinnableName: "PadOnly""#));
        assert!(swift.contains(r#"ColorResource(name: "MacOnly""#));
        assert!(!swift.contains("PhoneOnly"));
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

#[test]
fn a_missing_requested_app_icon_fails_but_an_empty_declared_set_is_valid() {
    let dir = TempDir::new("sweetpad-actool-missing-icon");
    let catalog = template_catalog(&dir);
    let args = compile_args(&catalog, &dir.join("out"), &dir);
    compile::compile(&args).unwrap();
    std::fs::remove_dir_all(catalog.join("AppIcon.appiconset")).unwrap();
    let error = compile::compile(&args).unwrap_err();
    assert!(error.contains("named \"AppIcon\""), "{error}");
}

#[test]
fn named_accent_and_widget_colors_supply_bundle_metadata_only_when_declared() {
    let dir = TempDir::new("sweetpad-actool-color-info");
    let catalog = template_catalog(&dir);
    let mut args = compile_args(&catalog, &dir.join("out"), &dir);
    args.flags.extend([
        ("--accent-color".into(), Some("AccentColor".into())),
        (
            "--widget-background-color".into(),
            Some("AccentColor".into()),
        ),
    ]);
    compile::compile(&args).unwrap();
    assert!(
        !std::fs::read_to_string(dir.join("partial.plist"))
            .unwrap()
            .contains("NSAccentColorName")
    );
    write_json(
        &catalog.join("AccentColor.colorset/Contents.json"),
        &json!({"colors": [{
            "idiom": "universal", "color": {"color-space": "srgb", "components": {
                "red": "0.25", "green": "0.50", "blue": "0.75", "alpha": "1.0"
            }}
        }]}),
    );
    compile::compile(&args).unwrap();
    let info = std::fs::read_to_string(dir.join("partial.plist")).unwrap();
    assert!(info.contains("<key>NSAccentColorName</key>\n\t<string>AccentColor</string>"));
    assert!(
        info.contains("<key>NSWidgetBackgroundColorName</key>\n\t<string>AccentColor</string>")
    );
    std::fs::remove_dir_all(catalog.join("AccentColor.colorset")).unwrap();
    write_json(
        &catalog.join("Scoped/Contents.json"),
        &json!({"properties": {"provides-namespace": true}}),
    );
    write_json(
        &catalog.join("Scoped/AccentColor.colorset/Contents.json"),
        &json!({"colors": [{
            "idiom": "universal", "color": {"color-space": "srgb", "components": {
                "red": "0.25", "green": "0.50", "blue": "0.75", "alpha": "1.0"
            }}
        }]}),
    );
    compile::compile(&args).unwrap();
    let info = std::fs::read_to_string(dir.join("partial.plist")).unwrap();
    assert!(!info.contains("NSAccentColorName"));
    assert!(!info.contains("NSWidgetBackgroundColorName"));
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

fn copy_resource_fixture(source: &Path, destination: &Path) {
    if source.is_dir() {
        std::fs::create_dir_all(destination).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            copy_resource_fixture(&entry.path(), &destination.join(entry.file_name()));
        }
    } else {
        std::fs::copy(source, destination).unwrap();
    }
}

fn resources() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/actool/resources")
}

#[test]
fn sticker_metadata_order_and_files_match_native_actool_and_deleted_members_disappear() {
    let dir = TempDir::new("sweetpad-stickers");
    let catalog = dir.join("Stickers.xcassets");
    copy_resource_fixture(&resources().join("Stickers.xcassets"), &catalog);
    let out = dir.join("out");
    let mut args = compile_args(&catalog, &out, &dir);
    for (key, value) in &mut args.flags {
        if key == "--app-icon" {
            *value = Some("MessageIcon".into());
        }
        if key == "--output-partial-info-plist" {
            *value = Some(out.join("partial.plist").display().to_string());
        }
    }
    args.flags.extend([
        ("--include-sticker-content".into(), None),
        ("--stickers-icon-role".into(), Some("extension".into())),
        (
            "--sticker-pack-identifier-prefix".into(),
            Some("dev.sweetpad.Fixture.sticker-pack".into()),
        ),
        (
            "--bundle-identifier".into(),
            Some("dev.sweetpad.Fixture".into()),
        ),
    ]);
    compile::compile(&args).unwrap();
    let expected: Value = serde_json::from_str(&fixture("expected/stickers.json")).unwrap();
    for (path, key) in [
        ("partial.plist", "partialInfo"),
        ("Ordered Pack.stickerpack/Info.plist", "packInfo"),
    ] {
        assert_same(
            &std::fs::read_to_string(out.join(path)).unwrap(),
            &xml_plist::of(&expected[key]),
            path,
        );
    }
    let mut files: Vec<_> = walk(&out)
        .into_iter()
        .map(|p| p.strip_prefix(&out).unwrap().display().to_string())
        .collect();
    files.sort();
    assert_eq!(json!(files), expected["files"]);
    let car = out.join("Assets.car");
    assert_eq!(json!(car_names(&car)), expected["carNames"]);
    assert_eq!(rendition_listing(&car), sorted(&expected["carRenditions"]));
    for (input, output) in [
        ("First.sticker/source.png", "First.png"),
        ("Second.sticker/animated.gif", "Second.gif"),
    ] {
        assert_eq!(
            std::fs::read(catalog.join("Ordered Pack.stickerpack").join(input)).unwrap(),
            std::fs::read(out.join("Ordered Pack.stickerpack").join(output)).unwrap()
        );
    }
    let pack = catalog.join("Ordered Pack.stickerpack");
    let mut contents: Value =
        serde_json::from_slice(&std::fs::read(pack.join("Contents.json")).unwrap()).unwrap();
    contents["stickers"].as_array_mut().unwrap().pop();
    write_json(&pack.join("Contents.json"), &contents);
    std::fs::remove_dir_all(pack.join("First.sticker")).unwrap();
    compile::compile(&args).unwrap();
    assert!(!out.join("Ordered Pack.stickerpack/First.png").exists());
    assert!(out.join("Ordered Pack.stickerpack/Second.gif").is_file());
}

#[test]
fn modern_icons_match_native_metadata_appearance_sizes_and_layer_order() {
    let dir = TempDir::new("sweetpad-modern-icon");
    let out = dir.join("out");
    let mut args = compile_args(&resources().join("Simple.icon"), &out, &dir);
    args.inputs.push(resources().join("Shared.xcassets"));
    for (key, value) in &mut args.flags {
        if key == "--app-icon" {
            *value = Some("Simple".into());
        }
        if key == "--output-partial-info-plist" {
            *value = Some(out.join("partial.plist").display().to_string());
        }
    }
    compile::compile(&args).unwrap();
    let expected: Value = serde_json::from_str(&fixture("expected/modern.json")).unwrap();
    assert_same(
        &std::fs::read_to_string(out.join("partial.plist")).unwrap(),
        &xml_plist::of(&expected["partialInfo"]),
        "modern icon metadata",
    );
    let mut icons: Vec<_> = renditions(&out.join("Assets.car"))
        .into_iter()
        .filter(|(key, _, _)| key[&2] == 220)
        .map(|(key, _, (w, h))| (key[&15], key[&7], w, h))
        .collect();
    icons.sort_unstable();
    assert_eq!(json!(icons), expected["staticIconRenditions"]);
    let records = renditions(&out.join("Assets.car"));
    let icon_id = records.iter().find(|(key, _, _)| key[&2] == 220).unwrap().0[&17];
    assert!(
        records
            .iter()
            .any(|(key, _, _)| key[&2] == 181 && key[&17] == icon_id),
        "same-named icon and imageset must remain available under one native identifier"
    );
    let reference = raster::decode(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/actool/expected/modern-light.png"),
        )
        .unwrap(),
    )
    .unwrap();
    let actual = raster::decode(&std::fs::read(out.join("Simple60x60@2x.png")).unwrap()).unwrap();
    assert_eq!(
        (actual.width, actual.height),
        (reference.width, reference.height)
    );
    let center = ((actual.height / 2 * actual.width + actual.width / 2) * 4) as usize;
    let dominant = |pixels: &[u8]| (0..3).max_by_key(|i| pixels[center + i]).unwrap();
    assert_eq!(
        dominant(&actual.pixels),
        dominant(&reference.pixels),
        "frontmost layer must match native ordering"
    );
    // Native still emits both icon idioms and loose PNGs for an iPhone-only
    // target. Only CFBundleIconName follows the requested device family.
    args.flags
        .retain(|(flag, value)| !(flag == "--target-device" && value.as_deref() == Some("ipad")));
    compile::compile(&args).unwrap();
    assert!(out.join("Simple76x76@2x~ipad.png").is_file());
    let mut phone_info = expected["partialInfo"].clone();
    phone_info["CFBundleIcons~ipad"]["CFBundlePrimaryIcon"]
        .as_object_mut()
        .unwrap()
        .remove("CFBundleIconName");
    assert_same(
        &std::fs::read_to_string(out.join("partial.plist")).unwrap(),
        &xml_plist::of(&phone_info),
        "iPhone-only modern icon metadata",
    );
    // A .icon input remains an icon asset even when it is not the primary
    // icon, as in Mastodon's action and intent extensions.
    args.flags.retain(|(flag, _)| flag != "--app-icon");
    compile::compile(&args).unwrap();
    assert_eq!(
        renditions(&out.join("Assets.car"))
            .into_iter()
            .filter(|(key, _, _)| key[&2] == 220)
            .count(),
        6
    );
    assert!(!out.join("Simple60x60@2x.png").exists());
    assert!(
        std::fs::read_to_string(out.join("partial.plist"))
            .unwrap()
            .contains("<dict/>")
    );
}

#[test]
fn modern_system_fills_preserve_original_layers_and_supply_all_appearances() {
    let dir = TempDir::new("sweetpad-modern-system-fill");
    let out = dir.join("out");
    let mut args = compile_args(&resources().join("System.icon"), &out, &dir);
    for (key, value) in &mut args.flags {
        if key == "--app-icon" {
            *value = Some("System".into());
        }
    }
    compile::compile(&args).unwrap();
    let expected: Value = serde_json::from_str(&fixture("expected/system.json")).unwrap();
    assert_same(
        &std::fs::read_to_string(dir.join("partial.plist")).unwrap(),
        &xml_plist::of(&expected["partialInfo"]),
        "system icon metadata",
    );
    let mut icons: Vec<_> = renditions(&out.join("Assets.car"))
        .into_iter()
        .filter(|(key, _, _)| key[&2] == 220)
        .map(|(key, _, (w, h))| (key[&15], key[&7], w, h))
        .collect();
    icons.sort_unstable();
    assert_eq!(json!(icons), expected["staticIconRenditions"]);
    // Native "none"/"automatic" layer fills retain the original artwork.
    // A transparent replacement would leave only the gray system background.
    for path in [
        out.join("System60x60@2x.png"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/actool/expected/system-light.png"),
    ] {
        let image = raster::decode(&std::fs::read(path).unwrap()).unwrap();
        let at = ((image.height / 2 * image.width + image.width / 2) * 4) as usize;
        assert!(
            u16::from(image.pixels[at]) > u16::from(image.pixels[at + 2]) + 50,
            "original red artwork: {:?}",
            &image.pixels[at..at + 4]
        );
    }
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
/// a namespaced star, a PDF glyph, and a GIF photo, which the stand-in
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
    std::fs::write(photo.join("photo.gif"), b"GIF89a").unwrap();
    write_json(
        &photo.join("Contents.json"),
        &json!({"images": [{"filename": "photo.gif", "idiom": "universal"}]}),
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
            "{prefix}1 image that isn't a PNG, JPEG, HEIF, PDF or SVG is missing from the app."
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
            "{prefix}the app icon, 1 color and 3 images that aren't PNGs, JPEGs, HEIFs, PDFs or SVGs are missing from the app."
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
fn image_direction_keys_and_mirroring_follow_the_declared_slots() {
    let temp = TempDir::new("sweetpad-actool-direction");
    let catalog = temp.join("Direction.xcassets");
    let set = catalog.join("Arrow.imageset");
    std::fs::create_dir_all(&set).unwrap();
    std::fs::write(set.join("arrow.png"), png_of(8, [0, 0, 255, 255])).unwrap();
    let fixed = json!({"filename": "arrow.png", "scale": "2x"});
    let left =
        json!({"filename": "arrow.png", "scale": "1x", "language-direction": "left-to-right"});
    let right =
        json!({"filename": "arrow.png", "scale": "2x", "language-direction": "right-to-left"});
    let empty_right = json!({"scale": "2x", "language-direction": "right-to-left"});
    let invalid = json!({"filename": "arrow.png", "language-direction": "unknown"});
    for (entries, expected) in [
        (json!([fixed]), vec![(0, false)]),
        (json!([left, invalid]), vec![(5, true)]),
        (json!([right]), vec![(4, true)]),
        (json!([left, right]), vec![(4, false), (5, false)]),
        (json!([left, empty_right]), vec![(5, false)]),
        (json!([fixed, left]), vec![(0, false), (0, false)]),
    ] {
        write_json(&set.join("Contents.json"), &json!({"images": entries}));
        for platform in [
            "iphonesimulator",
            "macosx",
            "watchsimulator",
            "appletvsimulator",
            "xrsimulator",
        ] {
            let out = temp.join(platform);
            compile::compile(&platform_args(&catalog, &out, platform)).unwrap();
            let data = std::fs::read(out.join("Assets.car")).unwrap();
            let store = super::car::tests::Store::read(&data);
            let direction = store.variable("KEYFORMAT")[12..]
                .chunks_exact(4)
                .position(|attr| attr == 4u32.to_le_bytes());
            assert_eq!(
                direction,
                expected.iter().any(|&(d, _)| d != 0).then_some(5),
                "{platform}: {entries}"
            );
            let (records, _) = store.tree("RENDITIONS");
            let mut actual: Vec<_> = records
                .iter()
                .map(|(key, value)| {
                    let code =
                        direction.map_or(0, |i| u16::from_le_bytes([key[2 * i], key[2 * i + 1]]));
                    (code, value[8] & 0x40 != 0)
                })
                .collect();
            actual.sort_unstable();
            assert_eq!(actual, expected, "{platform}: {entries}");
        }
    }
}

#[test]
fn only_the_size_class_axes_used_by_targeted_images_widen_the_catalog() {
    let temp = TempDir::new("sweetpad-actool-size-classes");
    let catalog = temp.join("Sizes.xcassets");
    let set = catalog.join("Pin.imageset");
    std::fs::create_dir_all(&set).unwrap();
    std::fs::write(set.join("pin.png"), png_of(8, [0, 0, 255, 255])).unwrap();
    let base = json!({"idiom": "universal", "scale": "1x", "filename": "pin.png"});
    for (horizontal, vertical) in [
        (None, None),
        (Some("compact"), None),
        (None, Some("regular")),
        (Some("compact"), Some("regular")),
    ] {
        let mut entries = vec![base.clone()];
        let mut slot = base.clone();
        if let Some(value) = horizontal {
            slot["width-class"] = json!(value);
        }
        if let Some(value) = vertical {
            slot["height-class"] = json!(value);
        }
        if slot != base {
            entries.push(slot);
        }
        // Xcode represents the default by omitting the axis. Invalid selectors
        // must never become extra copies of the default rendition.
        for value in ["any", "unsupported"] {
            let mut invalid = base.clone();
            invalid["width-class"] = json!(value);
            entries.push(invalid);
        }
        write_json(&set.join("Contents.json"), &json!({"images": entries}));
        for platform in [
            "iphoneos",
            "iphonesimulator",
            "xros",
            "xrsimulator",
            "macosx",
            "appletvsimulator",
            "watchsimulator",
        ] {
            let out = temp.join(platform);
            compile::compile(&platform_args(&catalog, &out, platform)).unwrap();
            let records = renditions(&out.join("Assets.car"));
            let supported = matches!(
                platform,
                "iphoneos" | "iphonesimulator" | "xros" | "xrsimulator"
            );
            let width = horizontal.is_some() && supported;
            let height = vertical.is_some() && supported;
            let mut slots = Vec::new();
            for (key, _, _) in records {
                assert_eq!(key.contains_key(&20), width, "{platform} horizontal axis");
                assert_eq!(key.contains_key(&21), height, "{platform} vertical axis");
                slots.push((
                    key.get(&20).copied().unwrap_or(0),
                    key.get(&21).copied().unwrap_or(0),
                ));
            }
            slots.sort_unstable();
            let mut expected = vec![(0, 0)];
            if width || height {
                expected.push((u16::from(width), 2 * u16::from(height)));
            }
            assert_eq!(slots, expected, "{platform} {horizontal:?} {vertical:?}");
        }
    }
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
fn a_symbol_sets_rendering_intent_selects_its_coreui_mode() {
    let dir = TempDir::new("sweetpad-actool-symbol-intents");
    let catalog = dir.join("Intents.xcassets");
    let cases = [
        ("automatic", 0),
        ("template", 1),
        ("multicolor", 2),
        ("hierarchical", 3),
        ("palette", 4),
        ("monochrome", 0),
    ];
    for (name, _) in cases {
        write_json(
            &catalog.join(format!("{name}.symbolset/Contents.json")),
            &json!({"properties": {"symbol-rendering-intent": name}}),
        );
    }
    let parsed = Catalog::read(&catalog);
    let modes: std::collections::BTreeMap<_, _> = parsed
        .assets()
        .into_iter()
        .map(|(_, asset)| (asset.name.as_str(), asset.symbol.unwrap() as u32))
        .collect();
    assert_eq!(modes, cases.into());
}

#[test]
fn layered_symbol_masters_retain_their_styles_intent_and_shared_margins() {
    let dir = TempDir::new("sweetpad-actool-symbol-layers");
    let catalog = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/actool/resources/LayeredSymbols.xcassets");
    let out = dir.join("out");
    compile::compile(&platform_args(&catalog, &out, "iphonesimulator")).unwrap();
    let data = std::fs::read(out.join("Assets.car")).unwrap();
    let (rows, _) = super::car::tests::Store::read(&data).tree("RENDITIONS");
    assert_eq!(rows.len(), 2);
    for (_, row) in rows {
        let word = |at| u32::from_le_bytes(row[at..at + 4].try_into().unwrap());
        let end = 184 + word(168) as usize;
        let mut fields = std::collections::BTreeMap::new();
        let mut at = 184;
        while at < end {
            fields.insert(word(at), at + 8);
            at += 8 + word(at + 4) as usize;
        }
        // Xcode records multicolor intent (2) and both layer flags (3)
        // even though these two static masters cannot interpolate.
        let modes = fields[&1019];
        assert_eq!((word(modes + 4), word(modes + 8)), (2, 3));
        let metrics = fields[&1018];
        for (offset, expected) in [(20, 0.85), (28, 1.7)] {
            assert!((f32::from_bits(word(metrics + offset)) - expected).abs() < 1e-5);
        }
        let svg = std::str::from_utf8(&row[end + 12..]).unwrap();
        let document = resvg::usvg::roxmltree::Document::parse(svg).unwrap();
        let css = document
            .descendants()
            .find(|n| n.has_tag_name("style"))
            .unwrap()
            .text()
            .unwrap();
        for rule in [
            "-sfsymbols-clear-behind:true",
            "fill:#227744",
            "fill:#FFAA33",
        ] {
            assert!(
                css.contains(rule),
                "{rule} missing from the standalone master"
            );
        }
    }
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

/// A compiled car's app icons as `regen_resources.py` lists Xcode's: each
/// icon image as `[name, idiom, subtype, scale, index, width, height, file]`
/// and each list of sizes as `[name, idiom, subtype, [[points, index], …]]`.
fn icon_listing(path: &Path) -> (Value, Value) {
    let data = std::fs::read(path).unwrap();
    let store = super::car::tests::Store::read(&data);
    let format = store.variable("KEYFORMAT");
    let le = |data: &[u8], at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    let attributes: Vec<u32> = (0..le(format, 8) as usize)
        .map(|i| le(format, 12 + 4 * i))
        .collect();
    let (renditions, _) = store.tree("RENDITIONS");
    let renditions: Vec<(std::collections::BTreeMap<u32, u16>, &[u8])> = renditions
        .iter()
        .map(|(key, value)| {
            let values = key.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
            (attributes.iter().copied().zip(values).collect(), *value)
        })
        .collect();
    let name = |value: &[u8]| {
        let field = &value[40..168];
        let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
        String::from_utf8(field[..end].to_vec()).unwrap()
    };
    // A list of sizes carries its icon's name; an icon image, its file's.
    let mut names = std::collections::BTreeMap::new();
    let mut lists = Vec::new();
    for (key, value) in renditions.iter().filter(|(key, _)| key[&2] == 218) {
        names.insert(key[&17], name(value));
        let payload = 184 + le(value, 168) as usize;
        assert_eq!(&value[payload..payload + 4], b"SISM");
        let sizes: Vec<Value> = (0..le(value, payload + 8) as usize)
            .map(|i| {
                let at = payload + 12 + 12 * i;
                json!([le(value, at), le(value, at + 8)])
            })
            .collect();
        lists.push(json!([name(value), key[&15], key[&16], sizes]));
    }
    let icons: Vec<Value> = renditions
        .iter()
        .filter(|(key, _)| key[&2] == 220)
        .map(|(key, value)| {
            json!([
                names[&key[&17]],
                key[&15],
                key[&16],
                key[&12],
                key[&9],
                le(value, 12),
                le(value, 16),
                name(value),
            ])
        })
        .collect();
    (sorted(&json!(icons)), sorted(&json!(lists)))
}

/// Each rendition in a compiled `Assets.car` as its part, idiom, scale,
/// size and file.
fn rendition_listing(path: &Path) -> Value {
    let data = std::fs::read(path).unwrap();
    let store = super::car::tests::Store::read(&data);
    let format = store.variable("KEYFORMAT");
    let le = |data: &[u8], at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    let attributes: Vec<u32> = (0..le(format, 8) as usize)
        .map(|i| le(format, 12 + 4 * i))
        .collect();
    let (renditions, _) = store.tree("RENDITIONS");
    let listing: Vec<Value> = renditions
        .iter()
        .map(|(key, value)| {
            let values = key.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
            let key: std::collections::BTreeMap<u32, u16> =
                attributes.iter().copied().zip(values).collect();
            let field = &value[40..168];
            let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
            json!([
                key[&2],
                key[&15],
                key[&12],
                le(value, 12),
                le(value, 16),
                std::str::from_utf8(&field[..end]).unwrap(),
            ])
        })
        .collect();
    sorted(&json!(listing))
}

/// A JSON array with its items in one order, whichever order they came in.
fn sorted(items: &Value) -> Value {
    let mut items = items.as_array().unwrap().clone();
    items.sort_by_key(ToString::to_string);
    Value::Array(items)
}

#[test]
fn watch_icons_match_actool() {
    let expected: Value = serde_json::from_str(&fixture("expected/watch.json")).unwrap();
    for (case, icon, alternate) in [
        ("single", "AppIcon", None),
        ("legacy", "Legacy", None),
        ("alternate", "AppIcon", Some("Legacy")),
    ] {
        let expected = &expected[case];
        let dir = TempDir::new("sweetpad-actool-watch-icon");
        let out = dir.join("out");
        let partial = out.join("partial.plist");
        let mut args = platform_args(&resources().join("Watch.xcassets"), &out, "watchos");
        for (flag, value) in [
            ("--minimum-deployment-target", Some("9.0")),
            ("--target-device", Some("watch")),
            ("--app-icon", Some(icon)),
            ("--alternate-app-icon", alternate),
            ("--output-partial-info-plist", partial.to_str()),
        ] {
            if let Some(value) = value {
                args.flags.push((flag.into(), Some(value.into())));
            }
        }
        compile::compile(&args).unwrap();
        assert_same(
            &std::fs::read_to_string(&partial).unwrap(),
            &xml_plist::of(&expected["partialInfo"]),
            &format!("{case} watch icon metadata"),
        );
        let mut files: Vec<String> = walk(&out)
            .iter()
            .map(|p| p.strip_prefix(&out).unwrap().display().to_string())
            .collect();
        files.sort();
        assert_eq!(json!(files), expected["files"], "{case}");
        let car = std::fs::read(out.join("Assets.car")).unwrap();
        let metadata = super::car::tests::Store::read(&car).variable("EXTENDED_METADATA");
        // `META`, 256 reserved bytes and the deployment target come first.
        let platform = &metadata[516..772];
        let end = platform.iter().position(|&b| b == 0).unwrap();
        assert_eq!(
            json!(std::str::from_utf8(&platform[..end]).unwrap()),
            expected["platform"]
        );
        let (icons, lists) = icon_listing(&out.join("Assets.car"));
        assert_eq!(
            icons,
            sorted(&expected["iconRenditions"]),
            "{case} icon images"
        );
        assert_eq!(lists, sorted(&expected["sizeLists"]), "{case} size lists");
    }
}

#[test]
fn a_jpeg_goes_in_the_car_as_it_is() {
    let dir = TempDir::new("sweetpad-actool-jpeg");
    let catalog = template_catalog(&dir);
    let set = catalog.join("Photo.imageset");
    std::fs::create_dir_all(&set).unwrap();
    // A JPEG's start, up to its frame header: 40 by 30 pixels.
    let jpeg = [
        &[0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0][..],
        &[0xff, 0xc0, 0, 11, 8, 0, 30, 0, 40, 1, 1, 0x11, 0],
        &[0xff, 0xda, 0, 2, 0xff, 0xd9],
    ]
    .concat();
    std::fs::write(set.join("photo.jpg"), &jpeg).unwrap();
    write_json(
        &set.join("Contents.json"),
        &json!({"images": [{"filename": "photo.jpg", "idiom": "universal", "scale": "2x"}]}),
    );
    let out = dir.join("out");
    compile::compile(&platform_args(&catalog, &out, "iphonesimulator")).unwrap();
    let car = std::fs::read(out.join("Assets.car")).unwrap();
    let raw = [
        &b"DWAR\0\0\0\0"[..],
        &u32::try_from(jpeg.len()).unwrap().to_le_bytes(),
        &jpeg,
    ]
    .concat();
    assert!(car.windows(raw.len()).any(|w| w == raw));
    let photos: Vec<_> = renditions(&out.join("Assets.car"))
        .into_iter()
        .filter(|(key, _, _)| key[&2] == 181)
        .map(|(key, layout, _)| (key[&12], layout))
        .collect();
    assert_eq!(photos, [(2, 12)]);
}

#[test]
fn a_data_set_goes_in_the_car_with_its_type() {
    let dir = TempDir::new("sweetpad-actool-data");
    let catalog = template_catalog(&dir);
    let set = catalog.join("Config.dataset");
    std::fs::create_dir_all(&set).unwrap();
    std::fs::write(set.join("config.json"), b"{}").unwrap();
    std::fs::write(set.join("pad.foo"), b"pad").unwrap();
    write_json(
        &set.join("Contents.json"),
        &json!({"data": [
            {"filename": "config.json", "idiom": "universal"},
            {"filename": "pad.foo", "idiom": "ipad", "universal-type-identifier": "dev.sweetpad.pad"},
        ]}),
    );
    let out = dir.join("out");
    compile::compile(&compile_args(&catalog, &out, &dir)).unwrap();
    let car = std::fs::read(out.join("Assets.car")).unwrap();
    let has = |bytes: &[u8]| car.windows(bytes.len()).any(|w| w == bytes);
    assert!(has(b"DWAR\0\0\0\0\x02\0\0\0{}"));
    assert!(has(b"DWAR\0\0\0\0\x03\0\0\0pad"));
    assert!(has(b"\x0c\0\0\0\0\0\0\0public.json\0"));
    assert!(has(b"dev.sweetpad.pad\0"));
    // Idiom 15 is the iPad's 2 for the second file.
    let idioms: Vec<u16> = renditions(&out.join("Assets.car"))
        .into_iter()
        .filter(|(_, layout, _)| *layout == 1000)
        .map(|(key, _, _)| key[&15])
        .collect();
    assert_eq!(idioms, [0, 2]);
    assert_eq!(car_names(&out.join("Assets.car")), ["Config"]);
}

#[test]
fn extensions_keep_all_standalone_icon_sizes() {
    let dir = TempDir::new("sweetpad-icon-extension");
    let catalog = dir.join("Assets.xcassets");
    let set = catalog.join("AppIcon.appiconset");
    std::fs::create_dir_all(&set).unwrap();
    let entries = [
        ("small.png", "iphone", "20x20", "2x", 40),
        ("phone.png", "iphone", "60x60", "2x", 120),
        ("tablet.png", "ipad", "76x76", "2x", 152),
        ("store.png", "ios-marketing", "1024x1024", "1x", 1024),
    ];
    let mut images = Vec::new();
    for (filename, idiom, size, scale, pixels) in entries {
        std::fs::write(set.join(filename), png_of(pixels, [7, 19, 31, 255])).unwrap();
        images.push(json!({"filename":filename,"idiom":idiom,"size":size,"scale":scale}));
    }
    write_json(&set.join("Contents.json"), &json!({"images":images}));
    let out = dir.join("out");
    let mut args = platform_args(&catalog, &out, "iphonesimulator");
    args.flags
        .push(("--app-icon".into(), Some("AppIcon".into())));
    args.flags
        .push(("--standalone-icon-behavior".into(), Some("all".into())));
    compile::compile(&args).unwrap();
    assert!(out.join("AppIcon20x20@2x.png").is_file());
    assert!(out.join("AppIcon60x60@2x.png").is_file());
    assert!(out.join("AppIcon76x76@2x~ipad.png").is_file());
    assert!(!out.join("AppIcon1024x1024.png").exists());

    // Native actool generates only the phone/tablet files for an unscaled
    // universal 1024 icon, even with --standalone-icon-behavior all.
    write_json(
        &set.join("Contents.json"),
        &json!({"images":[{
            "filename":"store.png", "idiom":"universal", "platform":"ios", "size":"1024x1024"
        }]}),
    );
    let universal = dir.join("universal");
    args.flags.retain(|(name, _)| name != "--compile");
    args.flags.push((
        "--compile".into(),
        Some(universal.to_string_lossy().into_owned()),
    ));
    compile::compile(&args).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&universal)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "AppIcon60x60@2x.png",
            "AppIcon76x76@2x~ipad.png",
            "Assets.car"
        ]
    );
}
