//! `sweetpad self-update` — upgrade the binary. Homebrew installs run
//! `brew upgrade sweetpad`; anything else gets pointed at the tap.

use crate::cli::output::Output;
use crate::cli::{CommandResult, Context, ErrorContext, Render, Rendered, process};

struct UpdateReport {
    method: &'static str,
    note: String,
}

impl Render for UpdateReport {
    fn human(&self, out: &Output) {
        out.note(&self.note);
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({ "method": self.method, "note": self.note })
    }
}

pub fn run(ctx: &mut Context) -> CommandResult {
    // Canonicalize first: Homebrew launches through a bin/ symlink
    // (/usr/local/bin on Intel matches neither substring), and current_exe
    // doesn't resolve it.
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_default();
    let path = exe.to_string_lossy();
    let brewed = path.contains("/Cellar/") || path.contains("/homebrew/");
    if brewed {
        ctx.out.note("updating via Homebrew…");
        process::stream("brew", &["upgrade", "sweetpad"], None)
            .context("running `brew upgrade sweetpad`")?;
        return Ok(Rendered::data(UpdateReport {
            method: "homebrew",
            note: "brew upgrade sweetpad completed".to_string(),
        }));
    }
    Ok(Rendered::data(UpdateReport {
        method: "manual",
        note: format!(
            "this sweetpad ({path}) wasn't installed via Homebrew; install/upgrade with \
             `brew install sweetpad-dev/tap/sweetpad`, or replace the binary from the \
             latest cli-v* release"
        ),
    }))
}
