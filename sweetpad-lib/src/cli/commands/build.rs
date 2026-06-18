//! `sweetpad build …` — compile the project (via `xcodebuild`, or `swift build`
//! for a Swift package). `build` stays purely "compile"; the run/install/launch
//! lifecycle lives under [`crate::cli::commands::app`].

use clap::Subcommand;

use crate::cli::backend::{self, BuildOptions};
use crate::cli::{CliResult, Context, resolve};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Compile the resolved scheme for the resolved destination.
    Start {
        /// Clean before building.
        #[arg(long)]
        clean: bool,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::Start { clean } => start(ctx, *clean),
    }
}

fn start(ctx: &mut Context, clean: bool) -> CliResult {
    let resolved = resolve::resolve(ctx)?;

    // Backend precedence: explicit `--backend` flag > per-project config >
    // auto-selection by project type. Auto-selection reproduces the historical
    // routing (Swift packages → `swift build`, else `xcodebuild`).
    let requested = ctx
        .global
        .backend
        .clone()
        .or_else(|| ctx.config.for_project(&resolved.container.key()).backend);
    let backend = backend::select(requested.as_deref(), &resolved.container)?;

    backend.build(ctx, &resolved, &BuildOptions { clean })
}
