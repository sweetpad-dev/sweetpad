//! `sweetpad app …` — the built app's lifecycle: build+install+launch, and the
//! running session, on a simulator or a physical device. The app is the noun;
//! these are its actions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use clap::Subcommand;

use crate::cli::resolve::{self, Resolved};
use crate::cli::xcodebuild::{self, AppBundle};
use crate::cli::{devicectl, simctl, CliError, CliResult, Context};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Build, install, and launch; follows logs and watches by request.
    Run {
        /// Target a connected physical device instead of a simulator.
        #[arg(long)]
        device: bool,
        /// Specific device UDID/name to target (implies --device).
        #[arg(long = "device-id")]
        device_id: Option<String>,
        /// Rebuild, reinstall, and relaunch whenever a source file changes.
        #[arg(long)]
        watch: bool,
        /// Don't stream the app's logs after launching (logs follow by default
        /// on simulators).
        #[arg(long = "no-logs")]
        no_logs: bool,
    },
    /// Build and install, without launching.
    Install,
    /// Launch an already-installed app.
    Launch,
    /// Stream the running app's logs (simulator).
    Logs,
    /// Terminate the running app.
    Stop,
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::Run { device, device_id, watch, no_logs } => {
            run_app(ctx, *device || device_id.is_some(), device_id.as_deref(), *watch, *no_logs)
        }
        Action::Install => simple(ctx, Stage::Install),
        Action::Launch => simple(ctx, Stage::Launch),
        Action::Logs => simple(ctx, Stage::Logs),
        Action::Stop => simple(ctx, Stage::Stop),
    }
}

/// Where the app runs.
enum Target {
    Simulator(String),
    Device(String),
}

/// A fully-resolved run: container/scheme/configuration/destination plus the
/// concrete target to deploy onto.
struct RunPlan {
    resolved: Resolved,
    scheme: String,
    configuration: String,
    destination: String,
    target: Target,
}

impl RunPlan {
    fn build_plan(&self) -> xcodebuild::BuildPlan<'_> {
        xcodebuild::BuildPlan {
            container: &self.resolved.container,
            scheme: &self.scheme,
            configuration: &self.configuration,
            destination: Some(&self.destination),
            clean: false,
        }
    }

    fn app_bundle(&self) -> Result<AppBundle, CliError> {
        let settings = xcodebuild::show_settings(
            &self.resolved.container,
            &self.scheme,
            &self.configuration,
            Some(&self.destination),
        )?;
        xcodebuild::app_bundle(&settings)
    }
}

fn run_app(
    ctx: &mut Context,
    device: bool,
    device_id: Option<&str>,
    watch: bool,
    no_logs: bool,
) -> CliResult {
    let plan = plan(ctx, device, device_id)?;
    deploy(ctx, &plan)?;

    if watch {
        return watch_loop(ctx, &plan);
    }
    if !no_logs {
        return follow_logs(ctx, &plan);
    }
    Ok(())
}

/// Resolve a full run plan, choosing a simulator (default) or a device.
fn plan(ctx: &mut Context, device: bool, device_id: Option<&str>) -> Result<RunPlan, CliError> {
    let resolved = resolve::resolve(ctx)?;
    let schemes = resolve::schemes(&resolved.container)?;
    let scheme = resolve::choose(ctx, "scheme", resolved.scheme.clone(), &schemes)?;
    let configuration = resolved
        .configuration
        .clone()
        .unwrap_or_else(|| "Debug".to_string());

    let (destination, target) = if device {
        let devices = devicectl::list()?;
        let dev = if let Some(id) = device_id {
            devicectl::find(&devices, id)
                .ok_or_else(|| CliError::new(format!("no device matching {id:?}")))?
        } else {
            let labels: Vec<String> = devices.iter().map(devicectl::Device::label).collect();
            let chosen = resolve::choose(ctx, "device", None, &labels)?;
            devices
                .iter()
                .find(|d| d.label() == chosen)
                .ok_or_else(|| CliError::new("device not found"))?
        };
        let platform = if dev.platform.is_empty() { "iOS" } else { &dev.platform };
        (
            format!("platform={platform},id={}", dev.udid),
            Target::Device(dev.udid.clone()),
        )
    } else {
        // Reuse the simulator-aware build-target resolution for the destination.
        let bt = resolve::build_target(ctx, &resolved)?;
        let udid = udid(&bt.destination)?;
        (bt.destination, Target::Simulator(udid))
    };

    let plan = RunPlan { resolved, scheme, configuration, destination, target };
    let bt = resolve::BuildTarget {
        scheme: plan.scheme.clone(),
        configuration: plan.configuration.clone(),
        destination: plan.destination.clone(),
    };
    resolve::remember(ctx, &plan.resolved, &bt);
    Ok(plan)
}

/// Build, install, and launch — the unit re-run by `--watch`.
fn deploy(ctx: &Context, plan: &RunPlan) -> CliResult {
    plan.build_plan().run()?;
    let app = plan.app_bundle()?;
    let app_path = app.path.display().to_string();
    match &plan.target {
        Target::Simulator(udid) => {
            simctl::boot(udid)?;
            simctl::install(udid, &app_path)?;
            let out = simctl::launch(udid, &app.bundle_id)?;
            ctx.out.note(&format!("launched {} → {}", app.bundle_id, out.trim()));
        }
        Target::Device(id) => {
            devicectl::install(id, &app_path)?;
            let out = devicectl::launch(id, &app.bundle_id)?;
            ctx.out.note(&format!("launched {} on device → {}", app.bundle_id, out.trim()));
        }
    }
    Ok(())
}

/// Follow the app's logs after launch (simulator only; devices print a note).
fn follow_logs(ctx: &Context, plan: &RunPlan) -> CliResult {
    match &plan.target {
        Target::Simulator(udid) => {
            let app = plan.app_bundle()?;
            stream_logs(ctx, udid, &app)
        }
        Target::Device(_) => {
            ctx.out.note("inline log streaming for devices is not supported yet; app left running");
            Ok(())
        }
    }
}

/// Poll the project's source tree and redeploy on change.
fn watch_loop(ctx: &Context, plan: &RunPlan) -> CliResult {
    let root = plan
        .resolved
        .container
        .path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut snapshot = scan_sources(&root);
    ctx.out.note("watching for changes (Ctrl-C to stop)");
    loop {
        std::thread::sleep(Duration::from_millis(800));
        let next = scan_sources(&root);
        if next != snapshot {
            snapshot = next;
            ctx.out.note("change detected — rebuilding");
            if let Err(e) = deploy(ctx, plan) {
                // Keep watching after a failed build instead of bailing out.
                ctx.out.error(&e.to_string());
            }
        }
    }
}

/// A snapshot of `.swift` file modification times under `root`, skipping hidden
/// and build directories.
fn scan_sources(root: &Path) -> BTreeMap<PathBuf, SystemTime> {
    let mut map = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || matches!(name.as_ref(), "DerivedData" | "build" | ".build" | "Pods") {
                continue;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("swift")
                && let Ok(mtime) = entry.metadata().and_then(|m| m.modified())
            {
                map.insert(path, mtime);
            }
        }
    }
    map
}

/// The stage-only `app` actions (install/launch/logs/stop) share resolution.
#[derive(Clone, Copy)]
enum Stage {
    Install,
    Launch,
    Logs,
    Stop,
}

fn simple(ctx: &mut Context, stage: Stage) -> CliResult {
    // These default to a simulator target (the common headless case).
    let plan = plan(ctx, false, None)?;
    let app = plan.app_bundle()?;
    let Target::Simulator(udid) = &plan.target else {
        unreachable!("simple stages resolve a simulator target");
    };

    match stage {
        Stage::Install => {
            plan.build_plan().run()?;
            simctl::boot(udid)?;
            simctl::install(udid, &app.path.display().to_string())?;
            ctx.out.note(&format!("installed {}", app.bundle_id));
        }
        Stage::Launch => {
            simctl::boot(udid)?;
            let out = simctl::launch(udid, &app.bundle_id)?;
            ctx.out.note(&format!("launched {} → {}", app.bundle_id, out.trim()));
        }
        Stage::Logs => return stream_logs(ctx, udid, &app),
        Stage::Stop => {
            simctl::terminate(udid, &app.bundle_id)?;
            ctx.out.note(&format!("terminated {}", app.bundle_id));
        }
    }
    Ok(())
}

/// Stream a simulator's log for the app's executable (best-effort predicate).
fn stream_logs(ctx: &Context, udid: &str, app: &AppBundle) -> CliResult {
    let name = app.path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let predicate = format!("processImagePath CONTAINS \"{name}\"");
    ctx.out.note(&format!("streaming logs for {} (Ctrl-C to stop)", app.bundle_id));
    crate::cli::process::stream(
        "xcrun",
        &[
            "simctl",
            "spawn",
            udid,
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

/// Extract the simulator UDID from a `platform=…,id=<udid>` destination.
fn udid(destination: &str) -> Result<String, CliError> {
    destination
        .split(',')
        .find_map(|kv| kv.trim().strip_prefix("id="))
        .map(str::to_string)
        .ok_or_else(|| {
            CliError::new(format!(
                "app commands need a destination with an id= (got {destination:?})"
            ))
        })
}
