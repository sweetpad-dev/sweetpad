//! `sweetpad app …` — the built app's lifecycle: build+install+launch, and the
//! running session. The app is the noun; these are its actions. All operate on
//! a simulator destination (physical devices land in a later iteration).

use clap::Subcommand;

use crate::cli::resolve::{self, BuildTarget, Resolved};
use crate::cli::xcodebuild::{self, AppBundle};
use crate::cli::{simctl, CliError, CliResult, Context};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Build, install, and launch on the resolved destination.
    Run {
        /// Target a physical device instead of a simulator.
        #[arg(long)]
        device: bool,
    },
    /// Build and install, without launching.
    Install,
    /// Launch an already-installed app.
    Launch,
    /// Stream the running app's logs.
    Logs,
    /// Terminate the running app.
    Stop,
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::Run { device } => run_app(ctx, *device),
        Action::Install => install_app(ctx),
        Action::Launch => launch_app(ctx),
        Action::Logs => logs_app(ctx),
        Action::Stop => stop_app(ctx),
    }
}

fn run_app(ctx: &mut Context, device: bool) -> CliResult {
    if device {
        return Err(CliError::new("physical device support is not implemented yet"));
    }
    let resolved = resolve::resolve(ctx)?;
    let target = resolve::build_target(ctx, &resolved)?;
    resolve::remember(ctx, &resolved, &target);
    let udid = udid(&target.destination)?;

    plan(&resolved, &target, false).run()?;
    let app = app_bundle(&resolved, &target)?;
    simctl::boot(&udid)?;
    simctl::install(&udid, &app.path.display().to_string())?;
    let out = simctl::launch(&udid, &app.bundle_id)?;
    ctx.out.note(&format!("launched {} → {}", app.bundle_id, out.trim()));
    Ok(())
}

fn install_app(ctx: &mut Context) -> CliResult {
    let resolved = resolve::resolve(ctx)?;
    let target = resolve::build_target(ctx, &resolved)?;
    resolve::remember(ctx, &resolved, &target);
    let udid = udid(&target.destination)?;

    plan(&resolved, &target, false).run()?;
    let app = app_bundle(&resolved, &target)?;
    simctl::boot(&udid)?;
    simctl::install(&udid, &app.path.display().to_string())?;
    ctx.out.note(&format!("installed {}", app.bundle_id));
    Ok(())
}

fn launch_app(ctx: &mut Context) -> CliResult {
    let resolved = resolve::resolve(ctx)?;
    let target = resolve::build_target(ctx, &resolved)?;
    resolve::remember(ctx, &resolved, &target);
    let udid = udid(&target.destination)?;

    let app = app_bundle(&resolved, &target)?;
    simctl::boot(&udid)?;
    let out = simctl::launch(&udid, &app.bundle_id)?;
    ctx.out.note(&format!("launched {} → {}", app.bundle_id, out.trim()));
    Ok(())
}

fn stop_app(ctx: &mut Context) -> CliResult {
    let resolved = resolve::resolve(ctx)?;
    let target = resolve::build_target(ctx, &resolved)?;
    let udid = udid(&target.destination)?;

    let app = app_bundle(&resolved, &target)?;
    simctl::terminate(&udid, &app.bundle_id)?;
    ctx.out.note(&format!("terminated {}", app.bundle_id));
    Ok(())
}

fn logs_app(ctx: &mut Context) -> CliResult {
    let resolved = resolve::resolve(ctx)?;
    let target = resolve::build_target(ctx, &resolved)?;
    let udid = udid(&target.destination)?;
    let app = app_bundle(&resolved, &target)?;

    // Best-effort filter: stream the simulator log for the app's executable.
    let name = app
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let predicate = format!("processImagePath CONTAINS \"{name}\"");
    ctx.out.note(&format!("streaming logs for {} (Ctrl-C to stop)", app.bundle_id));
    crate::cli::process::stream(
        "xcrun",
        &[
            "simctl",
            "spawn",
            &udid,
            "log",
            "stream",
            "--level=debug",
            "--style=compact",
            "--predicate",
            &predicate,
        ],
        None,
    )
}

/// Build an [`xcodebuild::BuildPlan`] borrowing the resolved target.
fn plan<'a>(resolved: &'a Resolved, target: &'a BuildTarget, clean: bool) -> xcodebuild::BuildPlan<'a> {
    xcodebuild::BuildPlan {
        container: &resolved.container,
        scheme: &target.scheme,
        configuration: &target.configuration,
        destination: Some(&target.destination),
        clean,
    }
}

/// Read the built app's `.app` path and bundle id from xcodebuild's settings.
fn app_bundle(resolved: &Resolved, target: &BuildTarget) -> Result<AppBundle, CliError> {
    let settings = xcodebuild::show_settings(
        &resolved.container,
        &target.scheme,
        &target.configuration,
        Some(&target.destination),
    )?;
    xcodebuild::app_bundle(&settings)
}

/// Extract the simulator UDID from a `platform=…,id=<udid>` destination.
fn udid(destination: &str) -> Result<String, CliError> {
    destination
        .split(',')
        .find_map(|kv| kv.trim().strip_prefix("id="))
        .map(str::to_string)
        .ok_or_else(|| {
            CliError::new(format!(
                "app commands need a simulator destination with an id= (got {destination:?})"
            ))
        })
}
