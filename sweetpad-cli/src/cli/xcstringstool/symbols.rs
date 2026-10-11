//! `xcstringstool generate-symbols --language swift`: a `LocalizedStringResource`
//! for each string added to a catalog by hand, so code can write
//! `Text(.welcomeTitle)`.
//!
//! The symbol's name comes from the key. The format specifiers and anything
//! that can't be in a Swift identifier are dropped and split it into words,
//! a key with no lowercase letter is lowercased first, and the words are
//! joined in lower camel case. A string with arguments, from the
//! source-language text or else the key, is a function that takes them.
//! The strings of `Localizable` are members of `LocalizedStringResource`
//! itself; any other table's are in an enum named for it.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

use super::format::{self, Token};
use super::{Catalog, Problem};

/// The keywords a symbol is written in backticks for.
const KEYWORDS: &[&str] = &[
    "Any",
    "Self",
    "_",
    "as",
    "associatedtype",
    "await",
    "borrowing",
    "break",
    "case",
    "catch",
    "class",
    "consuming",
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
    "nonisolated",
    "open",
    "operator",
    "precedencegroup",
    "private",
    "protocol",
    "public",
    "repeat",
    "rethrows",
    "return",
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

/// The names no symbol can have, even in backticks: a member of every type.
const RESERVED: &[&str] = &["self", "Type", "Protocol"];

/// One symbol's argument.
#[derive(Debug, Clone)]
struct Argument {
    position: usize,
    /// A substitution's name, which labels it; a plain specifier has none.
    label: Option<String>,
    /// The specifier without its position, like `%lld`.
    spec: String,
    swift_type: &'static str,
}

struct Symbol<'a> {
    key: &'a str,
    name: String,
    comment: Option<&'a str>,
    arguments: Vec<Argument>,
}

impl Symbol<'_> {
    /// The name, with the argument labels of a function: `items(_:)`.
    fn signature(&self) -> String {
        if self.arguments.is_empty() {
            return self.name.clone();
        }
        let labels = self.arguments.iter().fold(String::new(), |mut out, a| {
            let _ = write!(out, "{}:", a.label.as_deref().unwrap_or("_"));
            out
        });
        format!("{}({labels})", self.name)
    }
}

/// The Swift file for `catalog`'s table, empty when no string has a symbol,
/// and why the strings that should have one don't.
pub fn generate(catalog: &Catalog, table: &str) -> (String, Vec<Problem>) {
    let mut problems = Vec::new();
    let mut symbols = Vec::new();
    let problem = |key: &str, message: String| Problem {
        scope: "allLocales".into(),
        key: key.to_string(),
        path: None,
        message,
    };
    for (key, entry) in &catalog.strings {
        if entry.get("extractionState").and_then(Value::as_str) != Some("manual") {
            continue;
        }
        let symbol = symbol_name(key).and_then(|name| {
            Ok(Symbol {
                key,
                name,
                comment: entry.get("comment").and_then(Value::as_str),
                arguments: arguments(entry, key, &catalog.source_language)?,
            })
        });
        match symbol {
            Ok(symbol) => symbols.push(symbol),
            Err(message) => problems.push(problem(key, message)),
        }
    }
    // A property and a function, or functions with other labels, can share
    // a name.
    let mut by_signature: HashMap<String, Vec<&str>> = HashMap::new();
    for symbol in &symbols {
        by_signature
            .entry(symbol.signature())
            .or_default()
            .push(symbol.key);
    }
    let duplicates: HashMap<&str, &str> = by_signature
        .values()
        .filter(|keys| keys.len() > 1)
        .flat_map(|keys| {
            keys.iter().map(|key| {
                (
                    *key,
                    *keys.iter().find(|other| *other != key).unwrap_or(key),
                )
            })
        })
        .collect();
    for (key, other) in &duplicates {
        problems.push(problem(
            key,
            format!(
                "This string key would generate the same symbol as “{other}”. Please change the \
                 key or disable symbol generation for this string."
            ),
        ));
    }
    symbols.retain(|s| !duplicates.contains_key(s.key));
    symbols.sort_by(|a, b| a.key.nfc().cmp(b.key.nfc()));
    (render(table, &symbols), problems)
}

/// Whether the symbol for a string takes arguments.
pub(super) fn takes_arguments(entry: &Value, key: &str, source: &str) -> bool {
    arguments(entry, key, source).is_ok_and(|a| !a.is_empty())
}

/// The symbol a key names.
fn symbol_name(key: &str) -> Result<String, String> {
    let reserved = || {
        "This key is too close to a reserved keyword in Swift. Please rename the key.".to_string()
    };
    if RESERVED.contains(&key) {
        return Err(reserved());
    }
    if KEYWORDS.contains(&key) {
        return Ok(backticked(key));
    }
    let name = camel_case(key);
    if name.is_empty() {
        return Err(
            "Unable to derive a symbol name from this key. It only contains characters \
                    that are invalid in Swift."
                .into(),
        );
    }
    if RESERVED.contains(&name.as_str()) {
        return Err(reserved());
    }
    if KEYWORDS.contains(&name.as_str()) {
        return Ok(backticked(&name));
    }
    Ok(name)
}

/// A keyword as an identifier, in backticks.
fn backticked(keyword: &str) -> String {
    let tick = '`';
    format!("{tick}{keyword}{tick}")
}

/// `text` in lower camel case: its words, split where an identifier can't
/// go on and where case or letters begin, the first lowercased and each
/// other one capitalized. A leading digit or underscore becomes `_`.
fn camel_case(text: &str) -> String {
    let mut plain = text.replace(['\'', '’'], "");
    for token in format::tokens(&plain.clone()).iter().rev() {
        plain.replace_range(token.range(), " ");
    }
    let leading_underscore = plain.starts_with('_');
    let mut words = Vec::new();
    for token in plain.split(|c: char| c == '_' || !identifier_character(c)) {
        if token.is_empty() {
            continue;
        }
        let token = if token.chars().any(char::is_lowercase) {
            token.to_string()
        } else {
            token.to_lowercase()
        };
        words.extend(split_words(&token));
    }
    let mut name = String::new();
    if leading_underscore {
        name.push('_');
    }
    for (index, word) in words.iter().enumerate() {
        if index == 0 && !leading_underscore {
            name.push_str(&word.to_lowercase());
        } else {
            name.push_str(&capitalized(word));
        }
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name
}

/// A token split where a lowercase letter meets an uppercase one, and where
/// a letter follows something that isn't one: `getHTTP` is `get` and `HTTP`,
/// `1st` is `1` and `st`.
fn split_words(token: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut previous: Option<char> = None;
    for c in token.chars() {
        let boundary = previous.is_some_and(|p| {
            (p.is_lowercase() && c.is_uppercase()) || (!p.is_alphabetic() && c.is_alphabetic())
        });
        if boundary && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.push(c);
        previous = Some(c);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// Whether `c` can be in a Swift identifier, by the language reference's
/// grammar for identifier heads and characters.
fn identifier_character(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || c == '_'
        || matches!(u32::from(c),
            0x00A8 | 0x00AA | 0x00AD | 0x00AF | 0x00B2..=0x00B5 | 0x00B7..=0x00BA
            | 0x00BC..=0x00BE | 0x00C0..=0x00D6 | 0x00D8..=0x00F6 | 0x00F8..=0x02FF
            | 0x0300..=0x036F | 0x0370..=0x167F | 0x1681..=0x180D | 0x180F..=0x1DBF
            | 0x1DC0..=0x1DFF | 0x1E00..=0x1FFF | 0x200B..=0x200D | 0x202A..=0x202E
            | 0x203F..=0x2040 | 0x2054 | 0x2060..=0x206F | 0x2070..=0x20CF | 0x20D0..=0x20FF
            | 0x2100..=0x218F | 0x2460..=0x24FF | 0x2776..=0x2793 | 0x2C00..=0x2DFF
            | 0x2E80..=0x2FFF | 0x3004..=0x3007 | 0x3021..=0x302F | 0x3031..=0x303F
            | 0x3040..=0xD7FF | 0xF900..=0xFD3D | 0xFD40..=0xFDCF | 0xFDF0..=0xFE1F
            | 0xFE20..=0xFE2F | 0xFE30..=0xFE44 | 0xFE47..=0xFFFD)
        || ((0x1_0000..=0xE_FFFD).contains(&u32::from(c)) && u32::from(c) & 0xFFFF <= 0xFFFD)
}

/// A string's arguments, in position order. They come from its text in
/// every language, the source language first, counting every case of its
/// variations and substitutions, and each position takes the first
/// argument found for it; a string with no localization takes its key's.
fn arguments(entry: &Value, key: &str, source: &str) -> Result<Vec<Argument>, String> {
    let mut found = Vec::new();
    match entry.get("localizations").and_then(Value::as_object) {
        Some(localizations) if !localizations.is_empty() => {
            let mut languages: Vec<&String> = localizations.keys().collect();
            languages.sort_by_key(|l| (l.as_str() != source, l.as_str()));
            for language in languages {
                let localization = &localizations[language.as_str()];
                let subs = localization.get("substitutions").and_then(Value::as_object);
                for text in texts(localization) {
                    collect(text, subs, &mut found)?;
                }
                for sub in subs.into_iter().flat_map(|s| s.values()) {
                    for text in texts(sub) {
                        collect(&text.replace("%arg", ""), None, &mut found)?;
                    }
                }
            }
        }
        _ => collect(key, None, &mut found)?,
    }
    found.sort_by_key(|a| a.position);
    Ok(found)
}

/// Every text in a localization or substitution: its plain string and each
/// case of its variations, at any depth.
fn texts(value: &Value) -> Vec<&str> {
    let mut out: Vec<&str> = value
        .get("stringUnit")
        .and_then(|u| u.get("value"))
        .and_then(Value::as_str)
        .into_iter()
        .collect();
    for variation in value
        .get("variations")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|v| v.values())
    {
        for case in variation.as_object().into_iter().flat_map(|c| c.values()) {
            out.extend(texts(case));
        }
    }
    out
}

/// Add the arguments in `text` whose positions aren't taken yet. A
/// substitution is an argument of the type its specifier names, labeled with
/// its name.
fn collect(
    text: &str,
    subs: Option<&serde_json::Map<String, Value>>,
    found: &mut Vec<Argument>,
) -> Result<(), String> {
    for (index, token) in format::tokens(text).iter().enumerate() {
        let argument = match token {
            Token::Substitution { position, name, .. } => {
                let sub = subs.and_then(|s| s.get(*name));
                let spec = sub
                    .and_then(|s| s.get("formatSpecifier"))
                    .and_then(Value::as_str)
                    .unwrap_or("@");
                let arg = sub
                    .and_then(|s| s.get("argNum"))
                    .and_then(Value::as_u64)
                    .and_then(|a| usize::try_from(a).ok());
                let position = position.or(arg).unwrap_or(index + 1);
                if found.iter().any(|a: &Argument| a.position == position) {
                    continue;
                }
                let spec = format!("%{spec}");
                Argument {
                    position,
                    label: Some(camel_case(name)),
                    swift_type: swift_type(&value_type(&spec)).ok_or_else(|| unsupported(&spec))?,
                    spec,
                }
            }
            Token::Spec { position, .. } => {
                let position = position.unwrap_or(index + 1);
                if found.iter().any(|a| a.position == position) {
                    continue;
                }
                Argument {
                    position,
                    label: None,
                    spec: token.unpositioned(),
                    swift_type: swift_type(&token.value_type())
                        .ok_or_else(|| unsupported(&token.unpositioned()))?,
                }
            }
        };
        found.push(argument);
    }
    Ok(())
}

/// The length and conversion of a specifier written on its own, like `%lld`.
fn value_type(spec: &str) -> String {
    format::tokens(spec)
        .first()
        .map(Token::value_type)
        .unwrap_or_default()
}

/// The Swift type an argument of a specifier's length and conversion takes.
fn swift_type(value_type: &str) -> Option<&'static str> {
    Some(match value_type {
        "d" => "Int32",
        "u" => "UInt32",
        "ld" | "lld" => "Int",
        "lu" | "llu" => "UInt",
        "f" => "Float",
        "lf" | "g" => "Double",
        "@" => "String",
        _ => return None,
    })
}

fn unsupported(spec: &str) -> String {
    format!("Unable to create a Swift type from the format specifier “{spec}”.")
}

/// `text` as a Swift string literal's contents.
fn escaped(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

fn render(table: &str, symbols: &[Symbol]) -> String {
    if symbols.is_empty() {
        return String::new();
    }
    let available = "@available(macOS 13, iOS 16, tvOS 16, watchOS 9, *)";
    let mut out = format!(
        "// \n\
         // GeneratedStringSymbols_{table}.swift\n\
         // Auto-Generated symbols for localized strings defined in “{table}.xcstrings”.\n\
         // \n\
         \n\
         import Foundation\n\
         \n\
         #if SWIFT_PACKAGE\n\
         private nonisolated let resourceBundle = Foundation.Bundle.module\n\
         {available}\n\
         private nonisolated let resourceBundleDescription = LocalizedStringResource.BundleDescription.atURL(resourceBundle.bundleURL)\n\
         #else\n\
         \n\
         private class ResourceBundleClass {{}}\n\
         {available}\n\
         private nonisolated let resourceBundleDescription = LocalizedStringResource.BundleDescription.forClass(ResourceBundleClass.self)\n\
         #endif\n\
         \n\
         {available}\n\
         nonisolated extension LocalizedStringResource {{\n"
    );
    let namespaced = table != "Localizable";
    let indent = if namespaced { "        " } else { "    " };
    if namespaced {
        let name = capitalized(&camel_case(table));
        let _ = write!(
            out,
            "    /// Namespace for strings in file “{table}.xcstrings”.\n    enum {name} {{\n"
        );
    }
    let members: Vec<String> = symbols
        .iter()
        .map(|symbol| member(symbol, table, indent))
        .collect();
    out.push_str(&members.join("\n\n"));
    out.push_str(if namespaced { "\n    }\n}" } else { "\n}" });
    out
}

fn member(symbol: &Symbol, table: &str, indent: &str) -> String {
    let doc = |text: &str| text.replace("*/", "* /");
    let mut out = format!("{indent}/**\n");
    if let Some(comment) = symbol.comment {
        let _ = write!(out, "{indent} {}\n{indent} \n", doc(comment));
    }
    let _ = write!(
        out,
        "{indent} Localized string for key “{}” in table “{table}.xcstrings”.\n{indent} */\n",
        doc(symbol.key)
    );
    let key = escaped(symbol.key);
    let tail = format!(
        "table: \"{}\", bundle: resourceBundleDescription",
        escaped(table)
    );
    if symbol.arguments.is_empty() {
        let _ = write!(
            out,
            "{indent}static var {}: LocalizedStringResource {{\n\
             {indent}    LocalizedStringResource(\"{key}\", {tail})\n\
             {indent}}}",
            symbol.name
        );
        return out;
    }
    let name_of = |a: &Argument| {
        a.label
            .clone()
            .unwrap_or_else(|| format!("arg{}", a.position))
    };
    let parameters: Vec<String> = symbol
        .arguments
        .iter()
        .map(|a| match &a.label {
            Some(label) => format!("{label}: {}", a.swift_type),
            None => format!("_ arg{}: {}", a.position, a.swift_type),
        })
        .collect();
    let default_value: String = symbol
        .arguments
        .iter()
        .map(|a| {
            if a.swift_type == "String" {
                format!("\\({})", name_of(a))
            } else {
                format!("\\({}, specifier: \"{}\")", name_of(a), a.spec)
            }
        })
        .collect();
    let _ = write!(
        out,
        "{indent}static func {}({}) -> LocalizedStringResource {{\n\
         {indent}    LocalizedStringResource(\"{key}\", defaultValue: \"{default_value}\", {tail})\n\
         {indent}}}",
        symbol.name,
        parameters.join(", ")
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_xcstringstool() {
        for (key, name) in [
            ("account.name.on-my-device", "accountNameOnMyDevice"),
            ("HTTPServerURL", "httpserverURL"),
            ("UPPER_CASE_KEY", "upperCaseKey"),
            ("1st place", "_1StPlace"),
            ("_leading", "_Leading"),
            ("%lld items", "items"),
            ("a%@b", "aB"),
            ("don't stop", "dontStop"),
            ("Any", "`Any`"),
            ("Class", "`class`"),
        ] {
            assert_eq!(symbol_name(key).as_deref(), Ok(name), "{key}");
        }
        assert!(symbol_name("self").is_err());
        assert!(symbol_name("!!!").is_err());
    }

    #[test]
    fn a_namespace_is_the_table_in_upper_camel_case() {
        assert_eq!(capitalized(&camel_case("my-table")), "MyTable");
        assert_eq!(capitalized(&camel_case("2Table")), "_2Table");
    }

    #[test]
    fn substitution_arguments_take_their_names() {
        let entry: Value = serde_json::from_str(
            r#"{"localizations":{"en":{"stringUnit":{"value":"%@ got %#@n@"},
                "substitutions":{"n":{"argNum":2,"formatSpecifier":"lld"}}}}}"#,
        )
        .unwrap();
        let arguments = arguments(&entry, "k", "en").unwrap();
        let shown: Vec<(usize, Option<String>, &str)> = arguments
            .iter()
            .map(|a| (a.position, a.label.clone(), a.swift_type))
            .collect();
        assert_eq!(
            shown,
            [(1, None, "String"), (2, Some("n".to_string()), "Int")]
        );
    }

    #[test]
    fn identifier_characters_follow_the_grammar() {
        assert!(identifier_character('é'));
        assert!(identifier_character('😀'));
        assert!(identifier_character('€'));
        assert!(!identifier_character('\u{e000}'));
        assert!(!identifier_character('!'));
        assert!(!identifier_character('\u{f0000}'));
    }
}
