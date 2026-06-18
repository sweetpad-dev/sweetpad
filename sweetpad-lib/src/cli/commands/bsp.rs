//! `sweetpad bsp …` — Build Server Protocol integration. `bsp init` writes a
//! `buildServer.json` pointing sourcekit-lsp at this binary's bundled BSP
//! server (see [`crate::bsp`]), enabling cross-file autocomplete.

use std::path::{Path, PathBuf};

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
    let container = resolve::container(ctx)?;

    let mut args: Vec<String> = match &container {
        Container::Workspace(p) => vec!["--workspace".into(), p.display().to_string()],
        Container::Project(p) => vec!["--project".into(), p.display().to_string()],
        Container::SwiftPackage(p) => {
            init_swift_package(ctx, p, output);
            return Ok(());
        }
    };
    if let Some(out) = output {
        args.push("--output".into());
        args.push(out.display().to_string());
    }

    crate::bsp::write_config(&args).map_err(CliError::new)?;

    if ctx.out.is_json() {
        let path = buildserver_path(container.path(), output);
        ctx.out.json_value(&serde_json::json!({
            "buildServerJson": path.display().to_string(),
        }));
    }
    Ok(())
}

/// `bsp init` for a Swift package. sourcekit-lsp supports SwiftPM natively — it
/// reads `Package.swift` and builds the index itself — so we deliberately do
/// *not* write a `buildServer.json`; one would override that native path with
/// our BSP server, which only understands Xcode projects. We don't trigger a
/// build either; sourcekit-lsp populates the index on demand.
///
/// The one real hazard is a pre-existing `buildServer.json`: its mere presence
/// makes sourcekit-lsp use BSP and breaks SPM semantics, so flag it loudly. We
/// warn rather than delete — removing a file the user may have authored is their
/// call (or the extension's).
fn init_swift_package(ctx: &mut Context, manifest_path: &Path, output: Option<&Path>) {
    let config_path = buildserver_path(manifest_path, output);
    let stale = config_path.exists();

    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({
            "buildServerJson": null,
            "native": true,
            "staleBuildServerJson": stale.then(|| config_path.display().to_string()),
        }));
    } else {
        ctx.out.note(
            "Swift package detected; sourcekit-lsp supports SwiftPM natively, so no \
             buildServer.json is written",
        );
        if stale {
            ctx.out.note(&format!(
                "a buildServer.json already exists at {} — remove it so sourcekit-lsp \
                 uses its native SwiftPM support instead of the BSP server (which only \
                 handles Xcode projects)",
                config_path.display()
            ));
        }
    }
}

/// Where sourcekit-lsp looks for a `buildServer.json`: the explicit `--output`,
/// else next to the container (its parent directory).
fn buildserver_path(container: &Path, output: Option<&Path>) -> PathBuf {
    output.map_or_else(
        || {
            container
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("buildServer.json")
        },
        Path::to_path_buf,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buildserver_path_defaults_next_to_container() {
        assert_eq!(
            buildserver_path(Path::new("/pkg/Package.swift"), None),
            PathBuf::from("/pkg/buildServer.json")
        );
    }

    #[test]
    fn buildserver_path_honors_explicit_output() {
        assert_eq!(
            buildserver_path(
                Path::new("/pkg/Package.swift"),
                Some(Path::new("/tmp/bs.json"))
            ),
            PathBuf::from("/tmp/bs.json")
        );
    }
}
