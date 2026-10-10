//! `sweetpad self-update` — upgrade the binary. Homebrew installs run
//! `brew upgrade sweetpad`, mise installs are told the `mise upgrade` to run,
//! and anything else gets pointed at the tap.

use std::path::Path;

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
    // `/Cellar/` covers resolved keg paths everywhere (incl. Linuxbrew);
    // `/opt/homebrew/` is the Apple-silicon prefix. A bare `/homebrew/` would
    // also match unrelated checkouts like ~/dev/homebrew-tools/.
    let brewed = path.contains("/Cellar/") || path.contains("/opt/homebrew/");
    if brewed {
        ctx.out.note("updating via Homebrew…");
        // brew always writes `==> …` progress to stdout — captured under the
        // machine modes so it can't interleave with the envelope.
        if ctx.out.is_json() || ctx.out.is_ndjson() {
            let run = process::run_captured("brew", &["upgrade", "sweetpad"], None)?;
            if !run.success {
                return Err(crate::cli::CliError::new(format!(
                    "brew upgrade sweetpad exited with a non-zero status:\n{}",
                    run.tail
                ))
                .context("running 'brew upgrade sweetpad'"));
            }
        } else {
            process::stream("brew", &["upgrade", "sweetpad"], None)
                .context("running 'brew upgrade sweetpad'")?;
        }
        return Ok(Rendered::data(UpdateReport {
            method: "homebrew",
            note: "brew upgrade sweetpad completed".to_string(),
        }));
    }
    // Named rather than run: `mise upgrade` follows whichever mise.toml governs
    // the current directory, and a project's can pin a version the global one
    // doesn't.
    if let Some(tool) = mise_tool(&exe) {
        return Ok(Rendered::data(UpdateReport {
            method: "mise",
            note: format!(
                "this sweetpad was installed by mise; upgrade with 'mise upgrade {tool}'"
            ),
        }));
    }
    Ok(Rendered::data(UpdateReport {
        method: "manual",
        note: format!(
            "this sweetpad ({path}) wasn't installed via Homebrew; install/upgrade with \
             'brew install sweetpad-dev/tap/sweetpad', or replace the binary from the \
             latest cli-v* release"
        ),
    }))
}

/// The mise tool identifier `exe` was installed as, such as
/// `github:sweetpad-dev/sweetpad`. mise writes `.mise.backend.toml` into each
/// tool's directory, above its per-version ones, so the nearest ancestor that
/// holds one names the tool; reading it also covers a relocated
/// `MISE_DATA_DIR`. The few levels searched leave room for a `bin/` an archive
/// may carry.
fn mise_tool(exe: &Path) -> Option<String> {
    exe.ancestors().skip(1).take(4).find_map(|dir| {
        let text = std::fs::read_to_string(dir.join(".mise.backend.toml")).ok()?;
        text.lines().find_map(|line| {
            let value = line.strip_prefix("short")?.trim_start().strip_prefix('=')?;
            let value = value.trim().trim_matches('"');
            (!value.is_empty()).then(|| value.to_string())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    #[test]
    fn mise_tool_reads_the_backend_beside_the_versions() {
        let dir = TempDir::new("sweetpad-self-update-mise");
        let tool = dir.join("installs/github-sweetpad-dev-sweetpad");
        std::fs::create_dir_all(tool.join("0.1.12")).unwrap();
        std::fs::write(
            tool.join(".mise.backend.toml"),
            "short = \"github:sweetpad-dev/sweetpad\"\nfull = \"github:sweetpad-dev/sweetpad\"\n\n[opts]\nversion_prefix = \"cli-v\"\n",
        )
        .unwrap();

        assert_eq!(
            mise_tool(&tool.join("0.1.12/sweetpad")).as_deref(),
            Some("github:sweetpad-dev/sweetpad")
        );
        assert_eq!(
            mise_tool(&tool.join("0.1.12/bin/sweetpad")).as_deref(),
            Some("github:sweetpad-dev/sweetpad")
        );
    }

    #[test]
    fn mise_tool_is_none_outside_a_mise_install() {
        let dir = TempDir::new("sweetpad-self-update-plain");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        assert_eq!(mise_tool(&dir.join("bin/sweetpad")), None);
    }
}
