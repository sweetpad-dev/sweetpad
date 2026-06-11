use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use crate::file_cache::ParseCache;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Xcconfig {
    pub entries: Vec<Entry>,
    /// Source layout captured at parse time so [`serialize`] can reproduce
    /// the file byte-for-byte: every physical line is either trivia
    /// (comments, blank lines) replayed verbatim, or an entry's raw text
    /// (replayed only while the entry still matches it — see [`serialize`]).
    /// Empty for programmatically built configs.
    layout: Vec<Piece>,
    /// Set when the parsed source did *not* end with a newline, so
    /// [`serialize`] reproduces that. Inverted so the `Default` (false) gives
    /// programmatically built configs a conventional trailing newline.
    no_trailing_newline: bool,
}

/// One physical chunk of the source file, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    /// Line(s) that produced no entry: comments, blank lines. Verbatim,
    /// without the trailing newline.
    Raw(String),
    /// The physical line(s) — including continuations and inline comments —
    /// that produced `entries[index]`.
    Entry { index: usize, raw: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Assignment(Assignment),
    Include(Include),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub key: String,
    pub conditions: Vec<Condition>,
    pub value: String,
    /// Raw condition expression text from an xcspec Property/Option's
    /// `Condition` attribute. Evaluated against the resolved settings dict
    /// in a second pass; `None` means the assignment is unconditional.
    /// See [`crate::condition`] for the expression grammar.
    pub condition: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Include {
    pub path: String,
    pub optional: bool,
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at line {}", self.message, self.line)
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

pub fn parse(input: &str) -> Result<Xcconfig, ParseError> {
    // Comments are stripped from the whole content *before* any line
    // handling: a backslash inside a comment (`FOO = bar // see C:\`) is
    // comment text, not a line continuation, so the order matters.
    // Stripping preserves line count, so entry line numbers index into the
    // *original* input when capturing each entry's raw text for [`serialize`].
    let stripped = strip_comments(input);
    let mut entries = Vec::new();
    // 1-based (first, last) physical-line span of each entry, in order.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let iter = LineIter::new(&stripped);
    for (first_line, last_line, raw_line) in iter {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("#include") {
            let (optional, rest) = if let Some(r) = rest.strip_prefix('?') {
                (true, r)
            } else {
                (false, rest)
            };
            let rest = rest.trim_start();
            let path = parse_include_path(rest, first_line)?;
            entries.push(Entry::Include(Include { path, optional }));
            spans.push((first_line, last_line));
            continue;
        }
        let assignment = parse_assignment(trimmed, first_line)?;
        entries.push(Entry::Assignment(assignment));
        spans.push((first_line, last_line));
    }
    let layout = build_layout(input, &spans);
    Ok(Xcconfig {
        entries,
        layout,
        no_trailing_newline: !input.is_empty() && !input.ends_with('\n'),
    })
}

/// Interleave entry spans with the uncovered (trivia) lines of the source.
fn build_layout(input: &str, spans: &[(usize, usize)]) -> Vec<Piece> {
    let mut lines: Vec<&str> = input.split('\n').collect();
    // A trailing newline yields a final empty fragment that is not a line.
    if input.ends_with('\n') {
        lines.pop();
    }
    let mut layout = Vec::new();
    let mut line_no = 1usize;
    let mut entry_idx = 0usize;
    while line_no <= lines.len() {
        if let Some(&(first, last)) = spans.get(entry_idx)
            && first == line_no
        {
            let last = last.min(lines.len());
            layout.push(Piece::Entry {
                index: entry_idx,
                raw: lines[first - 1..last].join("\n"),
            });
            line_no = last + 1;
            entry_idx += 1;
        } else {
            layout.push(Piece::Raw(lines[line_no - 1].to_string()));
            line_no += 1;
        }
    }
    layout
}

/// Serialize back to xcconfig text.
///
/// Entries that still match what was parsed are replayed from their captured
/// raw text — preserving comments, spacing, and line continuations — so an
/// unmodified parse → serialize round trip is byte-exact. An entry that was
/// mutated since parsing (or appended without layout) is rendered canonically
/// (`KEY[k=v] = value` / `#include "path"`). Trivia lines (comments, blanks)
/// always replay verbatim.
#[must_use]
pub fn serialize(config: &Xcconfig) -> String {
    let mut out = String::new();
    let mut emitted = vec![false; config.entries.len()];
    for piece in &config.layout {
        match piece {
            Piece::Raw(s) => {
                out.push_str(s);
                out.push('\n');
            }
            Piece::Entry { index, raw } => {
                // An entry removed from `entries` drops its line entirely.
                let Some(entry) = config.entries.get(*index) else {
                    continue;
                };
                emitted[*index] = true;
                if reparse_single(raw).as_ref() == Some(entry) {
                    out.push_str(raw);
                } else {
                    out.push_str(&render_entry(entry));
                }
                out.push('\n');
            }
        }
    }
    for (i, entry) in config.entries.iter().enumerate() {
        if !emitted[i] {
            out.push_str(&render_entry(entry));
            out.push('\n');
        }
    }
    if config.no_trailing_newline && out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Canonical one-line rendering of an entry.
fn render_entry(entry: &Entry) -> String {
    match entry {
        Entry::Include(include) => {
            let opt = if include.optional { "?" } else { "" };
            format!("#include{opt} \"{}\"", include.path)
        }
        Entry::Assignment(a) => {
            let mut s = a.key.clone();
            for c in &a.conditions {
                s.push('[');
                s.push_str(&c.key);
                s.push('=');
                s.push_str(&c.value);
                s.push(']');
            }
            s.push_str(" =");
            if !a.value.is_empty() {
                s.push(' ');
                s.push_str(&a.value);
            }
            s
        }
    }
}

/// Parse a captured raw chunk back into the single entry it produced, or
/// `None` when it no longer parses that way (used to detect mutation).
fn reparse_single(raw: &str) -> Option<Entry> {
    let stripped = strip_comments(raw);
    let mut found: Option<Entry> = None;
    for (line_no, _last, line) in LineIter::new(&stripped) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if found.is_some() {
            return None;
        }
        if let Some(rest) = trimmed.strip_prefix("#include") {
            let (optional, rest) = if let Some(r) = rest.strip_prefix('?') {
                (true, r)
            } else {
                (false, rest)
            };
            let path = parse_include_path(rest.trim_start(), line_no).ok()?;
            found = Some(Entry::Include(Include { path, optional }));
        } else {
            found = Some(Entry::Assignment(parse_assignment(trimmed, line_no).ok()?));
        }
    }
    found
}

pub fn parse_file(path: &Path) -> Result<Xcconfig, Error> {
    let s = fs::read_to_string(path)?;
    Ok(parse(&s)?)
}

/// Process-global cache of parsed `.xcconfig` files, validated by `(len, mtime)`.
static CACHE: LazyLock<ParseCache<Xcconfig>> = LazyLock::new(ParseCache::new);

/// Like [`parse_file`] but served from an in-memory, mtime-validated cache.
/// `.xcconfig`s are re-read on every resolve (the project + target base
/// configs, plus any `-xcconfig` overlay and the files its `#include`s pull
/// in), so this caches each file's parse — keyed and re-validated per file, so
/// an edited include still reparses — until it changes on disk.
pub fn parse_file_cached(path: &Path) -> Result<Arc<Xcconfig>, Error> {
    CACHE.get_or_parse(path, parse_file)
}

/// Strip `//` line comments and `/* … */` block comments. Apple's parser
/// skips both forms, anywhere — `//` opens a comment even mid-"URL"
/// (`http://…` truncates to `http:`), and an unterminated `/*` runs to end
/// of input. A block comment reads as whitespace: one that sits within a
/// single line becomes a space, while newlines inside a spanning one are
/// preserved so parse-error line numbers stay aligned with the source.
fn strip_comments(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            // Line comment: drop up to (not including) the newline.
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            let mut newlines = 0usize;
            while i < bytes.len() {
                if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                if bytes[i] == b'\n' {
                    newlines += 1;
                }
                i += 1;
            }
            if newlines == 0 {
                out.push(' ');
            } else {
                out.extend(std::iter::repeat_n('\n', newlines));
            }
        } else {
            let c = input[i..].chars().next().expect("valid UTF-8 boundary");
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

fn parse_include_path(s: &str, line_no: usize) -> Result<String, ParseError> {
    let s = s.trim();
    let s = s.strip_prefix('"').ok_or_else(|| ParseError {
        message: "#include expects a quoted path".into(),
        line: line_no,
    })?;
    let end = s.find('"').ok_or_else(|| ParseError {
        message: "unterminated #include path".into(),
        line: line_no,
    })?;
    Ok(s[..end].to_string())
}

fn parse_assignment(s: &str, line_no: usize) -> Result<Assignment, ParseError> {
    let eq_idx = find_unbracketed_eq(s).ok_or_else(|| ParseError {
        message: "expected '=' in assignment".into(),
        line: line_no,
    })?;
    let lhs = &s[..eq_idx];
    let rhs = s[eq_idx + 1..].trim();
    // Xcconfig syntax doesn't use trailing semicolons (those are pbxproj
    // dictionary terminators) but users frequently sprinkle them in by
    // habit. Apple's resolver tolerates the typo by stripping a single
    // trailing `;`; mirror that.
    let rhs = rhs.strip_suffix(';').map_or(rhs, str::trim_end);
    let (key, conditions) = parse_key_with_conditions(lhs, line_no)?;
    Ok(Assignment {
        key,
        conditions,
        value: rhs.to_string(),
        condition: None,
    })
}

fn find_unbracketed_eq(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'[' => depth += 1,
            b']' => depth -= 1,
            b'=' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_key_with_conditions(
    lhs: &str,
    line_no: usize,
) -> Result<(String, Vec<Condition>), ParseError> {
    let lhs = lhs.trim();
    let (key, mut rest) = match lhs.find('[') {
        Some(idx) => (lhs[..idx].trim().to_string(), &lhs[idx..]),
        None => return Ok((lhs.to_string(), Vec::new())),
    };
    let mut conditions = Vec::new();
    while let Some(stripped) = rest.strip_prefix('[') {
        let end = stripped.find(']').ok_or_else(|| ParseError {
            message: "unterminated condition bracket".into(),
            line: line_no,
        })?;
        let bracket_content = &stripped[..end];
        for part in bracket_content.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let eq = part.find('=').ok_or_else(|| ParseError {
                message: format!("condition '{part}' missing '='"),
                line: line_no,
            })?;
            conditions.push(Condition {
                key: part[..eq].trim().to_string(),
                value: part[eq + 1..].trim().to_string(),
            });
        }
        rest = stripped[end + 1..].trim();
    }
    if !rest.is_empty() {
        return Err(ParseError {
            message: format!("unexpected content after conditions: '{rest}'"),
            line: line_no,
        });
    }
    Ok((key, conditions))
}

struct LineIter<'a> {
    lines: std::str::Lines<'a>,
    line_no: usize,
}

impl<'a> LineIter<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            lines: input.lines(),
            line_no: 0,
        }
    }
}

impl Iterator for LineIter<'_> {
    /// `(first_line, last_line, logical_line)` — the 1-based physical-line
    /// span the (continuation-joined) logical line was assembled from.
    type Item = (usize, usize, String);
    fn next(&mut self) -> Option<Self::Item> {
        let first = self.lines.next()?;
        self.line_no += 1;
        let first_line_no = self.line_no;
        let mut acc = first.to_string();
        while ends_with_unescaped_backslash(&acc) {
            acc = strip_trailing_backslash(&acc);
            acc.push(' ');
            match self.lines.next() {
                Some(line) => {
                    self.line_no += 1;
                    acc.push_str(line.trim_start());
                }
                None => break,
            }
        }
        Some((first_line_no, self.line_no, acc))
    }
}

fn ends_with_unescaped_backslash(s: &str) -> bool {
    let trimmed = s.trim_end();
    trimmed.ends_with('\\') && !trimmed.ends_with("\\\\")
}

fn strip_trailing_backslash(s: &str) -> String {
    s.trim_end()
        .strip_suffix('\\')
        .map_or_else(|| s.to_string(), |stripped| stripped.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assign(key: &str, value: &str) -> Entry {
        Entry::Assignment(Assignment {
            key: key.into(),
            conditions: Vec::new(),
            value: value.into(),
            condition: None,
        })
    }

    fn cond_assign(key: &str, conditions: &[(&str, &str)], value: &str) -> Entry {
        Entry::Assignment(Assignment {
            key: key.into(),
            conditions: conditions
                .iter()
                .map(|(k, v)| Condition {
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            value: value.into(),
            condition: None,
        })
    }

    fn include(path: &str, optional: bool) -> Entry {
        Entry::Include(Include {
            path: path.into(),
            optional,
        })
    }

    #[test]
    fn parses_empty() {
        let c = parse("").unwrap();
        assert!(c.entries.is_empty());
    }

    #[test]
    fn parses_comments_only() {
        let c = parse("// just a comment\n// another\n").unwrap();
        assert!(c.entries.is_empty());
    }

    #[test]
    fn parses_simple_assignment() {
        let c = parse("FOO = bar\n").unwrap();
        assert_eq!(c.entries, vec![assign("FOO", "bar")]);
    }

    #[test]
    fn parses_empty_value() {
        let c = parse("FOO =\n").unwrap();
        assert_eq!(c.entries, vec![assign("FOO", "")]);
    }

    #[test]
    fn strips_trailing_inline_comment() {
        let c = parse("FOO = bar // a comment\n").unwrap();
        assert_eq!(c.entries, vec![assign("FOO", "bar")]);
    }

    #[test]
    fn parses_conditional_sdk() {
        let c = parse("FOO[sdk=iphoneos*] = ios_device\n").unwrap();
        assert_eq!(
            c.entries,
            vec![cond_assign("FOO", &[("sdk", "iphoneos*")], "ios_device")]
        );
    }

    #[test]
    fn parses_multiple_conditions_in_brackets() {
        let c = parse("FOO[sdk=iphoneos*,arch=arm64] = combined\n").unwrap();
        assert_eq!(
            c.entries,
            vec![cond_assign(
                "FOO",
                &[("sdk", "iphoneos*"), ("arch", "arm64")],
                "combined"
            )]
        );
    }

    #[test]
    fn parses_stacked_condition_brackets() {
        let c = parse("FOO[sdk=iphoneos*][arch=arm64] = stacked\n").unwrap();
        assert_eq!(
            c.entries,
            vec![cond_assign(
                "FOO",
                &[("sdk", "iphoneos*"), ("arch", "arm64")],
                "stacked"
            )]
        );
    }

    #[test]
    fn parses_include() {
        let c = parse("#include \"common.xcconfig\"\n").unwrap();
        assert_eq!(c.entries, vec![include("common.xcconfig", false)]);
    }

    #[test]
    fn parses_optional_include() {
        let c = parse("#include? \"maybe.xcconfig\"\n").unwrap();
        assert_eq!(c.entries, vec![include("maybe.xcconfig", true)]);
    }

    #[test]
    fn joins_continuation_lines() {
        let c = parse("FOO = a \\\n    b \\\n    c\n").unwrap();
        assert_eq!(c.entries, vec![assign("FOO", "a b c")]);
    }

    #[test]
    fn preserves_value_with_equals() {
        // `=` inside an unbracketed value is fine; only `=` outside `[...]` is the separator.
        let c = parse("FOO = a=b=c\n").unwrap();
        assert_eq!(c.entries, vec![assign("FOO", "a=b=c")]);
    }

    #[test]
    fn parses_inherited_value() {
        let c = parse("FOO = $(inherited) extra\n").unwrap();
        assert_eq!(c.entries, vec![assign("FOO", "$(inherited) extra")]);
    }

    #[test]
    fn parses_modifier_value() {
        // Values with modifier syntax pass through unchanged at parse time.
        let c = parse("LOW = ${BASE:lower}\n").unwrap();
        assert_eq!(c.entries, vec![assign("LOW", "${BASE:lower}")]);
    }

    #[test]
    fn parses_multiple_entries() {
        let c = parse("A = 1\nB = 2\n#include \"x.xcconfig\"\nC = 3\n").unwrap();
        assert_eq!(c.entries.len(), 4);
        assert_eq!(c.entries[0], assign("A", "1"));
        assert_eq!(c.entries[1], assign("B", "2"));
        assert_eq!(c.entries[2], include("x.xcconfig", false));
        assert_eq!(c.entries[3], assign("C", "3"));
    }

    #[test]
    fn rejects_missing_equals() {
        assert!(parse("FOO bar\n").is_err());
    }

    #[test]
    fn rejects_unterminated_condition() {
        assert!(parse("FOO[sdk=ios = bar\n").is_err());
    }

    #[test]
    fn serialize_round_trips_verbatim() {
        let src = "// header comment\n\nFOO = bar // trailing\nBAZ[sdk=iphoneos*] = qux\n\nQUUX = a \\\n    b\n#include \"other.xcconfig\"\n";
        let c = parse(src).unwrap();
        assert_eq!(serialize(&c), src);
    }

    #[test]
    fn serialize_preserves_missing_trailing_newline() {
        let src = "FOO = bar";
        let c = parse(src).unwrap();
        assert_eq!(serialize(&c), src);
    }

    #[test]
    fn serialize_renders_mutated_entry_canonically_keeping_trivia() {
        let src = "// keep me\nFOO   =   bar // gone after edit\nBAZ = ok\n";
        let mut c = parse(src).unwrap();
        let Entry::Assignment(a) = &mut c.entries[0] else {
            panic!("expected assignment");
        };
        a.value = "changed".into();
        // The edited line is re-rendered canonically (its inline comment and
        // spacing belong to the old value); everything else stays verbatim.
        assert_eq!(serialize(&c), "// keep me\nFOO = changed\nBAZ = ok\n");
    }

    #[test]
    fn serialize_drops_removed_entries_and_appends_new_ones() {
        let src = "A = 1\nB = 2\n";
        let mut c = parse(src).unwrap();
        c.entries.remove(1);
        c.entries.push(Entry::Assignment(Assignment {
            key: "C".into(),
            conditions: vec![Condition {
                key: "sdk".into(),
                value: "macosx*".into(),
            }],
            value: "3".into(),
            condition: None,
        }));
        assert_eq!(serialize(&c), "A = 1\nC[sdk=macosx*] = 3\n");
    }

    #[test]
    fn serialize_renders_programmatic_config_canonically() {
        let c = Xcconfig {
            entries: vec![
                Entry::Include(Include {
                    path: "base.xcconfig".into(),
                    optional: true,
                }),
                Entry::Assignment(Assignment {
                    key: "EMPTY".into(),
                    conditions: Vec::new(),
                    value: String::new(),
                    condition: None,
                }),
            ],
            ..Default::default()
        };
        assert_eq!(serialize(&c), "#include? \"base.xcconfig\"\nEMPTY =\n");
    }
}
