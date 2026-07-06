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

/// The `app run` flags — also the top-level `sweetpad run`'s, so the flagship
/// spelling and the resource-first one stay a single definition.
#[derive(Debug, Clone, Default, clap::Args)]
#[allow(clippy::struct_excessive_bools)] // independent CLI toggles, not a state machine
pub struct RunArgs {
    #[command(flatten)]
    pub target: crate::cli::BuildTargetArgs,

    /// Target a connected physical device instead of a simulator
    /// (`--on device` is the same thing).
    #[arg(long, conflicts_with = "on")]
    pub device: bool,

    /// Specific device UDID/name to target (implies --device).
    #[arg(long = "device-id", conflicts_with = "on")]
    pub device_id: Option<String>,

    /// Build and run as a native macOS app (`--on mac` is the same thing).
    #[arg(long, conflicts_with_all = ["device", "device_id", "on"])]
    pub mac: bool,

    /// Don't stream the app's logs after launching (logs follow by default
    /// on simulators).
    #[arg(long = "no-logs")]
    pub no_logs: bool,

    /// Enable hot reload (iOS Simulator only): on each Swift save the file is
    /// recompiled and injected into the running app — no relaunch, state
    /// preserved. Requires the injection client (see CLI_DESIGN §9d). A
    /// project can default this on via `[run] hot = true` in sweetpad.toml.
    #[arg(long)]
    pub hot: bool,

    /// Disable hot reload for this run (overrides a `[run] hot = true`
    /// project default).
    #[arg(long = "no-hot", conflicts_with = "hot")]
    pub no_hot: bool,

    /// Hot-reload recompiler.
    #[arg(long = "hot-recompiler", value_name = "MODE", value_enum)]
    pub hot_recompiler: Option<HotRecompiler>,

    /// CI self-check (hidden): with `--hot`, after launch edit FILE once, wait
    /// for `.injected`, and exit 0/1 instead of entering the session. Drives
    /// the end-to-end hot-reload/injection test.
    #[arg(
        long = "hot-selfcheck",
        value_name = "FILE",
        hide = true,
        requires = "hot"
    )]
    pub hot_selfcheck: Option<std::path::PathBuf>,

    #[command(flatten)]
    pub launch: LaunchArgs,

    /// Extra arguments passed to xcodebuild verbatim (after `--`).
    #[arg(last = true, value_name = "XCODEBUILD_ARGS")]
    pub passthrough: Vec<String>,
}

/// Launch inputs shared by `run` and `launch`: process arguments,
/// environment, and wait-for-debugger. Simulator and macOS targets honor all
/// three; physical devices don't yet.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct LaunchArgs {
    /// Argument passed to the app process (repeatable).
    #[arg(long = "arg", value_name = "ARG")]
    pub args: Vec<String>,

    /// Environment variable for the app process, KEY=VALUE (repeatable).
    #[arg(long = "env", value_name = "KEY=VALUE")]
    pub env: Vec<String>,

    /// Launch suspended, waiting for a debugger to attach (`lldb -p <pid>`).
    #[arg(long = "wait-for-debugger")]
    pub wait_for_debugger: bool,
}

impl LaunchArgs {
    /// Parse the `KEY=VALUE` pairs, `prefix`ed per key (simctl forwards only
    /// `SIMCTL_CHILD_*`; the macOS direct spawn takes them raw).
    fn env_pairs(&self, prefix: &str) -> Result<Vec<(String, String)>, CliError> {
        self.env
            .iter()
            .map(|pair| {
                pair.split_once('=')
                    .map(|(k, v)| (format!("{prefix}{k}"), v.to_string()))
                    .ok_or_else(|| CliError::new(format!("--env takes KEY=VALUE (got {pair:?})")))
            })
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.args.is_empty() && self.env.is_empty() && !self.wait_for_debugger
    }
}

/// `app logs` stream shaping: narrow the predicate or change the level.
/// Simulator/macOS streams honor all of these (`log stream` natively);
/// physical-device logs (pymobiledevice3) keep their fixed process filter.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct LogFilterArgs {
    /// Only entries from this subsystem (e.g. com.example.app.networking).
    #[arg(long)]
    pub subsystem: Option<String>,

    /// Only entries from this category.
    #[arg(long)]
    pub category: Option<String>,

    /// A raw `log stream` predicate, replacing the default process match
    /// entirely (the escape hatch).
    #[arg(long, conflicts_with_all = ["subsystem", "category"])]
    pub predicate: Option<String>,

    /// Minimum level to stream: default, info, or debug.
    #[arg(long, value_parser = ["default", "info", "debug"])]
    pub level: Option<String>,
}

/// Where a lifecycle stage acts: the default simulator flow, or a
/// connected physical device.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct StageTargetArgs {
    /// Act on a connected physical device instead of a simulator.
    #[arg(long)]
    pub device: bool,

    /// Specific device UDID/name (implies --device).
    #[arg(long = "device-id")]
    pub device_id: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Build, install, launch, and follow logs; at an interactive terminal,
    /// press `r` to rebuild on demand.
    Run(RunArgs),
    /// Build and install, without launching.
    Install {
        #[command(flatten)]
        target: crate::cli::BuildTargetArgs,
        #[command(flatten)]
        stage: StageTargetArgs,
    },
    /// Launch an already-installed app.
    Launch {
        #[command(flatten)]
        target: crate::cli::BuildTargetArgs,
        #[command(flatten)]
        stage: StageTargetArgs,
        #[command(flatten)]
        launch: LaunchArgs,
    },
    /// Build, install, and launch suspended, then attach lldb (simulator).
    Debug {
        #[command(flatten)]
        target: crate::cli::BuildTargetArgs,
        #[command(flatten)]
        launch: LaunchArgs,
    },
    /// Remove the app from a simulator or device.
    Uninstall {
        #[command(flatten)]
        target: crate::cli::BuildTargetArgs,
        #[command(flatten)]
        stage: StageTargetArgs,
    },
    /// Stream the running app's logs (simulator). Uses the last-launched app
    /// when one is recorded; otherwise resolves the build target. With --json,
    /// emits the raw `log stream` NDJSON events — one JSON object per line —
    /// instead of the rendered text.
    Logs {
        #[command(flatten)]
        target: crate::cli::BuildTargetArgs,
        #[command(flatten)]
        filters: LogFilterArgs,
    },
    /// Terminate the running app (the last-launched one when recorded).
    Stop {
        #[command(flatten)]
        target: crate::cli::BuildTargetArgs,
        #[command(flatten)]
        stage: StageTargetArgs,
    },
    /// Open a URL on a simulator (deep links / universal links).
    OpenUrl {
        /// The URL to open (e.g. `myapp://path` or `https://example.com/x`).
        url: String,
        /// Simulator name or UDID to open it on (defaults to the booted one).
        #[arg(long)]
        simulator: Option<String>,
    },
}

impl Action {
    /// The default action for a bare `sweetpad app`: `app run` with no flags
    /// (everything resolves from config/state/pickers).
    #[must_use]
    pub fn default_run() -> Self {
        Action::Run(RunArgs::default())
    }
}

/// Settle hot reload for a run: the `--hot` flag, else the project's
/// `[run] hot` default (opted out per run with `--no-hot`); the recompiler
/// from `--hot-recompiler`, else `[run] hot_recompiler`. Simulator-only
/// enforcement stays in [`run_hot_session`]; here a `[run] hot = true`
/// project default is simply ignored for `--mac`/`--device` runs rather than
/// erroring on a committed file.
fn hot_settings(ctx: &Context, args: &RunArgs) -> (bool, Mode) {
    let run_defaults = resolve::container(ctx)
        .ok()
        .map(|c| ctx.project_file(&c).run.clone())
        .unwrap_or_default();
    let default_hot =
        run_defaults.hot.unwrap_or(false) && !args.mac && !args.device && args.device_id.is_none();
    let hot = !args.no_hot && (args.hot || default_hot);
    let config_mode = run_defaults.hot_recompiler.as_deref().and_then(|s| {
        let mode = Mode::parse(s);
        if mode.is_none() {
            ctx.out.warn(&format!(
                "sweetpad.toml: unknown [run] hot_recompiler {s:?} (use resolver|buildlog)"
            ));
        }
        mode
    });
    let hot_mode = args
        .hot_recompiler
        .map(HotRecompiler::mode)
        .or(config_mode)
        .unwrap_or(Mode::Resolver);
    (hot, hot_mode)
}

/// The two hot-reload recompilers (see CLI_DESIGN §9d).
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum HotRecompiler {
    /// Robust whole-module recompiles via the build-settings resolver (default).
    Resolver,
    /// Fast single-file recompiles recovered from the build transcript.
    Buildlog,
}

impl HotRecompiler {
    fn mode(self) -> Mode {
        match self {
            HotRecompiler::Resolver => Mode::Resolver,
            HotRecompiler::Buildlog => Mode::BuildLog,
        }
    }
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Run(args) => {
            ctx.targeting = args.target.clone().into();
            let (hot, hot_mode) = hot_settings(ctx, args);
            // The live build-and-run session streams its own output until you quit.
            run_app(
                ctx,
                &RunOpts {
                    device: args.device || args.device_id.is_some(),
                    device_id: args.device_id.as_deref(),
                    mac: args.mac,
                    no_logs: args.no_logs,
                    hot,
                    hot_explicit: args.hot,
                    hot_mode,
                    hot_selfcheck: args.hot_selfcheck.as_deref(),
                    launch: &args.launch,
                    passthrough: &args.passthrough,
                },
            )
            .map(|()| Rendered::Streamed)
        }
        Action::Install { target, stage } => {
            ctx.targeting = target.clone().into();
            simple(ctx, Stage::Install, &LaunchArgs::default(), stage)
        }
        Action::Launch {
            target,
            stage,
            launch,
        } => {
            ctx.targeting = target.clone().into();
            simple(ctx, Stage::Launch, launch, stage)
        }
        Action::Debug { target, launch } => {
            ctx.targeting = target.clone().into();
            debug(ctx, launch)
        }
        Action::Uninstall { target, stage } => {
            ctx.targeting = target.clone().into();
            simple(ctx, Stage::Uninstall, &LaunchArgs::default(), stage)
        }
        Action::Logs { target, filters } => {
            ctx.targeting = target.clone().into();
            simple_logs(ctx, filters)
        }
        Action::Stop { target, stage } => {
            ctx.targeting = target.clone().into();
            simple(ctx, Stage::Stop, &LaunchArgs::default(), stage)
        }
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
    /// Whether `--hot` was typed (vs. the `[run] hot` config default) — a
    /// config default is silently ignored for non-simulator targets instead
    /// of erroring on a committed file.
    hot_explicit: bool,
    hot_mode: Mode,
    hot_selfcheck: Option<&'a Path>,
    /// Launch args/env/wait-for-debugger for the app process.
    launch: &'a LaunchArgs,
    /// Extra xcodebuild arguments (after `--`), passed through verbatim.
    passthrough: &'a [String],
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
    /// Launch args/env/wait-for-debugger for the app process.
    launch: LaunchArgs,
    /// Extra xcodebuild arguments (after `--`), passed through verbatim.
    passthrough: Vec<String>,
}

impl RunPlan {
    /// The simctl launch options for this plan (env `SIMCTL_CHILD_`-prefixed).
    fn simctl_launch<'a>(
        &'a self,
        env: &'a [(String, String)],
    ) -> crate::cli::simctl::LaunchOptions<'a> {
        crate::cli::simctl::LaunchOptions {
            args: &self.launch.args,
            env,
            wait_for_debugger: self.launch.wait_for_debugger,
        }
    }
}

impl RunPlan {
    fn build_plan(&self) -> xcodebuild::BuildPlan<'_> {
        xcodebuild::BuildPlan {
            container: &self.resolved.container,
            scheme: &self.scheme,
            configuration: &self.configuration,
            destination: Some(&self.destination),
            sdk: self.resolved.sdk.as_deref(),
            clean: false,
            hot: self.hot,
            passthrough: &self.passthrough,
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
            // Must match the build's own -sdk (if any), or TARGET_BUILD_DIR
            // points at a different products dir than the one just built.
            sdk: self.resolved.sdk.clone().unwrap_or_default(),
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
        // The destination narrows multi-app schemes (iOS + watch companion)
        // to the app that actually runs there.
        xcodebuild::app_bundle(&settings, Some(&self.destination))
    }
}

fn run_app(ctx: &mut Context, opts: &RunOpts) -> CliResult {
    // `app run` is a live build-and-run session that streams logs until you quit —
    // there's no coherent one-shot JSON for it (a `--json` run would emit a silent
    // build and then human-formatted logs). Fail fast; build and launch as separate
    // steps if you need machine-readable output.
    if ctx.out.is_json() || ctx.out.is_ndjson() {
        return Err(CliError::new(
            "`app run` streams a live session and has no machine-readable form; use \
             `build start -o ndjson`, `app install`/`app launch --json`, and \
             `app logs -o ndjson` as separate steps",
        ));
    }

    let mut plan = plan(ctx, opts)?;

    // Hot reload is simulator-only. A typed `--hot` on another target errors
    // inside `run_hot_session`; the `[run] hot = true` *config default* is
    // simply ignored for `--on mac`/device/SPM runs — a committed file must
    // not break them (the flag-gated variants are pre-filtered in
    // `hot_settings`, but only the resolved plan knows about `--on` and
    // remembered macOS destinations). Clearing `plan.hot` also drops the
    // hot-only build flags and the summary's "hot reload on" tag.
    let hot = opts.hot && (opts.hot_explicit || matches!(plan.target, Target::Simulator(_)));
    plan.hot = hot;
    let plan = plan;

    // The hot session launches without debugger suspension and owns the log
    // stream as part of its UI — reject the flags it can't honor instead of
    // silently dropping them.
    if hot && plan.launch.wait_for_debugger {
        return Err(CliError::new(
            "--wait-for-debugger isn't supported with --hot; run without --hot to \
             attach a debugger at launch",
        ));
    }
    if hot && opts.no_logs {
        return Err(CliError::new(
            "--no-logs isn't supported with --hot; the hot session streams logs \
             as part of its UI",
        ));
    }

    print_summary(ctx, &plan);

    // Bring the Simulator window up so the running app is visible. Best-effort and
    // once per run — rebuilds reuse the same window, and only a simulator has a UI
    // to reveal (devices and macOS don't).
    if matches!(plan.target, Target::Simulator(_)) {
        let _ = simctl::open_app();
    }

    let result = if hot {
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

/// The in-process resolver that locates the built .app can't see passthrough
/// flags that move xcodebuild's output — installing a stale bundle from
/// default DerivedData would silently run old code. Warn instead.
fn warn_if_passthrough_moves_output(ctx: &Context, passthrough: &[String]) {
    let moves_output = |t: &String| {
        t == "-derivedDataPath"
            || t == "-xcconfig"
            || t.starts_with("SYMROOT=")
            || t.starts_with("OBJROOT=")
            || t.starts_with("CONFIGURATION_BUILD_DIR=")
            || t.starts_with("TARGET_BUILD_DIR=")
    };
    if let Some(flag) = passthrough.iter().find(|t| moves_output(t)) {
        ctx.out.warn(&format!(
            "{flag} can move the build output, but the app is installed from the \
             default build location — the launched bundle may be stale or missing"
        ));
    }
}

/// Resolve a full run plan, choosing a simulator (default), a device, or macOS.
#[allow(clippy::too_many_lines)]
fn plan(ctx: &mut Context, opts: &RunOpts) -> Result<RunPlan, CliError> {
    let resolved = resolve::resolve(ctx)?;
    let schemes = resolve::schemes(&resolved.container)?;
    if let Some(s) = &resolved.scheme {
        resolve::validate_choice("scheme", s, &schemes)?;
    }
    let scheme = resolve::choose(ctx, "scheme", resolved.scheme.clone(), &schemes)?;
    let configuration = resolve::settle_configuration(ctx, &resolved)?;

    let (destination, target) = if matches!(resolved.container, resolve::Container::SwiftPackage(_))
    {
        if opts.device || opts.mac || ctx.targeting.on.is_some() {
            return Err(CliError::new(
                "a Swift package executable runs on the host; --device/--mac/--on don't apply",
            ));
        }
        // No xcodebuild destination — `swift run` builds and runs the product.
        (String::new(), Target::SpmRun(scheme.clone()))
    } else if let Some(reference) = ctx.targeting.on.clone() {
        // `--on` picks the concrete target — simulator, device, or Mac — from
        // one human reference; it replaces the --mac/--device mode flags.
        let key = resolved.container.key();
        let sims = simctl::list()?;
        match resolve::resolve_on(ctx, &key, &reference, &sims)? {
            resolve::OnTarget::Mac => ("platform=macOS".to_string(), Target::Mac),
            resolve::OnTarget::Simulator { udid, specifier } => {
                (specifier, Target::Simulator(udid))
            }
            resolve::OnTarget::Device { udid, specifier } => (specifier, Target::Device(udid)),
        }
    } else if opts.mac {
        ("platform=macOS".to_string(), Target::Mac)
    } else if opts.device {
        let devices = devicectl::list()?;
        let dev = if let Some(id) = opts.device_id {
            devicectl::find(&devices, id).ok_or_else(|| {
                CliError::new(format!("no device matching {id:?}"))
                    .kind(ErrorKind::TargetResolution)
            })?
        } else {
            let labels: Vec<String> = devices.iter().map(devicectl::Device::label).collect();
            let chosen = resolve::choose(ctx, "device", None, &labels)?;
            devices
                .iter()
                .find(|d| d.label() == chosen)
                .ok_or_else(|| {
                    CliError::new("device not found").kind(ErrorKind::TargetResolution)
                })?
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
        let platform = destination_platform(&destination).unwrap_or_default();
        if platform.eq_ignore_ascii_case("macOS") {
            // The picker's "My Mac" row (or a config/remembered macOS
            // destination, with or without extra keys like arch=) runs the
            // native-app flow, not a simulator.
            (destination, Target::Mac)
        } else if !platform.is_empty() && !platform.to_ascii_lowercase().contains("simulator") {
            // A physical-device destination (platform=iOS,id=…) routes to
            // devicectl — handing its udid to simctl would fail with an
            // unrelated "Invalid device" much later.
            let udid = udid(&destination).map_err(|_| {
                CliError::new(format!(
                    "physical-device destinations need an id= (got {destination:?})"
                ))
                .kind(ErrorKind::TargetResolution)
            })?;
            (destination, Target::Device(udid))
        } else {
            let udid = destination_udid(&destination)?;
            (destination, Target::Simulator(udid))
        }
    };

    // Launch inputs reach simulator and macOS processes; devicectl has no
    // equivalent plumbing here yet — say so instead of silently dropping them.
    if matches!(target, Target::Device(_)) && !opts.launch.is_empty() {
        return Err(CliError::new(
            "--arg/--env/--wait-for-debugger aren't supported for physical devices yet",
        ));
    }

    warn_if_passthrough_moves_output(ctx, opts.passthrough);

    let plan = RunPlan {
        resolved,
        scheme,
        configuration,
        destination,
        target,
        hot: opts.hot,
        launch: opts.launch.clone(),
        passthrough: opts.passthrough.to_vec(),
    };
    let bt = resolve::BuildTarget {
        scheme: plan.scheme.clone(),
        configuration: plan.configuration.clone(),
        destination: plan.destination.clone(),
    };
    // Remember the picks — but a `--mac`/`--device`/`--on`/SPM destination is
    // a one-off mode, never the remembered context (persisting it would
    // silently retarget the next plain `build start`). A *picker* choice —
    // simulator or the "My Mac" row — is exactly what state is for.
    let picker_sourced = ctx.targeting.on.is_none()
        && !opts.mac
        && !opts.device
        && opts.device_id.is_none()
        && !matches!(plan.target, Target::SpmRun(_));
    resolve::remember(ctx, &plan.resolved, &bt, picker_sourced);
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

/// Escape a value for embedding inside a double-quoted NSPredicate string
/// literal: a raw `"` or `\` would otherwise terminate the literal and make
/// `log stream` reject the whole predicate.
fn predicate_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The `platform=` value from a `-destination` specifier, e.g. `iOS`.
fn destination_platform(spec: &str) -> Option<String> {
    spec.split(',')
        .find_map(|kv| kv.trim().strip_prefix("platform="))
        .map(str::to_string)
}

/// `swift run <product>` in the package directory: builds and runs the
/// executable, streaming its output until it exits. `--arg` rides after the
/// product name (SwiftPM passes everything there to the program) and `--env`
/// lands in the child's environment; `--wait-for-debugger` has no `swift run`
/// equivalent, so it errors instead of being silently dropped.
fn spm_run(ctx: &Context, plan: &RunPlan, product: &str) -> CliResult {
    if plan.launch.wait_for_debugger {
        return Err(CliError::new(
            "--wait-for-debugger isn't supported for a Swift package executable; \
             use `swift build` and attach lldb to the binary directly",
        ));
    }
    if !plan.passthrough.is_empty() {
        return Err(CliError::new(
            "`--` passthrough args are xcodebuild flags; a Swift package runs via \
             `swift run` (use --arg for program arguments)",
        ));
    }
    let cwd = plan
        .resolved
        .container
        .path()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf);
    let env = plan.launch.env_pairs("")?;
    let mut args: Vec<&str> = vec!["run", product];
    args.extend(plan.launch.args.iter().map(String::as_str));
    ctx.out.note(&format!("Running {product} (swift run)"));
    crate::cli::process::stream_env("swift", &args, cwd.as_deref(), &env)
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
            let env = plan.launch.env_pairs("SIMCTL_CHILD_")?;
            let opts = plan.simctl_launch(&env);
            let out = ctx.out.step("Launching app", || {
                simctl::launch_opts(udid, &app.bundle_id, &opts)
            })?;
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
            // `open` can forward arguments but not per-process env.
            let mut args: Vec<String> = vec![app.path.to_string_lossy().into_owned()];
            if !plan.launch.args.is_empty() {
                args.push("--args".to_string());
                args.extend(plan.launch.args.iter().cloned());
            }
            if !plan.launch.env.is_empty() {
                ctx.out
                    .warn("--env is not forwarded through `open`; run without --no-logs to launch the executable directly");
            }
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            crate::cli::process::stream("open", &arg_refs, None)
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
        // Ctrl-C during the initial build cancels the whole run before anything
        // launched — exit as a user cancel (6), not success.
        BuildOutcome::Aborted => {
            return Err(CliError::new("cancelled").kind(ErrorKind::UserCancel));
        }
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

    let mut detach = false;
    loop {
        match rawmode::poll_key() {
            rawmode::Input::Key(ch) => match classify_key(ch) {
                SessionKey::Rebuild => match do_rebuild(ctx, plan, &mut running, &filter) {
                    RebuildOutcome::Continue { launched } => {
                        ever_launched |= launched;
                        session_hint(ctx, filterable);
                    }
                    // Ctrl-C during the rebuild cancels the whole run; fall
                    // through to the shared teardown so a session that never
                    // launched anything still exits non-zero.
                    RebuildOutcome::Quit => break,
                },
                SessionKey::Quit => break,
                // `d`: stop watching but leave the app running.
                SessionKey::Detach => {
                    detach = true;
                    break;
                }
                SessionKey::Screenshot => session_screenshot(ctx, plan),
                SessionKey::Foreground => {
                    let _ = simctl::open_app();
                }
                SessionKey::Clear => ctx.out.line("\x1b[2J\x1b[H"),
                SessionKey::Help => session_keys_help(ctx, filterable),
                // Inert unless an os_log stream is actually being filtered (see
                // `filterable`).
                SessionKey::Filter(level) => {
                    if filterable {
                        set_filter(ctx, &filter, level);
                    }
                }
                SessionKey::Suspend => {
                    // The TSTP handler restores the cooked terminal for the
                    // shell; SIGCONT re-asserts raw mode on `fg`.
                    crate::cli::signals::suspend_self();
                    ctx.out.note("resumed");
                    session_hint(ctx, filterable);
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
        if detach {
            ctx.out
                .note(&format!("detached — {} keeps running", r.name));
            if matches!(r.kind, RunningKind::Mac) {
                ctx.out.warn(
                    "the macOS app's output pipes close when sweetpad exits — its next \
                     print may terminate it; relaunch from Finder for a long-lived detach",
                );
            }
            detach_app(r);
        } else {
            terminate_app(r);
        }
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
/// scrollback. Status lines are progress chatter, so they follow the output
/// contract: stderr (stdout stays the app's own output) and muted by `--quiet`.
/// Without color (non-TTY) every line is a plain committed line.
#[allow(clippy::print_stderr)] // live hot-reload status line, drawn in place
fn hot_logger(out: &Output) -> Logger {
    use std::io::Write as _;
    let color = out.use_color();
    let quiet = out.is_quiet();
    Arc::new(move |m: &str| {
        if quiet {
            return;
        }
        if !color {
            eprintln!("{m}");
        } else if m.ends_with('…') {
            eprint!("\r\x1b[2K\x1b[1;35m{m}\x1b[0m");
            let _ = std::io::stderr().flush();
        } else {
            eprintln!("\r\x1b[2K\x1b[1;35m{m}\x1b[0m");
        }
    })
}

#[allow(clippy::too_many_lines)] // linear session setup/teardown, clearer unsplit
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
    // The guard removes it on every exit path — the early error returns below
    // would otherwise leak a multi-MB transcript per failed run (the pid-keyed
    // name means no later run cleans it up either).
    let work = std::env::temp_dir().join(format!("sweetpad-hot-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&work);
    let _work_guard = RemoveDirOnDrop(work.clone());
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
        // Ctrl-C during the build cancels the hot session before it starts —
        // a user cancel (exit 6), not a success.
        BuildOutcome::Aborted => {
            return Err(CliError::new("cancelled").kind(ErrorKind::UserCancel));
        }
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
    let mut launch_env = inject::client::launch_env(&dylib, &client_opts);
    // User --env pairs ride alongside the injection client's.
    launch_env.extend(plan.launch.env_pairs("SIMCTL_CHILD_")?);
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
        Some(build_log.clone()),
        work,
    ));
    let log = hot_logger(&ctx.out);
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
    launch_hot(ctx, udid, &app, &launch_env, &plan.launch.args)?;
    // Hot reload has no live filter UI; use the default threshold, never cycled.
    let filter = Arc::new(AtomicU8::new(default_filter(&ctx.out).threshold()));
    let mut logs = start_logs(ctx, plan, &filter);
    // Watch the workspace; each save drives `server.inject`.
    let session = HotSession::start(Arc::clone(&server), &project_root);

    // CI self-check: edit a file once, assert `.injected`, exit. Otherwise the
    // interactive key loop (`r`/`q`), or — non-TTY — follow logs until Ctrl-C.
    let mut terminate_on_exit = true;
    let outcome = if let Some(file) = selfcheck {
        hot_selfcheck(ctx, &server, file, udid)
    } else if ctx.out.is_interactive() {
        terminate_on_exit = hot_key_loop(ctx, plan, udid, &launch_env, &mut logs, &build_log);
        Ok(())
    } else {
        ctx.out
            .note("hot reload: watching for Swift changes (Ctrl-C to stop)");
        if let Some(logs) = logs.as_mut() {
            logs.wait();
        }
        Ok(())
    };

    // Teardown: stop watcher + server, terminate the app (unless detached),
    // kill the log stream.
    session.shutdown();
    server.shutdown();
    if terminate_on_exit {
        let _ = simctl::terminate(udid, &app.bundle_id);
    }
    drop(logs);
    outcome
}

/// Removes a scratch directory on drop, so every exit path — including the
/// hot session's early error returns — cleans up after itself.
struct RemoveDirOnDrop(std::path::PathBuf);

impl Drop for RemoveDirOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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

    // A signal can kill the process anywhere in the (long) wait below, after
    // the nonce write but before the restore — so the pristine source is
    // first copied to a sibling backup, and a leftover backup from such a
    // death is restored on entry (the next run self-heals the fixture).
    let backup = selfcheck_backup_path(file);
    if backup.exists() {
        let _ = std::fs::copy(&backup, file);
        let _ = std::fs::remove_file(&backup);
        ctx.out
            .note("hot reload self-check: restored the fixture from a previous run's backup");
    }

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
    std::fs::write(&backup, &original)
        .map_err(|e| CliError::new(format!("self-check: write {}: {e}", backup.display())))?;
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
    // Restore the fixture regardless of outcome, and drop the backup only
    // once the pristine content is verifiably back in place — a failed
    // restore must keep the backup so the next run can self-heal.
    match std::fs::write(file, &original) {
        Ok(()) => {
            let _ = std::fs::remove_file(&backup);
        }
        Err(e) => ctx.out.alert(&format!(
            "self-check: failed to restore {}: {e} (backup kept at {})",
            file.display(),
            backup.display()
        )),
    }

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

/// Where [`hot_selfcheck`] keeps the pristine copy of the fixture while the
/// nonce edit is live: `<file>.sweetpad-selfcheck-backup`, next to the file.
fn selfcheck_backup_path(file: &Path) -> std::path::PathBuf {
    let name = file.file_name().map_or_else(
        || "fixture".to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    file.with_file_name(format!("{name}.sweetpad-selfcheck-backup"))
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

/// Boot, install, and launch the app with the hot-reload env (plus any user
/// `--arg`s). Shared by the first launch and each `r`. Logs stream separately
/// for the whole session ([`start_logs`]), so this doesn't touch them.
fn launch_hot(
    ctx: &Context,
    udid: &str,
    app: &AppBundle,
    env: &[(String, String)],
    args: &[String],
) -> CliResult {
    ctx.out.step("Booting simulator", || simctl::boot(udid))?;
    ctx.out.step("Installing app", || {
        simctl::install(udid, &app.path.display().to_string())
    })?;
    let opts = simctl::LaunchOptions {
        args,
        env,
        wait_for_debugger: false,
    };
    let launched = ctx.out.step("Launching app", || {
        simctl::launch_opts(udid, &app.bundle_id, &opts)
    })?;
    ctx.out
        .note(&format!("Launched {} → {}", app.bundle_id, launched.trim()));
    Ok(())
}

/// The `--hot` keypress loop: `r` full rebuild+relaunch (the client
/// reconnects), `s`/`o`/`c`/`h` as in the plain session, `d` detaches (the app
/// keeps running), `q`/Ctrl-C/Ctrl-D quit. Injection happens out-of-band via
/// the watcher. Returns whether the app should be terminated on teardown
/// (false after a detach).
fn hot_key_loop(
    ctx: &Context,
    plan: &RunPlan,
    udid: &str,
    env: &[(String, String)],
    logs: &mut Option<LogStream>,
    build_log: &Path,
) -> bool {
    let Ok(_raw) = rawmode::RawMode::enable() else {
        // No TTY for raw mode — just follow the log stream until Ctrl-C.
        if let Some(logs) = logs.as_mut() {
            logs.wait();
        }
        return true;
    };
    ctx.out
        .note("hot reload ready · edit a Swift file to inject · r rebuilds · d detaches · q quits");
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
                    // Re-tee the transcript so the build-log recompiler keeps
                    // seeing current frontend commands after the rebuild.
                    let _ = simctl::terminate(udid, &app.bundle_id);
                    match build(plan, &ctx.out, Some(build_log)) {
                        BuildOutcome::Ok => {
                            if let Err(e) = launch_hot(ctx, udid, &app, env, &plan.launch.args) {
                                ctx.out.error(&e);
                            }
                        }
                        BuildOutcome::Failed(e) => ctx.out.error(&e),
                        // Ctrl-C during the rebuild quits the hot session.
                        BuildOutcome::Aborted => break,
                    }
                }
                SessionKey::Quit => break,
                SessionKey::Detach => {
                    ctx.out.note("detached — the app keeps running");
                    return false;
                }
                SessionKey::Screenshot => session_screenshot(ctx, plan),
                SessionKey::Foreground => {
                    let _ = simctl::open_app();
                }
                SessionKey::Clear => ctx.out.line("\x1b[2J\x1b[H"),
                SessionKey::Help => ctx.out.note(
                    "r rebuild+relaunch · s screenshot · o focus simulator · c clear · \
                     d detach · q quit",
                ),
                SessionKey::Suspend => {
                    crate::cli::signals::suspend_self();
                    ctx.out.note("resumed");
                }
                // The hot session has no in-session filter keys — ignore them.
                SessionKey::Filter(_) | SessionKey::Ignore => {}
            },
            rawmode::Input::Idle => {}
            rawmode::Input::Closed => break,
        }
    }
    true
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
    /// The console child's slot in the signal handler's registry, so a
    /// SIGTERM mid-session reaps it alongside the log streams.
    reap_slot: Option<usize>,
}

enum RunningKind {
    /// Terminate via `simctl`; the attached console child (`Running.stream`) is what
    /// liveness is probed on.
    Simulator { udid: String, bundle_id: String },
    /// The console process launched the app; terminate via devicectl, which
    /// addresses processes by the `.app` directory name in their executable
    /// path (there is no terminate-by-bundle-id).
    Device { id: String, app_dir: String },
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
    /// Slot in the signal handler's child registry, so a SIGTERM mid-session
    /// still kills the stream child.
    reap_slot: Option<usize>,
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
        // Deregister before the reap: after wait() the pid can be recycled,
        // and the handler must never signal a stranger.
        crate::cli::signals::unregister_child(self.reap_slot.take());
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
            let env = plan.launch.env_pairs("SIMCTL_CHILD_")?;
            let opts = plan.simctl_launch(&env);
            let mut child = ctx.out.step("Launching app", || {
                simctl::spawn_console(udid, &app.bundle_id, &opts)
            })?;
            render_console(&mut child, ctx.out.use_color(), filter);
            let reap_slot = crate::cli::signals::register_child(child.id());
            Ok(Running {
                stream: Some(child),
                kind: RunningKind::Simulator {
                    udid: udid.clone(),
                    bundle_id: app.bundle_id.clone(),
                },
                name: app.bundle_id,
                reported_exit: false,
                reap_slot,
            })
        }
        Target::Device(id) => {
            ctx.out.step("Installing app on device", || {
                devicectl::install(id, &app_path)
            })?;
            let mut child = devicectl::spawn_console(id, &app.bundle_id)?;
            render_console(&mut child, ctx.out.use_color(), filter);
            let reap_slot = crate::cli::signals::register_child(child.id());
            Ok(Running {
                stream: Some(child),
                kind: RunningKind::Device {
                    id: id.clone(),
                    app_dir: app_dir_name(&app.path),
                },
                name: app.bundle_id,
                reported_exit: false,
                reap_slot,
            })
        }
        Target::Mac => {
            // The direct spawn honors both --arg and --env (no simctl between
            // us and the process).
            let env = plan.launch.env_pairs("")?;
            let mut cmd = std::process::Command::new(app.executable.as_os_str());
            cmd.args(&plan.launch.args)
                .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| {
                CliError::new(format!("failed to run `{}`: {e}", app.executable.display()))
            })?;
            render_console(&mut child, ctx.out.use_color(), filter);
            let reap_slot = crate::cli::signals::register_child(child.id());
            Ok(Running {
                stream: Some(child),
                kind: RunningKind::Mac,
                name: app.bundle_id,
                reported_exit: false,
                reap_slot,
            })
        }
        Target::SpmRun(_) => unreachable!("SPM run does not use the interactive session"),
    }
}

/// Detach from the running app: stop watching without stopping the app (the
/// `d` key). For simulator/device targets the console child is only an
/// observer — reap it and go. A macOS target's streamed child *is* the app,
/// so it's left alone — but its stdout/stderr are pipes into this process,
/// and once the CLI exits their read ends close, so the app's next print
/// raises SIGPIPE and likely terminates it (the session warns on `d`).
fn detach_app(running: Running) {
    let Running {
        stream,
        kind,
        reap_slot,
        ..
    } = running;
    crate::cli::signals::unregister_child(reap_slot);
    match kind {
        RunningKind::Mac => drop(stream),
        RunningKind::Simulator { .. } | RunningKind::Device { .. } => {
            if let Some(mut stream) = stream {
                let _ = stream.kill();
                let _ = stream.wait();
            }
        }
    }
}

/// Terminate the running app and stop its output stream. The session-scoped
/// simulator log stream is left running — it's torn down once, at session end.
/// The `.app` directory name of a built bundle (`/…/My.app` → `My.app`) — the
/// key [`devicectl::terminate`] matches running processes by.
fn app_dir_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn terminate_app(running: Running) {
    let Running {
        stream,
        kind,
        reap_slot,
        ..
    } = running;
    crate::cli::signals::unregister_child(reap_slot);
    match kind {
        RunningKind::Simulator {
            udid, bundle_id, ..
        } => {
            let _ = simctl::terminate(&udid, &bundle_id);
        }
        RunningKind::Device { id, app_dir } => {
            let _ = devicectl::terminate(&id, &app_dir);
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
    let (mut child, reader) = match process::spawn_piped_group("xcodebuild", &args, cwd.as_deref())
    {
        Ok(pair) => pair,
        Err(e) => return BuildOutcome::Failed(e),
    };
    let pid = child.id();
    // The child leads its own process group, so a SIGINT delivered to *us*
    // (e.g. Ctrl-C during the `--hot` initial build, before raw mode is on)
    // must be forwarded or the build keeps running detached.
    crate::cli::signals::set_build_pgid(pid);
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
                match rawmode::poll_key() {
                    rawmode::Input::Key('\u{3}') => {
                        signal_group(pid, libc::SIGINT);
                        aborted.store(true, Ordering::Relaxed);
                        break;
                    }
                    // EOF on stdin (`/dev/null`, a closed pipe): nothing will
                    // ever arrive — stop polling instead of spinning on an
                    // always-readable fd for the whole build.
                    rawmode::Input::Closed => break,
                    rawmode::Input::Key(_) | rawmode::Input::Idle => {}
                }
            }
        }
    });

    // Beautify xcodebuild's merged output on this thread (the same path as
    // [`buildlog::run`], inlined so we own the child for the watcher), also
    // collecting diagnostics for the last-build artifact. Lossy decoding —
    // one bad byte from a run-script must not end the stream and SIGPIPE a
    // still-writing xcodebuild.
    let mut diagnostics: Vec<serde_json::Value> = Vec::new();
    process::read_lines_lossy(reader, &mut |line: &str| {
        if let Some(file) = capture_file.as_mut() {
            let _ = writeln!(file, "{line}");
        }
        let event = buildlog::parse_line(line);
        if matches!(event, buildlog::Event::Diagnostic { .. })
            && let Some(json) = buildlog::event_json(&event)
        {
            diagnostics.push(json);
        }
        if let Some(rendered) = progress.line(line) {
            out.line(rendered.as_str());
        }
    });
    // Erase the spinner before the post-build notes in case nothing ever
    // rendered (e.g. Ctrl-C during the silent prelude).
    drop(progress);

    // The output stream has ended, so the build is exiting: clear the forward
    // target *before* the reap, or a signal in the gap could target a recycled
    // process group.
    crate::cli::signals::clear_build_pgid();
    let status = child.wait();
    done.store(true, Ordering::Relaxed);
    let _ = watcher.join();

    if aborted.load(Ordering::Relaxed) {
        out.note("Build cancelled");
        return BuildOutcome::Aborted;
    }
    xcodebuild::record_build_diagnostics(
        &plan.resolved.container,
        matches!(&status, Ok(s) if s.success()),
        &diagnostics,
    );
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
            let env = plan.launch.env_pairs("SIMCTL_CHILD_")?;
            let launched = simctl::launch_opts(udid, &app.bundle_id, &plan.simctl_launch(&env))?;
            ctx.out
                .note(&format!("Launched {} → {}", app.bundle_id, launched.trim()));
            stream_logs(ctx, udid, &app, &LogFilterArgs::default())
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
            // Direct spawn (inherited stdio) so --arg/--env reach the process.
            let env = plan.launch.env_pairs("")?;
            let status = std::process::Command::new(app.executable.as_os_str())
                .args(&plan.launch.args)
                .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .status()
                .map_err(|e| {
                    CliError::new(format!("failed to run `{}`: {e}", app.executable.display()))
                })?;
            if status.success() {
                Ok(())
            } else {
                Err(
                    CliError::new(format!("{} exited with {status}", app.executable.display()))
                        .context("running the macOS app"),
                )
            }
        }
        Target::SpmRun(_) => unreachable!("SPM run handled before this match"),
    }
}

/// What the session does with a keystroke.
#[derive(Debug, PartialEq, Eq)]
enum SessionKey {
    Rebuild,
    Quit,
    /// Leave the app running and end the session (the flutter `d`).
    Detach,
    /// Save a simulator screenshot into ./sweetpad-shots/.
    Screenshot,
    /// Bring Simulator.app to the foreground.
    Foreground,
    /// Clear the terminal.
    Clear,
    /// Show the key list.
    Help,
    /// Set the live log filter (the `1`–`4` keys).
    Filter(LogFilter),
    /// Suspend to the shell (Ctrl-Z — raw mode eats the real one).
    Suspend,
    Ignore,
}

/// Map a keystroke to a session action (the flutter-run keymap): `r`/`R`
/// rebuild; `d` detaches (app keeps running); `s` screenshot; `o` foregrounds
/// the simulator; `c` clears; `h` lists keys; `q`, Ctrl-C, and Ctrl-D quit;
/// `1`–`4` set the log filter (debug/info/error/off); everything else is
/// ignored. The key is first folded to the Latin letter on its physical position
/// ([`map_key_to_latin`]), so the shortcuts work on non-Latin layouts (Cyrillic
/// `к`/`й`) without switching. (A closed stdin is handled separately as
/// [`rawmode::Input::Closed`].)
fn classify_key(key: char) -> SessionKey {
    match map_key_to_latin(key) {
        'r' | 'R' => SessionKey::Rebuild,
        'q' | 'Q' | '\u{3}' | '\u{4}' => SessionKey::Quit,
        // Ctrl-Z arrives as a byte with ISIG off; hand control back to the
        // shell properly instead of silently ignoring job control.
        '\u{1a}' => SessionKey::Suspend,
        'd' | 'D' => SessionKey::Detach,
        's' | 'S' => SessionKey::Screenshot,
        'o' | 'O' => SessionKey::Foreground,
        'c' | 'C' => SessionKey::Clear,
        'h' | 'H' => SessionKey::Help,
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

/// The session's short key hint; `h` prints the full list.
fn session_hint(ctx: &Context, _filterable: bool) {
    ctx.out.note("r rebuild · d detach · q quit · h keys");
}

/// The full keymap, on `h`. The log-level keys are shown only when there's an
/// os_log stream to filter (the simulator or a macOS app).
fn session_keys_help(ctx: &Context, filterable: bool) {
    ctx.out.note(
        "r rebuild+relaunch · s screenshot · o focus simulator · c clear · \
         d detach (leave the app running) · q quit (terminate the app)",
    );
    if filterable {
        ctx.out
            .note("log level: 1 debug · 2 info · 3 error · 4 off");
    }
}

/// The `s` key: screenshot a simulator target into ./sweetpad-shots/.
fn session_screenshot(ctx: &Context, plan: &RunPlan) {
    let Target::Simulator(udid) = &plan.target else {
        ctx.out.note("screenshots need a simulator target");
        return;
    };
    let name = sim_name(udid).unwrap_or_else(|| "simulator".to_string());
    let path = super::simulator::default_screenshot_path(&name);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(parent);
    }
    match simctl::screenshot(udid, &path.display().to_string()) {
        Ok(()) => ctx.out.note(&format!("📸 {}", path.display())),
        Err(e) => ctx.out.error(&e),
    }
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

/// `▶ scheme · configuration · destination [· hot reload]` — the session
/// header shown before the build, so what's about to run (and what was
/// auto-selected) is clear up front; the build time lands on the `✓ Launched
/// in N.Ns` line once it's known.
fn print_summary(ctx: &Context, plan: &RunPlan) {
    let hot = if plan.hot { " · hot reload on" } else { "" };
    ctx.out.note(&format!(
        "▶ {} · {} · {}{hot}",
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
        // try_wait() reaped the child, so its pid can now be recycled —
        // drop it from the signal handler's registry immediately rather than
        // at session end, or a later SIGTERM could signal a stranger.
        crate::cli::signals::unregister_child(running.reap_slot.take());
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
    let (program, args) = log_command(source, app, level, marker, &LogFilterArgs::default());
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
    filters: &LogFilterArgs,
) -> (&'static str, Vec<String>) {
    use std::fmt::Write as _;
    let exe = predicate_escape(process_name(app));
    // A raw --predicate replaces the default process match wholesale;
    // --subsystem/--category narrow it.
    let mut predicate = filters.predicate.clone().unwrap_or_else(|| {
        format!("process == \"{exe}\" AND (sender == \"{exe}\" OR sender == \"{exe}.debug.dylib\")")
    });
    if let Some(subsystem) = &filters.subsystem {
        let _ = write!(
            predicate,
            " AND subsystem == \"{}\"",
            predicate_escape(subsystem)
        );
    }
    if let Some(category) = &filters.category {
        let _ = write!(
            predicate,
            " AND category == \"{}\"",
            predicate_escape(category)
        );
    }
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
        // Multi-line messages (JSON dumps, stack traces) span several physical
        // lines; only the first matches the syslog shape. Print the rest
        // verbatim while they continue an entry that was just shown, instead
        // of silently dropping lines 2..n of the app's own message.
        let mut continuing = false;
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Some(entry) = pymobiledevice3::parse_line(&line) else {
                if continuing {
                    println!("{line}");
                }
                continue;
            };
            if entry.image != exe && entry.image != debug_dylib {
                continuing = false;
                continue;
            }
            let rendered = oslog::render_fields(
                Some(entry.timestamp),
                entry.level,
                entry.category,
                entry.message,
                color,
            );
            continuing = rendered.level.as_u8() >= filter.load(Ordering::Relaxed);
            if continuing {
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
    let reap_slot = crate::cli::signals::register_child(child.id());
    Some(LogStream {
        child,
        marker,
        reap_slot,
    })
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
    let reap_slot = crate::cli::signals::register_child(child.id());
    // `pymobiledevice3` is a direct child, killed on drop — no reparented `log` to reap.
    Some(LogStream {
        child,
        marker: None,
        reap_slot,
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
    Uninstall,
    Stop,
}

fn simple(
    ctx: &mut Context,
    stage: Stage,
    launch: &LaunchArgs,
    stage_target: &StageTargetArgs,
) -> CommandResult {
    let on_device = stage_target.device || stage_target.device_id.is_some();
    // `stop` acts on the *running* app: when a launch is recorded, use it
    // directly instead of resolving (and possibly prompting for, and
    // remembering) a whole build target just to kill a process. Explicit
    // targeting flags opt out — `app stop --scheme Other` means that scheme's
    // app, not whatever launched last.
    if matches!(stage, Stage::Stop)
        && !on_device
        && !explicit_targeting(ctx)
        && let Some(result) = simple_from_last_launched(ctx, stage)
    {
        return result;
    }

    // Simulator by default (the common headless case); --device/--device-id
    // switch every stage to devicectl.
    let opts = RunOpts {
        device: on_device,
        device_id: stage_target.device_id.as_deref(),
        mac: false,
        no_logs: true,
        hot: false,
        hot_explicit: false,
        hot_mode: Mode::Resolver,
        hot_selfcheck: None,
        launch,
        passthrough: &[],
    };
    let plan = plan(ctx, &opts)?;
    let app = plan.app_bundle()?;

    let report = match &plan.target {
        Target::Simulator(udid) => simple_on_simulator(ctx, stage, &plan, &app, udid)?,
        Target::Device(id) => simple_on_device(ctx, stage, &plan, &app, id)?,
        Target::Mac | Target::SpmRun(_) => {
            return Err(CliError::new(
                "app install/launch/uninstall/stop act on a simulator or device",
            ));
        }
    };
    Ok(Rendered::data(report))
}

/// `app debug`: build + install, launch with `--wait-for-debugger`
/// (the process starts suspended), then attach `lldb -p <pid>` interactively —
/// type `continue` in lldb to resume the app. Simulator-only for now.
fn debug(ctx: &mut Context, launch: &LaunchArgs) -> CommandResult {
    let opts = RunOpts {
        device: false,
        device_id: None,
        mac: false,
        no_logs: true,
        hot: false,
        hot_explicit: false,
        hot_mode: Mode::Resolver,
        hot_selfcheck: None,
        launch,
        passthrough: &[],
    };
    let plan = plan(ctx, &opts)?;
    let Target::Simulator(udid) = &plan.target else {
        return Err(CliError::new(
            "app debug currently supports simulator targets only",
        ));
    };
    let app = build_and_install(&plan, &ctx.out)?;

    let env = plan.launch.env_pairs("SIMCTL_CHILD_")?;
    let mut launch_opts = plan.simctl_launch(&env);
    launch_opts.wait_for_debugger = true;
    let launched = ctx.out.step("Launching app (suspended)", || {
        simctl::launch_opts(udid, &app.bundle_id, &launch_opts)
    })?;

    // `simctl launch` prints `<bundle>: <pid>`.
    let pid = launched
        .trim()
        .rsplit(':')
        .next()
        .map(str::trim)
        .and_then(|p| p.parse::<u32>().ok())
        .ok_or_else(|| {
            CliError::new(format!(
                "could not read the launched pid from simctl output {launched:?}"
            ))
        })?;

    ctx.out.note(&format!(
        "attaching lldb to {} (pid {pid}) — type `continue` to resume the app",
        app.bundle_id
    ));
    // Ctrl-C inside lldb is its break-into-the-debuggee gesture; the terminal
    // delivers it to the whole foreground group, and the CLI dying underneath
    // would orphan lldb against the shell prompt.
    crate::cli::signals::with_sigint_ignored(|| {
        process::stream("lldb", &["-p", &pid.to_string()], None)
    })
    .map(|()| Rendered::Streamed)
    .map_err(|e| e.context("attaching lldb"))
}

/// The simulator side of a lifecycle stage.
fn simple_on_simulator(
    ctx: &mut Context,
    stage: Stage,
    plan: &RunPlan,
    app: &AppBundle,
    udid: &str,
) -> Result<AppStageReport, CliError> {
    Ok(match stage {
        Stage::Install => {
            plan.build_plan()
                .run(&ctx.out)
                .map_err(|e| e.or_kind(ErrorKind::BuildFailure))?;
            ctx.out.step("Booting simulator", || simctl::boot(udid))?;
            ctx.out.step("Installing app", || {
                simctl::install(udid, &app.path.display().to_string())
            })?;
            stage_report(
                "installed",
                &format!("Installed {}", app.bundle_id),
                app,
                udid,
                None,
            )
        }
        Stage::Launch => {
            ctx.out.step("Booting simulator", || simctl::boot(udid))?;
            // Bring the Simulator window up so the launched app is visible (best-effort).
            let _ = simctl::open_app();
            let env = plan.launch.env_pairs("SIMCTL_CHILD_")?;
            let launch_opts = plan.simctl_launch(&env);
            let out = ctx.out.step("Launching app", || {
                simctl::launch_opts(udid, &app.bundle_id, &launch_opts)
            })?;
            let detail = out.trim().to_string();
            stage_report(
                "launched",
                &format!("Launched {} → {detail}", app.bundle_id),
                app,
                udid,
                Some(detail),
            )
        }
        Stage::Uninstall => {
            ctx.out.step("Booting simulator", || simctl::boot(udid))?;
            ctx.out.step("Uninstalling app", || {
                simctl::uninstall(udid, &app.bundle_id)
            })?;
            stage_report(
                "uninstalled",
                &format!("Uninstalled {}", app.bundle_id),
                app,
                udid,
                None,
            )
        }
        Stage::Stop => {
            ctx.out.step("Terminating app", || {
                simctl::terminate(udid, &app.bundle_id)
            })?;
            stage_report(
                "terminated",
                &format!("Terminated {}", app.bundle_id),
                app,
                udid,
                None,
            )
        }
    })
}

/// The devicectl side of a lifecycle stage.
fn simple_on_device(
    ctx: &mut Context,
    stage: Stage,
    plan: &RunPlan,
    app: &AppBundle,
    id: &str,
) -> Result<AppStageReport, CliError> {
    Ok(match stage {
        Stage::Install => {
            plan.build_plan()
                .run(&ctx.out)
                .map_err(|e| e.or_kind(ErrorKind::BuildFailure))?;
            ctx.out.step("Installing app on device", || {
                devicectl::install(id, &app.path.display().to_string())
            })?;
            stage_report(
                "installed",
                &format!("Installed {} on device", app.bundle_id),
                app,
                id,
                None,
            )
        }
        Stage::Launch => {
            let out = ctx.out.step("Launching app on device", || {
                devicectl::launch(id, &app.bundle_id)
            })?;
            let detail = out.trim().to_string();
            stage_report(
                "launched",
                &format!("Launched {} on device → {detail}", app.bundle_id),
                app,
                id,
                Some(detail),
            )
        }
        Stage::Uninstall => {
            ctx.out.step("Uninstalling app from device", || {
                devicectl::uninstall(id, &app.bundle_id)
            })?;
            stage_report(
                "uninstalled",
                &format!("Uninstalled {} from device", app.bundle_id),
                app,
                id,
                None,
            )
        }
        Stage::Stop => {
            ctx.out.step("Terminating app on device", || {
                devicectl::terminate(id, &app_dir_name(&app.path))
            })?;
            stage_report(
                "terminated",
                &format!("Terminated {} on device", app.bundle_id),
                app,
                id,
                None,
            )
        }
    })
}

fn stage_report(
    action: &'static str,
    note: &str,
    app: &AppBundle,
    udid: &str,
    detail: Option<String>,
) -> AppStageReport {
    AppStageReport {
        action,
        note: note.to_string(),
        bundle_id: app.bundle_id.clone(),
        udid: udid.to_string(),
        detail,
    }
}

/// Serve `app logs`/`app stop` from the recorded last launch when it targeted a
/// simulator — no scheme resolution, no build-settings query, no prompting.
/// `None` (fall back to the full plan) when nothing was recorded or the record
/// is for a device/macOS run.
fn simple_from_last_launched(ctx: &mut Context, stage: Stage) -> Option<CommandResult> {
    let (udid, app) = last_launched_sim(ctx)?;
    Some(match stage {
        Stage::Stop => ctx
            .out
            .step("Terminating app", || {
                simctl::terminate(&udid, &app.bundle_id)
            })
            .map(|()| {
                Rendered::data(AppStageReport {
                    action: "terminated",
                    note: format!("Terminated {}", app.bundle_id),
                    bundle_id: app.bundle_id.clone(),
                    udid: udid.clone(),
                    detail: None,
                })
            }),
        Stage::Install | Stage::Launch | Stage::Uninstall => {
            unreachable!("gated to Stop by the caller")
        }
    })
}

/// `app logs` — its own entry point so the stream filters reach
/// [`stream_logs`]. Uses the recorded last launch when available, else the
/// resolved build target.
fn simple_logs(ctx: &mut Context, filters: &LogFilterArgs) -> CommandResult {
    if !explicit_targeting(ctx)
        && let Some((udid, app)) = last_launched_sim(ctx)
    {
        ctx.out.step("Booting simulator", || simctl::boot(&udid))?;
        stream_logs(ctx, &udid, &app, filters)?;
        return Ok(Rendered::Streamed);
    }
    let opts = RunOpts {
        device: false,
        device_id: None,
        mac: false,
        no_logs: true,
        hot: false,
        hot_explicit: false,
        hot_mode: Mode::Resolver,
        hot_selfcheck: None,
        launch: &LaunchArgs::default(),
        passthrough: &[],
    };
    let plan = plan(ctx, &opts)?;
    let Target::Simulator(udid) = &plan.target else {
        return Err(CliError::new(
            "app logs is only supported for simulator targets",
        ));
    };
    let app = plan.app_bundle()?;
    // Boot first so the stream attaches instead of failing with "device is
    // not booted" when the simulator is shut down.
    ctx.out.step("Booting simulator", || simctl::boot(udid))?;
    stream_logs(ctx, udid, &app, filters)?;
    Ok(Rendered::Streamed)
}

/// Whether the invocation named its target explicitly (scheme, configuration,
/// destination, or `--on`) — the last-launched fast paths yield to it.
/// Container flags don't count: the recorded launch is already keyed per
/// container.
fn explicit_targeting(ctx: &Context) -> bool {
    ctx.targeting.scheme.is_some()
        || ctx.targeting.configuration.is_some()
        || ctx.targeting.destination.is_some()
        || ctx.targeting.on.is_some()
}

/// The recorded last launch, when it targeted a simulator: `(udid, bundle)`.
fn last_launched_sim(ctx: &Context) -> Option<(String, AppBundle)> {
    let container = resolve::container(ctx).ok()?;
    let last = ctx
        .state
        .projects
        .get(&container.key())?
        .last_launched_app
        .clone()?;
    if last.kind != "simulator" {
        return None;
    }
    let udid = last.simulator_udid.clone()?;
    let app = AppBundle {
        path: std::path::PathBuf::from(&last.app_path),
        bundle_id: last.bundle_identifier.clone(),
        executable: std::path::PathBuf::from(last.executable_name.as_deref().unwrap_or_default()),
    };
    Some((udid, app))
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
/// Under `--json` each os_log event is passed through as the raw NDJSON object
/// `log stream --style ndjson` produced — one JSON event per line on stdout —
/// instead of the human-rendered text (a stream has no single success envelope
/// to wrap).
///
/// Interruption runs through the handler's forward-only mode: Ctrl-C/SIGTERM
/// forward SIGINT to the `simctl` child and *return*, so this function
/// finishes normally — which is what lets it `pkill` the `log` process the
/// simulator reparented to `launchd_sim` (a marker rides in the predicate;
/// without this reap every stopped `app logs` left one streaming forever) —
/// and a user-stopped follow exits 0.
#[allow(clippy::print_stdout)] // non-interactive inline log follow
fn stream_logs(ctx: &Context, udid: &str, app: &AppBundle, filters: &LogFilterArgs) -> CliResult {
    ctx.out.note(&format!(
        "Streaming logs for {} (Ctrl-C to stop)",
        app.bundle_id
    ));
    let color = ctx.out.use_color();
    let json = ctx.out.is_json() || ctx.out.is_ndjson();
    let level = filters.level.as_deref().unwrap_or(log_level(&ctx.out));
    let marker = log_stream_marker();
    let (program, args) = log_command(
        &LogSource::Simulator(udid),
        app,
        level,
        Some(&marker),
        filters,
    );
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut child = process::spawn_piped(program, &refs, None)?;
    let reap_slot = crate::cli::signals::register_child(child.id());
    crate::cli::signals::set_forward_child(child.id());
    if let Some(stdout) = child.stdout.take() {
        process::read_lines_lossy(stdout, &mut |line: &str| {
            if json {
                // Already one JSON object per line; emit the event verbatim.
                if line.trim_start().starts_with('{') {
                    println!("{line}");
                }
            } else {
                println!("{}", oslog::render_ndjson_line(line, color).text);
            }
        });
    }
    // The stream has ended (child exiting): disarm before the reap so the
    // handler can never signal a recycled pid.
    crate::cli::signals::clear_forward_child();
    crate::cli::signals::unregister_child(reap_slot);
    let status = child.wait();
    let _ = process::run("pkill", &["-f", &marker], None, true);
    if crate::cli::signals::take_forwarded() {
        ctx.out.note("log stream stopped");
        return Ok(());
    }
    match status {
        Ok(s) if s.success() => Ok(()),
        _ => Err(CliError::new("log stream exited with a non-zero status")),
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
            .kind(ErrorKind::TargetResolution)
        })
}

/// The simulator a destination addresses: `id=` names it outright; a `name=`
/// specifier — the form the config examples use, valid for xcodebuild — is
/// resolved against `simctl list` (booted preferred, `simctl::find`'s
/// policy), so `run` accepts every destination `build` does.
fn destination_udid(destination: &str) -> Result<String, CliError> {
    if let Ok(u) = udid(destination) {
        return Ok(u);
    }
    let Some(name) = destination
        .split(',')
        .find_map(|kv| kv.trim().strip_prefix("name="))
    else {
        return Err(CliError::new(format!(
            "app commands need a destination with an id= or name= (got {destination:?})"
        ))
        .kind(ErrorKind::TargetResolution));
    };
    let sims = simctl::list()?;
    simctl::find(&sims, name)
        .map(|s| s.udid.clone())
        .ok_or_else(|| {
            CliError::new(format!(
                "the destination names the simulator {name:?}, but no such simulator exists"
            ))
            .kind(ErrorKind::TargetResolution)
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
        // Ctrl-Z suspends (raw mode eats the real one).
        assert_eq!(classify_key('\u{1a}'), SessionKey::Suspend);
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
        let (program, args) = log_command(
            &LogSource::Simulator("UDID-1"),
            &app,
            "info",
            Some("tag-7"),
            &LogFilterArgs::default(),
        );
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
        let (program, args) = log_command(
            &LogSource::Mac,
            &app,
            "debug",
            None,
            &LogFilterArgs::default(),
        );
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
