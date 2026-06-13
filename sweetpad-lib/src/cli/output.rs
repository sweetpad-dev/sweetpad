//! Output rendering: human/colored by default, machine-readable under `--json`.
//!
//! Color auto-disables when stdout is not a TTY, when `NO_COLOR` is set, or
//! with `--no-color`. Human messages and errors go to stderr; primary data
//! goes to stdout (so `--json` payloads pipe cleanly). This is intentionally
//! small for the scaffold — render helpers grow alongside the commands.

use std::io::{IsTerminal, Write};

use crate::cli::GlobalArgs;

/// Resolved output mode shared across commands.
pub struct Output {
    json: bool,
    color: bool,
    verbose: u8,
}

impl Output {
    #[must_use]
    pub fn new(global: &GlobalArgs) -> Self {
        let color = !global.no_color
            && std::env::var_os("NO_COLOR").is_none()
            && std::io::stdout().is_terminal();
        Self { json: global.json, color, verbose: global.verbose }
    }

    #[must_use]
    pub fn is_json(&self) -> bool {
        self.json
    }

    #[must_use]
    pub fn use_color(&self) -> bool {
        self.color
    }

    /// True when stdout is interactive — gates the interactive picker fallback
    /// in [`crate::cli::resolve`].
    #[must_use]
    pub fn is_interactive(&self) -> bool {
        !self.json && std::io::stderr().is_terminal()
    }

    /// Print a primary data line to stdout (human mode only — JSON commands
    /// build a payload and emit it via [`Output::json_value`]).
    pub fn line(&self, s: &str) {
        if !self.json {
            println!("{s}");
        }
    }

    /// Emit a JSON value to stdout. No-op in human mode.
    pub fn json_value(&self, value: &serde_json::Value) {
        if self.json
            && let Ok(s) = serde_json::to_string_pretty(value)
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

    /// An informational note to stderr (human mode only).
    pub fn note(&self, s: &str) {
        if !self.json {
            let _ = writeln!(std::io::stderr(), "{}", self.dim(s));
        }
    }

    /// A verbose-only diagnostic to stderr, gated on `-v`.
    pub fn debug(&self, s: &str) {
        if self.verbose > 0 && !self.json {
            let _ = writeln!(std::io::stderr(), "{}", self.dim(&format!("[debug] {s}")));
        }
    }

    /// Render an error. JSON mode emits a structured object to stderr; human
    /// mode prints a red-ish prefix.
    pub fn error(&self, msg: &str) {
        if self.json {
            let payload = serde_json::json!({ "error": { "message": msg } });
            if let Ok(s) = serde_json::to_string(&payload) {
                let _ = writeln!(std::io::stderr(), "{s}");
            }
        } else {
            let prefix = if self.color { "\x1b[31merror:\x1b[0m" } else { "error:" };
            let _ = writeln!(std::io::stderr(), "{prefix} {msg}");
        }
    }

    fn dim(&self, s: &str) -> String {
        if self.color { format!("\x1b[2m{s}\x1b[0m") } else { s.to_string() }
    }
}
