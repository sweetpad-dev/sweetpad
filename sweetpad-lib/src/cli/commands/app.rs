//! `sweetpad app …` — the built app's lifecycle: build+install+launch, and the
//! running session, on a simulator or a physical device. The app is the noun;
//! these are its actions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::resolve::{self, Resolved};
use crate::cli::xcodebuild::{self, AppBundle};
use crate::cli::{CliError, CliResult, Context, devicectl, simctl};

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
        /// Build and run as a native macOS app (launches the executable).
        #[arg(long, conflicts_with_all = ["device", "device_id"])]
        mac: bool,
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
            watch,
            no_logs,
        } => run_app(
            ctx,
            &RunOpts {
                device: *device || device_id.is_some(),
                device_id: device_id.as_deref(),
                mac: *mac,
                watch: *watch,
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
#[allow(clippy::struct_excessive_bools)]
struct RunOpts<'a> {
    device: bool,
    device_id: Option<&'a str>,
    mac: bool,
    watch: bool,
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

    // A Swift package executable runs (and streams) via `swift run`; deploy
    // does exactly that, so every mode routes through it.
    if matches!(plan.target, Target::SpmRun(_)) {
        if opts.watch {
            deploy(ctx, &plan)?;
            return watch_loop(ctx, &plan);
        }
        return deploy(ctx, &plan);
    }

    // --watch: deploy once, then redeploy on change (no inline logs — they'd
    // block the loop). --no-logs: deploy and return.
    if opts.watch {
        deploy(ctx, &plan)?;
        return watch_loop(ctx, &plan);
    }
    if opts.no_logs {
        return deploy(ctx, &plan);
    }

    // Default: deploy and follow logs inline until Ctrl-C.
    match &plan.target {
        Target::Simulator(udid) => {
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

/// Build, install, and launch (no log following) — the unit re-run by `--watch`.
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

/// Poll the project's source tree and redeploy on change.
fn watch_loop(ctx: &Context, plan: &RunPlan) -> CliResult {
    // The container's parent, or "." when it's a relative path (empty parent).
    let root = plan
        .resolved
        .container
        .path()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
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
            if name.starts_with('.')
                || matches!(name.as_ref(), "DerivedData" | "build" | ".build" | "Pods")
            {
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
    let opts = RunOpts {
        device: false,
        device_id: None,
        mac: false,
        watch: false,
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

/// Stream a simulator's log for the app's executable (best-effort predicate).
fn stream_logs(ctx: &Context, udid: &str, app: &AppBundle) -> CliResult {
    let name = app
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let predicate = format!("processImagePath CONTAINS \"{name}\"");
    ctx.out.note(&format!(
        "streaming logs for {} (Ctrl-C to stop)",
        app.bundle_id
    ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn udid_extracted_from_destination() {
        assert_eq!(udid("platform=iOS Simulator,id=ABCD").unwrap(), "ABCD");
        assert_eq!(udid("id=XYZ,platform=iOS Simulator").unwrap(), "XYZ");
        assert!(udid("platform=iOS Simulator,name=iPhone 15").is_err());
    }

    #[test]
    fn scan_sources_picks_swift_and_skips_build_dirs() {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sweetpad-scan-{n}"));
        std::fs::create_dir_all(root.join("Sources")).unwrap();
        std::fs::create_dir_all(root.join("Pods")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("Sources/A.swift"), "a").unwrap();
        std::fs::write(root.join("README.md"), "x").unwrap();
        std::fs::write(root.join("Pods/B.swift"), "b").unwrap();
        std::fs::write(root.join(".git/C.swift"), "c").unwrap();

        let snap = scan_sources(&root);
        let names: Vec<String> = snap
            .keys()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // Only the real source file; non-swift, Pods, and .git are excluded.
        assert_eq!(names, vec!["A.swift"]);

        // Adding a new source file changes the snapshot.
        std::fs::write(root.join("Sources/D.swift"), "d").unwrap();
        assert_ne!(scan_sources(&root), snap);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
