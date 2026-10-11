//! An asset catalog as the stand-in reads it: folders, namespaces and the
//! assets in them, with what each asset holds.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// One `.xcassets` folder.
#[derive(Debug)]
pub struct Catalog {
    pub path: PathBuf,
    pub root: Folder,
}

#[derive(Debug, Default)]
pub struct Folder {
    pub name: String,
    /// The folder's assets are named under it (`Icons/Star`), which
    /// `provides-namespace` in its `Contents.json` asks for.
    pub namespace: bool,
    pub children: Vec<Node>,
}

#[derive(Debug)]
pub enum Node {
    Folder(Folder),
    Asset(Asset),
}

#[derive(Debug)]
pub struct Asset {
    pub name: String,
    pub kind: Kind,
    /// The asset's folder.
    pub dir: PathBuf,
    /// The asset's folder, relative to the catalog: `./Icons/Star.imageset`.
    pub relative_path: String,
    /// The idiom of each entry in its `Contents.json`.
    pub idioms: Vec<String>,
    /// The files of an image or app icon set, one per entry.
    pub entries: Vec<Entry>,
    /// A color set's colors, one per entry.
    pub colors: Vec<ColorEntry>,
    /// How an image set's images are drawn: `template-rendering-intent`.
    pub rendering: Option<String>,
    /// Whether an image set keeps its PDFs and SVGs, for drawing them at
    /// other sizes, besides the bitmaps drawn from them:
    /// `preserves-vector-representation`.
    pub preserves_vector: bool,
    /// A symbol set rather than an image set.
    pub symbol: bool,
    /// Whether actool would compile anything for it: a color with a value,
    /// an image with a file.
    pub has_content: bool,
}

/// One entry of an image or app icon set's `Contents.json`.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub filename: Option<String>,
    pub idiom: String,
    /// `1x`, `2x` or `3x`, or none for a single-scale image.
    pub scale: Option<String>,
    /// An app icon's size in points, like `60x60`.
    pub size: Option<String>,
    /// The platform a universal app icon is for, like `ios`.
    pub platform: Option<String>,
    /// A variant for dark mode, a tint or high contrast.
    pub appearance: bool,
    /// Its appearances, as `(appearance, value)`: `("luminosity", "dark")`.
    pub appearances: Vec<(String, String)>,
    /// Whether it varies by something else the stand-in doesn't compile:
    /// a display gamut, locale, size class, memory or graphics class.
    pub other_traits: bool,
}

/// One entry of a color set's `Contents.json`.
#[derive(Debug, Clone, Default)]
pub struct ColorEntry {
    pub idiom: String,
    pub appearances: Vec<(String, String)>,
    pub other_traits: bool,
    /// The `color` object: its color space and components.
    pub color: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Color,
    Image,
    AppIcon,
    /// Any other set (data, sticker, texture, …), which has no symbol.
    Other,
}

impl Catalog {
    pub fn read(path: &Path) -> Catalog {
        let root = if path.extension().is_some_and(|e| e == "icon") {
            // An Icon Composer icon passed on its own is one app icon.
            Folder {
                children: vec![Node::Asset(Asset {
                    name: stem(path),
                    kind: Kind::AppIcon,
                    dir: path.to_path_buf(),
                    relative_path: ".".into(),
                    idioms: Vec::new(),
                    entries: Vec::new(),
                    colors: Vec::new(),
                    rendering: None,
                    preserves_vector: false,
                    symbol: false,
                    has_content: true,
                })],
                ..Folder::default()
            }
        } else {
            read_folder(path, ".", String::new())
        };
        Catalog {
            path: path.to_path_buf(),
            root,
        }
    }

    /// Every asset, depth first in name order, with its folder chain.
    pub fn assets(&self) -> Vec<(Vec<&Folder>, &Asset)> {
        fn walk<'a>(
            folder: &'a Folder,
            chain: &mut Vec<&'a Folder>,
            out: &mut Vec<(Vec<&'a Folder>, &'a Asset)>,
        ) {
            for child in &folder.children {
                match child {
                    Node::Asset(asset) => out.push((chain.clone(), asset)),
                    Node::Folder(sub) => {
                        chain.push(sub);
                        walk(sub, chain, out);
                        chain.pop();
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut Vec::new(), &mut out);
        out
    }

    /// The assets actool would put in `Assets.car`.
    pub fn compiled(&self) -> Vec<&Asset> {
        self.assets()
            .into_iter()
            .map(|(_, asset)| asset)
            .filter(|asset| asset.has_content)
            .collect()
    }
}

fn read_folder(dir: &Path, relative: &str, name: String) -> Folder {
    let namespace = contents(dir)
        .and_then(|c| c.pointer("/properties/provides-namespace")?.as_bool())
        .unwrap_or(false);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    let children = entries
        .iter()
        .map(|path| {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let relative = format!("{relative}/{file_name}");
            match path.extension().and_then(|e| e.to_str()) {
                Some(extension) if is_set(extension) => Node::Asset(read_asset(path, relative)),
                _ => Node::Folder(read_folder(path, &relative, file_name)),
            }
        })
        .collect();
    Folder {
        name,
        namespace,
        children,
    }
}

/// The folder extensions actool treats as one asset rather than a folder.
fn is_set(extension: &str) -> bool {
    matches!(
        extension,
        "colorset"
            | "imageset"
            | "symbolset"
            | "appiconset"
            | "icon"
            | "dataset"
            | "brandassets"
            | "imagestack"
            | "launchimage"
            | "stickerpack"
            | "stickersequence"
            | "sticker"
            | "spriteatlas"
            | "textureset"
            | "cubetextureset"
            | "mipmapset"
            | "arimageset"
            | "arobjectset"
            | "complicationset"
            | "gcdashboardimage"
            | "gcleaderboard"
            | "gcleaderboardset"
            | "solidimagestack"
    )
}

fn read_asset(path: &Path, relative_path: String) -> Asset {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let contents = contents(path);
    let entries = |key: &str| -> Vec<Value> {
        contents
            .as_ref()
            .and_then(|c| c.get(key)?.as_array().cloned())
            .unwrap_or_default()
    };
    let idioms = |entries: &[Value]| -> Vec<String> {
        entries
            .iter()
            .map(|e| {
                e.get("idiom")
                    .and_then(Value::as_str)
                    .unwrap_or("universal")
                    .to_string()
            })
            .collect()
    };
    let mut color_entries = Vec::new();
    let (kind, idioms, entries, has_content) = match extension {
        "colorset" => {
            let colors = entries("colors");
            let has = colors.iter().any(|e| e.get("color").is_some());
            color_entries = colors
                .iter()
                .map(|e| ColorEntry {
                    idiom: e
                        .get("idiom")
                        .and_then(Value::as_str)
                        .unwrap_or("universal")
                        .to_string(),
                    appearances: appearances(e),
                    other_traits: other_traits(e),
                    color: e.get("color").cloned(),
                })
                .collect();
            (Kind::Color, idioms(&colors), Vec::new(), has)
        }
        "imageset" | "symbolset" | "appiconset" => {
            let key = if extension == "symbolset" {
                "symbols"
            } else {
                "images"
            };
            let images = entries(key);
            let has = images.iter().any(|e| e.get("filename").is_some());
            let kind = if extension == "appiconset" {
                Kind::AppIcon
            } else {
                Kind::Image
            };
            (kind, idioms(&images), files(&images), has)
        }
        "icon" => (Kind::AppIcon, Vec::new(), Vec::new(), true),
        _ => (Kind::Other, Vec::new(), Vec::new(), true),
    };
    let rendering = contents
        .as_ref()
        .and_then(|c| c.pointer("/properties/template-rendering-intent")?.as_str())
        .map(str::to_string);
    let preserves_vector = contents
        .as_ref()
        .and_then(|c| {
            c.pointer("/properties/preserves-vector-representation")?
                .as_bool()
        })
        .unwrap_or(false);
    Asset {
        name: stem(path),
        kind,
        dir: path.to_path_buf(),
        relative_path,
        idioms,
        entries,
        colors: color_entries,
        rendering,
        preserves_vector,
        symbol: extension == "symbolset",
        has_content,
    }
}

/// An entry's appearances, as `(appearance, value)` pairs.
fn appearances(entry: &Value) -> Vec<(String, String)> {
    entry
        .get("appearances")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some((
                a.get("appearance")?.as_str()?.to_string(),
                a.get("value")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

/// Whether an entry varies by a trait other than idiom, scale and
/// appearance.
fn other_traits(entry: &Value) -> bool {
    entry
        .get("display-gamut")
        .and_then(Value::as_str)
        .is_some_and(|g| g != "sRGB")
        || [
            "locale",
            "width-class",
            "height-class",
            "memory",
            "graphics-feature-set",
            "subtype",
        ]
        .iter()
        .any(|k| entry.get(*k).is_some())
}

/// An image or app icon set's entries, as the stand-in uses them.
fn files(entries: &[Value]) -> Vec<Entry> {
    let text = |e: &Value, key: &str| e.get(key).and_then(Value::as_str).map(str::to_string);
    entries
        .iter()
        .map(|e| Entry {
            filename: text(e, "filename"),
            idiom: text(e, "idiom").unwrap_or_else(|| "universal".to_string()),
            scale: text(e, "scale"),
            size: text(e, "size"),
            platform: text(e, "platform"),
            appearance: e.get("appearances").is_some(),
            appearances: appearances(e),
            other_traits: other_traits(e),
        })
        .collect()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn contents(dir: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(dir.join("Contents.json")).ok()?;
    serde_json::from_str(&text).ok()
}
