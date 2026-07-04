//! `sweetpad open [xcode|sim|dd|config]` — open the things you otherwise
//! Finder-hunt for: the container in Xcode, Simulator.app, the project's
//! DerivedData folder, or the config file.

use clap::ValueEnum;

use crate::cli::output::Output;
use crate::cli::{CliError, CommandResult, Context, Render, Rendered, process, resolve, simctl};

/// What to open.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum What {
    /// The resolved workspace/project in Xcode.
    Xcode,
    /// Simulator.app.
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
            process::stream("open", &[&path], None)?;
            path
        }
        What::Sim => {
            simctl::open_app()?;
            "Simulator.app".to_string()
        }
        What::Dd => {
            let paths = super::derived_data::project_paths(ctx)?;
            let Some(first) = paths.first() else {
                return Err(CliError::new(
                    "no DerivedData folder exists for this project yet (build once first)",
                ));
            };
            let path = first.display().to_string();
            process::stream("open", &[&path], None)?;
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
                std::fs::write(&path, "# sweetpad configuration — see `sweetpad help config`\n")
                    .map_err(|e| {
                        CliError::new(format!("failed to create {}: {e}", path.display()))
                    })?;
            }
            let path_str = path.display().to_string();
            process::stream("open", &["-t", &path_str], None)?;
            path_str
        }
    };
    Ok(Rendered::data(OpenReport { opened }))
}
