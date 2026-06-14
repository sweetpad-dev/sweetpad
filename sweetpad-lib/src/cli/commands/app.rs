//! `sweetpad app …` — the built app's lifecycle: build+install+launch, and the
//! running session, on a simulator or a physical device. The app is the noun;
//! these are its actions.

use std::path::Path;
use std::process::Child;

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::resolve::{self, Resolved};
use crate::cli::xcodebuild::{self, AppBundle};
use crate::cli::{CliError, CliResult, Context, devicectl, rawmode, simctl};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Build, install, launch, and follow logs; press `r` to rebuild on demand.
    Run {
        /// Target a connected physical device instead of a simulator.
        #[arg(long)]
        device: bool,
        /// Specific device UDID/name to target (implies --device).
        #[arg(long = "device-id")]
        device_id: Option<String>,
        /// Build and run as a native macOS app (launches the executable).
        #[arg(long, conflicts_with_all = ["device", "device_id"])]
        mac: bool,
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
    /// Open a URL on a simulator — drives deep links / universal links in.
    OpenUrl {
        /// The URL to open (e.g. `myapp://path` or `https://example.com/x`).
        url: String,
        /// Simulator name or UDID to open it on (defaults to the booted one).
        #[arg(long)]
        simulator: Option<String>,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::Run {
            device,
            device_id,
            mac,
            no_logs,
        } => run_app(
            ctx,
            &RunOpts {
                device: *device || device_id.is_some(),
                device_id: device_id.as_deref(),
                mac: *mac,
                no_logs: *no_logs,
            },
        ),
        Action::Install => simple(ctx, Stage::Install),
        Action::Launch => simple(ctx, Stage::Launch),
        Action::Logs => simple(ctx, Stage::Logs),
        Action::Stop => simple(ctx, Stage::Stop),
        Action::OpenUrl { url, simulator } => open_url(ctx, url, simulator.as_deref()),
    }
}

/// Open a URL on a simulator. Unlike the install/launch lifecycle, this needs
/// no scheme or build — just a target simulator — so it resolves one directly
/// rather than going through the build plan.
fn open_url(ctx: &mut Context, url: &str, simulator: Option<&str>) -> CliResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, simulator)?;
    if !sim.is_booted() {
        simctl::boot(&sim.udid)?;
    }
    simctl::open_url(&sim.udid, url)?;
    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({
            "udid": sim.udid,
            "url": url,
        }));
    } else {
        ctx.out.note(&format!("opened {url} on {}", sim.label()));
    }
    Ok(())
}

/// Options for `app run`, gathered from the flags.
struct RunOpts<'a> {
    device: bool,
    device_id: Option<&'a str>,
    mac: bool,
    no_logs: bool,
}

/// Where the app runs.
enum Target {
    Simulator(String),
    Device(String),
    Mac,
    /// A Swift package executable, run on the host via `swift run <product>`.
    SpmRun(String),
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

fn run_app(ctx: &mut Context, opts: &RunOpts) -> CliResult {
    let plan = plan(ctx, opts)?;

    // A Swift package executable builds, runs, and streams in one `swift run`;
    // there's no separate log stream to background, so it stays a one-shot.
    if matches!(plan.target, Target::SpmRun(_)) {
        return deploy(ctx, &plan);
    }

    // --no-logs: deploy and return, no session.
    if opts.no_logs {
        return deploy(ctx, &plan);
    }

    // Default: deploy and follow logs. On a simulator at an interactive
    // terminal this is the rebuild session — logs stream in the background and
    // `r` rebuilds+relaunches on demand. Everything else (devices, macOS, and
    // non-interactive simulator runs) follows logs inline until Ctrl-C.
    match &plan.target {
        Target::Simulator(udid) => {
            if ctx.out.is_interactive() {
                return run_session(ctx, &plan, udid);
            }
            let app = build_and_install(&plan, &ctx.out)?;
            let out = simctl::launch(udid, &app.bundle_id)?;
            ctx.out
                .note(&format!("launched {} → {}", app.bundle_id, out.trim()));
            stream_logs(ctx, udid, &app)
        }
        Target::Device(id) => {
            // On device, logs come from launching with the console attached.
            let app = build_and_install(&plan, &ctx.out)?;
            ctx.out.note(&format!(
                "launching {} with console (Ctrl-C to stop)",
                app.bundle_id
            ));
            devicectl::launch_console(id, &app.bundle_id)
        }
        Target::Mac => {
            // A macOS app runs its executable directly; that streams its output.
            let app = build_and_install(&plan, &ctx.out)?;
            ctx.out
                .note(&format!("running {} (Ctrl-C to stop)", app.bundle_id));
            crate::cli::process::stream(&app.executable.to_string_lossy(), &[], None)
        }
        // SPM is handled by the early return above.
        Target::SpmRun(_) => unreachable!("SPM run handled before this match"),
    }
}

/// Resolve a full run plan, choosing a simulator (default), a device, or macOS.
fn plan(ctx: &mut Context, opts: &RunOpts) -> Result<RunPlan, CliError> {
    let resolved = resolve::resolve(ctx)?;
    let schemes = resolve::schemes(&resolved.container)?;
    let scheme = resolve::choose(ctx, "scheme", resolved.scheme.clone(), &schemes)?;
    let configuration = resolved
        .configuration
        .clone()
        .unwrap_or_else(|| "Debug".to_string());

    let (destination, target) = if matches!(resolved.container, resolve::Container::SwiftPackage(_))
    {
        if opts.device || opts.mac {
            return Err(CliError::new(
                "a Swift package executable runs on the host; --device/--mac don't apply",
            ));
        }
        // No xcodebuild destination — `swift run` builds and runs the product.
        (String::new(), Target::SpmRun(scheme.clone()))
    } else if opts.mac {
        ("platform=macOS".to_string(), Target::Mac)
    } else if opts.device {
        let devices = devicectl::list()?;
        let dev = if let Some(id) = opts.device_id {
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
        let platform = if dev.platform.is_empty() {
            "iOS"
        } else {
            &dev.platform
        };
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

    let plan = RunPlan {
        resolved,
        scheme,
        configuration,
        destination,
        target,
    };
    let bt = resolve::BuildTarget {
        scheme: plan.scheme.clone(),
        configuration: plan.configuration.clone(),
        destination: plan.destination.clone(),
    };
    resolve::remember(ctx, &plan.resolved, &bt);
    Ok(plan)
}

/// Build and install onto the target, returning the launchable app. Shared by
/// every flow; the launch step is chosen by the caller.
fn build_and_install(plan: &RunPlan, out: &Output) -> Result<AppBundle, CliError> {
    plan.build_plan().run(out)?;
    let app = plan.app_bundle()?;
    let app_path = app.path.display().to_string();
    match &plan.target {
        Target::Simulator(udid) => {
            simctl::boot(udid)?;
            simctl::install(udid, &app_path)?;
        }
        Target::Device(id) => {
            devicectl::install(id, &app_path)?;
        }
        // A macOS app is built in place; there's no install step.
        Target::Mac => {}
        // SPM executables never reach here (run_app routes them to `swift run`).
        Target::SpmRun(_) => unreachable!("SPM run does not build/install via xcodebuild"),
    }
    Ok(app)
}

/// `swift run <product>` in the package directory: builds and runs the
/// executable, streaming its output until it exits.
fn spm_run(ctx: &Context, plan: &RunPlan, product: &str) -> CliResult {
    let cwd = plan
        .resolved
        .container
        .path()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf);
    ctx.out.note(&format!("running {product} (swift run)"));
    crate::cli::process::stream("swift", &["run", product], cwd.as_deref())
}

/// Build, install, and launch (no log following) — used by `--no-logs` and SPM.
fn deploy(ctx: &Context, plan: &RunPlan) -> CliResult {
    // SPM executables build+run in one `swift run` step, not build+install+launch.
    if let Target::SpmRun(product) = &plan.target {
        return spm_run(ctx, plan, product);
    }

    let app = build_and_install(plan, &ctx.out)?;
    match &plan.target {
        Target::Simulator(udid) => {
            let out = simctl::launch(udid, &app.bundle_id)?;
            ctx.out
                .note(&format!("launched {} → {}", app.bundle_id, out.trim()));
        }
        Target::Device(id) => {
            let out = devicectl::launch(id, &app.bundle_id)?;
            ctx.out.note(&format!(
                "launched {} on device → {}",
                app.bundle_id,
                out.trim()
            ));
        }
        Target::Mac => {
            // Non-blocking launch (the logs/foreground path runs the executable).
            crate::cli::process::stream("open", &[&app.path.to_string_lossy()], None)?;
            ctx.out.note(&format!("launched {}", app.bundle_id));
        }
        // Handled by the early return above.
        Target::SpmRun(_) => {}
    }
    Ok(())
}

/// Interactive simulator session: build+install+launch, stream the app's logs
/// in the background, and rebuild+relaunch on demand. `r` rebuilds; `q`, Ctrl-C,
/// or Ctrl-D quit. Logs keep streaming throughout — raw mode flips only stdin's
/// line discipline, not the terminal's output handling (see [`rawmode`]).
fn run_session(ctx: &Context, plan: &RunPlan, udid: &str) -> CliResult {
    let app = build_and_install(plan, &ctx.out)?;
    let launched = simctl::launch(udid, &app.bundle_id)?;
    ctx.out
        .note(&format!("launched {} → {}", app.bundle_id, launched.trim()));

    // Raw mode needs a terminal on stdin; without one (piped input) fall back to
    // plain inline log following.
    let Ok(raw) = rawmode::RawMode::enable() else {
        return stream_logs(ctx, udid, &app);
    };

    let mut logs = Some(spawn_logs(udid, &app)?);
    session_hint(ctx);
    loop {
        match classify_key(raw.read_key()) {
            SessionKey::Rebuild => {
                stop_logs(&mut logs);
                ctx.out.note("rebuilding…");
                match build_and_relaunch(plan, udid, &ctx.out) {
                    Ok(app) => {
                        logs = Some(spawn_logs(udid, &app)?);
                        session_hint(ctx);
                    }
                    // Keep the session alive after a failed build — fix and press
                    // `r` again. No log stream runs until the next good build.
                    Err(e) => ctx.out.error(&e.to_string()),
                }
            }
            SessionKey::Quit => break,
            SessionKey::Ignore => {}
        }
    }
    stop_logs(&mut logs);
    Ok(())
}

/// Build, install, and relaunch on the simulator, returning the launchable app.
/// The rebuild unit re-run by the session's `r` key.
fn build_and_relaunch(plan: &RunPlan, udid: &str, out: &Output) -> Result<AppBundle, CliError> {
    let app = build_and_install(plan, out)?;
    let launched = simctl::launch(udid, &app.bundle_id)?;
    out.note(&format!("relaunched {} → {}", app.bundle_id, launched.trim()));
    Ok(app)
}

/// What the session does with a keystroke.
#[derive(Debug, PartialEq, Eq)]
enum SessionKey {
    Rebuild,
    Quit,
    Ignore,
}

/// Map a raw keystroke (or `None` for EOF) to a session action. `r` rebuilds;
/// `q`, Ctrl-C (`0x03`), Ctrl-D (`0x04`), and a closed stdin quit.
fn classify_key(key: Option<u8>) -> SessionKey {
    match key {
        Some(b'r' | b'R') => SessionKey::Rebuild,
        Some(b'q' | b'Q' | 0x03 | 0x04) | None => SessionKey::Quit,
        Some(_) => SessionKey::Ignore,
    }
}

fn session_hint(ctx: &Context) {
    ctx.out.note("press r to rebuild & relaunch · q to quit");
}

/// Spawn the simulator log stream as a background child (best-effort predicate
/// on the app's executable name), inheriting stdout so logs appear live.
fn spawn_logs(udid: &str, app: &AppBundle) -> Result<Child, CliError> {
    let name = app
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let predicate = format!("processImagePath CONTAINS \"{name}\"");
    crate::cli::process::spawn(
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

/// Kill and reap the background log stream, if one is running.
fn stop_logs(logs: &mut Option<Child>) {
    if let Some(mut child) = logs.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
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
    let opts = RunOpts {
        device: false,
        device_id: None,
        mac: false,
        no_logs: true,
    };
    let plan = plan(ctx, &opts)?;
    let Target::Simulator(udid) = &plan.target else {
        return Err(CliError::new(
            "app install/launch/logs/stop are only supported for simulator targets",
        ));
    };
    let app = plan.app_bundle()?;

    match stage {
        Stage::Install => {
            plan.build_plan().run(&ctx.out)?;
            simctl::boot(udid)?;
            simctl::install(udid, &app.path.display().to_string())?;
            ctx.out.note(&format!("installed {}", app.bundle_id));
        }
        Stage::Launch => {
            simctl::boot(udid)?;
            let out = simctl::launch(udid, &app.bundle_id)?;
            ctx.out
                .note(&format!("launched {} → {}", app.bundle_id, out.trim()));
        }
        Stage::Logs => return stream_logs(ctx, udid, &app),
        Stage::Stop => {
            simctl::terminate(udid, &app.bundle_id)?;
            ctx.out.note(&format!("terminated {}", app.bundle_id));
        }
    }
    Ok(())
}

/// Follow a simulator's log for the app inline until Ctrl-C — the non-interactive
/// fallback (the interactive session backgrounds the same stream via [`spawn_logs`]).
fn stream_logs(ctx: &Context, udid: &str, app: &AppBundle) -> CliResult {
    ctx.out.note(&format!(
        "streaming logs for {} (Ctrl-C to stop)",
        app.bundle_id
    ));
    let mut child = spawn_logs(udid, app)?;
    match child.wait() {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(CliError::new("log stream exited with a non-zero status")),
        Err(e) => Err(CliError::new(format!("failed to wait for log stream: {e}"))),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udid_extracted_from_destination() {
        assert_eq!(udid("platform=iOS Simulator,id=ABCD").unwrap(), "ABCD");
        assert_eq!(udid("id=XYZ,platform=iOS Simulator").unwrap(), "XYZ");
        assert!(udid("platform=iOS Simulator,name=iPhone 15").is_err());
    }

    #[test]
    fn session_keys_map_to_actions() {
        // `r` rebuilds (either case).
        assert_eq!(classify_key(Some(b'r')), SessionKey::Rebuild);
        assert_eq!(classify_key(Some(b'R')), SessionKey::Rebuild);
        // `q`, Ctrl-C, Ctrl-D, and EOF all quit.
        assert_eq!(classify_key(Some(b'q')), SessionKey::Quit);
        assert_eq!(classify_key(Some(b'Q')), SessionKey::Quit);
        assert_eq!(classify_key(Some(0x03)), SessionKey::Quit);
        assert_eq!(classify_key(Some(0x04)), SessionKey::Quit);
        assert_eq!(classify_key(None), SessionKey::Quit);
        // Anything else is ignored — the session keeps streaming logs.
        assert_eq!(classify_key(Some(b'x')), SessionKey::Ignore);
        assert_eq!(classify_key(Some(b'\n')), SessionKey::Ignore);
    }
}
