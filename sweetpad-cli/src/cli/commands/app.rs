//! `sweetpad app …` — the built app's lifecycle: build+install+launch, and the
//! running session, on a simulator or a physical device. The app is the noun;
//! these are its actions.

use std::path::Path;
use std::process::Child;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use clap::Subcommand;

mod ax;
mod macwin;

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
    /// ('--on device' is the same thing). Conflicts with a *typed* '--on'
    /// post-parse — a clap-level conflict would also fire on an env-sourced
    /// SWEETPAD_ON, breaking flag-beats-env.
    #[arg(long)]
    pub device: bool,

    /// Specific device UDID/name to target (implies --device).
    #[arg(long = "device-id")]
    pub device_id: Option<String>,

    /// Build and run as a native macOS app ('--on mac' is the same thing).
    #[arg(long, conflicts_with_all = ["device", "device_id"])]
    pub mac: bool,

    /// Don't stream the app's logs after launching (logs follow by default
    /// on simulators).
    #[arg(long = "no-logs")]
    pub no_logs: bool,

    /// Build, launch, and return, leaving the app running after the CLI
    /// exits. On macOS the app is spawned in its own session with its output
    /// redirected to a log file, so '--env' is honored and a later print
    /// can't kill it; on a simulator or device the app already outlives the
    /// CLI, so this behaves like '--no-logs'.
    #[arg(long)]
    pub detach: bool,

    /// Enable hot reload (iOS Simulator and native macOS apps): on each Swift
    /// save the file is recompiled and injected into the running app — no
    /// relaunch, state preserved. Requires the injection client (see
    /// CLI_DESIGN §9d). A project can default this on via '[run] hot = true'
    /// in sweetpad.toml.
    #[arg(long)]
    pub hot: bool,

    /// Disable hot reload for this run (overrides a '[run] hot = true'
    /// project default).
    #[arg(long = "no-hot", conflicts_with = "hot")]
    pub no_hot: bool,

    /// Hot-reload recompiler.
    #[arg(long = "hot-recompiler", value_name = "MODE", value_enum)]
    pub hot_recompiler: Option<HotRecompiler>,

    /// Keep the App Sandbox for a '--hot' macOS run instead of the automatic
    /// ephemeral un-sandboxing; a sandboxed product then fails the injection
    /// preflight with the manual fix.
    #[arg(long = "keep-sandbox")]
    pub keep_sandbox: bool,

    /// Sign the '--hot' macOS build with this entitlements file instead of
    /// auto-deriving a sandbox-stripped one.
    #[arg(
        long = "hot-entitlements",
        value_name = "FILE",
        conflicts_with = "keep_sandbox"
    )]
    pub hot_entitlements: Option<std::path::PathBuf>,

    /// CI self-check (hidden): with '--hot', after launch edit FILE once, wait
    /// for '.injected', and exit 0/1 instead of entering the session. Drives
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

    /// Extra arguments passed to xcodebuild verbatim (after '--').
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

    /// Launch suspended, waiting for a debugger to attach ('lldb -p <pid>').
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
}

/// `app debug --batch` — non-interactive lldb for scripts and agents. Without
/// '--batch', `app debug` still drops you at an interactive lldb prompt.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct DebugBatchArgs {
    /// Run lldb non-interactively: execute the '--cmd' commands, then let the
    /// session end, instead of handing over an interactive prompt. The exit
    /// code reflects whether the session launched, not what lldb found — parse
    /// the streamed output for the result, or use 'app diagnose' for a report.
    #[arg(long)]
    pub batch: bool,

    /// An lldb command to run in '--batch' mode (repeatable, in order) —
    /// lldb's own '-o/--one-line' (sweetpad's '-o' already selects the output
    /// format). Commands are forwarded verbatim, so include your own
    /// 'run'/'continue' and 'quit'.
    #[arg(long = "cmd", value_name = "LLDB_CMD", requires = "batch")]
    pub cmd: Vec<String>,

    /// An lldb command to run only if the target crashes in '--batch' mode
    /// (repeatable) — lldb's '-k/--one-line-on-crash'.
    #[arg(long = "on-crash", value_name = "LLDB_CMD", requires = "batch")]
    pub on_crash: Vec<String>,

    /// Kill the '--batch' session after this many seconds (0 disables) so an
    /// unattended run can't block forever on an app that stays up. Default 300.
    #[arg(long, value_name = "SECS", default_value_t = 300)]
    pub timeout: u64,
}

/// Which output channels `app logs` follows on macOS. A Mac app can log through
/// the unified log (`os_log`/`Logger`) or plain stdout/stderr (`print`), and the
/// two never overlap — following only one would silently miss the other.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum LogChannel {
    /// The app's unified log (`os_log`/`Logger`), via `log stream`/`log show`.
    Oslog,
    /// The stdout/stderr a detached launch captured to a file (see
    /// [`detached_log_path`]); `print`, `NSLog`'s stderr leg, C `printf`.
    Stdout,
    /// Both, interleaved by arrival (the default).
    #[default]
    Both,
}

/// `app logs` stream shaping: narrow the predicate, change the level, pick the
/// macOS channel, or ask for recent history. Subsystem/category/predicate/level
/// shape the os_log stream (`log stream` natively); physical-device logs
/// (pymobiledevice3) keep their fixed process filter.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct LogFilterArgs {
    /// Only entries from this subsystem (e.g. com.example.app.networking).
    #[arg(long)]
    pub subsystem: Option<String>,

    /// Only entries from this category.
    #[arg(long)]
    pub category: Option<String>,

    /// A raw 'log stream' predicate, replacing the default process match
    /// entirely (the escape hatch).
    #[arg(long, conflicts_with_all = ["subsystem", "category"])]
    pub predicate: Option<String>,

    /// Minimum level to stream: default, info, or debug. Streams at 'info'
    /// unless '-v', so an app's own debug entries stay hidden until you pass
    /// '--level debug'. The system does not persist debug entries (nor, usually,
    /// info), so those exist only while following live — '--last' cannot show
    /// them however low this is set.
    #[arg(long, value_parser = ["default", "info", "debug"])]
    pub level: Option<String>,

    /// On macOS, which output to follow: the app's os_log ('oslog'), the
    /// stdout/stderr a detached launch captured ('stdout'), or 'both' (the
    /// default). Simulator and device logs are os_log only.
    #[arg(long, value_enum, default_value_t = LogChannel::Both)]
    pub source: LogChannel,

    /// Print the last DUR of history and exit instead of following — e.g.
    /// '2m', '90s', '1h'. Post-mortem for an app that has gone quiet or exited:
    /// os_log via 'log show' (which retains history 'log stream' can't), plus
    /// the captured stdout/stderr on macOS. Reaches only what the system
    /// persisted, so an app's debug entries are never here — follow with
    /// '--level debug' to see those. Not available for physical devices.
    #[arg(long, value_name = "DUR")]
    pub last: Option<String>,

    /// Stop following as soon as a line contains TEXT, and exit 0 — 'run this,
    /// wait for that' as one call, instead of a background stream and a guessed
    /// sleep. Plain substring (not a pattern), matched against the rendered
    /// line, so it reads the same as what prints.
    #[arg(long, value_name = "TEXT", conflicts_with = "last")]
    pub until: Option<String>,

    /// Give up following after DUR — e.g. '30s', '2m', '1h'. On its own it
    /// bounds the follow and exits 0; with '--until' it is the deadline for the
    /// match, and missing it exits non-zero.
    #[arg(long, value_name = "DUR", conflicts_with = "last", value_parser = parse_duration)]
    pub timeout: Option<Duration>,
}

/// Parse a `30s` / `2m` / `1h` duration; a bare number is seconds.
fn parse_duration(spec: &str) -> Result<Duration, CliError> {
    let bad = || {
        CliError::new(format!(
            "invalid duration {spec:?} — use a count with a unit, e.g. '30s', '2m', '1h'"
        ))
    };
    let spec = spec.trim();
    let (digits, scale) = match spec.strip_suffix(['s', 'm', 'h']) {
        Some(head) => (
            head,
            match spec.as_bytes().last() {
                Some(b'm') => 60,
                Some(b'h') => 3600,
                _ => 1,
            },
        ),
        None => (spec, 1),
    };
    let count: u64 = digits.parse().map_err(|_| bad())?;
    if count == 0 {
        return Err(bad());
    }
    Ok(Duration::from_secs(count * scale))
}

/// The `--until` stop condition, shared by the os_log stream and (on macOS) the
/// captured stdout tail so a match on either ends the follow. Sighting is
/// recorded in `hit`; ending the follow means SIGTERM-ing the `log stream`
/// child, whose EOF unblocks the reader.
struct UntilWatch {
    text: String,
    hit: Arc<AtomicBool>,
    /// The `log stream` child to end on a match. `None` under '--source stdout',
    /// where the tail owns the thread and returns on its own.
    stream_pid: Option<u32>,
}

impl UntilWatch {
    /// Record a sighting when `rendered` contains the watched text, and end the
    /// follow. Returns whether this line matched.
    fn sees(&self, rendered: &str) -> bool {
        if self.hit.load(Ordering::Relaxed) || !rendered.contains(&self.text) {
            return false;
        }
        self.hit.store(true, Ordering::Relaxed);
        if let Some(pid) = self.stream_pid {
            process::terminate(pid);
        }
        true
    }
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

    /// Act on the native macOS app ('--on mac' is the same thing).
    #[arg(long, conflicts_with_all = ["device", "device_id"])]
    pub mac: bool,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Build, install, launch, and follow logs; at an interactive terminal,
    /// press 'r' to rebuild on demand.
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
    /// Debug under lldb: on a simulator, launch suspended and attach; on
    /// macOS, hand the executable to lldb and `run` it. '--batch' drives lldb
    /// non-interactively from '--cmd' commands, for scripts and agents.
    Debug {
        #[command(flatten)]
        target: crate::cli::BuildTargetArgs,
        #[command(flatten)]
        stage: StageTargetArgs,
        #[command(flatten)]
        launch: LaunchArgs,
        #[command(flatten)]
        batch: DebugBatchArgs,
    },
    /// Run the app under lldb, catch the first Objective-C exception or crash,
    /// print a structured report, and quit. Built for unattended/agent use:
    /// bounded by '--timeout', and the result is the report ('-o json' for the
    /// machine-readable form), not the exit code. Simulator and macOS only.
    Diagnose {
        #[command(flatten)]
        target: crate::cli::BuildTargetArgs,
        #[command(flatten)]
        stage: StageTargetArgs,
        #[command(flatten)]
        launch: LaunchArgs,
        /// Give up after this many seconds if the app neither crashes nor
        /// exits, killing it and reporting a timeout (0 disables). Default 30.
        #[arg(long, value_name = "SECS", default_value_t = 30)]
        timeout: u64,
    },
    /// Remove the app from a simulator or device.
    Uninstall {
        #[command(flatten)]
        target: crate::cli::BuildTargetArgs,
        #[command(flatten)]
        stage: StageTargetArgs,
    },
    /// Stream the running app's logs — simulator, device, or macOS. Uses the
    /// last-launched app when one is recorded; otherwise resolves the build
    /// target. On macOS it follows both the app's os_log and the stdout/stderr a
    /// detached launch captured ('--source' narrows this); '--last <dur>' prints
    /// recent history and exits instead of following. With --json, emits one
    /// JSON object per line instead of the rendered text.
    Logs {
        #[command(flatten)]
        target: crate::cli::BuildTargetArgs,
        #[command(flatten)]
        stage: StageTargetArgs,
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
        /// The URL to open (e.g. 'myapp://path' or 'https://example.com/x').
        url: String,
        /// Simulator name or UDID to open it on (defaults to the booted one).
        #[arg(long)]
        simulator: Option<String>,
    },
    /// Save a PNG screenshot of the running app: a macOS app's window, or
    /// the simulator it launched on.
    Screenshot(ScreenshotArgs),
    /// Inspect or drive a running macOS app's UI through accessibility
    /// ('ui' alone runs 'ui tree').
    Ui {
        #[command(subcommand)]
        action: Option<UiAction>,
    },
}

/// The `app ui` verbs (CLI_DESIGN §9i).
#[derive(Debug, Subcommand)]
pub enum UiAction {
    /// Print the app's accessibility tree — every element it exposes, with
    /// the labels and roles the other verbs match on.
    Tree(UiTreeArgs),
    /// Press a control (the 'AXPress' action a button or menu item offers).
    Click(UiSelectArgs),
    /// Replace a text field's contents (sets 'AXValue'; this is not
    /// keystroke synthesis, so an app watching for individual key events may
    /// not react).
    Type(UiTypeArgs),
}

/// How every `app ui` verb finds the running macOS app.
#[derive(Debug, clap::Args)]
pub struct UiAppArgs {
    #[command(flatten)]
    pub target: crate::cli::BuildTargetArgs,

    /// Drive this process directly, skipping app resolution (for macOS
    /// processes sweetpad didn't launch).
    #[arg(long, value_name = "PID")]
    pub pid: Option<i32>,
}

/// Flags for `app ui tree`.
#[derive(Debug, clap::Args)]
pub struct UiTreeArgs {
    #[command(flatten)]
    pub app: UiAppArgs,

    /// How many levels below the application element to descend.
    #[arg(long, value_name = "N", default_value_t = 20)]
    pub depth: usize,
}

/// Which element a verb acts on. Matching nothing, or several without
/// '--nth', is an error rather than a guess.
#[derive(Debug, clap::Args)]
pub struct UiQueryArgs {
    /// The element's identifier or visible label. Exact matches win over
    /// substring ones, and case is ignored.
    #[arg(long, value_name = "TEXT")]
    pub label: Option<String>,

    /// Restrict to one role, e.g. 'button', 'textfield', 'menuitem' (the
    /// 'AX' prefix is optional).
    #[arg(long, value_name = "ROLE")]
    pub role: Option<String>,

    /// Which match to take when several tie: 1-based, front-to-back.
    #[arg(long, value_name = "N")]
    pub nth: Option<usize>,
}

/// Flags for `app ui click`.
#[derive(Debug, clap::Args)]
pub struct UiSelectArgs {
    #[command(flatten)]
    pub app: UiAppArgs,
    #[command(flatten)]
    pub query: UiQueryArgs,
}

/// Flags for `app ui type`.
#[derive(Debug, clap::Args)]
pub struct UiTypeArgs {
    /// The text to put in the field.
    pub text: String,
    #[command(flatten)]
    pub app: UiAppArgs,
    #[command(flatten)]
    pub query: UiQueryArgs,
}

/// Flags for `app screenshot` (CLI_DESIGN §9h).
#[derive(Debug, clap::Args)]
pub struct ScreenshotArgs {
    #[command(flatten)]
    pub target: crate::cli::BuildTargetArgs,

    /// File to write the screenshot to (default:
    /// ./sweetpad-shots/<app>-<time>.png).
    // `--output-file`, not `--output`: the global `-o/--output` selects the
    // output *mode* (json/ndjson) on every command, so it owns that flag.
    #[arg(long = "output-file")]
    pub output_file: Option<std::path::PathBuf>,

    /// Which window to capture when the app has several: 1-based,
    /// front-to-back (default: the frontmost).
    #[arg(long, value_name = "N")]
    pub window: Option<usize>,

    /// Capture this process's window directly, skipping app resolution
    /// (for macOS processes sweetpad didn't launch).
    #[arg(long, value_name = "PID")]
    pub pid: Option<i32>,

    /// Also copy the screenshot to the clipboard.
    #[arg(long)]
    pub clipboard: bool,
}

impl Action {
    /// The default action for a bare `sweetpad app`: `app run` with no flags.
    /// clap never parses `RunArgs` on this path, so its `env = …` attrs never
    /// run — the `SWEETPAD_*` layer is folded in by hand, or a bare
    /// `sweetpad app` would silently ignore an env context that
    /// `sweetpad app run` honors.
    #[must_use]
    pub fn default_run() -> Self {
        let env = crate::cli::Targeting::from_env();
        Action::Run(RunArgs {
            target: crate::cli::BuildTargetArgs {
                scheme: crate::cli::SchemeArgs {
                    container: crate::cli::ContainerArgs {
                        workspace: env.workspace,
                        project: env.project,
                    },
                    scheme: env.scheme,
                },
                configuration: env.configuration,
                destination: env.destination,
                on: env.on,
                sdk: env.sdk,
            },
            ..RunArgs::default()
        })
    }
}

/// Settle hot reload for a run: the `--hot` flag, else the project's
/// `[run] hot` default (opted out per run with `--no-hot`); the recompiler
/// from `--hot-recompiler`, else `[run] hot_recompiler`. Target enforcement
/// (simulator or mac) stays in [`run_hot_session`]; here a `[run] hot = true`
/// project default is simply ignored for `--device` runs rather than erroring
/// on a committed file.
fn hot_settings(ctx: &Context, args: &RunArgs) -> (bool, Mode) {
    let run_defaults = resolve::container(ctx)
        .ok()
        .map(|c| ctx.project_file(&c).run.clone())
        .unwrap_or_default();
    let default_hot = run_defaults.hot.unwrap_or(false) && !args.device && args.device_id.is_none();
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
            crate::cli::settle_on_vs_mode(
                &mut ctx.targeting,
                args.mac || args.device || args.device_id.is_some(),
            )?;
            let (hot, hot_mode) = hot_settings(ctx, args);
            // The live build-and-run session streams its own output until you quit.
            run_app(
                ctx,
                &RunOpts {
                    device: args.device || args.device_id.is_some(),
                    device_id: args.device_id.as_deref(),
                    mac: args.mac,
                    no_logs: args.no_logs,
                    detach: args.detach,
                    hot,
                    hot_explicit: args.hot,
                    hot_mode,
                    hot_selfcheck: args.hot_selfcheck.as_deref(),
                    keep_sandbox: args.keep_sandbox,
                    hot_entitlements: args.hot_entitlements.as_deref(),
                    launch: &args.launch,
                    passthrough: &args.passthrough,
                },
            )
        }
        Action::Install { target, stage } => {
            ctx.targeting = target.clone().into();
            settle_stage_mode(ctx, stage)?;
            simple(ctx, Stage::Install, &LaunchArgs::default(), stage)
        }
        Action::Launch {
            target,
            stage,
            launch,
        } => {
            ctx.targeting = target.clone().into();
            settle_stage_mode(ctx, stage)?;
            simple(ctx, Stage::Launch, launch, stage)
        }
        Action::Debug {
            target,
            stage,
            launch,
            batch,
        } => {
            ctx.targeting = target.clone().into();
            settle_stage_mode(ctx, stage)?;
            debug(ctx, stage, launch, batch)
        }
        Action::Diagnose {
            target,
            stage,
            launch,
            timeout,
        } => {
            ctx.targeting = target.clone().into();
            settle_stage_mode(ctx, stage)?;
            diagnose(ctx, stage, launch, *timeout)
        }
        Action::Uninstall { target, stage } => {
            ctx.targeting = target.clone().into();
            settle_stage_mode(ctx, stage)?;
            simple(ctx, Stage::Uninstall, &LaunchArgs::default(), stage)
        }
        Action::Logs {
            target,
            stage,
            filters,
        } => {
            ctx.targeting = target.clone().into();
            settle_stage_mode(ctx, stage)?;
            simple_logs(ctx, stage, filters)
        }
        Action::Stop { target, stage } => {
            ctx.targeting = target.clone().into();
            settle_stage_mode(ctx, stage)?;
            simple(ctx, Stage::Stop, &LaunchArgs::default(), stage)
        }
        Action::OpenUrl { url, simulator } => open_url(ctx, url, simulator.as_deref()),
        Action::Screenshot(args) => {
            ctx.targeting = args.target.clone().into();
            screenshot(ctx, args)
        }
        Action::Ui { action } => ui(ctx, action.as_ref()),
    }
}

/// One mode-vs-`--on` policy for the lifecycle stages, matching `app run`:
/// a typed `--device`/`--device-id` beats an env-sourced `SWEETPAD_ON`
/// (instead of silently losing to it), and a typed `--on` alongside them is
/// rejected.
fn settle_stage_mode(ctx: &mut Context, stage: &StageTargetArgs) -> Result<(), CliError> {
    crate::cli::settle_on_vs_mode(
        &mut ctx.targeting,
        stage.device || stage.device_id.is_some(),
    )
}

/// Open a URL on a simulator. Unlike the install/launch lifecycle, this needs
/// no scheme or build — just a target simulator — so it resolves one directly
/// rather than going through the build plan.
fn open_url(ctx: &mut Context, url: &str, simulator: Option<&str>) -> CommandResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, simulator, "--simulator <name|udid>")?;
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
    /// `--detach`: launch and return, leaving the app running.
    detach: bool,
    hot: bool,
    /// Whether `--hot` was typed (vs. the `[run] hot` config default) — a
    /// config default is silently ignored for non-simulator targets instead
    /// of erroring on a committed file.
    hot_explicit: bool,
    hot_mode: Mode,
    hot_selfcheck: Option<&'a Path>,
    /// `--keep-sandbox`: don't auto-strip a hot macOS build's App Sandbox.
    keep_sandbox: bool,
    /// `--hot-entitlements`: sign the hot macOS build with this file.
    hot_entitlements: Option<&'a Path>,
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
    /// The entitlements file a hot macOS build signs with — the ephemeral
    /// sandbox-stripped copy or a `--hot-entitlements` file (§9d zero-config
    /// sandbox stripping). Settled once in [`plan`], so session rebuilds
    /// (`r`) keep the same signing posture.
    hot_entitlements: Option<std::path::PathBuf>,
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
            hot_entitlements: self.hot_entitlements.as_deref(),
            passthrough: &self.passthrough,
        }
    }

    /// The `-derivedDataPath` the passthrough hands xcodebuild, if any — the
    /// app locator must look where the build actually put the product.
    /// Product-relocating build settings the locator can't model (`SYMROOT=`,
    /// `OBJROOT=`, `CONFIGURATION_BUILD_DIR=`) are refused loudly: silently
    /// looking in the default DerivedData would install whatever stale `.app`
    /// a previous plain build left there.
    fn passthrough_derived_data(&self) -> Result<Option<std::path::PathBuf>, CliError> {
        let mut derived_data = None;
        let mut iter = self.passthrough.iter().peekable();
        while let Some(arg) = iter.next() {
            if arg == "-derivedDataPath" {
                derived_data = iter.peek().map(std::path::PathBuf::from);
            } else if let Some((key, _)) = arg.split_once('=')
                && matches!(key, "SYMROOT" | "OBJROOT" | "CONFIGURATION_BUILD_DIR")
            {
                return Err(CliError::new(format!(
                    "`-- {key}=…` relocates the built product where the app locator can't \
                     follow; use `-- -derivedDataPath <dir>` instead"
                )));
            }
        }
        Ok(derived_data)
    }

    /// Resolve every target's build settings via the in-process resolver (the
    /// engine behind `settings show`), with no xcodebuild spawn — including a
    /// passthrough `-derivedDataPath`. Swift packages never reach here — they
    /// run via `swift run`, not a build/install/launch.
    fn resolved_settings(&self) -> Result<Vec<xcodebuild::TargetBuildSettings>, CliError> {
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
            derived_data_path: self.passthrough_derived_data()?,
            // We go on to install and launch what this resolves, so it has to
            // name the bundle `xcodebuild` actually wrote — including when the
            // user has moved Derived Data in Xcode (issue #306).
            read_xcode_locations: true,
            keys: None,
        };
        let resolved =
            sweetpad_core::build_settings::resolve_build_settings(&opts).map_err(CliError::new)?;
        Ok(resolved
            .into_iter()
            .map(|t| xcodebuild::TargetBuildSettings {
                target: t.target,
                settings: t.settings,
            })
            .collect())
    }

    /// Locate the built `.app`: [`resolved_settings`](Self::resolved_settings)
    /// computes the same TARGET_BUILD_DIR/product the build produced. The
    /// destination narrows multi-app schemes (iOS + watch companion) to the
    /// app that actually runs there.
    fn app_bundle(&self) -> Result<AppBundle, CliError> {
        let settings = self.resolved_settings()?;
        xcodebuild::app_bundle(&settings, Some(&self.destination))
    }
}

/// Whether this run drives a hot-reload session. A typed `--hot` holds for any
/// target — [`run_hot_session`] is what refuses devices and SPM executables —
/// while the `[run] hot = true` config default auto-applies to simulators
/// only: on macOS you type `--hot`, so a committed file can't break `--on mac`,
/// device, or SPM runs. The flag-gated variants are pre-filtered in
/// `hot_settings`, but only the resolved plan knows about `--on` and remembered
/// macOS destinations, so the decision lands here.
fn session_hot(hot: bool, explicit: bool, target: &Target) -> bool {
    hot && (explicit || matches!(target, Target::Simulator(_)))
}

/// Whether machine-readable output was asked for, and this invocation streams —
/// so there is no coherent one-shot payload to emit. `--no-logs` and `--detach`
/// deploy and return, so they *do* have one; the session forms print logs until
/// you quit, and a `--json` run of those would emit a silent build followed by
/// human-formatted logs.
fn streaming_under_machine_output(out: &Output, streams: bool) -> Option<CliError> {
    (streams && (out.is_json() || out.is_ndjson())).then(|| {
        CliError::new(
            "this `app run` streams a live session and has no machine-readable form; add \
             `--no-logs` (build, install, launch, and exit) or `--detach`, or use \
             `build start -o ndjson`, `app install`/`app launch --json`, and \
             `app logs -o ndjson` as separate steps",
        )
    })
}

fn run_app(ctx: &mut Context, opts: &RunOpts) -> CommandResult {
    // Refuse before resolving anything when the flags alone settle it: without
    // `--no-logs`/`--detach` every path streams, whatever the target turns out
    // to be.
    if let Some(e) = streaming_under_machine_output(&ctx.out, !(opts.no_logs || opts.detach)) {
        return Err(e);
    }

    let mut plan = plan(ctx, opts)?;

    // This narrowed value drives the session dispatch below; clearing
    // `plan.hot` also drops the hot-only build flags and the summary's "hot
    // reload on" tag.
    let mut hot = session_hot(opts.hot, opts.hot_explicit, &plan.target);

    // The same rule applied to a busy injection port: one `--hot` session owns
    // `:8887`, and a committed default must not turn "another session is
    // already running" into a failed run. A typed `--hot` still fails loudly
    // inside `run_hot_session`.
    if hot && !opts.hot_explicit && !inject::server::port_available() {
        let who = inject::server::port_holder()
            .map_or_else(|| "another session".to_string(), |pid| format!("pid {pid}"));
        ctx.out.warn(&format!(
            "hot reload off for this run: {who} holds 127.0.0.1:8887. The \
             `[run] hot = true` default yields; type `--hot` to fail instead"
        ));
        hot = false;
    }
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
    if (opts.keep_sandbox || opts.hot_entitlements.is_some()) && !matches!(plan.target, Target::Mac)
    {
        return Err(CliError::new(
            "--keep-sandbox and --hot-entitlements apply to a hot macOS build (only a macOS \
             product carries the App Sandbox entitlement this strips)",
        ));
    }
    if hot && opts.detach {
        return Err(CliError::new(
            "--detach isn't supported with --hot; hot reload has to stay attached to \
             recompile and inject (press `d` in the session to detach and leave it running)",
        ));
    }

    // The rest is settled by the resolved plan: a hot session streams by
    // definition, and an SPM executable builds *and* runs in one `swift run`,
    // so its output is the program's own.
    if let Some(e) =
        streaming_under_machine_output(&ctx.out, hot || matches!(plan.target, Target::SpmRun(_)))
    {
        return Err(e);
    }

    print_summary(ctx, &plan);

    // Bring the Simulator window up so the running app is visible. Best-effort and
    // once per run — rebuilds reuse the same window, and only a simulator has a UI
    // to reveal (devices and macOS don't).
    if matches!(plan.target, Target::Simulator(_)) {
        let _ = simctl::open_app();
    }

    let result = if hot {
        // Hot reload owns its own build + launch + watch session (simulator or mac).
        run_hot_session(ctx, &plan, opts.hot_mode, opts.hot_selfcheck).map(|()| Rendered::Streamed)
    } else if matches!(plan.target, Target::SpmRun(_)) {
        // A Swift package executable builds, runs, and streams in one `swift run`;
        // there's no separate log stream to background, so it stays a one-shot.
        deploy(ctx, &plan).map(|_| Rendered::Streamed)
    } else if opts.detach {
        // --detach: deploy and return, leaving the app running.
        deploy_detached(ctx, &plan).map(Rendered::data)
    } else if opts.no_logs {
        // --no-logs: deploy and return, no session.
        deploy(ctx, &plan).map(Rendered::data)
    } else if ctx.out.is_interactive() {
        // The interactive rebuild session: output streams in the background and
        // `r` rebuilds+relaunches on demand.
        run_session(ctx, &plan).map(|()| Rendered::Streamed)
    } else {
        // Non-interactive (CI/piped): one-shot launch + inline follow until Ctrl-C.
        follow_once(ctx, &plan).map(|()| Rendered::Streamed)
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
#[allow(clippy::too_many_lines)] // one linear ladder per target mode
fn plan(ctx: &mut Context, opts: &RunOpts) -> Result<RunPlan, CliError> {
    let mut resolved = resolve::resolve(ctx)?;
    let schemes = resolve::schemes(&resolved.container)?;
    let scheme = resolve::settle_scheme(ctx, &mut resolved, &schemes, true)?;
    let configuration = resolve::settle_configuration(ctx, &mut resolved, true)?;

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
        // one human reference; it replaces the --mac/--device mode flags. The
        // Mac fast path skips the simctl spawn it never needs.
        let key = resolved.container.key();
        if resolve::on_is_mac(ctx, &key, &reference) {
            ("platform=macOS".to_string(), Target::Mac)
        } else {
            let sims = simctl::list()?;
            match resolve::resolve_on(ctx, &key, &reference, &sims)? {
                resolve::OnTarget::Mac => ("platform=macOS".to_string(), Target::Mac),
                resolve::OnTarget::Simulator { udid, specifier } => {
                    (specifier, Target::Simulator(udid))
                }
                resolve::OnTarget::Device { udid, specifier } => (specifier, Target::Device(udid)),
            }
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
        // destination here so the scheme picker doesn't run a second time. A
        // remembered simulator deleted under its pin (an Xcode update)
        // recovers to the picker instead of failing the install. Picks are
        // platform-filtered — a macOS-only scheme goes straight to the Mac.
        let key = resolved.container.key();
        let destination = match resolved.destination.clone() {
            Some(d) => resolve::refresh_stale_destination(
                ctx,
                &mut resolved,
                &key,
                &d,
                &scheme,
                &configuration,
                true,
            )?
            .unwrap_or(d),
            None => resolve::pick_destination_for(ctx, &resolved, &scheme, &configuration, true)?,
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

    warn_if_passthrough_moves_output(ctx, opts.passthrough);

    let mut plan = RunPlan {
        resolved,
        scheme,
        configuration,
        destination,
        target,
        hot: opts.hot,
        hot_entitlements: None,
        launch: opts.launch.clone(),
        passthrough: opts.passthrough.to_vec(),
    };
    // A product-relocating passthrough the app locator can't follow fails
    // here, before a build is spent on it.
    plan.passthrough_derived_data()?;
    // A hot macOS build may need to sign with an ephemeral sandbox-stripped
    // entitlements file (§9d zero-config sandbox stripping) — settled here so
    // every session build (including `r` rebuilds) carries the override.
    if plan.hot && matches!(plan.target, Target::Mac) {
        plan.hot_entitlements = hot_sandbox_override(ctx, &plan, opts)?;
    }
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

/// Settle the entitlements story for a hot macOS build (§9d zero-config
/// sandbox stripping): resolve the app target's effective
/// `CODE_SIGN_ENTITLEMENTS`, and when it asserts the App Sandbox, derive the
/// ephemeral stripped copy the build will sign with. `--keep-sandbox` /
/// `[run] auto_unsandbox = false` opt out; `--hot-entitlements FILE`
/// substitutes a caller-supplied plist (a missing FILE is a hard error — an
/// explicit flag must not degrade silently). Every *other* failure
/// (resolver, unreadable plist, strip) warns and falls back to no override,
/// leaving the built-product preflight to explain a still-sandboxed app.
fn hot_sandbox_override(
    ctx: &Context,
    plan: &RunPlan,
    opts: &RunOpts,
) -> Result<Option<std::path::PathBuf>, CliError> {
    use crate::cli::inject::sandbox::{self, SandboxPlan};

    let auto = ctx
        .project_file(&plan.resolved.container)
        .run
        .auto_unsandbox
        .unwrap_or(true);
    let keep = opts.keep_sandbox || !auto;

    // The effective entitlements file: resolved per app target and
    // configuration by the in-process engine, so `$(SRCROOT)` interpolation
    // and conditional `CODE_SIGN_ENTITLEMENTS[sdk=macosx*]` spellings are
    // already applied. A relative value is SRCROOT-relative (how the signer
    // reads it).
    let effective = || -> Result<Option<std::path::PathBuf>, CliError> {
        let settings = plan.resolved_settings()?;
        let target = xcodebuild::app_target(&settings, Some(&plan.destination))?;
        let Some(value) = target
            .settings
            .get("CODE_SIGN_ENTITLEMENTS")
            .filter(|v| !v.is_empty())
        else {
            return Ok(None);
        };
        let path = std::path::PathBuf::from(value);
        if path.is_absolute() {
            return Ok(Some(path));
        }
        let srcroot = target
            .settings
            .get("SRCROOT")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| CliError::new("resolved settings carry no SRCROOT"))?;
        Ok(Some(srcroot.join(path)))
    };
    // Only resolve when the answer can matter: a user-supplied file skips
    // resolution entirely, and an opted-out run never strips.
    let entitlements = if opts.hot_entitlements.is_some() || keep {
        None
    } else {
        match effective() {
            Ok(e) => e,
            Err(e) => {
                ctx.out.warn(&format!(
                    "hot reload: could not resolve the effective entitlements ({e}); \
                     building without the sandbox strip"
                ));
                return Ok(None);
            }
        }
    };

    match sandbox::plan(
        entitlements.as_deref(),
        opts.hot_entitlements,
        keep,
        &plan.resolved.container.path().display().to_string(),
        &plan.configuration,
    ) {
        Ok(SandboxPlan::Override(file)) => {
            if opts.hot_entitlements.is_some() {
                ctx.out.note(&format!(
                    "hot reload: signing with {} (--hot-entitlements)",
                    file.display()
                ));
            } else {
                ctx.out.note(&format!(
                    "hot reload: running un-sandboxed for injection ({} only, ephemeral) — \
                     app data lives in ~/Library, not the sandbox container",
                    plan.configuration
                ));
            }
            Ok(Some(file))
        }
        Ok(SandboxPlan::Unneeded | SandboxPlan::KeptSandbox) => Ok(None),
        // An explicit --hot-entitlements that can't be used is the user's to
        // fix, not something to quietly build past.
        Err(e) if opts.hot_entitlements.is_some() => Err(CliError::new(e)),
        Err(e) => {
            ctx.out.warn(&format!(
                "hot reload: {e}; building without the sandbox strip"
            ));
            Ok(None)
        }
    }
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

/// What a finite `app run` produced. `--no-logs` and `--detach` build, install,
/// launch and exit, so unlike the session forms they have a result worth
/// reporting: the launched bundle and — where the launcher tells us — its pid,
/// so a caller doesn't have to scrape them out of the human notes.
struct RunReport {
    bundle_id: String,
    /// The `-destination` specifier this ran on.
    destination: String,
    /// Present when the launcher reports one: simctl prints `<bundle>: <pid>`
    /// and a directly spawned macOS app is our own child. `devicectl` reports
    /// no pid we can rely on, so a device launch leaves this `null`.
    pid: Option<u32>,
    /// Whether the app was left running (`--detach`).
    detached: bool,
    /// The human lines, mirroring what each launcher reported, in the order
    /// they read. They travel with the report rather than being printed as
    /// they are produced, so a follow-up hint cannot land above the launch it
    /// refers to.
    notes: Vec<String>,
}

impl Render for RunReport {
    fn human(&self, out: &Output) {
        for note in &self.notes {
            out.note(note);
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "built": true,
            "bundleId": self.bundle_id,
            "destination": self.destination,
            "pid": self.pid,
            "detached": self.detached,
        })
    }
}

/// The pid `simctl launch` reports, from its `<bundle>: <pid>` line. `None`
/// when the output doesn't parse — a missing pid is worth reporting as `null`,
/// never worth failing a launch that already succeeded.
fn launched_pid(output: &str) -> Option<u32> {
    output.trim().rsplit(':').next()?.trim().parse().ok()
}

/// Build, install, and launch (no log following) — used by `--no-logs` and SPM.
/// `--detach`: build, launch, and return with the app still running.
///
/// Only macOS needs its own path — a simulator or device app already runs
/// outside this process, so [`deploy`] leaves it alive on its own.
fn deploy_detached(ctx: &Context, plan: &RunPlan) -> Result<RunReport, CliError> {
    if !matches!(plan.target, Target::Mac) {
        return deploy(ctx, plan).map(|r| RunReport {
            detached: true,
            ..r
        });
    }
    let app = build_and_install(plan, &ctx.out)?;
    let (pid, log) = spawn_detached_mac(ctx, plan, &app)?;
    Ok(RunReport {
        notes: detached_mac_notes(&app.bundle_id, pid, log.as_deref()),
        destination: plan.destination.clone(),
        bundle_id: app.bundle_id,
        pid: Some(pid),
        detached: true,
    })
}

/// What a detached macOS launch reports, in reading order: what was launched,
/// where its output went, and what ends it. Built as one list rather than
/// printed as each fact arrives, so the hint cannot precede the launch it
/// refers to.
fn detached_mac_notes(bundle_id: &str, pid: u32, log: Option<&Path>) -> Vec<String> {
    let mut notes = vec![format!("Launched {bundle_id} (pid {pid}) — detached")];
    notes.extend(log.map(|log| format!("output → {}", log.display())));
    notes.push(format!("'sweetpad app stop' terminates {bundle_id}"));
    notes
}

fn deploy(ctx: &Context, plan: &RunPlan) -> Result<RunReport, CliError> {
    // SPM executables build+run in one `swift run` step, not build+install+launch.
    if let Target::SpmRun(product) = &plan.target {
        spm_run(ctx, plan, product)?;
        // `swift run` already streamed the program's own output and the program
        // has since exited; there is no launch left to narrate.
        return Ok(RunReport {
            notes: Vec::new(),
            destination: plan.destination.clone(),
            bundle_id: product.clone(),
            pid: None,
            detached: false,
        });
    }

    let app = build_and_install(plan, &ctx.out)?;
    let (notes, pid) = match &plan.target {
        Target::Simulator(udid) => {
            let env = plan.launch.env_pairs("SIMCTL_CHILD_")?;
            let opts = plan.simctl_launch(&env);
            let out = ctx.out.step("Launching app", || {
                simctl::launch_opts(udid, &app.bundle_id, &opts)
            })?;
            (
                vec![format!("Launched {} → {}", app.bundle_id, out.trim())],
                launched_pid(&out),
            )
        }
        Target::Device(id) => {
            let out = ctx.out.step("Launching app on device", || {
                devicectl::launch(
                    id,
                    &app.bundle_id,
                    &plan.launch.args,
                    &plan.launch.env_pairs("DEVICECTL_CHILD_")?,
                    plan.launch.wait_for_debugger,
                )
            })?;
            (
                vec![format!(
                    "Launched {} on device → {}",
                    app.bundle_id,
                    out.trim()
                )],
                None,
            )
        }
        Target::Mac => {
            // One macOS launch mechanism for every non-session path: spawning
            // the executable directly is what lets `--env` and
            // `--wait-for-debugger` work at all (`open` forwards arguments but
            // neither of those), and it keeps `--no-logs`, `--detach` and
            // `app launch` behaving identically.
            let (pid, log) = spawn_detached_mac(ctx, plan, &app)?;
            let mut notes = vec![format!("Launched {} (pid {pid})", app.bundle_id)];
            notes.extend(log.map(|log| format!("output → {}", log.display())));
            (notes, Some(pid))
        }
        // Handled by the early return above.
        Target::SpmRun(_) => unreachable!("handled by the early return"),
    };
    Ok(RunReport {
        notes,
        destination: plan.destination.clone(),
        bundle_id: app.bundle_id,
        pid,
        detached: false,
    })
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
        // launched — exit as a user cancel (6), not success. The background
        // `simctl boot` is joined first, or its child outlives the CLI and
        // boots the simulator the user just cancelled.
        BuildOutcome::Aborted => {
            let _ = boot.wait();
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

/// `app run --hot` — the built-in hot-reload session (iOS Simulator and native
/// macOS apps).
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
    // The injectable targets: a simulator (simctl forwards the insert env into
    // the simulated process) and a native mac app (spawned by us with the env
    // set directly). Devices strip DYLD_INSERT_LIBRARIES; a Swift package
    // executable has no .app bundle to inject into.
    match &plan.target {
        Target::Simulator(_) | Target::Mac => {}
        Target::Device(_) => {
            return Err(CliError::new(
                "--hot is not supported on physical devices (iOS strips DYLD_INSERT_LIBRARIES)",
            ));
        }
        Target::SpmRun(_) => {
            return Err(CliError::new(
                "--hot needs an .app bundle; a Swift package executable has none",
            ));
        }
    }
    let sdk = inject::sdk_for_destination(&plan.destination).ok_or_else(|| {
        CliError::new(format!(
            "--hot needs a simulator or macOS destination; got {:?}",
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
        // a user cancel (exit 6), not a success. Join the background boot
        // first so its `simctl boot` child doesn't outlive the cancel.
        BuildOutcome::Aborted => {
            let _ = boot.wait();
            return Err(CliError::new("cancelled").kind(ErrorKind::UserCancel));
        }
    }
    let app = plan.app_bundle()?;

    // A mac product must actually be injectable — an explicit entitlements
    // file or a re-signing build phase can undo the hot build's settings; fail
    // here with the cause instead of launching a dead session.
    if matches!(plan.target, Target::Mac) {
        inject::mac_preflight(&app.path).map_err(|e| CliError::new(format!("hot reload: {e}")))?;
    }

    // Resolve the injection client dylib + the launch env: SIMCTL_CHILD_-
    // prefixed for a simctl launch (stripped as it's forwarded into the
    // simulated process), raw for the direct mac spawn.
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
    let env_prefix = match &plan.target {
        Target::Simulator(_) => "SIMCTL_CHILD_",
        _ => "",
    };
    let mut launch_env = inject::client::launch_env(&dylib, &client_opts, env_prefix);
    // User --env pairs ride alongside the injection client's.
    launch_env.extend(plan.launch.env_pairs(env_prefix)?);
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
    // Finish the background boot first; the sim launch's own boot then confirms it.
    let _ = boot.wait();
    // Hot reload has no live filter UI; use the default threshold, never cycled.
    let filter = Arc::new(AtomicU8::new(default_filter(&ctx.out).threshold()));
    let mut hot_app = HotApp::new(&plan.target, Arc::clone(&filter));
    hot_app.launch(ctx, &app, &launch_env, &plan.launch.args)?;

    // A mac app that never dials back is running uninjected (something undid
    // the insert env); surface that instead of leaving a silently dead session.
    // The self-check does its own (fatal) connect wait, and the flag keeps a
    // watchdog that outlives a short session quiet after teardown.
    let session_done = Arc::new(AtomicBool::new(false));
    if matches!(plan.target, Target::Mac) && selfcheck.is_none() {
        let server = Arc::clone(&server);
        let log = Arc::clone(&log);
        let done = Arc::clone(&session_done);
        std::thread::spawn(move || {
            if !server.wait_connected(std::time::Duration::from_secs(15))
                && !done.load(Ordering::Relaxed)
            {
                log(
                    "hot reload: the app hasn't connected to :8887 — it's likely running \
                     uninjected. A run-script phase may be re-signing the product; \
                     `codesign -d -vv --entitlements - <app>` shows what it carries.",
                );
            }
        });
    }

    let mut logs = start_logs(ctx, plan, &filter);
    // Watch the workspace; each save drives `server.inject`.
    let session = HotSession::start(Arc::clone(&server), &project_root);

    // CI self-check: edit a file once, assert `.injected`, exit. Otherwise the
    // interactive key loop (`r`/`q`), or — non-TTY — follow logs until Ctrl-C.
    let mut terminate_on_exit = true;
    let outcome = if let Some(file) = selfcheck {
        hot_selfcheck(ctx, &server, file, &plan.target)
    } else if ctx.out.is_interactive() {
        terminate_on_exit = hot_key_loop(
            ctx,
            plan,
            &mut hot_app,
            &app,
            &launch_env,
            &mut logs,
            &build_log,
        );
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
    session_done.store(true, Ordering::Relaxed);
    session.shutdown();
    server.shutdown();
    if terminate_on_exit {
        hot_app.terminate(&app);
    } else {
        hot_app.detach();
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
fn hot_selfcheck(
    ctx: &Context,
    server: &Arc<InjectServer>,
    file: &Path,
    target: &Target,
) -> CliResult {
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
        // The backup is the only pristine copy — it must outlive a failed
        // restore (EACCES, full disk), or the fixture stays nonce-corrupted
        // with nothing left to heal from.
        if std::fs::copy(&backup, file).is_ok() {
            let _ = std::fs::remove_file(&backup);
            ctx.out
                .note("hot reload self-check: restored the fixture from a previous run's backup");
        } else {
            ctx.out.warn(&format!(
                "hot reload self-check: could not restore {} from {} — keeping the backup",
                file.display(),
                backup.display()
            ));
        }
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
    if app_logged_marker(target, &nonce, Duration::from_secs(20)) {
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

/// Poll the target's unified log for `nonce` (emitted by the fixture's
/// injection observer via `os_log`) — the simulator's via `simctl spawn`, a
/// mac app's via the host `log` — returning true once it appears or false
/// after `timeout`.
fn app_logged_marker(target: &Target, nonce: &str, timeout: std::time::Duration) -> bool {
    use std::time::{Duration, Instant};
    let predicate = format!("eventMessage CONTAINS \"{nonce}\"");
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let shown = match target {
            Target::Simulator(udid) => process::capture(
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
            ),
            Target::Mac => process::capture(
                "log",
                &[
                    "show",
                    "--last",
                    "1m",
                    "--style",
                    "compact",
                    "--predicate",
                    &predicate,
                ],
                None,
            ),
            Target::Device(_) | Target::SpmRun(_) => return false,
        };
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

/// The launched hot-session app, with the target-specific launch / terminate /
/// relaunch strategy: the simulator app is simctl's (installed and terminated
/// by bundle id), the mac app is our own spawned child (killed directly, its
/// piped stdout/stderr rendered as console output).
enum HotApp<'a> {
    Sim {
        udid: &'a str,
    },
    Mac {
        child: Option<Child>,
        reap_slot: Option<usize>,
        filter: Arc<AtomicU8>,
    },
}

impl HotApp<'_> {
    fn new(target: &Target, filter: Arc<AtomicU8>) -> HotApp<'_> {
        match target {
            Target::Simulator(udid) => HotApp::Sim { udid },
            Target::Mac => HotApp::Mac {
                child: None,
                reap_slot: None,
                filter,
            },
            Target::Device(_) | Target::SpmRun(_) => {
                unreachable!("hot sessions run on a simulator or the mac")
            }
        }
    }

    /// Launch (or relaunch) the app with the injection env. The mac arm kills
    /// any previous instance first — one app, one window across `r` relaunches.
    fn launch(
        &mut self,
        ctx: &Context,
        app: &AppBundle,
        env: &[(String, String)],
        args: &[String],
    ) -> CliResult {
        match self {
            HotApp::Sim { udid } => launch_hot(ctx, udid, app, env, args),
            HotApp::Mac {
                child,
                reap_slot,
                filter,
            } => {
                terminate_mac_child(child, reap_slot);
                let mut cmd = std::process::Command::new(app.executable.as_os_str());
                cmd.args(args)
                    .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                let mut c = ctx.out.step("Launching app", || {
                    cmd.spawn().map_err(|e| {
                        CliError::new(format!("failed to run `{}`: {e}", app.executable.display()))
                    })
                })?;
                render_console(&mut c, ctx.out.use_color(), filter);
                *reap_slot = crate::cli::signals::register_child(c.id());
                *child = Some(c);
                ctx.out.note(&format!("Launched {}", app.bundle_id));
                Ok(())
            }
        }
    }

    /// Terminate the running app (before each relaunch and on quit).
    fn terminate(&mut self, app: &AppBundle) {
        match self {
            HotApp::Sim { udid } => {
                let _ = simctl::terminate(udid, &app.bundle_id);
            }
            HotApp::Mac {
                child, reap_slot, ..
            } => terminate_mac_child(child, reap_slot),
        }
    }

    /// Leave the app running at session end (the `d` detach). The mac child's
    /// handle is dropped without killing; it leaves the signal registry so a
    /// SIGTERM to the CLI no longer reaps it.
    fn detach(&mut self) {
        if let HotApp::Mac {
            child, reap_slot, ..
        } = self
        {
            crate::cli::signals::unregister_child(reap_slot.take());
            drop(child.take());
        }
    }

    /// The `d` confirmation. The mac app's console runs through our pipes, so
    /// its next print after the CLI exits raises SIGPIPE and may stop it —
    /// same caveat as the plain session's detach.
    fn detach_note(&self) -> &'static str {
        match self {
            HotApp::Sim { .. } => "detached — the app keeps running",
            HotApp::Mac { .. } => {
                "detached — the app keeps running (its console pipes close with the CLI; \
                 a later print may stop it)"
            }
        }
    }

    /// Bring the app's UI forward (the `o` key): the Simulator window, or the
    /// mac app itself (`open` on the bundle activates the running instance).
    fn foreground(&self, app: &AppBundle) {
        match self {
            HotApp::Sim { .. } => {
                let _ = simctl::open_app();
            }
            HotApp::Mac { .. } => {
                let _ = process::run("open", &[&app.path.display().to_string()], None, true);
            }
        }
    }

    /// The `h` key list, with the target's own foreground wording.
    fn help_note(&self) -> &'static str {
        match self {
            HotApp::Sim { .. } => {
                "r rebuild+relaunch · s screenshot · o focus simulator · c clear · \
                 d detach · q quit"
            }
            HotApp::Mac { .. } => {
                "r rebuild+relaunch · s screenshot · o focus app · c clear · d detach · q quit"
            }
        }
    }
}

/// Kill and reap a mac hot-session child, deregistering it first so the signal
/// handler never signals a recycled pid.
fn terminate_mac_child(child: &mut Option<Child>, reap_slot: &mut Option<usize>) {
    crate::cli::signals::unregister_child(reap_slot.take());
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// The `--hot` keypress loop: `r` full rebuild+relaunch (the client
/// reconnects), `s`/`o`/`c`/`h` as in the plain session, `d` detaches (the app
/// keeps running), `q`/Ctrl-C/Ctrl-D quit. Injection happens out-of-band via
/// the watcher. Returns whether the app should be terminated on teardown
/// (false after a detach).
fn hot_key_loop(
    ctx: &Context,
    plan: &RunPlan,
    hot_app: &mut HotApp,
    app: &AppBundle,
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
                    hot_app.terminate(&app);
                    match build(plan, &ctx.out, Some(build_log)) {
                        BuildOutcome::Ok => {
                            if let Err(e) = hot_app.launch(ctx, &app, env, &plan.launch.args) {
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
                    ctx.out.note(hot_app.detach_note());
                    return false;
                }
                SessionKey::Screenshot => session_screenshot(ctx, plan),
                SessionKey::Foreground => hot_app.foreground(app),
                SessionKey::Clear => ctx.out.line("\x1b[2J\x1b[H"),
                SessionKey::Help => ctx.out.note(hot_app.help_note()),
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
            let mut child = devicectl::spawn_console(
                id,
                &app.bundle_id,
                &plan.launch.args,
                &plan.launch.env_pairs("DEVICECTL_CHILD_")?,
            )?;
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
            if plan.launch.wait_for_debugger {
                stop_for_debugger(ctx, child.id());
            }
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
    // process group. The watcher stops before the reap for the same reason —
    // it signals the group by raw pid, and a keystroke landing after `wait()`
    // freed the pgid would SIGINT a recycled group (costs one poll tick).
    crate::cli::signals::clear_build_pgid();
    done.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    let status = child.wait();

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
            stream_logs(
                ctx,
                &LogSource::Simulator(udid),
                &app,
                &LogFilterArgs::default(),
            )
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
            devicectl::launch_console(
                id,
                &app.bundle_id,
                &plan.launch.args,
                &plan.launch.env_pairs("DEVICECTL_CHILD_")?,
            )
        }
        Target::Mac => {
            ctx.out
                .note(&format!("Running {} (Ctrl-C to stop)", app.bundle_id));
            // Stream the app's os_log alongside its inherited stdout/stderr; the
            // non-interactive path has no live filter, so use the default threshold.
            let filter = Arc::new(AtomicU8::new(default_filter(&ctx.out).threshold()));
            let _logs = start_logs(ctx, plan, &filter);
            // This path runs the app in the foreground and waits for it, so a
            // stopped-before-main process would just hang with no pid to
            // attach to. Refuse rather than accept and ignore.
            if plan.launch.wait_for_debugger {
                return Err(CliError::new(
                    "--wait-for-debugger needs a launch that returns: use `app launch --mac \
                     --wait-for-debugger` (it reports the stopped pid), or run at an \
                     interactive terminal",
                ));
            }
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

/// The `s` key: screenshot a simulator or macOS target into ./sweetpad-shots/.
fn session_screenshot(ctx: &Context, plan: &RunPlan) {
    let result = match &plan.target {
        Target::Simulator(udid) => {
            let name = sim_name(udid).unwrap_or_else(|| "simulator".to_string());
            let path = super::simulator::default_screenshot_path(&name);
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                let _ = std::fs::create_dir_all(parent);
            }
            simctl::screenshot(udid, &path.display().to_string()).map(|()| path)
        }
        Target::Mac => plan.app_bundle().and_then(|app| {
            let shot = mac_shot_for(&app.executable, &app.bundle_id)?;
            let path = super::simulator::default_screenshot_path(&shot.name);
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                let _ = std::fs::create_dir_all(parent);
            }
            let (window, _) = wait_for_window(ctx, &shot, None)?;
            macwin::capture_window(window.number, &path).map(|()| path)
        }),
        Target::Device(_) | Target::SpmRun(_) => {
            ctx.out.note("screenshots need a simulator or macOS target");
            return;
        }
    };
    match result {
        Ok(path) => ctx.out.note(&format!("📸 {}", path.display())),
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
    let Some(child) = running.stream.as_mut() else {
        return;
    };
    // Deregister around the probe: `try_wait` returning `Some` *is* the reap,
    // after which the pid can be recycled and the handler must never signal
    // it — a session stays open for hours after an app crash, plenty of time
    // for the pid space to wrap. Not-exited re-registers (a microsecond
    // window, same class as the accepted spawn→register gap).
    crate::cli::signals::unregister_child(running.reap_slot.take());
    if matches!(child.try_wait(), Ok(Some(_))) {
        ctx.out.alert(&format!("✗ {} exited", running.name));
        running.reported_exit = true;
    } else {
        running.reap_slot = crate::cli::signals::register_child(child.id());
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
    let predicate = log_predicate(app, filters, marker);
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

/// The NSPredicate shared by [`log_command`] (stream) and [`log_show_command`]
/// (history): match the app's process image — and the Xcode 15+ `.debug.dylib`
/// sender that carries app code in Debug builds — so logs show even without a
/// `Logger(subsystem:)`, while Apple framework chatter stays out. A raw
/// `--predicate` replaces this wholesale; `--subsystem`/`--category` narrow it.
/// See [`log_command`] for what a `marker` clause is for.
fn log_predicate(app: &AppBundle, filters: &LogFilterArgs, marker: Option<&str>) -> String {
    use std::fmt::Write as _;
    let exe = predicate_escape(process_name(app));
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
    predicate
}

/// Build the one-shot `log show --style ndjson --last <dur>` command for the
/// backfill (`app logs --last`) — the host `log` for a macOS app, or `xcrun
/// simctl spawn <udid> log` for a simulator. Unlike `log stream`, `log show`
/// selects verbosity with the `--info`/`--debug` flags rather than `--level`, so
/// `level` maps to those; and it needs no reaping marker, since it exits on its
/// own instead of streaming until killed.
fn log_show_command(
    source: &LogSource,
    app: &AppBundle,
    filters: &LogFilterArgs,
    last: &str,
    level: &str,
) -> (&'static str, Vec<String>) {
    let predicate = log_predicate(app, filters, None);
    let mut show = vec![
        "show".to_string(),
        "--style".to_string(),
        "ndjson".to_string(),
        "--last".to_string(),
        last.to_string(),
        "--predicate".to_string(),
        predicate,
    ];
    // `log show` omits Info/Debug entries unless asked; `--debug` implies both.
    match level {
        "debug" => {
            show.push("--info".to_string());
            show.push("--debug".to_string());
        }
        "info" => show.push("--info".to_string()),
        _ => {}
    }
    match source {
        LogSource::Mac => ("log", show),
        LogSource::Simulator(udid) => {
            let mut args = vec![
                "simctl".to_string(),
                "spawn".to_string(),
                (*udid).to_string(),
                "log".to_string(),
            ];
            args.append(&mut show);
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
        // Lossy line reads: one invalid-UTF-8 byte must not end the thread —
        // dropping the pipe's read end SIGPIPEs the still-writing child.
        process::read_lines_lossy(stdout, &mut |line| {
            let rendered = oslog::render_ndjson_line(line, color);
            if rendered.level.as_u8() >= filter.load(Ordering::Relaxed) {
                println!("{}", rendered.text);
            }
        });
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
            // Lossy line reads: a binary dump on the app's own stdout must
            // not end this thread — on the Mac target the streamed child *is*
            // the app, and a dropped read end SIGPIPE-kills it mid-session.
            process::read_lines_lossy(pipe, &mut |line| {
                if is_boot_noise(line) {
                    return;
                }
                // Console output has no timestamp of its own; stamp it with the local
                // time the line arrived, so it lines up with the os_log stream.
                let now = oslog::now_clock();
                let rendered = oslog::render_console_line(Some(&now), line, color);
                if rendered.level.as_u8() >= filter.load(Ordering::Relaxed) {
                    println!("{}", rendered.text);
                }
            });
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
        process::read_lines_lossy(stderr, &mut |line| {
            if line.trim().is_empty() || is_boot_noise(line) {
                return;
            }
            let rendered = oslog::render_fields(None, "Error", "system", line, color);
            if rendered.level.as_u8() >= filter.load(Ordering::Relaxed) {
                println!("{}", rendered.text);
            }
        });
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
        process::read_lines_lossy(stdout, &mut |line| {
            let Some(entry) = pymobiledevice3::parse_line(line) else {
                if continuing {
                    println!("{line}");
                }
                return;
            };
            if entry.image != exe && entry.image != debug_dylib {
                continuing = false;
                return;
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
        });
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
        mac: stage_target.mac,
        no_logs: true,
        detach: false,
        hot: false,
        hot_explicit: false,
        hot_mode: Mode::Resolver,
        hot_selfcheck: None,
        keep_sandbox: false,
        hot_entitlements: None,
        launch,
        passthrough: &[],
    };
    let plan = plan(ctx, &opts)?;
    let app = plan.app_bundle()?;

    let report = match &plan.target {
        Target::Simulator(udid) => simple_on_simulator(ctx, stage, &plan, &app, udid)?,
        Target::Device(id) => simple_on_device(ctx, stage, &plan, &app, id)?,
        // A macOS app needs no install step — it runs in place out of
        // DerivedData — but `launch` and `stop` are real operations on it.
        Target::Mac if matches!(stage, Stage::Stop) => {
            stop_mac(ctx, &app.executable, &app.bundle_id)?
        }
        Target::Mac if matches!(stage, Stage::Launch) => launch_mac(ctx, &plan, &app)?,
        Target::Mac | Target::SpmRun(_) => {
            return Err(CliError::new(
                "app install/uninstall act on a simulator or device — a macOS app is built \
                 in place; use `app launch --mac` to start it or `app run --mac` to follow it",
            ));
        }
    };
    if matches!(stage, Stage::Launch) {
        record_last_launched(ctx, &plan);
    }
    Ok(Rendered::data(report))
}

/// Where a detached macOS app's console output goes. Its stdio cannot be a
/// pipe to us — we exit immediately and the app would die on its next `print`
/// (see [`HotApp::detach_note`]) — so it is redirected to a file the user can
/// tail afterwards (`app logs --mac`, or [`follow_console_file`]).
pub(crate) fn detached_log_path(bundle_id: &str) -> Option<std::path::PathBuf> {
    let dir = sweetpad_core::paths::sweetpad_state_dir()?.join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(format!("{bundle_id}.log")))
}

/// Open (truncating) the file a detached macOS app's stdout/stderr are captured
/// to, stamped with a one-line run header. The file is reused per bundle id, so
/// truncating is what keeps a later `app logs` read from replaying `print`
/// output left by a previous run — the exact staleness that made the captured
/// file untrustworthy. The two returned handles share one open file
/// description, so stdout and stderr append in order after the header. `None`
/// (no writable state dir) leaves the caller to discard the app's stdio.
fn open_detached_log(
    path: &Path,
    app: &AppBundle,
    plan: &RunPlan,
) -> Option<(std::fs::File, std::fs::File)> {
    use std::io::Write as _;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .ok()?;
    let args = plan.launch.args.join(" ");
    let suffix = if args.is_empty() {
        String::new()
    } else {
        format!(" — args: {args}")
    };
    let _ = writeln!(
        &file,
        "=== sweetpad launched {} at {}{suffix} ===",
        process_name(app),
        oslog::now_clock(),
    );
    let err = file.try_clone().ok()?;
    Some((file, err))
}

/// Start a macOS app and return, leaving it running — the counterpart to
/// [`stop_mac`]. Unlike `app run --mac`, the process does not belong to this
/// CLI: it gets its own session (so a Ctrl-C in this terminal can't reach it)
/// and file-backed stdio (so it survives our exit).
fn launch_mac(
    ctx: &mut Context,
    plan: &RunPlan,
    app: &AppBundle,
) -> Result<AppStageReport, CliError> {
    let (pid, log) = spawn_detached_mac(ctx, plan, app)?;
    Ok(AppStageReport {
        action: "launched",
        note: format!("Launched {}", app.bundle_id),
        bundle_id: app.bundle_id.clone(),
        udid: None,
        pid: pid.try_into().ok(),
        detail: log.map(|p| format!("output → {}", p.display())),
    })
}

/// Give a directly-spawned macOS app its own session, detaching it from this
/// terminal's job control. Wanted when the app outlives us; not wanted for a
/// session child that Ctrl-C should still reach.
fn own_session_on_spawn(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // Safety: `setsid` is async-signal-safe and touches no shared state; this
    // closure runs in the forked child before `exec`.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

/// Stop a just-spawned app so a debugger can attach, and report its pid.
///
/// Deliberately signalled from the parent *after* `spawn`, not from
/// `pre_exec`: that hook runs before `exec`, and stopping there deadlocks —
/// `spawn` blocks reading the child's exec-status pipe, which never closes.
/// The cost is that the stop lands just after `exec` (during dyld load)
/// rather than being a hard pre-`main` guarantee.
fn stop_for_debugger(ctx: &Context, pid: u32) {
    if let Ok(pid) = i32::try_from(pid) {
        // Safety: signalling a child we just spawned and have not reaped.
        unsafe {
            libc::kill(pid, libc::SIGSTOP);
        }
    }
    ctx.out.note(&format!(
        "stopped for the debugger — attach to pid {pid}, then `kill -CONT {pid}` to continue"
    ));
}

/// Spawn a macOS app so it outlives this process, returning its pid and the
/// file its console output goes to. Shared by `app launch --mac` and
/// `app run --mac --detach`.
///
/// Spawning the executable directly (rather than via `open`) is what lets
/// `--env` reach the process; `open` forwards arguments but not environment.
fn spawn_detached_mac(
    ctx: &Context,
    plan: &RunPlan,
    app: &AppBundle,
) -> Result<(u32, Option<std::path::PathBuf>), CliError> {
    // Replace any instance already running, matching `simctl launch
    // --terminate-running-process` and `devicectl … --terminate-existing`.
    // Without this the same verb means "replace" on two targets and "start a
    // duplicate" on the third — and the duplicate would silently ignore the
    // `--arg`/`--env` the caller just passed.
    if let Ok(pids) = macwin::pids_for_executable(&app.executable)
        && !pids.is_empty()
    {
        for pid in &pids {
            // Safety: SIGTERM to a pid we resolved from the app's own
            // executable path; the app gets its normal termination path.
            unsafe {
                libc::kill(*pid, libc::SIGTERM);
            }
        }
    }
    let env = plan.launch.env_pairs("")?;
    let log = detached_log_path(&app.bundle_id);
    let mut cmd = std::process::Command::new(app.executable.as_os_str());
    cmd.args(&plan.launch.args)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(std::process::Stdio::null());
    match log.as_ref().and_then(|p| open_detached_log(p, app, plan)) {
        Some((out, err)) => {
            cmd.stdout(out).stderr(err);
        }
        // No writable state dir: discard rather than inherit our stdio, which
        // would tie the app's lifetime to this terminal.
        None => {
            cmd.stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
        }
    }
    // Own session: the app must not receive this terminal's job-control
    // signals once we're gone. `--wait-for-debugger` additionally stops the
    // process before it runs `main`, so a debugger can attach to the pid we
    // report — the macOS equivalent of simctl's `--wait-for-debugger`, which
    // this path used to accept and silently drop.
    own_session_on_spawn(&mut cmd);
    let child = ctx.out.step("Launching app", || {
        cmd.spawn().map_err(|e| {
            CliError::new(format!("failed to run `{}`: {e}", app.executable.display()))
        })
    })?;
    // Deliberately not registered with the signal registry and never waited
    // on: it outlives this process. Dropping `Child` on Unix does not kill it.
    if plan.launch.wait_for_debugger {
        stop_for_debugger(ctx, child.id());
    }
    Ok((child.id(), log))
}

/// Terminate a running macOS app by its executable path: SIGTERM to every
/// matching pid (the graceful quit — the app can save state and exit).
fn stop_mac(ctx: &Context, executable: &Path, bundle_id: &str) -> Result<AppStageReport, CliError> {
    let pids = macwin::pids_for_executable(executable)?;
    if pids.is_empty() {
        return Err(CliError::new(format!("{bundle_id} isn't running")));
    }
    ctx.out.step("Terminating app", || {
        for pid in &pids {
            unsafe {
                libc::kill(*pid, libc::SIGTERM);
            }
        }
    });
    Ok(AppStageReport {
        action: "terminated",
        note: format!("Terminated {bundle_id}"),
        bundle_id: bundle_id.to_string(),
        udid: None,
        pid: pids.first().copied(),
        detail: None,
    })
}

/// The `RunOpts` shared by `app debug`/`app diagnose`: build + install and
/// hand off to lldb, never a hot session, no log streaming.
fn lldb_run_opts<'a>(stage_target: &'a StageTargetArgs, launch: &'a LaunchArgs) -> RunOpts<'a> {
    RunOpts {
        device: stage_target.device || stage_target.device_id.is_some(),
        device_id: stage_target.device_id.as_deref(),
        mac: stage_target.mac,
        no_logs: true,
        detach: false,
        hot: false,
        hot_explicit: false,
        hot_mode: Mode::Resolver,
        hot_selfcheck: None,
        keep_sandbox: false,
        hot_entitlements: None,
        launch,
        passthrough: &[],
    }
}

/// Build, install, and launch the app **suspended** on `udid`, returning the
/// bundle and the stopped pid for lldb to attach to. Shared by interactive
/// `app debug`, `app debug --batch`, and `app diagnose` on a simulator.
fn launch_suspended_on_sim(
    ctx: &Context,
    plan: &RunPlan,
    udid: &str,
) -> Result<(AppBundle, u32), CliError> {
    let app = build_and_install(plan, &ctx.out)?;
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
    Ok((app, pid))
}

/// `app debug`: attach lldb to the built app. Interactive by default; with
/// `--batch` it drives lldb from `--cmd` commands and streams the output
/// (for scripts and agents). On a simulator the app is launched suspended and
/// lldb attaches; on macOS lldb owns the launch outright.
fn debug(
    ctx: &mut Context,
    stage_target: &StageTargetArgs,
    launch: &LaunchArgs,
    batch: &DebugBatchArgs,
) -> CommandResult {
    // `--batch` streams lldb's output live; like `app run` there's no coherent
    // one-shot JSON for it. Point at `app diagnose` for a structured report.
    if batch.batch && (ctx.out.is_json() || ctx.out.is_ndjson()) {
        return Err(CliError::new(
            "`app debug --batch` streams lldb output and has no machine-readable form; use \
             `app diagnose -o json` for a structured exception/crash report",
        ));
    }
    let opts = lldb_run_opts(stage_target, launch);
    let plan = plan(ctx, &opts)?;
    match &plan.target {
        // A macOS app runs on this machine, so lldb can own the launch
        // outright — no suspended-launch-then-attach dance, and breakpoints
        // can be set before the process exists.
        Target::Mac if batch.batch => debug_batch_mac(ctx, &plan, batch),
        Target::Mac => debug_mac(ctx, &plan),
        Target::Simulator(udid) => {
            let udid = udid.clone();
            if batch.batch {
                debug_batch_sim(ctx, &plan, &udid, batch)
            } else {
                debug_sim_interactive(ctx, &plan, &udid)
            }
        }
        Target::Device(_) => Err(CliError::new(
            "app debug can't drive a physical device yet; debug on a simulator, or \
             attach Xcode to the device",
        )),
        Target::SpmRun(_) => Err(CliError::new(
            "app debug works on an app target; for a Swift package run `lldb -- swift run`",
        )),
    }
}

/// Interactive `app debug` on a simulator: launch suspended, then attach
/// `lldb -p <pid>` and hand over the terminal — type `continue` to resume.
fn debug_sim_interactive(ctx: &mut Context, plan: &RunPlan, udid: &str) -> CommandResult {
    let (app, pid) = launch_suspended_on_sim(ctx, plan, udid)?;
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

/// `app debug --batch` on a simulator: launch suspended, then run lldb
/// non-interactively against the attached pid from the user's `--cmd` chain.
fn debug_batch_sim(
    ctx: &mut Context,
    plan: &RunPlan,
    udid: &str,
    batch: &DebugBatchArgs,
) -> CommandResult {
    let (app, pid) = launch_suspended_on_sim(ctx, plan, udid)?;
    let args = batch_lldb_args(&LldbTarget::AttachPid(pid), &batch.cmd, &batch.on_crash);
    ctx.out.note(&format!(
        "running lldb --batch against {} (pid {pid})",
        app.bundle_id
    ));
    let pid_i32 = i32::try_from(pid).unwrap_or(0);
    run_lldb_streamed(&args, &[], batch_timeout(batch.timeout), || vec![pid_i32])
        .map(|()| Rendered::Streamed)
}

/// `app debug --batch` on macOS: lldb owns the launch, driven from `--cmd`.
fn debug_batch_mac(ctx: &mut Context, plan: &RunPlan, batch: &DebugBatchArgs) -> CommandResult {
    let app = build_and_install(plan, &ctx.out)?;
    let env = plan.launch.env_pairs("")?;
    let exe = app.executable.display().to_string();
    let args = batch_lldb_args(
        &LldbTarget::Mac {
            exe: &exe,
            args: &plan.launch.args,
        },
        &batch.cmd,
        &batch.on_crash,
    );
    ctx.out
        .note(&format!("running lldb --batch on {}", app.bundle_id));
    let executable = app.executable.clone();
    run_lldb_streamed(&args, &env, batch_timeout(batch.timeout), || {
        macwin::pids_for_executable(&executable).unwrap_or_default()
    })
    .map(|()| Rendered::Streamed)
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
                devicectl::launch(
                    id,
                    &app.bundle_id,
                    &plan.launch.args,
                    &plan.launch.env_pairs("DEVICECTL_CHILD_")?,
                    plan.launch.wait_for_debugger,
                )
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
        udid: Some(udid.to_string()),
        pid: None,
        detail,
    }
}

/// Serve `app stop` from the recorded last launch when it targeted a
/// simulator or macOS — no scheme resolution, no build-settings query, no
/// prompting. `None` (fall back to the full plan) when nothing was recorded
/// or the record is for a device run.
fn simple_from_last_launched(ctx: &mut Context, stage: Stage) -> Option<CommandResult> {
    match stage {
        Stage::Stop => {}
        Stage::Install | Stage::Launch | Stage::Uninstall => {
            unreachable!("gated to Stop by the caller")
        }
    }
    let last = last_launched(ctx)?;
    match last.kind.as_str() {
        "simulator" => {
            let udid = last.simulator_udid.clone()?;
            Some(
                ctx.out
                    .step("Terminating app", || {
                        simctl::terminate(&udid, &last.bundle_identifier)
                    })
                    .map(|()| {
                        Rendered::data(AppStageReport {
                            action: "terminated",
                            note: format!("Terminated {}", last.bundle_identifier),
                            bundle_id: last.bundle_identifier.clone(),
                            udid: Some(udid.clone()),
                            pid: None,
                            detail: None,
                        })
                    }),
            )
        }
        "macos" => {
            let exe = mac_executable(&last)?;
            Some(stop_mac(ctx, &exe, &last.bundle_identifier).map(Rendered::data))
        }
        "device" => {
            // The record already holds everything `devicectl` needs, so a bare
            // `app stop` shouldn't demand an explicit --device.
            let id = last.destination_id.clone()?;
            let app_dir = last
                .app_name
                .clone()
                .or_else(|| last.executable_name.clone())?;
            Some(
                ctx.out
                    .step("Terminating app", || devicectl::terminate(&id, &app_dir))
                    .map(|()| {
                        Rendered::data(AppStageReport {
                            action: "terminated",
                            note: format!("Terminated {}", last.bundle_identifier),
                            bundle_id: last.bundle_identifier.clone(),
                            udid: None,
                            pid: None,
                            detail: None,
                        })
                    }),
            )
        }
        _ => None,
    }
}

/// The recorded last launch for this project, whatever it targeted.
fn last_launched(ctx: &Context) -> Option<LastLaunchedApp> {
    let container = resolve::container(ctx).ok()?;
    ctx.state
        .projects
        .get(&container.key())?
        .last_launched_app
        .clone()
}

/// The executable path inside a recorded macOS launch's `.app` bundle.
fn mac_executable(last: &LastLaunchedApp) -> Option<std::path::PathBuf> {
    let name = last.executable_name.as_deref()?;
    Some(
        std::path::PathBuf::from(&last.app_path)
            .join("Contents/MacOS")
            .join(name),
    )
}

/// `app logs` — its own entry point so the stream filters reach
/// [`stream_logs`]. Uses the recorded last launch when available, else the
/// resolved build target.
fn simple_logs(
    ctx: &mut Context,
    stage_target: &StageTargetArgs,
    filters: &LogFilterArgs,
) -> CommandResult {
    // An explicit --mac/--device names the target, so the simulator fast path
    // must yield to it just as explicit targeting does.
    if !explicit_targeting(ctx)
        && !stage_target.mac
        && !stage_target.device
        && stage_target.device_id.is_none()
        && let Some((udid, app)) = last_launched_sim(ctx)
    {
        ctx.out.step("Booting simulator", || simctl::boot(&udid))?;
        stream_logs(ctx, &LogSource::Simulator(&udid), &app, filters)?;
        return Ok(Rendered::Streamed);
    }
    let opts = RunOpts {
        device: stage_target.device || stage_target.device_id.is_some(),
        device_id: stage_target.device_id.as_deref(),
        mac: stage_target.mac,
        no_logs: true,
        detach: false,
        hot: false,
        hot_explicit: false,
        hot_mode: Mode::Resolver,
        hot_selfcheck: None,
        keep_sandbox: false,
        hot_entitlements: None,
        launch: &LaunchArgs::default(),
        passthrough: &[],
    };
    let plan = plan(ctx, &opts)?;
    let app = plan.app_bundle()?;
    match &plan.target {
        Target::Simulator(udid) => {
            // Boot first so the stream attaches instead of failing with
            // "device is not booted" when the simulator is shut down.
            ctx.out.step("Booting simulator", || simctl::boot(udid))?;
            stream_logs(ctx, &LogSource::Simulator(udid), &app, filters)?;
        }
        // The host's own `log stream`, the same source `app run --mac` uses.
        Target::Mac => stream_logs(ctx, &LogSource::Mac, &app, filters)?,
        Target::Device(_) => {
            return Err(CliError::new(
                "app logs can't follow a physical device yet — a device's os_log needs \
                 pymobiledevice3 (as `app run --device` uses); use `app run --device` to \
                 follow it during a run",
            ));
        }
        Target::SpmRun(_) => {
            return Err(CliError::new(
                "a Swift package executable has no os_log stream; its output goes to the \
                 terminal during `app run`",
            ));
        }
    }
    Ok(Rendered::Streamed)
}

/// `app debug` for a native macOS app: build, then hand the executable to
/// lldb and let *it* launch the process.
///
/// The simulator path has to launch first and attach, because `simctl` owns
/// the launch. Locally there is no such constraint, and lldb-launches is
/// strictly better: breakpoints can be set before `run`, and there is no
/// window where a suspended process is waiting for an attach that might fail.
fn debug_mac(ctx: &mut Context, plan: &RunPlan) -> CommandResult {
    let app = build_and_install(plan, &ctx.out)?;
    // lldb passes its own environment to the target (`target.inherit-env`
    // defaults on), so `--env` reaches the app without a settings dance.
    let env = plan.launch.env_pairs("")?;
    let exe = app.executable.display().to_string();
    let mut args: Vec<&str> = vec!["--", &exe];
    args.extend(plan.launch.args.iter().map(String::as_str));
    ctx.out.note(&format!(
        "starting lldb for {} — type `run` to launch it, `quit` to leave",
        app.bundle_id
    ));
    // Ctrl-C is lldb's break-into-the-debuggee gesture; the terminal delivers
    // it to the whole foreground group, and the CLI dying underneath would
    // orphan lldb against the shell prompt.
    crate::cli::signals::with_sigint_ignored(|| process::stream_env("lldb", &args, None, &env))
        .map(|()| Rendered::Streamed)
}

/// How lldb should reach the app: macOS lldb owns the launch (the executable
/// and its args go after `--`, started with `run`); the simulator path
/// attaches to an already-launched, suspended pid and `continue`s it.
enum LldbTarget<'a> {
    Mac { exe: &'a str, args: &'a [String] },
    AttachPid(u32),
}

impl LldbTarget<'_> {
    /// The lldb verb that starts/resumes the target: `run` when lldb owns the
    /// launch, `continue` when it attached to a suspended process.
    fn start_verb(&self) -> &'static str {
        match self {
            LldbTarget::Mac { .. } => "run",
            LldbTarget::AttachPid(_) => "continue",
        }
    }

    /// The leading `-p <pid>` for an attach, empty when lldb owns the launch.
    fn attach_flag(&self) -> Vec<String> {
        match self {
            LldbTarget::AttachPid(pid) => vec!["-p".to_string(), pid.to_string()],
            LldbTarget::Mac { .. } => Vec::new(),
        }
    }

    /// The trailing `-- <exe> <args…>` for a macOS launch, empty for an attach.
    fn launch_suffix(&self) -> Vec<String> {
        match self {
            LldbTarget::Mac { exe, args } => {
                let mut v = vec!["--".to_string(), (*exe).to_string()];
                v.extend(args.iter().cloned());
                v
            }
            LldbTarget::AttachPid(_) => Vec::new(),
        }
    }
}

/// Push an lldb one-line command (`-o <cmd>`) onto an argv.
fn push_one_line(args: &mut Vec<String>, cmd: &str) {
    args.push("-o".to_string());
    args.push(cmd.to_string());
}

/// Sentinels `app diagnose` prints (via `script print`) between the sections
/// of its lldb chain, so a captured transcript splits into clean pieces even
/// though lldb interleaves prompts and diagnostics. `-Q` suppresses lldb's
/// command echo so nothing but these markers and command output appears.
const SENTINEL_EXC: &str = "@@SWEETPAD_EXC@@";
const SENTINEL_REASON: &str = "@@SWEETPAD_REASON@@";
const SENTINEL_BT: &str = "@@SWEETPAD_BT@@";
const SENTINEL_END: &str = "@@SWEETPAD_END@@";

/// The `lldb -b` argv for `app diagnose`: break on `objc_exception_throw` (lldb
/// then stops with `stop reason = hit Objective-C exception`), start the app,
/// and dump the exception name/reason and a backtrace between the sentinels
/// before killing it. `$arg1` is `objc_exception_throw`'s first argument — the
/// `NSException` — valid only at that breakpoint, so the caller ignores those
/// fields for a plain signal crash or a clean exit.
fn diagnose_lldb_args(target: &LldbTarget) -> Vec<String> {
    let mut a = vec!["-b".to_string(), "-Q".to_string()];
    a.extend(target.attach_flag());
    push_one_line(&mut a, "breakpoint set -n objc_exception_throw");
    push_one_line(&mut a, target.start_verb());
    push_one_line(&mut a, &format!("script print('{SENTINEL_EXC}')"));
    push_one_line(&mut a, "po (id)[(id)$arg1 name]");
    push_one_line(&mut a, &format!("script print('{SENTINEL_REASON}')"));
    push_one_line(&mut a, "po (id)[(id)$arg1 reason]");
    push_one_line(&mut a, &format!("script print('{SENTINEL_BT}')"));
    push_one_line(&mut a, "bt");
    push_one_line(&mut a, &format!("script print('{SENTINEL_END}')"));
    push_one_line(&mut a, "process kill");
    push_one_line(&mut a, "quit");
    a.extend(target.launch_suffix());
    a
}

/// The `lldb -b` argv for `app debug --batch`: forward the user's `--cmd`
/// commands as `-o` (in order) and `--on-crash` commands as `-k`, verbatim.
fn batch_lldb_args(target: &LldbTarget, cmds: &[String], on_crash: &[String]) -> Vec<String> {
    let mut a = vec!["-b".to_string()];
    a.extend(target.attach_flag());
    for c in cmds {
        push_one_line(&mut a, c);
    }
    for c in on_crash {
        a.push("-k".to_string());
        a.push(c.clone());
    }
    a.extend(target.launch_suffix());
    a
}

/// What `app diagnose` extracts from an lldb transcript. `stop_reason` present
/// means the app stopped on an exception or a signal (not a clean exit); the
/// exception name/reason are set only for an Objective-C exception.
#[derive(Debug, Default, PartialEq, Eq)]
struct DiagnoseOutcome {
    stop_reason: Option<String>,
    signal: Option<String>,
    exit_status: Option<i32>,
    exception_name: Option<String>,
    exception_reason: Option<String>,
    backtrace: Vec<String>,
}

impl DiagnoseOutcome {
    fn stopped(&self) -> bool {
        self.stop_reason.is_some()
    }
}

/// Parse an `app diagnose` lldb transcript into an outcome. The first
/// `stop reason = …` is the real stop — the trailing `exited with status = 9
/// killed` from our own `process kill` carries none, so it never masquerades
/// as the result. Section extraction is best-effort; the raw transcript is
/// always carried alongside for the cases parsing can't cover.
fn parse_diagnose(transcript: &str) -> DiagnoseOutcome {
    let stop_reason = transcript.lines().find_map(|l| {
        l.split_once("stop reason = ")
            .map(|(_, r)| r.trim().to_string())
    });
    let is_objc = stop_reason
        .as_deref()
        .is_some_and(|r| r.contains("Objective-C exception"));
    let signal = stop_reason.as_deref().and_then(|r| {
        r.strip_prefix("signal ")
            .map(|s| s.split_whitespace().next().unwrap_or(s).to_string())
    });
    // A clean exit only counts when nothing stopped us first.
    let exit_status = if stop_reason.is_none() {
        transcript
            .split("exited with status = ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|n| n.parse::<i32>().ok())
    } else {
        None
    };

    let section = |start: &str, end: &str| -> Option<String> {
        let s = transcript.find(start)? + start.len();
        let e = transcript[s..]
            .find(end)
            .map_or(transcript.len(), |i| s + i);
        Some(transcript[s..e].trim().to_string())
    };
    // A `po` on a non-exception stop (or after exit) prints an `error:` line;
    // drop those rather than surface them as a name/reason.
    let clean = |v: Option<String>| -> Option<String> {
        v.filter(|s| !s.is_empty() && !s.contains("error:"))
            .map(|s| s.trim_matches('"').to_string())
    };
    let (exception_name, exception_reason) = if is_objc {
        (
            clean(section(SENTINEL_EXC, SENTINEL_REASON)),
            clean(section(SENTINEL_REASON, SENTINEL_BT)),
        )
    } else {
        (None, None)
    };
    let backtrace = section(SENTINEL_BT, SENTINEL_END)
        .map(|bt| {
            bt.lines()
                .map(str::trim)
                .filter(|l| l.contains("frame #"))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    DiagnoseOutcome {
        stop_reason,
        signal,
        exit_status,
        exception_name,
        exception_reason,
        backtrace,
    }
}

/// A `--timeout <secs>` value as a `Duration`; `0` means unbounded.
fn batch_timeout(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

/// Wait for `child`, giving up after `timeout` (zero = wait indefinitely).
/// Returns whether the wait timed out; the caller kills on `true`.
fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> bool {
    if timeout.is_zero() {
        let _ = child.wait();
        return false;
    }
    let start = Instant::now();
    loop {
        match child.try_wait() {
            // Exited, or a wait error we can't recover here — either way, done.
            Ok(Some(_)) | Err(_) => return false,
            Ok(None) => {}
        }
        if start.elapsed() >= timeout {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Kill the debuggee (its pids from `cleanup_pids`) and then lldb — the order
/// that lets lldb notice the inferior died and unwind its own batch cleanly.
fn kill_lldb_and_inferior(child: &mut std::process::Child, cleanup_pids: impl Fn() -> Vec<i32>) {
    for pid in cleanup_pids() {
        // Safety: SIGKILL to a pid we resolved from the app we launched.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Spawn `lldb <args>` with its output **captured** to a temp file, wait up to
/// `timeout`, and on expiry kill the inferior and lldb. Returns the transcript
/// and whether it timed out. Capturing to a file (not a pipe) avoids a
/// full-pipe deadlock when a chatty app blocks lldb inside `run`.
fn run_lldb_captured(
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
    cleanup_pids: impl Fn() -> Vec<i32>,
) -> Result<(String, bool), CliError> {
    let path = std::env::temp_dir().join(format!("sweetpad-diagnose-{}.log", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| {
            CliError::new(format!(
                "failed to open diagnose log {}: {e}",
                path.display()
            ))
        })?;
    let err = file
        .try_clone()
        .map_err(|e| CliError::new(format!("failed to set up diagnose capture: {e}")))?;
    let mut child = std::process::Command::new("lldb")
        .args(args)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(std::process::Stdio::null())
        .stdout(file)
        .stderr(err)
        .spawn()
        .map_err(|e| CliError::new(format!("failed to run `lldb`: {e}")))?;
    let slot = crate::cli::signals::register_child(child.id());
    let timed_out = wait_with_timeout(&mut child, timeout);
    if timed_out {
        kill_lldb_and_inferior(&mut child, cleanup_pids);
    }
    crate::cli::signals::unregister_child(slot);
    let transcript = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    Ok((transcript, timed_out))
}

/// Spawn `lldb <args>` with stdio inherited (output **streams** to the
/// terminal), enforcing `timeout`. Errors if the session had to be killed.
fn run_lldb_streamed(
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
    cleanup_pids: impl Fn() -> Vec<i32>,
) -> Result<(), CliError> {
    let mut child = std::process::Command::new("lldb")
        .args(args)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .spawn()
        .map_err(|e| CliError::new(format!("failed to run `lldb`: {e}")))?;
    let slot = crate::cli::signals::register_child(child.id());
    let timed_out = wait_with_timeout(&mut child, timeout);
    if timed_out {
        kill_lldb_and_inferior(&mut child, cleanup_pids);
    }
    crate::cli::signals::unregister_child(slot);
    if timed_out {
        return Err(CliError::new(format!(
            "lldb --batch hit the {}s timeout and was killed; raise --timeout, or add `quit` to \
             your --cmd chain",
            timeout.as_secs()
        )));
    }
    Ok(())
}

/// `app diagnose`: run the app under lldb, catch the first Objective-C
/// exception or crash, and report it structurally. Simulator and macOS only.
fn diagnose(
    ctx: &mut Context,
    stage_target: &StageTargetArgs,
    launch: &LaunchArgs,
    timeout_secs: u64,
) -> CommandResult {
    let opts = lldb_run_opts(stage_target, launch);
    let plan = plan(ctx, &opts)?;
    match &plan.target {
        Target::Mac => diagnose_mac(ctx, &plan, timeout_secs),
        Target::Simulator(udid) => {
            let udid = udid.clone();
            diagnose_sim(ctx, &plan, &udid, timeout_secs)
        }
        Target::Device(_) => Err(CliError::new(
            "app diagnose can't drive a physical device yet; diagnose on a simulator or macOS",
        )),
        Target::SpmRun(_) => Err(CliError::new(
            "app diagnose works on an app target; for a Swift package run \
             `lldb -b -o run -o bt -- <binary>`",
        )),
    }
}

/// `app diagnose` on macOS: lldb owns the launch, so breakpoints are armed
/// before the process exists.
fn diagnose_mac(ctx: &mut Context, plan: &RunPlan, timeout_secs: u64) -> CommandResult {
    let app = build_and_install(plan, &ctx.out)?;
    // lldb passes its own environment to the target, so `--env` reaches it.
    let env = plan.launch.env_pairs("")?;
    let exe = app.executable.display().to_string();
    let args = diagnose_lldb_args(&LldbTarget::Mac {
        exe: &exe,
        args: &plan.launch.args,
    });
    ctx.out.note(&format!(
        "running {} under lldb (timeout {timeout_secs}s) — catching the first exception or crash",
        app.bundle_id
    ));
    let executable = app.executable.clone();
    let (transcript, timed_out) =
        run_lldb_captured(&args, &env, batch_timeout(timeout_secs), || {
            macwin::pids_for_executable(&executable).unwrap_or_default()
        })?;
    Ok(Rendered::data(DiagnoseReport {
        target: "macOS",
        bundle_id: app.bundle_id,
        pid: None,
        timed_out,
        timeout_secs,
        outcome: parse_diagnose(&transcript),
        transcript,
    }))
}

/// `app diagnose` on a simulator: launch suspended, then attach lldb.
fn diagnose_sim(ctx: &mut Context, plan: &RunPlan, udid: &str, timeout_secs: u64) -> CommandResult {
    let (app, pid) = launch_suspended_on_sim(ctx, plan, udid)?;
    let args = diagnose_lldb_args(&LldbTarget::AttachPid(pid));
    ctx.out.note(&format!(
        "attaching lldb to {} (pid {pid}, timeout {timeout_secs}s) — catching the first \
         exception or crash",
        app.bundle_id
    ));
    let pid_i32 = i32::try_from(pid).unwrap_or(0);
    let (transcript, timed_out) =
        run_lldb_captured(&args, &[], batch_timeout(timeout_secs), || vec![pid_i32])?;
    Ok(Rendered::data(DiagnoseReport {
        target: "simulator",
        bundle_id: app.bundle_id,
        pid: Some(pid),
        timed_out,
        timeout_secs,
        outcome: parse_diagnose(&transcript),
        transcript,
    }))
}

/// The `app diagnose` payload: the parsed outcome plus the full lldb
/// transcript. Human mode prints a one-line verdict and the backtrace; `--json`
/// carries every field, including the transcript, for an agent to act on.
struct DiagnoseReport {
    target: &'static str,
    bundle_id: String,
    pid: Option<u32>,
    timed_out: bool,
    timeout_secs: u64,
    outcome: DiagnoseOutcome,
    transcript: String,
}

impl Render for DiagnoseReport {
    fn human(&self, out: &Output) {
        if self.timed_out {
            out.note(&format!(
                "{}: no exception or crash within {}s — the app was still running and has been \
                 killed",
                self.bundle_id, self.timeout_secs
            ));
            return;
        }
        match (
            &self.outcome.exception_name,
            &self.outcome.signal,
            self.outcome.exit_status,
        ) {
            (Some(name), _, _) => {
                let reason = self
                    .outcome
                    .exception_reason
                    .as_deref()
                    .unwrap_or("<no reason>");
                out.note(&format!(
                    "{}: caught Objective-C exception {name}: {reason}",
                    self.bundle_id
                ));
            }
            (None, Some(sig), _) => {
                out.note(&format!("{}: crashed with {sig}", self.bundle_id));
            }
            (None, None, Some(status)) => {
                out.note(&format!(
                    "{}: exited cleanly (status {status}) — no exception or crash observed",
                    self.bundle_id
                ));
            }
            _ => out.note(&format!(
                "{}: {}",
                self.bundle_id,
                self.outcome
                    .stop_reason
                    .as_deref()
                    .unwrap_or("no stop observed")
            )),
        }
        if !self.outcome.backtrace.is_empty() {
            out.line("");
            for frame in &self.outcome.backtrace {
                out.line(frame);
            }
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "bundleId": self.bundle_id,
            "target": self.target,
            "pid": self.pid,
            "timedOut": self.timed_out,
            "timeoutSecs": self.timeout_secs,
            "stopped": self.outcome.stopped(),
            "stopReason": self.outcome.stop_reason,
            "signal": self.outcome.signal,
            "exitStatus": self.outcome.exit_status,
            "exception": self.outcome.exception_name.as_ref().map(|name| serde_json::json!({
                "name": name,
                "reason": self.outcome.exception_reason,
            })),
            "backtrace": self.outcome.backtrace,
            "transcript": self.transcript,
        })
    }
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
    let last = last_launched(ctx)?;
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
/// mode, or `{ action, bundleId, udid, pid, detail }` in the JSON envelope.
/// `udid` carries the simulator/device id; a macOS stage has none and reports
/// the process `pid` instead.
struct AppStageReport {
    action: &'static str,
    note: String,
    bundle_id: String,
    udid: Option<String>,
    pid: Option<i32>,
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
            "pid": self.pid,
            "detail": self.detail,
        })
    }
}

/// The `app screenshot` payload: where the PNG landed and what was captured
/// (`udid` for a simulator capture; `pid`/`windowId`/`windows` for a macOS
/// window).
struct ShotReport {
    path: String,
    /// What was captured, for the human note; not serialized.
    label: String,
    udid: Option<String>,
    pid: Option<i32>,
    window_id: Option<u32>,
    bundle_id: Option<String>,
    windows: Option<usize>,
}

impl Render for ShotReport {
    fn human(&self, out: &Output) {
        out.note(&format!(
            "saved screenshot of {} to {}",
            self.label, self.path
        ));
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path,
            "udid": self.udid,
            "pid": self.pid,
            "windowId": self.window_id,
            "bundleId": self.bundle_id,
            "windows": self.windows,
        })
    }
}

/// A macOS app to capture: its live pids plus naming for the default file
/// and the report.
struct MacShot {
    pids: Vec<i32>,
    /// Display name — the executable name, or `pid N` — used for the note
    /// and (slugged) the default filename.
    name: String,
    bundle_id: Option<String>,
}

/// `app screenshot` — capture the running app to a PNG (CLI_DESIGN §9h): a
/// macOS app's window via the window server + `screencapture`, or the
/// simulator it launched on via `simctl io screenshot`. Resolution mirrors
/// `stop`: an explicit `--pid` wins, then the recorded last launch, then the
/// resolved build target — no build, ever.
fn screenshot(ctx: &mut Context, args: &ScreenshotArgs) -> CommandResult {
    if let Some(pid) = args.pid {
        if explicit_targeting(ctx) {
            return Err(CliError::new(
                "--pid captures a process directly; scheme/destination flags don't apply",
            ));
        }
        // Positive pids only — 0/negative would address a process *group* in
        // the liveness probe below.
        if pid <= 0 {
            return Err(CliError::new("--pid takes a positive process id"));
        }
        // ESRCH now beats "no on-screen window" after a 5s wait.
        if unsafe { libc::kill(pid, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return Err(CliError::new(format!("no process with pid {pid}")));
        }
        return mac_screenshot(
            ctx,
            &MacShot {
                pids: vec![pid],
                name: format!("pid {pid}"),
                bundle_id: None,
            },
            args,
        );
    }

    // The recorded last launch serves the no-flags case with no scheme
    // resolution and no prompting, exactly like `stop`.
    if !explicit_targeting(ctx)
        && let Some(last) = last_launched(ctx)
    {
        match last.kind.as_str() {
            "macos" => {
                if let Some(exe) = mac_executable(&last) {
                    let shot = mac_shot_for(&exe, &last.bundle_identifier)?;
                    return mac_screenshot(ctx, &shot, args);
                }
                // No executable recorded (an older state file) — fall through
                // to the resolver, which knows it.
            }
            "simulator" => {
                if let Some(udid) = last.simulator_udid.clone() {
                    return simulator_screenshot(ctx, &udid, args);
                }
            }
            "device" => {
                return Err(CliError::new(
                    "physical devices don't support screenshots (devicectl exposes no capture)",
                ));
            }
            _ => {}
        }
    }

    // Full resolution: the destination decides which capture path runs.
    let opts = RunOpts {
        device: false,
        device_id: None,
        mac: false,
        no_logs: true,
        detach: false,
        hot: false,
        hot_explicit: false,
        hot_mode: Mode::Resolver,
        hot_selfcheck: None,
        keep_sandbox: false,
        hot_entitlements: None,
        launch: &LaunchArgs::default(),
        passthrough: &[],
    };
    let plan = plan(ctx, &opts)?;
    match &plan.target {
        Target::Mac => {
            let app = plan.app_bundle()?;
            let shot = mac_shot_for(&app.executable, &app.bundle_id)?;
            mac_screenshot(ctx, &shot, args)
        }
        Target::Simulator(udid) => simulator_screenshot(ctx, udid, args),
        Target::Device(_) => Err(CliError::new(
            "physical devices don't support screenshots (devicectl exposes no capture)",
        )),
        Target::SpmRun(_) => Err(CliError::new(
            "a Swift package executable has no app bundle to capture; use --pid for a \
             window it opened",
        )),
    }
}

/// Build the [`MacShot`] for an executable path, erroring when nothing runs.
fn mac_shot_for(executable: &Path, bundle_id: &str) -> Result<MacShot, CliError> {
    let pids = macwin::pids_for_executable(executable)?;
    if pids.is_empty() {
        return Err(CliError::new(format!(
            "{bundle_id} isn't running — launch it with `sweetpad app run --mac --no-logs`"
        )));
    }
    Ok(MacShot {
        pids,
        name: process_name_of(executable),
        bundle_id: Some(bundle_id.to_string()),
    })
}

/// The executable's file name, for notes and the default screenshot name.
fn process_name_of(executable: &Path) -> String {
    executable
        .file_name()
        .map_or_else(|| "app".to_string(), |n| n.to_string_lossy().into_owned())
}

/// Capture a macOS app's window: TCC preflight, a short grace poll for the
/// first window (a just-`open`ed app may not have mapped one yet), pick,
/// `screencapture`.
fn mac_screenshot(ctx: &Context, shot: &MacShot, args: &ScreenshotArgs) -> CommandResult {
    let (window, count) = wait_for_window(ctx, shot, args.window)?;
    let path = args
        .output_file
        .clone()
        .unwrap_or_else(|| super::simulator::default_screenshot_path(&shot.name));
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(parent);
    }
    macwin::capture_window(window.number, &path)?;
    if args.clipboard {
        super::simulator::copy_png_to_clipboard(&path)?;
        ctx.out.note("copied to the clipboard");
    }
    Ok(Rendered::data(ShotReport {
        path: path.display().to_string(),
        // In `--pid` mode the name *is* "pid N" — don't repeat it.
        label: if shot.bundle_id.is_some() {
            format!("{} (pid {})", shot.name, window.pid)
        } else {
            shot.name.clone()
        },
        udid: None,
        pid: Some(window.pid),
        window_id: Some(window.number),
        bundle_id: shot.bundle_id.clone(),
        windows: Some(count),
    }))
}

/// The window-poll + permission half of [`mac_screenshot`], shared with the
/// session's `s` key. Fails fast on a missing Screen Recording permission —
/// without it `screencapture` silently produces the wallpaper — requesting
/// the one-time OS prompt only on an interactive terminal.
fn wait_for_window(
    ctx: &Context,
    shot: &MacShot,
    index: Option<usize>,
) -> Result<(macwin::WindowInfo, usize), CliError> {
    if index == Some(0) {
        return Err(CliError::new("--window is 1-based (1 is the frontmost)"));
    }
    if !macwin::has_screen_capture_access() {
        if ctx.out.is_interactive() {
            macwin::request_screen_capture_access();
        }
        return Err(macwin::permission_error());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let windows = macwin::list_windows()?;
        match macwin::pick_window(&windows, &shot.pids, index) {
            Ok(picked) => return Ok(picked),
            Err(reason) => {
                if Instant::now() >= deadline {
                    return Err(CliError::new(format!("{}: {reason}", shot.name)));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}

/// The simulator side of `app screenshot`: the same `simctl io screenshot`
/// capture `simulator screenshot` does, reached from the app's own verb so
/// one command serves the loop on either destination.
fn simulator_screenshot(ctx: &Context, udid: &str, args: &ScreenshotArgs) -> CommandResult {
    // simctl captures the whole device screen; there is no window to pick.
    // Accepting the flag and ignoring it would silently not do what was asked.
    if args.window.is_some() {
        return Err(CliError::new(
            "--window applies to a macOS app's windows; a simulator capture is the whole \
             device screen",
        ));
    }
    let name = sim_name(udid).unwrap_or_else(|| "simulator".to_string());
    let path = args
        .output_file
        .clone()
        .unwrap_or_else(|| super::simulator::default_screenshot_path(&name));
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(parent);
    }
    simctl::screenshot(udid, &path.display().to_string())?;
    if args.clipboard {
        super::simulator::copy_png_to_clipboard(&path)?;
        ctx.out.note("copied to the clipboard");
    }
    Ok(Rendered::data(ShotReport {
        path: path.display().to_string(),
        label: name,
        udid: Some(udid.to_string()),
        pid: None,
        window_id: None,
        bundle_id: None,
        windows: None,
    }))
}

/// The `app ui tree` payload: the app that was inspected and its element
/// tree.
struct UiTreeReport {
    app: String,
    pid: i32,
    root: ax::Node,
}

impl Render for UiTreeReport {
    fn human(&self, out: &Output) {
        for line in ax::outline(&self.root) {
            out.line(&line);
        }
        // A `--pid` run names the app "pid N" already; don't say it twice.
        let where_ = if self.app == format!("pid {}", self.pid) {
            self.app.clone()
        } else {
            format!("{} (pid {})", self.app, self.pid)
        };
        out.note(&format!("{} elements in {where_}", self.root.count()));
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "app": self.app,
            "pid": self.pid,
            "elements": self.root.count(),
            "tree": ax::to_json(&self.root),
        })
    }
}

/// The `app ui click` / `app ui type` payload: what was acted on, and how.
struct UiActReport {
    app: String,
    pid: i32,
    /// `click` or `type` — the verb, for the note and the JSON.
    verb: &'static str,
    element: String,
    path: Vec<usize>,
    /// The text written, for `type` only.
    text: Option<String>,
}

impl Render for UiActReport {
    fn human(&self, out: &Output) {
        match &self.text {
            Some(text) => out.note(&format!("set {} to {text:?} in {}", self.element, self.app)),
            None => out.note(&format!("clicked {} in {}", self.element, self.app)),
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "app": self.app,
            "pid": self.pid,
            "action": self.verb,
            "element": self.element,
            "path": self.path,
            "text": self.text,
        })
    }
}

/// `app ui` — inspect or drive a running macOS app (CLI_DESIGN §9i). A bare
/// `app ui` reads as `app ui tree`, the one verb that only observes.
fn ui(ctx: &mut Context, action: Option<&UiAction>) -> CommandResult {
    let Some(action) = action else {
        ctx.targeting = crate::cli::Targeting::from_env();
        return ui_tree(ctx, None, 20);
    };
    match action {
        UiAction::Tree(args) => {
            ctx.targeting = args.app.target.clone().into();
            ui_tree(ctx, args.app.pid, args.depth)
        }
        UiAction::Click(args) => {
            ctx.targeting = args.app.target.clone().into();
            ui_act(ctx, &args.app, &args.query, None)
        }
        UiAction::Type(args) => {
            ctx.targeting = args.app.target.clone().into();
            ui_act(ctx, &args.app, &args.query, Some(&args.text))
        }
    }
}

/// `app ui tree` — snapshot and print the whole exposed hierarchy.
fn ui_tree(ctx: &mut Context, pid: Option<i32>, depth: usize) -> CommandResult {
    let shot = resolve_ui_app(ctx, pid)?;
    let pid = ui_preflight(ctx, &shot)?;
    let root = ax::snapshot(pid, depth)?;
    Ok(Rendered::data(UiTreeReport {
        app: shot.name,
        pid,
        root,
    }))
}

/// The shared body of `app ui click` and `app ui type`: resolve the app,
/// snapshot it, match one element, act. `text` decides which.
fn ui_act(
    ctx: &mut Context,
    app: &UiAppArgs,
    query: &UiQueryArgs,
    text: Option<&str>,
) -> CommandResult {
    let query = ax::Query {
        label: query.label.clone(),
        role: query.role.clone(),
        nth: query.nth,
    };
    // An empty query would match the application element itself and press
    // something arbitrary; make the caller say what they meant.
    if query.is_empty() {
        return Err(CliError::new(
            "name the element with --label, or --role for a lone control; \
             `sweetpad app ui tree` shows what the app exposes",
        ));
    }
    let shot = resolve_ui_app(ctx, app.pid)?;
    let pid = ui_preflight(ctx, &shot)?;
    let root = ax::snapshot(pid, usize::MAX)?;
    let target = ax::find(&root, &query).map_err(CliError::new)?;

    match text {
        Some(text) => ax::act(pid, target, &ax::Act::SetValue(text))?,
        None => ax::act(pid, target, &ax::Act::Perform("AXPress"))?,
    }
    Ok(Rendered::data(UiActReport {
        app: shot.name,
        pid,
        verb: if text.is_some() { "type" } else { "click" },
        element: target.describe(),
        path: target.path.clone(),
        text: text.map(str::to_string),
    }))
}

/// Check the Accessibility grant and settle on one pid to drive.
///
/// An app with several live processes is ambiguous in a way a screenshot's
/// frontmost-window rule isn't — there is no "frontmost" element tree — so
/// this refuses rather than picking.
fn ui_preflight(ctx: &Context, shot: &MacShot) -> Result<i32, CliError> {
    if !ax::has_accessibility_access() {
        if ctx.out.is_interactive() {
            ax::request_accessibility_access();
        }
        return Err(ax::permission_error());
    }
    match shot.pids.as_slice() {
        [pid] => Ok(*pid),
        [] => Err(CliError::new(format!("{} isn't running", shot.name))),
        many => Err(CliError::new(format!(
            "{} has {} running processes ({}); pass --pid to say which",
            shot.name,
            many.len(),
            many.iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        ))),
    }
}

/// Resolve which macOS app `app ui` drives, mirroring `screenshot`'s order:
/// an explicit `--pid` wins, then the recorded last launch, then the resolved
/// build target. Never builds.
fn resolve_ui_app(ctx: &mut Context, pid: Option<i32>) -> Result<MacShot, CliError> {
    if let Some(pid) = pid {
        if explicit_targeting(ctx) {
            return Err(CliError::new(
                "--pid drives a process directly; scheme/destination flags don't apply",
            ));
        }
        if pid <= 0 {
            return Err(CliError::new("--pid takes a positive process id"));
        }
        if unsafe { libc::kill(pid, 0) } != 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return Err(CliError::new(format!("no process with pid {pid}")));
        }
        return Ok(MacShot {
            pids: vec![pid],
            name: format!("pid {pid}"),
            bundle_id: None,
        });
    }

    if !explicit_targeting(ctx)
        && let Some(last) = last_launched(ctx)
    {
        match last.kind.as_str() {
            "macos" => {
                if let Some(exe) = mac_executable(&last) {
                    return mac_shot_for(&exe, &last.bundle_identifier);
                }
            }
            "simulator" | "device" => return Err(ui_not_mac(&last.kind)),
            _ => {}
        }
    }

    let opts = RunOpts {
        device: false,
        device_id: None,
        mac: false,
        no_logs: true,
        detach: false,
        hot: false,
        hot_explicit: false,
        hot_mode: Mode::Resolver,
        hot_selfcheck: None,
        keep_sandbox: false,
        hot_entitlements: None,
        launch: &LaunchArgs::default(),
        passthrough: &[],
    };
    let plan = plan(ctx, &opts)?;
    match &plan.target {
        Target::Mac => {
            let app = plan.app_bundle()?;
            mac_shot_for(&app.executable, &app.bundle_id)
        }
        Target::Simulator(_) => Err(ui_not_mac("simulator")),
        Target::Device(_) => Err(ui_not_mac("device")),
        Target::SpmRun(_) => Err(CliError::new(
            "a Swift package executable has no app bundle to drive; use --pid for a \
             process it started",
        )),
    }
}

/// The one error every non-macOS destination gets, naming what does work
/// there instead.
fn ui_not_mac(kind: &str) -> CliError {
    CliError::new(format!(
        "`app ui` drives macOS apps through the Accessibility API, which doesn't reach a \
         {kind}. For a simulator, `app screenshot` captures the screen and `app open-url` \
         drives it by deep link; scripted taps need a UI test target run through \
         `sweetpad test`"
    ))
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
fn stream_logs(
    ctx: &Context,
    source: &LogSource,
    app: &AppBundle,
    filters: &LogFilterArgs,
) -> CliResult {
    let is_mac = matches!(source, LogSource::Mac);
    // The captured stdout/stderr file only exists for macOS launches.
    if filters.source == LogChannel::Stdout && !is_mac {
        return Err(CliError::new(
            "--source stdout applies to macOS apps (the stdout/stderr a detached launch \
             captures); simulator and device logs are os_log only",
        ));
    }

    // --last: dump recent history and return, rather than following — for an
    // app that has gone quiet or already exited.
    if let Some(last) = filters.last.as_deref() {
        return backfill_logs(ctx, source, app, filters, last);
    }

    let color = ctx.out.use_color();
    let json = ctx.out.is_json() || ctx.out.is_ndjson();
    let limit = filters.timeout;
    let hit = Arc::new(AtomicBool::new(false));

    // On macOS, follow the captured stdout/stderr file whenever the channel
    // includes it: a Mac app that logs through `print` writes nothing to os_log,
    // so an os_log-only follow would sit empty while the app is talking.
    let console_file = (is_mac && filters.source != LogChannel::Oslog)
        .then(|| detached_log_path(&app.bundle_id))
        .flatten()
        .filter(|p| p.exists());

    ctx.out.note(&match (filters.until.as_deref(), limit) {
        (Some(text), _) => format!("Following {} until {text:?}", app.bundle_id),
        (None, Some(d)) => format!("Streaming logs for {} for {}s", app.bundle_id, d.as_secs()),
        (None, None) => format!("Streaming logs for {} (Ctrl-C to stop)", app.bundle_id),
    });

    // stdout-only: there's no os_log stream to run, so the file tail is the
    // whole job and owns this thread. A childless follow ends on Ctrl-C via the
    // signal handler's default (kill children, exit) — like `tail -f`.
    if filters.source == LogChannel::Stdout {
        let Some(path) = console_file else {
            return Err(CliError::new(format!(
                "no captured output for {} — a detached launch (`app run --detach`, \
                 `app launch --mac`, or `app run --mac --no-logs`) writes it; a foreground \
                 `app run --mac` streams stdout inline instead",
                app.bundle_id
            )));
        };
        follow_captured_only(&path, color, json, filters, &hit, limit);
        return until_result(ctx, filters, &hit, limit);
    }

    // os_log stream (the reap-on-exit machinery below), plus the file tail on a
    // background thread for `both`. `stop` ends that thread once the stream does.
    let stop = Arc::new(AtomicBool::new(false));
    let level = filters.level.as_deref().unwrap_or(log_level(&ctx.out));
    let marker = log_stream_marker();
    let (program, args) = log_command(source, app, level, Some(&marker), filters);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut child = process::spawn_piped(program, &refs, None)?;
    let stream_pid = child.id();

    // Both sources can satisfy `--until`, and either one matching ends the
    // stream child — which is also what unblocks the reader below.
    let watch = filters.until.as_deref().map(|text| UntilWatch {
        text: text.to_string(),
        hit: Arc::clone(&hit),
        stream_pid: Some(stream_pid),
    });
    if let Some(path) = console_file {
        let stop = Arc::clone(&stop);
        let watch = filters.until.as_deref().map(|text| UntilWatch {
            text: text.to_string(),
            hit: Arc::clone(&hit),
            stream_pid: Some(stream_pid),
        });
        std::thread::spawn(move || {
            follow_console_file(&path, color, json, &stop, watch.as_ref());
        });
    }
    // The deadline ends the same child a match would, so both exits leave the
    // reader through one path. It stands down early when the stream ends first.
    if let Some(d) = limit {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            if expire(d, &stop) {
                process::terminate(stream_pid);
            }
        });
    }

    let reap_slot = crate::cli::signals::register_child(stream_pid);
    crate::cli::signals::set_forward_child(stream_pid);
    if let Some(stdout) = child.stdout.take() {
        process::read_lines_lossy(stdout, &mut |line: &str| {
            emit_log_line(line, color, json);
            if let Some(w) = watch.as_ref() {
                w.sees(&oslog::render_ndjson_line(line, false).text);
            }
        });
    }
    // The stream has ended (child exiting): disarm before the reap so the
    // handler can never signal a recycled pid, and stop the file tail.
    crate::cli::signals::clear_forward_child();
    crate::cli::signals::unregister_child(reap_slot);
    stop.store(true, Ordering::Relaxed);
    let status = child.wait();
    let _ = process::run("pkill", &["-f", &marker], None, true);
    if crate::cli::signals::take_forwarded() {
        ctx.out.note("log stream stopped");
        return Ok(());
    }
    // A `--until` match or a deadline ends the child by signal, so its status is
    // this command's outcome only when neither of those did it.
    if filters.until.is_some() || limit.is_some() {
        return until_result(ctx, filters, &hit, limit);
    }
    match status {
        Ok(s) if s.success() => Ok(()),
        _ => Err(CliError::new("log stream exited with a non-zero status")),
    }
}

/// `--source stdout`: the captured file is the only source, so the tail owns
/// this thread and a `--until` match simply returns from it. A deadline raises
/// the same `stop` the tail already watches.
fn follow_captured_only(
    path: &Path,
    color: bool,
    json: bool,
    filters: &LogFilterArgs,
    hit: &Arc<AtomicBool>,
    limit: Option<Duration>,
) {
    let stop = Arc::new(AtomicBool::new(false));
    let watch = filters.until.as_deref().map(|text| UntilWatch {
        text: text.to_string(),
        hit: Arc::clone(hit),
        stream_pid: None,
    });
    if let Some(d) = limit {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            expire(d, &stop);
            stop.store(true, Ordering::Relaxed);
        });
    }
    follow_console_file(path, color, json, &stop, watch.as_ref());
}

/// Sleep out `limit` in short slices, returning whether it ran to the end —
/// `false` means `stop` was raised first (the stream ended on its own) and the
/// deadline should stand down rather than signal an unrelated pid.
fn expire(limit: Duration, stop: &AtomicBool) -> bool {
    const SLICE: Duration = Duration::from_millis(100);
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(SLICE);
    }
    !stop.load(Ordering::Relaxed)
}

/// The outcome of a bounded follow. `--until` is a question — exit 0 only when
/// the text showed up, and report the deadline plainly when it didn't, since an
/// agent's next move differs entirely between "saw it" and "gave up". A bare
/// `--timeout` asks nothing, so reaching the deadline is success.
fn until_result(
    ctx: &Context,
    filters: &LogFilterArgs,
    hit: &AtomicBool,
    limit: Option<Duration>,
) -> CliResult {
    let Some(text) = filters.until.as_deref() else {
        return Ok(());
    };
    if hit.load(Ordering::Relaxed) {
        ctx.out.note(&format!("Matched {text:?}"));
        return Ok(());
    }
    Err(CliError::new(match limit {
        Some(d) => format!(
            "timed out after {}s without a log line containing {text:?}",
            d.as_secs()
        ),
        None => format!("log stream ended without a line containing {text:?}"),
    }))
}

/// Emit one `log stream` ndjson line: verbatim in json/ndjson mode (it is
/// already one object per line), or rendered as a colored `HH:MM:SS.sss L [cat]`
/// line otherwise. Shared by the live follow ([`stream_logs`]) and the backfill
/// ([`backfill_logs`]).
#[allow(clippy::print_stdout)] // the point of `app logs` is stdout
fn emit_log_line(line: &str, color: bool, json: bool) {
    if json {
        // Already one JSON object per line; emit the event verbatim.
        if line.trim_start().starts_with('{') {
            println!("{line}");
        }
    } else {
        println!("{}", oslog::render_ndjson_line(line, color).text);
    }
}

/// Emit one line of a macOS app's captured stdout/stderr. In json mode it
/// becomes a `{"source":"stdout",…}` object — the `eventMessage`/`timestamp`
/// keys match the os_log ndjson schema, and `source` marks it apart from the
/// os_log events on the same stream. Otherwise it renders as a blue `[print]`
/// note ([`oslog::render_console_line`]), stamped with the local arrival time so
/// it lines up with the os_log lines. Input is bytes (the app owns its stdout,
/// which need not be UTF-8); a trailing newline is trimmed.
#[allow(clippy::print_stdout)] // the point of `app logs` is stdout
fn emit_console_line(buf: &[u8], color: bool, json: bool) {
    let mut line = buf;
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line = &line[..line.len() - 1];
    }
    let text = String::from_utf8_lossy(line);
    let now = oslog::now_clock();
    if json {
        let obj = serde_json::json!({
            "source": "stdout",
            "timestamp": now,
            "eventMessage": text,
        });
        println!("{obj}");
    } else {
        println!(
            "{}",
            oslog::render_console_line(Some(&now), &text, color).text
        );
    }
}

/// Follow a detached macOS app's captured stdout/stderr file
/// ([`detached_log_path`]) from the top, then keep reading as it grows — the
/// `tail -f` half of `app logs` on macOS. Reading from the top is safe because
/// [`open_detached_log`] truncates per launch, so the file holds only the
/// current run. Returns when `stop` is set (the os_log stream ended) or the file
/// becomes unreadable; a childless caller instead ends on Ctrl-C.
fn follow_console_file(
    path: &Path,
    color: bool,
    json: bool,
    stop: &AtomicBool,
    watch: Option<&UntilWatch>,
) {
    use std::io::{BufRead, BufReader};
    /// Cap an unterminated line so a newline-less binary dump can't grow the
    /// buffer without bound (mirrors [`process::read_lines_lossy`]).
    const MAX_LINE: usize = 1 << 20;
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };
    // A captured line is its own message, so `--until` matches it directly
    // rather than through the os_log renderer.
    let matched = |buf: &[u8]| watch.is_some_and(|w| w.sees(&String::from_utf8_lossy(buf)));
    let mut reader = BufReader::new(file);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = match reader.fill_buf() {
            // At EOF: flush any pending partial line, then stop or wait for the
            // app to write more. A regular file keeps yielding new bytes past a
            // previous EOF, so re-polling is all `tail -f` needs.
            Ok([]) => {
                if stop.load(Ordering::Relaxed) {
                    if !buf.is_empty() {
                        emit_console_line(&buf, color, json);
                    }
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            Ok(chunk) => chunk,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        let consumed = if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..=pos]);
            emit_console_line(&buf, color, json);
            if matched(&buf) {
                return;
            }
            buf.clear();
            pos + 1
        } else {
            buf.extend_from_slice(available);
            available.len()
        };
        reader.consume(consumed);
        if buf.len() >= MAX_LINE {
            emit_console_line(&buf, color, json);
            if matched(&buf) {
                return;
            }
            buf.clear();
        }
    }
}

/// `app logs --last <dur>`: dump recent history and return, instead of
/// following. os_log history comes from `log show --last` (`log stream` keeps no
/// history); on macOS the captured stdout/stderr file is dumped too. The channel
/// selects which. Physical devices never reach here — [`simple_logs`] refuses
/// them before this, and their syslog has no history query anyway.
fn backfill_logs(
    ctx: &Context,
    source: &LogSource,
    app: &AppBundle,
    filters: &LogFilterArgs,
    last: &str,
) -> CliResult {
    let color = ctx.out.use_color();
    let json = ctx.out.is_json() || ctx.out.is_ndjson();

    if filters.source != LogChannel::Stdout {
        let level = filters.level.as_deref().unwrap_or(log_level(&ctx.out));
        let (program, args) = log_show_command(source, app, filters, last, level);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let run = process::run_captured(program, &refs, None)?;
        if !run.success {
            return Err(CliError::new(format!(
                "log show failed — {}",
                run.tail.trim()
            )));
        }
        for line in run.combined.lines() {
            emit_log_line(line, color, json);
        }
    }

    // The captured file is the whole current run (truncated per launch), so it's
    // dumped in full rather than sliced by `last` — its lines carry no os_log
    // timestamp to slice on.
    if matches!(source, LogSource::Mac)
        && filters.source != LogChannel::Oslog
        && let Some(path) = detached_log_path(&app.bundle_id).filter(|p| p.exists())
        && let Ok(bytes) = std::fs::read(&path)
    {
        for line in bytes.split(|&b| b == b'\n') {
            if !line.is_empty() {
                emit_console_line(line, color, json);
            }
        }
    }
    Ok(())
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
    fn the_pid_is_read_from_simctls_bundle_colon_pid_line() {
        assert_eq!(launched_pid("com.example.App: 84637\n"), Some(84637));
        // A bundle id contains dots, not colons, so the last field is the pid.
        assert_eq!(launched_pid("com.a.b.c: 1"), Some(1));
        // Never worth failing a launch that already succeeded.
        assert_eq!(launched_pid(""), None);
        assert_eq!(launched_pid("com.example.App: not-a-pid"), None);
    }

    #[test]
    fn a_detached_launch_narrates_itself_in_the_order_it_happened() {
        // The hint names the app the line above launched, so it has to come
        // after it. It read the other way around while it was printed as the
        // report was built and the report rendered afterwards.
        let notes = detached_mac_notes("com.example.App", 4242, Some(Path::new("/tmp/App.log")));
        let at = |needle: &str| {
            notes
                .iter()
                .position(|n| n.contains(needle))
                .unwrap_or_else(|| panic!("no {needle} line in {notes:?}"))
        };
        assert!(at("Launched") < at("output →"), "{notes:?}");
        assert!(at("output →") < at("app stop"), "{notes:?}");
        assert!(notes[0].contains("pid 4242"), "{notes:?}");
        // Backticks render literally in a terminal.
        assert!(!notes.iter().any(|n| n.contains('`')), "{notes:?}");

        // No captured output: that line drops out, the order of the rest holds.
        let notes = detached_mac_notes("com.example.App", 7, None);
        assert_eq!(notes.len(), 2);
        assert!(notes[0].starts_with("Launched"), "{notes:?}");
        assert!(notes[1].contains("app stop"), "{notes:?}");
    }

    #[test]
    fn only_the_streaming_forms_of_app_run_refuse_machine_output() {
        // The refusal is about what *this* invocation does, not about the verb:
        // `--no-logs`/`--detach` build, launch and exit, so they have a payload.
        let out = |mode| {
            Output::new(&crate::cli::GlobalArgs {
                chdir: None,
                developer_dir: None,
                output: mode,
                json: false,
                non_interactive: true,
                no_color: true,
                verbose: false,
                quiet: false,
                gh_annotations: false,
            })
        };
        let json = out(Some(crate::cli::OutputMode::Json));
        let ndjson = out(Some(crate::cli::OutputMode::Ndjson));
        let human = out(None);

        assert!(streaming_under_machine_output(&json, true).is_some());
        assert!(streaming_under_machine_output(&ndjson, true).is_some());
        assert!(streaming_under_machine_output(&json, false).is_none());
        // Human mode streams happily — the guard is only about machine output.
        assert!(streaming_under_machine_output(&human, true).is_none());
    }

    #[test]
    fn udid_extracted_from_destination() {
        assert_eq!(udid("platform=iOS Simulator,id=ABCD").unwrap(), "ABCD");
        assert_eq!(udid("id=XYZ,platform=iOS Simulator").unwrap(), "XYZ");
        assert!(udid("platform=iOS Simulator,name=iPhone 15").is_err());
    }

    #[test]
    fn a_config_default_turns_hot_on_for_simulators_only() {
        let sim = Target::Simulator("UDID".into());
        // A typed `--hot` holds for every target; refusing devices and SPM
        // executables is `run_hot_session`'s job, with a reason.
        assert!(session_hot(true, true, &sim));
        assert!(session_hot(true, true, &Target::Mac));
        assert!(session_hot(true, true, &Target::Device("UDID".into())));

        // The `[run] hot = true` default yields on everything but a simulator,
        // so a committed file can't break those runs.
        assert!(session_hot(true, false, &sim));
        assert!(
            !session_hot(true, false, &Target::Mac),
            "a committed default must not send a mac run down the hot path"
        );
        assert!(!session_hot(true, false, &Target::Device("UDID".into())));
        assert!(!session_hot(true, false, &Target::SpmRun("cli".into())));

        // No hot at all (or `--no-hot`, already folded in by the caller) wins.
        assert!(!session_hot(false, true, &sim));
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

    #[test]
    fn log_show_command_wraps_simctl_and_maps_level_to_flags() {
        let app = test_app();
        // macOS at debug: the host `log show`, `--last`, and both verbosity flags.
        let (program, args) = log_show_command(
            &LogSource::Mac,
            &app,
            &LogFilterArgs::default(),
            "2m",
            "debug",
        );
        assert_eq!(program, "log");
        assert_eq!(&args[..5], &["show", "--style", "ndjson", "--last", "2m"]);
        assert!(args.contains(&"--info".to_string()) && args.contains(&"--debug".to_string()));
        // Simulator at info: wrapped in `simctl spawn`, only `--info` (no `--debug`).
        let (program, args) = log_show_command(
            &LogSource::Simulator("UDID-1"),
            &app,
            &LogFilterArgs::default(),
            "90s",
            "info",
        );
        assert_eq!(program, "xcrun");
        assert_eq!(&args[..5], &["simctl", "spawn", "UDID-1", "log", "show"]);
        assert!(args.contains(&"--info".to_string()) && !args.contains(&"--debug".to_string()));
    }

    #[test]
    fn log_channel_defaults_to_both() {
        assert_eq!(LogFilterArgs::default().source, LogChannel::Both);
    }

    #[test]
    fn diagnose_args_mac_owns_launch_sim_attaches() {
        let args = plan_args("mac");
        // macOS: lldb owns the launch — `run`, and the executable after `--`.
        assert!(args.windows(2).any(|w| w == ["-o", "run"]));
        assert!(
            args.windows(2)
                .any(|w| w == ["--", "/tmp/MyApp.app/Contents/MacOS/MyApp"])
        );
        assert!(!args.iter().any(|a| a == "-p"));

        let args = plan_args("sim");
        // Simulator: attach to the suspended pid — `-p 4242`, and `continue`.
        assert!(args.windows(2).any(|w| w == ["-p", "4242"]));
        assert!(args.windows(2).any(|w| w == ["-o", "continue"]));
        assert!(!args.iter().any(|a| a == "run"));
        assert!(!args.iter().any(|a| a == "--"));
    }

    fn plan_args(kind: &str) -> Vec<String> {
        let target = if kind == "mac" {
            LldbTarget::Mac {
                exe: "/tmp/MyApp.app/Contents/MacOS/MyApp",
                args: &[],
            }
        } else {
            LldbTarget::AttachPid(4242)
        };
        diagnose_lldb_args(&target)
    }

    #[test]
    fn durations_take_a_unit_or_default_to_seconds() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration(" 45 ").unwrap(), Duration::from_secs(45));
        for bad in ["", "0s", "s", "-5s", "2d", "1.5m", "abc"] {
            assert!(parse_duration(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn until_matches_once_and_records_the_sighting() {
        let hit = Arc::new(AtomicBool::new(false));
        let watch = UntilWatch {
            text: "ready to serve".to_string(),
            hit: Arc::clone(&hit),
            stream_pid: None,
        };
        assert!(!watch.sees("12:00:00.000 I [net] starting up"));
        assert!(!hit.load(Ordering::Relaxed));
        assert!(watch.sees("12:00:01.000 I [net] ready to serve on :8080"));
        assert!(hit.load(Ordering::Relaxed));
        // Already satisfied: a later line must not re-fire the stop.
        assert!(!watch.sees("12:00:02.000 I [net] ready to serve again"));
    }

    #[test]
    fn captured_tail_ends_on_an_until_match_without_waiting_for_stop() {
        let dir = std::env::temp_dir().join(format!("sweetpad-until-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("captured.log");
        std::fs::write(&path, b"booting\nlistening on 8080\nstill going\n").unwrap();

        let hit = Arc::new(AtomicBool::new(false));
        let watch = UntilWatch {
            text: "listening on 8080".to_string(),
            hit: Arc::clone(&hit),
            stream_pid: None,
        };
        // `stop` stays false throughout: only the match may end this, and the
        // test would hang rather than pass if the tail ignored it.
        follow_console_file(&path, false, false, &AtomicBool::new(false), Some(&watch));
        assert!(hit.load(Ordering::Relaxed));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_deadline_stands_down_when_the_stream_ends_first() {
        let stop = Arc::new(AtomicBool::new(false));
        {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                stop.store(true, Ordering::Relaxed);
            });
        }
        // An hour-long deadline must still return promptly, and report that it
        // did not expire — otherwise it would signal a pid it no longer owns.
        assert!(!expire(Duration::from_secs(3600), &stop));
    }

    #[test]
    fn batch_args_forward_cmds_and_on_crash_verbatim() {
        let args = batch_lldb_args(
            &LldbTarget::AttachPid(7),
            &["b main".to_string(), "run".to_string()],
            &["bt".to_string()],
        );
        assert_eq!(&args[..3], &["-b", "-p", "7"]);
        assert!(args.windows(2).any(|w| w == ["-o", "b main"]));
        assert!(args.windows(2).any(|w| w == ["-o", "run"]));
        assert!(args.windows(2).any(|w| w == ["-k", "bt"]));
    }

    #[test]
    fn parse_objc_exception_transcript() {
        // Captured verbatim from `lldb -b -Q` against a raising binary.
        let t = "Process 67090 stopped\n\
* thread #1, queue = 'com.apple.main-thread', stop reason = hit Objective-C exception\n\
@@SWEETPAD_EXC@@\nMyExc\n\n@@SWEETPAD_REASON@@\nboom 42\n\n@@SWEETPAD_BT@@\n\
* thread #1, stop reason = hit Objective-C exception\n  \
* frame #0: 0x0001 libobjc.A.dylib`objc_exception_throw\n    \
frame #1: 0x0002 CoreFoundation`+[NSException raise:format:] + 128\n\
@@SWEETPAD_END@@\nProcess 67090 exited with status = 9 (0x00000009) killed\n";
        let o = parse_diagnose(t);
        assert!(o.stopped());
        assert_eq!(o.stop_reason.as_deref(), Some("hit Objective-C exception"));
        assert_eq!(o.exception_name.as_deref(), Some("MyExc"));
        assert_eq!(o.exception_reason.as_deref(), Some("boom 42"));
        assert_eq!(o.signal, None);
        assert_eq!(o.exit_status, None); // killed by us, not a real exit
        assert_eq!(o.backtrace.len(), 2);
        assert!(o.backtrace[0].contains("objc_exception_throw"));
    }

    #[test]
    fn parse_clean_exit_transcript() {
        // No stop reason; the `po` sections carry lldb errors we must drop.
        let t = "Process 67104 launched: '/tmp/okbin' (arm64)\n\
Process 67104 exited with status = 0 (0x00000000)\n\
@@SWEETPAD_EXC@@\n\
error: unable to evaluate expression while the process is exited\n\
@@SWEETPAD_REASON@@\n@@SWEETPAD_BT@@\n@@SWEETPAD_END@@\n";
        let o = parse_diagnose(t);
        assert!(!o.stopped());
        assert_eq!(o.exit_status, Some(0));
        assert_eq!(o.exception_name, None);
        assert_eq!(o.exception_reason, None);
        assert!(o.backtrace.is_empty());
    }

    #[test]
    fn parse_signal_crash_transcript() {
        // A plain signal crash (not an ObjC throw): signal set, no exception.
        let t = "Process 30835 stopped\n\
* thread #1, queue = 'com.apple.main-thread', stop reason = signal SIGABRT\n\
@@SWEETPAD_EXC@@\nerror: no Objective-C exception\n\
@@SWEETPAD_REASON@@\n@@SWEETPAD_BT@@\n  \
* frame #0: 0x00 libsystem_kernel.dylib`__pthread_kill + 8\n\
@@SWEETPAD_END@@\n";
        let o = parse_diagnose(t);
        assert!(o.stopped());
        assert_eq!(o.signal.as_deref(), Some("SIGABRT"));
        assert_eq!(o.exception_name, None); // not an ObjC exception → no $arg1
        assert_eq!(o.backtrace.len(), 1);
    }
}
