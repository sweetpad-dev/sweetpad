//! What `sweetpad dap` needs from `run` (CLI_DESIGN §9u): the editor's launch
//! configuration resolved to the same [`RunPlan`], the app built, installed
//! and started suspended, and the request that reaches it through lldb-dap.
//! It is a child of `app` so the adapter resolves and launches through `run`'s
//! own code rather than a copy of it.
//!
//! Each destination reaches lldb-dap its own way. A simulator app starts
//! under `simctl launch --wait-for-debugger` and lldb-dap attaches to its pid.
//! A macOS or Mac Catalyst app is launched by lldb-dap itself. A physical
//! device's app starts under `devicectl … --start-stopped` and lldb attaches
//! through its `device` commands. An iOS app running on the Mac ("Designed for
//! iPad") starts through LaunchServices, so lldb-dap waits for it by name.

use std::path::PathBuf;
use std::process::Child;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use sweetpad_lib::destination::DestinationSpec;

use super::{
    LaunchArgs, LogStream, RunOpts, RunPlan, Target, app_dir_name, default_filter, install_built,
    launch_sim_console, macwin, plan, print_summary, process_name, signal_group, start_logs,
};
use crate::cli::inject::recompiler::Mode;
use crate::cli::output::Output;
use crate::cli::xcodebuild::AppBundle;
use crate::cli::{
    CliError, CliResult, Context, ErrorKind, Targeting, buildlog, devicectl, oslog, process,
    resolve, simctl, xcodebuild,
};

/// The editor's launch or attach configuration. Every field is optional:
/// what's missing falls back to the project's configured and remembered
/// scheme and destination, as it does for `run`.
#[derive(Debug, Default)]
pub(crate) struct LaunchConfig {
    /// The directory to resolve the project from, in place of the adapter's
    /// own working directory.
    pub cwd: Option<PathBuf>,
    pub workspace: Option<PathBuf>,
    pub project: Option<PathBuf>,
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    /// What `--on` takes (`booted`, `mac`, a simulator or device name, a
    /// UDID), or a raw `-destination` specifier when it holds an `=`, which
    /// is how Mac Catalyst and Designed for iPad are named.
    pub destination: Option<String>,
    /// Arguments for the app process.
    pub args: Vec<String>,
    /// `KEY=VALUE` environment for the app process.
    pub env: Vec<String>,
    /// Extra `xcodebuild` arguments, as `run` takes after `--`.
    pub xcodebuild_args: Vec<String>,
    /// lldb-dap's own launch and attach fields (`sourceMap`, `initCommands`,
    /// …), passed through.
    pub lldb: Map<String, Value>,
    /// Attach only: the process to attach to.
    pub pid: Option<u32>,
}

impl LaunchConfig {
    /// Read the configuration from a `launch` or `attach` request's arguments.
    pub(crate) fn from_arguments(arguments: &Value) -> Result<Self, CliError> {
        let empty = Map::new();
        let fields = arguments.as_object().unwrap_or(&empty);
        Ok(LaunchConfig {
            cwd: string(fields, "cwd")?.map(PathBuf::from),
            workspace: string(fields, "workspace")?.map(PathBuf::from),
            project: string(fields, "project")?.map(PathBuf::from),
            scheme: string(fields, "scheme")?,
            configuration: string(fields, "configuration")?,
            destination: string(fields, "destination")?,
            args: strings(fields, "args")?,
            env: env(fields)?,
            xcodebuild_args: strings(fields, "xcodebuildArgs")?,
            lldb: match fields.get("lldb") {
                None | Some(Value::Null) => Map::new(),
                Some(Value::Object(lldb)) => lldb.clone(),
                Some(_) => return Err(field_error("'lldb' takes an object of lldb-dap fields")),
            },
            pid: match fields.get("pid") {
                None | Some(Value::Null) => None,
                Some(value) => Some(pid(value)?),
            },
        })
    }
}

fn field_error(message: impl Into<String>) -> CliError {
    CliError::new(message).kind(ErrorKind::Usage)
}

/// A string field; an empty string counts as unset, as an empty `SWEETPAD_*`
/// variable does.
fn string(fields: &Map<String, Value>, key: &str) -> Result<Option<String>, CliError> {
    match fields.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(field_error(format!("'{key}' takes a string"))),
    }
}

fn strings(fields: &Map<String, Value>, key: &str) -> Result<Vec<String>, CliError> {
    match fields.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| field_error(format!("'{key}' takes a list of strings")))
            })
            .collect(),
        Some(_) => Err(field_error(format!("'{key}' takes a list of strings"))),
    }
}

/// `env` as an object (`{"KEY": "value"}`) or a list of `KEY=VALUE` strings,
/// the two shapes editors' debug configurations use.
fn env(fields: &Map<String, Value>) -> Result<Vec<String>, CliError> {
    match fields.get("env") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(vars)) => vars
            .iter()
            .map(|(key, value)| match value {
                Value::String(s) => Ok(format!("{key}={s}")),
                Value::Number(_) | Value::Bool(_) => Ok(format!("{key}={value}")),
                _ => Err(field_error(format!("'env.{key}' takes a string"))),
            })
            .collect(),
        Some(Value::Array(_)) => {
            let pairs = strings(fields, "env")?;
            match pairs.iter().find(|pair| !pair.contains('=')) {
                Some(bad) => Err(field_error(format!("'env' takes KEY=VALUE (got {bad:?})"))),
                None => Ok(pairs),
            }
        }
        Some(_) => Err(field_error("'env' takes an object or a list of KEY=VALUE")),
    }
}

/// A pid as a number, or as the string a process picker hands over.
fn pid(value: &Value) -> Result<u32, CliError> {
    let parsed = match value {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    };
    parsed
        .filter(|pid| *pid > 0)
        .ok_or_else(|| field_error(format!("'pid' takes a process id (got {value})")))
}

/// Where the session's output goes: the editor's debug console.
pub(crate) trait Sink: Sync {
    /// A line of build or launch output.
    fn console(&self, text: &str);
    /// A build error or warning, rendered as the build log shows it, with the
    /// `file:line:col` the editor links it to.
    fn diagnostic(&self, severity: &str, location: Option<&str>, text: &str);
    /// What the launch is doing now, for the editor's progress indicator.
    fn step(&self, message: &str);
}

/// Cancels a launch in flight, when the editor disconnects or cancels the
/// build's progress: the running `xcodebuild` is interrupted, and the steps
/// after it don't start.
#[derive(Debug, Default)]
pub(crate) struct Cancel {
    cancelled: AtomicBool,
    /// The running build's process group, or 0.
    build: AtomicU32,
}

impl Cancel {
    pub(crate) fn fire(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let pgid = self.build.load(Ordering::SeqCst);
        if pgid != 0 {
            signal_group(pgid, libc::SIGINT);
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn check(&self) -> CliResult {
        if self.is_cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }

    /// Record the build's process group; one cancelled before the build
    /// started is interrupted right away.
    fn arm(&self, pgid: u32) {
        self.build.store(pgid, Ordering::SeqCst);
        if self.is_cancelled() {
            signal_group(pgid, libc::SIGINT);
        }
    }

    fn disarm(&self) {
        self.build.store(0, Ordering::SeqCst);
    }
}

fn cancelled() -> CliError {
    CliError::new("the debug session ended before the app launched")
}

/// What the session sends lldb-dap in place of the editor's request, and the
/// app it is about to debug.
pub(crate) struct Prepared {
    /// `attach` or `launch`: what lldb-dap is asked to do.
    pub command: &'static str,
    pub arguments: Value,
    pub app: AppSession,
    /// Starts the app once lldb-dap is waiting for it, for the one route where
    /// the debugger can neither launch the app nor find it already suspended.
    pub start: Option<Box<dyn FnOnce() -> CliResult + Send>>,
}

/// The app a debug session runs, and the streams that follow it.
pub(crate) struct AppSession {
    kind: AppKind,
    /// `simctl launch --console-pty`, whose output is the app's stdout and
    /// stderr.
    console: Option<Child>,
    console_slot: Option<usize>,
    logs: Option<LogStream>,
}

enum AppKind {
    Simulator {
        udid: String,
        bundle_id: String,
    },
    Device {
        udid: String,
        app_dir: String,
    },
    /// lldb-dap launched it, or waited for it, and stops it itself.
    Mac,
}

impl AppSession {
    /// Stop following the app, and stop the app too when `terminate`. lldb
    /// has already been told to kill or detach from it; this makes sure a
    /// simulator or device app that outlived the debugger goes as well.
    pub(crate) fn end(mut self, terminate: bool) {
        crate::cli::signals::unregister_child(self.console_slot.take());
        if terminate {
            match &self.kind {
                AppKind::Simulator { udid, bundle_id } => {
                    let _ = simctl::terminate(udid, bundle_id);
                }
                AppKind::Device { udid, app_dir } => {
                    let _ = devicectl::terminate(udid, app_dir);
                }
                AppKind::Mac => {}
            }
        }
        // SIGKILL, not a forwarded SIGTERM: simctl passes a catchable signal
        // on to the app, which must keep running after a detach.
        if let Some(mut console) = self.console.take() {
            let _ = console.kill();
            let _ = console.wait();
        }
        drop(self.logs.take());
    }
}

/// Whether a line the app wrote is about how it was launched rather than
/// about the app, which the session leaves out as `run` does: the simulator's
/// boot noise, AppKit's note on the `-ApplePersistenceIgnoreState` sweetpad
/// adds, and the lines Xcode's debug-dylib stub writes whenever a debugger
/// starts the app.
pub(crate) fn is_launch_noise(line: &str) -> bool {
    super::is_boot_noise(line)
        || super::is_persistence_note(line)
        || line.contains("] [PreviewsAgentExecutorLibrary] ")
}

/// Resolve the plan for `config`, as `run` does with the same flags. A debug
/// session has no terminal to prompt in, so a scheme or destination nobody
/// named fails with the ways to name one.
fn settle(
    ctx: &mut Context,
    config: &LaunchConfig,
    launch: &LaunchArgs,
) -> Result<RunPlan, CliError> {
    if let Some(dir) = &config.cwd {
        std::env::set_current_dir(dir).map_err(|e| {
            field_error(format!(
                "'cwd': cannot change directory to {}: {e}",
                dir.display()
            ))
        })?;
    }
    let raw = config.destination.as_ref().filter(|d| d.contains('='));
    ctx.targeting = Targeting {
        workspace: config.workspace.clone(),
        project: config.project.clone(),
        scheme: config.scheme.clone(),
        configuration: config.configuration.clone(),
        destination: raw.cloned(),
        on: config.destination.clone().filter(|_| raw.is_none()),
        sdk: None,
    };
    let passthrough = ctx.xcodebuild_args(xcodebuild::Action::Build, &config.xcodebuild_args)?;
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
        launch,
        passthrough: &passthrough,
    };
    plan(ctx, &opts).map_err(|e| in_debug_terms(ctx, e))
}

/// Restate a "nothing chosen and no terminal to ask in" error for a debug
/// session, whose fixes are a launch-configuration field or one terminal run
/// rather than a flag.
fn in_debug_terms(ctx: &Context, err: CliError) -> CliError {
    let text = err.to_string();
    if err.error_kind() != ErrorKind::TargetResolution
        || !text.contains("the terminal is not interactive")
    {
        return err;
    }
    let what = text
        .strip_prefix("no ")
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("destination")
        .to_string();
    let choices = (what == "scheme")
        .then(|| resolve::container_silently(ctx))
        .flatten()
        .and_then(|container| resolve::schemes(&container).ok())
        .map(|schemes| format!(" (one of {})", schemes.join(", ")))
        .unwrap_or_default();
    CliError::new(format!(
        "no {what} to debug: set \"{what}\" in the launch configuration{choices}, or run \
         'sweetpad run' once in a terminal to pick one, which is then remembered"
    ))
    .kind(ErrorKind::TargetResolution)
}

/// Build the app and start it for lldb-dap, per the destination.
pub(crate) fn prepare_launch(
    ctx: &mut Context,
    config: &LaunchConfig,
    sink: &dyn Sink,
    cancel: &Cancel,
) -> Result<Prepared, CliError> {
    let launch = LaunchArgs {
        args: config.args.clone(),
        env: config.env.clone(),
        wait_for_debugger: true,
        restore_state: false,
    };
    let plan = settle(ctx, config, &launch)?;
    print_summary(ctx, &plan);
    let prepared = match &plan.target {
        Target::Simulator(udid) => launch_on_simulator(ctx, &plan, config, udid, sink, cancel),
        Target::Device(udid) => launch_on_device(ctx, &plan, config, udid, sink, cancel),
        Target::Mac if designed_for_ipad(&plan) => {
            launch_designed_for_ipad(ctx, &plan, config, sink, cancel)
        }
        Target::Mac => launch_on_mac(ctx, &plan, config, sink, cancel),
        Target::SpmRun(_) => Err(CliError::new(
            "a Swift package executable has no app to debug through 'sweetpad dap'; point \
             lldb-dap's own 'launch' at the built binary instead",
        )),
    }?;
    super::record_last_launched(ctx, &plan);
    Ok(prepared)
}

/// Whether the plan runs an iOS app on the Mac ("Designed for iPad"), which
/// only LaunchServices can start.
fn designed_for_ipad(plan: &RunPlan) -> bool {
    DestinationSpec::parse(&plan.destination)
        .variant
        .is_some_and(|variant| variant.starts_with("Designed for"))
}

/// The live log filter a debug session streams at: info and above, as `run`
/// shows by default.
fn log_filter(out: &Output) -> Arc<AtomicU8> {
    Arc::new(AtomicU8::new(default_filter(out).threshold()))
}

fn launch_on_simulator(
    ctx: &Context,
    plan: &RunPlan,
    config: &LaunchConfig,
    udid: &str,
    sink: &dyn Sink,
    cancel: &Cancel,
) -> Result<Prepared, CliError> {
    let _ = simctl::open_app();
    let app = build_and_install(ctx, plan, sink, cancel)?;
    cancel.check()?;
    sink.step("Launching");
    let filter = log_filter(&ctx.out);
    let (mut console, pid) = launch_sim_console(ctx, plan, &app, udid, &filter)?;
    let pid = pid.or_else(|| {
        poll(Duration::from_secs(5), || {
            simctl::app_pids(udid, &app_dir_name(&app.path), process_name(&app))
                .into_iter()
                .next()
        })
    });
    let Some(pid) = pid else {
        let _ = console.kill();
        let _ = console.wait();
        return Err(CliError::new(format!(
            "{} launched but its process never showed up on the simulator",
            app.bundle_id
        )));
    };
    let console_slot = crate::cli::signals::register_child(console.id());
    let logs = start_logs(ctx, plan, &filter);
    Ok(Prepared {
        command: "attach",
        arguments: lldb_arguments(
            config,
            json!({ "pid": pid, "program": app.executable }),
            &Map::new(),
        ),
        app: AppSession {
            kind: AppKind::Simulator {
                udid: udid.to_string(),
                bundle_id: app.bundle_id,
            },
            console: Some(console),
            console_slot,
            logs,
        },
        start: None,
    })
}

fn launch_on_mac(
    ctx: &Context,
    plan: &RunPlan,
    config: &LaunchConfig,
    sink: &dyn Sink,
    cancel: &Cancel,
) -> Result<Prepared, CliError> {
    let app = build_and_install(ctx, plan, sink, cancel)?;
    cancel.check()?;
    let env: Vec<String> = plan
        .launch
        .env_pairs("")?
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    // lldb-dap owns this process and relays its stdout, so only os_log is
    // streamed here.
    let logs = start_logs(ctx, plan, &log_filter(&ctx.out));
    Ok(Prepared {
        command: "launch",
        arguments: lldb_arguments(
            config,
            json!({ "program": app.executable, "args": plan.launch.args, "env": env }),
            &Map::new(),
        ),
        app: AppSession {
            kind: AppKind::Mac,
            console: None,
            console_slot: None,
            logs,
        },
        start: None,
    })
}

/// An iOS app on the Mac runs only when LaunchServices starts it, so neither
/// lldb-dap's launch nor a suspended spawn reaches it. lldb-dap waits for its
/// executable by name, and the session opens the app once lldb-dap has its
/// configuration and is waiting. The app's stdout and stderr go to a file the
/// session follows.
fn launch_designed_for_ipad(
    ctx: &Context,
    plan: &RunPlan,
    config: &LaunchConfig,
    sink: &dyn Sink,
    cancel: &Cancel,
) -> Result<Prepared, CliError> {
    let app = build_and_install(ctx, plan, sink, cancel)?;
    cancel.check()?;
    // An instance already running would satisfy nobody's wait: lldb waits for
    // a new process.
    if let Ok(pids) = macwin::pids_for_executable(&app.executable) {
        for pid in pids {
            process::terminate(pid.unsigned_abs());
        }
    }
    let output = std::env::temp_dir().join(format!(
        "sweetpad-dap-{}-{}.log",
        process_name(&app),
        std::process::id()
    ));
    std::fs::write(&output, b"")
        .map_err(|e| CliError::new(format!("creating {}: {e}", output.display())))?;
    let logs = start_logs(ctx, plan, &log_filter(&ctx.out));
    let mut open: Vec<String> = vec![
        "-n".into(),
        "--stdout".into(),
        output.display().to_string(),
        "--stderr".into(),
        output.display().to_string(),
    ];
    for (key, value) in plan.launch.env_pairs("")? {
        open.push("--env".into());
        open.push(format!("{key}={value}"));
    }
    open.push(app.path.display().to_string());
    if !plan.launch.args.is_empty() {
        open.push("--args".into());
        open.extend(plan.launch.args.iter().cloned());
    }
    let start = move || {
        let args: Vec<&str> = open.iter().map(String::as_str).collect();
        process::capture("open", &args, None).map(|_| follow_file(output))
    };
    Ok(Prepared {
        command: "attach",
        arguments: lldb_arguments(
            config,
            json!({ "program": app.executable, "waitFor": true }),
            &Map::new(),
        ),
        app: AppSession {
            kind: AppKind::Mac,
            console: None,
            console_slot: None,
            logs,
        },
        start: Some(Box::new(start)),
    })
}

/// Print what's appended to `path` as app output, until the process exits.
#[allow(clippy::print_stdout)] // stdout is the session's app-output capture
fn follow_file(path: PathBuf) {
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        let Ok(file) = std::fs::File::open(&path) else {
            return;
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            match reader.read_line(&mut line) {
                Ok(0) => std::thread::sleep(Duration::from_millis(200)),
                Ok(_) if line.ends_with('\n') => {
                    let now = oslog::now_clock();
                    let text = line.trim_end_matches('\n');
                    println!(
                        "{}",
                        oslog::render_console_line(Some(&now), text, false).text
                    );
                    line.clear();
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
    });
}

fn launch_on_device(
    ctx: &Context,
    plan: &RunPlan,
    config: &LaunchConfig,
    udid: &str,
    sink: &dyn Sink,
    cancel: &Cancel,
) -> Result<Prepared, CliError> {
    if let Some(device) = refuse_pre_devicectl(udid)?
        && let Some(note) = missing_device_symbols(&device)
    {
        sink.console(&note);
    }
    let app = build_and_install(ctx, plan, sink, cancel)?;
    cancel.check()?;
    sink.step("Launching");
    let env = plan.launch.env_pairs("DEVICECTL_CHILD_")?;
    ctx.out.step("Launching app on device", || {
        devicectl::launch(udid, &app.bundle_id, &plan.launch.args, &env, true)
    })?;
    let app_dir = app_dir_name(&app.path);
    let process = poll(Duration::from_secs(15), || {
        devicectl::app_processes(udid, &app_dir)
            .ok()?
            .into_iter()
            .find(|p| p.main)
    })
    .ok_or_else(|| {
        CliError::new(format!(
            "{} launched but its process never showed up on the device",
            app.bundle_id
        ))
    })?;
    let logs = start_logs(ctx, plan, &log_filter(&ctx.out));
    Ok(Prepared {
        command: "attach",
        arguments: device_attach(config, udid, &app, &process),
        app: AppSession {
            kind: AppKind::Device {
                udid: udid.to_string(),
                app_dir,
            },
            console: None,
            console_slot: None,
            logs,
        },
        start: None,
    })
}

/// devicectl drives only iOS 17 and later. An older device fails its install
/// with an error that doesn't say so, so the version is checked first. The
/// device comes back when devicectl lists it.
fn refuse_pre_devicectl(udid: &str) -> Result<Option<devicectl::Device>, CliError> {
    let Some(device) = devicectl::list()
        .ok()
        .and_then(|devices| devices.into_iter().find(|d| d.udid == udid))
    else {
        return Ok(None);
    };
    let major = device
        .os_version
        .split('.')
        .next()
        .and_then(|m| m.parse::<u32>().ok());
    if device.platform_label() == "iOS" && major.is_some_and(|m| m < 17) {
        return Err(CliError::new(format!(
            "{} runs iOS {}, and 'sweetpad dap' reaches devices through devicectl, which needs \
             iOS 17 or later; debug it from the VS Code extension, which keeps a CodeLLDB route \
             for older devices, or from Xcode",
            device.name, device.os_version
        ))
        .kind(ErrorKind::TargetResolution));
    }
    Ok(Some(device))
}

/// A note for a device whose system libraries this Mac has no copy of.
/// Without one, lldb reads each library out of the device's memory as the app
/// loads it, which over a network connection takes minutes. Xcode makes the
/// copy, under `iOS DeviceSupport`, the first time it runs an app on the
/// device; devicectl has no command for it.
fn missing_device_symbols(device: &devicectl::Device) -> Option<String> {
    if device.platform_label() != "iOS" || device.os_version.is_empty() {
        return None;
    }
    let dir = std::env::var_os("HOME")
        .map(PathBuf::from)?
        .join("Library/Developer/Xcode/iOS DeviceSupport");
    let version = format!("{} (", device.os_version);
    let present = std::fs::read_dir(&dir).ok().is_some_and(|entries| {
        entries.flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with(&version) || name.contains(&format!(" {version}"))
        })
    });
    (!present).then(|| {
        format!(
            "this Mac has no copy of iOS {}'s system libraries yet, so the debugger reads them \
             from {} as the app loads them, which can take minutes. Run any app on the device \
             from Xcode once to have Xcode copy them",
            device.os_version, device.name
        )
    })
}

/// The attach request for a device process: lldb selects the remote-iOS
/// platform, maps its local copy of the app onto the one on the device, and
/// attaches through CoreDevice, the route the VS Code extension takes.
fn device_attach(
    config: &LaunchConfig,
    udid: &str,
    app: &AppBundle,
    process: &devicectl::AppProcess,
) -> Value {
    let ours = Map::from_iter([
        (
            "initCommands".to_string(),
            json!(["platform select remote-ios"]),
        ),
        // Attaching stops the app with a SIGSTOP that lldb-dap reports only
        // after it resumes the app, as a stop nobody asked for.
        (
            "postRunCommands".to_string(),
            json!(["process handle SIGSTOP --pass false --stop false --notify false"]),
        ),
        (
            "preRunCommands".to_string(),
            json!([format!(
                "script lldb.target.module[0].SetPlatformFileSpec(lldb.SBFileSpec({}))",
                python_string(&process.app_path)
            )]),
        ),
    ]);
    lldb_arguments(
        config,
        json!({
            "program": app.path,
            "attachCommands": [
                handled(&format!("device select {udid}")),
                handled(&format!("device process attach --pid {}", process.pid)),
            ],
        }),
        &ours,
    )
}

/// `command` run through the debugger from lldb's Python, which is how the
/// extension's CodeLLDB route runs the `device` commands.
fn handled(command: &str) -> String {
    format!(
        "script lldb.debugger.HandleCommand({})",
        python_string(command)
    )
}

/// A single-quoted Python string literal for lldb's `script` command.
fn python_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Reach an app that is already running, with no build: by `pid`, or by
/// finding the process of the app the plan builds.
pub(crate) fn prepare_attach(
    ctx: &mut Context,
    config: &LaunchConfig,
) -> Result<Prepared, CliError> {
    // A pid alone names a process on this Mac, which covers simulator apps.
    if let Some(pid) = config.pid
        && config.destination.is_none()
        && config.scheme.is_none()
    {
        return Ok(Prepared {
            command: "attach",
            arguments: lldb_arguments(config, json!({ "pid": pid }), &Map::new()),
            app: AppSession {
                kind: AppKind::Mac,
                console: None,
                console_slot: None,
                logs: None,
            },
            start: None,
        });
    }
    let plan = settle(ctx, config, &LaunchArgs::default())?;
    let app = plan
        .app_bundle()
        .map_err(|e| e.context("finding the app to attach to (launch it once so it is built)"))?;
    let filter = log_filter(&ctx.out);
    let not_running = || {
        CliError::new(format!("{} isn't running; launch it first", app.bundle_id))
            .kind(ErrorKind::TargetResolution)
    };
    let (arguments, kind) = match &plan.target {
        Target::Simulator(udid) => {
            let pid = config
                .pid
                .or_else(|| {
                    simctl::app_pids(udid, &app_dir_name(&app.path), process_name(&app))
                        .into_iter()
                        .next()
                })
                .ok_or_else(not_running)?;
            (
                lldb_arguments(
                    config,
                    json!({ "pid": pid, "program": app.executable }),
                    &Map::new(),
                ),
                AppKind::Simulator {
                    udid: udid.clone(),
                    bundle_id: app.bundle_id.clone(),
                },
            )
        }
        Target::Mac => {
            let pid = config
                .pid
                .or_else(|| {
                    macwin::pids_for_executable(&app.executable)
                        .ok()?
                        .first()
                        .map(|pid| pid.unsigned_abs())
                })
                .ok_or_else(not_running)?;
            (
                lldb_arguments(
                    config,
                    json!({ "pid": pid, "program": app.executable }),
                    &Map::new(),
                ),
                AppKind::Mac,
            )
        }
        Target::Device(udid) => {
            let app_dir = app_dir_name(&app.path);
            let processes = devicectl::app_processes(udid, &app_dir)?;
            let process = processes
                .iter()
                .find(|p| config.pid.map_or(p.main, |pid| i64::from(pid) == p.pid))
                .ok_or_else(not_running)?;
            (
                device_attach(config, udid, &app, process),
                AppKind::Device {
                    udid: udid.clone(),
                    app_dir,
                },
            )
        }
        Target::SpmRun(_) => {
            return Err(CliError::new(
                "a Swift package executable has no app to attach to; pass 'pid' instead",
            ));
        }
    };
    let logs = start_logs(ctx, &plan, &filter);
    Ok(Prepared {
        command: "attach",
        arguments,
        app: AppSession {
            kind,
            console: None,
            console_slot: None,
            logs,
        },
        start: None,
    })
}

/// lldb-dap's arguments: the editor's `lldb` fields, then the adapter's own
/// on top. Command lists in `ours` run ahead of the editor's, which extend
/// them rather than replace them.
fn lldb_arguments(config: &LaunchConfig, fields: Value, ours: &Map<String, Value>) -> Value {
    let mut arguments = config.lldb.clone();
    for (key, value) in ours {
        let mut list = value.as_array().cloned().unwrap_or_default();
        if let Some(Value::Array(theirs)) = arguments.get(key) {
            list.extend(theirs.iter().cloned());
        }
        arguments.insert(key.clone(), Value::Array(list));
    }
    if let Value::Object(fields) = fields {
        arguments.extend(fields);
    }
    Value::Object(arguments)
}

/// Retry `probe` until it finds something or `limit` runs out.
fn poll<T>(limit: Duration, mut probe: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(found) = probe() {
            return Some(found);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Build and install as `run` does, with the build's output going to the
/// editor and the build interruptible by `cancel`.
fn build_and_install(
    ctx: &Context,
    plan: &RunPlan,
    sink: &dyn Sink,
    cancel: &Cancel,
) -> Result<AppBundle, CliError> {
    install_built(plan, &ctx.out, || {
        build(plan, &ctx.out, sink, cancel)?;
        cancel.check()?;
        sink.step("Installing");
        Ok(())
    })
}

/// Run the build, streaming the short build log to `sink` with each error
/// and warning carrying its location. Mirrors the run session's build, with
/// `cancel` in place of the Ctrl-C watcher.
fn build(plan: &RunPlan, out: &Output, sink: &dyn Sink, cancel: &Cancel) -> CliResult {
    cancel.check()?;
    sink.step("Building");
    let build_plan = plan.build_plan();
    build_plan.prepare_result_bundle();
    let (parts, cwd) = build_plan.command();
    let args: Vec<&str> = parts.iter().map(String::as_str).collect();
    let (mut child, reader, leftovers) =
        process::spawn_piped_group("xcodebuild", &args, cwd.as_deref())?;
    let pid = child.id();
    cancel.arm(pid);
    crate::cli::signals::set_build_pgid(pid);
    let mut progress = buildlog::BuildProgress::start(out, "Building");
    let mut diagnostics: Vec<serde_json::Value> = Vec::new();
    let mut blocker = buildlog::BlockerWatch::default();
    let mut parser = buildlog::LogParser::default();
    let mut show = |parsed: &buildlog::Parsed| {
        if matches!(parsed.event, buildlog::Event::Diagnostic { .. })
            && let Some(json) = buildlog::event_json(&parsed.event)
        {
            diagnostics.push(json);
        }
        let Some(rendered) = progress.parsed(parsed) else {
            return;
        };
        match &parsed.event {
            buildlog::Event::Diagnostic { kind, location, .. } => {
                let severity = match kind {
                    buildlog::DiagKind::Error => "error",
                    buildlog::DiagKind::Warning => "warning",
                    buildlog::DiagKind::Note => "note",
                };
                sink.diagnostic(severity, location.as_deref(), &rendered);
            }
            buildlog::Event::Compile { name } => {
                sink.console(&rendered);
                sink.step(&format!("Compiling {name}"));
            }
            _ => sink.console(&rendered),
        }
    };
    process::read_lines_lossy(reader, &mut |line: &str| {
        blocker.line(line);
        parser.push(line).iter().for_each(&mut show);
    });
    parser.finish().iter().for_each(&mut show);
    crate::cli::signals::clear_build_pgid();
    cancel.disarm();
    let status = child.wait();
    drop(leftovers);
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let ok = matches!(&status, Ok(s) if s.success());
    if !ok && let Some(banner) = progress.close_failed(buildlog::ResultKind::BuildFailed) {
        sink.console(&banner);
    }
    xcodebuild::record_build_diagnostics(&plan.resolved.container, ok, &diagnostics);
    match status {
        Ok(s) if s.success() => Ok(()),
        // The editor shows this message on its own, away from the build log,
        // so it repeats the first errors.
        Ok(_) => {
            Err(
                xcodebuild::build_failure(&parts, diagnostics, blocker.hint(), true, true, "")
                    .context("building the app"),
            )
        }
        Err(e) => Err(CliError::new(format!("failed to wait for xcodebuild: {e}"))
            .context("building the app")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_field_of_a_launch_configuration() {
        let config = LaunchConfig::from_arguments(&json!({
            "type": "sweetpad",
            "request": "launch",
            "scheme": "MyApp",
            "configuration": "Debug",
            "destination": "iPhone 18 Pro",
            "args": ["-Flag", "YES"],
            "env": { "A": "1", "B": 2 },
            "xcodebuildArgs": ["-quiet"],
            "lldb": { "sourceMap": [] },
        }))
        .unwrap();
        assert_eq!(config.scheme.as_deref(), Some("MyApp"));
        assert_eq!(config.destination.as_deref(), Some("iPhone 18 Pro"));
        assert_eq!(config.args, ["-Flag", "YES"]);
        assert_eq!(config.env, ["A=1", "B=2"]);
        assert_eq!(config.xcodebuild_args, ["-quiet"]);
        assert!(config.lldb.contains_key("sourceMap"));
    }

    #[test]
    fn env_takes_a_list_of_pairs_and_refuses_a_bare_word() {
        let ok = LaunchConfig::from_arguments(&json!({ "env": ["A=1"] })).unwrap();
        assert_eq!(ok.env, ["A=1"]);
        let bad = LaunchConfig::from_arguments(&json!({ "env": ["A"] })).unwrap_err();
        assert!(bad.to_string().contains("KEY=VALUE"), "{bad}");
    }

    #[test]
    fn empty_strings_count_as_unset() {
        let config = LaunchConfig::from_arguments(&json!({ "scheme": "" })).unwrap();
        assert_eq!(config.scheme, None);
    }

    #[test]
    fn a_wrong_type_names_the_field() {
        let err = LaunchConfig::from_arguments(&json!({ "args": "-Flag" })).unwrap_err();
        assert!(err.to_string().contains("'args'"), "{err}");
    }

    #[test]
    fn pid_takes_a_number_or_a_pickers_string() {
        let number = LaunchConfig::from_arguments(&json!({ "pid": 42 })).unwrap();
        assert_eq!(number.pid, Some(42));
        let string = LaunchConfig::from_arguments(&json!({ "pid": "42" })).unwrap();
        assert_eq!(string.pid, Some(42));
        assert!(LaunchConfig::from_arguments(&json!({ "pid": 0 })).is_err());
    }

    #[test]
    fn our_commands_run_ahead_of_the_editors() {
        let config = LaunchConfig::from_arguments(&json!({
            "lldb": { "initCommands": ["settings set a b"], "sourceMap": [["/a", "/b"]] },
        }))
        .unwrap();
        let ours = Map::from_iter([(
            "initCommands".to_string(),
            json!(["platform select remote-ios"]),
        )]);
        let args = lldb_arguments(&config, json!({ "pid": 7 }), &ours);
        assert_eq!(
            args["initCommands"],
            json!(["platform select remote-ios", "settings set a b"])
        );
        assert_eq!(args["sourceMap"], json!([["/a", "/b"]]));
        assert_eq!(args["pid"], 7);
    }

    #[test]
    fn the_adapters_fields_win_over_the_editors() {
        let config = LaunchConfig::from_arguments(&json!({ "lldb": { "pid": 1 } })).unwrap();
        let args = lldb_arguments(&config, json!({ "pid": 7 }), &Map::new());
        assert_eq!(args["pid"], 7);
    }

    #[test]
    fn python_string_escapes_quotes_and_backslashes() {
        assert_eq!(python_string(r"/a/it's\x.app"), r"'/a/it\'s\\x.app'");
    }

    #[test]
    fn a_cancel_before_the_build_stops_the_next_step() {
        let cancel = Cancel::default();
        assert!(cancel.check().is_ok());
        cancel.fire();
        assert!(cancel.check().is_err());
    }
}
