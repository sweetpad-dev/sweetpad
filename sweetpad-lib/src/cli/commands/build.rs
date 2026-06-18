//! `sweetpad build …` — compile the project (via `xcodebuild`, or `swift build`
//! for a Swift package), and generate a build tool's config from the Xcode
//! project. `build` stays purely "compile"; the run/install/launch lifecycle
//! lives under [`crate::cli::commands::app`].

use std::path::{Path, PathBuf};

use clap::Subcommand;

use crate::cli::backend::{self, BuildBackend, BuildOptions};
use crate::cli::resolve::{Container, Resolved};
use crate::cli::{CliError, CliResult, Context, resolve};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Compile the resolved scheme for the resolved destination.
    Start {
        /// Clean before building.
        #[arg(long)]
        clean: bool,
    },
    /// Generate the selected backend's config from the Xcode project (e.g.
    /// `--backend xtool` writes `Package.swift` + `xtool.yml`). Native backends
    /// (xcodebuild, swiftpm) read the project directly and generate nothing.
    Generate {
        /// Directory to write the config into. Defaults to the project
        /// directory, where it is meant to be committed.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Show the normalized build plan (identity + source/dependency graph) that
    /// config-generating backends consume. Use `--json` for machine output.
    Plan,
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::Start { clean } => start(ctx, *clean),
        Action::Generate { output } => generate(ctx, output.as_deref()),
        Action::Plan => plan(ctx),
    }
}

fn start(ctx: &mut Context, clean: bool) -> CliResult {
    let resolved = resolve::resolve(ctx)?;
    let backend = select_backend(ctx, &resolved)?;
    backend.build(ctx, &resolved, &BuildOptions { clean })
}

fn generate(ctx: &mut Context, output: Option<&Path>) -> CliResult {
    let resolved = resolve::resolve(ctx)?;
    let backend = select_backend(ctx, &resolved)?;
    let out_dir = output.map_or_else(|| project_dir(&resolved.container), Path::to_path_buf);
    backend.generate(ctx, &resolved, &out_dir)
}

fn plan(ctx: &mut Context) -> CliResult {
    let resolved = resolve::resolve(ctx)?;
    let plan = crate::cli::plan::build_plan(ctx, &resolved)?;

    if ctx.out.is_json() {
        let value = serde_json::to_value(&plan).map_err(|e| CliError::new(e.to_string()))?;
        ctx.out.json_value(&value);
        return Ok(());
    }

    ctx.out.line(&format!("scheme:        {}", plan.scheme));
    ctx.out
        .line(&format!("configuration: {}", plan.configuration));
    ctx.out.line(&format!("app target:    {}", plan.app_target));
    ctx.out
        .line(&format!("product name:  {}", plan.product_name));
    ctx.out.line(&format!("bundle id:     {}", plan.bundle_id));
    if let Some(dt) = &plan.deployment_target {
        ctx.out.line(&format!("deployment:    iOS {dt}"));
    }
    if let Some(p) = &plan.supported_platforms {
        ctx.out.line(&format!("platforms:     {p}"));
    }
    ctx.out
        .line(&format!("sources:       {} file(s)", plan.sources.len()));
    for s in &plan.sources {
        ctx.out.line(&format!("  {}", s.display()));
    }
    if !plan.dependencies.is_empty() {
        ctx.out
            .line(&format!("dependencies:  {}", plan.dependencies.join(", ")));
    }
    Ok(())
}

/// Resolve the backend to use for a command. Precedence: explicit `--backend`
/// flag > per-project config (`backend = …`) > auto-selection by project type
/// (Swift packages → `swift build`, else `xcodebuild`).
fn select_backend(
    ctx: &Context,
    resolved: &Resolved,
) -> Result<&'static dyn BuildBackend, CliError> {
    let requested = ctx
        .global
        .backend
        .clone()
        .or_else(|| ctx.config.for_project(&resolved.container.key()).backend);
    backend::select(requested.as_deref(), &resolved.container)
}

/// Default output directory for generated config: the directory containing the
/// project container (or the current directory for a bare, relative path).
fn project_dir(container: &Container) -> PathBuf {
    container
        .path()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}
