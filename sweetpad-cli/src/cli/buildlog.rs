//! A native reimplementation of `xcbeautify`: parse raw `xcodebuild` output into
//! structured [`Event`]s, then render a concise, colorized stream. Parsing is
//! decoupled from rendering so the events can also feed other consumers (CI
//! summaries, diagnostics) later.
//!
//! [`parse_line`] is pure and exhaustively unit-tested without Xcode; [`run`]
//! wires it to a live `xcodebuild` via [`crate::cli::process::stream_lines`].

use std::path::Path;
use std::time::{Duration, Instant};

use crate::cli::output::Output;
use crate::cli::progress::Spinner;
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
            // Tool-prefixed diagnostics (`clang: error: …`, `xcodebuild:
            // error: …`) put the tool name where a location would sit; a
            // non-path prefix folds into the message instead of becoming a
            // bogus `file=clang` in artifacts and annotations.
            let prefix = line[..idx].trim();
            let (location, message) = if location_like(prefix) {
                (
                    Some(prefix.to_string()),
                    line[idx + marker.len()..].trim().to_string(),
                )
            } else {
                (
                    None,
                    format!("{prefix}: {}", line[idx + marker.len()..].trim()),
                )
            };
            return Some(Event::Diagnostic {
                kind,
                location,
                message,
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
/// low-signal lines (copies, plist, notes, unrecognized output); `quiet` keeps
/// only what can't be ignored — errors, warnings, and failure banners — so
/// `sweetpad -q build start` is silent until something is wrong.
#[must_use]
pub fn render(event: &Event, color: bool, verbose: bool, quiet: bool) -> Option<String> {
    let c = Colors::new(color);
    if quiet {
        return match event {
            Event::Diagnostic {
                kind: DiagKind::Error,
                ..
            }
            | Event::Diagnostic {
                kind: DiagKind::Warning,
                ..
            }
            | Event::Result(ResultKind::BuildFailed | ResultKind::TestFailed)
            | Event::TestFailed { .. } => render(event, color, verbose, false),
            _ => None,
        };
    }
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

/// The live progress of a build: a `⠋ label Ns` spinner shown while
/// `xcodebuild` is in one of its silent stretches — the planning prelude, or an
/// up-to-date build that compiles nothing — erased the moment the first real
/// line renders so the streamed output takes its place. Inert off an
/// interactive TTY. The same start instant stamps the elapsed time onto the
/// closing success banner.
#[allow(clippy::struct_excessive_bools)] // captured Output toggles, not a state machine
pub struct BuildProgress {
    spinner: Option<Spinner>,
    start: Instant,
    color: bool,
    verbose: bool,
    quiet: bool,
    gh_annotations: bool,
}

impl BuildProgress {
    /// Begin tracking a build, animating `⠋ label …` until the first line prints.
    /// `label` names the action (`Building`, `Testing`). `--quiet` suppresses
    /// the spinner along with the rest of the progress chatter.
    #[must_use]
    pub fn start(out: &Output, label: &str) -> Self {
        Self {
            spinner: Some(Spinner::start_timed(
                label,
                out.is_interactive() && !out.is_quiet(),
                out.use_color(),
            )),
            start: Instant::now(),
            color: out.use_color(),
            verbose: out.is_verbose(),
            quiet: out.is_quiet(),
            gh_annotations: out.gh_annotations(),
        }
    }

    /// Render one raw `xcodebuild` line for display, or `None` to suppress it.
    /// The first line that renders stops and erases the spinner; the closing
    /// success banner is stamped with the elapsed build time. With
    /// `--gh-annotations`, a diagnostic also carries its `::error`/`::warning`
    /// workflow-command line.
    pub fn line(&mut self, raw: &str) -> Option<String> {
        let event = parse_line(raw);
        let mut rendered = render(&event, self.color, self.verbose, self.quiet)?;
        // First line through — hand the terminal over from the spinner to the
        // streamed output (dropping the spinner erases its line).
        self.spinner = None;
        if self.gh_annotations
            && let Some(annotation) = gh_annotation(&event)
        {
            rendered.push('\n');
            rendered.push_str(&annotation);
        }
        Some(stamp_time(
            rendered,
            &event,
            self.start.elapsed(),
            self.color,
        ))
    }
}

/// Whether a diagnostic prefix names a source location (`file[:line[:col]]`)
/// rather than the emitting tool: path-shaped (contains a separator), or a
/// bare file with a plausible extension.
fn location_like(prefix: &str) -> bool {
    if prefix.is_empty() {
        return false;
    }
    let file = prefix.split(':').next().unwrap_or(prefix);
    file.contains('/') || Path::new(file).extension().is_some()
}

/// The GitHub Actions workflow-command line for a diagnostic
/// (`::error file=…,line=…::message`), or `None` for everything else.
#[must_use]
pub fn gh_annotation(event: &Event) -> Option<String> {
    let Event::Diagnostic {
        kind,
        location,
        message,
    } = event
    else {
        return None;
    };
    let level = match kind {
        DiagKind::Error => "error",
        DiagKind::Warning => "warning",
        DiagKind::Note => return None,
    };
    // Workflow commands need %/\r/\n escaped in the message.
    let escaped = message
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A");
    let props = location.as_deref().map(location_props).unwrap_or_default();
    Some(format!("::{level}{props}::{escaped}"))
}

/// `file:line:col` → ` file=…,line=…,col=…` annotation properties; a location
/// that doesn't parse still anchors as a bare `file=`. Property values get
/// the workflow-command escaping GitHub requires (`%`, `\r`, `\n`, `:`, `,`)
/// — an unescaped comma in a path would split the property list and anchor
/// the annotation to the wrong file.
fn location_props(loc: &str) -> String {
    use std::fmt::Write as _;
    let mut parts = loc.rsplitn(3, ':');
    let col = parts.next().and_then(|p| p.parse::<u32>().ok());
    let (line, file) = match (col, parts.next(), parts.next()) {
        (Some(_), Some(line), Some(file)) if line.parse::<u32>().is_ok() => {
            (Some(line.to_string()), file.to_string())
        }
        _ => (None, loc.to_string()),
    };
    let mut props = format!(" file={}", escape_property(&file));
    if let Some(line) = line {
        let _ = write!(props, ",line={line}");
        if let Some(col) = col {
            let _ = write!(props, ",col={col}");
        }
    }
    props
}

/// GitHub workflow-command *property* escaping (stricter than the message's:
/// `:` and `,` delimit properties).
fn escape_property(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace(':', "%3A")
        .replace(',', "%2C")
}

/// Append the elapsed time to a terminal *success* banner (`✓ Build succeeded
/// (5.3s)`); every other line passes through untouched.
fn stamp_time(rendered: String, event: &Event, elapsed: Duration, color: bool) -> String {
    if matches!(
        event,
        Event::Result(
            ResultKind::BuildSucceeded | ResultKind::CleanSucceeded | ResultKind::TestSucceeded
        )
    ) {
        let time = Colors::new(color).green_bold(&format!("({:.1}s)", elapsed.as_secs_f64()));
        format!("{rendered} {time}")
    } else {
        rendered
    }
}

/// Run a command, beautifying its stdout line-by-line via [`BuildProgress`].
/// Returns whether it succeeded.
pub fn run(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    out: &Output,
    label: &str,
) -> Result<bool, CliError> {
    Ok(run_collecting(program, args, cwd, out, label)?.0)
}

/// Like [`run`], but also collects each diagnostic as its
/// [`event_json`]-shaped object — the input to the last-build diagnostics
/// artifact.
pub fn run_collecting(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    out: &Output,
    label: &str,
) -> Result<(bool, Vec<serde_json::Value>), CliError> {
    let mut progress = BuildProgress::start(out, label);
    let mut diagnostics = Vec::new();
    let ok = process::stream_lines(program, args, cwd, |line| {
        if let Event::Diagnostic { .. } = parse_line(line)
            && let Some(json) = event_json(&parse_line(line))
        {
            diagnostics.push(json);
        }
        if let Some(rendered) = progress.line(line) {
            out.line(&rendered);
        }
    })?;
    Ok((ok, diagnostics))
}

/// Diagnostics parsed out of a full captured transcript (the `--json` path).
#[must_use]
pub fn diagnostics_from_transcript(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .filter_map(|line| {
            let event = parse_line(line);
            matches!(event, Event::Diagnostic { .. })
                .then(|| event_json(&event))
                .flatten()
        })
        .collect()
}

/// One parsed [`Event`] as an NDJSON object for `-o ndjson` consumers, or
/// `None` for lines with no machine value (unrecognized output, and the raw
/// `** … **` banners — the stream's own terminal result event carries the
/// outcome instead).
#[must_use]
pub fn event_json(event: &Event) -> Option<serde_json::Value> {
    use serde_json::json;
    let task = |kind: &str, name: &str| json!({ "event": "task", "kind": kind, "name": name });
    Some(match event {
        Event::Compile { name } => task("compile", name),
        Event::Link { target } => task("link", target),
        Event::CodeSign { name } => task("codesign", name),
        Event::Copy { name } => task("copy", name),
        Event::ProcessPlist { name } => task("plist", name),
        Event::Diagnostic {
            kind,
            location,
            message,
        } => json!({
            "event": "diagnostic",
            "severity": match kind {
                DiagKind::Error => "error",
                DiagKind::Warning => "warning",
                DiagKind::Note => "note",
            },
            "location": location,
            "message": message,
        }),
        Event::TestPassed { name, duration } => {
            json!({ "event": "test", "status": "passed", "name": name, "duration": duration })
        }
        Event::TestFailed { name } => json!({ "event": "test", "status": "failed", "name": name }),
        Event::SuiteStarted { name } => json!({ "event": "suite", "name": name }),
        Event::Result(_) | Event::Other(_) => return None,
    })
}

/// Error/warning counts, elapsed time, and the diagnostic events accumulated
/// by [`run_ndjson`] — counts fold into the terminal result payload, the
/// diagnostics into the last-build artifact.
#[derive(Debug, Default)]
pub struct StreamStats {
    pub errors: u32,
    pub warnings: u32,
    pub duration_ms: u64,
    pub diagnostics: Vec<serde_json::Value>,
}

/// Run a command emitting each parsed event as an NDJSON line on stdout — the
/// `-o ndjson` path for builds/tests. The caller folds the returned
/// [`StreamStats`] into its terminal `{"event":"result"}` payload, so the
/// stream ends with exactly one summary line.
pub fn run_ndjson(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    out: &Output,
) -> Result<(bool, StreamStats), CliError> {
    let start = Instant::now();
    let mut stats = StreamStats::default();
    let ok = process::stream_lines(program, args, cwd, |line| {
        let event = parse_line(line);
        if let Some(json) = event_json(&event) {
            if let Event::Diagnostic { kind, .. } = &event {
                match kind {
                    DiagKind::Error => stats.errors += 1,
                    DiagKind::Warning => stats.warnings += 1,
                    DiagKind::Note => {}
                }
                stats.diagnostics.push(json.clone());
            }
            out.ndjson_event(&json);
        }
    })?;
    stats.duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok((ok, stats))
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
    fn gh_annotations_carry_location_and_escapes() {
        let with_loc = parse_line("/src/Foo.swift:12:5: error: bad % thing");
        assert_eq!(
            gh_annotation(&with_loc).unwrap(),
            "::error file=/src/Foo.swift,line=12,col=5::bad %25 thing"
        );
        let bare = parse_line("error: no signing certificate");
        assert_eq!(
            gh_annotation(&bare).unwrap(),
            "::error::no signing certificate"
        );
        let warn = parse_line("/a.swift:1:1: warning: unused");
        assert!(gh_annotation(&warn).unwrap().starts_with("::warning file="));
        // Tasks and notes carry no annotation.
        assert!(gh_annotation(&parse_line("CompileSwift x.swift")).is_none());
    }

    #[test]
    fn tool_prefixed_errors_fold_the_tool_into_the_message() {
        // `clang:`/`xcodebuild:` prefixes are tools, not files — a location of
        // "clang" would anchor annotations and artifacts to a bogus file.
        let e = parse_line("clang: error: linker command failed with exit code 1");
        assert_eq!(
            e,
            Event::Diagnostic {
                kind: DiagKind::Error,
                location: None,
                message: "clang: linker command failed with exit code 1".to_string(),
            }
        );
        assert_eq!(
            gh_annotation(&e).unwrap(),
            "::error::clang: linker command failed with exit code 1"
        );
        assert!(matches!(
            parse_line("xcodebuild: error: Scheme Demo is not currently configured for the test action."),
            Event::Diagnostic { location: None, .. }
        ));
        // Real locations keep anchoring, path-shaped or bare-file-with-extension.
        assert!(matches!(
            parse_line("Foo.swift:3:1: warning: unused"),
            Event::Diagnostic { location: Some(_), .. }
        ));
    }

    #[test]
    fn annotation_property_values_are_escaped() {
        // GitHub splits properties on `,` and `:` — a path containing them
        // must ride escaped or the annotation anchors to the wrong file.
        let e = parse_line("/Users/x/My Project,v2/Foo.swift:12:5: error: bad thing");
        assert_eq!(
            gh_annotation(&e).unwrap(),
            "::error file=/Users/x/My Project%2Cv2/Foo.swift,line=12,col=5::bad thing"
        );
    }

    #[test]
    fn ndjson_events_serialize_tasks_diagnostics_and_tests() {
        let compile = event_json(&parse_line("CompileSwift normal arm64 /src/Foo.swift")).unwrap();
        assert_eq!(compile["event"], "task");
        assert_eq!(compile["kind"], "compile");

        let diag = event_json(&parse_line("/src/Foo.swift:3:1: warning: unused")).unwrap();
        assert_eq!(diag["event"], "diagnostic");
        assert_eq!(diag["severity"], "warning");
        assert_eq!(diag["location"], "/src/Foo.swift:3:1");

        let test =
            event_json(&parse_line("Test Case 'LoginTests.testFoo' passed (0.001 seconds)"))
                .unwrap();
        assert_eq!(test["event"], "test");
        assert_eq!(test["status"], "passed");

        // Banners and unrecognized lines emit nothing (the terminal result
        // event is the caller's).
        assert!(event_json(&parse_line("** BUILD SUCCEEDED **")).is_none());
        assert!(event_json(&parse_line("random noise")).is_none());
    }

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
        assert!(render(&copy, false, false, false).is_none());
        assert!(render(&copy, false, true, false).is_some());

        // Errors always show.
        let err = Event::Diagnostic {
            kind: DiagKind::Error,
            location: None,
            message: "boom".into(),
        };
        assert_eq!(render(&err, false, false, false), Some("error: boom".to_string()));
    }

    #[test]
    fn render_colorizes_when_enabled() {
        let ok = Event::Result(ResultKind::BuildSucceeded);
        let plain = render(&ok, false, false, false).unwrap();
        let colored = render(&ok, true, false, false).unwrap();
        assert!(!plain.contains('\x1b'));
        assert!(colored.contains('\x1b'));
    }

    #[test]
    fn stamp_time_marks_success_banners_only() {
        let ok = Event::Result(ResultKind::BuildSucceeded);
        let banner = render(&ok, false, false, false).unwrap();
        assert_eq!(
            stamp_time(banner, &ok, Duration::from_millis(5340), false),
            "✓ Build succeeded (5.3s)"
        );

        // Failures and ordinary lines are left exactly as rendered.
        let failed = Event::Result(ResultKind::BuildFailed);
        let failed_banner = render(&failed, false, false, false).unwrap();
        assert_eq!(
            stamp_time(
                failed_banner.clone(),
                &failed,
                Duration::from_secs(9),
                false
            ),
            failed_banner
        );
        let compile = Event::Compile {
            name: "ContentView.swift".to_string(),
        };
        let line = render(&compile, false, false, false).unwrap();
        assert_eq!(
            stamp_time(line.clone(), &compile, Duration::from_secs(9), false),
            line
        );
    }
}
