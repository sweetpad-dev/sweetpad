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

impl DiagKind {
    /// The kind a recorded diagnostic's `severity` names.
    #[must_use]
    pub fn from_severity(severity: &str) -> Self {
        match severity {
            "error" => Self::Error,
            "warning" => Self::Warning,
            _ => Self::Note,
        }
    }
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

/// Terminal `** … **` banners. `build-for-testing` closes on `** TEST BUILD
/// … **`, which is a build's outcome, not a test run's.
fn parse_banner(t: &str) -> Option<Event> {
    let kind = if t.contains("** BUILD SUCCEEDED **") || t.contains("** TEST BUILD SUCCEEDED **") {
        ResultKind::BuildSucceeded
    } else if t.contains("** BUILD FAILED **") || t.contains("** TEST BUILD FAILED **") {
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
    // Task headers start at column 0. Indented occurrences of the same verbs
    // are not fresh tasks — notably the tab-indented "The following build
    // commands failed:" summary after a failure, which would otherwise render
    // as spurious `Compiling …` lines *after* the failure banner.
    if line.starts_with(char::is_whitespace) {
        return Event::Other(line.to_string());
    }
    let stripped = strip_target_annotation(t);
    let verb = t.split_whitespace().next().unwrap_or("");
    match verb {
        "CompileSwift" | "SwiftCompile" | "CompileC" | "CompileXIB" | "CompileStoryboard" => {
            match swift_batch_len(t) {
                // A one-file batch header names the same file as the per-file
                // line right behind it; let that one carry the name so the file
                // is announced once.
                Some(1) => Event::Other(line.to_string()),
                // A wider batch is a group, and naming one arbitrary member of
                // it reads as a lone file. Count it instead.
                Some(n) => Event::Compile {
                    name: format!("{n} files"),
                },
                None => match source_name(t) {
                    Some(name) => Event::Compile { name },
                    // Xcode 27 opens each target's Swift work with a bare
                    // `SwiftCompile normal arm64 (in target …)` that names no
                    // file; the per-file lines behind it announce the work.
                    None if matches!(verb, "CompileSwift" | "SwiftCompile") => {
                        Event::Other(line.to_string())
                    }
                    None => Event::Compile {
                        name: "source".to_string(),
                    },
                },
            }
        }
        "CompileSwiftSources" => Event::Compile {
            name: "Swift sources".to_string(),
        },
        "CompileAssetCatalog" => Event::Copy {
            name: "asset catalog".to_string(),
        },
        "Ld" => Event::Link {
            target: ld_target(stripped).unwrap_or_else(|| "binary".to_string()),
        },
        "CodeSign" => Event::CodeSign {
            name: verb_argument(stripped).map_or_else(|| "bundle".to_string(), base),
        },
        "CpResource" | "PBXCp" | "Copy" | "CpHeader" | "Ditto" | "CopySwiftLibs" => Event::Copy {
            name: last_token(stripped).map_or_else(|| "files".to_string(), |s| base(&s)),
        },
        "ProcessInfoPlistFile" => Event::ProcessPlist {
            name: stripped
                .split_whitespace()
                .find(|tok| Path::new(tok).extension().is_some_and(|e| e == "plist"))
                .map_or_else(|| "Info.plist".to_string(), base),
        },
        _ => Event::Other(line.to_string()),
    }
}

/// Strip xcodebuild's trailing `(in target 'X' from project 'Y')` (or the
/// older `(in target 'X')`) annotation from a task line — its tokens would
/// otherwise be mistaken for the task's file arguments.
fn strip_target_annotation(t: &str) -> &str {
    t.rfind(" (in target '").map_or(t, |i| t[..i].trim_end())
}

/// How many sources a `SwiftCompile normal <arch> Compiling\ a.swift,\ b.swift
/// <paths…>` batch header covers, or `None` for the per-file form that carries a
/// path in that slot. xcodebuild emits both shapes for the same work — one
/// header per batch, then one line per file it compiled — which is why an
/// unfiltered log announces most files twice.
///
/// Entries are counted by their separators rather than by token, so a filename
/// containing an escaped space stays one entry.
fn swift_batch_len(t: &str) -> Option<usize> {
    let mut toks = t.split_whitespace().skip(3);
    if !toks.next()?.starts_with("Compiling\\") {
        return None;
    }
    let separators = toks
        .take_while(|tok| !tok.starts_with('/'))
        .filter(|tok| tok.ends_with(",\\"))
        .count();
    Some(separators + 1)
}

/// The linked binary's path from an annotation-stripped `Ld` line: everything
/// between the verb and the trailing `normal [<arch>]` tokens, so a product
/// path containing spaces survives intact.
fn ld_target(stripped: &str) -> Option<String> {
    const ARCHS: [&str; 6] = ["arm64", "arm64e", "armv7", "armv7s", "x86_64", "i386"];
    let mut rest = verb_argument(stripped)?;
    while let Some((head, tail)) = rest.rsplit_once(char::is_whitespace) {
        if tail == "normal" || ARCHS.contains(&tail) {
            rest = head.trim_end();
        } else {
            break;
        }
    }
    (!rest.is_empty()).then(|| base(rest))
}

/// Everything after the leading verb of a task line — the argument as one
/// string, not whitespace-split (paths may contain spaces).
fn verb_argument(stripped: &str) -> Option<&str> {
    let rest = stripped.split_once(char::is_whitespace)?.1.trim();
    (!rest.is_empty()).then_some(rest)
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
        } => (verbose || *kind != DiagKind::Note)
            .then(|| diagnostic_line(kind, location.as_deref(), message, color)),
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
    /// The last line rendered was an error announcing a list of details, so
    /// the indented lines that follow belong to it (see [`opens_a_list`]).
    continues: bool,
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
                out.use_color_stderr(),
            )),
            start: Instant::now(),
            color: out.use_color(),
            verbose: out.is_verbose(),
            quiet: out.is_quiet(),
            gh_annotations: out.gh_annotations(),
            continues: false,
        }
    }

    /// Render one raw `xcodebuild` line for display, or `None` to suppress it.
    /// The first line that renders stops and erases the spinner; the closing
    /// success banner is stamped with the elapsed build time. With
    /// `--gh-annotations`, a diagnostic also carries its `::error`/`::warning`
    /// workflow-command line.
    pub fn line(&mut self, raw: &str) -> Option<String> {
        self.parsed(&Parsed {
            raw: raw.to_string(),
            event: parse_line(raw),
        })
    }

    /// [`line`](Self::line) for a line [`LogParser`] has already parsed.
    pub fn parsed(&mut self, parsed: &Parsed) -> Option<String> {
        let Parsed { raw, event } = parsed;
        if self.continues {
            if raw.starts_with(char::is_whitespace) && !raw.trim().is_empty() {
                return Some(Colors::new(self.color).red(&format!("  {}", raw.trim())));
            }
            self.continues = false;
        }
        let mut rendered = render(event, self.color, self.verbose, self.quiet)?;
        self.continues = opens_a_list(event);
        // First line through — hand the terminal over from the spinner to the
        // streamed output (dropping the spinner erases its line).
        self.spinner = None;
        if self.gh_annotations
            && let Some(annotation) = gh_annotation(event)
        {
            rendered.push('\n');
            rendered.push_str(&annotation);
        }
        Some(stamp_time(
            rendered,
            event,
            self.start.elapsed(),
            self.color,
        ))
    }
}

/// Whether an error announces details on the lines after it. `xcodebuild:
/// error: Could not resolve package dependencies:` puts the actual reason on
/// the indented lines that follow, which parse as unrecognized output and would
/// otherwise be hidden. Only a location-less error ending in a colon counts, so
/// the indented source excerpt under a compiler error is left alone.
fn opens_a_list(event: &Event) -> bool {
    matches!(
        event,
        Event::Diagnostic {
            kind: DiagKind::Error,
            location: None,
            message,
        } if message.ends_with(':')
    )
}

/// One line of output and the event it parsed to.
#[derive(Debug)]
pub struct Parsed {
    pub raw: String,
    pub event: Event,
}

/// xcodebuild's output as events, one per line, except where an error owns the
/// lines printed under it.
///
/// A destination xcodebuild cannot use fails with one line (`Timed out waiting
/// for all destinations matching the provided destination specifier to become
/// available`, `Unable to find a device matching the provided destination
/// specifier:`), and the reason comes after a blank line, in xcodebuild's own
/// listing of the destinations it considered. A locked phone or one without
/// Developer Mode is named only there. So the error is held until the listing
/// ends and carries the part of it that explains the failure
/// ([`DestinationListing`]), and every consumer (human, `-o json`,
/// `-o ndjson`) gets the same diagnostic.
#[derive(Debug, Default)]
pub struct LogParser {
    held: Option<(String, DestinationListing)>,
}

impl LogParser {
    /// Parse one raw line and return what it completes: usually just that
    /// line; nothing while a destination error is still reading its listing;
    /// and the error followed by this line once a line at column 0 ends the
    /// listing.
    pub fn push(&mut self, line: &str) -> Vec<Parsed> {
        let mut done = Vec::new();
        if let Some((_, listing)) = &mut self.held {
            if line.trim().is_empty() || line.starts_with(char::is_whitespace) {
                listing.line(line.trim());
                return done;
            }
            done.extend(self.finish());
        }
        let event = parse_line(line);
        match &event {
            Event::Diagnostic {
                kind: DiagKind::Error,
                location: None,
                message,
            } if message.contains("provided destination specifier") => {
                self.held = Some((line.to_string(), DestinationListing::new(message)));
            }
            _ => done.push(Parsed {
                raw: line.to_string(),
                event,
            }),
        }
        done
    }

    /// The destination error still waiting when the output ends — xcodebuild
    /// exits right after printing the listing, so this is where it usually
    /// comes out.
    pub fn finish(&mut self) -> Option<Parsed> {
        let (raw, listing) = self.held.take()?;
        Some(Parsed {
            raw,
            event: Event::Diagnostic {
                kind: DiagKind::Error,
                location: None,
                message: listing.message(),
            },
        })
    }
}

/// How many listed destinations a destination error carries at most.
const MAX_LISTED: usize = 8;

/// The listing xcodebuild prints under a destination error, cut down to the
/// part that explains it.
///
/// xcodebuild lists every destination the scheme can use and every one it
/// cannot, which on a machine with the usual simulators is dozens of lines,
/// nearly all beside the point. Kept: the specifier xcodebuild echoes as the
/// requested destination and the entries for it, every usable destination
/// with an `error:` (the locked phone, Developer Mode off), and the prose
/// between sections. The rest are counted. Only the "Unable to find" error
/// echoes a specifier; under a timeout, the `error:` entries are what explain
/// it.
#[derive(Debug)]
struct DestinationListing {
    lines: Vec<String>,
    requested: Option<Vec<(String, String)>>,
    section: Option<Section>,
    kept: usize,
    omitted: usize,
}

/// One `Destinations compatible with the "App" scheme:` block of the listing.
#[derive(Debug)]
struct Section {
    header: String,
    /// The "compatible" (older Xcode: "available") side, as opposed to
    /// "incompatible" / "ineligible", whose every entry has an `error:` saying
    /// its platform does not match.
    usable: bool,
    /// The header is written once, ahead of the first entry kept under it.
    written: bool,
}

impl DestinationListing {
    fn new(message: &str) -> Self {
        Self {
            lines: vec![message.to_string()],
            requested: None,
            section: None,
            kept: 0,
            omitted: 0,
        }
    }

    /// Take one trimmed line of the listing.
    fn line(&mut self, t: &str) {
        if t.is_empty() {
            return;
        }
        if t.starts_with('{') && t.ends_with('}') {
            let fields = listing_fields(t);
            let Some(section) = &mut self.section else {
                // An entry ahead of any section is the specifier echoed back.
                self.lines.push(format!("  {t}"));
                self.requested.get_or_insert(fields);
                return;
            };
            let requested = self
                .requested
                .as_deref()
                .is_some_and(|r| is_requested(r, &fields));
            let explains = section.usable && field(&fields, "error").is_some();
            if (requested || explains) && self.kept < MAX_LISTED {
                if !section.written {
                    self.lines.push(format!("  {}", section.header));
                    section.written = true;
                }
                self.lines.push(format!("    {t}"));
                self.kept += 1;
            } else {
                self.omitted += 1;
            }
        } else if t.ends_with(':') && t.to_ascii_lowercase().contains("destinations") {
            let lower = t.to_ascii_lowercase();
            let usable = !(lower.contains("incompatible") || lower.contains("ineligible"));
            self.section = Some(Section {
                header: t.to_string(),
                usable,
                written: false,
            });
        } else {
            self.lines.push(format!("  {t}"));
        }
    }

    /// The error's message with the kept lines under it.
    fn message(&self) -> String {
        let mut lines = self.lines.clone();
        match self.omitted {
            0 => {}
            1 => lines.push("  (1 other destination omitted)".to_string()),
            n => lines.push(format!("  ({n} other destinations omitted)")),
        }
        lines.join("\n")
    }
}

/// The `key:value` fields of one `{ platform:iOS, id:…, name:…, error:… }`
/// listing entry. `error:` comes last and is free text that can hold ", " of
/// its own, so it is split off whole first; a piece of the rest without a
/// `key:` of its own belongs to the value before it (a name with a comma).
fn listing_fields(entry: &str) -> Vec<(String, String)> {
    let inner = entry.trim_start_matches('{').trim_end_matches('}').trim();
    let (head, error) = match inner.strip_prefix("error:") {
        Some(error) => ("", Some(error)),
        None => inner
            .split_once(", error:")
            .map_or((inner, None), |(head, error)| (head, Some(error))),
    };
    let mut fields: Vec<(String, String)> = Vec::new();
    for piece in head.split(", ").filter(|p| !p.is_empty()) {
        match piece.split_once(':') {
            Some((key, value)) if !key.is_empty() && key.chars().all(char::is_alphanumeric) => {
                fields.push((key.to_string(), value.to_string()));
            }
            _ => {
                if let Some((_, value)) = fields.last_mut() {
                    value.push_str(", ");
                    value.push_str(piece);
                }
            }
        }
    }
    if let Some(error) = error {
        fields.push(("error".to_string(), error.trim().to_string()));
    }
    fields
}

/// Whether a listing entry is the destination xcodebuild echoed as requested:
/// the same id, else the same name (on the same platform, when one was
/// given), else, for a bare platform such as `generic/platform=iOS`, that
/// platform's placeholder ("Any iOS Device").
fn is_requested(requested: &[(String, String)], entry: &[(String, String)]) -> bool {
    if let Some(id) = field(requested, "id") {
        return field(entry, "id").is_some_and(|e| e.eq_ignore_ascii_case(id));
    }
    let platform = field(requested, "platform");
    let same_platform = platform.is_none() || field(entry, "platform") == platform;
    if let Some(name) = field(requested, "name") {
        return same_platform && field(entry, "name") == Some(name);
    }
    platform.is_some()
        && same_platform
        && field(entry, "id").is_some_and(|id| id.ends_with(":placeholder"))
}

fn field<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
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
    // Peel up to two trailing `:<number>` segments off the right; whatever
    // remains — colons and all — is the file. A column-less `file:line`
    // (SwiftLint's file-level violations) must still anchor to its line
    // rather than degrade to a bare `file=path%3Aline` GitHub can't map.
    let mut file = loc;
    let mut nums: Vec<u32> = Vec::new();
    for _ in 0..2 {
        if let Some((rest, tail)) = file.rsplit_once(':')
            && let Ok(n) = tail.parse::<u32>()
            && !rest.is_empty()
        {
            nums.push(n);
            file = rest;
        } else {
            break;
        }
    }
    // Peeled right-to-left: one number is the line; two are col then line.
    let (line, col) = match nums.as_slice() {
        [line] => (Some(*line), None),
        [col, line] => (Some(*line), Some(*col)),
        _ => (None, None),
    };
    let mut props = format!(" file={}", escape_property(file));
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
) -> Result<(bool, Vec<serde_json::Value>, Option<String>), CliError> {
    let mut progress = BuildProgress::start(out, label);
    let mut diagnostics = Vec::new();
    let mut watch = BlockerWatch::default();
    let mut parser = LogParser::default();
    let mut show = |parsed: &Parsed| {
        if let Event::Diagnostic { .. } = parsed.event
            && let Some(json) = event_json(&parsed.event)
        {
            diagnostics.push(json);
        }
        if let Some(rendered) = progress.parsed(parsed) {
            out.line(&rendered);
        }
    };
    let ok = process::stream_lines(program, args, cwd, |line| {
        watch.line(line);
        parser.push(line).iter().for_each(&mut show);
    })?;
    parser.finish().iter().for_each(&mut show);
    Ok((ok, diagnostics, watch.hint()))
}

/// Watches a build's output for a failure that no diagnostic describes and no
/// code change fixes — the build is blocked on a policy decision instead.
///
/// An SPM build-tool plugin (SwiftLint, SwiftGen, SwiftFormat) has to be
/// approved before it runs. Xcode asks with a trust prompt; from the CLI the
/// build simply fails, and nothing in the output says which flag unblocks it.
#[derive(Debug, Default)]
pub struct BlockerWatch {
    in_failed_list: bool,
    plugin: Option<String>,
}

impl BlockerWatch {
    pub fn line(&mut self, line: &str) {
        // Older Xcode says it outright, anywhere in the output.
        if line.contains("must be enabled before it can be used") {
            self.plugin = Some(quoted_name(line).unwrap_or_else(|| "a".to_string()));
            return;
        }
        if line.starts_with("The following build commands failed:") {
            self.in_failed_list = true;
            return;
        }
        // Newer Xcode only reports the validation step by name, and reports it
        // on success too — so it counts as the cause only where the failed
        // commands are listed.
        if self.in_failed_list && line.contains("Validate plug-in") {
            self.plugin = Some(quoted_name(line).unwrap_or_else(|| "a".to_string()));
        }
    }

    /// How to get past it, naming the flag — which is the whole point, since
    /// the flag is not discoverable from the failure.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        self.plugin.as_ref().map(|plugin| {
            format!(
                "{plugin} build-tool plugin must be approved before it can run, and Xcode's \
                 trust prompt has no CLI equivalent; retry with \
                 '-- -skipPackagePluginValidation'"
            )
        })
    }
}

/// The first quoted run in `line`, accepting the curly quotes Xcode prints as
/// well as plain ones.
fn quoted_name(line: &str) -> Option<String> {
    let is_quote = |c: char| c == '"' || c == '\u{201c}' || c == '\u{201d}';
    let rest = line.split_once(is_quote)?.1;
    let name = rest.split(is_quote).next()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Scan a full captured transcript for a blocked build (the `--json` path,
/// and `test`'s failure report).
#[must_use]
pub fn blocker_from_transcript(text: &str) -> Option<String> {
    let mut watch = BlockerWatch::default();
    for line in text.lines() {
        watch.line(line);
    }
    watch.hint()
}

/// Diagnostics parsed out of a full captured transcript (the `--json` path).
#[must_use]
pub fn diagnostics_from_transcript(text: &str) -> Vec<serde_json::Value> {
    let mut parser = LogParser::default();
    let mut parsed: Vec<Parsed> = text.lines().flat_map(|line| parser.push(line)).collect();
    parsed.extend(parser.finish());
    parsed
        .iter()
        .filter(|p| matches!(p.event, Event::Diagnostic { .. }))
        .filter_map(|p| event_json(&p.event))
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

/// A finished build's error/warning counts and elapsed time, for the terminal
/// result payload. Tallied from the parsed diagnostics rather than by any one
/// runner, so every mode that parsed them reports the same numbers.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BuildStats {
    pub errors: usize,
    pub warnings: usize,
    pub duration_ms: u64,
}

impl BuildStats {
    #[must_use]
    pub fn tally(diagnostics: &[serde_json::Value], elapsed: Duration) -> Self {
        let count = |severity: &str| {
            diagnostics
                .iter()
                .filter(|d| d["severity"] == severity)
                .count()
        };
        Self {
            errors: count("error"),
            warnings: count("warning"),
            duration_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        }
    }
}

/// Run a command emitting each parsed event as an NDJSON line on stdout — the
/// `-o ndjson` path for builds/tests. Returns whether it succeeded, the
/// diagnostic events, and the blocker hint, as [`run_collecting`] does; the
/// caller closes the stream with its terminal `{"event":"result"}` line, so the
/// stream ends with exactly one summary line.
pub fn run_ndjson(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    out: &Output,
) -> Result<(bool, Vec<serde_json::Value>, Option<String>), CliError> {
    let mut diagnostics = Vec::new();
    let mut watch = BlockerWatch::default();
    let mut parser = LogParser::default();
    let mut emit = |parsed: &Parsed| {
        if let Some(json) = event_json(&parsed.event) {
            if matches!(parsed.event, Event::Diagnostic { .. }) {
                diagnostics.push(json.clone());
            }
            out.ndjson_event(&json);
        }
    };
    let ok = process::stream_lines(program, args, cwd, |line| {
        watch.line(line);
        parser.push(line).iter().for_each(&mut emit);
    })?;
    parser.finish().iter().for_each(&mut emit);
    Ok((ok, diagnostics, watch.hint()))
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

/// One diagnostic as the build log shows it, `error: <location>: <message>`:
/// red for an error, yellow for a warning, dim for a note.
#[must_use]
pub fn diagnostic_line(
    kind: &DiagKind,
    location: Option<&str>,
    message: &str,
    color: bool,
) -> String {
    let c = Colors::new(color);
    let loc = location.map(|l| format!("{l}: ")).unwrap_or_default();
    match kind {
        DiagKind::Error => c.red(&format!("error: {loc}{message}")),
        DiagKind::Warning => c.yellow(&format!("warning: {loc}{message}")),
        DiagKind::Note => c.dim(&format!("note: {loc}{message}")),
    }
}

/// The word a build's outcome is reported by, colored the way the build log's
/// closing line is: green for a success, bold red for a failure.
#[must_use]
pub fn outcome_word(ok: bool, word: &str, color: bool) -> String {
    let c = Colors::new(color);
    if ok { c.green(word) } else { c.red_bold(word) }
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

    #[test]
    fn a_recorded_diagnostic_is_colored_like_the_live_one() {
        let line = |severity: &str, color| {
            diagnostic_line(
                &DiagKind::from_severity(severity),
                Some("A.swift:3:1"),
                "boom",
                color,
            )
        };
        assert_eq!(
            line("error", true),
            "\x1b[31merror: A.swift:3:1: boom\x1b[0m"
        );
        assert_eq!(
            line("warning", true),
            "\x1b[33mwarning: A.swift:3:1: boom\x1b[0m"
        );
        assert_eq!(
            line("remark", true),
            "\x1b[2mnote: A.swift:3:1: boom\x1b[0m"
        );
        assert_eq!(line("error", false), "error: A.swift:3:1: boom");
        assert_eq!(
            outcome_word(false, "FAILED", true),
            "\x1b[1;31mFAILED\x1b[0m"
        );
        assert_eq!(outcome_word(true, "succeeded", false), "succeeded");
    }

    /// The Xcode 26 shape: the validation step is named only in the list of
    /// failed build commands, with curly quotes.
    const BLOCKED_26: &str = "\
Prepare packages
Validate plug-in \u{201c}SwiftLintPlugin\u{201d} in package \u{201c}swiftlint\u{201d}
** BUILD FAILED **
The following build commands failed:
\tValidate plug-in \u{201c}SwiftLintPlugin\u{201d} in package \u{201c}swiftlint\u{201d}
\tBuilding workspace Ampol with scheme Ampol and configuration Debug
(3 failures)
";

    #[test]
    fn a_plugin_that_needs_approval_names_the_flag_that_unblocks_it() {
        let hint = blocker_from_transcript(BLOCKED_26).expect("no blocker found");
        assert!(hint.contains("SwiftLintPlugin"), "{hint}");
        assert!(hint.contains("-skipPackagePluginValidation"), "{hint}");
        // Terminals print backticks literally.
        assert!(!hint.contains('`'), "{hint}");

        // Older Xcode says it outright, without a failed-command list.
        let older = "error: Plugin \"SwiftLintPlugin\" from package \"SwiftLint\" must be \
                     enabled before it can be used\n";
        let hint = blocker_from_transcript(older).expect("no blocker found");
        assert!(hint.contains("SwiftLintPlugin"), "{hint}");
        assert!(hint.contains("-skipPackagePluginValidation"), "{hint}");
    }

    #[test]
    fn a_validation_step_that_succeeded_is_not_a_blocker() {
        // The same line appears on a healthy build once the plugin is trusted.
        // Firing there would tell every passing build to pass a flag it does
        // not need, so only the failed-command list counts.
        let succeeded = "\
Prepare packages
Validate plug-in \u{201c}SwiftLintPlugin\u{201d} in package \u{201c}swiftlint\u{201d}
CompileSwiftSources normal arm64
** BUILD SUCCEEDED **
";
        assert!(blocker_from_transcript(succeeded).is_none());

        // A failure with some other cause is left to the diagnostics.
        let other = "\
** BUILD FAILED **
The following build commands failed:
\tCompileSwift normal arm64 /work/App/Main.swift
(1 failure)
";
        assert!(blocker_from_transcript(other).is_none());
    }
    use super::*;

    fn plain_progress() -> BuildProgress {
        BuildProgress {
            spinner: None,
            start: Instant::now(),
            color: false,
            verbose: false,
            quiet: false,
            gh_annotations: false,
            continues: false,
        }
    }

    /// `xcodebuild -resolvePackageDependencies` prints why it failed on the
    /// indented lines under its header. Those lines are shown with it, and the
    /// first line that is not indented ends them.
    #[test]
    fn the_reason_under_an_xcodebuild_error_is_shown_with_it() {
        let mut progress = plain_progress();
        let shown: Vec<String> = [
            "Resolve Package Graph",
            "xcodebuild: error: Could not resolve package dependencies:",
            "  Disabled default traits on package 'swift-collections' that declares no traits.",
            "  fatalError",
            "Writing error result bundle",
            "  still hidden",
        ]
        .iter()
        .filter_map(|line| progress.line(line))
        .collect();
        assert_eq!(
            shown,
            [
                "error: xcodebuild: Could not resolve package dependencies:",
                "  Disabled default traits on package 'swift-collections' that declares no traits.",
                "  fatalError",
            ]
        );
    }

    /// The indented source excerpt under a compiler error stays hidden: the
    /// error has a location and does not announce a list.
    #[test]
    fn a_compiler_errors_source_excerpt_is_not_a_continuation() {
        let mut progress = plain_progress();
        let shown: Vec<String> = [
            "/src/Foo.swift:12:5: error: cannot find 'x' in scope",
            "    let y = x",
            "            ^",
        ]
        .iter()
        .filter_map(|line| progress.line(line))
        .collect();
        assert_eq!(
            shown,
            ["error: /src/Foo.swift:12:5: cannot find 'x' in scope"]
        );
    }

    /// Xcode 27 building for a paired iPhone that is locked: the timeout line,
    /// two blank lines, then the listing with the reason. Captured from
    /// `sweetpad build --on <udid> -v` on the CI fixture app.
    const DESTINATION_TIMEOUT_27: &str = "\
Writing result bundle at path:
\t/Users/me/.local/state/sweetpad/results/SweetpadCIApp-07bf27757d6d516c-build.xcresult

xcodebuild: error: Timed out waiting for all destinations matching the provided destination specifier to become available


\tDestinations compatible with the \"SweetpadCIApp\" scheme:
\t\t{ platform:iOS, arch:arm64, id:00008110-000559182E90401E, name:Iphone 13, error:Iphone 13 needs to be unlocked to enable development services Please unlock the device. }
";

    /// Xcode 27 with a destination id that matches nothing, from the same
    /// fixture with `--destination id=00000000-0000000000000000`. Captured
    /// verbatim except that the listing is cut to a few entries per section
    /// (the machine listed 16 compatible and 10 incompatible).
    const DESTINATION_NOT_FOUND_27: &str = "\
xcodebuild: error: Unable to find a device matching the provided destination specifier:
\t\t{ id:00000000-0000000000000000 }

\tThe requested device could not be found because no available devices matched the request.

\tDestinations compatible with the \"SweetpadCIApp\" scheme:
\t\t{ platform:macOS, arch:arm64, variant:Designed for [iPad,iPhone], id:00006030-0018296E1A28001C, name:My Mac }
\t\t{ platform:iOS, arch:arm64, id:00008110-000559182E90401E, name:Iphone 13 }
\t\t{ platform:iOS, id:dvtdevice-DVTiPhonePlaceholder-iphoneos:placeholder, name:Any iOS Device }
\t\t{ platform:iOS Simulator, arch:arm64, id:F13C004A-0824-4870-B4F2-29AAEE36636E, OS:27.0, name:iPhone 17 }

\tDestinations incompatible with the \"SweetpadCIApp\" scheme:
\t\t{ platform:macOS, arch:arm64e, id:00006030-0018296E1A28001C, name:My Mac, error:My Mac\u{2019}s macOS platform doesn\u{2019}t match SweetpadCIApp.app\u{2019}s supported platforms. You can change SweetpadCIApp.app\u{2019}s Base SDK or Supported Platforms to support My Mac. }
\t\t{ platform:tvOS Simulator, arch:arm64, id:2CD2A3F5-8763-46B5-B7FC-F04117966B44, OS:27.0, name:Apple TV 4K (3rd generation), error:Apple TV 4K (3rd generation)\u{2019}s tvOS Simulator platform doesn\u{2019}t match SweetpadCIApp.app\u{2019}s supported platforms. You can change SweetpadCIApp.app\u{2019}s Base SDK or Supported Platforms to support Apple TV 4K (3rd generation). }
";

    fn only_message(transcript: &str) -> String {
        let diagnostics = diagnostics_from_transcript(transcript);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0]["severity"], "error");
        assert!(diagnostics[0]["location"].is_null());
        diagnostics[0]["message"].as_str().unwrap().to_string()
    }

    /// The reason a device timed out is in the listing after the blank lines,
    /// so the timeout diagnostic carries it.
    #[test]
    fn a_destination_timeout_carries_the_reason_from_the_listing() {
        assert_eq!(
            only_message(DESTINATION_TIMEOUT_27),
            "xcodebuild: Timed out waiting for all destinations matching the provided \
             destination specifier to become available\n  \
             Destinations compatible with the \"SweetpadCIApp\" scheme:\n    \
             { platform:iOS, arch:arm64, id:00008110-000559182E90401E, name:Iphone 13, \
             error:Iphone 13 needs to be unlocked to enable development services Please \
             unlock the device. }"
        );
    }

    /// Nothing listed is the requested id, and no usable destination reports
    /// an error: the echo and the explanation stay, every entry is counted.
    /// The incompatible side's errors are only platform mismatches.
    #[test]
    fn a_destination_not_found_keeps_the_echo_and_counts_the_listing() {
        assert_eq!(
            only_message(DESTINATION_NOT_FOUND_27),
            "xcodebuild: Unable to find a device matching the provided destination specifier:\n  \
             { id:00000000-0000000000000000 }\n  \
             The requested device could not be found because no available devices matched \
             the request.\n  \
             (6 other destinations omitted)"
        );
    }

    /// The older headers ("Available" / "Ineligible destinations for …"), in
    /// the shape the feedback log quotes from a timed out wireless iPhone.
    /// The simulators around it are the noise the listing drops.
    #[test]
    fn the_older_listing_headers_are_read_the_same_way() {
        let transcript = "\
xcodebuild: error: Timed out waiting for all destinations matching the provided destination specifier to become available

\tAvailable destinations for the \"Reflow\" scheme:
\t\t{ platform:iOS, arch:arm64, id:00008110-000559182E90401E, name:Iphone 13, error:Browsing on the local area network for Iphone 13, which has previously reported preparation errors. The device must be opted into Developer Mode to connect wirelessly. }
\t\t{ platform:iOS Simulator, arch:arm64, id:F13C004A-0824-4870-B4F2-29AAEE36636E, OS:18.0, name:iPhone 16 }
\t\t{ platform:iOS Simulator, arch:arm64, id:C25725CE-4886-4E68-B032-55EB2918FF60, OS:18.0, name:iPhone 16 Pro }

\tIneligible destinations for the \"Reflow\" scheme:
\t\t{ platform:watchOS Simulator, id:ABD4EBF9-910F-4A29-89D7-9B40DB2D7D18, OS:11.0, name:Apple Watch Series 10, error:watchOS doesn't match Reflow's supported platforms. }
";
        let message = only_message(transcript);
        assert!(
            message.contains("\n  Available destinations for the \"Reflow\" scheme:\n    "),
            "{message}"
        );
        assert!(
            message.contains("must be opted into Developer Mode to connect wirelessly"),
            "{message}"
        );
        assert!(!message.contains("iPhone 16"), "{message}");
        assert!(!message.contains("Ineligible"), "{message}");
        assert!(
            message.ends_with("(3 other destinations omitted)"),
            "{message}"
        );
    }

    /// A requested destination that is listed as unusable is the cause, so it
    /// is kept even from the incompatible side; the others there are not.
    #[test]
    fn the_requested_destination_is_kept_from_either_side() {
        let transcript = "\
xcodebuild: error: Unable to find a destination matching the provided destination specifier:
\t\t{ generic:1, platform:iOS }

\tIneligible destinations for the \"App\" scheme:
\t\t{ platform:iOS, id:dvtdevice-DVTiPhonePlaceholder-iphoneos:placeholder, name:Any iOS Device, error:iOS 27.0 is not installed. Please download and install the platform from Xcode > Settings > Components. }
\t\t{ platform:tvOS, id:dvtdevice-DVTiOSDevicePlaceholder-appletvos:placeholder, name:Any tvOS Device, error:tvOS 27.0 is not installed. }
";
        let message = only_message(transcript);
        assert!(message.contains("name:Any iOS Device"), "{message}");
        assert!(!message.contains("Any tvOS Device"), "{message}");
        assert!(
            message.ends_with("(1 other destination omitted)"),
            "{message}"
        );

        let by_id = [("id".to_string(), "abc".to_string())];
        let by_name = [
            ("platform".to_string(), "iOS Simulator".to_string()),
            ("name".to_string(), "iPhone 17".to_string()),
        ];
        let entry = listing_fields(
            "{ platform:iOS Simulator, arch:arm64, id:ABC, OS:27.0, name:iPhone 17 }",
        );
        assert!(is_requested(&by_id, &entry));
        assert!(is_requested(&by_name, &entry));
        assert!(!is_requested(
            &[("name".to_string(), "iPhone 17 Pro".to_string())],
            &entry
        ));
    }

    /// `error:` is free text with commas of its own, and a device name can
    /// hold one too.
    #[test]
    fn listing_fields_keep_commas_inside_values() {
        let fields = listing_fields(
            "{ platform:iOS, id:X, name:Bob, Work, error:Browsing for Bob, which failed. }",
        );
        assert_eq!(field(&fields, "name"), Some("Bob, Work"));
        assert_eq!(
            field(&fields, "error"),
            Some("Browsing for Bob, which failed.")
        );
        assert_eq!(
            field(
                &listing_fields("{ platform:macOS, variant:Designed for [iPad,iPhone], id:Y }"),
                "variant"
            ),
            Some("Designed for [iPad,iPhone]")
        );
    }

    /// The live paths see the same diagnostic as the transcript: held while
    /// the listing prints, rendered once it ends (here by the end of output),
    /// with a line at column 0 ending the listing early.
    #[test]
    fn the_stream_holds_a_destination_error_until_its_listing_ends() {
        let mut parser = LogParser::default();
        let mut progress = plain_progress();
        let mut shown: Vec<String> = Vec::new();
        for line in DESTINATION_TIMEOUT_27.lines() {
            for parsed in parser.push(line) {
                shown.extend(progress.parsed(&parsed));
            }
        }
        assert!(shown.is_empty(), "{shown:?}");
        let last = parser.finish().expect("the error is held");
        let rendered = progress.parsed(&last).unwrap();
        assert!(
            rendered.starts_with("error: xcodebuild: Timed out"),
            "{rendered}"
        );
        assert!(rendered.contains("needs to be unlocked"), "{rendered}");

        let mut parser = LogParser::default();
        let mut events: Vec<Event> = Vec::new();
        for line in [
            "xcodebuild: error: Unable to find a device matching the provided destination specifier:",
            "\t\t{ id:00000000-0000000000000000 }",
            "",
            "** BUILD FAILED **",
        ] {
            events.extend(parser.push(line).into_iter().map(|p| p.event));
        }
        assert!(parser.finish().is_none());
        assert!(matches!(&events[0], Event::Diagnostic { message, .. }
            if message.ends_with("{ id:00000000-0000000000000000 }")));
        assert_eq!(events[1], Event::Result(ResultKind::BuildFailed));
    }

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
    fn column_less_locations_still_anchor_line() {
        // SwiftLint-style file-level diagnostics have no column; `file:line`
        // used to degrade to a bogus `file=path%3Aline` GitHub can't map.
        assert_eq!(
            location_props("/x/File.swift:10"),
            " file=/x/File.swift,line=10"
        );
        assert_eq!(
            location_props("/x/File.swift:10:3"),
            " file=/x/File.swift,line=10,col=3"
        );
        assert_eq!(location_props("/x/File.swift"), " file=/x/File.swift");
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
            parse_line(
                "xcodebuild: error: Scheme Demo is not currently configured for the test action."
            ),
            Event::Diagnostic { location: None, .. }
        ));
        // Real locations keep anchoring, path-shaped or bare-file-with-extension.
        assert!(matches!(
            parse_line("Foo.swift:3:1: warning: unused"),
            Event::Diagnostic {
                location: Some(_),
                ..
            }
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

        let test = event_json(&parse_line(
            "Test Case 'LoginTests.testFoo' passed (0.001 seconds)",
        ))
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
    fn one_file_batch_headers_defer_to_the_per_file_line() {
        // xcodebuild emits the header and then the per-file line below; only the
        // latter should announce the file.
        let header = "SwiftCompile normal arm64 Compiling\\ ContentView.swift \
                      /a/ContentView.swift (in target 'App' from project 'App')";
        assert!(matches!(parse_line(header), Event::Other(_)));
        assert_eq!(
            parse_line("SwiftCompile normal arm64 /a/ContentView.swift (in target 'App')"),
            Event::Compile {
                name: "ContentView.swift".to_string()
            }
        );
    }

    #[test]
    fn wider_batch_headers_report_their_count() {
        let header = "SwiftCompile normal arm64 Compiling\\ A.swift,\\ B.swift,\\ C.swift \
                      /a/A.swift /a/B.swift /a/C.swift (in target 'App' from project 'App')";
        assert_eq!(
            parse_line(header),
            Event::Compile {
                name: "3 files".to_string()
            }
        );
    }

    #[test]
    fn a_swift_compile_that_names_no_file_is_not_a_compile_line() {
        // Xcode 27 prints this once per target ahead of the per-file lines.
        let bare =
            "SwiftCompile normal arm64 (in target 'SweetpadCIApp' from project 'SweetpadCIApp')";
        assert_eq!(parse_line(bare), Event::Other(bare.to_string()));
        assert!(render(&parse_line(bare), false, false, false).is_none());
        assert!(event_json(&parse_line(bare)).is_none());
    }

    #[test]
    fn batch_entries_survive_escaped_spaces_in_a_filename() {
        // `My File.swift` arrives as two tokens; the separator count keeps it one entry.
        let header = "SwiftCompile normal arm64 Compiling\\ My\\ File.swift \
                      /a/My\\ File.swift (in target 'App')";
        assert!(matches!(parse_line(header), Event::Other(_)));
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
    fn task_names_exclude_the_target_annotation() {
        // The `(in target 'X' from project 'Y')` suffix is metadata, not a
        // file — `Copying 'App')` was the old rendering.
        assert_eq!(
            parse_line(
                "CpResource /d/App.app/Foo.png /src/Foo.png (in target 'App' from project 'App')"
            ),
            Event::Copy {
                name: "Foo.png".to_string()
            }
        );
        assert_eq!(
            parse_line(
                "ProcessInfoPlistFile /d/App.app/Info.plist /src/Info.plist (in target 'App' from project 'App')"
            ),
            Event::ProcessPlist {
                name: "Info.plist".to_string()
            }
        );
    }

    #[test]
    fn task_paths_with_spaces_survive() {
        assert_eq!(
            parse_line(
                "Ld /d/My App.app/My App normal arm64 (in target 'My App' from project 'My App')"
            ),
            Event::Link {
                target: "My App".to_string()
            }
        );
        assert_eq!(
            parse_line("CodeSign /d/My App.app (in target 'My App' from project 'My App')"),
            Event::CodeSign {
                name: "My App.app".to_string()
            }
        );
    }

    #[test]
    fn indented_failure_summary_lines_are_not_tasks() {
        // xcodebuild's post-failure "The following build commands failed:"
        // block repeats task lines tab-indented; they must not render as
        // fresh `Compiling …` tasks after the failure banner.
        assert_eq!(
            parse_line(
                "\tCompileSwift normal arm64 /a/File.swift (in target 'App' from project 'App')"
            ),
            Event::Other(
                "\tCompileSwift normal arm64 /a/File.swift (in target 'App' from project 'App')"
                    .to_string()
            )
        );
        // Unindented task lines still parse.
        assert!(matches!(
            parse_line("CompileSwift normal arm64 /a/File.swift"),
            Event::Compile { .. }
        ));
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
        // `build-for-testing` ran no test, so its banner is a build's: it
        // stamps the time on success and survives `-q` on failure.
        assert_eq!(
            parse_line("** TEST BUILD SUCCEEDED **"),
            Event::Result(ResultKind::BuildSucceeded)
        );
        assert_eq!(
            parse_line("** TEST BUILD FAILED **"),
            Event::Result(ResultKind::BuildFailed)
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
        assert_eq!(
            render(&err, false, false, false),
            Some("error: boom".to_string())
        );
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
