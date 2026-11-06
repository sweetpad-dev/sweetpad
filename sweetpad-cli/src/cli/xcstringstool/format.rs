//! The printf-style specifiers in a String Catalog's strings: `%lld`, `%2$@`,
//! `%.1f`, and the `%#@name@` a substitution fills in.

use std::fmt::Write as _;
use std::ops::Range;

/// The conversions a specifier can end in.
const CONVERSIONS: &[u8] = b"diouxXfFeEgGaAcsSpn@C";

/// The conversions that format a number.
const NUMERIC: &[u8] = b"diouxXfFeEgGaA";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token<'a> {
    /// `%2$.1lf`: position 2, options `.1`, length `l`, conversion `f`.
    Spec {
        range: Range<usize>,
        position: Option<usize>,
        options: &'a str,
        length: &'a str,
        conversion: char,
    },
    /// `%#@name@`, or `%1$#@name@`.
    Substitution {
        range: Range<usize>,
        position: Option<usize>,
        name: &'a str,
    },
}

impl Token<'_> {
    pub fn range(&self) -> Range<usize> {
        match self {
            Token::Spec { range, .. } | Token::Substitution { range, .. } => range.clone(),
        }
    }

    pub fn position(&self) -> Option<usize> {
        match self {
            Token::Spec { position, .. } | Token::Substitution { position, .. } => *position,
        }
    }

    /// Whether a specifier formats a number.
    pub fn is_numeric(&self) -> bool {
        matches!(self, Token::Spec { conversion, .. } if NUMERIC.contains(&(*conversion as u8)))
    }

    /// The length and conversion, `lld` in `%2$lld`.
    pub fn value_type(&self) -> String {
        match self {
            Token::Spec {
                length, conversion, ..
            } => format!("{length}{conversion}"),
            Token::Substitution { .. } => String::new(),
        }
    }

    /// The specifier without its position: `%.1f` for `%2$.1f`.
    pub fn unpositioned(&self) -> String {
        match self {
            Token::Spec {
                options,
                length,
                conversion,
                ..
            } => format!("%{options}{length}{conversion}"),
            Token::Substitution { name, .. } => format!("%#@{name}@"),
        }
    }
}

/// The specifiers in `text`, in order. `%%` is a percent sign, not one.
pub fn tokens(text: &str) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 1;
        if bytes.get(j) == Some(&b'%') {
            i = j + 1;
            continue;
        }
        let digits = j + bytes[j..].iter().take_while(|b| b.is_ascii_digit()).count();
        let mut position = None;
        if digits > j && bytes.get(digits) == Some(&b'$') {
            position = text[j..digits].parse().ok();
            j = digits + 1;
        }
        if bytes[j..].starts_with(b"#@")
            && let Some(end) = bytes[j + 2..].iter().position(|b| *b == b'@')
        {
            let end = j + 2 + end;
            out.push(Token::Substitution {
                range: start..end + 1,
                position,
                name: &text[j + 2..end],
            });
            i = end + 1;
            continue;
        }
        let options_start = j;
        j += bytes[j..]
            .iter()
            .take_while(|b| b"-+#0".contains(b))
            .count();
        j += bytes[j..]
            .iter()
            .take_while(|b| b.is_ascii_digit() || **b == b'*')
            .count();
        if bytes.get(j) == Some(&b'.') {
            j += 1;
            j += bytes[j..]
                .iter()
                .take_while(|b| b.is_ascii_digit() || **b == b'*')
                .count();
        }
        let options = &text[options_start..j];
        let length_start = j;
        if let Some(length) = ["hh", "ll", "h", "l", "q", "L", "z", "t", "j"]
            .iter()
            .find(|l| bytes[j..].starts_with(l.as_bytes()))
        {
            j += length.len();
        }
        match bytes.get(j) {
            Some(c) if CONVERSIONS.contains(c) => {
                out.push(Token::Spec {
                    range: start..j + 1,
                    position,
                    options,
                    length: &text[length_start..j],
                    conversion: *c as char,
                });
                i = j + 1;
            }
            _ => i = start + 1,
        }
    }
    out
}

/// `text` with every specifier given its position, counting from one in
/// the order they appear: `%@ has %lld` becomes `%1$@ has %2$lld`.
pub fn positioned(text: &str) -> String {
    let mut out = String::new();
    let mut last = 0;
    for (index, token) in tokens(text).iter().enumerate() {
        let range = token.range();
        out.push_str(&text[last..range.start]);
        if token.position().is_some() {
            out.push_str(&text[range.clone()]);
        } else {
            let _ = write!(out, "%{}${}", index + 1, &text[range.start + 1..range.end]);
        }
        last = range.end;
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specifiers_and_substitutions_are_found() {
        let text = "%2$.1lf of %@, 100%% and %#@n@ then %3$#@m@ %d%";
        let found: Vec<String> = tokens(text)
            .iter()
            .map(|t| format!("{:?}:{}", t.position(), &text[t.range()]))
            .collect();
        assert_eq!(
            found,
            [
                "Some(2):%2$.1lf",
                "None:%@",
                "None:%#@n@",
                "Some(3):%3$#@m@",
                "None:%d"
            ]
        );
        let first = &tokens(text)[0];
        assert_eq!(first.value_type(), "lf");
        assert_eq!(first.unpositioned(), "%.1lf");
        assert!(first.is_numeric());
        assert!(!tokens(text)[1].is_numeric());
    }

    #[test]
    fn positioning_numbers_specifiers_in_order() {
        assert_eq!(positioned("%@ has %lld file"), "%1$@ has %2$lld file");
        assert_eq!(positioned("100%% of %d"), "100%% of %1$d");
    }
}
