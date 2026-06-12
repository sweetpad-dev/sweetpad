use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use crate::file_cache::ParseCache;

/// An insertion-order-preserving string-keyed map.
///
/// Xcode does not keep `project.pbxproj` dictionaries sorted — objects keep
/// the historical order in which they were added — so a faithful
/// re-serialization (see [`crate::pbxproj_writer`]) must replay the order in
/// which keys appeared in the source. Lookups stay O(1) via a side index.
#[derive(Debug, Clone, Default)]
pub struct Dict {
    entries: Vec<(String, Value)>,
    index: HashMap<String, usize>,
    /// Layout hint recorded by the parser: `true` when the dict's source
    /// representation sat entirely on one line (`{isa = PBXBuildFile; … };`).
    /// Pure formatting metadata — ignored by equality — that lets the writer
    /// replay the original single-line/multi-line choice for object entries.
    single_line: bool,
}

impl Dict {
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
    /// keeping the key's original position (last value wins, like a plist).
    pub fn insert(&mut self, key: String, value: Value) {
        if let Some(&i) = self.index.get(&key) {
            self.entries[i].1 = value;
        } else {
            self.index.insert(key.clone(), self.entries.len());
            self.entries.push((key, value));
        }
    }

    /// Key/value pairs in insertion (source) order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.entries.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.entries.iter().map(|(_, v)| v)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The parser's layout hint: `true` when this dict was written on a
    /// single line in the source. `false` for programmatically built dicts.
    #[must_use]
    pub fn is_single_line(&self) -> bool {
        self.single_line
    }

    pub fn set_single_line(&mut self, single_line: bool) {
        self.single_line = single_line;
    }
}

/// Equality is order-sensitive over the entries but ignores the
/// `single_line` layout hint (formatting, not data).
impl PartialEq for Dict {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl Eq for Dict {}

impl FromIterator<(String, Value)> for Dict {
    fn from_iter<T: IntoIterator<Item = (String, Value)>>(iter: T) -> Self {
        let mut d = Dict::new();
        for (k, v) in iter {
            d.insert(k, v);
        }
        d
    }
}

impl<'a> IntoIterator for &'a Dict {
    type Item = (&'a String, &'a Value);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (String, Value)>,
        fn(&'a (String, Value)) -> (&'a String, &'a Value),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter().map(|(k, v)| (k, v))
    }
}

impl std::ops::Index<&str> for Dict {
    type Output = Value;

    fn index(&self, key: &str) -> &Value {
        self.get(key).expect("no entry found for key")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    String(String),
    Array(Vec<Value>),
    Dict(Dict),
}

impl Value {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(s) = self {
            Some(s)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Value]> {
        if let Value::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_dict(&self) -> Option<&Dict> {
        if let Value::Dict(d) = self {
            Some(d)
        } else {
            None
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_dict().and_then(|d| d.get(key))
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

pub fn parse(input: &str) -> Result<Value, ParseError> {
    let mut p = Parser::new(input);
    p.skip_ws();
    let v = p.parse_value(0)?;
    p.skip_ws();
    if p.peek().is_some() {
        return Err(p.error("trailing data after root value"));
    }
    Ok(v)
}

pub fn parse_file(path: &Path) -> Result<Value, Error> {
    let s = fs::read_to_string(path)?;
    Ok(parse(&s)?)
}

/// Process-global cache of parsed pbxproj files, validated by `(len, mtime)`.
static CACHE: LazyLock<ParseCache<Value>> = LazyLock::new(ParseCache::new);

/// Like [`parse_file`] but served from an in-memory, mtime-validated cache,
/// returning a shared `Arc<Value>`. The long-lived node addon parses the same
/// `project.pbxproj` on every `build-settings` / `list` call, so this hands
/// back the cached AST until the file changes on disk. The raw [`parse_file`]
/// stays for one-shot reads (e.g. xcspec parsing, cached separately by
/// [`crate::catalog_cache`]).
pub fn parse_file_cached(path: &Path) -> Result<Arc<Value>, Error> {
    CACHE.get_or_parse(path, parse_file)
}

/// Recursion guard for nested dicts/arrays. Real pbxproj files nest a handful
/// of levels deep; 512 is well past anything Xcode writes and keeps a
/// pathological `(((((…` input from overflowing the stack.
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

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.input.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    fn skip_ws(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => {
                    self.pos += 1;
                }
                Some(b'/') => match self.peek_at(1) {
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
                        loop {
                            match self.peek() {
                                None => return,
                                Some(b'*') if self.peek_at(1) == Some(b'/') => {
                                    self.pos += 2;
                                    break;
                                }
                                _ => self.pos += 1,
                            }
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
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_dict(depth),
            Some(b'(') => self.parse_array(depth),
            Some(b'"') => Ok(Value::String(self.parse_quoted_string()?)),
            // OpenStep plist also allows `<HEX BYTES>` data literals. We don't
            // need the bytes for anything in pbxproj/xcspec consumers, but the
            // syntax has to parse so we can skip past it. Surface it as a
            // String containing the lowercase hex digits.
            Some(b'<') => Ok(Value::String(self.parse_data_literal()?)),
            Some(c) if Self::is_bare_start(c) => Ok(Value::String(self.parse_bare_string()?)),
            Some(c) => Err(self.error(format!("unexpected character {:?}", c as char))),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn parse_data_literal(&mut self) -> Result<String, ParseError> {
        self.pos += 1;
        let mut hex = String::new();
        while let Some(b) = self.peek() {
            match b {
                b'>' => {
                    self.pos += 1;
                    return Ok(hex);
                }
                b' ' | b'\t' | b'\n' | b'\r' => {
                    self.pos += 1;
                }
                b'0'..=b'9' | b'a'..=b'f' => {
                    hex.push(b as char);
                    self.pos += 1;
                }
                b'A'..=b'F' => {
                    hex.push(b.to_ascii_lowercase() as char);
                    self.pos += 1;
                }
                _ => {
                    return Err(self.error(format!("bad hex digit {:?}", b as char)));
                }
            }
        }
        Err(self.error("unterminated data literal"))
    }

    fn parse_dict(&mut self, depth: usize) -> Result<Value, ParseError> {
        let start = self.pos;
        self.pos += 1;
        let mut map = Dict::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    let single_line = !self.input[start..self.pos].contains(&b'\n');
                    map.set_single_line(single_line);
                    return Ok(Value::Dict(map));
                }
                None => return Err(self.error("unterminated dict")),
                _ => {}
            }
            let key = self.parse_string_value()?;
            self.skip_ws();
            if self.peek() != Some(b'=') {
                return Err(self.error("expected '=' after dict key"));
            }
            self.pos += 1;
            let value = self.parse_value(depth + 1)?;
            self.skip_ws();
            if self.peek() != Some(b';') {
                return Err(self.error("expected ';' after dict value"));
            }
            self.pos += 1;
            map.insert(key, value);
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b')') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                None => return Err(self.error("unterminated array")),
                _ => {}
            }
            let v = self.parse_value(depth + 1)?;
            items.push(v);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b')') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.error("expected ',' or ')' in array")),
            }
        }
    }

    fn parse_string_value(&mut self) -> Result<String, ParseError> {
        match self.peek() {
            Some(b'"') => self.parse_quoted_string(),
            Some(c) if Self::is_bare_start(c) => self.parse_bare_string(),
            Some(c) => Err(self.error(format!("unexpected character {:?}", c as char))),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn parse_quoted_string(&mut self) -> Result<String, ParseError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' {
                    break;
                }
                self.pos += 1;
            }
            let bytes = &self.input[start..self.pos];
            let s =
                std::str::from_utf8(bytes).map_err(|_| self.error("invalid UTF-8 in string"))?;
            out.push_str(s);
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.advance() {
                        None => return Err(self.error("unterminated escape")),
                        Some(b'\\') => out.push('\\'),
                        Some(b'"') => out.push('"'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'0') => out.push('\0'),
                        Some(b'a') => out.push('\x07'),
                        Some(b'b') => out.push('\x08'),
                        Some(b'f') => out.push('\x0c'),
                        Some(b'v') => out.push('\x0b'),
                        Some(b'U') => {
                            let mut code: u32 = 0;
                            for _ in 0..4 {
                                let d = self
                                    .advance()
                                    .ok_or_else(|| self.error("incomplete unicode escape"))?;
                                let v: u32 = match d {
                                    b'0'..=b'9' => u32::from(d - b'0'),
                                    b'a'..=b'f' => u32::from(d - b'a' + 10),
                                    b'A'..=b'F' => u32::from(d - b'A' + 10),
                                    _ => return Err(self.error("bad hex digit")),
                                };
                                code = code * 16 + v;
                            }
                            match char::from_u32(code) {
                                Some(c) => out.push(c),
                                None => return Err(self.error("bad unicode codepoint")),
                            }
                        }
                        Some(other) => out.push(other as char),
                    }
                }
                None => return Err(self.error("unterminated string")),
                _ => unreachable!(),
            }
        }
    }

    fn parse_bare_string(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b == b'/' {
                if matches!(self.peek_at(1), Some(b'/' | b'*')) {
                    break;
                }
                self.pos += 1;
            } else if Self::is_bare_cont(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err(self.error("expected string"));
        }
        let bytes = &self.input[start..self.pos];
        std::str::from_utf8(bytes)
            .map(String::from)
            .map_err(|_| self.error("invalid UTF-8 in bare string"))
    }

    fn is_bare_start(b: u8) -> bool {
        b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'_' | b'.' | b'$' | b'+' | b'-' | b':' | b'@' | b'*' | b'/'
            )
    }

    fn is_bare_cont(b: u8) -> bool {
        !matches!(
            b,
            b' ' | b'\t'
                | b'\n'
                | b'\r'
                | b'{'
                | b'}'
                | b'('
                | b')'
                | b','
                | b';'
                | b'='
                | b'"'
                | b'<'
                | b'>'
        )
    }

    fn error(&self, msg: impl Into<String>) -> ParseError {
        let (line, column) = self.line_column();
        ParseError {
            message: msg.into(),
            line,
            column,
            offset: self.pos,
        }
    }

    fn line_column(&self) -> (usize, usize) {
        let mut line = 1usize;
        let mut col = 1usize;
        for &b in &self.input[..self.pos.min(self.input.len())] {
            if b == b'\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_dict() {
        let v = parse("{}").unwrap();
        assert_eq!(v, Value::Dict(Dict::new()));
    }

    #[test]
    fn preserves_key_insertion_order() {
        let v = parse("{ z = 1; a = 2; m = 3; }").unwrap();
        let keys: Vec<&str> = v.as_dict().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["z", "a", "m"]);
    }

    #[test]
    fn records_single_line_hint() {
        let v = parse("{ a = { x = 1; }; b = {\n x = 1; }; }").unwrap();
        let d = v.as_dict().unwrap();
        assert!(
            d.get("a")
                .and_then(Value::as_dict)
                .unwrap()
                .is_single_line()
        );
        assert!(
            !d.get("b")
                .and_then(Value::as_dict)
                .unwrap()
                .is_single_line()
        );
        // The hint is formatting metadata: both dicts are still equal.
        assert_eq!(d.get("a"), d.get("b"));
    }

    #[test]
    fn parses_simple_dict() {
        let v = parse("{ a = b; }").unwrap();
        assert_eq!(v.get("a").and_then(Value::as_str), Some("b"));
    }

    #[test]
    fn parses_quoted_string() {
        let v = parse(r#"{ k = "hello world"; }"#).unwrap();
        assert_eq!(v.get("k").and_then(Value::as_str), Some("hello world"));
    }

    #[test]
    fn parses_escapes() {
        let v = parse(r#"{ k = "a\nb\t\"c\\d"; }"#).unwrap();
        assert_eq!(v.get("k").and_then(Value::as_str), Some("a\nb\t\"c\\d"));
    }

    #[test]
    fn parses_unicode_escape() {
        let v = parse(r#"{ k = "\U00E9"; }"#).unwrap();
        assert_eq!(v.get("k").and_then(Value::as_str), Some("é"));
    }

    #[test]
    fn parses_array() {
        let v = parse("{ k = (a, b, c); }").unwrap();
        let arr = v.get("k").and_then(Value::as_array).unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str(), Some("a"));
        assert_eq!(arr[2].as_str(), Some("c"));
    }

    #[test]
    fn parses_array_trailing_comma() {
        let v = parse("{ k = (a, b,); }").unwrap();
        let arr = v.get("k").and_then(Value::as_array).unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn parses_empty_array() {
        let v = parse("{ k = (); }").unwrap();
        let arr = v.get("k").and_then(Value::as_array).unwrap();
        assert!(arr.is_empty());
    }

    #[test]
    fn parses_nested() {
        let v = parse("{ a = { b = c; d = (e, f); }; }").unwrap();
        let inner = v.get("a").unwrap();
        assert_eq!(inner.get("b").and_then(Value::as_str), Some("c"));
        assert_eq!(
            inner.get("d").and_then(Value::as_array).map(<[_]>::len),
            Some(2)
        );
    }

    #[test]
    fn skips_line_comments() {
        let v = parse("// header\n{ a = b; }").unwrap();
        assert_eq!(v.get("a").and_then(Value::as_str), Some("b"));
    }

    #[test]
    fn skips_block_comments() {
        let v = parse("{ a /* the a key */ = b /* the b val */; }").unwrap();
        assert_eq!(v.get("a").and_then(Value::as_str), Some("b"));
    }

    #[test]
    fn parses_pbxproj_header() {
        let src = "// !$*UTF8*$!\n{\n  archiveVersion = 1;\n  classes = {};\n  objectVersion = 60;\n  objects = {};\n  rootObject = ABC;\n}\n";
        let v = parse(src).unwrap();
        assert_eq!(v.get("archiveVersion").and_then(Value::as_str), Some("1"));
        assert_eq!(v.get("rootObject").and_then(Value::as_str), Some("ABC"));
    }

    #[test]
    fn rejects_unterminated_dict() {
        assert!(parse("{ a = b;").is_err());
    }

    #[test]
    fn rejects_missing_semicolon() {
        assert!(parse("{ a = b }").is_err());
    }

    #[test]
    fn rejects_unterminated_string() {
        assert!(parse("{ a = \"unclosed; }").is_err());
    }

    #[test]
    fn handles_bare_string_with_special_chars() {
        let v = parse("{ k = com.apple.product-type.tool; }").unwrap();
        assert_eq!(
            v.get("k").and_then(Value::as_str),
            Some("com.apple.product-type.tool")
        );
    }
}
