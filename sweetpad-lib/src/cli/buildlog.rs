//! A native reimplementation of `xcbeautify`: parse raw `xcodebuild` output into
//! structured [`Event`]s, then render a concise, colorized stream. Parsing is
//! decoupled from rendering so the events can also feed other consumers (CI
//! summaries, diagnostics) later.
//!
//! [`parse_line`] is pure and exhaustively unit-tested without Xcode; [`run`]
//! wires it to a live `xcodebuild` via [`crate::cli::process::stream_lines`].

use std::path::Path;

use crate::cli::output::Output;
use crate::cli::{CliError, process};

/// A structured event parsed from one line of `xcodebuild` output.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A source file compiled (`CompileSwift`, `CompileC`, …).
    Compile { name: String },
    /// Linking a binary (`Ld`).
    Link { target: String },
    /// Code signing (`CodeSign`).
    CodeSign { name: String },
    /// Resource copy / asset step (low-signal; shown only when verbose).
    Copy { name: String },
    /// Info.plist processing (low-signal).
    ProcessPlist { name: String },
    /// A compiler/linker diagnostic.
    Diagnostic {
        kind: DiagKind,
        /// `file:line:col` when present.
        location: Option<String>,
        message: String,
    },
    /// A passed test case.
    TestPassed { name: String, duration: String },
    /// A failed test case.
    TestFailed { name: String },
    /// A test suite that just started.
    SuiteStarted { name: String },
    /// A terminal `** … **` banner.
    Result(ResultKind),
    /// Anything not recognized (shown only when verbose).
    Other(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum DiagKind {
    Warning,
    Error,
    Note,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResultKind {
    BuildSucceeded,
    BuildFailed,
    TestSucceeded,
    TestFailed,
    CleanSucceeded,
}

/// Parse a single line of `xcodebuild` output. Always returns an event
/// ([`Event::Other`] for unrecognized lines).
#[must_use]
pub fn parse_line(line: &str) -> Event {
    let t = line.trim();
    parse_banner(t)
        .or_else(|| parse_test(t))
        .or_else(|| parse_diagnostic(line, t))
        .unwrap_or_else(|| parse_task(line, t))
}

/// Terminal `** … **` banners.
fn parse_banner(t: &str) -> Option<Event> {
    let kind = if t.contains("** BUILD SUCCEEDED **") {
        ResultKind::BuildSucceeded
    } else if t.contains("** BUILD FAILED **") {
        ResultKind::BuildFailed
    } else if t.contains("** TEST SUCCEEDED **") {
        ResultKind::TestSucceeded
    } else if t.contains("** TEST FAILED **") {
        ResultKind::TestFailed
    } else if t.contains("** CLEAN SUCCEEDED **") {
        ResultKind::CleanSucceeded
    } else {
        return None;
    };
    Some(Event::Result(kind))
}

/// Test case and suite lines.
fn parse_test(t: &str) -> Option<Event> {
    if let Some(rest) = t.strip_prefix("Test Case '")
        && let Some((name, tail)) = rest.split_once("' ")
    {
        let name = clean_test_name(name);
        if tail.starts_with("passed") {
            return Some(Event::TestPassed {
                name,
                duration: parse_paren(tail),
            });
        }
        if tail.starts_with("failed") {
            return Some(Event::TestFailed { name });
        }
    }
    if let Some(rest) = t.strip_prefix("Test Suite '")
        && t.contains("started")
        && let Some((name, _)) = rest.split_once('\'')
    {
        return Some(Event::SuiteStarted {
            name: name.to_string(),
        });
    }
    None
}

/// Compiler/linker diagnostics, with or without a `file:line:col` prefix.
fn parse_diagnostic(line: &str, t: &str) -> Option<Event> {
    for (marker, kind) in [
        (": error: ", DiagKind::Error),
        (": warning: ", DiagKind::Warning),
        (": note: ", DiagKind::Note),
    ] {
        if let Some(idx) = line.find(marker) {
            return Some(Event::Diagnostic {
                kind,
                location: Some(line[..idx].trim().to_string()),
                message: line[idx + marker.len()..].trim().to_string(),
            });
        }
    }
    if let Some(rest) = t.strip_prefix("error: ") {
        return Some(Event::Diagnostic {
            kind: DiagKind::Error,
            location: None,
            message: rest.to_string(),
        });
    }
    if let Some(rest) = t.strip_prefix("warning: ") {
        return Some(Event::Diagnostic {
            kind: DiagKind::Warning,
            location: None,
            message: rest.to_string(),
        });
    }
    None
}

/// Task lines, keyed on the leading verb; unrecognized lines become `Other`.
fn parse_task(line: &str, t: &str) -> Event {
    match t.split_whitespace().next().unwrap_or("") {
        "CompileSwift" | "SwiftCompile" | "CompileC" | "CompileXIB" | "CompileStoryboard" => {
            Event::Compile {
                name: source_name(t).unwrap_or_else(|| "source".to_string()),
            }
        }
        "CompileSwiftSources" => Event::Compile {
            name: "Swift sources".to_string(),
        },
        "CompileAssetCatalog" => Event::Copy {
            name: "asset catalog".to_string(),
        },
        "Ld" => Event::Link {
            target: t
                .split_whitespace()
                .nth(1)
                .map_or_else(|| "binary".to_string(), base),
        },
        "CodeSign" => Event::CodeSign {
            name: t
                .split_whitespace()
                .nth(1)
                .map_or_else(|| "bundle".to_string(), base),
        },
        "CpResource" | "PBXCp" | "Copy" | "CpHeader" | "Ditto" | "CopySwiftLibs" => Event::Copy {
            name: last_token(t).map_or_else(|| "files".to_string(), |s| base(&s)),
        },
        "ProcessInfoPlistFile" => Event::ProcessPlist {
            name: t
                .split_whitespace()
                .nth(1)
                .map_or_else(|| "Info.plist".to_string(), base),
        },
        _ => Event::Other(line.to_string()),
    }
}

/// Render an event for the terminal, or `None` to suppress it. `verbose` keeps
/// low-signal lines (copies, plist, notes, unrecognized output).
#[must_use]
pub fn render(event: &Event, color: bool, verbose: bool) -> Option<String> {
    let c = Colors::new(color);
    match event {
        Event::Compile { name } => Some(c.dim(&format!("  Compiling {name}"))),
        Event::Link { target } => Some(c.dim(&format!("  Linking {target}"))),
        Event::CodeSign { name } => Some(c.dim(&format!("  Signing {name}"))),
        Event::Copy { name } => verbose.then(|| c.dim(&format!("  Copying {name}"))),
        Event::ProcessPlist { name } => verbose.then(|| c.dim(&format!("  Processing {name}"))),
        Event::Diagnostic {
            kind,
            location,
            message,
        } => {
            let loc = location
                .as_deref()
                .map(|l| format!("{l}: "))
                .unwrap_or_default();
            match kind {
                DiagKind::Error => Some(c.red(&format!("error: {loc}{message}"))),
                DiagKind::Warning => Some(c.yellow(&format!("warning: {loc}{message}"))),
                DiagKind::Note => verbose.then(|| c.dim(&format!("note: {loc}{message}"))),
            }
        }
        Event::TestPassed { name, duration } => Some(c.green(&format!("  ✓ {name} ({duration})"))),
        Event::TestFailed { name } => Some(c.red(&format!("  ✗ {name}"))),
        Event::SuiteStarted { name } => Some(c.bold(&format!("Suite {name}"))),
        Event::Result(kind) => Some(match kind {
            ResultKind::BuildSucceeded => c.green_bold("✓ Build succeeded"),
            ResultKind::CleanSucceeded => c.green_bold("✓ Clean succeeded"),
            ResultKind::TestSucceeded => c.green_bold("✓ Tests succeeded"),
            ResultKind::BuildFailed => c.red_bold("✗ Build failed"),
            ResultKind::TestFailed => c.red_bold("✗ Tests failed"),
        }),
        Event::Other(raw) => verbose.then(|| raw.clone()),
    }
}

/// Run a command, beautifying its stdout line-by-line via [`parse_line`] /
/// [`render`]. Returns whether it succeeded.
pub fn run(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    out: &Output,
) -> Result<bool, CliError> {
    let color = out.use_color();
    let verbose = out.is_verbose();
    process::stream_lines(program, args, cwd, |line| {
        if let Some(rendered) = render(&parse_line(line), color, verbose) {
            out.line(&rendered);
        }
    })
}

// --- helpers ---

/// Final path component of `path`.
fn base(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// First whitespace-separated token that looks like a source file.
fn source_name(line: &str) -> Option<String> {
    const EXTS: [&str; 9] = [
        ".swift",
        ".m",
        ".mm",
        ".c",
        ".cpp",
        ".cc",
        ".metal",
        ".xib",
        ".storyboard",
    ];
    line.split_whitespace()
        .find(|tok| EXTS.iter().any(|e| tok.ends_with(e)))
        .map(base)
}

/// Last whitespace-separated token of a line.
fn last_token(line: &str) -> Option<String> {
    line.split_whitespace().last().map(str::to_string)
}

/// `-[AppTests testArithmetic]` → `AppTests.testArithmetic`.
fn clean_test_name(raw: &str) -> String {
    raw.trim_matches(|c| c == '-' || c == '+' || c == '[' || c == ']')
        .replace(' ', ".")
}

/// Extract `0.123 seconds` from `passed (0.123 seconds).`.
fn parse_paren(tail: &str) -> String {
    match (tail.find('('), tail.find(')')) {
        (Some(a), Some(b)) if b > a + 1 => tail[a + 1..b].to_string(),
        _ => String::new(),
    }
}

/// ANSI color helpers, no-ops when color is disabled.
struct Colors {
    on: bool,
}

impl Colors {
    fn new(on: bool) -> Self {
        Self { on }
    }
    fn wrap(&self, code: &str, s: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    fn dim(&self, s: &str) -> String {
        self.wrap("2", s)
    }
    fn red(&self, s: &str) -> String {
        self.wrap("31", s)
    }
    fn yellow(&self, s: &str) -> String {
        self.wrap("33", s)
    }
    fn green(&self, s: &str) -> String {
        self.wrap("32", s)
    }
    fn bold(&self, s: &str) -> String {
        self.wrap("1", s)
    }
    fn green_bold(&self, s: &str) -> String {
        self.wrap("1;32", s)
    }
    fn red_bold(&self, s: &str) -> String {
        self.wrap("1;31", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compile_lines() {
        assert_eq!(
            parse_line("CompileSwift normal arm64 /a/b/ContentView.swift (in target 'X')"),
            Event::Compile {
                name: "ContentView.swift".to_string()
            }
        );
        assert_eq!(
            parse_line(
                "CompileC /o/foo.o /a/foo.c normal arm64 c com.apple.compilers.llvm.clang.1_0.compiler"
            ),
            Event::Compile {
                name: "foo.c".to_string()
            }
        );
    }

    #[test]
    fn parses_link_and_sign() {
        assert_eq!(
            parse_line("Ld /d/App.app/App normal (in target 'App')"),
            Event::Link {
                target: "App".to_string()
            }
        );
        assert_eq!(
            parse_line("CodeSign /d/App.app"),
            Event::CodeSign {
                name: "App.app".to_string()
            }
        );
    }

    #[test]
    fn parses_diagnostics_with_location() {
        let e = parse_line("/a/File.swift:10:5: error: cannot find 'foo' in scope");
        assert_eq!(
            e,
            Event::Diagnostic {
                kind: DiagKind::Error,
                location: Some("/a/File.swift:10:5".to_string()),
                message: "cannot find 'foo' in scope".to_string(),
            }
        );
        assert!(matches!(
            parse_line("/a/File.swift:3:1: warning: unused variable"),
            Event::Diagnostic {
                kind: DiagKind::Warning,
                ..
            }
        ));
    }

    #[test]
    fn parses_test_cases() {
        assert_eq!(
            parse_line("Test Case '-[AppTests testArithmetic]' passed (0.001 seconds)."),
            Event::TestPassed {
                name: "AppTests.testArithmetic".to_string(),
                duration: "0.001 seconds".to_string()
            }
        );
        assert_eq!(
            parse_line("Test Case '-[AppTests testBoom]' failed (0.002 seconds)."),
            Event::TestFailed {
                name: "AppTests.testBoom".to_string()
            }
        );
    }

    #[test]
    fn parses_result_banners() {
        assert_eq!(
            parse_line("** BUILD SUCCEEDED **"),
            Event::Result(ResultKind::BuildSucceeded)
        );
        assert_eq!(
            parse_line("** TEST FAILED **"),
            Event::Result(ResultKind::TestFailed)
        );
    }

    #[test]
    fn unknown_line_is_other() {
        assert_eq!(parse_line("note: Using new build system"), {
            // "note: " prefix isn't a diagnostic marker form we special-case at start
            Event::Other("note: Using new build system".to_string())
        });
        assert_eq!(
            parse_line("random noise"),
            Event::Other("random noise".to_string())
        );
    }

    #[test]
    fn render_suppresses_noise_unless_verbose() {
        let copy = Event::Copy {
            name: "x".to_string(),
        };
        assert!(render(&copy, false, false).is_none());
        assert!(render(&copy, false, true).is_some());

        // Errors always show.
        let err = Event::Diagnostic {
            kind: DiagKind::Error,
            location: None,
            message: "boom".into(),
        };
        assert_eq!(render(&err, false, false), Some("error: boom".to_string()));
    }

    #[test]
    fn render_colorizes_when_enabled() {
        let ok = Event::Result(ResultKind::BuildSucceeded);
        let plain = render(&ok, false, false).unwrap();
        let colored = render(&ok, true, false).unwrap();
        assert!(!plain.contains('\x1b'));
        assert!(colored.contains('\x1b'));
    }
}
