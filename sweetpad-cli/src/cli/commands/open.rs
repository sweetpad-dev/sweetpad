//! `sweetpad open [xcode|sim|dd|config]` — open the things you otherwise
//! Finder-hunt for: the container in Xcode, the simulator window, the project's
//! DerivedData folder, or the config file.
//!
//! Off macOS there's no Xcode or Simulator to open, so those fail and say so.
//! Folders go to `xdg-open`, and the config file to `$VISUAL` or `$EDITOR` at
//! a terminal, else `xdg-open`. Linux has no `open` of macOS's kind, and
//! usually none at all.

use clap::ValueEnum;

use crate::cli::output::Output;
use crate::cli::{
    CliError, CommandResult, Context, ErrorKind, Render, Rendered, process, resolve, simctl,
};

/// What to open.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum What {
    /// The resolved workspace/project in Xcode.
    Xcode,
    /// The simulator window.
    Sim,
    /// This project's DerivedData folder in Finder.
    Dd,
    /// The sweetpad config file (created empty if missing) in the default editor.
    Config,
}

struct OpenReport {
    opened: String,
}

impl Render for OpenReport {
    fn human(&self, out: &Output) {
        out.note(&format!("opened {}", self.opened));
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({ "opened": self.opened })
    }
}

pub fn run(ctx: &mut Context, what: What) -> CommandResult {
    let opened = match what {
        What::Xcode => {
            let container = resolve::container(ctx)?;
            let path = container.path().display().to_string();
            if !cfg!(target_os = "macos") {
                return Err(macos_only(&format!(
                    "Xcode runs only on macOS, so there's nothing here to open {path} in"
                )));
            }
            process::stream("open", &[&path], None)?;
            path
        }
        What::Sim => {
            if !cfg!(target_os = "macos") {
                return Err(macos_only("the Simulator runs only on macOS"));
            }
            simctl::open_app()?
        }
        What::Dd => {
            let scope = super::derived_data::project_scope(ctx)?;
            let Some(first) = scope.own.first() else {
                return Err(CliError::new(
                    "no DerivedData folder exists for this project yet (build once first)",
                ));
            };
            let path = first.display().to_string();
            open_with_desktop(&path, &[])?;
            path
        }
        What::Config => {
            let Some(path) = crate::cli::config::Config::path() else {
                return Err(CliError::new("cannot locate the config directory"));
            };
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        CliError::new(format!("failed to create {}: {e}", parent.display()))
                    })?;
                }
                std::fs::write(
                    &path,
                    "# sweetpad configuration — see 'sweetpad help config'\n",
                )
                .map_err(|e| CliError::new(format!("failed to create {}: {e}", path.display())))?;
            }
            let path_str = path.display().to_string();
            let editor = ["VISUAL", "EDITOR"]
                .iter()
                .find_map(|key| std::env::var(key).ok().filter(|v| !v.trim().is_empty()));
            match editor {
                Some(editor) if !cfg!(target_os = "macos") && ctx.out.is_interactive() => {
                    // As Git runs it: the variable may carry arguments.
                    let script = format!("{editor} \"$1\"");
                    process::stream("sh", &["-c", &script, "sh", &path_str], None)?;
                }
                _ => open_with_desktop(&path_str, &["-t"])?,
            }
            path_str
        }
    };
    Ok(Rendered::data(OpenReport { opened }))
}

/// A failure because this host has no macOS app to open.
fn macos_only(message: &str) -> CliError {
    CliError::new(message).kind(ErrorKind::ToolMissing)
}

/// Open `path` the way the desktop opens it: `open` (with `mac_args`) on
/// macOS, `xdg-open` elsewhere. Without a desktop the error says where the
/// file is, which is what the user came for.
fn open_with_desktop(path: &str, mac_args: &[&str]) -> Result<(), CliError> {
    if cfg!(target_os = "macos") {
        let mut args = mac_args.to_vec();
        args.push(path);
        return process::stream("open", &args, None);
    }
    process::stream("xdg-open", &[path], None).map_err(|e| e.tip(Some(format!("it's at {path}"))))
}
