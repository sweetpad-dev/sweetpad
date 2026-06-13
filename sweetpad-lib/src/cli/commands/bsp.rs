//! `sweetpad bsp …` — Build Server Protocol integration. `bsp init` writes a
//! `buildServer.json` pointing sourcekit-lsp at this binary's bundled BSP
//! server (see [`crate::bsp`]), enabling cross-file autocomplete.

use std::path::PathBuf;

use clap::Subcommand;

use crate::cli::resolve::{self, Container};
use crate::cli::{CliError, CliResult, Context};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Generate buildServer.json for sourcekit-lsp autocomplete.
    Init {
        /// Where to write buildServer.json (defaults to the project's parent).
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::Init { output } => init(ctx, output.as_deref()),
    }
}

fn init(ctx: &mut Context, output: Option<&std::path::Path>) -> CliResult {
    let resolved = resolve::resolve(ctx)?;

    let mut args: Vec<String> = match &resolved.container {
        Container::Workspace(p) => vec!["--workspace".into(), p.display().to_string()],
        Container::Project(p) => vec!["--project".into(), p.display().to_string()],
        Container::SwiftPackage(p) => {
            return Err(CliError::new(format!(
                "bsp init needs an Xcode project/workspace; found a Swift package ({})",
                p.display()
            )));
        }
    };
    if let Some(out) = output {
        args.push("--output".into());
        args.push(out.display().to_string());
    }

    crate::bsp::write_config(&args).map_err(CliError::new)?;

    if ctx.out.is_json() {
        let path = output.map_or_else(
            || {
                resolved
                    .container
                    .path()
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join("buildServer.json")
            },
            PathBuf::from,
        );
        ctx.out.json_value(&serde_json::json!({
            "buildServerJson": path.display().to_string(),
        }));
    }
    Ok(())
}
