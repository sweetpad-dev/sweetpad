//! `sweetpad app …` — the built app's lifecycle: build+install+launch, and the
//! running session, on a simulator or a physical device. The app is the noun;
//! these are its actions.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Child;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Subcommand;

use crate::cli::inject::recompiler::{Mode, Recompiler};
use crate::cli::inject::server::{InjectServer, Logger};
use crate::cli::inject::{self, HotSession};
use crate::cli::output::Output;
use crate::cli::resolve::{self, Resolved};
use crate::cli::xcodebuild::{self, AppBundle};
use crate::cli::{CliError, CliResult, Context, buildlog, devicectl, process, rawmode, simctl};

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

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
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

    // Hot reload owns its own build + launch + watch session (simulator only).
    if opts.hot {
        return run_hot_session(ctx, &plan, opts.hot_mode, opts.hot_selfcheck);
    }

    // A Swift package executable builds, runs, and streams in one `swift run`;
    // there's no separate log stream to background, so it stays a one-shot.
    if matches!(plan.target, Target::SpmRun(_)) {
        return deploy(ctx, &plan);
    }

    // --no-logs: deploy and return, no session.
    if opts.no_logs {
        return deploy(ctx, &plan);
    }

    // Default: build, launch, and follow the app's output. At an interactive
    // terminal this is the rebuild session — output streams in the background
    // and `r` rebuilds+relaunches on demand. Non-interactive (CI/piped) runs
    // fall back to a one-shot launch + inline follow until Ctrl-C.
    if ctx.out.is_interactive() {
        run_session(ctx, &plan)
    } else {
        follow_once(ctx, &plan)
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

/// Interactive rebuild session: build + launch + stream the app's output, then
/// rebuild + relaunch on demand. `r` rebuilds; `q`, Ctrl-C, or Ctrl-D quit. Raw
/// mode flips only stdin's line discipline (see [`rawmode`]) so output keeps
/// streaming; the build is interruptible (Ctrl-C aborts it without leaving the
/// session). The running app is terminated before each relaunch and on quit.
fn run_session(ctx: &Context, plan: &RunPlan) -> CliResult {
    // Raw mode needs a terminal on stdin; without one (piped input) fall back to
    // a one-shot launch + inline follow.
    let Ok(_raw) = rawmode::RawMode::enable() else {
        return follow_once(ctx, plan);
    };

    // Initial build + launch. A failed/aborted first build exits — there's
    // nothing running to attach a session to.
    build(plan, &ctx.out, None)?;
    let mut running = Some(start_app(ctx, plan)?);
    session_hint(ctx);

    loop {
        match rawmode::poll_key() {
            rawmode::Input::Key(key) => match classify_key(key) {
                SessionKey::Rebuild => {
                    ctx.out.note("↻ restarting — rebuilding…");
                    // Stop the old app first so build output is clean and the
                    // relaunch is always a fresh process.
                    if let Some(old) = running.take() {
                        terminate_app(old);
                    }
                    match build(plan, &ctx.out, None) {
                        Ok(()) => match start_app(ctx, plan) {
                            Ok(r) => running = Some(r),
                            Err(e) => ctx.out.error(&e.to_string()),
                        },
                        // Failed/aborted build: nothing runs until the next `r`.
                        Err(e) => ctx.out.error(&e.to_string()),
                    }
                    session_hint(ctx);
                }
                SessionKey::Quit => break,
                SessionKey::Ignore => {}
            },
            rawmode::Input::Idle => {}
            rawmode::Input::Closed => break,
        }
    }
    if let Some(r) = running.take() {
        terminate_app(r);
    }
    Ok(())
}

/// `app run --hot` — the built-in hot-reload session (iOS Simulator only).
///
/// Builds with the interposable / frontend-command flags, starts the injection
/// server on `:8887`, launches the app with the client dylib injected, then
/// watches the workspace: each Swift save is recompiled and `.load`-ed into the
/// running app — no relaunch, state preserved. `r` still does a full
/// rebuild+relaunch (the client reconnects); `q`/Ctrl-C/Ctrl-D quit.
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
    build(plan, &ctx.out, Some(&build_log))?;
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
    let log: Logger = Arc::new(|m: &str| println!("{m}"));
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

    // Install + launch with the client injected, then stream the app's logs.
    let mut stream = launch_hot(ctx, udid, &app, &launch_env)?;
    // Watch the workspace; each save drives `server.inject`.
    let session = HotSession::start(Arc::clone(&server), &project_root);

    // CI self-check: edit a file once, assert `.injected`, exit. Otherwise the
    // interactive key loop (`r`/`q`), or — non-TTY — follow logs until Ctrl-C.
    let outcome = if let Some(file) = selfcheck {
        hot_selfcheck(ctx, &server, file)
    } else if ctx.out.is_interactive() {
        hot_key_loop(ctx, plan, udid, &launch_env, &mut stream);
        Ok(())
    } else {
        ctx.out
            .note("hot reload: watching for Swift changes (Ctrl-C to stop)");
        let _ = stream.wait();
        Ok(())
    };

    // Teardown: stop watcher + server, terminate the app, kill the log stream.
    session.shutdown();
    server.shutdown();
    let _ = simctl::terminate(udid, &app.bundle_id);
    let _ = stream.kill();
    let _ = stream.wait();
    let _ = std::fs::remove_dir_all(
        std::env::temp_dir().join(format!("sweetpad-hot-{}", std::process::id())),
    );
    outcome
}

/// CI self-check: wait for the client to connect, edit `file` once (driving the
/// watcher → recompile → `.load`), and assert a `.injected` response. Returns an
/// error if the client never connects or injection fails/times out — so
/// `app run --hot --hot-selfcheck FILE` is a hard pass/fail end-to-end test.
fn hot_selfcheck(ctx: &Context, server: &Arc<InjectServer>, file: &Path) -> CliResult {
    use std::time::Duration;

    ctx.out
        .note("hot reload self-check: waiting for the app to connect…");
    if !server.wait_connected(Duration::from_secs(30)) {
        return Err(CliError::new(
            "hot reload self-check: the in-app client never connected to :8887",
        ));
    }
    let baseline = server.result_counts();

    // Edit the file to drive the watcher (append a unique trailing comment).
    let original = std::fs::read_to_string(file)
        .map_err(|e| CliError::new(format!("self-check: read {}: {e}", file.display())))?;
    let edited = format!(
        "{original}\n// sweetpad hot-reload self-check {}\n",
        std::process::id()
    );
    std::fs::write(file, &edited)
        .map_err(|e| CliError::new(format!("self-check: write {}: {e}", file.display())))?;
    ctx.out
        .note(&format!("hot reload self-check: edited {}", file.display()));

    // The first inject is the slowest: the resolver primes its frontend-command
    // cache with a whole-module `swiftc -###` dry-run before compiling + linking.
    // Be generous so a slow/contended CI runner doesn't flake (the real watcher
    // loop has no such deadline — this bound only guards the self-check).
    let result = server.wait_for_result(baseline, Duration::from_secs(180));
    // Restore the file regardless of outcome.
    let _ = std::fs::write(file, original);

    match result {
        Some(true) => {
            ctx.out.note("hot reload self-check: ✅ .injected");
            Ok(())
        }
        Some(false) => Err(CliError::new("hot reload self-check: ❌ injection failed")),
        None => Err(CliError::new(
            "hot reload self-check: ❌ timed out waiting for .injected",
        )),
    }
}

/// Boot, install, and launch the app with the hot-reload env, returning the
/// backgrounded log-stream child. Shared by the first launch and each `r`.
fn launch_hot(
    ctx: &Context,
    udid: &str,
    app: &AppBundle,
    env: &[(String, String)],
) -> Result<Child, CliError> {
    simctl::boot(udid)?;
    simctl::install(udid, &app.path.display().to_string())?;
    let launched = simctl::launch_with_env(udid, &app.bundle_id, env)?;
    ctx.out
        .note(&format!("launched {} → {}", app.bundle_id, launched.trim()));
    spawn_logs(udid, app)
}

/// The `--hot` keypress loop: `r` full rebuild+relaunch (the client reconnects),
/// `q`/Ctrl-C/Ctrl-D quit. Injection happens out-of-band via the watcher.
fn hot_key_loop(
    ctx: &Context,
    plan: &RunPlan,
    udid: &str,
    env: &[(String, String)],
    stream: &mut Child,
) {
    let Ok(_raw) = rawmode::RawMode::enable() else {
        // No TTY for raw mode — just follow logs until the stream ends.
        let _ = stream.wait();
        return;
    };
    ctx.out
        .note("hot reload ready · edit a Swift file to inject · r rebuilds · q quits");
    loop {
        match rawmode::poll_key() {
            rawmode::Input::Key(key) => match classify_key(key) {
                SessionKey::Rebuild => {
                    ctx.out.note("↻ full rebuild — relaunching…");
                    let app = match plan.app_bundle() {
                        Ok(a) => a,
                        Err(e) => {
                            ctx.out.error(&e.to_string());
                            continue;
                        }
                    };
                    let _ = simctl::terminate(udid, &app.bundle_id);
                    let _ = stream.kill();
                    let _ = stream.wait();
                    match build(plan, &ctx.out, None) {
                        Ok(()) => match launch_hot(ctx, udid, &app, env) {
                            Ok(child) => *stream = child,
                            Err(e) => ctx.out.error(&e.to_string()),
                        },
                        Err(e) => ctx.out.error(&e.to_string()),
                    }
                }
                SessionKey::Quit => break,
                SessionKey::Ignore => {}
            },
            rawmode::Input::Idle => {}
            rawmode::Input::Closed => break,
        }
    }
}

/// A launched app in the interactive session: the background process streaming
/// its output (simulator log stream, device console, or — for macOS — the app
/// itself), plus what's needed to terminate the app between rebuilds and on quit.
struct Running {
    stream: Child,
    kind: RunningKind,
}

enum RunningKind {
    /// The log stream is separate from the app; terminate via simctl.
    Simulator { udid: String, bundle_id: String },
    /// The console process launched the app; terminate via devicectl.
    Device { id: String, bundle_id: String },
    /// The streamed child *is* the macOS app; killing it stops the app.
    Mac,
}

/// Install (where applicable) and launch the just-built app, starting the
/// background stream of its output. Assumes [`build`] already produced the
/// bundle, so it never builds itself.
fn start_app(ctx: &Context, plan: &RunPlan) -> Result<Running, CliError> {
    let app = plan.app_bundle()?;
    let app_path = app.path.display().to_string();
    match &plan.target {
        Target::Simulator(udid) => {
            simctl::boot(udid)?;
            simctl::install(udid, &app_path)?;
            let launched = simctl::launch(udid, &app.bundle_id)?;
            ctx.out
                .note(&format!("launched {} → {}", app.bundle_id, launched.trim()));
            Ok(Running {
                stream: spawn_logs(udid, &app)?,
                kind: RunningKind::Simulator {
                    udid: udid.clone(),
                    bundle_id: app.bundle_id,
                },
            })
        }
        Target::Device(id) => {
            devicectl::install(id, &app_path)?;
            ctx.out.note(&format!(
                "launching {} on device with console",
                app.bundle_id
            ));
            Ok(Running {
                stream: devicectl::spawn_console(id, &app.bundle_id)?,
                kind: RunningKind::Device {
                    id: id.clone(),
                    bundle_id: app.bundle_id,
                },
            })
        }
        Target::Mac => {
            ctx.out.note(&format!("running {}", app.bundle_id));
            Ok(Running {
                stream: process::spawn(&app.executable.to_string_lossy(), &[], None)?,
                kind: RunningKind::Mac,
            })
        }
        Target::SpmRun(_) => unreachable!("SPM run does not use the interactive session"),
    }
}

/// Terminate the running app and stop its output stream.
fn terminate_app(running: Running) {
    let Running { mut stream, kind } = running;
    match kind {
        RunningKind::Simulator { udid, bundle_id } => {
            let _ = simctl::terminate(&udid, &bundle_id);
        }
        RunningKind::Device { id, bundle_id } => {
            let _ = devicectl::terminate(&id, &bundle_id);
        }
        // The macOS app *is* the streamed child — killing it below stops it.
        RunningKind::Mac => {}
    }
    let _ = stream.kill();
    let _ = stream.wait();
}

/// Run the build, letting Ctrl-C abort it without leaving the session. While
/// xcodebuild runs, a watcher thread polls stdin: Ctrl-C (`0x03`) sends SIGINT
/// to the build's process group; any other key is swallowed so stray presses
/// during a long build can't queue up as commands once we're back at the prompt.
/// Returns `Ok` on success, `Err` on a failed or aborted build.
fn build(plan: &RunPlan, out: &Output, capture: Option<&std::path::Path>) -> CliResult {
    use std::io::Write as _;
    let (parts, cwd) = plan.build_plan().command();
    let args: Vec<&str> = parts.iter().map(String::as_str).collect();
    let mut child = process::spawn_piped_group("xcodebuild", &args, cwd.as_deref())?;
    let pid = child.id();
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
                if let rawmode::Input::Key(0x03) = rawmode::poll_key() {
                    signal_group(pid, libc::SIGINT);
                    aborted.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
    });

    // Beautify xcodebuild's piped stdout on this thread (the same path as
    // [`buildlog::run`], inlined so we own the child for the watcher).
    let color = out.use_color();
    let verbose = out.is_verbose();
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(file) = capture_file.as_mut() {
                let _ = writeln!(file, "{line}");
            }
            if let Some(rendered) = buildlog::render(&buildlog::parse_line(&line), color, verbose) {
                out.line(&rendered);
            }
        }
    }

    let status = child.wait();
    done.store(true, Ordering::Relaxed);
    let _ = watcher.join();

    if aborted.load(Ordering::Relaxed) {
        return Err(CliError::new("build aborted"));
    }
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err(CliError::new("xcodebuild exited with a non-zero status")),
        Err(e) => Err(CliError::new(format!("failed to wait for xcodebuild: {e}"))),
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
                .note(&format!("launched {} → {}", app.bundle_id, launched.trim()));
            stream_logs(ctx, udid, &app)
        }
        Target::Device(id) => {
            ctx.out.note(&format!(
                "launching {} with console (Ctrl-C to stop)",
                app.bundle_id
            ));
            devicectl::launch_console(id, &app.bundle_id)
        }
        Target::Mac => {
            ctx.out
                .note(&format!("running {} (Ctrl-C to stop)", app.bundle_id));
            process::stream(&app.executable.to_string_lossy(), &[], None)
        }
        Target::SpmRun(_) => unreachable!("SPM run handled before this match"),
    }
}

/// What the session does with a keystroke.
#[derive(Debug, PartialEq, Eq)]
enum SessionKey {
    Rebuild,
    Quit,
    Ignore,
}

/// Map a keystroke to a session action. `r` rebuilds; `q`, Ctrl-C (`0x03`), and
/// Ctrl-D (`0x04`) quit; everything else is ignored. (A closed stdin is handled
/// separately as [`rawmode::Input::Closed`].)
fn classify_key(key: u8) -> SessionKey {
    match key {
        b'r' | b'R' => SessionKey::Rebuild,
        b'q' | b'Q' | 0x03 | 0x04 => SessionKey::Quit,
        _ => SessionKey::Ignore,
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
        assert_eq!(classify_key(b'r'), SessionKey::Rebuild);
        assert_eq!(classify_key(b'R'), SessionKey::Rebuild);
        // `q`, Ctrl-C, and Ctrl-D all quit.
        assert_eq!(classify_key(b'q'), SessionKey::Quit);
        assert_eq!(classify_key(b'Q'), SessionKey::Quit);
        assert_eq!(classify_key(0x03), SessionKey::Quit);
        assert_eq!(classify_key(0x04), SessionKey::Quit);
        // Anything else is ignored — the session keeps streaming output.
        assert_eq!(classify_key(b'x'), SessionKey::Ignore);
        assert_eq!(classify_key(b'\n'), SessionKey::Ignore);
    }
}
