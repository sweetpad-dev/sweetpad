//! `sweetpad build …` — compile the project (via `xcodebuild`, or `swift build`
//! for a Swift package). `build` stays purely "compile"; the run/install/launch
//! lifecycle lives under [`crate::cli::commands::app`].

use clap::Subcommand;

use crate::cli::{CliResult, Context, resolve, swiftpm, xcodebuild};

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

    // Swift packages have no simulator destination; build them with the `swift`
    // toolchain rather than routing through xcodebuild (which would force a
    // destination on us).
    if matches!(resolved.container, resolve::Container::SwiftPackage(_)) {
        let configuration = resolved
            .configuration
            .clone()
            .unwrap_or_else(|| "Debug".to_string());
        ctx.out.note(&format!(
            "building Swift package ({configuration}) with swift build"
        ));
        return swiftpm::build(&resolved.container, &configuration, clean);
    }

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
        hot: false,
    }
    .run(&ctx.out)
}
