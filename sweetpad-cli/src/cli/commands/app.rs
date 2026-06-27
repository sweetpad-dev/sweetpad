//! `sweetpad app …` — the built app's lifecycle: build+install+launch, and the
//! running session, on a simulator or a physical device. The app is the noun;
//! these are its actions.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Child;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::time::Instant;

use clap::Subcommand;

use crate::cli::inject::recompiler::{Mode, Recompiler};
use crate::cli::inject::server::{InjectServer, Logger};
use crate::cli::inject::{self, HotSession};
use crate::cli::output::Output;
use crate::cli::resolve::{self, Resolved};
use crate::cli::state::LastLaunchedApp;
use crate::cli::xcodebuild::{self, AppBundle};
use crate::cli::{
    CliError, CliResult, CommandResult, Context, ErrorContext, ErrorKind, Render, Rendered,
    buildlog, devicectl, oslog, process, pymobiledevice3, rawmode, simctl,
};
use sweetpad_core::build_settings::BuildSettingsOptions;

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
        /// Enable hot reload (iOS Simulator only): on each Swift save the file is
        /// recompiled and injected into the running app — no relaunch, state
        /// preserved. Requires the injection client (see CLI_DESIGN §9d).
        #[arg(long)]
        hot: bool,
        /// Hot-reload recompiler: `resolver` (default — robust whole-module via
        /// the build-settings resolver) or `buildlog` (fast single-file recovered
        /// from the build transcript).
        #[arg(long = "hot-recompiler", value_name = "MODE")]
        hot_recompiler: Option<String>,
        /// CI self-check (hidden): with `--hot`, after launch edit FILE once, wait
        /// for `.injected`, and exit 0/1 instead of entering the session. Drives
        /// the end-to-end hot-reload/injection test.
        #[arg(
            long = "hot-selfcheck",
            value_name = "FILE",
            hide = true,
            requires = "hot"
        )]
        hot_selfcheck: Option<std::path::PathBuf>,
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

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Run {
            device,
            device_id,
            mac,
            no_logs,
            hot,
            hot_recompiler,
            hot_selfcheck,
        } => {
            let hot_mode = match hot_recompiler.as_deref() {
                None => Mode::Resolver,
                Some(s) => Mode::parse(s).ok_or_else(|| {
                    CliError::new(format!(
                        "unknown --hot-recompiler {s:?} (use resolver|buildlog)"
                    ))
                })?,
            };
            // The live build-and-run session streams its own output until you quit.
            run_app(
                ctx,
                &RunOpts {
                    device: *device || device_id.is_some(),
                    device_id: device_id.as_deref(),
                    mac: *mac,
                    no_logs: *no_logs,
                    hot: *hot,
                    hot_mode,
                    hot_selfcheck: hot_selfcheck.as_deref(),
                },
            )
            .map(|()| Rendered::Streamed)
        }
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
fn open_url(ctx: &mut Context, url: &str, simulator: Option<&str>) -> CommandResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, simulator)?;
    if !sim.is_booted() {
        ctx.out
            .step("Booting simulator", || simctl::boot(&sim.udid))?;
    }
    simctl::open_url(&sim.udid, url)?;
    Ok(Rendered::data(OpenUrlReport {
        udid: sim.udid.clone(),
        url: url.to_string(),
        label: sim.label(),
    }))
}

/// The result of `app open-url`: a confirmation note in human mode, or
/// `{ udid, url }` in the JSON envelope.
struct OpenUrlReport {
    udid: String,
    url: String,
    label: String,
}

impl Render for OpenUrlReport {
    fn human(&self, out: &Output) {
        out.note(&format!("Opened {} on {}", self.url, self.label));
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({ "udid": self.udid, "url": self.url })
    }
}

/// Options for `app run`, gathered from the flags.
#[allow(clippy::struct_excessive_bools)] // independent CLI toggles, not a state machine
struct RunOpts<'a> {
    device: bool,
    device_id: Option<&'a str>,
    mac: bool,
    no_logs: bool,
    hot: bool,
    hot_mode: Mode,
    hot_selfcheck: Option<&'a Path>,
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
    /// Build with the hot-reload flags (`-interposable` + frontend command lines).
    hot: bool,
}

impl RunPlan {
    fn build_plan(&self) -> xcodebuild::BuildPlan<'_> {
        xcodebuild::BuildPlan {
            container: &self.resolved.container,
            scheme: &self.scheme,
            configuration: &self.configuration,
            destination: Some(&self.destination),
            clean: false,
            hot: self.hot,
        }
    }

    /// Locate the built `.app` via the in-process build-settings resolver (the
    /// engine behind `settings show`), with no xcodebuild spawn. It computes the
    /// same TARGET_BUILD_DIR/product the build produced. Swift packages never
    /// reach here — they run via `swift run`, not a build/install/launch.
    fn app_bundle(&self) -> Result<AppBundle, CliError> {
        let (project, workspace) = match &self.resolved.container {
            resolve::Container::Project(p) => (Some(p.clone()), None),
            resolve::Container::Workspace(p) => (None, Some(p.clone())),
            resolve::Container::SwiftPackage(_) => {
                return Err(CliError::new("Swift packages have no .app bundle"));
            }
        };
        let opts = BuildSettingsOptions {
            project,
            workspace,
            scheme: Some(self.scheme.clone()),
            target: None,
            configuration: self.configuration.clone(),
            sdk: String::new(),
            arch: String::new(),
            destination: sweetpad_lib::destination::parse_destination_arg(&self.destination),
            xcconfig: None,
            xcode: None,
            xcspec_root: None,
            sdksettings_root: None,
            catalog_cache: None,
            derived_data_path: None,
            keys: None,
        };
        let resolved =
            sweetpad_core::build_settings::resolve_build_settings(&opts).map_err(CliError::new)?;
        let settings: Vec<xcodebuild::TargetBuildSettings> = resolved
            .into_iter()
            .map(|t| xcodebuild::TargetBuildSettings {
                target: t.target,
                settings: t.settings,
            })
            .collect();
        xcodebuild::app_bundle(&settings)
    }
}

fn run_app(ctx: &mut Context, opts: &RunOpts) -> CliResult {
    // `app run` is a live build-and-run session that streams logs until you quit —
    // there's no coherent one-shot JSON for it (a `--json` run would emit a silent
    // build and then human-formatted logs). Fail fast; build and launch as separate
    // steps if you need machine-readable output.
    if ctx.out.is_json() {
        return Err(CliError::new(
            "`app run` streams a live session and does not support --json; \
             build and launch as separate steps for machine-readable output",
        ));
    }

    let plan = plan(ctx, opts)?;
    print_summary(ctx, &plan);

    // Bring the Simulator window up so the running app is visible. Best-effort and
    // once per run — rebuilds reuse the same window, and only a simulator has a UI
    // to reveal (devices and macOS don't).
    if matches!(plan.target, Target::Simulator(_)) {
        let _ = simctl::open_app();
    }

    let result = if opts.hot {
        // Hot reload owns its own build + launch + watch session (simulator only).
        run_hot_session(ctx, &plan, opts.hot_mode, opts.hot_selfcheck)
    } else if matches!(plan.target, Target::SpmRun(_)) {
        // A Swift package executable builds, runs, and streams in one `swift run`;
        // there's no separate log stream to background, so it stays a one-shot.
        deploy(ctx, &plan)
    } else if opts.no_logs {
        // --no-logs: deploy and return, no session.
        deploy(ctx, &plan)
    } else if ctx.out.is_interactive() {
        // The interactive rebuild session: output streams in the background and
        // `r` rebuilds+relaunches on demand.
        run_session(ctx, &plan)
    } else {
        // Non-interactive (CI/piped): one-shot launch + inline follow until Ctrl-C.
        follow_once(ctx, &plan)
    };

    // Record the launch only once it actually happened: an `Ok` result means the
    // app built and launched, so the state never advertises a `last launched`
    // bundle that a failed/aborted build never produced. Best-effort.
    if result.is_ok() {
        record_last_launched(ctx, &plan);
    }
    result
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
        // Scheme and configuration are already settled above; resolve only the
        // destination here so the scheme picker doesn't run a second time.
        let destination = match resolved.destination.clone() {
            Some(d) => d,
            None => resolve::pick_destination(ctx, &resolved.container.key(), &simctl::list()?)?,
        };
        let udid = udid(&destination)?;
        (destination, Target::Simulator(udid))
    };

    let plan = RunPlan {
        resolved,
        scheme,
        configuration,
        destination,
        target,
        hot: opts.hot,
    };
    let bt = resolve::BuildTarget {
        scheme: plan.scheme.clone(),
        configuration: plan.configuration.clone(),
        destination: plan.destination.clone(),
    };
    resolve::remember(ctx, &plan.resolved, &bt);
    Ok(plan)
}

/// A simulator boot kicked off on a background thread so it comes up *while* the
/// project builds, rather than serializing boot-after-build. [`wait`](BgBoot::wait)
/// joins it at the point install/launch needs the device — so the device is up by
/// then, but the boot's seconds overlapped the build instead of adding to it. Every
/// caller must `wait` before any further `simctl boot` on the same device, so the two
/// never run concurrently. A no-op for device/macOS targets (nothing to boot).
struct BgBoot {
    handle: Option<std::thread::JoinHandle<Result<(), CliError>>>,
}

impl BgBoot {
    /// Spawn the boot for a simulator target; do nothing for any other target.
    fn start(target: &Target) -> Self {
        let handle = if let Target::Simulator(udid) = target {
            let udid = udid.clone();
            Some(std::thread::spawn(move || simctl::boot(&udid)))
        } else {
            None
        };
        BgBoot { handle }
    }

    /// Join the background boot, surfacing its result (`Ok` if there was none, e.g. a
    /// non-simulator target or an already-joined handle).
    fn wait(&mut self) -> Result<(), CliError> {
        match self.handle.take() {
            Some(h) => h
                .join()
                .unwrap_or_else(|_| Err(CliError::new("simulator boot thread panicked"))),
            None => Ok(()),
        }
    }
}

/// Build and install onto the target, returning the launchable app. Shared by
/// every flow; the launch step is chosen by the caller.
fn build_and_install(plan: &RunPlan, out: &Output) -> Result<AppBundle, CliError> {
    // Boot the simulator while the build runs; joined at the boot step below so it's
    // ready for install without the boot serializing after the build.
    let mut boot = BgBoot::start(&plan.target);
    plan.build_plan().run(out)?;
    let app = plan.app_bundle()?;
    let app_path = app.path.display().to_string();
    match &plan.target {
        Target::Simulator(udid) => {
            out.step("Booting simulator", || boot.wait())?;
            out.step("Installing app", || simctl::install(udid, &app_path))?;
        }
        Target::Device(id) => {
            out.step("Installing app on device", || {
                devicectl::install(id, &app_path)
            })?;
        }
        // A macOS app is built in place; there's no install step.
        Target::Mac => {}
        // SPM executables never reach here (run_app routes them to `swift run`).
        Target::SpmRun(_) => unreachable!("SPM run does not build/install via xcodebuild"),
    }
    Ok(app)
}

/// Record the app being launched into the project's state — for re-launch and
/// `context show`, mirroring the extension's `lastLaunchedApp`. Best-effort: a
/// missing bundle or write failure never derails a run. SPM executables have no
/// `.app`, so they're skipped. Captures the intended launch (the bundle the
/// resolver says the build produces), so it reflects `app run`, not `install`.
fn record_last_launched(ctx: &mut Context, plan: &RunPlan) {
    let (kind, simulator_udid, destination_id, destination_type) = match &plan.target {
        Target::Simulator(udid) => ("simulator", Some(udid.clone()), None, None),
        Target::Device(id) => (
            "device",
            None,
            Some(id.clone()),
            destination_platform(&plan.destination),
        ),
        Target::Mac => ("macos", None, None, None),
        Target::SpmRun(_) => return,
    };
    let Ok(app) = plan.app_bundle() else {
        return;
    };
    let file_name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    let last = LastLaunchedApp {
        kind: kind.to_string(),
        app_path: app.path.display().to_string(),
        bundle_identifier: app.bundle_id,
        app_name: file_name(&app.path),
        executable_name: file_name(&app.executable),
        simulator_udid,
        destination_id,
        destination_type,
    };
    let key = plan.resolved.container.key();
    ctx.state.project_mut(&key).last_launched_app = Some(last);
    let _ = ctx.state.save();
}

/// The `platform=` value from a `-destination` specifier, e.g. `iOS`.
fn destination_platform(spec: &str) -> Option<String> {
    spec.split(',')
        .find_map(|kv| kv.trim().strip_prefix("platform="))
        .map(str::to_string)
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
    ctx.out.note(&format!("Running {product} (swift run)"));
    crate::cli::process::stream("swift", &["run", product], cwd.as_deref())
        .context("running the package executable")
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
            let out = ctx
                .out
                .step("Launching app", || simctl::launch(udid, &app.bundle_id))?;
            ctx.out
                .note(&format!("Launched {} → {}", app.bundle_id, out.trim()));
        }
        Target::Device(id) => {
            let out = ctx.out.step("Launching app on device", || {
                devicectl::launch(id, &app.bundle_id)
            })?;
            ctx.out.note(&format!(
                "Launched {} on device → {}",
                app.bundle_id,
                out.trim()
            ));
        }
        Target::Mac => {
            // Non-blocking launch (the logs/foreground path runs the executable).
            crate::cli::process::stream("open", &[&app.path.to_string_lossy()], None)
                .context("launching the macOS app")?;
            ctx.out.note(&format!("Launched {}", app.bundle_id));
        }
        // Handled by the early return above.
        Target::SpmRun(_) => {}
    }
    Ok(())
}

/// Interactive rebuild session: build + launch + stream the app's output, then
/// rebuild + relaunch on demand. `r` rebuilds; `q`, Ctrl-C, or Ctrl-D quit. Raw
/// mode flips only stdin's line discipline (see [`rawmode`]) so output keeps
/// streaming. Ctrl-C while a build is running cancels the build *and* the session;
/// a failed build keeps the session open to retry. The running app is terminated
/// before each relaunch and on quit.
fn run_session(ctx: &Context, plan: &RunPlan) -> CliResult {
    // Raw mode needs a terminal on stdin; without one (piped input) fall back to
    // a one-shot launch + inline follow.
    let Ok(_raw) = rawmode::RawMode::enable() else {
        return follow_once(ctx, plan);
    };

    // Live log filter: the stream carries every level; show those at or above this
    // threshold, set live by the 1/2/3 keys.
    let filter = Arc::new(AtomicU8::new(default_filter(&ctx.out).threshold()));
    // Boot the simulator on a background thread so it comes up while the project
    // builds. Joined below before install — or, on a failed build, before the log
    // stream so it attaches to a booted device instead of failing with "device is
    // not booted". A no-op for device/macOS targets.
    let mut boot = BgBoot::start(&plan.target);
    // Build + launch. A failure keeps the session (nothing running) so you can fix
    // the error and press `r`, instead of being dropped back to the shell.
    let started = Instant::now();
    let mut ever_launched = false;
    let mut running = match build(plan, &ctx.out, None) {
        BuildOutcome::Ok => {
            // Finish the background boot before installing; start_app's own boot then
            // confirms it (a fast no-op now the device is already up).
            let _ = boot.wait();
            match start_app(ctx, plan, &filter) {
                Ok(r) => {
                    note_launch(ctx, "Launched", started);
                    ever_launched = true;
                    Some(r)
                }
                Err(e) => {
                    ctx.out.error(&e);
                    None
                }
            }
        }
        BuildOutcome::Failed(e) => {
            ctx.out.error(&e);
            // Nothing launched, but the session stays open to fix and rebuild. Finish
            // the boot so the log stream ([`start_logs`]) attaches to a booted device
            // and it's ready for the next `r`. Best-effort.
            let _ = boot.wait();
            None
        }
        // Ctrl-C during the build cancels the whole run, not just the build.
        BuildOutcome::Aborted => return Ok(()),
    };
    // The log stream is session-scoped: started once and kept across rebuilds (its
    // name-based predicate follows the relaunched app), so rebuilds never tear it
    // down. Dropped on exit.
    let logs = start_logs(ctx, plan, &filter);
    // The level keys are meaningful only when there's an os_log stream to filter
    // (the simulator, a macOS app, or a device with pymobiledevice3) — not a device
    // on its raw console.
    let filterable = logs.is_some();
    session_hint(ctx, filterable);

    loop {
        match rawmode::poll_key() {
            rawmode::Input::Key(ch) => match classify_key(ch) {
                SessionKey::Rebuild => match do_rebuild(ctx, plan, &mut running, &filter) {
                    RebuildOutcome::Continue { launched } => {
                        ever_launched |= launched;
                        session_hint(ctx, filterable);
                    }
                    // Ctrl-C during the rebuild cancels the whole run.
                    RebuildOutcome::Quit => {
                        if let Some(r) = running.take() {
                            terminate_app(r);
                        }
                        return Ok(());
                    }
                },
                SessionKey::Quit => break,
                // Inert unless an os_log stream is actually being filtered (see
                // `filterable`).
                SessionKey::Filter(level) => {
                    if filterable {
                        set_filter(ctx, &filter, level);
                    }
                }
                SessionKey::Ignore => {}
            },
            rawmode::Input::Idle => {}
            rawmode::Input::Closed => break,
        }
        // Notice if the app crashed/exited, so the logs going quiet isn't a mystery.
        if let Some(r) = running.as_mut() {
            check_exit(ctx, r);
        }
    }
    if let Some(r) = running.take() {
        terminate_app(r);
    }
    // A session that never produced a running app (the build kept failing) exits
    // non-zero, so a script or wrapper around `app run` sees the failure even
    // though the session stayed open for you to retry.
    if ever_launched {
        Ok(())
    } else {
        Err(CliError::new(
            "app run ended without a successful build — nothing was launched",
        ))
    }
}

/// `app run --hot` — the built-in hot-reload session (iOS Simulator only).
///
/// Builds with the interposable / frontend-command flags, starts the injection
/// server on `:8887`, launches the app with the client dylib injected, then
/// watches the workspace: each Swift save is recompiled and `.load`-ed into the
/// running app — no relaunch, state preserved. `r` still does a full
/// rebuild+relaunch (the client reconnects); `q`/Ctrl-C/Ctrl-D quit.
/// The hot-session status logger: bold magenta (when color is on) so the lines stand
/// out from the streamed os_log. One save stays on one line — an in-progress message
/// (ends with `…`) is drawn in place (carriage-return + clear-line, no newline) so the
/// outcome overwrites it; any other line commits with a newline and stays in the
/// scrollback. Without color (non-TTY) every line is a plain committed line.
#[allow(clippy::print_stdout)] // live hot-reload status line, drawn in place
fn hot_logger(color: bool) -> Logger {
    use std::io::Write as _;
    Arc::new(move |m: &str| {
        if !color {
            println!("{m}");
        } else if m.ends_with('…') {
            print!("\r\x1b[2K\x1b[1;35m{m}\x1b[0m");
            let _ = std::io::stdout().flush();
        } else {
            println!("\r\x1b[2K\x1b[1;35m{m}\x1b[0m");
        }
    })
}

fn run_hot_session(
    ctx: &Context,
    plan: &RunPlan,
    mode: Mode,
    selfcheck: Option<&Path>,
) -> CliResult {
    let Target::Simulator(udid) = &plan.target else {
        return Err(CliError::new(
            "--hot is only supported for the iOS Simulator (not devices, macOS, or SPM)",
        ));
    };
    let sdk = inject::sdk_for_destination(&plan.destination).ok_or_else(|| {
        CliError::new(format!(
            "--hot needs a simulator destination; got {:?}",
            plan.destination
        ))
    })?;

    let developer_dir = process::capture("xcode-select", &["-p"], None)?
        .trim()
        .to_string();
    let project_root = plan
        .resolved
        .container
        .path()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| std::path::PathBuf::from("."), Path::to_path_buf);

    // Per-session scratch dir for the recompiler's objects/dylibs + build log.
    let work = std::env::temp_dir().join(format!("sweetpad-hot-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&work);
    let build_log = work.join("build.log");

    // Boot the simulator on a background thread so it comes up while the project
    // builds (and the injection server/dylib are resolved); joined before launch_hot.
    let mut boot = BgBoot::start(&plan.target);

    // Build first (capturing the transcript for the build-log recompiler).
    ctx.out.note(&format!(
        "hot reload: building {} ({}) for {} [{}]",
        plan.scheme,
        plan.configuration,
        plan.destination,
        match mode {
            Mode::Resolver => "resolver",
            Mode::BuildLog => "build-log",
        }
    ));
    match build(plan, &ctx.out, Some(&build_log)) {
        BuildOutcome::Ok => {}
        BuildOutcome::Failed(e) => return Err(e),
        // Ctrl-C during the build cancels the hot session before it starts.
        BuildOutcome::Aborted => return Ok(()),
    }
    let app = plan.app_bundle()?;

    // Resolve the injection client dylib + the SIMCTL_CHILD_* launch env.
    // `SWEETPAD_HOTRELOAD_DYLIB` overrides the lookup (used by CI to point at a
    // downloaded client matching the active Xcode).
    let client_opts = inject::client::ClientOptions {
        developer_dir: developer_dir.clone(),
        sdk: sdk.to_string(),
        project_root: project_root.clone(),
        override_path: std::env::var_os("SWEETPAD_HOTRELOAD_DYLIB").map(std::path::PathBuf::from),
    };
    let dylib = inject::client::resolve_dylib(&client_opts, &|msg| ctx.out.note(msg))
        .map_err(CliError::new)?;
    let launch_env = inject::client::launch_env(&dylib, &client_opts);
    ctx.out
        .note(&format!("hot reload: injecting {}", dylib.display()));

    // The recompiler + injection server (server must listen before launch).
    let recompiler = Arc::new(Recompiler::new(
        mode,
        &plan.resolved.container,
        plan.scheme.clone(),
        plan.configuration.clone(),
        sdk.to_string(),
        inject::host_arch(),
        developer_dir,
        Some(build_log),
        work,
    ));
    let log = hot_logger(ctx.out.use_color());
    let server =
        Arc::new(InjectServer::start(recompiler, Arc::clone(&log)).map_err(CliError::new)?);

    // SwiftUI views need the Inject package to redraw on injection; warn once
    // if it's absent (UIKit apps don't need it, so this is advisory only).
    if inject::inject_dependency_present(&project_root) == Some(false) {
        ctx.out.note(
            "hot reload: the `Inject` package isn't in Package.resolved — SwiftUI views \
             won't redraw on save until you add https://github.com/krzysztofzablocki/Inject \
             and annotate them with @ObserveInjection + .enableInjection() (UIKit apps can ignore this)",
        );
    }

    // Install + launch with the client injected, then start the session log
    // stream (kept across `r` relaunches; its predicate follows the app by name).
    // Finish the background boot first; launch_hot's own boot then confirms it.
    let _ = boot.wait();
    launch_hot(ctx, udid, &app, &launch_env)?;
    // Hot reload has no live filter UI; use the default threshold, never cycled.
    let filter = Arc::new(AtomicU8::new(default_filter(&ctx.out).threshold()));
    let mut logs = start_logs(ctx, plan, &filter);
    // Watch the workspace; each save drives `server.inject`.
    let session = HotSession::start(Arc::clone(&server), &project_root);

    // CI self-check: edit a file once, assert `.injected`, exit. Otherwise the
    // interactive key loop (`r`/`q`), or — non-TTY — follow logs until Ctrl-C.
    let outcome = if let Some(file) = selfcheck {
        hot_selfcheck(ctx, &server, file, udid)
    } else if ctx.out.is_interactive() {
        hot_key_loop(ctx, plan, udid, &launch_env, &mut logs);
        Ok(())
    } else {
        ctx.out
            .note("hot reload: watching for Swift changes (Ctrl-C to stop)");
        if let Some(logs) = logs.as_mut() {
            logs.wait();
        }
        Ok(())
    };

    // Teardown: stop watcher + server, terminate the app, kill the log stream.
    session.shutdown();
    server.shutdown();
    let _ = simctl::terminate(udid, &app.bundle_id);
    drop(logs);
    let _ = std::fs::remove_dir_all(
        std::env::temp_dir().join(format!("sweetpad-hot-{}", std::process::id())),
    );
    outcome
}

/// Marker token in the hot-reload fixture's `ContentView.swift` that the
/// self-check rewrites to a unique nonce. See `ci/fixture-app`.
const SELFCHECK_MARKER: &str = "SWEETPAD_MARKER_ORIGINAL";

/// CI self-check: wait for the client to connect, rewrite the fixture's marker to
/// a unique nonce (driving the watcher → recompile → `.load`), assert `.injected`,
/// then confirm the running app logged the **new** nonce — proving the injected
/// code actually ran, not merely that the patch was accepted. A hard pass/fail
/// end-to-end test for `app run --hot --hot-selfcheck FILE`.
fn hot_selfcheck(ctx: &Context, server: &Arc<InjectServer>, file: &Path, udid: &str) -> CliResult {
    use std::time::Duration;

    ctx.out
        .note("hot reload self-check: waiting for the app to connect…");
    if !server.wait_connected(Duration::from_secs(30)) {
        return Err(CliError::new(
            "hot reload self-check: the in-app client never connected to :8887",
        ));
    }
    let baseline = server.result_counts();

    // Rewrite the marker to a unique nonce: a real behavioral change (the
    // interposed `sweetpadHotReloadMarker()` returns the nonce) that the fixture
    // logs on the injection notification.
    let original = std::fs::read_to_string(file)
        .map_err(|e| CliError::new(format!("self-check: read {}: {e}", file.display())))?;
    if !original.contains(SELFCHECK_MARKER) {
        return Err(CliError::new(format!(
            "self-check: {} has no `{SELFCHECK_MARKER}` marker (expected the hot-reload fixture)",
            file.display()
        )));
    }
    let nonce = format!("SWEETPAD_NONCE_{}", std::process::id());
    std::fs::write(file, original.replace(SELFCHECK_MARKER, &nonce))
        .map_err(|e| CliError::new(format!("self-check: write {}: {e}", file.display())))?;
    ctx.out
        .note(&format!("hot reload self-check: edited {}", file.display()));

    // The first inject is the slowest: the resolver primes its frontend-command
    // cache with a whole-module `swiftc -###` dry-run before compiling + linking.
    // Be generous so a slow/contended CI runner doesn't flake (the real watcher
    // loop has no such deadline — this bound only guards the self-check).
    let result = server.wait_for_result(baseline, Duration::from_secs(180));
    // Restore the fixture regardless of outcome.
    let _ = std::fs::write(file, &original);

    match result {
        Some(true) => ctx.out.note("hot reload self-check: ✅ .injected"),
        Some(false) => return Err(CliError::new("hot reload self-check: ❌ injection failed")),
        None => {
            return Err(CliError::new(
                "hot reload self-check: ❌ timed out waiting for .injected",
            ));
        }
    }

    // Behavioral check: the app must have logged the new nonce, proving the
    // injected code executed (not just that the client accepted the patch).
    ctx.out
        .note("hot reload self-check: confirming the new code ran…");
    if app_logged_marker(udid, &nonce, Duration::from_secs(20)) {
        ctx.out
            .note("hot reload self-check: ✅ new code ran (marker observed in the app log)");
        Ok(())
    } else {
        Err(CliError::new(
            "hot reload self-check: ❌ injected, but the app never logged the new marker \
             (the patch was accepted but the new code did not run)",
        ))
    }
}

/// Poll the simulator's unified log for `nonce` (emitted by the fixture's
/// injection observer via `os_log`), returning true once it appears or false
/// after `timeout`.
fn app_logged_marker(udid: &str, nonce: &str, timeout: std::time::Duration) -> bool {
    use std::time::{Duration, Instant};
    let predicate = format!("eventMessage CONTAINS \"{nonce}\"");
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let shown = process::capture(
            "xcrun",
            &[
                "simctl",
                "spawn",
                udid,
                "log",
                "show",
                "--last",
                "1m",
                "--style",
                "compact",
                "--predicate",
                &predicate,
            ],
            None,
        );
        if shown.is_ok_and(|out| out.contains(nonce)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1500));
    }
    false
}

/// Boot, install, and launch the app with the hot-reload env. Shared by the
/// first launch and each `r`. Logs stream separately for the whole session
/// ([`start_logs`]), so this doesn't touch them.
fn launch_hot(ctx: &Context, udid: &str, app: &AppBundle, env: &[(String, String)]) -> CliResult {
    ctx.out.step("Booting simulator", || simctl::boot(udid))?;
    ctx.out.step("Installing app", || {
        simctl::install(udid, &app.path.display().to_string())
    })?;
    let launched = ctx.out.step("Launching app", || {
        simctl::launch_with_env(udid, &app.bundle_id, env)
    })?;
    ctx.out
        .note(&format!("Launched {} → {}", app.bundle_id, launched.trim()));
    Ok(())
}

/// The `--hot` keypress loop: `r` full rebuild+relaunch (the client reconnects),
/// `q`/Ctrl-C/Ctrl-D quit. Injection happens out-of-band via the watcher.
fn hot_key_loop(
    ctx: &Context,
    plan: &RunPlan,
    udid: &str,
    env: &[(String, String)],
    logs: &mut Option<LogStream>,
) {
    let Ok(_raw) = rawmode::RawMode::enable() else {
        // No TTY for raw mode — just follow the log stream until Ctrl-C.
        if let Some(logs) = logs.as_mut() {
            logs.wait();
        }
        return;
    };
    ctx.out
        .note("hot reload ready · edit a Swift file to inject · r rebuilds · q quits");
    loop {
        match rawmode::poll_key() {
            rawmode::Input::Key(key) => match classify_key(key) {
                SessionKey::Rebuild => {
                    ctx.out.note("»  Full rebuild — relaunching…");
                    let app = match plan.app_bundle() {
                        Ok(a) => a,
                        Err(e) => {
                            ctx.out.error(&e);
                            continue;
                        }
                    };
                    // The session log stream follows the app by name, so it's
                    // left running — just terminate, rebuild, and relaunch.
                    let _ = simctl::terminate(udid, &app.bundle_id);
                    match build(plan, &ctx.out, None) {
                        BuildOutcome::Ok => {
                            if let Err(e) = launch_hot(ctx, udid, &app, env) {
                                ctx.out.error(&e);
                            }
                        }
                        BuildOutcome::Failed(e) => ctx.out.error(&e),
                        // Ctrl-C during the rebuild quits the hot session.
                        BuildOutcome::Aborted => break,
                    }
                }
                SessionKey::Quit => break,
                // The hot session has no in-session filter keys — ignore them.
                SessionKey::Filter(_) | SessionKey::Ignore => {}
            },
            rawmode::Input::Idle => {}
            rawmode::Input::Closed => break,
        }
    }
}

/// A launched app in the interactive session, plus what's needed to terminate it
/// between rebuilds and on quit. `stream` is the child whose stdout/stderr *is* the
/// app's console output: the simulator's `simctl launch --console-pty`, the device
/// console, or (macOS) the app process itself. Its exit signals the app's own exit
/// ([`check_exit`]); os_log is streamed separately ([`LogStream`]).
struct Running {
    stream: Option<Child>,
    kind: RunningKind,
    /// App identifier for status lines (bundle id, or the macOS executable name).
    name: String,
    /// Set once we've reported the app exiting, so we don't repeat it each tick.
    reported_exit: bool,
}

enum RunningKind {
    /// Terminate via `simctl`; the attached console child (`Running.stream`) is what
    /// liveness is probed on.
    Simulator { udid: String, bundle_id: String },
    /// The console process launched the app; terminate via devicectl.
    Device { id: String, bundle_id: String },
    /// The streamed child *is* the macOS app; killing it stops the app.
    Mac,
}

/// The session's os_log stream — the simulator's (via `simctl spawn`) or a macOS
/// app's (the host `log stream`). Its predicate matches by process name, so one
/// stream follows the app across rebuild/relaunch. The simulator's `log` process is
/// reparented to `launchd_sim` and outlives our `simctl` child, so it's reaped by a
/// predicate marker; the host stream is a direct child, killed directly. The reader
/// runs detached; [`Drop`] stops the stream at session end.
struct LogStream {
    child: Child,
    /// The simulator stream's session-unique, regex-safe predicate tag, used to reap
    /// its reparented `log` process on drop without touching another session's stream
    /// for the same app (see [`log_stream_marker`]). `None` for the host macOS
    /// stream, which is a direct child and needs no reaping.
    marker: Option<String>,
}

impl LogStream {
    /// Block until the stream ends on its own (e.g. the simulator shuts down).
    /// Used by the non-interactive `--hot` follow; Ctrl-C usually ends the
    /// process first.
    fn wait(&mut self) {
        let _ = self.child.wait();
    }
}

impl Drop for LogStream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // `simctl spawn … log stream` reparents the `log` process to launchd_sim,
        // so killing our simctl child leaves it running — reap it by the
        // session-unique tag embedded in its predicate. Best-effort. The host macOS
        // stream is a direct child (no marker), already killed above.
        if let Some(marker) = &self.marker {
            let _ = process::run("pkill", &["-f", marker], None, true);
        }
    }
}

/// Install (where applicable) and launch the just-built app, returning a [`Running`]
/// whose `stream` is the launched child carrying the app's console output — rendered
/// as `N [print]` by [`render_console`] and gated by `filter` like os_log. Assumes
/// [`build`] already produced the bundle, so it never builds itself; os_log is
/// streamed separately ([`start_logs`]).
fn start_app(ctx: &Context, plan: &RunPlan, filter: &Arc<AtomicU8>) -> Result<Running, CliError> {
    let app = plan.app_bundle()?;
    let app_path = app.path.display().to_string();
    match &plan.target {
        Target::Simulator(udid) => {
            ctx.out.step("Booting simulator", || simctl::boot(udid))?;
            ctx.out
                .step("Installing app", || simctl::install(udid, &app_path))?;
            // `--console-pty` keeps the launch attached, so this child's stdout/stderr
            // are the app's; its exit means the app exited.
            let mut child = ctx.out.step("Launching app", || {
                simctl::spawn_console(udid, &app.bundle_id)
            })?;
            render_console(&mut child, ctx.out.use_color(), filter);
            Ok(Running {
                stream: Some(child),
                kind: RunningKind::Simulator {
                    udid: udid.clone(),
                    bundle_id: app.bundle_id.clone(),
                },
                name: app.bundle_id,
                reported_exit: false,
            })
        }
        Target::Device(id) => {
            ctx.out.step("Installing app on device", || {
                devicectl::install(id, &app_path)
            })?;
            let mut child = devicectl::spawn_console(id, &app.bundle_id)?;
            render_console(&mut child, ctx.out.use_color(), filter);
            Ok(Running {
                stream: Some(child),
                kind: RunningKind::Device {
                    id: id.clone(),
                    bundle_id: app.bundle_id.clone(),
                },
                name: app.bundle_id,
                reported_exit: false,
            })
        }
        Target::Mac => {
            let mut child =
                process::spawn_piped_both(&app.executable.to_string_lossy(), &[], None)?;
            render_console(&mut child, ctx.out.use_color(), filter);
            Ok(Running {
                stream: Some(child),
                kind: RunningKind::Mac,
                name: app.bundle_id,
                reported_exit: false,
            })
        }
        Target::SpmRun(_) => unreachable!("SPM run does not use the interactive session"),
    }
}

/// Terminate the running app and stop its output stream. The session-scoped
/// simulator log stream is left running — it's torn down once, at session end.
fn terminate_app(running: Running) {
    let Running { stream, kind, .. } = running;
    match kind {
        RunningKind::Simulator {
            udid, bundle_id, ..
        } => {
            let _ = simctl::terminate(&udid, &bundle_id);
        }
        RunningKind::Device { id, bundle_id } => {
            let _ = devicectl::terminate(&id, &bundle_id);
        }
        // The macOS app *is* the streamed child — killing it below stops it.
        RunningKind::Mac => {}
    }
    if let Some(mut stream) = stream {
        let _ = stream.kill();
        let _ = stream.wait();
    }
}

/// The result of an interactive [`build`]. A Ctrl-C [`BuildOutcome::Aborted`]
/// cancels the whole session; a [`BuildOutcome::Failed`] build keeps the session
/// open so the error can be fixed and rebuilt with `r`.
enum BuildOutcome {
    /// Built successfully.
    Ok,
    /// The user pressed Ctrl-C — cancel the session.
    Aborted,
    /// Build failed (non-zero exit, or a spawn/wait error); carries the error.
    Failed(CliError),
}

/// Run the build, with Ctrl-C cancelling both the build and the session. While
/// xcodebuild runs, a watcher thread polls stdin: Ctrl-C (`0x03`) sends SIGINT to
/// the build's process group and reports [`BuildOutcome::Aborted`]; any other key
/// is swallowed so stray presses during a long build can't queue up as commands
/// once we're back at the prompt. A non-zero exit is [`BuildOutcome::Failed`],
/// which keeps the session open to fix and rebuild.
fn build(plan: &RunPlan, out: &Output, capture: Option<&std::path::Path>) -> BuildOutcome {
    use std::io::Write as _;
    let (parts, cwd) = plan.build_plan().command();
    let args: Vec<&str> = parts.iter().map(String::as_str).collect();
    let mut child = match process::spawn_piped_group("xcodebuild", &args, cwd.as_deref()) {
        Ok(child) => child,
        Err(e) => return BuildOutcome::Failed(e),
    };
    let pid = child.id();
    // Spinner + elapsed timer while xcodebuild is silent (its planning prelude,
    // or a no-op up-to-date build); erased as soon as the first line renders.
    let mut progress = buildlog::BuildProgress::start(out, "Building");
    // For the build-log recompiler (path A): tee the *raw* transcript (with its
    // `EMIT_FRONTEND_COMMAND_LINES` frontend commands) to a file, while the
    // beautifier still renders the structured stream below.
    let mut capture_file = capture.and_then(|p| std::fs::File::create(p).ok());

    let aborted = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let watcher = std::thread::spawn({
        let aborted = Arc::clone(&aborted);
        let done = Arc::clone(&done);
        move || {
            while !done.load(Ordering::Relaxed) {
                if let rawmode::Input::Key('\u{3}') = rawmode::poll_key() {
                    signal_group(pid, libc::SIGINT);
                    aborted.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
    });

    // Beautify xcodebuild's piped stdout on this thread (the same path as
    // [`buildlog::run`], inlined so we own the child for the watcher).
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(file) = capture_file.as_mut() {
                let _ = writeln!(file, "{line}");
            }
            if let Some(rendered) = progress.line(&line) {
                out.line(&rendered);
            }
        }
    }
    // Erase the spinner before the post-build notes in case nothing ever
    // rendered (e.g. Ctrl-C during the silent prelude).
    drop(progress);

    let status = child.wait();
    done.store(true, Ordering::Relaxed);
    let _ = watcher.join();

    if aborted.load(Ordering::Relaxed) {
        out.note("Build cancelled");
        return BuildOutcome::Aborted;
    }
    match status {
        Ok(s) if s.success() => BuildOutcome::Ok,
        Ok(_) => BuildOutcome::Failed(
            CliError::new("xcodebuild exited with a non-zero status")
                .context("building the app")
                .kind(ErrorKind::BuildFailure),
        ),
        Err(e) => BuildOutcome::Failed(
            CliError::new(format!("failed to wait for xcodebuild: {e}"))
                .context("building the app"),
        ),
    }
}

/// SIGINT (etc.) a process group spawned via [`process::spawn_piped_group`].
/// The child leads its own group, so its pid is the group id; the negative
/// target signals the whole tree, mirroring a terminal Ctrl-C.
fn signal_group(pid: u32, sig: libc::c_int) {
    // Safety: kill() with a pgid and signal number; failure (already-exited
    // group) is harmless and ignored.
    unsafe {
        libc::kill(-pid.cast_signed(), sig);
    }
}

/// One build + launch + inline follow until Ctrl-C — the non-interactive path
/// (CI/piped, or when stdin isn't a terminal).
fn follow_once(ctx: &Context, plan: &RunPlan) -> CliResult {
    let app = build_and_install(plan, &ctx.out)?;
    match &plan.target {
        Target::Simulator(udid) => {
            let launched = simctl::launch(udid, &app.bundle_id)?;
            ctx.out
                .note(&format!("Launched {} → {}", app.bundle_id, launched.trim()));
            stream_logs(ctx, udid, &app)
        }
        Target::Device(id) => {
            ctx.out.note(&format!(
                "Launching {} with console (Ctrl-C to stop)",
                app.bundle_id
            ));
            // Stream the device's os_log (pymobiledevice3) alongside the devicectl
            // console; no live filter on the non-interactive path, so use the default.
            let filter = Arc::new(AtomicU8::new(default_filter(&ctx.out).threshold()));
            let _logs = start_logs(ctx, plan, &filter);
            devicectl::launch_console(id, &app.bundle_id)
        }
        Target::Mac => {
            ctx.out
                .note(&format!("Running {} (Ctrl-C to stop)", app.bundle_id));
            // Stream the app's os_log alongside its inherited stdout/stderr; the
            // non-interactive path has no live filter, so use the default threshold.
            let filter = Arc::new(AtomicU8::new(default_filter(&ctx.out).threshold()));
            let _logs = start_logs(ctx, plan, &filter);
            process::stream(&app.executable.to_string_lossy(), &[], None)
                .context("running the macOS app")
        }
        Target::SpmRun(_) => unreachable!("SPM run handled before this match"),
    }
}

/// What the session does with a keystroke.
#[derive(Debug, PartialEq, Eq)]
enum SessionKey {
    Rebuild,
    Quit,
    /// Set the live log filter (the `1`–`4` keys).
    Filter(LogFilter),
    Ignore,
}

/// Map a keystroke to a session action. `r` rebuilds; `q`, Ctrl-C, and Ctrl-D
/// quit; `1`–`4` set the log filter (debug/info/error/off); everything else is
/// ignored. The key is first folded to the Latin letter on its physical position
/// ([`map_key_to_latin`]), so the shortcuts work on non-Latin layouts (Cyrillic
/// `к`/`й`) without switching. (A closed stdin is handled separately as
/// [`rawmode::Input::Closed`].)
fn classify_key(key: char) -> SessionKey {
    match map_key_to_latin(key) {
        'r' | 'R' => SessionKey::Rebuild,
        'q' | 'Q' | '\u{3}' | '\u{4}' => SessionKey::Quit,
        '1' => SessionKey::Filter(LogFilter::Debug),
        '2' => SessionKey::Filter(LogFilter::Info),
        '3' => SessionKey::Filter(LogFilter::Error),
        '4' => SessionKey::Filter(LogFilter::Off),
        _ => SessionKey::Ignore,
    }
}

/// Fold a character typed on a non-Latin keyboard layout to the Latin letter on
/// the same physical key, so the session shortcuts work without switching layouts.
/// Ported from Flutter's `keyboardLayoutMappings` — mapped by key *position*, not
/// visual resemblance (Cyrillic `р` sits on the QWERTY `h` key, so → `h`, not `p`).
/// Covers the Cyrillic ЙЦУКЕН family over the letter positions: Russian, Ukrainian,
/// and Belarusian share every letter key except `s`, which types `ы` (Russian) or
/// `і` (Ukrainian/Belarusian). Every other character passes through. (The other
/// Ukrainian-specific letters — є/ї/ґ — sit on punctuation keys, not shortcut keys.)
fn map_key_to_latin(key: char) -> char {
    match key {
        'й' => 'q',
        'ц' => 'w',
        'у' => 'e',
        'к' => 'r',
        'е' => 't',
        'н' => 'y',
        'г' => 'u',
        'ш' => 'i',
        'щ' => 'o',
        'з' => 'p',
        'ф' => 'a',
        'ы' | 'і' => 's',
        'в' => 'd',
        'а' => 'f',
        'п' => 'g',
        'р' => 'h',
        'о' => 'j',
        'л' => 'k',
        'д' => 'l',
        'я' => 'z',
        'ч' => 'x',
        'с' => 'c',
        'м' => 'v',
        'и' => 'b',
        'т' => 'n',
        'ь' => 'm',
        'Й' => 'Q',
        'Ц' => 'W',
        'У' => 'E',
        'К' => 'R',
        'Е' => 'T',
        'Н' => 'Y',
        'Г' => 'U',
        'Ш' => 'I',
        'Щ' => 'O',
        'З' => 'P',
        'Ф' => 'A',
        'Ы' | 'І' => 'S',
        'В' => 'D',
        'А' => 'F',
        'П' => 'G',
        'Р' => 'H',
        'О' => 'J',
        'Л' => 'K',
        'Д' => 'L',
        'Я' => 'Z',
        'Ч' => 'X',
        'С' => 'C',
        'М' => 'V',
        'И' => 'B',
        'Т' => 'N',
        'Ь' => 'M',
        other => other,
    }
}

/// The session's key hint. The log-level keys are shown only when there's an
/// os_log stream to filter (the simulator or a macOS app); a device session just
/// rebuilds and quits.
fn session_hint(ctx: &Context, filterable: bool) {
    let hint = if filterable {
        "r rebuild · q quit  │  log level: 1 debug · 2 info · 3 error · 4 off"
    } else {
        "r rebuild · q quit"
    };
    ctx.out.note(hint);
}

/// A live log-filter choice (the `1`–`4` keys). `Debug`/`Info`/`Error` show that
/// level *and above*; `Off` mutes the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogFilter {
    Debug,
    Info,
    Error,
    Off,
}

impl LogFilter {
    /// The minimum entry level to show, as a `u8` compared against
    /// [`oslog::Level::as_u8`]. `Off` sits above the highest level — nothing matches.
    fn threshold(self) -> u8 {
        match self {
            LogFilter::Debug => oslog::Level::Debug.as_u8(),
            LogFilter::Info => oslog::Level::Info.as_u8(),
            LogFilter::Error => oslog::Level::Error.as_u8(),
            LogFilter::Off => oslog::Level::Fault.as_u8() + 1,
        }
    }

    /// What this level shows, for the inline `log level:` marker.
    fn description(self) -> &'static str {
        match self {
            LogFilter::Debug => "all logs",
            LogFilter::Info => "info and above",
            LogFilter::Error => "errors only",
            LogFilter::Off => "muted",
        }
    }
}

/// The default live filter: `info` (hides Debug noise like the Xcode debug-dylib
/// bootstrap), or `debug` under `-v`/`--verbose`.
fn default_filter(out: &Output) -> LogFilter {
    if out.is_verbose() {
        LogFilter::Debug
    } else {
        LogFilter::Info
    }
}

/// Apply a log filter and print an inline marker, so the new threshold is visible
/// in the stream and reads as a setting that governs the logs from here on.
fn set_filter(ctx: &Context, filter: &AtomicU8, choice: LogFilter) {
    filter.store(choice.threshold(), Ordering::Relaxed);
    ctx.out
        .note(&format!("── log level: {} ──", choice.description()));
}

/// What an `r` rebuild asks the session to do next.
enum RebuildOutcome {
    /// Carry on; `launched` records whether the app came back up (a failed build
    /// keeps the session open with nothing running).
    Continue { launched: bool },
    /// Ctrl-C during the rebuild: cancel the whole session.
    Quit,
}

/// Stop the running app, rebuild, and relaunch (the `r` key). The session log
/// stream is left running; it follows the relaunched app by process name. Ctrl-C
/// during the rebuild returns [`RebuildOutcome::Quit`] so the session ends.
fn do_rebuild(
    ctx: &Context,
    plan: &RunPlan,
    running: &mut Option<Running>,
    filter: &Arc<AtomicU8>,
) -> RebuildOutcome {
    ctx.out.note("»  Restarting — rebuilding…");
    if let Some(old) = running.take() {
        terminate_app(old);
    }
    let started = Instant::now();
    match build(plan, &ctx.out, None) {
        BuildOutcome::Ok => match start_app(ctx, plan, filter) {
            Ok(r) => {
                *running = Some(r);
                note_launch(ctx, "Relaunched", started);
                RebuildOutcome::Continue { launched: true }
            }
            Err(e) => {
                ctx.out.error(&e);
                RebuildOutcome::Continue { launched: false }
            }
        },
        // Failed build: nothing runs until the next rebuild; the session stays open.
        BuildOutcome::Failed(e) => {
            ctx.out.error(&e);
            RebuildOutcome::Continue { launched: false }
        }
        BuildOutcome::Aborted => RebuildOutcome::Quit,
    }
}

/// `▶ scheme · configuration · destination` — the run summary shown before the
/// build, so what's about to run (and what was auto-selected) is clear up front.
fn print_summary(ctx: &Context, plan: &RunPlan) {
    ctx.out.note(&format!(
        "▶ {} · {} · {}",
        plan.scheme,
        plan.configuration,
        destination_label(plan)
    ));
}

/// A human-readable destination name for the summary (simulator/device name where
/// available, else a generic label).
fn destination_label(plan: &RunPlan) -> String {
    match &plan.target {
        Target::Simulator(udid) => sim_name(udid).unwrap_or_else(|| "iOS Simulator".to_string()),
        Target::Device(_) => "device".to_string(),
        Target::Mac => "macOS".to_string(),
        Target::SpmRun(product) => format!("swift run {product}"),
    }
}

/// Look up a booted/known simulator's name by udid (best-effort).
fn sim_name(udid: &str) -> Option<String> {
    simctl::list()
        .ok()?
        .into_iter()
        .find(|s| s.udid == udid)
        .map(|s| s.name)
}

/// Print `✓ {verb} in {N.N}s` for the build+launch that began at `started`.
fn note_launch(ctx: &Context, verb: &str, started: Instant) {
    ctx.out.note(&format!(
        "✓ {verb} in {:.1}s",
        started.elapsed().as_secs_f64()
    ));
}

/// Notice (once) if the running app has exited/crashed, detected by its launched
/// child (the attached console / app process) exiting. Best-effort: a missed notice
/// only costs the convenience alert, never correctness.
fn check_exit(ctx: &Context, running: &mut Running) {
    if running.reported_exit {
        return;
    }
    let exited = running
        .stream
        .as_mut()
        .and_then(|c| c.try_wait().ok().flatten())
        .is_some();
    if exited {
        ctx.out.alert(&format!("✗ {} exited", running.name));
        running.reported_exit = true;
    }
}

/// Where an os_log stream is tapped: a simulator (via `simctl spawn`) or the host
/// (a macOS app). Both speak the same `log stream --style ndjson` format, so
/// [`render_logs`] formats either.
enum LogSource<'a> {
    Simulator(&'a str),
    Mac,
}

/// Spawn an os_log stream as a background child with both stdout and stderr piped,
/// for [`render_logs`] to format and [`render_log_stderr`] to filter. See
/// [`log_command`] for the stream's shape.
fn spawn_logs(
    source: &LogSource,
    app: &AppBundle,
    level: &str,
    marker: Option<&str>,
) -> Result<Child, CliError> {
    let (program, args) = log_command(source, app, level, marker);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    process::spawn_piped_both(program, &refs, None)
}

/// The os_log stream level: `info` by default — which hides `Debug`-level entries
/// like the Xcode debug-dylib bootstrap chatter (and the app's own `.debug()`
/// lines) — raised to `debug` under `-v`/`--verbose`.
fn log_level(out: &Output) -> &'static str {
    if out.is_verbose() { "debug" } else { "info" }
}

/// Build the `log stream --style ndjson` command for the app's os_log output at
/// `level` (see [`log_level`]) — `xcrun simctl spawn <udid> log stream …` for a
/// simulator, or the host `log stream …` for a macOS app. The predicate matches by
/// process image — and the Xcode 15+ `.debug.dylib` sender, which carries app code
/// in Debug builds — so logs show even when the app sets no `Logger(subsystem:)`,
/// while Apple framework chatter stays out. ndjson is what [`oslog`] parses.
///
/// A `marker` appends an always-true clause that embeds a session-unique tag in the
/// predicate (and so in the reparented `log` process's argv), so the session can
/// later reap exactly its own stream. No process is named the tag, so
/// `process != "<tag>"` holds for every entry — the matched set is unchanged. Only
/// the simulator reparents its `log` process; the host stream is a direct child, so
/// it's spawned without a marker.
fn log_command(
    source: &LogSource,
    app: &AppBundle,
    level: &str,
    marker: Option<&str>,
) -> (&'static str, Vec<String>) {
    use std::fmt::Write as _;
    let exe = process_name(app);
    let mut predicate = format!(
        "process == \"{exe}\" AND (sender == \"{exe}\" OR sender == \"{exe}.debug.dylib\")"
    );
    if let Some(marker) = marker {
        let _ = write!(
            predicate,
            " AND (process CONTAINS \"{marker}\" OR process != \"{marker}\")"
        );
    }
    let mut stream = vec![
        "stream".to_string(),
        "--level".to_string(),
        level.to_string(),
        "--style".to_string(),
        "ndjson".to_string(),
        "--predicate".to_string(),
        predicate,
    ];
    match source {
        LogSource::Mac => ("log", stream),
        LogSource::Simulator(udid) => {
            let mut args = vec![
                "simctl".to_string(),
                "spawn".to_string(),
                (*udid).to_string(),
                "log".to_string(),
            ];
            args.append(&mut stream);
            ("xcrun", args)
        }
    }
}

/// Render a `log stream` child's ndjson stdout (the simulator or a macOS app) as
/// colored lines on a detached thread, dropping entries below the live `filter`
/// threshold. The thread ends when the child's stdout closes — i.e. when the stream
/// is dropped/killed, or the process exits — so it's never joined.
#[allow(clippy::print_stdout)] // live os_log stream on a detached thread
fn render_logs(child: &mut Child, color: bool, filter: Arc<AtomicU8>) {
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let rendered = oslog::render_ndjson_line(&line, color);
            if rendered.level.as_u8() >= filter.load(Ordering::Relaxed) {
                println!("{}", rendered.text);
            }
        }
    });
}

/// Render a launched app's own stdout and stderr as blue `HH:MM:SS.sss N [print]`
/// lines on detached threads — its direct console output (`print()`, etc.), stamped
/// with the local arrival time, distinct from os_log ([`render_logs`]). Both pipes
/// are drained so neither blocks the app; known
/// boot noise ([`is_boot_noise`]) is dropped, and lines obey the live `filter` like
/// os_log, so `4 off` silences them too.
#[allow(clippy::print_stdout)] // live app stdout/stderr stream on detached threads
fn render_console(child: &mut Child, color: bool, filter: &Arc<AtomicU8>) {
    let pipes: [Option<Box<dyn std::io::Read + Send>>; 2] = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ];
    for pipe in pipes.into_iter().flatten() {
        let filter = Arc::clone(filter);
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines() {
                let Ok(line) = line else { break };
                if is_boot_noise(&line) {
                    continue;
                }
                // Console output has no timestamp of its own; stamp it with the local
                // time the line arrived, so it lines up with the os_log stream.
                let now = oslog::now_clock();
                let rendered = oslog::render_console_line(Some(&now), &line, color);
                if rendered.level.as_u8() >= filter.load(Ordering::Relaxed) {
                    println!("{}", rendered.text);
                }
            }
        });
    }
}

/// Render the os_log stream child's **stderr** on a detached thread: drop known
/// boot-time noise (see [`is_boot_noise`]) and surface anything else as an
/// `E [system]` line, so a genuine `log` / `simctl` diagnostic (a rejected
/// predicate, say) reads like the rest of the output instead of an unprefixed raw
/// line. Gated by the live `filter` like [`render_logs`], so `4 off` silences it too.
#[allow(clippy::print_stdout)] // live log-tool stderr stream on a detached thread
fn render_log_stderr(child: &mut Child, color: bool, filter: Arc<AtomicU8>) {
    let Some(stderr) = child.stderr.take() else {
        return;
    };
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() || is_boot_noise(&line) {
                continue;
            }
            let rendered = oslog::render_fields(None, "Error", "system", &line, color);
            if rendered.level.as_u8() >= filter.load(Ordering::Relaxed) {
                println!("{}", rendered.text);
            }
        }
    });
}

/// Whether a line is harmless boot-time noise worth hiding wherever it surfaces — the
/// log-stream stderr ([`render_log_stderr`]) or the app's own console
/// ([`render_console`]). A process launched into the simulator's user context can't
/// resolve the host uid against the sim's user database, so libSystem prints
/// `getpwuid_r did not find a match for uid <n>`. It says nothing useful, so drop it;
/// genuine diagnostics fall through to their renderer.
fn is_boot_noise(line: &str) -> bool {
    line.contains("getpwuid_r did not find a match for uid")
}

/// Render a device's `pymobiledevice3` syslog stdout on a detached thread, mirroring
/// [`render_logs`]: parse each line, keep only the app's own images (its executable
/// or `.debug.dylib`, the analog of the `log stream` `sender ==` predicate), drop
/// entries below the live `filter` threshold, and format via [`oslog::render_fields`]
/// so device logs read identically to the simulator's.
#[allow(clippy::print_stdout)] // live device syslog stream on a detached thread
fn render_device_logs(child: &mut Child, color: bool, exe: String, filter: Arc<AtomicU8>) {
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    let debug_dylib = format!("{exe}.debug.dylib");
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Some(entry) = pymobiledevice3::parse_line(&line) else {
                continue;
            };
            if entry.image != exe && entry.image != debug_dylib {
                continue;
            }
            let rendered = oslog::render_fields(
                Some(entry.timestamp),
                entry.level,
                entry.category,
                entry.message,
                color,
            );
            if rendered.level.as_u8() >= filter.load(Ordering::Relaxed) {
                println!("{}", rendered.text);
            }
        }
    });
}

/// Start the session's os_log stream (see [`LogStream`]) — the simulator's or a
/// macOS app's via `log stream`, or a device's via `pymobiledevice3`. `None` if the
/// app bundle or stream can't be resolved, or (for a device) when `pymobiledevice3`
/// is missing — logs are best-effort and the device console keeps working. The
/// stream carries every level; `filter` decides what's shown, so the live filter
/// can reveal debug on demand without restarting it.
fn start_logs(ctx: &Context, plan: &RunPlan, filter: &Arc<AtomicU8>) -> Option<LogStream> {
    let app = plan.app_bundle().ok()?;
    let (source, marker) = match &plan.target {
        Target::Simulator(udid) => (LogSource::Simulator(udid), Some(log_stream_marker())),
        Target::Mac => (LogSource::Mac, None),
        Target::Device(_) => return start_device_logs(ctx, &app, filter),
        Target::SpmRun(_) => return None,
    };
    let mut child = spawn_logs(&source, &app, "debug", marker.as_deref()).ok()?;
    render_logs(&mut child, ctx.out.use_color(), Arc::clone(filter));
    render_log_stderr(&mut child, ctx.out.use_color(), Arc::clone(filter));
    Some(LogStream { child, marker })
}

/// Start a physical device's os_log stream via `pymobiledevice3` — the host `log`
/// can't target a device, and the devicectl console carries only stdout/stderr, so
/// this is where `os_log`/`Logger` output comes from. Augments the console; returns
/// `None` with an install hint when `pymobiledevice3` is absent, so the run keeps
/// its console output.
fn start_device_logs(ctx: &Context, app: &AppBundle, filter: &Arc<AtomicU8>) -> Option<LogStream> {
    if !pymobiledevice3::is_available() {
        ctx.out.alert(&format!(
            "{} not found — device os_log won't be streamed (the console still shows stdout/stderr).",
            pymobiledevice3::BINARY
        ));
        ctx.out
            .note("  install: brew install uv && uv tool install pymobiledevice3");
        return None;
    }
    let exe = process_name(app).to_string();
    let mut child = pymobiledevice3::spawn(&exe).ok()?;
    render_device_logs(&mut child, ctx.out.use_color(), exe, Arc::clone(filter));
    // `pymobiledevice3` is a direct child, killed on drop — no reparented `log` to reap.
    Some(LogStream {
        child,
        marker: None,
    })
}

/// A per-session, regex-safe tag for the log stream's predicate, so its reparented
/// `log` process can be reaped by exactly this session on drop (see [`LogStream`]).
/// Unique across concurrent runs (our pid) and across streams within one run (a
/// counter); plain ASCII, so `pkill -f` matches it literally rather than as a regex.
fn log_stream_marker() -> String {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("sweetpad-logstream-{}-{seq}", std::process::id())
}

/// The app's process name (CFBundleExecutable) — the predicate key for the log
/// stream and the marker used to reap it.
fn process_name(app: &AppBundle) -> &str {
    app.executable
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
}

/// The stage-only `app` actions (install/launch/logs/stop) share resolution.
#[derive(Clone, Copy)]
enum Stage {
    Install,
    Launch,
    Logs,
    Stop,
}

fn simple(ctx: &mut Context, stage: Stage) -> CommandResult {
    // These default to a simulator target (the common headless case).
    let opts = RunOpts {
        device: false,
        device_id: None,
        mac: false,
        no_logs: true,
        hot: false,
        hot_mode: Mode::Resolver,
        hot_selfcheck: None,
    };
    let plan = plan(ctx, &opts)?;
    let Target::Simulator(udid) = &plan.target else {
        return Err(CliError::new(
            "app install/launch/logs/stop are only supported for simulator targets",
        ));
    };
    let app = plan.app_bundle()?;

    let report = match stage {
        Stage::Install => {
            plan.build_plan()
                .run(&ctx.out)
                .map_err(|e| e.or_kind(ErrorKind::BuildFailure))?;
            ctx.out.step("Booting simulator", || simctl::boot(udid))?;
            ctx.out.step("Installing app", || {
                simctl::install(udid, &app.path.display().to_string())
            })?;
            AppStageReport {
                action: "installed",
                note: format!("Installed {}", app.bundle_id),
                bundle_id: app.bundle_id.clone(),
                udid: udid.clone(),
                detail: None,
            }
        }
        Stage::Launch => {
            ctx.out.step("Booting simulator", || simctl::boot(udid))?;
            // Bring the Simulator window up so the launched app is visible (best-effort).
            let _ = simctl::open_app();
            let out = ctx
                .out
                .step("Launching app", || simctl::launch(udid, &app.bundle_id))?;
            let detail = out.trim().to_string();
            AppStageReport {
                action: "launched",
                note: format!("Launched {} → {detail}", app.bundle_id),
                bundle_id: app.bundle_id.clone(),
                udid: udid.clone(),
                detail: Some(detail),
            }
        }
        Stage::Logs => {
            // Boot first so the stream attaches instead of failing with "device is
            // not booted" when the simulator is shut down.
            ctx.out.step("Booting simulator", || simctl::boot(udid))?;
            return stream_logs(ctx, udid, &app).map(|()| Rendered::Streamed);
        }
        Stage::Stop => {
            ctx.out.step("Terminating app", || {
                simctl::terminate(udid, &app.bundle_id)
            })?;
            AppStageReport {
                action: "terminated",
                note: format!("Terminated {}", app.bundle_id),
                bundle_id: app.bundle_id.clone(),
                udid: udid.clone(),
                detail: None,
            }
        }
    };
    Ok(Rendered::data(report))
}

/// The result of an `app install`/`launch`/`stop` stage: a status note in human
/// mode, or `{ action, bundleId, udid, detail }` in the JSON envelope.
struct AppStageReport {
    action: &'static str,
    note: String,
    bundle_id: String,
    udid: String,
    detail: Option<String>,
}

impl Render for AppStageReport {
    fn human(&self, out: &Output) {
        out.note(&self.note);
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "action": self.action,
            "bundleId": self.bundle_id,
            "udid": self.udid,
            "detail": self.detail,
        })
    }
}

/// Follow a simulator's log for the app inline until Ctrl-C — the non-interactive
/// fallback (the interactive session backgrounds the same stream via [`spawn_logs`]).
#[allow(clippy::print_stdout)] // non-interactive inline log follow
fn stream_logs(ctx: &Context, udid: &str, app: &AppBundle) -> CliResult {
    ctx.out.note(&format!(
        "Streaming logs for {} (Ctrl-C to stop)",
        app.bundle_id
    ));
    let color = ctx.out.use_color();
    let (program, args) = log_command(&LogSource::Simulator(udid), app, log_level(&ctx.out), None);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let ok = process::stream_lines(program, &refs, None, |line| {
        println!("{}", oslog::render_ndjson_line(line, color).text);
    })?;
    if ok {
        Ok(())
    } else {
        Err(CliError::new("log stream exited with a non-zero status"))
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
    fn bg_boot_is_a_noop_for_non_simulator_targets() {
        // No simulator → no thread is spawned and waiting just succeeds; a second
        // wait (handle already taken) is still Ok. The simulator path spawns a real
        // `simctl boot`, so it's covered by the run e2e rather than here.
        let mut boot = BgBoot::start(&Target::Mac);
        assert!(boot.wait().is_ok());
        assert!(boot.wait().is_ok());
    }

    #[test]
    fn session_keys_map_to_actions() {
        // `r` rebuilds (either case).
        assert_eq!(classify_key('r'), SessionKey::Rebuild);
        assert_eq!(classify_key('R'), SessionKey::Rebuild);
        // `q`, Ctrl-C, and Ctrl-D all quit.
        assert_eq!(classify_key('q'), SessionKey::Quit);
        assert_eq!(classify_key('Q'), SessionKey::Quit);
        assert_eq!(classify_key('\u{3}'), SessionKey::Quit);
        assert_eq!(classify_key('\u{4}'), SessionKey::Quit);
        // 1–4 set the log filter to debug/info/error/off.
        assert_eq!(classify_key('1'), SessionKey::Filter(LogFilter::Debug));
        assert_eq!(classify_key('2'), SessionKey::Filter(LogFilter::Info));
        assert_eq!(classify_key('3'), SessionKey::Filter(LogFilter::Error));
        assert_eq!(classify_key('4'), SessionKey::Filter(LogFilter::Off));
        // Anything else is ignored — the session keeps streaming output.
        assert_eq!(classify_key('x'), SessionKey::Ignore);
        assert_eq!(classify_key('\n'), SessionKey::Ignore);
    }

    #[test]
    fn cyrillic_layout_keys_map_to_the_same_actions() {
        // The R and Q physical keys on the ЙЦУКЕН layout type к and й.
        assert_eq!(classify_key('к'), SessionKey::Rebuild);
        assert_eq!(classify_key('К'), SessionKey::Rebuild);
        assert_eq!(classify_key('й'), SessionKey::Quit);
        assert_eq!(classify_key('Й'), SessionKey::Quit);
    }

    #[test]
    fn key_mapping_is_by_position_not_appearance() {
        // Cyrillic р sits on the QWERTY h key — mapped by position, not its
        // look-alike `p`. Latin input and unmapped chars pass through.
        assert_eq!(map_key_to_latin('р'), 'h');
        assert_eq!(map_key_to_latin('к'), 'r');
        assert_eq!(map_key_to_latin('й'), 'q');
        assert_eq!(map_key_to_latin('r'), 'r');
        assert_eq!(map_key_to_latin('1'), '1');
        // The S key types ы on Russian, і on Ukrainian/Belarusian — both → s.
        assert_eq!(map_key_to_latin('ы'), 's');
        assert_eq!(map_key_to_latin('і'), 's');
        assert_eq!(map_key_to_latin('І'), 'S');
    }

    #[test]
    fn drops_getpwuid_boot_noise_but_keeps_real_diagnostics() {
        // The libSystem uid-lookup warning the simulator's `log` prints on every
        // launch — dropped, for any uid.
        assert!(is_boot_noise("getpwuid_r did not find a match for uid 503"));
        assert!(is_boot_noise("getpwuid_r did not find a match for uid 0"));
        // A genuine `log`/`simctl` diagnostic survives, to render as `E [system]`.
        assert!(!is_boot_noise("log: Invalid predicate"));
        assert!(!is_boot_noise(""));
    }

    #[test]
    fn log_stream_markers_are_unique_and_regex_safe() {
        let (a, b) = (log_stream_marker(), log_stream_marker());
        assert_ne!(a, b);
        // Only ASCII alphanumerics and hyphens, so `pkill -f` matches it literally
        // (no regex metacharacters) — see [`LogStream`].
        assert!(a.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'));
    }

    #[test]
    fn filter_descriptions_are_unambiguous() {
        assert_eq!(LogFilter::Debug.description(), "all logs");
        assert_eq!(LogFilter::Info.description(), "info and above");
        assert_eq!(LogFilter::Error.description(), "errors only");
        assert_eq!(LogFilter::Off.description(), "muted");
        // `Off` sits above every real level, so nothing passes the filter.
        assert!(LogFilter::Off.threshold() > LogFilter::Error.threshold());
    }

    fn test_app() -> AppBundle {
        AppBundle {
            path: std::path::PathBuf::from("/tmp/MyApp.app"),
            bundle_id: "com.example.MyApp".to_string(),
            executable: std::path::PathBuf::from("/tmp/MyApp.app/Contents/MacOS/MyApp"),
        }
    }

    #[test]
    fn log_command_simulator_wraps_simctl_spawn_with_marker() {
        let app = test_app();
        let (program, args) =
            log_command(&LogSource::Simulator("UDID-1"), &app, "info", Some("tag-7"));
        assert_eq!(program, "xcrun");
        assert_eq!(&args[..5], &["simctl", "spawn", "UDID-1", "log", "stream"]);
        let predicate = args.last().unwrap();
        // Matches the app's process and both its bare + `.debug.dylib` senders.
        assert!(predicate.contains(r#"process == "MyApp""#));
        assert!(predicate.contains(r#"sender == "MyApp.debug.dylib""#));
        // The marker rides in the predicate so the reparented `log` process is reapable.
        assert!(predicate.contains("tag-7"));
    }

    #[test]
    fn log_command_mac_runs_host_log_without_marker() {
        let app = test_app();
        let (program, args) = log_command(&LogSource::Mac, &app, "debug", None);
        // The host `log` binary directly — no `simctl spawn` wrapper.
        assert_eq!(program, "log");
        assert_eq!(&args[..2], &["stream", "--level"]);
        assert!(!args.contains(&"spawn".to_string()));
        let predicate = args.last().unwrap();
        assert!(predicate.contains(r#"process == "MyApp""#));
        // A direct child needs no reaping tag, so no marker clause is appended.
        assert!(!predicate.contains("CONTAINS"));
    }
}
