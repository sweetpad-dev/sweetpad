//! `sweetpad build …` — compile the project (via `xcodebuild`). `build` stays
//! purely "compile"; the run/install/launch lifecycle lives under
//! [`crate::cli::commands::app`].

use clap::Subcommand;

use crate::cli::{resolve, xcodebuild, CliResult, Context};

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
    let target = resolve::build_target(ctx, &resolved)?;
    resolve::remember(ctx, &resolved, &target);

    ctx.out.note(&format!(
        "building {} ({}) for {}",
        target.scheme, target.configuration, target.destination
    ));

    xcodebuild::BuildPlan {
        container: &resolved.container,
        scheme: &target.scheme,
        configuration: &target.configuration,
        destination: Some(&target.destination),
        clean,
    }
    .run()
}
