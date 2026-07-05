//! Output rendering: human/colored by default, machine-readable under `--json`.
//!
//! Color auto-disables when stdout is not a TTY, when `NO_COLOR` is set, or
//! with `--no-color`. Human messages and errors go to stderr; primary data
//! goes to stdout (so `--json` payloads pipe cleanly). This is intentionally
//! small for the scaffold — render helpers grow alongside the commands.

// `Output` IS the sanctioned stdout/stderr sink: every emitter here writes
// directly, so the crate-level print lint is allowed for this module.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::{IsTerminal, Write};

use crate::cli::progress::Spinner;
use crate::cli::{CliError, GlobalArgs};

/// Resolved output mode shared across commands.
#[allow(clippy::struct_excessive_bools)] // independent output toggles, not a state machine
pub struct Output {
    json: bool,
    ndjson: bool,
    non_interactive: bool,
    color: bool,
    color_stderr: bool,
    verbose: bool,
    quiet: bool,
    gh_annotations: bool,
}

impl Output {
    #[must_use]
    pub fn new(global: &GlobalArgs) -> Self {
        // `-o` wins over the `--json` alias; `-o quiet` folds into `--quiet`.
        let mode = global.output.unwrap_or(if global.json {
            crate::cli::OutputMode::Json
        } else {
            crate::cli::OutputMode::Human
        });
        let json = mode == crate::cli::OutputMode::Json;
        let ndjson = mode == crate::cli::OutputMode::Ndjson;
        let quiet = global.quiet || mode == crate::cli::OutputMode::Quiet;
        // `--no-color`/`NO_COLOR` always win; otherwise `CLICOLOR_FORCE`/
        // `FORCE_COLOR` force color even when piped, else default to a TTY
        // check. `NO_COLOR` follows its spec (present and non-empty); the
        // opt-in vars use truthy parsing so `FOO=0` doesn't mean "on".
        let no_color =
            global.no_color || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let force_color = env_truthy("CLICOLOR_FORCE") || env_truthy("FORCE_COLOR");
        // Color is decided per stream: `sweetpad build 2> err.log` must not
        // write ANSI escapes into the log, and `sweetpad build > out.txt`
        // must not strip color from the interactive stderr chatter.
        let color = !no_color && (force_color || std::io::stdout().is_terminal());
        let color_stderr = !no_color && (force_color || std::io::stderr().is_terminal());
        // CI is first-class non-interactive: prompts must strict-error there
        // even when the runner fakes a TTY.
        let non_interactive =
            global.non_interactive || env_truthy("SWEETPAD_NONINTERACTIVE") || env_truthy("CI");
        Self {
            json,
            ndjson,
            non_interactive,
            color,
            color_stderr,
            // `--quiet` wins over `--verbose`, so a script that always passes
            // `-v` can still be quieted.
            verbose: global.verbose && !quiet,
            quiet,
            gh_annotations: global.gh_annotations,
        }
    }

    /// True when `--gh-annotations` was passed — build/test diagnostics also
    /// emit GitHub Actions `::error`/`::warning` lines.
    #[must_use]
    pub fn gh_annotations(&self) -> bool {
        self.gh_annotations
    }

    #[must_use]
    pub fn is_json(&self) -> bool {
        self.json
    }

    /// True under `-o ndjson` — the long-running verbs stream one JSON event
    /// per line instead of human text.
    #[must_use]
    pub fn is_ndjson(&self) -> bool {
        self.ndjson
    }

    /// Emit one NDJSON event line to stdout (compact). No-op outside ndjson
    /// mode, so emitters can call it unconditionally.
    pub fn ndjson_event(&self, event: &serde_json::Value) {
        if self.ndjson && let Ok(s) = serde_json::to_string(event) {
            println!("{s}");
        }
    }

    #[must_use]
    pub fn use_color(&self) -> bool {
        self.color
    }

    /// Color for *stderr* writes (status chatter, spinners, prompts) — decided
    /// by stderr's own TTY-ness, independent of stdout's.
    #[must_use]
    pub fn use_color_stderr(&self) -> bool {
        self.color_stderr
    }

    /// True when the terminal is interactive — gates the picker fallback in
    /// [`crate::cli::resolve`], spinners, and `app run`'s rebuild session. False
    /// under `--json`/`-o ndjson`, `--non-interactive`/`SWEETPAD_NONINTERACTIVE`,
    /// or when stderr is not a TTY.
    #[must_use]
    pub fn is_interactive(&self) -> bool {
        !self.json && !self.ndjson && !self.non_interactive && std::io::stderr().is_terminal()
    }

    /// True when `-v`/`--verbose` was passed — surfaces raw/extra output.
    #[must_use]
    pub fn is_verbose(&self) -> bool {
        self.verbose
    }

    /// True when `-q`/`--quiet` was passed — mutes advisory chatter.
    #[must_use]
    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// Print a primary data line to stdout (human mode only — JSON commands
    /// build a payload and emit it via [`Output::json_value`], and ndjson
    /// streams own stdout for events).
    pub fn line(&self, s: &str) {
        if !self.json && !self.ndjson {
            println!("{s}");
        }
    }

    /// Emit a command's result as the standardized JSON success envelope
    /// (`{schema, ok: true, data}`) on stdout. No-op in human mode. This is the
    /// single success-envelope site — the dispatcher routes migrated commands'
    /// payloads here, and self-emitting commands call it directly, so every
    /// `--json` result is wrapped identically.
    pub fn json_value(&self, data: &serde_json::Value) {
        if self.json
            && let Ok(s) = serde_json::to_string_pretty(&serde_json::json!({
                "schema": 1,
                "ok": true,
                "data": data,
            }))
        {
            println!("{s}");
        }
    }

    /// Emit a self-rendered command's machine result: the success envelope
    /// under `--json`, the terminal `{"event":"result"}` line under
    /// `-o ndjson` — so a Streamed command still honors both stdout
    /// contracts. No-op in human mode.
    pub fn result_value(&self, data: &serde_json::Value) {
        if self.ndjson {
            self.ndjson_event(&serde_json::json!({
                "event": "result",
                "ok": true,
                "data": data,
            }));
        } else {
            self.json_value(data);
        }
    }

    /// Print a list item to stdout (human mode only), optionally marked as the
    /// currently selected entry (green `*` when color is on).
    pub fn item(&self, name: &str, selected: bool) {
        if self.json || self.ndjson {
            return;
        }
        if selected {
            let line = if self.color {
                format!("\x1b[32m* {name}\x1b[0m")
            } else {
                format!("* {name}")
            };
            println!("{line}");
        } else {
            println!("  {name}");
        }
    }

    /// An informational note to stderr (human mode only, muted by `--quiet`).
    pub fn note(&self, s: &str) {
        if !self.json && !self.ndjson && !self.quiet {
            let _ = writeln!(std::io::stderr(), "{}", self.dim(s));
        }
    }

    /// A `warning:`-prefixed line to stderr. Unlike [`note`](Output::note) it is
    /// *not* muted by `--quiet` or `--json` — warnings flag real problems (a
    /// corrupt state file, a config typo) that should surface even in scripts;
    /// stderr keeps them out of any JSON payload on stdout.
    pub fn warn(&self, s: &str) {
        let prefix = if self.color_stderr {
            "\x1b[33mwarning:\x1b[0m"
        } else {
            "warning:"
        };
        let _ = writeln!(std::io::stderr(), "{prefix} {s}");
    }

    /// A prominent (non-dimmed, yellow) status line to stderr — e.g. the running
    /// app exiting. Stands out from the dim session notes.
    pub fn alert(&self, s: &str) {
        if !self.json && !self.ndjson && !self.quiet {
            let line = if self.color_stderr {
                format!("\x1b[33m{s}\x1b[0m")
            } else {
                s.to_string()
            };
            let _ = writeln!(std::io::stderr(), "{line}");
        }
    }

    /// Run `f` while a transient `⠋ message` spinner animates on stderr, erased
    /// when `f` returns — so the caller's next [`note`](Output::note) replaces it
    /// in place. Animates only when interactive (TTY, not `--json`); otherwise
    /// `f` just runs. Use for long, otherwise-silent steps (boot, install).
    pub fn step<T>(&self, message: &str, f: impl FnOnce() -> T) -> T {
        let _spinner = Spinner::start(
            message,
            self.is_interactive() && !self.quiet,
            self.color_stderr,
        );
        f()
    }

    /// A verbose-only diagnostic to stderr, gated on `-v`.
    pub fn debug(&self, s: &str) {
        if self.verbose && !self.json && !self.ndjson {
            let _ = writeln!(std::io::stderr(), "{}", self.dim(&format!("[debug] {s}")));
        }
    }

    /// Render an error. JSON/ndjson modes emit a structured object (the
    /// flattened message) to stderr. Human mode prints a red `error:` prefix
    /// with the operation [`headline`](CliError::headline) in bold; any
    /// underlying [`detail`](CliError::detail) follows on the next line, dimmed
    /// and indented two spaces — so "what we were doing" reads at a glance and
    /// the raw tool output sits quietly beneath it.
    pub fn error(&self, err: &CliError) {
        if self.json || self.ndjson {
            let payload = serde_json::json!({
                "schema": 1,
                "ok": false,
                "error": { "code": err.error_kind().code_str(), "message": err.to_string() },
            });
            if let Ok(s) = serde_json::to_string(&payload) {
                let _ = writeln!(std::io::stderr(), "{s}");
            }
            return;
        }
        let prefix = if self.color_stderr {
            "\x1b[31merror:\x1b[0m"
        } else {
            "error:"
        };
        let stderr = std::io::stderr();
        match err.headline() {
            Some(headline) => {
                let _ = writeln!(&stderr, "{prefix} {}", self.bold(headline));
                let _ = writeln!(&stderr, "  {}", self.dim(err.detail()));
            }
            None => {
                let _ = writeln!(&stderr, "{prefix} {}", err.detail());
            }
        }
    }

    // `dim`/`bold` style stderr messages only (note/debug/error) — they follow
    // the stderr color decision.
    fn dim(&self, s: &str) -> String {
        if self.color_stderr {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    /// True when the value would enable an opt-in switch. Exposed for other
    /// `SWEETPAD_*` toggles so every env var parses truthiness the same way.
    #[must_use]
    pub fn truthy(value: &str) -> bool {
        !matches!(
            value.to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        )
    }

    fn bold(&self, s: &str) -> String {
        if self.color_stderr {
            format!("\x1b[1m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

/// Whether an opt-in env var is set to a truthy value — set and not
/// `0`/`false`/`no`/`off`/empty (case-insensitive), so `SWEETPAD_NONINTERACTIVE=0`
/// does not mean "on".
fn env_truthy(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| Output::truthy(&v))
}

#[cfg(test)]
mod tests {
    use super::Output;

    #[test]
    fn truthy_rejects_the_offy_spellings() {
        for off in ["", "0", "false", "FALSE", "no", "off", "Off"] {
            assert!(!Output::truthy(off), "{off:?} should be falsy");
        }
        for on in ["1", "true", "yes", "on", "anything"] {
            assert!(Output::truthy(on), "{on:?} should be truthy");
        }
    }
}
