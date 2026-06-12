use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Element {
    pub name: String,
    /// Attributes in source order. Xcode's scheme/workspace writer emits
    /// attributes in a meaning-bearing, non-alphabetical order (e.g.
    /// `BlueprintIdentifier` before `BuildableName`), so order must survive
    /// a parse → [`serialize`] round trip.
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
}

impl Element {
    #[must_use]
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[must_use]
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == name)
    }

    #[must_use]
    pub fn descendants_named<'a>(&'a self, name: &'a str) -> Vec<&'a Element> {
        let mut out = Vec::new();
        self.collect_descendants(name, &mut out);
        out
    }

    fn collect_descendants<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            }
            c.collect_descendants(name, out);
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
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

/// Recursion guard for nested elements. Xcode's scheme/workspace files nest a
/// handful of levels deep; 256 is well past anything real, keeps a
/// pathological `<a><a><a>…` input from overflowing the stack, and leaves
/// headroom for [`Parser::parse_element`]'s sizeable debug-build frames on a
/// 2 MiB test-thread stack. The serializer shares the bound: a parsed tree
/// can never exceed it, so [`serialize`] only truncates on programmatically
/// built trees deeper than this.
const MAX_DEPTH: usize = 256;

pub fn parse(input: &str) -> Result<Element, ParseError> {
    let mut p = Parser::new(input);
    p.skip_prolog()?;
    let root = p.parse_element(0)?;
    p.skip_trailing()?;
    Ok(root)
}

pub fn parse_file(path: &Path) -> Result<Element, Error> {
    let s = fs::read_to_string(path)?;
    Ok(parse(&s)?)
}

/// Serialize an element tree back to Xcode's `.xcscheme` /
/// `contents.xcworkspacedata` on-disk format: the UTF-8 XML declaration,
/// three-space indentation, one attribute per line (`key = "value"`), and an
/// explicit closing tag for every element (Xcode never self-closes).
/// Attribute order is whatever [`Element::attributes`] holds — source order
/// after a parse — so a parse → serialize round trip is byte-exact.
///
/// Trees deeper than [`MAX_DEPTH`] cannot come out of [`parse`]; if one is
/// built programmatically anyway, children past the bound are dropped rather
/// than overflowing the stack (the signature predates the guard and cannot
/// surface an error).
#[must_use]
pub fn serialize(root: &Element) -> String {
    let mut out = String::with_capacity(1 << 12);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    write_element(&mut out, root, 0);
    out
}

fn write_element(out: &mut String, e: &Element, depth: usize) {
    let indent = "   ".repeat(depth);
    out.push_str(&indent);
    out.push('<');
    out.push_str(&e.name);
    for (key, value) in &e.attributes {
        out.push('\n');
        out.push_str(&indent);
        out.push_str("   ");
        out.push_str(key);
        out.push_str(" = \"");
        push_escaped(out, value);
        out.push('"');
    }
    out.push_str(">\n");
    if !e.text.is_empty() {
        out.push_str(&indent);
        out.push_str("   ");
        push_escaped(out, &e.text);
        out.push('\n');
    }
    if depth < MAX_DEPTH {
        for child in &e.children {
            write_element(out, child, depth + 1);
        }
    }
    out.push_str(&indent);
    out.push_str("</");
    out.push_str(&e.name);
    out.push_str(">\n");
}

/// Escape the way Xcode's writer does: the four markup-significant
/// characters as named entities, line breaks and tabs as numeric ones.
fn push_escaped(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            _ => out.push(c),
        }
    }
}

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

    fn starts_with(&self, s: &[u8]) -> bool {
        self.input[self.pos..].starts_with(s)
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn skip_prolog(&mut self) -> Result<(), ParseError> {
        loop {
            self.skip_ws();
            if self.starts_with(b"<?") {
                self.pos += 2;
                while !self.starts_with(b"?>") {
                    if self.pos >= self.input.len() {
                        return Err(self.error("unterminated processing instruction"));
                    }
                    self.pos += 1;
                }
                self.pos += 2;
            } else if self.starts_with(b"<!--") {
                self.skip_comment()?;
            } else if self.starts_with(b"<!") {
                // <!DOCTYPE …> — may carry an internal subset in brackets
                // (`<!DOCTYPE x [ <!ENTITY …> ]>`) whose own '>' characters
                // must not terminate the declaration early.
                self.pos += 2;
                let mut bracket_depth = 0usize;
                loop {
                    match self.peek() {
                        Some(b'[') => bracket_depth += 1,
                        Some(b']') => bracket_depth = bracket_depth.saturating_sub(1),
                        Some(b'>') if bracket_depth == 0 => break,
                        Some(_) => {}
                        None => return Err(self.error("unterminated <!...>")),
                    }
                    self.pos += 1;
                }
                self.pos += 1;
            } else {
                return Ok(());
            }
        }
    }

    fn skip_trailing(&mut self) -> Result<(), ParseError> {
        loop {
            self.skip_ws();
            if self.peek().is_none() {
                return Ok(());
            }
            if self.starts_with(b"<!--") {
                self.skip_comment()?;
            } else if self.starts_with(b"<?") {
                self.pos += 2;
                while !self.starts_with(b"?>") {
                    if self.pos >= self.input.len() {
                        return Err(self.error("unterminated processing instruction"));
                    }
                    self.pos += 1;
                }
                self.pos += 2;
            } else {
                return Err(self.error("trailing data after root element"));
            }
        }
    }

    fn skip_comment(&mut self) -> Result<(), ParseError> {
        self.pos += 4;
        while !self.starts_with(b"-->") {
            if self.pos >= self.input.len() {
                return Err(self.error("unterminated comment"));
            }
            self.pos += 1;
        }
        self.pos += 3;
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn parse_element(&mut self, depth: usize) -> Result<Element, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error("nesting depth limit exceeded"));
        }
        self.skip_ws();
        if self.peek() != Some(b'<') {
            return Err(self.error("expected '<'"));
        }
        self.pos += 1;
        let name = self.parse_name()?;
        let mut attributes: Vec<(String, String)> = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'/') => {
                    self.pos += 1;
                    if self.peek() != Some(b'>') {
                        return Err(self.error("expected '>' after '/' in self-closing tag"));
                    }
                    self.pos += 1;
                    return Ok(Element {
                        name,
                        attributes,
                        children: Vec::new(),
                        text: String::new(),
                    });
                }
                Some(b'>') => {
                    self.pos += 1;
                    break;
                }
                None => return Err(self.error("unterminated start tag")),
                _ => {
                    let key = self.parse_name()?;
                    self.skip_ws();
                    if self.peek() != Some(b'=') {
                        return Err(self.error("expected '=' in attribute"));
                    }
                    self.pos += 1;
                    self.skip_ws();
                    let value = self.parse_attr_value()?;
                    // Duplicate attribute: last value wins, first position kept.
                    if let Some(slot) = attributes.iter_mut().find(|(k, _)| *k == key) {
                        slot.1 = value;
                    } else {
                        attributes.push((key, value));
                    }
                }
            }
        }

        let mut children: Vec<Element> = Vec::new();
        let mut text = String::new();
        loop {
            let text_start = self.pos;
            while let Some(b) = self.peek() {
                if b == b'<' {
                    break;
                }
                self.pos += 1;
            }
            let slice = &self.input[text_start..self.pos];
            let s = std::str::from_utf8(slice)
                .map_err(|_| self.error("invalid UTF-8 in element text"))?;
            text.push_str(&decode_entities(s));

            if self.peek().is_none() {
                return Err(self.error(format!("unterminated element <{name}>")));
            }

            if self.starts_with(b"</") {
                self.pos += 2;
                let end_name = self.parse_name()?;
                if end_name != name {
                    return Err(self.error(format!(
                        "mismatched closing tag </{end_name}>, expected </{name}>"
                    )));
                }
                self.skip_ws();
                if self.peek() != Some(b'>') {
                    return Err(self.error("expected '>' in closing tag"));
                }
                self.pos += 1;
                return Ok(Element {
                    name,
                    attributes,
                    children,
                    text: text.trim().to_string(),
                });
            }
            if self.starts_with(b"<!--") {
                self.skip_comment()?;
                continue;
            }
            if self.starts_with(b"<![CDATA[") {
                self.pos += 9;
                let cdata_start = self.pos;
                while !self.starts_with(b"]]>") {
                    if self.pos >= self.input.len() {
                        return Err(self.error("unterminated CDATA"));
                    }
                    self.pos += 1;
                }
                let cdata_slice = &self.input[cdata_start..self.pos];
                text.push_str(
                    std::str::from_utf8(cdata_slice)
                        .map_err(|_| self.error("invalid UTF-8 in CDATA"))?,
                );
                self.pos += 3;
                continue;
            }
            let child = self.parse_element(depth + 1)?;
            children.push(child);
        }
    }

    fn parse_name(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b':') {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err(self.error("expected name"));
        }
        std::str::from_utf8(&self.input[start..self.pos])
            .map(String::from)
            .map_err(|_| self.error("invalid UTF-8 in name"))
    }

    fn parse_attr_value(&mut self) -> Result<String, ParseError> {
        let quote = self
            .peek()
            .ok_or_else(|| self.error("expected attribute value"))?;
        if quote != b'"' && quote != b'\'' {
            return Err(self.error("attribute value must be quoted"));
        }
        self.pos += 1;
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b == quote {
                break;
            }
            self.pos += 1;
        }
        if self.peek() != Some(quote) {
            return Err(self.error("unterminated attribute value"));
        }
        let slice = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| self.error("invalid UTF-8 in attribute value"))?;
        let value = decode_entities(slice);
        self.pos += 1;
        Ok(value)
    }

    fn error(&self, msg: impl Into<String>) -> ParseError {
        let (line, column) = self.line_column();
        ParseError {
            message: msg.into(),
            line,
            column,
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

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let Some(semi) = s[i..].find(';') else {
            out.push(c);
            continue;
        };
        let entity = &s[i + 1..i + semi];
        let decoded: Option<char> = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                if let Some(hex) = entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                {
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                } else if let Some(dec) = entity.strip_prefix('#') {
                    dec.parse::<u32>().ok().and_then(char::from_u32)
                } else {
                    None
                }
            }
        };
        if let Some(d) = decoded {
            out.push(d);
            // Advance the iterator past the consumed entity bytes.
            while let Some(&(j, _)) = chars.peek() {
                if j <= i + semi {
                    chars.next();
                } else {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_element() {
        let e = parse("<Scheme/>").unwrap();
        assert_eq!(e.name, "Scheme");
        assert!(e.attributes.is_empty());
        assert!(e.children.is_empty());
    }

    #[test]
    fn parses_attributes() {
        let e = parse(r#"<Scheme version="1.0" lang="en"/>"#).unwrap();
        assert_eq!(e.attr("version"), Some("1.0"));
        assert_eq!(e.attr("lang"), Some("en"));
    }

    #[test]
    fn parses_attributes_with_spaces_around_equals() {
        let e = parse(r#"<Scheme  version = "1.3"  lang = "en" />"#).unwrap();
        assert_eq!(e.attr("version"), Some("1.3"));
        assert_eq!(e.attr("lang"), Some("en"));
    }

    #[test]
    fn parses_single_quoted_attributes() {
        let e = parse(r"<Scheme version='1.0'/>").unwrap();
        assert_eq!(e.attr("version"), Some("1.0"));
    }

    #[test]
    fn parses_xml_declaration() {
        let e = parse(r#"<?xml version="1.0" encoding="UTF-8"?><Root/>"#).unwrap();
        assert_eq!(e.name, "Root");
    }

    #[test]
    fn skips_doctype_with_internal_subset() {
        // A DOCTYPE's internal subset can contain markup declarations whose
        // own '>' must not terminate the DOCTYPE early.
        let e = parse("<!DOCTYPE plist [ <!ENTITY foo \"bar\"> ]><Root/>").unwrap();
        assert_eq!(e.name, "Root");
    }

    #[test]
    fn parses_nested_elements() {
        let e = parse("<a><b/><c><d/></c></a>").unwrap();
        assert_eq!(e.children.len(), 2);
        assert_eq!(e.children[0].name, "b");
        assert_eq!(e.children[1].name, "c");
        assert_eq!(e.children[1].children[0].name, "d");
    }

    #[test]
    fn skips_comments() {
        let e = parse("<!-- top --><a><!-- inside --><b/></a><!-- trailing -->").unwrap();
        assert_eq!(e.name, "a");
        assert_eq!(e.children.len(), 1);
        assert_eq!(e.children[0].name, "b");
    }

    #[test]
    fn decodes_entities_in_attributes() {
        let e = parse(r#"<a x="A &amp; B &lt;C&gt;"/>"#).unwrap();
        assert_eq!(e.attr("x"), Some("A & B <C>"));
    }

    #[test]
    fn decodes_numeric_entities() {
        let e = parse(r#"<a x="&#65;&#x42;"/>"#).unwrap();
        assert_eq!(e.attr("x"), Some("AB"));
    }

    #[test]
    fn rejects_mismatched_closing_tag() {
        assert!(parse("<a></b>").is_err());
    }

    #[test]
    fn rejects_unterminated_element() {
        assert!(parse("<a>").is_err());
    }

    #[test]
    fn child_lookup_works() {
        let e =
            parse("<Scheme><BuildAction/><LaunchAction buildConfiguration=\"Debug\"/></Scheme>")
                .unwrap();
        assert!(e.child("BuildAction").is_some());
        assert_eq!(
            e.child("LaunchAction")
                .and_then(|c| c.attr("buildConfiguration")),
            Some("Debug")
        );
    }

    #[test]
    fn descendants_named_finds_nested() {
        let e = parse("<a><b><c/></b><c><c/></c></a>").unwrap();
        let descendants = e.descendants_named("c");
        assert_eq!(descendants.len(), 3);
    }

    #[test]
    fn preserves_attribute_order() {
        let e =
            parse(r#"<Ref BlueprintIdentifier="X" BuildableName="N" BlueprintName="B"/>"#).unwrap();
        let keys: Vec<&str> = e.attributes.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            vec!["BlueprintIdentifier", "BuildableName", "BlueprintName"]
        );
    }

    #[test]
    fn serializes_in_xcode_layout() {
        let e = parse(
            r#"<Scheme LastUpgradeVersion="1640" version="1.7"><BuildAction parallelizeBuildables="YES"><BuildActionEntries></BuildActionEntries></BuildAction></Scheme>"#,
        )
        .unwrap();
        assert_eq!(
            serialize(&e),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <Scheme\n   LastUpgradeVersion = \"1640\"\n   version = \"1.7\">\n\
             \x20\x20\x20<BuildAction\n      parallelizeBuildables = \"YES\">\n\
             \x20\x20\x20\x20\x20\x20<BuildActionEntries>\n\
             \x20\x20\x20\x20\x20\x20</BuildActionEntries>\n\
             \x20\x20\x20</BuildAction>\n\
             </Scheme>\n"
        );
    }

    #[test]
    fn serializes_escaped_attribute_values() {
        let e = parse(r#"<a scriptText="&quot;${X}&quot; 2&gt;&amp;1&#10;"/>"#).unwrap();
        assert_eq!(
            serialize(&e),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <a\n   scriptText = \"&quot;${X}&quot; 2&gt;&amp;1&#10;\">\n</a>\n"
        );
    }
}
