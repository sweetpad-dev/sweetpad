//! Mini-language for evaluating xcspec `Condition` attributes.
//!
//! Matches the grammar implemented by Apple's open-source `swift-build`
//! (`Sources/SWBMacro/MacroConditionExpression.swift`):
//!
//! ```text
//! expression       → ternary
//! ternary          → or ('?' expression ':' ternary)?
//! or               → and ('||' and)*           (alias: `or`)
//! and              → equality ('&&' equality)* (alias: `and`)
//! equality         → unary (('==' | '!=' | 'contains' | 'is' | 'isnot') unary)*
//! unary            → '!' unary | primary       (alias for `!`: `not`)
//! primary          → '(' expression ')' | string
//! string           → '"…"' | '\'…\'' | unquoted-run
//! ```
//!
//! A `string` may contain embedded `$(VAR)` / `${VAR}` references which the
//! evaluator expands against the supplied resolved-settings dict before
//! comparing or boolean-testing.
//!
//! Equality compares the two operands as expanded strings. Boolean context
//! (`&&`, `||`, `!`, the ternary condition) coerces via NSString.boolValue:
//! after optional whitespace, sign, and zeroes, a `Y`/`y`/`T`/`t` or nonzero
//! digit is truthy ("YES", "true", "1", "+9", "01"); everything else —
//! including arbitrary non-empty strings — is false.
//!
//! See `tests/MacroConditionExpressionTests.swift` in `swift-build` for the
//! canonical expected behaviour the tests below mirror.

use std::collections::BTreeMap;

use crate::resolver::expand_one;

/// Parse a raw condition expression. Returns `None` if the input is empty
/// whitespace, which Apple treats as an always-true (i.e. unconditional) test.
#[must_use]
pub fn parse(input: &str) -> Option<Expr> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let tokens = lex(trimmed);
    let mut parser = Parser {
        tokens,
        pos: 0,
        depth: 0,
    };
    let expr = parser.parse_expression();
    // Trailing garbage means a malformed condition; fall back to always-true
    // so the assignment is included (Apple's resolver does the same on parse
    // errors, since the alternative is silently dropping settings).
    if parser.pos != parser.tokens.len() {
        return None;
    }
    Some(expr)
}

/// Evaluate a parsed condition expression against `settings`. References to
/// `$(VAR)` are resolved through `expand_one` so nested expansions and
/// modifiers (`:default=`, `:lower`, etc.) work just like elsewhere.
#[must_use]
pub fn evaluate(expr: &Expr, settings: &BTreeMap<String, String>) -> bool {
    eval_bool(expr, settings)
}

/// AST node. `String` carries the raw source span — variable references are
/// expanded at evaluation time, not parse time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// Quoted or unquoted literal that may embed `$(VAR)` refs.
    Str(String),
    /// `!` / `not`
    Not(Box<Expr>),
    Eq(Box<Expr>, Box<Expr>),
    Ne(Box<Expr>, Box<Expr>),
    Contains(Box<Expr>, Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
}

// =====================================================================
// Lexer
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    EqEq,
    NotEq,
    AndAnd,
    OrOr,
    Bang,
    LParen,
    RParen,
    Question,
    Colon,
    Contains,
    /// Either a quoted string (`"…"`/`'…'` with quotes stripped) or an
    /// unquoted run of constant characters that may include `$(VAR)`
    /// substrings. The lexer preserves the raw text either way; quote
    /// stripping happens here.
    Str(String),
}

#[allow(clippy::too_many_lines)]
fn lex(input: &str) -> Vec<Token> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        match b {
            b'(' => {
                tokens.push(Token::LParen);
                i += 1;
                continue;
            }
            b')' => {
                tokens.push(Token::RParen);
                i += 1;
                continue;
            }
            b'?' => {
                tokens.push(Token::Question);
                i += 1;
                continue;
            }
            b':' => {
                tokens.push(Token::Colon);
                i += 1;
                continue;
            }
            _ => {}
        }
        if bytes[i..].starts_with(b"==") {
            tokens.push(Token::EqEq);
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"!=") {
            tokens.push(Token::NotEq);
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"&&") {
            tokens.push(Token::AndAnd);
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"||") {
            tokens.push(Token::OrOr);
            i += 2;
            continue;
        }
        if b == b'!' {
            tokens.push(Token::Bang);
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            // Quoted string. We accept either quote style and strip the
            // outer quotes. There's no escape processing — every condition
            // in the captured xcspec corpus uses straightforward content.
            let quote = b;
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != quote {
                j += 1;
            }
            let s = std::str::from_utf8(&bytes[start..j])
                .unwrap_or("")
                .to_string();
            tokens.push(Token::Str(s));
            // Advance past the closing quote if present; otherwise to EOF so
            // the parser sees an end-of-input and can recover.
            i = j.saturating_add(1).min(bytes.len());
            continue;
        }
        // Bare run: consume until whitespace or an operator boundary. We
        // also have to handle `$(...)` and `${...}` substrings as nested
        // groups so a paren-close inside a reference doesn't terminate the
        // token.
        let start = i;
        let mut depth: i32 = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if depth == 0 {
                if c.is_ascii_whitespace() {
                    break;
                }
                if matches!(c, b'(' | b')' | b'?' | b':') {
                    break;
                }
                if bytes[i..].starts_with(b"==")
                    || bytes[i..].starts_with(b"!=")
                    || bytes[i..].starts_with(b"&&")
                    || bytes[i..].starts_with(b"||")
                {
                    break;
                }
                if c == b'!' {
                    break;
                }
            }
            if c == b'$' && i + 1 < bytes.len() && (bytes[i + 1] == b'(' || bytes[i + 1] == b'{') {
                depth += 1;
                i += 2;
                continue;
            }
            if (c == b')' || c == b'}') && depth > 0 {
                depth -= 1;
                i += 1;
                continue;
            }
            i += 1;
        }
        let raw = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
        // Word-shaped tokens that double as operators.
        match raw {
            "contains" => tokens.push(Token::Contains),
            "is" => tokens.push(Token::EqEq),
            "isnot" => tokens.push(Token::NotEq),
            "and" => tokens.push(Token::AndAnd),
            "or" => tokens.push(Token::OrOr),
            "not" => tokens.push(Token::Bang),
            _ => tokens.push(Token::Str(raw.to_string())),
        }
    }
    tokens
}

// =====================================================================
// Parser (recursive descent, low → high precedence)
// =====================================================================

/// Grammar-descent depth cap. Real xcspec conditions nest a handful of
/// levels; the cap keeps a pathologically nested expression from
/// overflowing the stack (matching the sibling parsers' guards). Tighter
/// than the structural parsers' caps because each grammar level costs
/// several call frames (expression → ternary → or → and → equality →
/// unary → primary). Past it the parser collapses to its usual recovery
/// value instead of recursing.
const MAX_DEPTH: usize = 64;

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
    fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned()?;
        self.pos += 1;
        Some(t)
    }
    fn eat(&mut self, want: &Token) -> bool {
        if self.peek() == Some(want) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_expression(&mut self) -> Expr {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Expr {
        let cond = self.parse_or();
        if self.eat(&Token::Question) {
            if self.depth >= MAX_DEPTH {
                return cond;
            }
            self.depth += 1;
            let then_branch = self.parse_expression();
            // The `:` is mandatory in a well-formed ternary; missing one we
            // treat as parse error and fall back to the OR expression alone.
            let result = if self.eat(&Token::Colon) {
                let else_branch = self.parse_ternary();
                Expr::Ternary(Box::new(cond), Box::new(then_branch), Box::new(else_branch))
            } else {
                cond
            };
            self.depth -= 1;
            return result;
        }
        cond
    }

    fn parse_or(&mut self) -> Expr {
        let first = self.parse_and();
        if self.peek() != Some(&Token::OrOr) {
            return first;
        }
        let mut alts = vec![first];
        while self.eat(&Token::OrOr) {
            alts.push(self.parse_and());
        }
        Expr::Or(alts)
    }

    fn parse_and(&mut self) -> Expr {
        let first = self.parse_equality();
        if self.peek() != Some(&Token::AndAnd) {
            return first;
        }
        let mut alts = vec![first];
        while self.eat(&Token::AndAnd) {
            alts.push(self.parse_equality());
        }
        Expr::And(alts)
    }

    fn parse_equality(&mut self) -> Expr {
        let mut left = self.parse_unary();
        loop {
            let op = match self.peek() {
                Some(Token::EqEq) => Token::EqEq,
                Some(Token::NotEq) => Token::NotEq,
                Some(Token::Contains) => Token::Contains,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_unary();
            left = match op {
                Token::EqEq => Expr::Eq(Box::new(left), Box::new(right)),
                Token::NotEq => Expr::Ne(Box::new(left), Box::new(right)),
                Token::Contains => Expr::Contains(Box::new(left), Box::new(right)),
                _ => unreachable!(),
            };
        }
        left
    }

    fn parse_unary(&mut self) -> Expr {
        if self.eat(&Token::Bang) {
            if self.depth >= MAX_DEPTH {
                return Expr::Str(String::new());
            }
            self.depth += 1;
            let inner = self.parse_unary();
            self.depth -= 1;
            return Expr::Not(Box::new(inner));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Expr {
        match self.bump() {
            Some(Token::LParen) => {
                if self.depth >= MAX_DEPTH {
                    return Expr::Str(String::new());
                }
                self.depth += 1;
                let inner = self.parse_expression();
                self.depth -= 1;
                // Tolerate missing close paren rather than panic — yields a
                // best-effort parse.
                let _ = self.eat(&Token::RParen);
                inner
            }
            Some(Token::Str(s)) => Expr::Str(s),
            // Misplaced operators / EOF / etc. collapse to an empty constant
            // (false in boolean context, "" in string context). This keeps
            // odd inputs from panicking.
            _ => Expr::Str(String::new()),
        }
    }
}

// =====================================================================
// Evaluator
// =====================================================================

fn eval_string(expr: &Expr, settings: &BTreeMap<String, String>) -> String {
    match expr {
        Expr::Str(s) => expand_one(s, settings),
        Expr::Not(inner) => bool_to_yesno(!eval_bool(inner, settings)),
        Expr::Eq(a, b) => bool_to_yesno(eval_string(a, settings) == eval_string(b, settings)),
        Expr::Ne(a, b) => bool_to_yesno(eval_string(a, settings) != eval_string(b, settings)),
        Expr::Contains(a, b) => {
            bool_to_yesno(eval_string(a, settings).contains(&eval_string(b, settings)))
        }
        Expr::And(alts) => bool_to_yesno(alts.iter().all(|e| eval_bool(e, settings))),
        Expr::Or(alts) => bool_to_yesno(alts.iter().any(|e| eval_bool(e, settings))),
        Expr::Ternary(cond, then_b, else_b) => {
            if eval_bool(cond, settings) {
                eval_string(then_b, settings)
            } else {
                eval_string(else_b, settings)
            }
        }
    }
}

fn eval_bool(expr: &Expr, settings: &BTreeMap<String, String>) -> bool {
    match expr {
        Expr::Str(s) => bool_value(&expand_one(s, settings)),
        Expr::Not(inner) => !eval_bool(inner, settings),
        Expr::Eq(a, b) => eval_string(a, settings) == eval_string(b, settings),
        Expr::Ne(a, b) => eval_string(a, settings) != eval_string(b, settings),
        Expr::Contains(a, b) => eval_string(a, settings).contains(&eval_string(b, settings)),
        Expr::And(alts) => alts.iter().all(|e| eval_bool(e, settings)),
        Expr::Or(alts) => alts.iter().any(|e| eval_bool(e, settings)),
        Expr::Ternary(cond, then_b, else_b) => {
            if eval_bool(cond, settings) {
                eval_bool(then_b, settings)
            } else {
                eval_bool(else_b, settings)
            }
        }
    }
}

fn bool_value(s: &str) -> bool {
    // NSString.boolValue semantics — what Apple's spec engine coerces with:
    // skip leading whitespace, an optional `+`/`-` sign, and any zeroes;
    // truthy iff the next character is `Y`/`y`/`T`/`t` (so "YES", "yes",
    // "true", "t" …) or a nonzero digit ("1", "9", "+1", "01"). Everything
    // else — "NO", "0", "", "no1", arbitrary strings — is false.
    let mut chars = s.trim_start().chars();
    let mut c = chars.next();
    if matches!(c, Some('+' | '-')) {
        c = chars.next();
    }
    while c == Some('0') {
        c = chars.next();
    }
    matches!(c, Some('Y' | 'y' | 'T' | 't' | '1'..='9'))
}

fn bool_to_yesno(b: bool) -> String {
    if b { "YES".into() } else { "NO".into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn ev(input: &str, settings: &BTreeMap<String, String>) -> bool {
        evaluate(&parse(input).unwrap(), settings)
    }

    // The cases below mirror swift-build's MacroConditionExpressionTests.

    #[test]
    fn constants() {
        let s = BTreeMap::new();
        assert!(!ev("\"\"", &s));
        assert!(!ev("\"const\"", &s));
        assert!(ev("\"YES\"", &s));
        assert!(!ev("\"NO\"", &s));
        // Unquoted bare YES/NO behave the same.
        assert!(ev("YES", &s));
        assert!(!ev("NO", &s));
    }

    #[test]
    fn references_to_undefined_are_false() {
        let s = BTreeMap::new();
        assert!(!ev("$(UNSET)", &s));
    }

    #[test]
    fn references_pick_up_settings() {
        let s = dict(&[("IS_TRUE", "YES"), ("IS_FALSE", "NO")]);
        assert!(ev("$(IS_TRUE)", &s));
        assert!(!ev("$(IS_FALSE)", &s));
    }

    #[test]
    fn arbitrary_string_value_is_false_in_bool_context() {
        let s = dict(&[("FOO", "foobar")]);
        // "foobar" is truthy *only* if it equals "YES"; here it doesn't.
        assert!(!ev("$(FOO)", &s));
    }

    #[test]
    fn equality_compares_as_strings() {
        let s = dict(&[("FOO", "foo")]);
        assert!(ev("$(FOO) == foo", &s));
        assert!(!ev("$(FOO) == bar", &s));
        assert!(ev("$(FOO) != bar", &s));
        assert!(!ev("$(FOO) != foo", &s));
    }

    #[test]
    fn equality_against_empty_string() {
        let s = dict(&[("EMPTY", "")]);
        assert!(ev("$(EMPTY) == \"\"", &s));
        assert!(ev("foo != \"\"", &s));
    }

    #[test]
    fn quoted_and_unquoted_literals_compare_equal() {
        let s = dict(&[("X", "mh_dylib")]);
        assert!(ev("$(X) == mh_dylib", &s));
        assert!(ev("$(X) == 'mh_dylib'", &s));
        assert!(ev("$(X) == \"mh_dylib\"", &s));
    }

    #[test]
    fn contains_is_substring() {
        let s = dict(&[("HAY", "needle in haystack"), ("NEEDLE", "needle")]);
        assert!(ev("$(HAY) contains $(NEEDLE)", &s));
        assert!(ev("$(HAY) contains needle", &s));
        assert!(!ev("$(HAY) contains missing", &s));
    }

    #[test]
    fn unary_not() {
        let s = dict(&[("Y", "YES"), ("N", "NO")]);
        assert!(!ev("!$(Y)", &s));
        assert!(ev("!$(N)", &s));
        assert!(ev("not $(N)", &s));
    }

    #[test]
    fn logical_and_or() {
        let s = dict(&[("A", "YES"), ("B", "YES"), ("C", "NO")]);
        assert!(ev("$(A) && $(B)", &s));
        assert!(!ev("$(A) && $(C)", &s));
        assert!(ev("$(A) || $(C)", &s));
        assert!(!ev("$(C) || $(C)", &s));
        assert!(ev("$(A) and $(B)", &s));
        assert!(ev("$(C) or $(A)", &s));
    }

    #[test]
    fn parens_override_precedence() {
        let s = dict(&[("FOO", "foo"), ("BAR", "baz"), ("BAZ", "baz")]);
        assert!(ev("$(FOO) == foo && ($(BAR) == baz || $(BAZ) == baz)", &s));
    }

    #[test]
    fn ternary_picks_branch() {
        let s = dict(&[("T", "YES"), ("F", "NO")]);
        assert!(ev("$(T) ? YES : NO", &s));
        assert!(!ev("$(F) ? YES : NO", &s));
    }

    #[test]
    fn corpus_examples_parse_and_evaluate() {
        // A sampling of the actual condition strings that ship in the
        // captured xcspec corpus. We're just confirming none of them blow
        // up during parse/evaluate.
        let s = dict(&[
            ("MACH_O_TYPE", "mh_dylib"),
            ("CLANG_ENABLE_MODULES", "YES"),
            ("LINKER_DRIVER", "clang"),
            ("PLATFORM_NAME", "macosx"),
            ("SWIFT_OPTIMIZATION_LEVEL", "-Onone"),
            ("DEPLOYMENT_POSTPROCESSING", "YES"),
            ("SKIP_INSTALL", "NO"),
            ("INSTALL_PATH", "/Applications"),
            ("PRODUCT_TYPE", "com.apple.product-type.application"),
        ]);
        for raw in [
            "$(MACH_O_TYPE) == mh_dylib",
            "$(MACH_O_TYPE) == mh_dylib && $(LINKER_DRIVER) == clang",
            "$(MACH_O_TYPE) == mh_execute || $(MACH_O_TYPE) == mh_bundle",
            "$(CLANG_ENABLE_MODULES)",
            "$(PLATFORM_NAME) == 'macosx'",
            "$(SWIFT_OPTIMIZATION_LEVEL) != '-Onone'",
            "$(DEPLOYMENT_POSTPROCESSING)  &&  !$(SKIP_INSTALL)  &&  $(INSTALL_PATH) != \"\"",
            "$(PRODUCT_TYPE) == 'com.apple.product-type.library.static'",
        ] {
            // Just ensure each parses without crashing.
            let _ = ev(raw, &s);
        }
        // Targeted checks.
        assert!(ev("$(MACH_O_TYPE) == mh_dylib", &s));
        assert!(ev("$(CLANG_ENABLE_MODULES)", &s));
        assert!(ev(
            "$(DEPLOYMENT_POSTPROCESSING)  &&  !$(SKIP_INSTALL)  &&  $(INSTALL_PATH) != \"\"",
            &s
        ));
        assert!(!ev(
            "$(MACH_O_TYPE) == mh_execute || $(MACH_O_TYPE) == mh_bundle",
            &s
        ));
    }

    #[test]
    fn bare_no_as_whole_condition_is_false() {
        // The corpus contains a few `Condition = NO;` entries where the
        // whole expression is just the literal "NO" — the Property must
        // never apply.
        let s = BTreeMap::new();
        assert!(!ev("NO", &s));
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(parse("").is_none());
        assert!(parse("   ").is_none());
    }
}
