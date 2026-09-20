//! Parser for `project.xcproj`, the JSON-shaped project document Xcode 27.2
//! writes in place of `project.pbxproj`.
//!
//! **It is not JSON.** Xcode's canonical output puts a trailing comma after
//! every array element and object member, and the format accepts `//` and
//! `/* … */` comments, which the writer then drops. A JSON library rejects a
//! file Xcode just wrote, so this is an Apple project-domain format that
//! happens to look standard — hand-rolled here for the same reason `pbxproj`
//! and `xcconfig` are (`DOCS.md` §3).
//!
//! Numbers keep their source lexeme rather than becoming an `f64`. The format
//! uses them for a handful of flags and version components, and a value that
//! is read and written back should come out as it went in.
//!
//! The document's shape is Apple's, published as
//! [`apple/xcode-project-format`](https://github.com/apple/xcode-project-format);
//! this module parses the syntax and leaves the schema to its callers.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use crate::file_cache::ParseCache;

/// An insertion-order-preserving string-keyed map.
///
/// Xcode's printer emits object members in a fixed order that is neither
/// alphabetical throughout nor the order a decoder would reconstruct, so a
/// faithful re-serialization has to replay the order the source used.
#[derive(Debug, Clone, Default)]
pub struct Object {
    entries: Vec<(String, Value)>,
    index: HashMap<String, usize>,
}

impl Object {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.index.get(key).map(|&i| &self.entries[i].1)
    }

    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.index.contains_key(key)
    }

    /// Insert a key/value pair. A duplicate key replaces the value in place,
    /// keeping the key's original position — last value wins, as in JSON.
    pub fn insert(&mut self, key: String, value: Value) {
        if let Some(&i) = self.index.get(&key) {
            self.entries[i].1 = value;
        } else {
            self.index.insert(key.clone(), self.entries.len());
            self.entries.push((key, value));
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl<'a> IntoIterator for &'a Object {
    type Item = (&'a str, &'a Value);
    type IntoIter = Box<dyn Iterator<Item = (&'a str, &'a Value)> + 'a>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

/// Order is formatting, not data, so two objects with the same pairs are equal.
impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(k, v)| other.get(k).is_some_and(|o| o == v))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number's source lexeme, e.g. `"110"` or `"1.5e3"`.
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(Object),
}

impl Value {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(n) => n.parse().ok(),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => n.parse().ok(),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Look up a key when this value is an object.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object().and_then(|o| o.get(key))
    }
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at line {}, column {}",
            self.message, self.line, self.column
        )
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Parse(ParseError),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<ParseError> for Error {
    fn from(e: ParseError) -> Self {
        Error::Parse(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Parse(e) => write!(f, "parse error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// The file Xcode writes inside an `.xcodeproj` when the project uses this
/// format, beside (or instead of) `project.pbxproj`.
pub const DOCUMENT_NAME: &str = "project.xcproj";

pub fn parse(input: &str) -> Result<Value, ParseError> {
    let mut p = Parser::new(input);
    p.skip_trivia();
    let v = p.parse_value(0)?;
    p.skip_trivia();
    if p.peek().is_some() {
        return Err(p.error("trailing data after root value"));
    }
    Ok(v)
}

pub fn parse_file(path: &Path) -> Result<Value, Error> {
    let s = fs::read_to_string(path)?;
    Ok(parse(&s)?)
}

/// Process-global cache of parsed documents, validated by `(len, mtime)`.
static CACHE: LazyLock<ParseCache<Value>> = LazyLock::new(ParseCache::new);

/// Like [`parse_file`] but served from an in-memory, mtime-validated cache.
pub fn parse_file_cached(path: &Path) -> Result<Arc<Value>, Error> {
    CACHE.get_or_parse(path, parse_file)
}

/// Recursion guard for nested arrays and objects. A project's file tree is the
/// deepest thing in a real document and nests with the directory tree; 512 is
/// far past that and keeps a pathological `[[[[[…` input off the stack.
const MAX_DEPTH: usize = 512;

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        let (line, column) = self.line_column();
        ParseError {
            message: message.into(),
            line,
            column,
            offset: self.pos,
        }
    }

    fn line_column(&self) -> (usize, usize) {
        let upto = &self.input[..self.pos.min(self.input.len())];
        // `clippy::naive_bytecount` wants the `bytecount` crate; a parse
        // error is rare enough that a dependency for it is not worth it.
        #[allow(clippy::naive_bytecount)]
        let line = 1 + upto.iter().filter(|&&b| b == b'\n').count();
        let column = 1 + upto.iter().rev().take_while(|&&b| b != b'\n').count();
        (line, column)
    }

    /// Whitespace and comments. The format allows `//` to end of line and
    /// `/* … */`; Xcode's own writer emits neither, but a hand-edited file or
    /// another tool's output may carry them.
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(b) if b.is_ascii_whitespace() => {
                    self.pos += 1;
                }
                Some(b'/') => match self.input.get(self.pos + 1) {
                    Some(b'/') => {
                        self.pos += 2;
                        while let Some(b) = self.peek() {
                            self.pos += 1;
                            if b == b'\n' {
                                break;
                            }
                        }
                    }
                    Some(b'*') => {
                        self.pos += 2;
                        // An unterminated comment runs to the end, where the
                        // caller's own "expected a value" error takes over.
                        while self.pos < self.input.len() {
                            if self.input[self.pos] == b'*'
                                && self.input.get(self.pos + 1) == Some(&b'/')
                            {
                                self.pos += 2;
                                break;
                            }
                            self.pos += 1;
                        }
                    }
                    _ => return,
                },
                _ => return,
            }
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error("nesting depth limit exceeded"));
        }
        match self.peek() {
            Some(b'{') => self.parse_object(depth),
            Some(b'[') => self.parse_array(depth),
            Some(b'"') => Ok(Value::String(self.parse_string()?)),
            Some(b't') => self.parse_literal("true", Value::Bool(true)),
            Some(b'f') => self.parse_literal("false", Value::Bool(false)),
            Some(b'n') => self.parse_literal("null", Value::Null),
            Some(b) if b == b'-' || b.is_ascii_digit() => self.parse_number(),
            Some(b) => Err(self.error(format!("unexpected character '{}'", b as char))),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn parse_literal(&mut self, word: &str, value: Value) -> Result<Value, ParseError> {
        if self.input[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error(format!("expected `{word}`")))
        }
    }

    fn parse_number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        {
            self.pos += 1;
        }
        let lexeme = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| self.error("number is not valid UTF-8"))?;
        if lexeme.parse::<f64>().is_err() {
            return Err(self.error(format!("malformed number `{lexeme}`")));
        }
        Ok(Value::Number(lexeme.to_string()))
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        // Caller has already peeked the opening quote.
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(b) = self.bump() else {
                return Err(self.error("unterminated string"));
            };
            match b {
                b'"' => return Ok(out),
                b'\\' => self.parse_escape(&mut out)?,
                _ => {
                    // Copy the whole UTF-8 sequence; the input came from a
                    // `&str`, so a lead byte is always followed by its
                    // continuations.
                    let start = self.pos - 1;
                    while self.peek().is_some_and(|b| (b & 0xC0) == 0x80) {
                        self.pos += 1;
                    }
                    out.push_str(
                        std::str::from_utf8(&self.input[start..self.pos])
                            .map_err(|_| self.error("string is not valid UTF-8"))?,
                    );
                }
            }
        }
    }

    fn parse_escape(&mut self, out: &mut String) -> Result<(), ParseError> {
        let Some(b) = self.bump() else {
            return Err(self.error("unterminated escape"));
        };
        let c = match b {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => return self.parse_unicode_escape(out),
            other => {
                return Err(self.error(format!("unknown escape `\\{}`", other as char)));
            }
        };
        out.push(c);
        Ok(())
    }

    /// `\uXXXX`, joining a surrogate pair into the character it encodes. A lone
    /// surrogate has no Unicode scalar to become, so it is rejected rather than
    /// silently replaced.
    fn parse_unicode_escape(&mut self, out: &mut String) -> Result<(), ParseError> {
        let first = self.parse_hex4()?;
        let scalar = if (0xD800..0xDC00).contains(&first) {
            if self.peek() != Some(b'\\') || self.input.get(self.pos + 1) != Some(&b'u') {
                return Err(self.error("high surrogate is not followed by a low surrogate"));
            }
            self.pos += 2;
            let second = self.parse_hex4()?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(self.error("high surrogate is not followed by a low surrogate"));
            }
            0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00)
        } else {
            first
        };
        let c = char::from_u32(scalar)
            .ok_or_else(|| self.error(format!("`\\u{scalar:04X}` is not a character")))?;
        out.push(c);
        Ok(())
    }

    fn parse_hex4(&mut self) -> Result<u32, ParseError> {
        let end = self.pos + 4;
        if end > self.input.len() {
            return Err(self.error("truncated `\\u` escape"));
        }
        let digits = std::str::from_utf8(&self.input[self.pos..end])
            .ok()
            .and_then(|s| u32::from_str_radix(s, 16).ok())
            .ok_or_else(|| self.error("`\\u` escape needs four hex digits"))?;
        self.pos = end;
        Ok(digits)
    }

    fn parse_array(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.pos += 1; // `[`
        let mut items = Vec::new();
        loop {
            self.skip_trivia();
            // A trailing comma before the bracket is what Xcode writes, so the
            // closing bracket is checked before every element, not just first.
            match self.peek() {
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                None => return Err(self.error("unterminated array")),
                _ => {}
            }
            items.push(self.parse_value(depth + 1)?);
            self.skip_trivia();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.error("expected `,` or `]` in array")),
            }
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.pos += 1; // `{`
        let mut object = Object::new();
        loop {
            self.skip_trivia();
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(object));
                }
                Some(b'"') => {}
                None => return Err(self.error("unterminated object")),
                _ => return Err(self.error("expected a quoted key in object")),
            }
            let key = self.parse_string()?;
            self.skip_trivia();
            if self.peek() != Some(b':') {
                return Err(self.error("expected `:` after object key"));
            }
            self.pos += 1;
            self.skip_trivia();
            let value = self.parse_value(depth + 1)?;
            object.insert(key, value);
            self.skip_trivia();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(object));
                }
                _ => return Err(self.error("expected `,` or `}` in object")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document as `xcprojformatter` prints it: trailing commas throughout,
    /// build-setting conditions carried in the key, a configuration that is a
    /// bare string until it needs an xcconfig, and a build phase that is a bare
    /// string until it needs properties.
    const CANONICAL: &str = r#"{
  "organization": "Sweetpad",
  "default-configuration": "Release",
  "configurations": [
    "Debug",
    { "name": "Release", "file": { "anchor": "group", "relative-path": "Config/Release.xcconfig" } },
  ],
  "localizations": {
    "development": "en",
    "supported": [
      "en",
      "fr",
    ],
  },
  "files": [
    {
      "kind": "group",
      "name": "Sources",
      "children": [
        { "path": "App.swift", "target-membership": [ "MyApp/compile-sources" ] },
      ],
    },
    { "kind": "folder", "path": "Shared", "target-membership": [ "MyApp" ] },
  ],
  "targets": [
    {
      "name": "MyApp",
      "id": "0123456789ABCDEF01234567",
      "product-type": "com.apple.product-type.application",
      "build-phases": [
        "compile-sources",
      ],
      "build-settings": {
        "CODE_SIGN_IDENTITY[sdk=iphoneos*]": "Apple Development",
        "PRODUCT_NAME": "MyApp",
      },
    },
  ],
  "build-settings": {
    "OTHER_LDFLAGS": [
      "-ObjC",
      "-lz",
    ],
    "SDKROOT": "iphoneos",
    "SWIFT_OPTIMIZATION_LEVEL[config=Debug]": "-Onone",
    "SWIFT_VERSION": "6.0",
  },
}
"#;

    #[test]
    fn reads_a_canonical_document() {
        let doc = parse(CANONICAL).expect("parse");

        assert_eq!(
            doc.get("organization").and_then(Value::as_str),
            Some("Sweetpad")
        );
        assert_eq!(
            doc.get("default-configuration").and_then(Value::as_str),
            Some("Release")
        );

        // A configuration is a bare name until it carries an xcconfig.
        let configs = doc.get("configurations").and_then(Value::as_array).unwrap();
        assert_eq!(configs[0].as_str(), Some("Debug"));
        assert_eq!(
            configs[1].get("name").and_then(Value::as_str),
            Some("Release")
        );
        assert_eq!(
            configs[1]
                .get("file")
                .and_then(|f| f.get("relative-path"))
                .and_then(Value::as_str),
            Some("Config/Release.xcconfig")
        );

        // Per-configuration and per-SDK settings live in the key, the way
        // `pbxproj` and `xcconfig` spell them.
        let settings = doc.get("build-settings").unwrap();
        assert_eq!(
            settings
                .get("SWIFT_OPTIMIZATION_LEVEL[config=Debug]")
                .and_then(Value::as_str),
            Some("-Onone")
        );
        let flags = settings
            .get("OTHER_LDFLAGS")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(flags[0].as_str(), Some("-ObjC"));

        let target = &doc.get("targets").and_then(Value::as_array).unwrap()[0];
        assert_eq!(target.get("name").and_then(Value::as_str), Some("MyApp"));
        assert_eq!(
            target
                .get("build-settings")
                .and_then(|s| s.get("CODE_SIGN_IDENTITY[sdk=iphoneos*]"))
                .and_then(Value::as_str),
            Some("Apple Development")
        );
        // A build phase with no properties of its own is its kind alone.
        assert_eq!(
            target
                .get("build-phases")
                .and_then(Value::as_array)
                .unwrap()[0]
                .as_str(),
            Some("compile-sources")
        );

        // A file declares the targets it belongs to, rather than a target
        // listing its files.
        let group = &doc.get("files").and_then(Value::as_array).unwrap()[0];
        let child = &group.get("children").and_then(Value::as_array).unwrap()[0];
        assert_eq!(
            child
                .get("target-membership")
                .and_then(Value::as_array)
                .unwrap()[0]
                .as_str(),
            Some("MyApp/compile-sources")
        );
    }

    #[test]
    fn object_key_order_is_the_source_order() {
        let doc = parse(r#"{ "z": 1, "a": 2, "m": 3 }"#).unwrap();
        let keys: Vec<&str> = doc.as_object().unwrap().iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["z", "a", "m"]);
    }

    #[test]
    fn a_repeated_key_keeps_its_place_and_takes_the_last_value() {
        let doc = parse(r#"{ "a": 1, "b": 2, "a": 3 }"#).unwrap();
        let object = doc.as_object().unwrap();
        assert_eq!(object.get("a").and_then(Value::as_i64), Some(3));
        let keys: Vec<&str> = object.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["a", "b"]);
    }

    #[test]
    fn comments_are_trivia() {
        let doc = parse(
            r#"// leading
            {
              /* block */ "a": 1, // trailing
              "b": [ 2, /* inner */ 3 ]
            }"#,
        )
        .unwrap();
        assert_eq!(doc.get("a").and_then(Value::as_i64), Some(1));
        assert_eq!(doc.get("b").and_then(Value::as_array).unwrap().len(), 2);
    }

    #[test]
    fn empty_and_trailing_comma_containers_parse() {
        assert_eq!(parse("[]").unwrap(), Value::Array(vec![]));
        assert_eq!(parse("{}").unwrap(), Value::Object(Object::new()));
        assert_eq!(parse("[1,]").unwrap().as_array().unwrap().len(), 1);
        assert_eq!(parse(r#"{"a":1,}"#).unwrap().as_object().unwrap().len(), 1);
        // A comma with nothing before it is still a syntax error.
        assert!(parse("[,1]").is_err());
        assert!(parse(r#"{,"a":1}"#).is_err());
    }

    #[test]
    fn numbers_keep_their_lexeme() {
        let doc = parse(r#"{ "a": 110, "b": -1.5e3, "c": 0 }"#).unwrap();
        assert_eq!(doc.get("a"), Some(&Value::Number("110".into())));
        assert_eq!(doc.get("a").and_then(Value::as_i64), Some(110));
        assert_eq!(doc.get("b").and_then(Value::as_f64), Some(-1500.0));
        assert!(parse(r#"{ "a": 1.2.3 }"#).is_err());
    }

    #[test]
    fn literals_parse() {
        let doc = parse(r#"{ "t": true, "f": false, "n": null }"#).unwrap();
        assert_eq!(doc.get("t").and_then(Value::as_bool), Some(true));
        assert_eq!(doc.get("f").and_then(Value::as_bool), Some(false));
        assert_eq!(doc.get("n"), Some(&Value::Null));
        assert!(parse(r#"{ "t": tru }"#).is_err());
    }

    #[test]
    fn string_escapes_decode() {
        let doc = parse(r#"{ "s": "a\"b\\c\/d\n\té😀" }"#).unwrap();
        assert_eq!(
            doc.get("s").and_then(Value::as_str),
            Some("a\"b\\c/d\n\té😀")
        );
    }

    #[test]
    fn malformed_escapes_are_rejected() {
        assert!(parse(r#"{ "s": "\q" }"#).is_err());
        assert!(parse(r#"{ "s": "\u00" }"#).is_err());
        // A high surrogate with no low surrogate encodes no character.
        assert!(parse(r#"{ "s": "\uD83D" }"#).is_err());
        assert!(parse(r#"{ "s": "\uD83Dx" }"#).is_err());
        assert!(parse(r#"{ "s": "unterminated }"#).is_err());
    }

    #[test]
    fn non_ascii_passes_through_unescaped() {
        let doc = parse("{ \"s\": \"héllo 😀\" }").unwrap();
        assert_eq!(doc.get("s").and_then(Value::as_str), Some("héllo 😀"));
    }

    #[test]
    fn errors_carry_a_position() {
        let err = parse("{\n  \"a\": }\n}").unwrap_err();
        assert_eq!(err.line, 2);
        assert!(err.to_string().contains("line 2"), "{err}");
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let deep = "[".repeat(MAX_DEPTH + 2) + &"]".repeat(MAX_DEPTH + 2);
        let err = parse(&deep).unwrap_err();
        assert!(err.message.contains("depth"), "{err}");
    }

    #[test]
    fn a_document_must_have_one_root() {
        assert!(parse("").is_err());
        assert!(parse("{} {}").is_err());
        assert!(parse("// only a comment").is_err());
    }
}
