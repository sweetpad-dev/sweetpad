//! `sweetpad build …` — compile the project (via `xcodebuild`, or `swift build`
//! for a Swift package). `build` stays purely "compile"; the run/install/launch
//! lifecycle lives under [`crate::cli::commands::app`].

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::{
    CommandResult, Context, ErrorKind, Render, Rendered, resolve, swiftpm, xcodebuild,
};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Compile the resolved scheme for the resolved destination.
    Start {
        /// Clean before building.
        #[arg(long)]
        clean: bool,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Start { clean } => start(ctx, *clean),
    }
}

/// The build result. Human mode already streamed the beautified log, so this
/// renders nothing extra there; `--json` emits it as the terminal envelope.
struct BuildReport {
    scheme: Option<String>,
    configuration: String,
    destination: Option<String>,
}

impl Render for BuildReport {
    fn human(&self, _out: &Output) {}

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "built": true,
            "scheme": self.scheme,
            "configuration": self.configuration,
            "destination": self.destination,
        })
    }
}

fn start(ctx: &mut Context, clean: bool) -> CommandResult {
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
        swiftpm::build(&resolved.container, &configuration, clean, ctx.out.is_json())
            .map_err(|e| e.or_kind(ErrorKind::BuildFailure))?;
        return Ok(Rendered::data(BuildReport {
            scheme: None,
            configuration,
            destination: None,
        }));
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
    .map_err(|e| e.or_kind(ErrorKind::BuildFailure))?;

    Ok(Rendered::data(BuildReport {
        scheme: Some(target.scheme),
        configuration: target.configuration,
        destination: Some(target.destination),
    }))
}
