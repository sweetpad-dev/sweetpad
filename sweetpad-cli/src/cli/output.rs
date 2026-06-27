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
    non_interactive: bool,
    color: bool,
    verbose: bool,
    quiet: bool,
}

impl Output {
    #[must_use]
    pub fn new(global: &GlobalArgs) -> Self {
        // `--no-color`/`NO_COLOR` always win; otherwise `CLICOLOR_FORCE`/
        // `FORCE_COLOR` force color even when piped, else default to a TTY check.
        let no_color = global.no_color || std::env::var_os("NO_COLOR").is_some();
        let force_color = std::env::var_os("CLICOLOR_FORCE").is_some()
            || std::env::var_os("FORCE_COLOR").is_some();
        let color = !no_color && (force_color || std::io::stdout().is_terminal());
        let non_interactive =
            global.non_interactive || std::env::var_os("SWEETPAD_NONINTERACTIVE").is_some();
        Self {
            json: global.json,
            non_interactive,
            color,
            // `--quiet` wins over `--verbose`, so a script that always passes
            // `-v` can still be quieted.
            verbose: global.verbose && !global.quiet,
            quiet: global.quiet,
        }
    }

    #[must_use]
    pub fn is_json(&self) -> bool {
        self.json
    }

    #[must_use]
    pub fn use_color(&self) -> bool {
        self.color
    }

    /// True when the terminal is interactive — gates the picker fallback in
    /// [`crate::cli::resolve`], spinners, and `app run`'s rebuild session. False
    /// under `--json`, `--non-interactive`/`SWEETPAD_NONINTERACTIVE`, or when
    /// stderr is not a TTY.
    #[must_use]
    pub fn is_interactive(&self) -> bool {
        !self.json && !self.non_interactive && std::io::stderr().is_terminal()
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
    /// build a payload and emit it via [`Output::json_value`]).
    pub fn line(&self, s: &str) {
        if !self.json {
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

    /// Print a list item to stdout (human mode only), optionally marked as the
    /// currently selected entry (green `*` when color is on).
    pub fn item(&self, name: &str, selected: bool) {
        if self.json {
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
        if !self.json && !self.quiet {
            let _ = writeln!(std::io::stderr(), "{}", self.dim(s));
        }
    }

    /// A prominent (non-dimmed, yellow) status line to stderr — e.g. the running
    /// app exiting. Stands out from the dim session notes.
    pub fn alert(&self, s: &str) {
        if !self.json && !self.quiet {
            let line = if self.color {
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
        let _spinner = Spinner::start(message, self.is_interactive() && !self.quiet, self.color);
        f()
    }

    /// A verbose-only diagnostic to stderr, gated on `-v`.
    pub fn debug(&self, s: &str) {
        if self.verbose && !self.json {
            let _ = writeln!(std::io::stderr(), "{}", self.dim(&format!("[debug] {s}")));
        }
    }

    /// Render an error. JSON mode emits a structured object (the flattened
    /// message) to stderr. Human mode prints a red `error:` prefix with the
    /// operation [`headline`](CliError::headline) in bold; any underlying
    /// [`detail`](CliError::detail) follows on the next line, dimmed and indented
    /// two spaces — so "what we were doing" reads at a glance and the raw tool
    /// output sits quietly beneath it.
    pub fn error(&self, err: &CliError) {
        if self.json {
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
        let prefix = if self.color {
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

    fn dim(&self, s: &str) -> String {
        if self.color {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    fn bold(&self, s: &str) -> String {
        if self.color {
            format!("\x1b[1m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}
