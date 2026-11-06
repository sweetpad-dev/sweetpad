//! What Xcode finds inside a synchronized folder
//! (`PBXFileSystemSynchronizedRootGroup`), as the nodes the PIF group tree
//! lists in its place.
//!
//! A synchronized folder names a directory, not its contents, and the PIF has
//! no node type for it: Xcode walks the directory and emits ordinary groups and
//! files. Folders become groups. Package-like folders (`.xcassets`, `.docc`,
//! `.bundle`, a SwiftPM package, anything in `explicitFolders`) are single
//! files. `.xcdatamodeld` becomes a version group, and `.lproj` folders are
//! dissolved into variant groups that collect each localized file's
//! translations.

use std::cmp::Ordering;
use std::fs;
use std::path::Path;

use crate::pbxproj::{Dict, Value};
use crate::pif_file_types;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Group,
    File,
    VariantGroup,
    VersionGroup,
}

#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub kind: Kind,
    pub name: String,
    pub path: String,
    /// The path below the synchronized folder, `/`-joined. A variant group
    /// takes its primary translation's, which is what its GUID hashes.
    pub rel: String,
    pub file_type: String,
    /// The language of a translation inside a variant group.
    pub region: Option<String>,
    /// A folder holding a `Package.swift`.
    pub package: bool,
    pub children: Vec<Node>,
}

impl Node {
    /// The node and everything under it, parents first.
    pub(crate) fn walk(&self) -> Vec<&Node> {
        let mut out = vec![self];
        if self.kind == Kind::Group {
            for child in &self.children {
                out.extend(child.walk());
            }
        }
        out
    }
}

/// The nodes for the synchronized folder `root`, whose directory is `dir`.
/// A directory that can't be read has no nodes.
pub(crate) fn expand(dir: &Path, root: &Dict, development_region: &str) -> Vec<Node> {
    let settings = Settings {
        explicit_folders: root
            .get("explicitFolders")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        explicit_types: root
            .get("explicitFileTypes")
            .and_then(Value::as_dict)
            .map(|d| {
                d.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        development_region,
    };
    children(dir, "", &settings, 0)
}

struct Settings<'a> {
    explicit_folders: Vec<String>,
    explicit_types: Vec<(String, String)>,
    development_region: &'a str,
}

/// Synchronized folders nested deeper than this are treated as a cycle.
const MAX_DEPTH: usize = 64;

fn children(dir: &Path, rel_dir: &str, settings: &Settings, depth: usize) -> Vec<Node> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut groups = Vec::new();
    let mut leaves = Vec::new();
    let mut translations: Vec<(String, String, bool)> = Vec::new();
    let mut names: Vec<(String, bool)> = entries
        .filter_map(Result::ok)
        .map(|e| {
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir())
                || (e.file_type().is_ok_and(|t| t.is_symlink()) && e.path().is_dir());
            (e.file_name().to_string_lossy().into_owned(), is_dir)
        })
        .collect();
    names.sort();
    for (name, is_dir) in names {
        let rel = join(rel_dir, &name);
        let path = dir.join(&name);
        let extension = extension(&name);
        if !is_dir {
            leaves.push(file(&name, &rel, file_type(&path, &rel, settings)));
            continue;
        }
        if extension == Some("lproj") {
            let lang = name.trim_end_matches(".lproj").to_string();
            let Ok(inner) = fs::read_dir(&path) else {
                continue;
            };
            for entry in inner.filter_map(Result::ok) {
                let file_name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
                translations.push((lang.clone(), file_name, is_dir));
            }
            continue;
        }
        if settings.explicit_folders.iter().any(|f| f == &rel) {
            let file_type = extension.and_then(pif_file_types::for_extension).unwrap_or(
                if extension.is_some() {
                    "wrapper"
                } else {
                    "folder"
                },
            );
            leaves.push(file(&name, &rel, file_type.to_string()));
            continue;
        }
        if extension == Some("xcdatamodeld") {
            leaves.push(version_group(&path, &name, rel));
            continue;
        }
        if let Some(wrapper) = extension
            .and_then(pif_file_types::for_extension)
            .filter(|t| t.starts_with("wrapper") || t.starts_with("folder"))
        {
            leaves.push(file(&name, &rel, wrapper.to_string()));
            continue;
        }
        if path.join("Package.swift").is_file() {
            let mut node = file(&name, &rel, "wrapper".into());
            node.package = true;
            leaves.push(node);
            continue;
        }
        let inner = if depth < MAX_DEPTH {
            children(&path, &rel, settings, depth + 1)
        } else {
            Vec::new()
        };
        groups.push(Node {
            kind: Kind::Group,
            name: name.clone(),
            path: name,
            rel,
            file_type: String::new(),
            region: None,
            package: false,
            children: inner,
        });
    }
    leaves.extend(variant_groups(dir, rel_dir, translations, settings));
    groups.sort_by(|a, b| by_name(&a.name, &b.name));
    leaves.sort_by(|a, b| by_name(&a.name, &b.name));
    groups.extend(leaves);
    groups
}

/// A Core Data model, `.xcdatamodeld`, with one file per model version.
fn version_group(path: &Path, name: &str, rel: String) -> Node {
    let mut versions: Vec<Node> = fs::read_dir(path)
        .map(|it| {
            it.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".xcdatamodel"))
                .map(|n| {
                    let child_rel = join(&rel, &n);
                    file(&n, &child_rel, "wrapper.xcdatamodel".into())
                })
                .collect()
        })
        .unwrap_or_default();
    versions.sort_by(|a, b| by_name(&a.name, &b.name));
    Node {
        kind: Kind::VersionGroup,
        name: name.to_string(),
        path: name.to_string(),
        rel,
        file_type: "wrapper.xcdatamodeld".into(),
        region: None,
        package: false,
        children: versions,
    }
}

/// The `.lproj` translations at one level, grouped by file name without its
/// extension: `Base.lproj/View.xib` and `de.lproj/View.strings` are one
/// variant group named after the primary translation, the `Base` one if there
/// is one and the development region's otherwise.
fn variant_groups(
    dir: &Path,
    rel_dir: &str,
    translations: Vec<(String, String, bool)>,
    settings: &Settings,
) -> Vec<Node> {
    let mut by_stem: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for (lang, file_name, _) in translations {
        let stem = file_name
            .rsplit_once('.')
            .map_or(file_name.as_str(), |(s, _)| s)
            .to_string();
        match by_stem.iter_mut().find(|(s, _)| *s == stem) {
            Some((_, list)) => list.push((lang, file_name)),
            None => by_stem.push((stem, vec![(lang, file_name)])),
        }
    }
    let rank = |lang: &str| -> u8 {
        if lang == "Base" {
            0
        } else if lang == settings.development_region {
            1
        } else {
            2
        }
    };
    by_stem
        .into_iter()
        .map(|(_, mut list)| {
            list.sort_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then_with(|| a.0.cmp(&b.0)));
            let children: Vec<Node> = list
                .iter()
                .map(|(lang, file_name)| {
                    let path = format!("{lang}.lproj/{file_name}");
                    let rel = join(rel_dir, &path);
                    let on_disk = dir.join(format!("{lang}.lproj")).join(file_name);
                    let mut node = file(&path, &rel, file_type(&on_disk, &rel, settings));
                    node.region = Some(lang.clone());
                    node
                })
                .collect();
            let (_, primary_name) = &list[0];
            Node {
                kind: Kind::VariantGroup,
                name: primary_name.clone(),
                path: String::new(),
                rel: children[0].rel.clone(),
                file_type: String::new(),
                region: None,
                package: false,
                children,
            }
        })
        .collect()
}

fn file(name: &str, rel: &str, file_type: String) -> Node {
    Node {
        kind: Kind::File,
        name: name.rsplit('/').next().unwrap_or(name).to_string(),
        path: name.to_string(),
        rel: rel.to_string(),
        file_type,
        region: None,
        package: false,
        children: Vec::new(),
    }
}

/// A file's type: `explicitFileTypes`, else its extension's. A name without
/// an extension (dotfiles included) is `text`, an unknown extension `file`.
/// A property list is `text.plist.xml` when it starts with an XML
/// declaration, which Xcode reads the file to find out.
fn file_type(path: &Path, rel: &str, settings: &Settings) -> String {
    if let Some((_, t)) = settings.explicit_types.iter().find(|(r, _)| r == rel) {
        return t.clone();
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match extension(&name) {
        None => "text".into(),
        Some(ext) if ext.eq_ignore_ascii_case("plist") => {
            let mut head = [0u8; 5];
            let xml = fs::File::open(path)
                .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
                .is_ok_and(|()| &head == b"<?xml");
            if xml { "text.plist.xml" } else { "text.plist" }.into()
        }
        Some(ext) => pif_file_types::for_extension(ext)
            .unwrap_or("file")
            .to_string(),
    }
}

/// The extension of a file name, `None` for a dotfile or a name without one.
fn extension(name: &str) -> Option<&str> {
    let trimmed = name.strip_prefix('.').unwrap_or(name);
    trimmed.rsplit_once('.').map(|(_, ext)| ext)
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Xcode's sibling order: by name without its extension, then by extension,
/// ignoring case (`App.swift` before `App+Menu.swift`), bytewise to break
/// ties.
fn by_name(a: &str, b: &str) -> Ordering {
    let key = |name: &str| {
        let lower = name.to_lowercase();
        match name.strip_prefix('.').unwrap_or(name).rsplit_once('.') {
            Some((_, ext)) => {
                let stem_len = lower.len() - ext.len() - 1;
                (
                    lower[..stem_len].to_string(),
                    lower[stem_len + 1..].to_string(),
                )
            }
            None => (lower, String::new()),
        }
    };
    key(a).cmp(&key(b)).then_with(|| a.cmp(b))
}
