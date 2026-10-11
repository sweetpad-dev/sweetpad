//! actool's `--compile`: what Swift Build links into the product for a
//! target's catalogs. There is no `Assets.car`, which is Apple's own format,
//! so the app icon and the images go in as loose PNGs, which iOS and UIKit
//! read without one, and a warning names what's still missing.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::catalog::{Asset, Catalog, Kind};
use super::{Args, catalogs, raster, target_devices, write, xml_plist};

/// The image formats UIKit loads from a loose file.
const RASTER: &[&str] = &["png", "jpg", "jpeg", "heic", "gif", "tif", "tiff", "bmp"];

pub(super) fn compile(args: &Args) -> Result<(), String> {
    let output = PathBuf::from(
        args.value("--compile")
            .ok_or("--compile needs a directory")?,
    );
    std::fs::create_dir_all(&output)
        .map_err(|e| format!("couldn't create {}: {e}", output.display()))?;
    let dependencies = args.value("--export-dependency-info").map(PathBuf::from);
    if let Some(path) = &dependencies {
        remove_previous_outputs(path, &output);
    }
    let catalogs = catalogs(args);
    let devices = target_devices(args);
    let mut written = Vec::new();
    let mut info = Map::new();

    let ios = matches!(
        args.value("--platform"),
        Some("iphoneos" | "iphonesimulator")
    );
    let icon_name = args.value("--app-icon");
    let icon_written = match (icon_name, ios) {
        (Some(name), true) => app_icon(&catalogs, name).is_some_and(|set| {
            let ok = write_ios_icon(set, name, &output, &mut written);
            if ok {
                info.extend(ios_icon_info(name, &devices));
            }
            ok
        }),
        _ => false,
    };
    let mut warnings = Vec::new();
    for catalog in &catalogs {
        let loose = write_images(catalog, &output, &devices, &mut written);
        let report = Report::of(catalog, icon_name, icon_written, &loose, args);
        if let Some(text) = report.warning() {
            warnings.push(format!("{}: warning: {text}", catalog.path.display()));
        }
    }

    if let Some(path) = args.value("--output-partial-info-plist") {
        write(Path::new(path), xml_plist(&Value::Object(info)).as_bytes())?;
    }
    if let Some(path) = &dependencies {
        write(path, &dependency_info(&args.inputs, &written))?;
    }
    if !warnings.is_empty() {
        println!("/* com.apple.actool.document.warnings */");
        for warning in warnings {
            println!("{warning}");
        }
    }
    println!("/* com.apple.actool.compilation-results */");
    for path in &written {
        println!("{}", path.display());
    }
    println!();
    Ok(())
}

/// The app icon set `--app-icon` names.
fn app_icon<'a>(catalogs: &'a [Catalog], name: &str) -> Option<&'a Asset> {
    catalogs
        .iter()
        .flat_map(|c| c.assets().into_iter().map(|(_, asset)| asset))
        .find(|a| a.kind == Kind::AppIcon && a.name == name && a.has_content)
}

/// The two files actool writes for an iOS app icon, whichever devices the
/// app targets: `AppIcon60x60@2x.png` for iPhone and
/// `AppIcon76x76@2x~ipad.png` for iPad. Each is the set's image of that size,
/// as it is, or else its 1024-point image scaled down. False when the set
/// has neither, or an image can't be read.
fn write_ios_icon(set: &Asset, name: &str, output: &Path, written: &mut Vec<PathBuf>) -> bool {
    let usable = |e: &&super::catalog::Entry| e.filename.is_some() && !e.appearance;
    let large = set.entries.iter().filter(usable).find(|e| {
        e.size.as_deref() == Some("1024x1024")
            && (e.idiom == "ios-marketing"
                || (e.idiom == "universal" && e.platform.as_deref().is_none_or(|p| p == "ios")))
    });
    let mut source: Option<Option<raster::Rgba>> = None;
    for (idiom, size, pixels, file) in [
        ("iphone", "60x60", 120, format!("{name}60x60@2x.png")),
        ("ipad", "76x76", 152, format!("{name}76x76@2x~ipad.png")),
    ] {
        let exact = set.entries.iter().filter(usable).find(|e| {
            e.idiom == idiom && e.size.as_deref() == Some(size) && e.scale.as_deref() == Some("2x")
        });
        let read = |entry: &super::catalog::Entry| {
            entry
                .filename
                .as_ref()
                .and_then(|f| std::fs::read(set.dir.join(f)).ok())
        };
        let bytes = if let Some(bytes) = exact.and_then(read) {
            bytes
        } else {
            let Some(large) = large else { return false };
            let decoded = source
                .get_or_insert_with(|| read(large).and_then(|bytes| raster::decode(&bytes).ok()));
            let Some(decoded) = decoded else { return false };
            match raster::encode(&raster::resize(decoded, pixels, pixels)) {
                Ok(bytes) => bytes,
                Err(_) => return false,
            }
        };
        let path = output.join(file);
        if write(&path, &bytes).is_err() {
            return false;
        }
        written.push(path);
    }
    true
}

/// The Info.plist keys actool writes for those files. The icon's name goes
/// with the devices the app targets.
fn ios_icon_info(name: &str, devices: &[String]) -> Map<String, Value> {
    let primary = |files: &[String], device: &str| {
        let mut icon = json!({ "CFBundleIconFiles": files });
        if devices.iter().any(|d| d == device) {
            icon["CFBundleIconName"] = json!(name);
        }
        json!({ "CFBundlePrimaryIcon": icon })
    };
    let iphone = format!("{name}60x60");
    let ipad = format!("{name}76x76");
    let mut info = Map::new();
    info.insert(
        "CFBundleIcons".into(),
        primary(std::slice::from_ref(&iphone), "iphone"),
    );
    info.insert(
        "CFBundleIcons~ipad".into(),
        primary(&[iphone, ipad], "ipad"),
    );
    info
}

/// Copy each image's raster files out as the loose files UIKit looks up by
/// name, `Logo@2x~ipad.png` and the like, under a folder per namespace, so
/// `UIImage(named: "Icons/Star")` finds `Icons/Star.png`. Returns the images
/// that got at least one file.
fn write_images<'a>(
    catalog: &'a Catalog,
    output: &Path,
    devices: &[String],
    written: &mut Vec<PathBuf>,
) -> HashSet<&'a Path> {
    let mut loose = HashSet::new();
    let mut taken: HashSet<PathBuf> = HashSet::new();
    for (chain, asset) in catalog.assets() {
        if asset.kind != Kind::Image || !asset.has_content {
            continue;
        }
        let folder: PathBuf = chain
            .iter()
            .filter(|f| f.namespace)
            .map(|f| f.name.as_str())
            .collect();
        for entry in &asset.entries {
            let Some(filename) = &entry.filename else {
                continue;
            };
            let applies = entry.idiom == "universal" || devices.contains(&entry.idiom);
            let extension = Path::new(filename)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if entry.appearance || !applies || !RASTER.contains(&extension.as_str()) {
                continue;
            }
            let scale = match entry.scale.as_deref() {
                Some("2x") => "@2x",
                Some("3x") => "@3x",
                _ => "",
            };
            let idiom = match entry.idiom.as_str() {
                "iphone" => "~iphone",
                "ipad" => "~ipad",
                _ => "",
            };
            let path = output
                .join(&folder)
                .join(format!("{}{scale}{idiom}.{extension}", asset.name));
            if !taken.insert(path.clone()) {
                continue;
            }
            let Ok(bytes) = std::fs::read(asset.dir.join(filename)) else {
                continue;
            };
            if write(&path, &bytes).is_ok() {
                written.push(path);
                loose.insert(asset.dir.as_path());
            }
        }
    }
    loose
}

/// What became of one catalog's assets.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Report {
    pub icon_missing: bool,
    pub alternate_icons: usize,
    pub colors: usize,
    /// Images that went in as loose files.
    pub loose_images: usize,
    /// Images with no file UIKit loads loose: PDFs, SVGs, symbols.
    pub vector_images: usize,
    pub other: usize,
}

impl Report {
    pub(super) fn of(
        catalog: &Catalog,
        icon_name: Option<&str>,
        icon_written: bool,
        loose: &HashSet<&Path>,
        args: &Args,
    ) -> Report {
        let alternates = args.values("--alternate-app-icon");
        let mut report = Report::default();
        for asset in catalog.compiled() {
            match asset.kind {
                Kind::AppIcon if Some(asset.name.as_str()) == icon_name => {
                    report.icon_missing = !icon_written;
                }
                Kind::AppIcon if alternates.contains(&asset.name.as_str()) => {
                    report.alternate_icons += 1;
                }
                // actool leaves out the icon sets no option names.
                Kind::AppIcon => {}
                Kind::Color => report.colors += 1,
                Kind::Image if loose.contains(asset.dir.as_path()) => report.loose_images += 1,
                Kind::Image => report.vector_images += 1,
                Kind::Other => report.other += 1,
            }
        }
        report
    }

    /// The warning, or none when the app has everything actool would have
    /// given it.
    pub(super) fn warning(&self) -> Option<String> {
        let counted = |n: usize, one: &str, many: &str| match n {
            0 => None,
            1 => Some((format!("1 {one}"), 1)),
            n => Some((format!("{n} {many}"), n)),
        };
        let parts: Vec<(String, usize)> = [
            self.icon_missing.then(|| ("the app icon".to_string(), 1)),
            counted(
                self.alternate_icons,
                "alternate app icon",
                "alternate app icons",
            ),
            counted(self.colors, "color", "colors"),
            counted(self.vector_images, "vector image", "vector images"),
            counted(self.other, "other asset", "other assets"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let missing = match parts.as_slice() {
            [] => None,
            [(one, n)] => Some(format!(
                "{one} {} missing from the app",
                if *n > 1 { "are" } else { "is" }
            )),
            [rest @ .., (last, _)] => Some(format!(
                "{} and {last} are missing from the app",
                rest.iter()
                    .map(|(text, _)| text.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        };
        let loose = match self.loose_images {
            0 => None,
            1 => Some("its 1 image loads in UIKit but not in SwiftUI's Image".to_string()),
            n => Some(format!(
                "its {n} images load in UIKit but not in SwiftUI's Image"
            )),
        };
        let consequence = match (missing, loose) {
            (None, None) => return None,
            (Some(missing), None) => missing,
            (None, Some(loose)) => loose,
            (Some(missing), Some(loose)) => format!("{missing}, and {loose}"),
        };
        Some(format!(
            "Linux builds don't compile asset catalogs yet, so {consequence}."
        ))
    }
}

/// ld64's dependency-info format, which Swift Build reads to know what the
/// compile read and wrote: a version record, a record per input catalog,
/// and one per file written.
fn dependency_info(inputs: &[PathBuf], outputs: &[PathBuf]) -> Vec<u8> {
    let mut out = vec![0x00];
    out.extend_from_slice(b"sweetpad-actool\0");
    for (opcode, path) in inputs
        .iter()
        .map(|p| (0x10, p))
        .chain(outputs.iter().map(|p| (0x40, p)))
    {
        out.push(opcode);
        out.extend_from_slice(path.display().to_string().as_bytes());
        out.push(0);
    }
    out
}

/// Remove what the last compile wrote into `output`, as its dependency file
/// lists it, so an image taken out of the catalog doesn't stay in the app.
fn remove_previous_outputs(dependencies: &Path, output: &Path) {
    let Ok(bytes) = std::fs::read(dependencies) else {
        return;
    };
    for record in bytes.split(|b| *b == 0) {
        let Some((&opcode, path)) = record.split_first() else {
            continue;
        };
        let path = PathBuf::from(String::from_utf8_lossy(path).into_owned());
        if opcode == 0x40 && path.starts_with(output) {
            let _ = std::fs::remove_file(path);
        }
    }
}
