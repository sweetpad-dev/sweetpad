//! actool's `--compile`: what Swift Build links into the product for a
//! target's catalogs. The colors, images and symbols go in an `Assets.car`
//! (see [`super::car`]), with each PDF and SVG drawn at the platform's
//! scales (see [`super::vector`]) and each symbol template read into its
//! masters (see [`super::glyph`]). An iOS app icon goes in as the loose PNGs
//! the home screen reads. A warning names what's still missing: images in
//! other formats, colors for traits the stand-in doesn't compile, and assets
//! of other kinds.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::car::{
    Appearance, Bitmap, Car, ColorSpace, Family, Glyph, Part, Rendering, Source, Variant,
};
use super::catalog::{Asset, Catalog, Kind};
use super::vector::{Drawing, Format};
use super::{Args, catalogs, glyph, raster, target_devices, write};
use crate::cli::xml_plist;

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
    let platform = args.value("--platform").unwrap_or_default();
    let mut car = Car::new(
        if platform == "macosx" {
            Family::Mac
        } else {
            Family::Ios
        },
        platform_name(platform),
        args.value("--minimum-deployment-target")
            .unwrap_or_default(),
    );
    let mut compiled = HashSet::new();
    for catalog in &catalogs {
        add_assets(catalog, &mut car, &devices, platform, &mut compiled);
    }
    if !car.is_empty() {
        let path = output.join("Assets.car");
        write(&path, &car.write(0))?;
        written.push(path);
    }
    let mut warnings = Vec::new();
    for catalog in &catalogs {
        let report = Report::of(catalog, icon_name, icon_written, &compiled, args);
        if let Some(text) = report.warning() {
            warnings.push(format!("{}: warning: {text}", catalog.path.display()));
        }
    }

    if let Some(path) = args.value("--output-partial-info-plist") {
        write(
            Path::new(path),
            xml_plist::of(&Value::Object(info)).as_bytes(),
        )?;
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

/// The name the metadata gives a platform.
fn platform_name(platform: &str) -> &'static str {
    match platform {
        "macosx" => "macosx",
        "appletvos" | "appletvsimulator" => "tvos",
        "watchos" | "watchsimulator" => "watchos",
        "xros" | "xrsimulator" => "xros",
        _ => "ios",
    }
}

/// Add a catalog's colors, images and symbols to `car`, and the folders of
/// the assets that went in to `compiled`. An asset whose name is in already,
/// from another catalog, stays out.
fn add_assets<'a>(
    catalog: &'a Catalog,
    car: &mut Car,
    devices: &[String],
    platform: &str,
    compiled: &mut HashSet<&'a Path>,
) {
    for (chain, asset) in catalog.assets() {
        if !asset.has_content {
            continue;
        }
        let name: String = chain
            .iter()
            .filter(|f| f.namespace)
            .map(|f| format!("{}/", f.name))
            .chain([asset.name.clone()])
            .collect();
        let added = match asset.kind {
            Kind::Color => add_color(car, &name, asset, devices, platform),
            Kind::Image if asset.symbol => add_symbol(car, &name, asset, devices, platform),
            Kind::Image => add_image(car, &name, asset, devices, platform),
            Kind::AppIcon | Kind::Other => false,
        };
        if added {
            compiled.insert(asset.dir.as_path());
        }
    }
}

/// A color set's color: components, or the name of a system color.
enum Color {
    Components(ColorSpace, Vec<f64>),
    System(String),
}

fn add_color(car: &mut Car, name: &str, asset: &Asset, devices: &[String], platform: &str) -> bool {
    let colors: Vec<(Variant, Color)> = ordered(&asset.colors, |c| &c.idiom)
        .into_iter()
        .filter_map(|c| {
            let variant = variant(
                &c.idiom,
                &c.appearances,
                c.other_traits,
                1,
                devices,
                platform,
            )?;
            let value = c.color.as_ref()?;
            let color = if let Some(system) = value.get("reference").and_then(Value::as_str) {
                Color::System(system.to_string())
            } else {
                let (space, components) = color(value)?;
                Color::Components(space, components)
            };
            Some((variant, color))
        })
        .collect();
    if colors.is_empty() {
        return false;
    }
    let Some(identifier) = car.facet(name, Part::Color) else {
        return false;
    };
    for (variant, color) in colors {
        match color {
            Color::Components(space, components) => {
                car.color(name, identifier, variant, space, &components);
            }
            Color::System(system) => car.system_color(name, identifier, variant, &system),
        }
    }
    true
}

fn add_image(car: &mut Car, name: &str, asset: &Asset, devices: &[String], platform: &str) -> bool {
    let rendering = match asset.rendering.as_deref() {
        Some("template") => Rendering::Template,
        Some("original") => Rendering::Original,
        _ => Rendering::Automatic,
    };
    let mut images = Vec::new();
    let mut vectors = Vec::new();
    let mut drawings = Vec::new();
    for entry in ordered(&asset.entries, |e| &e.idiom) {
        let Some(filename) = entry.filename.as_deref() else {
            continue;
        };
        let scale = match entry.scale.as_deref() {
            Some("1x") => Some(1),
            Some("2x") => Some(2),
            Some("3x") => Some(3),
            _ => None,
        };
        let Some(variant) = variant(
            &entry.idiom,
            &entry.appearances,
            entry.other_traits,
            scale.unwrap_or(1),
            devices,
            platform,
        ) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(asset.dir.join(filename)) else {
            continue;
        };
        if let Some(format) = Format::of(filename) {
            let Ok(drawing) = Drawing::parse(format, bytes.clone()) else {
                continue;
            };
            vectors.push((variant, filename, format, bytes));
            drawings.push(Drawn {
                variant,
                scale,
                filename,
                drawing,
            });
        } else if Path::new(filename)
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("png"))
            && let Ok(image) = raster::decode(&bytes)
        {
            let source = Source {
                vector: false,
                preserved: asset.preserves_vector,
            };
            images.push((variant, filename, Bitmap::from_rgba(&image), source));
        }
    }
    let source = Source {
        vector: true,
        preserved: asset.preserves_vector,
    };
    for (variant, filename, bitmap) in draw(&drawings, scales(platform)) {
        images.push((variant, filename, bitmap, source));
    }
    if images.is_empty() {
        return false;
    }
    let Some(identifier) = car.facet(name, Part::Image) else {
        return false;
    };
    for (variant, filename, image, source) in images {
        car.image(filename, identifier, variant, &image, rendering, source);
    }
    // A set that preserves vector data keeps each file, for the scale it's
    // for; any other keeps one per variant, for no scale.
    let mut kept: Vec<Variant> = Vec::new();
    for (variant, filename, format, bytes) in vectors {
        if !asset.preserves_vector {
            let any_scale = Variant {
                scale: 0,
                ..variant
            };
            if kept.contains(&any_scale) {
                continue;
            }
            kept.push(any_scale);
        }
        car.vector(
            filename,
            identifier,
            variant,
            format,
            &bytes,
            asset.preserves_vector,
        );
    }
    true
}

/// A set's PDF or SVG, parsed.
struct Drawn<'a> {
    variant: Variant,
    /// The scale the set gives it, or none for one drawn at every scale.
    scale: Option<u16>,
    filename: &'a str,
    drawing: Drawing,
}

/// The bitmaps actool draws from a set's PDFs and SVGs, one per scale the
/// platform has. A file for no scale is drawn at each. A file for one scale
/// is as many pixels as it has points at that scale, and each scale with no
/// file of its own is drawn from the file nearest it, the larger on a tie.
fn draw<'a>(drawings: &[Drawn<'a>], scales: &[u16]) -> Vec<(Variant, &'a str, Bitmap)> {
    let mut plan: Vec<(&Drawn, u16)> = Vec::new();
    let mut by_variant: Vec<Vec<&Drawn>> = Vec::new();
    for drawn in drawings {
        if drawn.scale.is_none() {
            plan.extend(scales.iter().map(|s| (drawn, *s)));
            continue;
        }
        let same = |d: &&Drawn| {
            d.variant.idiom == drawn.variant.idiom
                && d.variant.appearance == drawn.variant.appearance
        };
        match by_variant
            .iter_mut()
            .find(|group| group.first().is_some_and(same))
        {
            Some(group) => group.push(drawn),
            None => by_variant.push(vec![drawn]),
        }
    }
    for group in &by_variant {
        for &target in scales {
            let nearest = group.iter().min_by_key(|d| {
                let own = d.scale.unwrap_or(1);
                (own.abs_diff(target), std::cmp::Reverse(own))
            });
            if let Some(drawn) = nearest {
                plan.push((drawn, target));
            }
        }
    }
    plan.into_iter()
        .filter_map(|(drawn, target)| {
            let factor = f32::from(target) / f32::from(drawn.scale.unwrap_or(1));
            let bitmap = drawn.drawing.render(factor).ok()?;
            let variant = Variant {
                scale: target,
                ..drawn.variant
            };
            Some((variant, drawn.filename, bitmap))
        })
        .collect()
}

/// The scales a platform's screens have, which actool draws a vector image
/// at.
fn scales(platform: &str) -> &'static [u16] {
    match platform {
        "iphoneos" | "iphonesimulator" => &[1, 2, 3],
        "macosx" | "appletvos" | "appletvsimulator" => &[1, 2],
        _ => &[2],
    }
}

/// A symbol set: each master its template draws, for each variant it has.
fn add_symbol(
    car: &mut Car,
    name: &str,
    asset: &Asset,
    devices: &[String],
    platform: &str,
) -> bool {
    let sizing = glyph::Sizing::of(platform);
    let symbols: Vec<(Variant, &str, Vec<Glyph>)> = ordered(&asset.entries, |e| &e.idiom)
        .into_iter()
        .filter_map(|e| {
            let filename = e.filename.as_deref()?;
            let variant = variant(
                &e.idiom,
                &e.appearances,
                e.other_traits,
                1,
                devices,
                platform,
            )?;
            let bytes = std::fs::read(asset.dir.join(filename)).ok()?;
            Some((variant, filename, glyph::masters(&bytes, sizing).ok()?))
        })
        .collect();
    if symbols.is_empty() {
        return false;
    }
    let Some(identifier) = car.symbol(name) else {
        return false;
    };
    for (variant, filename, glyphs) in symbols {
        for glyph in &glyphs {
            car.glyph(filename, identifier, variant, glyph);
        }
    }
    true
}

/// A set's entries with the universal ones last, so that where a platform's
/// keys can't tell idioms apart, as macOS's can't, its own idiom wins.
fn ordered<T>(entries: &[T], idiom: impl Fn(&T) -> &String) -> Vec<&T> {
    let mut out: Vec<&T> = entries.iter().collect();
    out.sort_by_key(|e| idiom(e) == "universal");
    out
}

/// The rendition an entry is, or none when it's for a device the app
/// doesn't target or varies in a way the stand-in doesn't compile.
fn variant(
    idiom: &str,
    appearances: &[(String, String)],
    other_traits: bool,
    scale: u16,
    devices: &[String],
    platform: &str,
) -> Option<Variant> {
    // Macs have no 3x displays, so Xcode leaves those images out.
    if other_traits || (scale == 3 && platform == "macosx") {
        return None;
    }
    let ios = matches!(platform, "iphoneos" | "iphonesimulator");
    let targeted = devices.iter().any(|d| d == idiom);
    let idiom = match idiom {
        "universal" => 0,
        "iphone" if targeted => 1,
        "ipad" if targeted => 2,
        "tv" if targeted => 3,
        // Xcode keeps CarPlay's variants in an iOS app's catalog.
        "car" if ios => 4,
        "watch" if targeted => 5,
        "mac" if targeted && platform == "macosx" => 0,
        _ => return None,
    };
    let (mut dark, mut high) = (None, false);
    for (kind, value) in appearances {
        match (kind.as_str(), value.as_str()) {
            ("luminosity", "dark") => dark = Some(true),
            ("luminosity", "light") => dark = Some(false),
            ("contrast", "high") => high = true,
            _ => return None,
        }
    }
    let appearance = match (dark, high) {
        (None, false) => Appearance::Any,
        (Some(true), false) => Appearance::Dark,
        (Some(false), false) => Appearance::Light,
        (None, true) => Appearance::HighContrastAny,
        (Some(true), true) => Appearance::HighContrastDark,
        (Some(false), true) => return None,
    };
    Some(Variant {
        appearance,
        scale,
        idiom,
    })
}

/// A color set's `color` as its color space and components, alpha last, or
/// none for one that names a system color.
pub(super) fn color(color: &Value) -> Option<(ColorSpace, Vec<f64>)> {
    let space = match color
        .get("color-space")
        .and_then(Value::as_str)
        .unwrap_or("srgb")
    {
        "srgb" => ColorSpace::Srgb,
        "display-p3" => ColorSpace::DisplayP3,
        "extended-srgb" => ColorSpace::ExtendedSrgb,
        "extended-linear-srgb" => ColorSpace::ExtendedLinearSrgb,
        "gray-gamma-22" => ColorSpace::GrayGamma22,
        "extended-gray" => ColorSpace::ExtendedGray,
        _ => return None,
    };
    let names: &[&str] = if matches!(space, ColorSpace::GrayGamma22 | ColorSpace::ExtendedGray) {
        &["white", "alpha"]
    } else {
        &["red", "green", "blue", "alpha"]
    };
    let components = color.get("components")?;
    let values = names
        .iter()
        .map(|n| component(components.get(*n)?, *n == "alpha"))
        .collect::<Option<Vec<f64>>>()?;
    Some((space, values))
}

/// One component the way actool reads it: a decimal as a 32-bit float, and
/// a hex or whole number as eight bits out of 255. Alpha is always a
/// decimal.
fn component(value: &Value, alpha: bool) -> Option<f64> {
    let text = match value {
        Value::String(text) => text.trim(),
        #[allow(clippy::cast_possible_truncation)] // actool keeps 32 bits
        Value::Number(n) => return Some(f64::from(n.as_f64()? as f32)),
        _ => return None,
    };
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return Some(f64::from(u8::from_str_radix(hex, 16).ok()?) / 255.0);
    }
    if alpha || text.contains('.') {
        return text.parse::<f32>().ok().map(f64::from);
    }
    text.parse::<u8>().ok().map(|n| f64::from(n) / 255.0)
}

/// What one catalog's assets missed.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Report {
    pub icon_missing: bool,
    pub alternate_icons: usize,
    /// Colors that name a system color, or are only for traits the stand-in
    /// doesn't compile.
    pub colors: usize,
    /// Images and symbols with no file the stand-in could read: JPEGs and
    /// other formats, or a PDF, SVG or symbol template it couldn't parse.
    pub images: usize,
    pub other: usize,
}

impl Report {
    pub(super) fn of(
        catalog: &Catalog,
        icon_name: Option<&str>,
        icon_written: bool,
        compiled: &HashSet<&Path>,
        args: &Args,
    ) -> Report {
        let alternates = args.values("--alternate-app-icon");
        let mut report = Report::default();
        for asset in catalog.compiled() {
            let done = compiled.contains(asset.dir.as_path());
            match asset.kind {
                Kind::AppIcon if Some(asset.name.as_str()) == icon_name => {
                    report.icon_missing = !icon_written;
                }
                Kind::AppIcon if alternates.contains(&asset.name.as_str()) => {
                    report.alternate_icons += 1;
                }
                Kind::Color if !done => report.colors += 1,
                Kind::Image if !done => report.images += 1,
                // actool leaves out the icon sets no option names.
                Kind::AppIcon | Kind::Color | Kind::Image => {}
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
            counted(
                self.images,
                "image that isn't a PNG, PDF or SVG",
                "images that aren't PNGs, PDFs or SVGs",
            ),
            counted(self.other, "other asset", "other assets"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let missing = match parts.as_slice() {
            [] => return None,
            [(one, n)] => format!("{one} {}", if *n > 1 { "are" } else { "is" }),
            [rest @ .., (last, _)] => format!(
                "{} and {last} are",
                rest.iter()
                    .map(|(text, _)| text.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        Some(format!(
            "Linux builds don't compile everything in an asset catalog yet, so {missing} \
             missing from the app."
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
