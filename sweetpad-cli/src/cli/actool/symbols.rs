//! The asset symbols actool generates for a target's catalogs: Swift's
//! `ColorResource`/`ImageResource` constants and their AppKit, UIKit and
//! SwiftUI accessors, Objective-C's `ACColorName…` constants, and Xcode's
//! symbol index. Written the way Xcode 27's actool writes them, which the
//! golden test against `fixtures/actool` holds this to.

use std::fmt::Write as _;

use serde_json::{Value, json};

use super::catalog::{Asset, Catalog, Folder, Kind};

/// The frameworks that get accessors.
#[derive(Debug, Clone, Copy)]
pub struct Frameworks {
    pub appkit: bool,
    pub uikit: bool,
    pub swiftui: bool,
}

impl Default for Frameworks {
    fn default() -> Self {
        Frameworks {
            appkit: true,
            uikit: true,
            swiftui: true,
        }
    }
}

#[derive(Debug)]
pub struct Options {
    pub bundle_id: String,
    /// Accessors on `Color`, `UIColor`, `NSImage` and the rest, as well as the
    /// resource constants.
    pub extensions: bool,
    pub frameworks: Frameworks,
    /// The devices the build targets (`iphone`, `ipad`, `mac`, …). An asset
    /// that leaves one out can be thinned away on it, so its symbol is
    /// optional.
    pub target_devices: Vec<String>,
}

pub struct Symbols {
    pub swift: String,
    pub objc: String,
    pub index: Value,
}

pub fn generate(catalogs: &[Catalog], options: &Options) -> Symbols {
    let colors = Section::build(catalogs, Kind::Color, options);
    let images = Section::build(catalogs, Kind::Image, options);
    Symbols {
        swift: swift(&colors, &images, options),
        objc: objc(&colors, &images, options),
        index: index(&colors, &images),
    }
}

/// One kind's symbols: the scoped tree Swift nests in namespace enums, and
/// the flat list the header and the index use.
struct Section<'a> {
    kind: Kind,
    tree: Vec<Item<'a>>,
    flat: Vec<Flat<'a>>,
}

enum Item<'a> {
    Symbol(Symbol<'a>),
    /// A name that resolves to a symbol taken earlier in its scope.
    Duplicate {
        full_name: String,
        symbol: String,
    },
    Namespace {
        full_name: String,
        type_name: String,
        items: Vec<Item<'a>>,
    },
}

struct Symbol<'a> {
    full_name: String,
    /// The identifier, without backticks.
    plain: String,
    thinnable: bool,
    /// How an accessor reaches the constant: `.Icons.tint`.
    reference: String,
    _asset: &'a Asset,
}

struct Flat<'a> {
    catalog: &'a Catalog,
    asset: &'a Asset,
    namespaces: Vec<String>,
}

/// An asset in a scope, before it gets its symbol.
struct Scoped<'a> {
    catalog: &'a Catalog,
    asset: &'a Asset,
    namespaces: Vec<String>,
    thinnable: bool,
}

impl<'a> Section<'a> {
    fn build(catalogs: &'a [Catalog], kind: Kind, options: &Options) -> Section<'a> {
        let mut scoped = Vec::new();
        for catalog in catalogs {
            for (chain, asset) in catalog.assets() {
                if asset.kind != kind || !asset.has_content {
                    continue;
                }
                let applies = |device: &String| {
                    asset
                        .idioms
                        .iter()
                        .any(|idiom| idiom == "universal" || idiom == device)
                };
                if !options.target_devices.iter().any(applies) {
                    continue;
                }
                scoped.push(Scoped {
                    catalog,
                    asset,
                    namespaces: namespaces(&chain),
                    thinnable: !options.target_devices.iter().all(applies),
                });
            }
        }
        let mut flat: Vec<Flat> = scoped
            .iter()
            .map(|s| Flat {
                catalog: s.catalog,
                asset: s.asset,
                namespaces: s.namespaces.clone(),
            })
            .collect();
        flat.sort_by_cached_key(|f| decomposed(&f.asset.name));
        // Without the accessors there is nothing to resolve a thinnable
        // constant through, so actool leaves those assets out.
        let in_tree: Vec<&Scoped> = scoped
            .iter()
            .filter(|s| options.extensions || !s.thinnable)
            .collect();
        let tree = scope(&in_tree, 0, kind, "");
        Section { kind, tree, flat }
    }

    fn kind_name(&self) -> &'static str {
        match self.kind {
            Kind::Color => "color",
            _ => "image",
        }
    }

    fn type_name(&self) -> &'static str {
        match self.kind {
            Kind::Color => "ColorResource",
            _ => "ImageResource",
        }
    }

    fn is_empty(&self) -> bool {
        self.flat.is_empty()
    }
}

fn namespaces(chain: &[&Folder]) -> Vec<String> {
    chain
        .iter()
        .filter(|f| f.namespace)
        .map(|f| f.name.clone())
        .collect()
}

/// The items of the scope `depth` namespaces down, in name order, with
/// each name's symbol and the duplicates after the first.
fn scope<'a>(assets: &[&Scoped<'a>], depth: usize, kind: Kind, prefix: &str) -> Vec<Item<'a>> {
    enum Entry<'b, 'a> {
        Asset(&'b Scoped<'a>),
        Namespace(String),
    }
    let mut entries: Vec<(String, Entry)> = Vec::new();
    for asset in assets {
        match asset.namespaces.get(depth) {
            None => entries.push((asset.asset.name.clone(), Entry::Asset(asset))),
            Some(name) => {
                if !entries
                    .iter()
                    .any(|(n, e)| n == name && matches!(e, Entry::Namespace(_)))
                {
                    entries.push((name.clone(), Entry::Namespace(name.clone())));
                }
            }
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let suffix = match kind {
        Kind::Color => "Color",
        _ => "Image",
    };
    let mut taken: Vec<String> = Vec::new();
    let mut items = Vec::new();
    for (name, entry) in entries {
        match entry {
            Entry::Asset(scoped) => {
                let full_name = full_name(&scoped.namespaces, &scoped.asset.name);
                let plain = swift_name(&scoped.asset.name, suffix);
                if taken.contains(&plain) {
                    items.push(Item::Duplicate {
                        full_name,
                        symbol: plain,
                    });
                    continue;
                }
                taken.push(plain.clone());
                items.push(Item::Symbol(Symbol {
                    reference: format!("{prefix}.{}", escaped(&plain)),
                    full_name,
                    plain,
                    thinnable: scoped.thinnable,
                    _asset: scoped.asset,
                }));
            }
            Entry::Namespace(folder) => {
                let inner: Vec<&Scoped> = assets
                    .iter()
                    .copied()
                    .filter(|a| a.namespaces.get(depth) == Some(&folder))
                    .collect();
                let type_name = type_name(&folder);
                let path: Vec<String> = inner[0].namespaces[..=depth].to_vec();
                items.push(Item::Namespace {
                    full_name: full_name(&path[..depth], &path[depth]),
                    items: scope(&inner, depth + 1, kind, &format!("{prefix}.{type_name}")),
                    type_name,
                });
                let _ = name;
            }
        }
    }
    items
}

/// An asset's name under its namespaces, escaped for the string literals
/// and comments it's printed in.
fn full_name(namespaces: &[String], name: &str) -> String {
    let joined = namespaces
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(name))
        .collect::<Vec<_>>()
        .join("/");
    joined
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\'', "\\'")
}

/// An asset name's words: its runs of letters and digits, broken where the
/// case turns upward, where an acronym meets a capitalized word (`URLColor`
/// is `URL` and `Color`), and between letters and digits.
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for run in name.split(|c: char| !c.is_alphanumeric()) {
        let chars: Vec<char> = run.chars().collect();
        let mut start = 0;
        for i in 1..chars.len() {
            let (prev, cur) = (chars[i - 1], chars[i]);
            let next_lower = chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            let boundary = (prev.is_lowercase() && cur.is_uppercase())
                || (prev.is_uppercase() && cur.is_uppercase() && next_lower)
                || (prev.is_alphabetic() && cur.is_numeric())
                || (prev.is_numeric() && cur.is_alphabetic());
            if boundary {
                out.push(chars[start..i].iter().collect());
                start = i;
            }
        }
        if start < chars.len() {
            out.push(chars[start..].iter().collect());
        }
    }
    out
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn all_caps(words: &[String]) -> bool {
    !words.iter().any(|w| w.chars().any(char::is_lowercase))
}

/// The Swift identifier for an asset: camel case less a trailing `Color` or
/// `Image`, or an all-caps name kept as it is with `_` between its words.
fn swift_name(name: &str, suffix: &str) -> String {
    let mut words = words(name);
    if words.len() > 1 && words.last().is_some_and(|w| w == suffix) {
        words.pop();
    }
    let mut out = if all_caps(&words) {
        words.join("_")
    } else {
        words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                if i == 0 {
                    w.to_lowercase()
                } else {
                    capitalized(w)
                }
            })
            .collect()
    };
    if out.starts_with(|c: char| c.is_numeric()) {
        out.insert(0, '_');
    }
    out
}

/// A namespace's enum: its words capitalized and run together.
fn type_name(name: &str) -> String {
    let out: String = words(name).iter().map(|w| capitalized(w)).collect();
    if out.starts_with(|c: char| c.is_numeric()) {
        format!("_{out}")
    } else {
        out
    }
}

/// The Objective-C spelling of a name: its words capitalized, ASCII only.
/// An all-caps name of several words keeps its case and joins them with
/// `_`, after one in front.
fn objc_name(name: &str) -> String {
    let words = words(name);
    let out = if all_caps(&words) && words.len() > 1 {
        format!("_{}", words.join("_"))
    } else {
        words.iter().map(|w| capitalized(w)).collect()
    };
    out.chars().filter(char::is_ascii).collect()
}

/// Swift's reserved words, which an identifier spells in backticks.
const KEYWORDS: &[&str] = &[
    "Any",
    "Self",
    "_",
    "as",
    "associatedtype",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "default",
    "defer",
    "deinit",
    "do",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "false",
    "fileprivate",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "init",
    "inout",
    "internal",
    "is",
    "let",
    "nil",
    "operator",
    "precedencegroup",
    "private",
    "protocol",
    "public",
    "repeat",
    "rethrows",
    "return",
    "self",
    "static",
    "struct",
    "subscript",
    "super",
    "switch",
    "throw",
    "throws",
    "true",
    "try",
    "typealias",
    "var",
    "where",
    "while",
];

fn escaped(identifier: &str) -> String {
    if KEYWORDS.contains(&identifier) {
        let tick = '`';
        format!("{tick}{identifier}{tick}")
    } else {
        identifier.to_string()
    }
}

/// A sort key that orders accented Latin letters right after their base
/// letter, as Foundation's string comparison does by decomposing them.
fn decomposed(name: &str) -> Vec<u32> {
    // The base letter of each of U+00C0 to U+00FF, or a space for the ones
    // that don't decompose.
    const BASES: &str = "AAAAAA CEEEEIIII NOOOOO OUUUUY  aaaaaa ceeeeiiii nooooo ouuuuy y";
    let mut key = Vec::new();
    for c in name.chars() {
        let base = (c as u32)
            .checked_sub(0xC0)
            .and_then(|i| BASES.chars().nth(i as usize))
            .filter(|b| *b != ' ');
        match base {
            Some(base) => key.extend([base as u32, 0x300]),
            None => key.push(c as u32),
        }
    }
    key
}

/// The type an extension extends, and how it reads in a conflict warning.
#[derive(Clone, Copy, PartialEq)]
enum Host {
    NsColor,
    UiColor,
    Color,
    ShapeStyle,
    NsImage,
    UiImage,
}

impl Host {
    fn short(self) -> &'static str {
        match self {
            Host::NsColor => "NSColor",
            Host::UiColor => "UIColor",
            Host::Color | Host::ShapeStyle => "Color",
            Host::NsImage => "NSImage",
            Host::UiImage => "UIImage",
        }
    }

    fn qualified(self) -> &'static str {
        match self {
            Host::NsColor => "AppKit.NSColor",
            Host::UiColor => "UIKit.UIColor",
            Host::Color | Host::ShapeStyle => "SwiftUI.Color",
            Host::NsImage => "AppKit.NSImage",
            Host::UiImage => "UIKit.UIImage",
        }
    }

    /// The members an accessor of the same name would collide with, which
    /// actool replaces with a warning. Taken from Xcode 27's actool.
    fn taken(self) -> &'static [&'static str] {
        match self {
            Host::NsColor => NSCOLOR_TAKEN,
            Host::UiColor => UICOLOR_TAKEN,
            Host::Color | Host::ShapeStyle => COLOR_TAKEN,
            Host::NsImage => &[],
            Host::UiImage => UIIMAGE_TAKEN,
        }
    }

    /// The `#if` around an accessor's body, for the platforms where the type
    /// can't load the resource.
    fn guard(self) -> Option<&'static str> {
        match self {
            Host::NsColor | Host::NsImage => Some("!targetEnvironment(macCatalyst)"),
            Host::UiColor | Host::UiImage => Some("!os(watchOS)"),
            Host::Color | Host::ShapeStyle => None,
        }
    }
}

const COLOR_TAKEN: &[&str] = &[
    "black",
    "blue",
    "brown",
    "clear",
    "cyan",
    "gray",
    "green",
    "indigo",
    "mint",
    "orange",
    "pink",
    "primary",
    "purple",
    "red",
    "secondary",
    "teal",
    "white",
    "yellow",
];

const NSCOLOR_TAKEN: &[&str] = &[
    "black",
    "blue",
    "brown",
    "clear",
    "cyan",
    "darkGray",
    "gray",
    "green",
    "lightGray",
    "magenta",
    "orange",
    "purple",
    "red",
    "scrubberTexturedBackground",
    "systemBlue",
    "systemBrown",
    "systemCyan",
    "systemGray",
    "systemGreen",
    "systemIndigo",
    "systemMint",
    "systemOrange",
    "systemPink",
    "systemPurple",
    "systemRed",
    "systemTeal",
    "systemYellow",
    "white",
    "yellow",
];

const UICOLOR_TAKEN: &[&str] = &[
    "black",
    "blue",
    "brown",
    "clear",
    "cyan",
    "darkGray",
    "darkText",
    "gray",
    "green",
    "groupTableViewBackground",
    "label",
    "lightGray",
    "lightText",
    "link",
    "magenta",
    "opaqueSeparator",
    "orange",
    "placeholderText",
    "purple",
    "quaternaryLabel",
    "quaternarySystemFill",
    "red",
    "secondaryLabel",
    "secondarySystemBackground",
    "secondarySystemFill",
    "secondarySystemGroupedBackground",
    "separator",
    "systemBackground",
    "systemBlue",
    "systemBrown",
    "systemCyan",
    "systemFill",
    "systemGray",
    "systemGray2",
    "systemGray3",
    "systemGray4",
    "systemGray5",
    "systemGray6",
    "systemGreen",
    "systemGroupedBackground",
    "systemIndigo",
    "systemMint",
    "systemOrange",
    "systemPink",
    "systemPurple",
    "systemRed",
    "systemTeal",
    "systemYellow",
    "tertiaryLabel",
    "tertiarySystemBackground",
    "tertiarySystemFill",
    "tertiarySystemGroupedBackground",
    "white",
    "yellow",
];

const UIIMAGE_TAKEN: &[&str] = &["actions", "add", "checkmark", "remove", "strokedCheckmark"];

const AVAILABLE_ALL: &str = "@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)";

fn swift(colors: &Section, images: &Section, options: &Options) -> String {
    let f = options.frameworks;
    let ext = options.extensions;
    let mut out = String::from("import Foundation\n");
    for (on, module) in [
        (ext && f.appkit, "AppKit"),
        (ext && f.uikit, "UIKit"),
        (ext && f.swiftui, "SwiftUI"),
        (true, "DeveloperToolsSupport"),
    ] {
        if on {
            let _ = write!(out, "#if canImport({module})\nimport {module}\n#endif\n");
        }
    }
    out.push_str(
        "\n#if SWIFT_PACKAGE\n\
         private let resourceBundle = Foundation.Bundle.module\n\
         #else\n\
         private class ResourceBundleClass {}\n\
         private let resourceBundle = Foundation.Bundle(for: ResourceBundleClass.self)\n\
         #endif\n\n",
    );
    for section in [colors, images] {
        let title = match section.kind {
            Kind::Color => "Color",
            _ => "Image",
        };
        let _ = write!(
            out,
            "// MARK: - {title} Symbols -\n\n{AVAILABLE_ALL}\nextension DeveloperToolsSupport.{} {{\n{}\n}}\n\n",
            section.type_name(),
            body(&section.tree, 1, &|item, indent| Some(resource_item(
                section, item, indent
            ))),
        );
    }
    if !ext {
        return out;
    }

    out.push_str("// MARK: - Color Symbol Extensions -\n\n");
    if f.appkit {
        out.push_str(&accessor_extension(colors, Host::NsColor));
    }
    if f.uikit {
        out.push_str(&accessor_extension(colors, Host::UiColor));
    }
    if f.swiftui {
        let _ = write!(
            out,
            "#if canImport(SwiftUI)\n{AVAILABLE_ALL}\nextension SwiftUI.Color {{\n{}\n}}\n\n\
             {AVAILABLE_ALL}\nextension SwiftUI.ShapeStyle where Self == SwiftUI.Color {{\n{}\n}}\n#endif\n\n",
            body(&colors.tree, 1, &|item, indent| accessor_item(
                colors,
                Host::Color,
                item,
                indent
            )),
            body(&colors.tree, 1, &|item, indent| accessor_item(
                colors,
                Host::ShapeStyle,
                item,
                indent
            )),
        );
    }
    out.push_str("// MARK: - Image Symbol Extensions -\n\n");
    if f.appkit {
        out.push_str(&accessor_extension(images, Host::NsImage));
    }
    if f.uikit {
        out.push_str(&accessor_extension(images, Host::UiImage));
    }
    out.push_str("// MARK: - Thinnable Asset Support -\n\n");
    out.push_str(&thinnable_support(Kind::Color, f, !colors.is_empty()));
    out.push_str(&thinnable_support(Kind::Image, f, !images.is_empty()));
    out
}

/// A body's items, each with a blank line before and after.
fn body(items: &[Item], depth: usize, render: &dyn Fn(&Item, &str) -> Option<String>) -> String {
    let indent = "    ".repeat(depth);
    items
        .iter()
        .filter_map(|item| render(item, &indent))
        .fold(String::new(), |mut out, text| {
            let _ = write!(out, "\n{text}\n");
            out
        })
}

fn namespace_text(full_name: &str, type_name: &str, inner: &str, indent: &str) -> String {
    format!(
        "{indent}/// The \"{full_name}\" asset catalog resource namespace.\n\
         {indent}enum {type_name} {{\n{inner}\n{indent}}}"
    )
}

fn depth_of(indent: &str) -> usize {
    indent.len() / 4
}

fn resource_item(section: &Section, item: &Item, indent: &str) -> String {
    let kind = section.kind_name();
    match item {
        Item::Symbol(s) => {
            let (watch, label) = if s.thinnable {
                (
                    format!("{indent}@available(watchOS, unavailable)\n"),
                    "thinnableName",
                )
            } else {
                (String::new(), "name")
            };
            format!(
                "{indent}/// The \"{}\" asset catalog {kind} resource.\n{watch}\
                 {indent}static let {} = DeveloperToolsSupport.{}({label}: \"{}\", bundle: resourceBundle)",
                s.full_name,
                escaped(&s.plain),
                section.type_name(),
                s.full_name,
            )
        }
        Item::Duplicate { full_name, symbol } => format!(
            "{indent}#warning(\"The \\\"{full_name}\\\" {kind} asset name resolves to the symbol \\\"{symbol}\\\" which already exists. Try renaming the asset.\")"
        ),
        Item::Namespace {
            full_name,
            type_name,
            items,
        } => namespace_text(
            full_name,
            type_name,
            &body(items, depth_of(indent) + 1, &|item, indent| {
                Some(resource_item(section, item, indent))
            }),
            indent,
        ),
    }
}

fn accessor_extension(section: &Section, host: Host) -> String {
    let module = if matches!(host, Host::NsColor | Host::NsImage) {
        "AppKit"
    } else {
        "UIKit"
    };
    let available = match module {
        "AppKit" => "@available(macOS 14.0, *)\n@available(macCatalyst, unavailable)",
        _ => "@available(iOS 17.0, tvOS 17.0, *)\n@available(watchOS, unavailable)",
    };
    format!(
        "#if canImport({module})\n{available}\nextension {} {{\n{}\n}}\n#endif\n\n",
        host.qualified(),
        body(&section.tree, 1, &|item, indent| accessor_item(
            section, host, item, indent
        )),
    )
}

fn accessor_item(section: &Section, host: Host, item: &Item, indent: &str) -> Option<String> {
    let kind = section.kind_name();
    match item {
        Item::Duplicate { .. } => None,
        Item::Namespace {
            full_name,
            type_name,
            items,
        } => {
            // A constrained protocol extension can't nest an enum.
            if host == Host::ShapeStyle {
                return None;
            }
            Some(namespace_text(
                full_name,
                type_name,
                &body(items, depth_of(indent) + 1, &|item, indent| {
                    accessor_item(section, host, item, indent)
                }),
                indent,
            ))
        }
        Item::Symbol(s) => {
            let top_level = depth_of(indent) == 1;
            if top_level && host.taken().contains(&s.plain.as_str()) {
                if host == Host::ShapeStyle {
                    return None;
                }
                return Some(format!(
                    "{indent}#warning(\"The \\\"{}\\\" {kind} asset name resolves to a conflicting {} symbol \\\"{}\\\". Try renaming the asset.\")",
                    s.full_name,
                    host.short(),
                    s.plain,
                ));
            }
            let name = escaped(&s.plain);
            let (optional, init) = if s.thinnable {
                ("?", format!(".init(thinnableResource: {})", s.reference))
            } else {
                ("", format!(".init(resource: {})", s.reference))
            };
            let doc = format!(
                "{indent}/// The \"{}\" asset catalog {kind}.\n",
                s.full_name
            );
            if let Some(guard) = host.guard() {
                return Some(format!(
                    "{doc}{indent}static var {name}: {}{optional} {{\n\
                     #if {guard}\n\
                     {indent}    {init}\n\
                     #else\n\
                     {indent}    .init()\n\
                     #endif\n\
                     {indent}}}",
                    host.qualified(),
                ));
            }
            // SwiftUI's accessors fit on one line, and a thinnable one is
            // marked unavailable on watchOS.
            let (watch, init) = if s.thinnable {
                (format!("{indent}@available(watchOS, unavailable)\n"), init)
            } else {
                (String::new(), format!(".init({})", s.reference))
            };
            Some(format!(
                "{doc}{watch}{indent}static var {name}: SwiftUI.Color{optional} {{ {init} }}"
            ))
        }
    }
}

/// The private initializers a thinnable constant and its accessors go
/// through.
fn thinnable_support(kind: Kind, f: Frameworks, has_symbols: bool) -> String {
    let (resource, appkit_check, uikit_check, nstype, uitype) = match kind {
        Kind::Color => (
            "ColorResource",
            "AppKit.NSColor(named: NSColor.Name(thinnableName), bundle: bundle) != nil",
            "UIKit.UIColor(named: thinnableName, in: bundle, compatibleWith: nil) != nil",
            "AppKit.NSColor",
            "UIKit.UIColor",
        ),
        _ => (
            "ImageResource",
            "bundle.image(forResource: NSImage.Name(thinnableName)) != nil",
            "UIKit.UIImage(named: thinnableName, in: bundle, compatibleWith: nil) != nil",
            "AppKit.NSImage",
            "UIKit.UIImage",
        ),
    };
    let branch = |condition: &str, check: &str| {
        format!(
            "#{condition}\n        if {check} {{\n            self.init(name: thinnableName, bundle: bundle)\n        }} else {{\n            return nil\n        }}\n"
        )
    };
    let lookup = match (f.appkit, f.uikit) {
        (false, false) => "        return nil\n".to_string(),
        (appkit, uikit) => {
            let mut text = String::new();
            if appkit {
                text += &branch("if canImport(AppKit) && os(macOS)", appkit_check);
            }
            if uikit {
                let condition = if appkit {
                    "elseif canImport(UIKit) && !os(watchOS)"
                } else {
                    "if canImport(UIKit) && !os(watchOS)"
                };
                text += &branch(condition, uikit_check);
            }
            text + "#else\n        return nil\n#endif\n"
        }
    };
    let mut out = format!(
        "{AVAILABLE_ALL}\n@available(watchOS, unavailable)\nextension DeveloperToolsSupport.{resource} {{\n\n    \
         private init?(thinnableName: Swift.String, bundle: Foundation.Bundle) {{\n{lookup}    }}\n\n}}\n\n"
    );
    let convenience = |guard: &str, resource: &str| {
        format!(
            "    private convenience init?(thinnableResource: DeveloperToolsSupport.{resource}?) {{\n\
             #if {guard}\n        if let resource = thinnableResource {{\n            self.init(resource: resource)\n        }} else {{\n            return nil\n        }}\n\
             #else\n        return nil\n#endif\n    }}\n"
        )
    };
    if f.appkit && has_symbols {
        let _ = write!(
            out,
            "#if canImport(AppKit)\n@available(macOS 14.0, *)\n@available(macCatalyst, unavailable)\nextension {nstype} {{\n\n{}\n}}\n#endif\n\n",
            convenience("!targetEnvironment(macCatalyst)", resource),
        );
    }
    if f.uikit {
        let _ = write!(
            out,
            "#if canImport(UIKit)\n@available(iOS 17.0, tvOS 17.0, *)\n@available(watchOS, unavailable)\nextension {uitype} {{\n\n{}\n}}\n#endif\n\n",
            convenience("!os(watchOS)", resource),
        );
    }
    if kind == Kind::Color && f.swiftui {
        let init = "    private init?(thinnableResource: DeveloperToolsSupport.ColorResource?) {\n        if let resource = thinnableResource {\n            self.init(resource)\n        } else {\n            return nil\n        }\n    }\n";
        let _ = write!(
            out,
            "#if canImport(SwiftUI)\n{AVAILABLE_ALL}\nextension SwiftUI.Color {{\n\n{init}\n}}\n\n\
             {AVAILABLE_ALL}\nextension SwiftUI.ShapeStyle where Self == SwiftUI.Color {{\n\n{init}\n}}\n#endif\n\n"
        );
    }
    out
}

fn objc(colors: &Section, images: &Section, options: &Options) -> String {
    let mut out = String::from(
        "#import <Foundation/Foundation.h>\n\n\
         #if __has_attribute(swift_private)\n\
         #define AC_SWIFT_PRIVATE __attribute__((swift_private))\n\
         #else\n\
         #define AC_SWIFT_PRIVATE\n\
         #endif\n\n",
    );
    if !colors.is_empty() {
        let _ = write!(
            out,
            "/// The resource bundle ID.\nstatic NSString * const ACBundleID AC_SWIFT_PRIVATE = @\"{}\";\n\n",
            options.bundle_id
        );
    }
    let mut taken: Vec<String> = Vec::new();
    for section in [colors, images] {
        let prefix = objc_prefix(section.kind);
        for f in &section.flat {
            let full_name = full_name(&f.namespaces, &f.asset.name);
            let symbol: String = f
                .namespaces
                .iter()
                .map(|n| objc_name(n))
                .chain(std::iter::once(objc_name(&f.asset.name)))
                .collect();
            let symbol = format!("{prefix}{symbol}");
            if taken.contains(&symbol) {
                let _ = write!(
                    out,
                    "#warning The \"{full_name}\" {} asset name resolves to the symbol \"{symbol}\" which already exists. Try renaming the asset.\n\n",
                    section.kind_name(),
                );
                continue;
            }
            taken.push(symbol.clone());
            let _ = write!(
                out,
                "/// The \"{full_name}\" asset catalog {} resource.\nstatic NSString * const {symbol} AC_SWIFT_PRIVATE = @\"{full_name}\";\n\n",
                section.kind_name(),
            );
        }
    }
    out.push_str("#undef AC_SWIFT_PRIVATE\n");
    out
}

fn objc_prefix(kind: Kind) -> &'static str {
    match kind {
        Kind::Color => "ACColorName",
        _ => "ACImageName",
    }
}

fn index(colors: &Section, images: &Section) -> Value {
    let entries = |section: &Section| -> Value {
        let suffix = match section.kind {
            Kind::Color => "Color",
            _ => "Image",
        };
        section
            .flat
            .iter()
            .map(|f| {
                json!({
                    "catalogPath": f.catalog.path.display().to_string(),
                    "objcSymbol": format!("{}{}", objc_prefix(section.kind), objc_name(&f.asset.name)),
                    "swiftSymbol": escaped(&swift_name(&f.asset.name, suffix)),
                    "relativePath": f.asset.relative_path,
                })
            })
            .collect()
    };
    json!({
        "colors": entries(colors),
        "images": entries(images),
        "symbols": [],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_actool() {
        for (name, swift) in [
            ("BrandRed", "brandRed"),
            ("URLColor", "URL"),
            ("myURLColor", "myURL"),
            ("1stColor", "_1St"),
            ("HTML5Color", "HTML_5"),
            ("ABCdef", "abCdef"),
            ("UPPER_snake", "upperSnake"),
            ("x1y2", "x1Y2"),
            ("Color", "color"),
            ("emoji🎉", "emoji"),
            ("Ünïcode", "ünïcode"),
        ] {
            assert_eq!(swift_name(name, "Color"), swift, "{name}");
        }
        assert_eq!(objc_name("A_B"), "_A_B");
        assert_eq!(objc_name("Ünïcode"), "ncode");
        assert_eq!(type_name("my icons"), "MyIcons");
    }
}
