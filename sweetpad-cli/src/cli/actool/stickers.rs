//! Message-extension icons and ordered sticker packs, including GIF stickers.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::car::{Appearance, Bitmap, Car, Part, Rendering, Source, Variant};
use super::catalog::{Asset, Catalog};
use super::{Args, raster, write};
use crate::cli::xml_plist;

/// A Messages extension's icon as the loose PNGs Messages reads, each size
/// but the 29-point ones, which Xcode 27's actool leaves out, and the
/// `CFBundleIcons` keys that name them.
pub(super) fn icon(
    set: &Asset,
    name: &str,
    output: &Path,
    written: &mut Vec<PathBuf>,
    info: &mut Map<String, Value>,
) -> bool {
    let mut names = Vec::new();
    for entry in &set.entries {
        if entry.idiom == "ios-marketing"
            || entry.appearance
            || entry.other_traits
            || entry.size.as_deref() == Some("29x29")
        {
            continue;
        }
        let (Some(file), Some(size)) = (&entry.filename, &entry.size) else {
            continue;
        };
        let scale = entry
            .scale
            .as_deref()
            .filter(|s| *s != "1x")
            .map_or_else(String::new, |s| format!("@{s}"));
        let path = output.join(format!("{name}{size}{scale}.png"));
        let Ok(bytes) = std::fs::read(set.dir.join(file)) else {
            return false;
        };
        if write(&path, &bytes).is_err() {
            return false;
        }
        written.push(path);
        let name = format!("{name}{size}");
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        return false;
    }
    info.insert(
        "CFBundleIcons".into(),
        json!({"CFBundlePrimaryIcon": {"CFBundleIconFiles": names}}),
    );
    info.insert("MSMessagesExtensionStoreIconName".into(), json!(name));
    true
}

/// A Messages extension icon's App Store image in `car`: its 1024 by 768
/// `ios-marketing` image, a plain image with the icon's name, the only part
/// of the set Xcode's actool puts in the car. App Store Connect looks it up
/// there (ITMS-90644 without it).
pub(super) fn store_icon<'a>(
    car: &mut Car,
    name: &str,
    set: &'a Asset,
    compiled: &mut HashSet<&'a Path>,
) -> bool {
    let Some(file) = set
        .entries
        .iter()
        .find(|e| {
            e.idiom == "ios-marketing"
                && e.size.as_deref() == Some("1024x768")
                && !e.appearance
                && !e.other_traits
        })
        .and_then(|e| e.filename.as_deref())
    else {
        return false;
    };
    let Some(bitmap) = std::fs::read(set.dir.join(file))
        .ok()
        .and_then(|bytes| raster::decode(&bytes).ok())
        .map(|rgba| Bitmap::from_rgba(&rgba))
    else {
        return false;
    };
    let Some(identifier) = car.facet(name, Part::Image) else {
        return false;
    };
    let variant = Variant {
        appearance: Appearance::Any,
        scale: 1,
        idiom: 6,
    };
    let source = Source {
        vector: false,
        preserved: false,
    };
    car.image(
        file,
        identifier,
        variant,
        &bitmap,
        Rendering::Original,
        source,
    );
    compiled.insert(set.dir.as_path());
    true
}

fn contents(path: &Path) -> Result<Value, String> {
    let path = path.join("Contents.json");
    serde_json::from_slice(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn identifier(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub(super) fn compile<'a>(
    catalogs: &'a [Catalog],
    args: &Args,
    output: &Path,
    written: &mut Vec<PathBuf>,
    info: &mut Map<String, Value>,
    compiled: &mut HashSet<&'a Path>,
) -> Result<(), String> {
    for pack in catalogs
        .iter()
        .flat_map(Catalog::assets)
        .map(|(_, a)| a)
        .filter(|a| a.dir.extension().is_some_and(|e| e == "stickerpack"))
    {
        compiled.insert(pack.dir.as_path());
        if !args.has("--include-sticker-content") {
            continue;
        }
        if args.value("--sticker-pack-strings-file").is_some() {
            return Err("localized sticker-pack strings are not supported yet".into());
        }
        if info.contains_key("IMStickerPackPath") {
            return Err("a messages extension must select a single sticker pack".into());
        }
        let data = contents(&pack.dir)?;
        let properties = &data["properties"];
        let size = match properties["grid-size"].as_str().unwrap_or("regular") {
            "small" => "MSStickerSizeClassSmall",
            "regular" => "MSStickerSizeClassRegular",
            "large" => "MSStickerSizeClassLarge",
            other => return Err(format!("unknown sticker grid-size {other:?}")),
        };
        let directory = format!("{}.stickerpack", pack.name);
        let mut stickers = Vec::new();
        for entry in data["stickers"]
            .as_array()
            .ok_or("a sticker pack needs a stickers array")?
        {
            let member = entry["filename"]
                .as_str()
                .ok_or("a sticker pack member needs a filename")?;
            let path = pack.dir.join(member);
            if path.extension().is_none_or(|e| e != "sticker") {
                return Err(format!(
                    "unsupported sticker pack member {member:?}; use a PNG or GIF .sticker"
                ));
            }
            let sticker = contents(&path)?;
            let props = &sticker["properties"];
            let file = props["filename"]
                .as_str()
                .ok_or_else(|| format!("{member}: missing sticker filename"))?;
            let extension = Path::new(file)
                .extension()
                .and_then(|e| e.to_str())
                .ok_or("a sticker needs an image extension")?;
            if !matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "gif" | "jpg" | "jpeg"
            ) {
                return Err(format!("unsupported sticker image {file:?}"));
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("invalid sticker name")?;
            let filename = format!("{stem}.{extension}");
            let bytes =
                std::fs::read(path.join(file)).map_err(|e| format!("{member}/{file}: {e}"))?;
            let destination = output.join(&directory).join(&filename);
            write(&destination, &bytes)?;
            written.push(destination);
            let mut item = json!({"IMStickerIdentifier": filename});
            if let Some(label) = props["accessibility-label"].as_str() {
                item["IMStickerAccessibilityLabel"] = json!(label);
            }
            stickers.push(item);
        }
        let prefix = args.value("--sticker-pack-identifier-prefix").map_or_else(
            || {
                format!(
                    "{}.sticker-pack.",
                    args.value("--bundle-identifier").unwrap_or("")
                )
            },
            str::to_string,
        );
        let bundle_id = format!("{prefix}{}", identifier(&pack.name));
        let metadata = json!({"CFBundleIdentifier": bundle_id, "CFBundleName": pack.name,
            "CFBundlePackageType": "BUNDL", "CFBundleShortVersionString": "1.0", "CFBundleVersion": "1",
            "IMStickerPackLayout": size, "IMStickers": stickers});
        let path = output.join(&directory).join("Info.plist");
        write(&path, xml_plist::of(&metadata).as_bytes())?;
        written.push(path);
        info.insert("IMStickerPackPath".into(), json!(directory));
    }
    Ok(())
}
