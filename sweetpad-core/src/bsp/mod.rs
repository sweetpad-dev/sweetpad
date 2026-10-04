//! The Build Server Protocol server — see `DOCS.md` §8 (BSP server).
//!
//! Speaks BSP (JSON-RPC over stdio) to `sourcekit-lsp`, answering the questions
//! that drive editor intelligence: what targets exist, what files each contains,
//! and the compiler arguments for a file. The argv comes from the resolver/
//! generator core (`build_settings::resolve_compiler_arguments`), so it's derived
//! from the project, not parsed out of a build log.
//!
//! This is the walking-skeleton scope: the core requests, per-**target** argv
//! (⚠️ per-file later — see `DOCS.md` §8 (BSP server)), no `buildTarget/prepare` yet (v2).

mod control;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::app_locator::CommandLineSettings;
use crate::build_context::BuildContext;
use crate::build_settings::{self, BuildSettingsOptions};
use crate::framing::{read_message, write_message};
use crate::scratch::{ScratchDir, TmpdirLeftovers};
use crate::xcodebuild_args;
use control::{LogLevel, TelemetryServer};
use sweetpad_lib::{compiler_args, derived_data, project, scheme};

/// Write a `buildServer.json` so `sourcekit-lsp` discovers and launches this
/// server. Its `argv` is the current executable followed by
/// `serve_subcommand` — the caller's spelling of "run the server loop"
/// (`["bsp"]` for the standalone bsp-server binary, `["bsp", "serve"]` for the
/// sweetpad CLI) — plus the server flags, dropped into the workspace root (the
/// `.xcodeproj`'s parent, or `--output`).
pub fn write_config(args: &[String], serve_subcommand: &[&str]) -> Result<(), String> {
    let flags = parse_flags(args);
    // Accept either a `.xcodeproj` (`--project`) or a `.xcworkspace` (`--workspace`);
    // the BSP server resolves files against a workspace's member projects.
    let (root_flag, root) = flags
        .get("workspace")
        .map(|w| ("--workspace", w))
        .or_else(|| flags.get("project").map(|p| ("--project", p)))
        .ok_or(
            "config: --project <path.xcodeproj> or --workspace <path.xcworkspace> is required",
        )?;
    let root_abs = std::fs::canonicalize(root).map_err(|e| format!("{root_flag}: {e}"))?;
    // A project's embedded workspace is the project: the server keys it that
    // way, and `buildServer.json` belongs beside the `.xcodeproj`, not in it.
    let (root_flag, root_abs) = match sweetpad_lib::workspace::embedding_project(&root_abs) {
        Some(project) => ("--project", project.to_path_buf()),
        None => (root_flag, root_abs),
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    let mut server_argv = vec![exe.to_string_lossy().into_owned()];
    server_argv.extend(serve_subcommand.iter().map(|s| (*s).to_string()));
    server_argv.push(root_flag.into());
    server_argv.push(root_abs.to_string_lossy().into_owned());
    for (flag, key) in [
        ("--xcode", "xcode"),
        ("--derived-data-path", "derived-data-path"),
    ] {
        if let Some(v) = flags.get(key) {
            server_argv.push(flag.into());
            server_argv.push(v.clone());
        }
    }
    let config = json!({
        "name": "sweetpad",
        "version": env!("CARGO_PKG_VERSION"),
        "bspVersion": "2.2.0",
        "languages": LANGUAGE_IDS,
        "argv": server_argv,
    });

    let out = build_server_json_path(&root_abs, flags.get("output").map(Path::new));
    let body = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&out, format!("{body}\n"))
        .map_err(|e| format!("write {}: {e}", out.display()))?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

/// Where sourcekit-lsp finds the `buildServer.json` for `container`: the
/// explicit `output`, else beside the container. A project's embedded
/// workspace puts it beside the project, not inside the bundle.
#[must_use]
pub fn build_server_json_path(container: &Path, output: Option<&Path>) -> PathBuf {
    output.map_or_else(
        || {
            sweetpad_lib::workspace::normalize_stub_workspace(container)
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("buildServer.json")
        },
        Path::to_path_buf,
    )
}

/// Build settings the project's builds add above every project layer, from
/// the command line they run `xcodebuild` with: an `-xcconfig` overlay and
/// `KEY=VALUE` assignments, in order. The sweetpad CLI reads them from the
/// project's `sweetpad.toml`; a server started from `bsp.json` reads them
/// from that file's `buildArgs`, which the extension fills from
/// `sweetpad.build.args`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandLine {
    pub xcconfig: Option<PathBuf>,
    pub overrides: Vec<(String, String)>,
}

/// The settings a command line layers on every resolution. Its
/// `-derivedDataPath` is left out: the server fixes DerivedData at startup.
impl From<CommandLineSettings> for CommandLine {
    fn from(settings: CommandLineSettings) -> Self {
        Self {
            xcconfig: settings.xcconfig,
            overrides: settings.overrides,
        }
    }
}

/// Run the BSP server loop over stdin/stdout until EOF or `build/exit`.
pub fn run(args: &[String]) -> Result<(), String> {
    run_with(args, CommandLine::default())
}

/// [`run`], resolving every target's settings with `command_line` on top, so
/// the editor's compiler arguments and the `buildTarget/prepare` build agree
/// with the project's own builds.
pub fn run_with(args: &[String], command_line: CommandLine) -> Result<(), String> {
    let server = Arc::new(Server::resolve(args, command_line)?);
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    // Each write locks the output for one whole frame rather than holding the
    // lock across the loop, so the worker threads (change-watcher, prepare)
    // can interleave their messages between requests.
    let mut watching = false;

    // `buildTarget/prepare` runs `xcodebuild` (seconds-to-minutes), and
    // sourcekit-lsp blocks the requesting target's semantics until our response
    // arrives — so run it on a serialized worker and reply by id when the build
    // finishes, keeping the request loop responsive in the meantime.
    {
        let server = Arc::clone(&server);
        std::thread::spawn(move || {
            while let Some(job) = server.prepare_queue.take() {
                server.run_prepare(&job);
            }
        });
    }

    loop {
        let msg = match read_message(&mut reader) {
            Ok(Some(msg)) => msg,
            Ok(None) => break, // clean EOF
            Err(e) => {
                // The frame boundary is lost; log why before dying so the
                // failure is diagnosable instead of a silent exit.
                server.trace(&format!("fatal framing error: {e}"));
                server.prepare_queue.close();
                server.kill_prepare();
                server.shutdown_telemetry();
                return Err(e);
            }
        };
        let req = match serde_json::from_str::<Value>(&msg) {
            Ok(req) => req,
            Err(e) => {
                // JSON-RPC: a frame that isn't valid JSON gets a parse-error
                // response (id null) — silently dropping it would leave a
                // client that sent an id waiting forever.
                server.trace(&format!("recv: unparseable frame: {e}"));
                server.send(&json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("parse error: {e}") },
                }))?;
                continue;
            }
        };
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let id = req.get("id").cloned();
        let params = req.get("params");
        server.trace(&format!("recv: {msg}"));
        match method {
            "build/initialize" => server.reply(id, server.initialize())?,
            "build/initialized" => {
                // The client is now ready for notifications: watch the project
                // for structure changes and push `buildTarget/didChange`.
                if !watching {
                    Arc::clone(&server).spawn_change_watcher();
                    watching = true;
                    // Warm every target in the background. The cold window —
                    // the stretch where a target's header maps and generated
                    // sources don't exist yet, so its ObjC files can't resolve
                    // their imports — otherwise opens afresh for each target
                    // the first time somebody opens a file in it. Queued at the
                    // back, so a target the client actually asks about still
                    // goes first.
                    server.prepare_queue.push_back(PrepareJob {
                        id: None,
                        targets: server.current_targets(),
                    });
                }
            }
            "workspace/buildTargets" => server.reply(id, server.build_targets())?,
            "buildTarget/sources" => server.reply(id, server.sources(params))?,
            "buildTarget/inverseSources" => server.reply(id, server.inverse_sources(params))?,
            "textDocument/sourceKitOptions" => {
                server.reply(id, server.source_kit_options(params))?;
            }
            "buildTarget/prepare" => {
                // Hand off to the worker; it replies once the build is done. A
                // prepare without an id (shouldn't happen) is simply dropped.
                if let Some(id) = id {
                    let targets = server.requested_targets(params);
                    // Ahead of the startup warm-up: this one is a file somebody
                    // has open, and it is the request whose reply sourcekit-lsp
                    // is blocked on.
                    server.prepare_queue.push_front(PrepareJob {
                        id: Some(id),
                        targets,
                    });
                }
            }
            "workspace/waitForBuildSystemUpdates" => server.reply(id, json!({}))?,
            "build/shutdown" | "shutdown" => server.reply(id, Value::Null)?,
            "build/exit" | "exit" => break,
            _ => {
                // Unknown request: a minimal "method not found" so the client
                // isn't left waiting; notifications (no id) are ignored.
                if let Some(id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": format!("method not found: {method}") },
                    });
                    server.send(&resp)?;
                }
            }
        }
    }
    server.prepare_queue.close();
    server.kill_prepare();
    server.shutdown_telemetry();
    Ok(())
}

struct Server {
    /// The root the server was pointed at: a `.xcodeproj` or a `.xcworkspace`.
    /// A project's embedded workspace is its project here, as it is to Xcode.
    project_path: PathBuf,
    /// The member `.xcodeproj`s — `[project_path]` for a project root, or the
    /// workspace's project refs for a `.xcworkspace`. File→target and settings
    /// resolution iterate these, so each file in a multi-project workspace
    /// resolves against whichever member declares its target.
    projects: Vec<PathBuf>,
    /// Live-updatable config (configuration, scheme, destination platform),
    /// swapped when `bsp.json` changes.
    live: Mutex<LiveConfig>,
    /// `--sdk` / `--arch` overrides; `None` means infer the platform per target.
    sdk: Option<String>,
    arch: Option<String>,
    xcode: Option<PathBuf>,
    derived_data_path: Option<PathBuf>,
    /// The settings the project's builds pass on the command line, from the
    /// caller, applied to every resolution and to the prepare build above the
    /// ones `bsp.json` carries (see [`Self::command_line`]).
    command_line: CommandLine,
    /// Target names in pbxproj order (cached at startup).
    targets: Vec<String>,
    /// Debug log sink — the file named by `SWEETPAD_BSP_LOG`, else nothing.
    /// stdout is the BSP protocol, so debug output must go elsewhere; a file is
    /// also the only channel observable after sourcekit-lsp spawns us detached.
    log: Option<Mutex<std::fs::File>>,
    /// Telemetry socket served to connected extensions: `bsp/log` + `bsp/status`
    /// out, `bsp/setLogLevel` in. The extension assigns the path in `bsp.json`;
    /// `None` until bound, and in `--project` standalone mode (which has none).
    telemetry: Mutex<Option<Arc<TelemetryServer>>>,
    /// The watched `bsp.json` path (from `--config`). Live scheme/configuration
    /// changes the extension persists there are applied without a restart;
    /// immutable fields are fixed at startup. `None` in `--project` standalone mode.
    config_path: Option<PathBuf>,
    /// Verbosity of the `bsp/log` stream, retunable live via `bsp/setLogLevel`.
    log_level: Arc<AtomicU8>,
    /// Where the protocol goes: stdout, the BSP channel, for a real server.
    /// [`Self::send`] holds the lock for a whole frame.
    out: Mutex<Box<dyn Write + Send>>,
    /// The prepare worker's process and the `TMPDIR` its `swiftc`s get (see
    /// [`PrepareProcess`]).
    prepare_process: Mutex<PrepareProcess>,
    /// Work waiting for the prepare worker (see [`PrepareQueue`]).
    prepare_queue: PrepareQueue,
    /// Per-target record of the last prepare, so repeats over unchanged project
    /// files don't re-spawn `xcodebuild`.
    prepared: Mutex<BTreeMap<String, PrepareRecord>>,
    /// The last prepare failure, in the shape the extension's Doctor prints.
    /// Prepare is best-effort by design, so without this a failure reaches only
    /// the debug log — and a target that never prepares is a target whose ObjC
    /// files never resolve their imports.
    last_prepare_failure: Mutex<Option<String>>,
    /// Which targets build as Mac Catalyst for a Mac destination
    /// ([`Self::builds_as_catalyst`]), against the [`Self::prepare_stamps`]
    /// they were read at.
    catalyst: Mutex<CatalystTargets>,
}

/// [`Server::builds_as_catalyst`]'s answers, and the project and config
/// stamps they hold for.
#[derive(Default)]
struct CatalystTargets {
    stamps: Vec<Option<(u64, SystemTime)>>,
    targets: BTreeMap<String, bool>,
}

/// The platform the editor analyzes a target for ([`Server::editor_platform`]).
struct EditorPlatform {
    sdk: String,
    arch: String,
    /// Built as Mac Catalyst: iOS code on the macOS SDK, which a Mac run
    /// destination resolves and an `-sdk macosx` alone does not.
    catalyst: bool,
}

const TARGET_SCHEME: &str = "sweetpad://target/";
const LANGUAGE_IDS: [&str; 5] = ["swift", "objective-c", "objective-cpp", "c", "cpp"];

/// A queued `buildTarget/prepare`: the target names to prepare, and the request
/// `id` to answer once the build is done — `None` for the startup warm-up,
/// which nobody is waiting on.
struct PrepareJob {
    id: Option<Value>,
    targets: Vec<String>,
}

/// The prepare worker's inbox: a double-ended queue rather than a channel, because
/// the two producers want opposite ends. A `buildTarget/prepare` is a file
/// somebody just opened and a reply sourcekit-lsp is blocked on, so it goes to
/// the front; the startup warm-up is speculative and goes to the back, where a
/// real request can overtake whatever of it is left.
#[derive(Default)]
struct PrepareQueue {
    state: Mutex<PrepareQueueState>,
    ready: Condvar,
}

#[derive(Default)]
struct PrepareQueueState {
    jobs: VecDeque<PrepareJob>,
    /// Set on shutdown: pushes are dropped and the worker is let go.
    closed: bool,
}

impl PrepareQueue {
    fn push_front(&self, job: PrepareJob) {
        self.push(job, true);
    }

    fn push_back(&self, job: PrepareJob) {
        self.push(job, false);
    }

    fn push(&self, job: PrepareJob, front: bool) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.closed {
            return;
        }
        if front {
            state.jobs.push_front(job);
        } else {
            state.jobs.push_back(job);
        }
        self.ready.notify_one();
    }

    /// Block until a job is available, or return `None` once closed.
    fn take(&self) -> Option<PrepareJob> {
        let mut state = self.state.lock().ok()?;
        loop {
            if let Some(job) = state.jobs.pop_front() {
                return Some(job);
            }
            if state.closed {
                return None;
            }
            state = self.ready.wait(state).ok()?;
        }
    }

    /// Stop accepting work, drop what's queued, and wake the worker so it can
    /// exit. Whatever build is already running is killed separately, by
    /// [`Server::kill_prepare`].
    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.jobs.clear();
        }
        self.ready.notify_all();
    }
}

/// The process a prepare is running, held where shutdown can reach it.
#[derive(Default)]
struct PrepareProcess {
    /// The `xcodebuild` or `swiftc` running now, if any. Held so shutdown can
    /// kill it: `build/exit` during a prepare would otherwise orphan it
    /// (reparented to init, a minutes-long xcodebuild still burning CPU after
    /// every editor restart). The prepare worker reaps it.
    child: Option<Child>,
    /// The `TMPDIR` every prepare `swiftc` runs with, made for the first one
    /// and removed at shutdown. The Swift driver leaves a
    /// `TemporaryDirectory.*` in its `TMPDIR` whenever it dies before it
    /// finishes: killed at shutdown, or by its own diagnostics once nobody
    /// reads its pipe. The server runs under the user's editor, so that would
    /// be the user's `$TMPDIR`.
    swiftc_tmp: Option<ScratchDir>,
    /// Set at shutdown, after which nothing more is spawned.
    stopped: bool,
}

/// What the last prepare of a target did, so a repeat over unchanged inputs can
/// be skipped and a failure can be reported.
struct PrepareRecord {
    /// The project-file stamps it ran against — a change means the answer may
    /// differ, so the skip no longer applies.
    stamps: Vec<Option<(u64, SystemTime)>>,
    ok: bool,
    at: Instant,
}

/// How long a *failed* prepare stands in for the next attempt over unchanged
/// project files. A success is cached until the project changes; a failure is
/// retried after this, since what broke it (a missing tool, a half-written
/// file, a source error) often isn't visible in the pbxproj at all.
const PREPARE_RETRY_AFTER: Duration = Duration::from_secs(60);

/// The portion of config that can change while the server runs — re-read from
/// `bsp.json` when the extension rewrites it. Everything else (project/xcode/
/// derived data) is fixed at startup; toolchain/DD changes warrant a restart
/// instead.
#[derive(Clone, PartialEq)]
struct LiveConfig {
    configuration: String,
    scheme: Option<String>,
    /// The platform of the destination the extension builds for
    /// (`iphonesimulator`, `watchos`, …), from `bsp.json`'s
    /// `destinationPlatform`.
    destination_platform: Option<String>,
    /// The settings in `bsp.json`'s `buildArgs`.
    command_line: CommandLine,
}

/// The inputs the server needs, however they were obtained — from `--project`
/// flags or the extension's `bsp.json` (named by `--config`).
struct ResolvedConfig {
    project_path: PathBuf,
    configuration: String,
    scheme: Option<String>,
    destination_platform: Option<String>,
    sdk: Option<String>,
    arch: Option<String>,
    xcode: Option<PathBuf>,
    derived_data_path: Option<PathBuf>,
    /// Debug log file: from `bsp.json`'s `logPath`, else `$SWEETPAD_BSP_LOG`.
    log_path: Option<PathBuf>,
    /// Telemetry socket to bind, assigned by the extension in `bsp.json`. `None`
    /// in `--project` standalone mode (no telemetry).
    socket: Option<PathBuf>,
    /// The settings in `bsp.json`'s `buildArgs`: what the extension's builds
    /// pass `xcodebuild` from `sweetpad.build.args`. Empty for a `--project`
    /// server and for a `bsp.json` without them.
    command_line: CommandLine,
    /// What reading the config found wrong but worked around, logged once
    /// the server starts.
    warning: Option<String>,
}

/// The `SWEETPAD_BSP_LOG` env path (used by tests and the standalone paths).
fn env_log() -> Option<PathBuf> {
    std::env::var_os("SWEETPAD_BSP_LOG").map(PathBuf::from)
}

/// Cap on the debug log. The log now lives in the persistent per-project state
/// dir (not a reboot-cleared tmpdir), so an append-only file would grow without
/// bound; [`open_log`] truncates it on startup once it's over this size.
const BSP_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Open the debug log for append, creating its parent dir first so the first
/// write after a cold start doesn't silently drop. Bounds growth by truncating
/// on startup when the existing file is already over [`BSP_LOG_MAX_BYTES`].
fn open_log(path: &Path) -> Option<std::fs::File> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut opts = OpenOptions::new();
    opts.create(true).write(true);
    if std::fs::metadata(path).is_ok_and(|m| m.len() > BSP_LOG_MAX_BYTES) {
        opts.truncate(true);
    } else {
        opts.append(true);
    }
    opts.open(path).ok()
}

/// Locate the `bsp.json` for the project containing the cwd — the fallback when
/// `buildServer.json` carries no explicit `--config`. Walks up from the cwd
/// through the extension's discovery index and reads the nearest registered
/// ancestor's `bspConfig` path.
fn discover_config_from_cwd() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    crate::paths::lookup_index_entry(&cwd)
        .as_ref()
        .and_then(|entry| entry.get("bspConfig"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| {
            "no --config <bsp.json> given and no bspConfig registered for this directory; the SweetPad extension writes buildServer.json with --config, or use 'sweetpad bsp init' for a standalone config".to_string()
        })
}

impl ResolvedConfig {
    fn from_flags(project_path: PathBuf, flags: &BTreeMap<String, String>) -> Self {
        ResolvedConfig {
            project_path,
            configuration: flags
                .get("configuration")
                .cloned()
                .unwrap_or_else(|| "Debug".into()),
            scheme: flags.get("scheme").cloned(),
            destination_platform: None,
            sdk: flags.get("sdk").cloned(),
            arch: flags.get("arch").cloned(),
            xcode: flags.get("xcode").map(PathBuf::from),
            derived_data_path: flags.get("derived-data-path").map(PathBuf::from),
            log_path: env_log(),
            socket: None,
            command_line: CommandLine::default(),
            warning: None,
        }
    }

    /// Build from a `bsp.json` object (the key schema the extension writes). Path
    /// fields are normally absolute; any relative one is resolved against `base`.
    /// Any explicit flag still wins, so it can be combined with targeted overrides.
    fn from_json(
        value: &Value,
        base: &Path,
        flags: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let pull = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        // `base.join` roots a relative path at the project and leaves an absolute
        // one untouched — so out-of-tree paths (Xcode, the socket) stay as written.
        let resolve = |p: String| base.join(p);
        // `projectPath` is the Xcode container (`.xcodeproj`/`.xcworkspace`);
        // `workspacePath` is the VS Code workspace *folder*, which is not
        // openable as a project — accept it only when it actually names an
        // `.xcworkspace` (older configs carried the container there).
        let project_path = flags
            .get("workspace")
            .cloned()
            .or_else(|| flags.get("project").cloned())
            .or_else(|| pull("projectPath"))
            .or_else(|| {
                pull("workspacePath").filter(|p| {
                    Path::new(p)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("xcworkspace"))
                })
            })
            .ok_or("bsp.json missing projectPath/workspacePath")?;
        let (command_line, warning) = command_line_of(value, base);
        Ok(ResolvedConfig {
            project_path: resolve(project_path),
            configuration: flags
                .get("configuration")
                .cloned()
                .or_else(|| configuration_of(value))
                .unwrap_or_else(|| "Debug".into()),
            scheme: flags.get("scheme").cloned().or_else(|| pull("scheme")),
            destination_platform: pull("destinationPlatform"),
            sdk: flags.get("sdk").cloned(),
            arch: flags.get("arch").cloned(),
            xcode: flags
                .get("xcode")
                .cloned()
                .or_else(|| pull("developerDir"))
                .map(&resolve),
            derived_data_path: flags
                .get("derived-data-path")
                .cloned()
                .or_else(|| pull("derivedDataPath"))
                .map(&resolve),
            log_path: pull("logPath").map(&resolve).or_else(env_log),
            socket: pull("socket").map(&resolve),
            command_line,
            warning,
        })
    }

    /// Read and parse the `bsp.json` the extension writes. The extension now
    /// writes absolute paths (the config lives outside the project tree, in the
    /// host state dir), but any relative field still resolves against the
    /// workspace root the file names in `workspacePath`, falling back to the
    /// config file's own directory.
    fn from_file(path: &Path, flags: &BTreeMap<String, String>) -> Result<Self, String> {
        let raw =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let value: Value =
            serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))?;
        Self::from_json(&value, &config_base(&value, path), flags)
    }
}

/// The directory a relative path in the `bsp.json` at `path` resolves
/// against: the workspace root it names in `workspacePath`, else its own.
fn config_base(value: &Value, path: &Path) -> PathBuf {
    value
        .get("workspacePath")
        .and_then(Value::as_str)
        .map_or_else(
            || {
                path.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            },
            PathBuf::from,
        )
}

/// The command-line settings in a `bsp.json`'s `buildArgs`: the arguments
/// the extension's builds add to `xcodebuild`, from `sweetpad.build.args`.
/// They are read as `xcodebuild` running in `base`, the directory those
/// builds run in, reads them ([`CommandLineSettings::of`], the CLI's reader
/// too): a relative `-xcconfig` joins the directory's physical path, so
/// through a symlinked folder `../ci.xcconfig` is the real directory's
/// sibling. A file without `buildArgs` has none. Its `-derivedDataPath` is
/// the extension's to resolve into `derivedDataPath`.
///
/// The second half is a warning for a flag that ends `buildArgs` without its
/// value. The extension's builds fail on it, and the index reads the rest
/// without it, so a half-typed edit doesn't cost autocomplete the settings
/// before it.
fn command_line_of(value: &Value, base: &Path) -> (CommandLine, Option<String>) {
    let args = build_args(value);
    let warning = xcodebuild_args::dangling_flag(&args).map(|flag| {
        format!(
            "ignoring '{flag}' at the end of buildArgs: it has no value, and xcodebuild refuses it"
        )
    });
    (CommandLineSettings::of(&args, Some(base)).into(), warning)
}

/// A `bsp.json`'s `buildArgs`, the extension's `sweetpad.build.args`.
fn build_args(value: &Value) -> Vec<String> {
    value
        .get("buildArgs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// The configuration the extension's builds use: a `-configuration` in
/// `buildArgs` replaces the one the extension picks on their command line, so
/// it wins over the file's `configuration`, and the index resolves the
/// configuration the build compiles.
fn configuration_of(value: &Value) -> Option<String> {
    xcodebuild_args::last_value(&build_args(value), "-configuration")
        .map(str::to_string)
        .or_else(|| {
            value
                .get("configuration")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

impl Server {
    /// Resolve the server config. An explicit `--project`/`--workspace` stays
    /// self-contained (no config file, no telemetry — used by `bsp init` and the
    /// tests). Otherwise the `bsp.json` is located one of two ways: the explicit
    /// `--config` path `buildServer.json` carries (the extension writes this), or
    /// — when that's absent (an older or hand-written stub) — by discovering it
    /// from the cwd via the host-wide index the extension maintains. Either way it
    /// is then read and watched for live changes. `command_line` is layered on
    /// every resolution either way, above the settings a `bsp.json` carries.
    fn resolve(args: &[String], command_line: CommandLine) -> Result<Self, String> {
        let flags = parse_flags(args);
        let log_level = Arc::new(AtomicU8::new(LogLevel::Info as u8));

        if let Some(root) = flags.get("workspace").or_else(|| flags.get("project")) {
            let config = ResolvedConfig::from_flags(PathBuf::from(root), &flags);
            return Self::build(
                config,
                None,
                log_level,
                command_line,
                Box::new(io::stdout()),
            );
        }

        let config_file = match flags.get("config") {
            Some(path) => PathBuf::from(path),
            None => discover_config_from_cwd()?,
        };
        let config = ResolvedConfig::from_file(&config_file, &flags)?;
        Self::build(
            config,
            Some(config_file),
            log_level,
            command_line,
            Box::new(io::stdout()),
        )
    }

    /// The server for `config`, sending the protocol to `out`: stdout for a
    /// real server, a buffer for a test.
    fn build(
        config: ResolvedConfig,
        config_path: Option<PathBuf>,
        log_level: Arc<AtomicU8>,
        command_line: CommandLine,
        out: Box<dyn Write + Send>,
    ) -> Result<Self, String> {
        // A `.xcworkspace` root expands to its member projects; a `.xcodeproj`
        // root is a one-element list. Targets are the union across members.
        // A project's embedded `project.xcworkspace` opens the project, which
        // is what its DerivedData and its directory are keyed by.
        let root = sweetpad_lib::workspace::normalize_stub_workspace(&config.project_path);
        let projects: Vec<PathBuf> =
            if root.extension().and_then(|e| e.to_str()) == Some("xcworkspace") {
                sweetpad_lib::workspace::open(&root)
                    .map_err(|e| format!("open workspace: {e}"))?
                    .project_refs
            } else {
                vec![root.clone()]
            };
        let mut targets: Vec<String> = Vec::new();
        for p in &projects {
            if let Ok(ctx) = BuildContext::open(p) {
                for t in &ctx.project.targets {
                    if !targets.contains(&t.name) {
                        targets.push(t.name.clone());
                    }
                }
            }
        }
        // Surface a genuinely broken single project (a workspace tolerates a
        // member that won't open).
        if targets.is_empty() && projects.len() == 1 {
            BuildContext::open(&projects[0]).map_err(|e| format!("open project: {e}"))?;
        }
        // Log file from the config's `logPath` or the SWEETPAD_BSP_LOG env
        // (tests); telemetry streams logs regardless of the file.
        let log = config
            .log_path
            .as_deref()
            .and_then(open_log)
            .map(Mutex::new);
        let server = Server {
            project_path: root,
            projects,
            live: Mutex::new(LiveConfig {
                configuration: config.configuration,
                scheme: config.scheme,
                destination_platform: config.destination_platform,
                command_line: config.command_line,
            }),
            sdk: config.sdk,
            arch: config.arch,
            xcode: config.xcode,
            derived_data_path: config.derived_data_path,
            command_line,
            targets,
            log,
            telemetry: Mutex::new(None),
            config_path,
            log_level,
            out: Mutex::new(out),
            prepare_process: Mutex::new(PrepareProcess::default()),
            prepare_queue: PrepareQueue::default(),
            prepared: Mutex::new(BTreeMap::new()),
            last_prepare_failure: Mutex::new(None),
            catalyst: Mutex::new(CatalystTargets::default()),
        };
        server.bind_telemetry(config.socket.as_deref());
        server.log(&format!(
            "start: project={} xcode={:?} dd={:?} command_line={:?} telemetry={} \
             config_watch={:?} targets={:?}",
            server.project_path.display(),
            server.xcode,
            server.derived_data_path,
            server.command_line(),
            server.telemetry.lock().is_ok_and(|t| t.is_some()),
            server.config_path,
            server.targets,
        ));
        if let Some(warning) = &config.warning {
            server.log(warning);
        }
        Ok(server)
    }

    /// The current build configuration (swapped live when `bsp.json` changes).
    fn configuration(&self) -> String {
        self.live
            .lock()
            .map_or_else(|_| "Debug".into(), |c| c.configuration.clone())
    }

    /// The command-line settings every resolution and prepare build take:
    /// those in `bsp.json` (swapped live when it changes), then the caller's,
    /// which win. The caller's `-xcconfig` replaces the file's.
    fn command_line(&self) -> CommandLine {
        let from_file = self
            .live
            .lock()
            .map(|c| c.command_line.clone())
            .unwrap_or_default();
        CommandLine {
            xcconfig: self.command_line.xcconfig.clone().or(from_file.xcconfig),
            overrides: from_file
                .overrides
                .into_iter()
                .chain(self.command_line.overrides.iter().cloned())
                .collect(),
        }
    }

    /// Swap the live config and, when it actually changed, tell the client to
    /// re-pull options via `buildTarget/didChange`. A missing `configuration`
    /// keeps the current value; the diff prevents redundant refresh storms.
    fn apply_config(
        &self,
        configuration: Option<&str>,
        scheme: Option<String>,
        destination_platform: Option<String>,
        command_line: CommandLine,
    ) {
        let next = {
            let Ok(mut live) = self.live.lock() else {
                return;
            };
            let updated = LiveConfig {
                configuration: configuration
                    .map_or_else(|| live.configuration.clone(), str::to_string),
                scheme,
                destination_platform,
                command_line,
            };
            if updated == *live {
                return;
            }
            *live = updated.clone();
            updated
        };
        self.log(&format!(
            "config changed: configuration={} scheme={:?} destination_platform={:?} command_line={:?}",
            next.configuration, next.scheme, next.destination_platform, next.command_line
        ));
        self.notify_targets_changed();
    }

    /// Re-read `bsp.json` after a change: apply the volatile selection
    /// (configuration, scheme, destination platform and command-line
    /// settings) live via
    /// [`Self::apply_config`], and bind the
    /// telemetry socket if one has just appeared. Immutable fields (project/
    /// xcode/derived data) are deliberately not refreshed — they're fixed at
    /// startup, so a change to them needs a server restart.
    fn reload_from_file(&self, path: &Path) {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            return;
        };
        let configuration = configuration_of(&value);
        let pull = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        let (command_line, warning) = command_line_of(&value, &config_base(&value, path));
        if let Some(warning) = &warning {
            self.log(warning);
        }
        self.apply_config(
            configuration.as_deref(),
            pull("scheme"),
            pull("destinationPlatform"),
            command_line,
        );
        let socket = value
            .get("socket")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        self.bind_telemetry(socket.as_deref());
    }

    /// An operational log line: to the `SWEETPAD_BSP_LOG` file (unconditional)
    /// and the extension stream at `info`, so it's visible by default.
    fn log(&self, msg: &str) {
        self.write_log(msg);
        self.push_log(LogLevel::Info, msg);
    }

    /// A high-volume trace (raw JSON-RPC frames): to the file unconditionally,
    /// but to the extension stream only at `debug` so it's opt-in.
    fn trace(&self, msg: &str) {
        self.write_log(msg);
        self.push_log(LogLevel::Debug, msg);
    }

    /// Append a timestamped line to the debug log (no-op unless `SWEETPAD_BSP_LOG`
    /// is set). Epoch-millis timestamps keep it dependency-free.
    fn write_log(&self, msg: &str) {
        if let Some(file) = &self.log
            && let Ok(mut file) = file.lock()
        {
            let ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            let _ = writeln!(file, "[{ms}] {msg}");
            let _ = file.flush();
        }
    }

    /// Bind the telemetry socket the extension assigned in `bsp.json`, so
    /// connected extensions get `bsp/log` / `bsp/status` and can push
    /// `bsp/setLogLevel`. No-op without a socket (a headless / Neovim setup) or
    /// once bound — the path is stable, so it's bound at most once.
    fn bind_telemetry(&self, socket: Option<&Path>) {
        let Some(socket) = socket else {
            return;
        };
        let Ok(mut slot) = self.telemetry.lock() else {
            return;
        };
        if slot.is_some() {
            return;
        }
        let level = Arc::clone(&self.log_level);
        if let Some(server) = TelemetryServer::bind(socket, move |lvl| {
            level.store(LogLevel::parse(lvl) as u8, Ordering::Relaxed);
        }) {
            *slot = Some(server);
        }
    }

    /// The bound telemetry server, if any — cloned out so the lock isn't held
    /// across a (possibly blocking) socket write.
    fn telemetry(&self) -> Option<Arc<TelemetryServer>> {
        self.telemetry.lock().ok().and_then(|s| s.clone())
    }

    /// Stream one log line to connected extensions when the live level permits it
    /// (the `SWEETPAD_BSP_LOG` file is unaffected).
    fn push_log(&self, level: LogLevel, msg: &str) {
        if level as u8 > self.log_level.load(Ordering::Relaxed) {
            return;
        }
        if let Some(server) = self.telemetry() {
            server.broadcast(
                "bsp/log",
                json!({ "level": level.as_str(), "message": msg }),
            );
        }
    }

    /// Push a coarse status phase to connected extensions' status bars (no-op
    /// when nothing is connected).
    fn push_status(&self, phase: &str, detail: Option<&str>) {
        if let Some(server) = self.telemetry() {
            server.broadcast("bsp/status", json!({ "phase": phase, "detail": detail }));
        }
    }

    /// Unlink the telemetry socket on a clean shutdown.
    fn shutdown_telemetry(&self) {
        if let Some(server) = self.telemetry() {
            server.shutdown();
        }
    }

    fn project_dir(&self) -> &Path {
        self.project_path.parent().unwrap_or_else(|| Path::new("."))
    }

    /// The member `.xcodeproj` that declares `target`. A single-project root
    /// returns it directly; a workspace finds the first member whose targets
    /// include `target` (a cross-project name clash resolves to the first).
    fn project_for_target(&self, target: &str) -> PathBuf {
        if self.projects.len() == 1 {
            return self.projects[0].clone();
        }
        self.projects
            .iter()
            .find(|p| {
                BuildContext::open(p)
                    .map(|c| c.project.targets.iter().any(|t| t.name == target))
                    .unwrap_or(false)
            })
            .or_else(|| self.projects.first())
            .cloned()
            .unwrap_or_else(|| self.project_path.clone())
    }

    /// Whether the root is a `.xcworkspace` (prepare builds with `-workspace`).
    fn is_workspace(&self) -> bool {
        self.project_path.extension().and_then(|e| e.to_str()) == Some("xcworkspace")
    }

    /// The Xcode **Developer** directory (what `DEVELOPER_DIR` / `xcodebuild`
    /// want), normalized from `--xcode` which may be given as either the `.app`
    /// bundle or the Developer dir itself.
    fn developer_dir(&self) -> Option<PathBuf> {
        let x = self.xcode.as_ref()?;
        let nested = x.join("Contents/Developer");
        Some(if nested.is_dir() { nested } else { x.clone() })
    }

    fn initialize(&self) -> Value {
        self.push_status("ready", None);
        // Advertise the per-file options extension and `prepareProvider`, so
        // sourcekit-lsp's background indexing delegates `buildTarget/prepare` to
        // us (we build dependency modules on demand). When we can locate the
        // build's DerivedData, also advertise its index store for project-wide
        // navigation from the index-while-building data.
        let mut data = json!({ "sourceKitOptionsProvider": true, "prepareProvider": true });
        if let Some(dd) = self.derived_data().map(|l| l.folder) {
            data["indexStorePath"] = json!(dd.join("Index.noindex/DataStore").to_string_lossy());
            data["indexDatabasePath"] =
                json!(dd.join("Index.noindex/IndexDatabase").to_string_lossy());
        }
        json!({
            "displayName": "sweetpad",
            "version": env!("CARGO_PKG_VERSION"),
            "bspVersion": "2.2.0",
            "capabilities": { "languageIds": LANGUAGE_IDS },
            "dataKind": "sourceKit",
            "data": data,
        })
    }

    /// Where the build's DerivedData lands: under the `--derived-data-path`
    /// override, else wherever this machine's Xcode settings put it
    /// (`~/Library/Developer/Xcode/DerivedData/<name>-<hash>` by default). The
    /// same locator places the editor arguments' build products, since
    /// [`Self::options_for`] reads the same settings.
    ///
    /// The folder is keyed the way `xcodebuild` keys the one it writes
    /// ([`derived_data::ContainerKey`]): a root reached through a symlink, or
    /// spelled `/private/tmp/…`, shares the folder of its standardized path.
    /// The home is the account's, as `xcodebuild` finds it, whatever `$HOME`
    /// says ([`sweetpad_lib::host::home`]). `None` without a home or an
    /// override to find it from.
    fn derived_data(&self) -> Option<derived_data::Locations> {
        let home = sweetpad_lib::host::home()
            .map(|home| home.to_string_lossy().into_owned())
            .unwrap_or_default();
        if home.is_empty() && self.derived_data_path.is_none() {
            return None;
        }
        let key = derived_data::ContainerKey::of(&self.project_path);
        if key.name.is_empty() {
            return None;
        }
        Some(derived_data::resolve(
            &key,
            &home,
            self.derived_data_path.as_deref(),
            true,
        ))
    }

    fn build_targets(&self) -> Value {
        let base = file_uri(self.project_dir());
        // Re-read the target set so a project regenerated mid-session (a target
        // added/removed) is reflected when the client re-queries after a
        // `buildTarget/didChange`.
        let targets = self.current_targets();
        let target_list: Vec<Value> = targets
            .iter()
            .map(|name| {
                // Only edges to targets we also expose are useful to sourcekit-lsp;
                // drop any that fall outside this project's target set.
                let deps: Vec<Value> = project::target_dependencies(&self.project_for_target(name), name)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|d| targets.contains(d))
                    .map(|d| target_id(&d))
                    .collect();
                json!({
                    "id": target_id(name),
                    "displayName": name,
                    "baseDirectory": base,
                    "tags": [],
                    "languageIds": LANGUAGE_IDS,
                    "dependencies": deps,
                    "capabilities": { "canCompile": true, "canTest": false, "canRun": false, "canDebug": false },
                })
            })
            .collect();
        json!({ "targets": target_list })
    }

    /// The project's current target names — re-read from disk (the pbxproj parse
    /// is `(len, mtime)`-cached, so this is cheap and reflects edits), falling
    /// back to the startup set if the project momentarily fails to open.
    fn current_targets(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in &self.projects {
            if let Ok(ctx) = BuildContext::open(p) {
                for t in &ctx.project.targets {
                    if !out.contains(&t.name) {
                        out.push(t.name.clone());
                    }
                }
            }
        }
        if out.is_empty() {
            self.targets.clone()
        } else {
            out
        }
    }

    /// Watch the project file (and the `bsp.json` config, when present)
    /// and react without an LSP restart: a project-structure change pushes
    /// `buildTarget/didChange` so the client re-queries targets/sources; a config
    /// change re-applies the live scheme/configuration. Per-request resolution is
    /// already fresh (the parse cache is mtime-validated); this is the push that
    /// tells the client to ask again. Polling (no notify dependency) keeps it
    /// portable; the interval is overridable via `SWEETPAD_BSP_WATCH_MS` (tests).
    fn spawn_change_watcher(self: Arc<Self>) {
        let interval = std::env::var("SWEETPAD_BSP_WATCH_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .map_or(Duration::from_millis(1500), Duration::from_millis);
        let documents: Vec<PathBuf> = self
            .projects
            .iter()
            .flat_map(|p| document_paths(p))
            .collect();
        let config = self.config_path.clone();
        std::thread::spawn(move || {
            let stamp_all =
                |paths: &[PathBuf]| paths.iter().map(|p| file_stamp(p)).collect::<Vec<_>>();
            let mut last_doc = stamp_all(&documents);
            let mut last_cfg = config.as_deref().map(file_stamp);
            loop {
                std::thread::sleep(interval);
                let now_doc = stamp_all(&documents);
                if now_doc != last_doc {
                    last_doc = now_doc;
                    self.notify_targets_changed();
                }
                if let (Some(path), Some(prev)) = (config.as_deref(), last_cfg.as_mut()) {
                    let now_cfg = file_stamp(path);
                    if now_cfg != *prev {
                        *prev = now_cfg;
                        self.reload_from_file(path);
                    }
                }
            }
        });
    }

    /// Send `buildTarget/didChange` marking every current target changed.
    fn notify_targets_changed(&self) {
        self.notify_changed(&self.current_targets());
    }

    /// Send `buildTarget/didChange` for `targets` — the push that tells the
    /// client to re-pull their options.
    fn notify_changed(&self, targets: &[String]) {
        let changes: Vec<Value> = targets
            .iter()
            .map(|t| json!({ "target": target_id(t), "kind": 2 }))
            .collect();
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "buildTarget/didChange",
            "params": { "changes": changes },
        });
        let _ = self.send(&notif);
    }

    /// Run a queued `buildTarget/prepare`: build each requested target's
    /// dependency modules + generated inputs, then answer the request. Always
    /// replies (even on build failure) — prepare is best-effort, and a missing
    /// response would wedge sourcekit-lsp's semantics for that target.
    fn run_prepare(&self, job: &PrepareJob) {
        self.push_status("preparing", Some(&job.targets.join(", ")));
        let mut built = Vec::new();
        let mut failure = None;
        for target in &job.targets {
            // A build that fails still writes the header maps — they come out of
            // an early phase, not out of compiling — so an attempt counts as a
            // change to re-read whether or not it succeeded.
            let Some(ok) = self.prepare_target(target) else {
                continue;
            };
            built.push(target.clone());
            if !ok && failure.is_none() {
                failure = self.last_prepare_failure();
            }
        }
        match &failure {
            Some(detail) => self.push_status("failed", Some(detail)),
            None => self.push_status("ready", None),
        }
        if let Some(id) = &job.id {
            let resp = json!({ "jsonrpc": "2.0", "id": id, "result": {} });
            let _ = self.send(&resp);
        }
        // The build is what puts a target's header maps and generated sources on
        // disk, and the editor arguments name those only once they exist — so
        // whatever the client resolved before this ran is missing them. Nothing
        // else pushes that: the change watcher fires on project edits, and a
        // build isn't one. Sent after the reply, so the client re-pulls a target
        // it already considers prepared.
        if !built.is_empty() {
            self.notify_changed(&built);
        }
    }

    /// Whether `target` was prepared recently enough, against project files that
    /// haven't changed since, for another build to be pointless.
    fn prepare_is_current(&self, target: &str, stamps: &[Option<(u64, SystemTime)>]) -> bool {
        let Ok(prepared) = self.prepared.lock() else {
            return false;
        };
        let Some(record) = prepared.get(target) else {
            return false;
        };
        record.stamps == stamps && (record.ok || record.at.elapsed() < PREPARE_RETRY_AFTER)
    }

    /// The `(len, mtime)` of every file a prepare's result depends on: each
    /// member project and the live config. Same stamps as the change watcher
    /// polls, which is what makes "unchanged" mean the same thing to both.
    fn prepare_stamps(&self) -> Vec<Option<(u64, SystemTime)>> {
        self.projects
            .iter()
            .flat_map(|p| document_paths(p))
            .map(|p| file_stamp(&p))
            .chain(self.config_path.as_deref().map(file_stamp))
            .collect()
    }

    /// Prepare `target` so its sources become semantically analyzable: build the
    /// modules it imports. The fast path emits each dependency's module with
    /// `swiftc` directly (no `xcodebuild` process, no link, single arch) when the
    /// whole closure — the target and its transitive deps — is pure Swift;
    /// anything else (packages, C-family, code-gen) falls back to a real
    /// `xcodebuild`, as does a self-build that unexpectedly fails.
    ///
    /// `None` when the target was already current, else whether the build
    /// succeeded — either way the client should re-read what's on disk.
    fn prepare_target(&self, target: &str) -> Option<bool> {
        let stamps = self.prepare_stamps();
        if self.prepare_is_current(target, &stamps) {
            self.log(&format!("prepare: {target} already current; skipping"));
            return None;
        }
        let proj = self.project_for_target(target);
        // The self-build fast path is single-project: a workspace target's deps
        // can live in another member, so let xcodebuild handle the closure.
        let deps = project::transitive_dependencies(&proj, target).unwrap_or_default();
        // `is_self_buildable` is false for a target with any C-family source, so
        // a closure that qualifies has no header maps or generated sources to
        // produce — which is what makes it sound to skip the real build here
        // even though "prepared" then doesn't mean "artifacts on disk".
        let closure_simple = !self.is_workspace()
            && std::iter::once(target)
                .chain(deps.iter().map(String::as_str))
                .all(|t| project::is_self_buildable(&proj, t).unwrap_or(false));
        if closure_simple {
            // The target itself is type-checked live by sourcekit-lsp; we only
            // need its dependency modules on disk.
            if deps.iter().all(|dep| self.self_build_module(dep)) {
                self.log(&format!(
                    "prepare: {target} self-built {} dependency module(s)",
                    deps.len()
                ));
                self.record_prepare(target, stamps, true, None);
                return Some(true);
            }
            self.log(&format!(
                "prepare: {target} self-build failed; falling back to xcodebuild"
            ));
        }
        Some(self.xcodebuild_prepare(target, stamps))
    }

    /// The last prepare failure's detail, for the terminal status and the Doctor.
    fn last_prepare_failure(&self) -> Option<String> {
        self.last_prepare_failure.lock().ok()?.clone()
    }

    /// Record how a prepare went, keeping a failure's detail for the terminal
    /// status and the Doctor — the debug log alone reaches nobody who hasn't
    /// already been told where it is.
    fn record_prepare(
        &self,
        target: &str,
        stamps: Vec<Option<(u64, SystemTime)>>,
        ok: bool,
        detail: Option<String>,
    ) {
        if let Ok(mut prepared) = self.prepared.lock() {
            prepared.insert(
                target.to_string(),
                PrepareRecord {
                    stamps,
                    ok,
                    at: Instant::now(),
                },
            );
        }
        if let Ok(mut slot) = self.last_prepare_failure.lock() {
            *slot = (!ok).then(|| {
                format!(
                    "{target}: {}",
                    detail.unwrap_or_else(|| "no detail".to_string())
                )
            });
        }
    }

    /// Emit one dependency's `.swiftmodule` with `swiftc` straight into the
    /// products dir its dependents search, using the same editor arguments we
    /// feed sourcekit-lsp (plus `-emit-module`). Returns whether it produced the
    /// module. The module name and products dir are read back out of those args
    /// so the output lands exactly where dependents' `-I` looks.
    fn self_build_module(&self, target: &str) -> bool {
        let swift_sources: Vec<PathBuf> =
            project::target_source_files(&self.project_for_target(target), target)
                .unwrap_or_default()
                .into_iter()
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("swift"))
                .collect();
        let Some(first) = swift_sources.first() else {
            return true; // nothing to emit
        };
        let Some(args) = self.compiler_arguments(target, first) else {
            return false;
        };
        let Some(products) = arg_values(&args, "-I")
            .into_iter()
            .find(|p| p.contains("/Build/Products/"))
        else {
            return false;
        };
        let module_name = arg_values(&args, "-module-name")
            .into_iter()
            .next()
            .unwrap_or_else(|| target.to_string());
        let module_path = Path::new(&products).join(format!("{module_name}.swiftmodule"));
        if std::fs::create_dir_all(&products).is_err() {
            return false;
        }
        let Some(tmp) = self.swiftc_tmp() else {
            self.log(&format!("prepare: no TMPDIR to emit {module_name} in"));
            return false;
        };
        let swiftc = self.developer_dir().map_or_else(
            || PathBuf::from("swiftc"),
            |dev| dev.join("Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc"),
        );
        let mut cmd = Command::new(&swiftc);
        if let Some(dev) = self.developer_dir() {
            cmd.env("DEVELOPER_DIR", dev);
        }
        cmd.env("TMPDIR", &tmp)
            .arg("-emit-module")
            .arg("-emit-module-path")
            .arg(&module_path)
            .args(&args);
        match self.run_prepare_process(&mut cmd) {
            Ok((Some(status), _)) if status.success() => {
                self.log(&format!(
                    "prepare: emitted module {module_name} -> {}",
                    module_path.display()
                ));
                true
            }
            Ok((_, stderr)) => {
                let stderr = String::from_utf8_lossy(&stderr);
                let tail: String = stderr.lines().rev().take(6).collect::<Vec<_>>().join(" | ");
                self.log(&format!("prepare: emit {module_name} failed: {tail}"));
                false
            }
            Err(e) => {
                self.log(&format!(
                    "prepare: failed to launch swiftc for {module_name}: {e}"
                ));
                false
            }
        }
    }

    /// The `xcodebuild` invocation that prepares `target`, and a phrase naming
    /// how, for the log.
    fn prepare_command(&self, target: &str) -> (Command, String) {
        let owning = self.project_for_target(target);
        let scheme = project::scheme_for_target(&owning, target);
        let EditorPlatform { sdk, arch, .. } = self.editor_platform(target);
        let developer = self.developer_dir();
        let mut cmd = Command::new(developer.as_ref().map_or_else(
            || PathBuf::from("xcodebuild"),
            |dev| dev.join("usr/bin/xcodebuild"),
        ));
        if let Some(dev) = &developer {
            cmd.env("DEVELOPER_DIR", dev);
        }
        cmd.arg("build");
        // A `-target` build has no scheme to name the workspace's other members,
        // so it always addresses the owning project directly.
        if self.is_workspace() && scheme.is_some() {
            cmd.args(["-workspace".as_ref(), self.project_path.as_os_str()]);
        } else {
            cmd.args(["-project".as_ref(), owning.as_os_str()]);
        }
        let how = if let Some(scheme) = &scheme {
            let destination = format!("generic/platform={}", platform_name(&sdk));
            cmd.args(["-scheme", scheme])
                .args(["-destination", &destination]);
            if let Some(dd) = &self.derived_data_path {
                // An explicit override needs `-derivedDataPath` (which xcodebuild
                // only accepts with `-scheme`); without it the default
                // DerivedData already matches our search paths.
                cmd.args(["-derivedDataPath".as_ref(), dd.as_os_str()]);
            }
            format!("scheme {scheme} ({destination})")
        } else {
            cmd.args(["-target", target])
                .args(["-sdk", &sdk])
                .args(["-arch", &arch]);
            // The two roots every output path hangs off. Named after the same
            // DerivedData the arguments resolve against, so `BUILT_PRODUCTS_DIR`
            // and `TEMP_DIR` land where the editor is already looking.
            for (key, dir) in self.build_roots() {
                cmd.arg(format!("{key}={}", dir.display()));
            }
            format!("target {target} (-sdk {sdk} -arch {arch})")
        };
        cmd.args(["-configuration", &self.configuration()]);
        // The settings the project's builds take, so the prepare build writes
        // what the arguments above resolve against. Before the fixed ones
        // below, which win: prepare never signs.
        let command_line = self.command_line();
        if let Some(xcconfig) = &command_line.xcconfig {
            cmd.args(["-xcconfig".as_ref(), xcconfig.as_os_str()]);
        }
        cmd.args(
            command_line
                .overrides
                .iter()
                .map(|(k, v)| format!("{k}={v}")),
        )
        // Prepare only needs modules, not a signed/launchable product, and
        // must not stall on validation prompts in a headless run.
        .args([
            "CODE_SIGNING_ALLOWED=NO",
            "-skipMacroValidation",
            "-skipPackagePluginValidation",
        ]);
        (cmd, how)
    }

    /// Build `target` via `xcodebuild` so its dependency `.swiftmodule`s,
    /// header maps and generated inputs land in the DerivedData our search paths
    /// point at — the fallback for closures the `swiftc` fast path can't emit.
    ///
    /// By **scheme** where one builds the target, because a scheme build lays
    /// the whole tree out for us. Where none does, by `-target` with `SYMROOT`
    /// and `OBJROOT` named explicitly: a bare `-target` build otherwise writes
    /// into the project's own `build/` directory rather than the DerivedData the
    /// editor arguments point at, which is the reason this used to skip such a
    /// target outright. Skipping is no longer an option — the header maps a
    /// target's ObjC files need to resolve their imports exist only after a
    /// build of that target.
    fn xcodebuild_prepare(&self, target: &str, stamps: Vec<Option<(u64, SystemTime)>>) -> bool {
        let (mut cmd, how) = self.prepare_command(target);
        self.log(&format!("prepare: building {how} for target {target}"));
        // The build keeps the server's `TMPDIR`, the user's, where SwiftPM's
        // locks are shared, and leaves a `TemporaryDirectory.*` there.
        let leftovers = TmpdirLeftovers::before(&cmd);
        let ran = self.run_prepare_process(&mut cmd);
        let removed = leftovers.remove();
        if removed > 0 {
            self.log(&format!(
                "prepare: removed {removed} TemporaryDirectory.* the build left in TMPDIR"
            ));
        }
        let (status, stderr_buf) = match ran {
            Ok(ran) => ran,
            Err(e) => {
                let detail = format!("could not launch xcodebuild: {e}");
                self.log(&format!("prepare: {target} {detail}"));
                self.record_prepare(target, stamps, false, Some(detail));
                return false;
            }
        };
        match status {
            Some(st) if st.success() => {
                self.log(&format!("prepare: {target} build ok"));
                self.record_prepare(target, stamps, true, None);
                true
            }
            Some(st) => {
                // Best-effort as far as the reply goes — sourcekit-lsp gets
                // whatever modules did build — but recorded, because a target
                // that never prepares is a target whose ObjC files never resolve
                // their imports, and that used to be visible only in a log file
                // nobody knows to read.
                let code = st.code().unwrap_or(-1);
                let stderr = String::from_utf8_lossy(&stderr_buf);
                let tail: String = stderr.lines().rev().take(8).collect::<Vec<_>>().join(" | ");
                self.log(&format!("prepare: {target} build exit={code}: {tail}"));
                self.record_prepare(target, stamps, false, Some(format!("exit={code}: {tail}")));
                false
            }
            None => {
                let detail = "xcodebuild status unavailable".to_string();
                self.log(&format!("prepare: {target} {detail}"));
                self.record_prepare(target, stamps, false, Some(detail));
                false
            }
        }
    }

    /// `SYMROOT` / `OBJROOT` for a `-target` build: the products and
    /// intermediates roots inside the DerivedData the editor arguments resolve
    /// against. Empty when that directory can't be located, which leaves
    /// xcodebuild its own defaults rather than a half-pinned tree.
    fn build_roots(&self) -> Vec<(&'static str, PathBuf)> {
        self.derived_data().map_or_else(Vec::new, |dd| {
            vec![("SYMROOT", dd.products), ("OBJROOT", dd.intermediates)]
        })
    }

    /// Run `cmd` to the end as the prepare worker's process and return how it
    /// exited (`None` when that can't be read) and what it wrote to stderr.
    ///
    /// Spawned rather than run with `output()` so the child stays killable: the
    /// pipe handles are taken first, then the child is parked in
    /// [`PrepareProcess::child`], where shutdown can reach it while this thread
    /// drains the pipes. The spawn happens under the lock shutdown takes, so a
    /// process is either parked before shutdown kills it or never spawned.
    ///
    /// # Errors
    ///
    /// When `cmd` can't be spawned, or the server is shutting down.
    fn run_prepare_process(&self, cmd: &mut Command) -> io::Result<(Option<ExitStatus>, Vec<u8>)> {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let (stdout_pipe, stderr_pipe) = {
            let mut process = self
                .prepare_process
                .lock()
                .map_err(|_| io::Error::other("the prepare lock is poisoned"))?;
            if process.stopped {
                return Err(io::Error::other("the server is shutting down"));
            }
            let mut child = cmd.spawn()?;
            let pipes = (child.stdout.take(), child.stderr.take());
            process.child = Some(child);
            pipes
        };
        // Drain stdout on a helper thread so neither pipe fills and wedges the
        // process; stderr (the interesting stream on failure) drains here.
        let stdout_drain = stdout_pipe.map(|mut s| {
            std::thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = s.read_to_end(&mut sink);
            })
        });
        let mut stderr = Vec::new();
        if let Some(mut s) = stderr_pipe {
            let _ = s.read_to_end(&mut stderr);
        }
        if let Some(t) = stdout_drain {
            let _ = t.join();
        }
        // Reap. Shutdown may have killed the child, but it leaves the handle in
        // the slot for us — `wait` then just collects the killed status.
        let status = self
            .prepare_process
            .lock()
            .ok()
            .and_then(|mut process| process.child.take())
            .and_then(|mut c| c.wait().ok());
        Ok((status, stderr))
    }

    /// The `TMPDIR` for a prepare `swiftc` (see [`PrepareProcess::swiftc_tmp`]),
    /// made on first use. `None` once the server is shutting down, or when the
    /// directory can't be made.
    fn swiftc_tmp(&self) -> Option<PathBuf> {
        let mut process = self.prepare_process.lock().ok()?;
        if process.stopped {
            return None;
        }
        if process.swiftc_tmp.is_none() {
            process.swiftc_tmp = ScratchDir::new("sweetpad-bsp-swiftc").ok();
        }
        process.swiftc_tmp.as_deref().map(Path::to_path_buf)
    }

    /// Stop the prepare worker's processes: kill the one running, if any, wait
    /// for it, and remove the `TMPDIR` the `swiftc`s ran with. Nothing is
    /// spawned after this. Called on shutdown, so `build/exit` neither orphans
    /// a running prepare nor leaves what it wrote in `TMPDIR`. The handle
    /// stays in the slot for the worker to reap, which then collects the
    /// status this wait already did.
    fn kill_prepare(&self) {
        let Ok(mut process) = self.prepare_process.lock() else {
            return;
        };
        process.stopped = true;
        if let Some(child) = process.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // Removed only now that the process is gone: a live one could still
        // be writing there.
        process.swiftc_tmp = None;
    }

    fn sources(&self, params: Option<&Value>) -> Value {
        let requested = self.requested_targets(params);
        let listing = self.listed_sources();
        let items: Vec<Value> = requested
            .iter()
            .map(|target| {
                let files = listing
                    .iter()
                    .find(|(t, _)| t == target)
                    .map_or_else(|| self.source_files(target), |(_, files)| files.clone());
                let sources: Vec<Value> = files
                    .iter()
                    .map(|p| json!({ "uri": file_uri(p), "kind": 1, "generated": false }))
                    .collect();
                json!({ "target": target_id(target), "sources": sources })
            })
            .collect();
        json!({ "items": items })
    }

    fn inverse_sources(&self, params: Option<&Value>) -> Value {
        let path = params
            .and_then(|p| p.get("textDocument"))
            .and_then(|d| d.get("uri"))
            .and_then(Value::as_str)
            .map(path_from_uri)
            .unwrap_or_default();
        let standardized = project::standardize(&path);
        // Re-read the target list (not the startup snapshot) so files in a
        // target added after `buildTarget/didChange` resolve to an owner.
        let owning: Vec<Value> = self
            .listed_sources()
            .iter()
            .filter(|(_, files)| sources_contain(files, &path, &standardized))
            .map(|(t, _)| target_id(t))
            .collect();
        json!({ "targets": owning })
    }

    fn source_kit_options(&self, params: Option<&Value>) -> Value {
        let Some(params) = params else {
            return Value::Null;
        };
        let uri = params
            .get("textDocument")
            .and_then(|d| d.get("uri"))
            .or_else(|| params.get("uri"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let path = path_from_uri(uri);

        // A header is not a build input — it belongs to no `PBXSourcesBuildPhase`,
        // so no target's source list names it — and its extension tells clang
        // nothing about the dialect inside. Answer it from a translation unit
        // that would include it instead.
        if is_header(&path) {
            return self.header_options(&path);
        }

        // The owning target: the request's `target`, else the first that lists
        // the file.
        let standardized = project::standardize(&path);
        let target = params
            .get("target")
            .and_then(|t| t.get("uri"))
            .and_then(Value::as_str)
            .map(target_name_from_uri)
            .or_else(|| {
                self.listed_sources()
                    .into_iter()
                    .find(|(_, files)| sources_contain(files, &path, &standardized))
                    .map(|(t, _)| t)
            });

        let Some(target) = target else {
            self.log(&format!(
                "sourceKitOptions: no target owns {}",
                path.display()
            ));
            return Value::Null;
        };
        let Some(args) = self.compiler_arguments(&target, &path) else {
            return Value::Null;
        };
        json!({
            "compilerArguments": args,
            "workingDirectory": self.project_dir().to_string_lossy(),
        })
    }

    /// The editor arguments for a header: its companion translation unit's
    /// invocation, retargeted at the header. Everything a header needs to parse
    /// — the sysroot, the include and framework search paths, the language
    /// dialect — is a property of the sources that include it, never of the
    /// header itself, so the companion's arguments are the answer. Returns null
    /// when the project has no clang source to borrow from.
    fn header_options(&self, header: &Path) -> Value {
        let Some((target, companion)) = self.header_companion(header) else {
            self.log(&format!(
                "sourceKitOptions: no clang source to borrow arguments from for {}",
                header.display()
            ));
            return Value::Null;
        };
        let opts = self.editor_options(&target, &self.editor_platform(&target));
        let inv = match build_settings::resolve_file_arguments(&opts, &companion) {
            Ok(inv) => inv,
            Err(e) => {
                self.log(&format!(
                    "resolve failed: target={target} header={} companion={} err={e}",
                    header.display(),
                    companion.display()
                ));
                return Value::Null;
            }
        };
        let mut args = editor_arguments(&inv.arguments);
        // Left to itself clang reads a `.h` as a plain C header and rejects the
        // ObjC or C++ it finds, so name the dialect the companion compiles as.
        // A later `-x` wins over any the companion's own arguments carry.
        args.push("-x".into());
        args.push(header_dialect(&companion).into());
        args.push(header.to_string_lossy().into_owned());
        self.log(&format!(
            "sourceKitOptions: target={target} header={} companion={} args={}",
            header.display(),
            companion.display(),
            args.len(),
        ));
        json!({
            "compilerArguments": args,
            "workingDirectory": self.project_dir().to_string_lossy(),
        })
    }

    /// The translation unit whose arguments stand in for `header`: the sibling
    /// with the same stem (`Foo.h` → `Foo.m`), else any clang source in the same
    /// directory, else the project's first clang source. Nearest first — the
    /// closer the companion, the likelier it shares the header's search paths
    /// and dialect. A `.swift` file is never a companion: it compiles through
    /// swiftc, whose arguments say nothing about how to parse a header.
    fn header_companion(&self, header: &Path) -> Option<(String, PathBuf)> {
        // Source paths are built on a canonicalized project dir while the editor
        // spells the header however the user opened it, so accept a source that
        // sits beside either spelling.
        let standardized = project::standardize(header);
        let dirs: Vec<&Path> = [header.parent(), standardized.parent()]
            .into_iter()
            .flatten()
            .collect();
        let stem = header.file_stem();

        let mut in_dir: Option<(String, PathBuf)> = None;
        let mut anywhere: Option<(String, PathBuf)> = None;
        for target in self.current_targets() {
            for source in self.source_files(&target) {
                if !is_clang_source(&source) {
                    continue;
                }
                let beside = source.parent().is_some_and(|p| dirs.contains(&p));
                if beside && source.file_stem() == stem {
                    return Some((target, source));
                }
                if beside && in_dir.is_none() {
                    in_dir = Some((target.clone(), source.clone()));
                } else if anywhere.is_none() {
                    anywhere = Some((target.clone(), source));
                }
            }
        }
        in_dir.or(anywhere)
    }

    /// The targets named in a `{ targets: [{uri}] }` param, or all targets.
    fn requested_targets(&self, params: Option<&Value>) -> Vec<String> {
        params
            .and_then(|p| p.get("targets"))
            .and_then(Value::as_array)
            .map_or_else(
                || self.current_targets(),
                |arr| {
                    arr.iter()
                        .filter_map(|t| {
                            t.get("uri")
                                .and_then(Value::as_str)
                                .map(target_name_from_uri)
                        })
                        .collect()
                },
            )
    }

    /// Every current target with its source files, except that a file several
    /// targets compile is listed only under the ones [`OwnerRanking`] prefers. sourcekit-lsp reads a file through one of the targets listing
    /// it, the first by target URI, so without this a file shared by an iOS
    /// and a watchOS app reads as whichever name sorts first, and the
    /// `#if os(…)` branch of the app being built goes dead. The arguments for
    /// each target's other files still name every file it compiles.
    fn listed_sources(&self) -> Vec<(String, Vec<PathBuf>)> {
        let mut listing: Vec<(String, Vec<PathBuf>)> = self
            .current_targets()
            .into_iter()
            .map(|target| {
                let files = self.source_files(&target);
                (target, files)
            })
            .collect();
        let mut owners: BTreeMap<&Path, Vec<&str>> = BTreeMap::new();
        for (target, files) in &listing {
            for file in files {
                let entry = owners.entry(file).or_default();
                if !entry.contains(&target.as_str()) {
                    entry.push(target);
                }
            }
        }
        let mut ranking = OwnerRanking::new(self);
        let mut unlisted: BTreeSet<(String, PathBuf)> = BTreeSet::new();
        for (file, targets) in owners.into_iter().filter(|(_, t)| t.len() > 1) {
            let keep = ranking.preferred(&targets);
            for target in targets.into_iter().filter(|t| !keep.iter().any(|k| k == t)) {
                unlisted.insert((target.to_string(), file.to_path_buf()));
            }
        }
        ranking.log_decisions();
        if unlisted.is_empty() {
            return listing;
        }
        for (target, files) in &mut listing {
            files.retain(|f| !unlisted.contains(&(target.clone(), f.clone())));
        }
        listing
    }

    /// The targets the scheme `name` builds: its build entries, then the
    /// targets those depend on. A scheme with no file is one Xcode
    /// autocreates, which builds the target it is named after.
    fn scheme_targets(&self, name: &str) -> (Vec<String>, Vec<String>) {
        let entries: Vec<String> = match scheme::locate(&self.project_path, name) {
            Some(path) => scheme::parse_file(&path)
                .map(|s| {
                    s.build_entries
                        .into_iter()
                        .map(|e| e.buildable.blueprint_name)
                        .collect()
                })
                .unwrap_or_default(),
            None => vec![name.to_string()],
        };
        let mut built = entries.clone();
        for target in &entries {
            let dependencies =
                project::transitive_dependencies(&self.project_for_target(target), target)
                    .unwrap_or_default();
            for dependency in dependencies {
                if !built.contains(&dependency) {
                    built.push(dependency);
                }
            }
        }
        (entries, built)
    }

    fn source_files(&self, target: &str) -> Vec<PathBuf> {
        project::target_source_files(&self.project_for_target(target), target).unwrap_or_default()
    }

    /// The editor compiler arguments for `file` in `target`: the engine's
    /// per-file invocation (a clang file gated to its own language, a `.swift`
    /// file the whole module), reduced to an editor invocation (no build actions
    /// / explicit-module plumbing), with the inputs appended.
    fn compiler_arguments(&self, target: &str, file: &Path) -> Option<Vec<String>> {
        let opts = self.editor_options(target, &self.editor_platform(target));
        let inv = match build_settings::resolve_file_arguments(&opts, file) {
            Ok(inv) => inv,
            Err(e) => {
                self.log(&format!(
                    "resolve failed: target={target} file={} err={e}",
                    file.display()
                ));
                return None;
            }
        };
        let mut args = editor_arguments(&inv.arguments);
        args.extend(inv.input_files);
        self.log(&format!(
            "sourceKitOptions: target={target} file={} tool={} args={}",
            file.display(),
            inv.tool,
            args.len(),
        ));
        Some(args)
    }

    /// The SDK + arch sourcekit-lsp should analyze `target` with: the
    /// selected destination's platform when the target builds for it, as a
    /// target listing `macosx` builds natively for My Mac and a Catalyst one
    /// builds as Catalyst; otherwise the platform its `SDKROOT` or
    /// `SUPPORTED_PLATFORMS` names ([`editor_sdk_for`]). Either way a device
    /// platform reads as its **simulator** (editor-friendly — no
    /// device/signing, and the usual dev build). Arch defaults to the host's
    /// (simulator and macOS builds match the host: arm64 on Apple Silicon,
    /// x86_64 on Intel). `--sdk`/`--arch` flags override, each independently.
    fn editor_platform(&self, target: &str) -> EditorPlatform {
        let arch = self.editor_arch();
        if let Some(sdk) = self.sdk.as_deref() {
            return EditorPlatform {
                sdk: sdk.to_string(),
                arch,
                catalyst: false,
            };
        }
        let (sdkroot, supported) = self.authored_platform(target);
        let destination = self.destination_family();
        let native = destination.filter(|d| platform_families(&sdkroot, &supported).contains(d));
        let catalyst =
            native.is_none() && destination == Some("macosx") && self.builds_as_catalyst(target);
        let sdk = native
            .or(catalyst.then_some("macosx"))
            .unwrap_or_else(|| editor_sdk_for(&sdkroot, &supported));
        self.log(&format!(
            "platform {target}: SDKROOT={sdkroot:?} platforms={supported:?} \
             destination={destination:?} -> sdk={sdk} catalyst={catalyst} arch={arch}"
        ));
        EditorPlatform {
            sdk: sdk.to_string(),
            arch,
            catalyst,
        }
    }

    /// The options that resolve `target` for the editor on `platform`. A
    /// Catalyst target resolves through a Mac run destination.
    fn editor_options(&self, target: &str, platform: &EditorPlatform) -> BuildSettingsOptions {
        let mut opts = self.options_for(target, &platform.sdk, &platform.arch);
        if platform.catalyst {
            opts.destination = mac_destination(&platform.arch);
        }
        opts
    }

    /// The selected destination's platform, named by the SDK the editor reads
    /// it with ([`platform_family`]).
    fn destination_family(&self) -> Option<&'static str> {
        self.live
            .lock()
            .ok()
            .and_then(|l| l.destination_platform.as_deref().and_then(platform_family))
    }

    /// Whether `target` builds for the destination platform `destination`
    /// ([`platform_family`]): natively, or on a Mac as Mac Catalyst.
    fn builds_for(&self, target: &str, destination: &str) -> bool {
        let (sdkroot, supported) = self.authored_platform(target);
        platform_families(&sdkroot, &supported).contains(&destination)
            || (destination == "macosx" && self.builds_as_catalyst(target))
    }

    /// Whether `target` builds as Mac Catalyst for a Mac destination, the
    /// way `xcodebuild -destination platform=macOS` builds it: through the
    /// selected scheme when it builds the target, since that build takes one
    /// variant from its apps, and a framework a "Designed for iPad" app
    /// embeds builds for `iphoneos` although on its own it would take
    /// Catalyst; otherwise the target alone. Read once per project and
    /// `bsp.json` stamps.
    fn builds_as_catalyst(&self, target: &str) -> bool {
        let stamps = self.prepare_stamps();
        if let Ok(cache) = self.catalyst.lock()
            && cache.stamps == stamps
            && let Some(catalyst) = cache.targets.get(target)
        {
            return *catalyst;
        }
        let arch = self.editor_arch();
        let resolve = |scheme: Option<String>| {
            let mut opts = self.options_for(target, "macosx", &arch);
            if scheme.is_some() {
                opts.target = None;
                opts.scheme = scheme;
            }
            opts.destination = mac_destination(&arch);
            opts.keys = Some(vec!["IS_MACCATALYST".to_string()]);
            build_settings::resolve_build_settings(&opts)
                .unwrap_or_default()
                .into_iter()
                .map(|t| {
                    (
                        t.target,
                        t.settings.get("IS_MACCATALYST").is_some_and(|v| v == "YES"),
                    )
                })
                .collect::<BTreeMap<String, bool>>()
        };
        let scheme = self.live.lock().ok().and_then(|l| l.scheme.clone());
        let mut found = scheme.map(|s| resolve(Some(s))).unwrap_or_default();
        if !found.contains_key(target) {
            found.extend(resolve(None));
        }
        let catalyst = found.get(target).copied().unwrap_or(false);
        if let Ok(mut cache) = self.catalyst.lock() {
            if cache.stamps != stamps {
                *cache = CatalystTargets {
                    stamps,
                    targets: BTreeMap::new(),
                };
            }
            cache.targets.extend(found);
            cache.targets.entry(target.to_string()).or_insert(catalyst);
        }
        catalyst
    }

    fn editor_arch(&self) -> String {
        self.arch.clone().unwrap_or_else(|| host_arch().to_string())
    }

    /// `target`'s *authored* `SDKROOT` (e.g. `iphoneos`) and
    /// `SUPPORTED_PLATFORMS`, lowercased. A real `--sdk` replaces SDKROOT with
    /// that SDK's path, but a sentinel the catalog doesn't know leaves it
    /// untouched.
    fn authored_platform(&self, target: &str) -> (String, String) {
        let probe = self.options_for(target, "auto", &self.editor_arch());
        let settings = build_settings::resolve_build_settings(&probe)
            .ok()
            .and_then(|mut t| {
                t.retain(|s| s.target == target);
                t.pop()
            })
            .map(|t| t.settings);
        let read = |k: &str| {
            settings
                .as_ref()
                .and_then(|s| s.get(k))
                .cloned()
                .unwrap_or_default()
                .to_lowercase()
        };
        (read("SDKROOT"), read("SUPPORTED_PLATFORMS"))
    }

    fn options_for(&self, target: &str, sdk: &str, arch: &str) -> BuildSettingsOptions {
        // For a `.xcworkspace` root, resolve *through the workspace* rather than
        // the owning member project: `xcodebuild_prepare` builds with
        // `-workspace`, so DerivedData is keyed by the workspace path, and the
        // resolver only hashes that container when the workspace is declared.
        // A project root prepares with `-project`, which keys it by the
        // project, as the resolver does with no workspace declared.
        let (project, workspace) = if self.is_workspace() {
            (None, Some(self.project_path.clone()))
        } else {
            (Some(self.project_for_target(target)), None)
        };
        let command_line = self.command_line();
        BuildSettingsOptions {
            project,
            workspace,
            scheme: None,
            target: Some(target.to_string()),
            configuration: self.configuration(),
            sdk: sdk.to_string(),
            arch: arch.to_string(),
            destination: None,
            // The project's builds pass these on the command line, and the
            // index has to read the target the way they build it.
            xcconfig: command_line.xcconfig,
            xcode: self.xcode.clone(),
            xcspec_root: None,
            sdksettings_root: None,
            catalog_cache: None,
            derived_data_path: self.derived_data_path.clone(),
            overrides: command_line.overrides,
            // The index must point at the same tree the editor's builds write
            // to, so honour whatever this machine's Xcode is configured with.
            read_xcode_locations: true,
            keys: None,
        }
    }
}

/// Build-only / output-producing flags the editor front end doesn't want:
/// stripping them leaves a parse + type-check invocation against implicit
/// modules (SourceKit manages its own module cache). ⚠️ Refine against real
/// `sourcekit-lsp` in Layer 2 (DOCS.md §8 (BSP server)).
const STRIP_FLAGS: &[&str] = &[
    "-explicit-module-build",
    "-validate-clang-modules-once",
    "-emit-module",
    "-emit-dependencies",
    "-emit-objc-header",
    "-emit-const-values",
    "-c",
    "-experimental-emit-module-separately",
    "-no-emit-module-separately-wmo",
    "-save-temps",
    "-use-frontend-parseable-output",
    "-incremental",
    "-enable-batch-mode",
    "-disable-cmo",
    "-whole-module-optimization",
];

fn editor_arguments(build_args: &[String]) -> Vec<String> {
    build_args
        .iter()
        .filter(|a| !STRIP_FLAGS.contains(&a.as_str()))
        .cloned()
        .collect()
}

fn parse_flags(args: &[String]) -> BTreeMap<String, String> {
    let mut flags = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        if let Some(key) = args[i].strip_prefix("--") {
            if let Some((k, v)) = key.split_once('=') {
                flags.insert(k.to_string(), v.to_string());
                i += 1;
            } else if i + 1 < args.len() {
                flags.insert(key.to_string(), args[i + 1].clone());
                i += 2;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    flags
}

fn target_id(name: &str) -> Value {
    // Percent-encode the name so the id is a syntactically valid URI: a target
    // like `My App` must not emit a raw space — a client that parses and
    // re-serializes the id would normalize it to `My%20App`, which would then
    // fail to round-trip back to a known target name on our side.
    let mut uri = String::from(TARGET_SCHEME);
    percent_encode_into(&mut uri, name);
    json!({ "uri": uri })
}

fn target_name_from_uri(uri: &str) -> String {
    percent_decode(uri.strip_prefix(TARGET_SCHEME).unwrap_or(uri))
}

fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    percent_encode_into(&mut out, &path.to_string_lossy());
    out
}

pub(crate) fn path_from_uri(uri: &str) -> PathBuf {
    PathBuf::from(percent_decode(uri.strip_prefix("file://").unwrap_or(uri)))
}

/// Header extensions an editor opens. None of them name a build input, so a
/// file with one of these never appears in a target's source list.
const HEADER_EXTS: &[&str] = &["h", "hh", "hpp", "hxx", "pch", "inl"];

fn is_header(path: &Path) -> bool {
    extension_of(path).is_some_and(|e| HEADER_EXTS.contains(&e.as_str()))
}

/// Whether `path` is a C-family source, judged by the same extension table that
/// decides how the build compiles it.
fn is_clang_source(path: &Path) -> bool {
    !compiler_args::clang_languages(std::slice::from_ref(&path.to_string_lossy().into_owned()))
        .is_empty()
}

/// The clang `-x` dialect a header should parse as, taken from the companion
/// translation unit's language. Objective-C is the fallback because it accepts
/// plain C too, so a header of unknown provenance still parses.
fn header_dialect(companion: &Path) -> &'static str {
    let langs = compiler_args::clang_languages(std::slice::from_ref(
        &companion.to_string_lossy().into_owned(),
    ));
    match langs.iter().next().map(String::as_str) {
        Some("sourcecode.cpp.objcpp") => "objective-c++-header",
        Some("sourcecode.cpp.cpp") => "c++-header",
        Some("sourcecode.c.c") => "c-header",
        _ => "objective-c-header",
    }
}

/// A path's extension, lowercased — extensions are matched case-insensitively
/// because a case-insensitive filesystem lets a project spell `.h` as `.H`.
fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Whether `sources` names `path`. The editor spells a path however the user
/// opened the file — through a symlinked checkout, or with `..` segments —
/// while a target's source list is built on a canonicalized project dir, so an
/// exact comparison misses files that are plainly there. Compare exactly first
/// (the common case, no syscalls), then by standardized spelling.
fn sources_contain(sources: &[PathBuf], path: &Path, standardized: &Path) -> bool {
    sources.iter().any(|s| s == path)
        || sources
            .iter()
            .any(|s| project::standardize(s) == standardized)
}

/// Append `s` to `out` percent-encoded: RFC 3986 unreserved bytes and `/` pass
/// through, everything else becomes `%XX`.
fn percent_encode_into(out: &mut String, s: &str) {
    for b in s.bytes() {
        match b {
            b'/' | b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
            }
        }
    }
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // Decode on bytes only: slicing `raw` here would panic on a non-char
        // boundary when a client sends `%` followed by unencoded non-ASCII
        // (several BSP clients don't percent-encode), and hex is validated
        // per digit so a stray `%+5` stays literal.
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(hi) = (bytes[i + 1] as char).to_digit(16)
            && let Some(lo) = (bytes[i + 2] as char).to_digit(16)
        {
            // Two hex digits are ≤ 0xFF by construction.
            out.push(u8::try_from(hi * 16 + lo).unwrap_or(u8::MAX));
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Both names an `.xcodeproj` can hold its document under. Stamping the pair
/// rather than the one that exists right now keeps the fingerprint honest
/// across a conversion between the two formats, where the file that carries
/// the project changes name.
fn document_paths(xcodeproj: &Path) -> [PathBuf; 2] {
    [
        xcodeproj.join("project.pbxproj"),
        xcodeproj.join(sweetpad_lib::xcproj::DOCUMENT_NAME),
    ]
}

/// A change fingerprint for a file — `(len, mtime)`, or `None` if it can't be
/// stat'd. Comparing it across polls detects an edit without a notify dependency.
fn file_stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// The value following each occurrence of `flag` in an argv (`-I <dir>` → the
/// dirs). Lets the self-build executor read the products dir and module name
/// back out of the editor arguments instead of recomputing them.
fn arg_values(args: &[String], flag: &str) -> Vec<String> {
    args.iter()
        .zip(args.iter().skip(1))
        .filter(|(a, _)| a.as_str() == flag)
        .map(|(_, v)| v.clone())
        .collect()
}

/// The `xcodebuild -destination 'generic/platform=…'` name for an SDK, used to
/// build a target for the platform the editor analyzes it as.
fn platform_name(sdk: &str) -> &'static str {
    sweetpad_lib::destination::Platform::from_sdk(&sweetpad_lib::project::canonicalize_sdk_base(
        sdk,
    ))
    .map_or("macOS", |p| p.label)
}

impl Server {
    /// Write a JSON-RPC result response (skipped for notifications, which have no
    /// id) and log the full outgoing JSON.
    // `result` is owned: it's moved into the response object (`json!` needs an
    // owned `Value`); the id-less early return — a malformed request — only drops it.
    #[allow(clippy::needless_pass_by_value)]
    fn reply(&self, id: Option<Value>, result: Value) -> Result<(), String> {
        let Some(id) = id else {
            return Ok(());
        };
        let resp = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        self.send(&resp)
    }

    /// Write one JSON-RPC message to the output, holding the lock for the whole
    /// frame so the request loop and the watcher thread never interleave
    /// output.
    fn send(&self, msg: &Value) -> Result<(), String> {
        self.trace(&format!("send: {msg}"));
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        write_message(&mut *out, &msg.to_string())
    }
}

/// The host's arch in Apple naming — the default the editor analyzes for
/// (simulator and macOS builds target the host arch).
fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        _ => "arm64",
    }
}

/// Pick the editor's SDK name for a target from its resolved `SDKROOT` and
/// `SUPPORTED_PLATFORMS`. `SDKROOT` is normally a concrete SDK (`iphoneos`), but
/// a multiplatform target sets it to `auto` and names its platforms in
/// `SUPPORTED_PLATFORMS` — so derive from whichever carries the platform. Device
/// platforms map to their simulator (editor-friendly: no device / signing);
/// anything unrecognized falls back to macOS. Never returns `auto`, which would
/// reach sourcekitd as `-sdk auto` and fail to load a standard library.
fn editor_sdk_for(sdkroot: &str, supported_platforms: &str) -> &'static str {
    let sdkroot = sdkroot.trim().to_lowercase();
    let platform = if sdkroot.is_empty() || sdkroot == "auto" {
        supported_platforms.to_lowercase()
    } else {
        sdkroot
    };
    if platform.contains("iphone") {
        "iphonesimulator"
    } else if platform.contains("appletv") {
        "appletvsimulator"
    } else if platform.contains("watch") {
        "watchsimulator"
    } else if platform.contains("xr") {
        "xrsimulator"
    } else {
        "macosx"
    }
}

/// The SDK the editor reads `platform` (an SDK or platform name) with, as
/// [`editor_sdk_for`] picks it, which names a device platform by its
/// simulator. `None` for a value that names no platform.
fn platform_family(platform: &str) -> Option<&'static str> {
    let platform = platform.trim();
    if platform.is_empty() || platform.eq_ignore_ascii_case("auto") || platform.contains("$(") {
        return None;
    }
    Some(editor_sdk_for(platform, ""))
}

/// The run destination for this Mac, as a build for My Mac names it.
fn mac_destination(arch: &str) -> Option<sweetpad_lib::destination::RunDestination> {
    sweetpad_lib::destination::parse_destination_arg(&format!("platform=macOS,arch={arch}"))
}

/// The platforms a target with `sdkroot` and `supported_platforms` builds
/// for, each named by the SDK the editor reads it with ([`platform_family`]).
fn platform_families(sdkroot: &str, supported_platforms: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    for platform in std::iter::once(sdkroot).chain(supported_platforms.split_whitespace()) {
        if let Some(family) = platform_family(platform)
            && !out.contains(&family)
        {
            out.push(family);
        }
    }
    out
}

/// Picks which of a shared file's targets list it: the ones the selected
/// scheme builds, its own entries before the targets they depend on, then of
/// those left the ones that build for the selected destination's platform.
/// A rule that matches none of them leaves the set as it was, so with nothing
/// selected every target keeps the file. The scheme of an iOS app that embeds
/// its watchOS app builds both, and settles on the iOS app as its own entry.
struct OwnerRanking<'a> {
    server: &'a Server,
    scheme: Option<String>,
    destination: Option<&'static str>,
    /// [`Server::scheme_targets`], read on first use.
    scheme_targets: Option<(Vec<String>, Vec<String>)>,
    /// [`Server::builds_for`] the destination per target, read on first use.
    builds: BTreeMap<String, bool>,
    /// Each set of owners decided, with the targets kept and the number of
    /// files it decided for.
    decisions: BTreeMap<Vec<String>, (Vec<String>, usize)>,
}

impl<'a> OwnerRanking<'a> {
    fn new(server: &'a Server) -> Self {
        let (scheme, destination) = server
            .live
            .lock()
            .map(|l| (l.scheme.clone(), l.destination_platform.clone()))
            .unwrap_or_default();
        Self {
            server,
            scheme,
            destination: destination.as_deref().and_then(platform_family),
            scheme_targets: None,
            builds: BTreeMap::new(),
            decisions: BTreeMap::new(),
        }
    }

    fn preferred(&mut self, owners: &[&str]) -> Vec<String> {
        let key: Vec<String> = owners.iter().map(|t| (*t).to_string()).collect();
        if let Some((keep, files)) = self.decisions.get_mut(&key) {
            *files += 1;
            return keep.clone();
        }
        let mut keep = key.clone();
        if let Some(scheme) = &self.scheme {
            let (entries, built) = self
                .scheme_targets
                .get_or_insert_with(|| self.server.scheme_targets(scheme));
            narrow(&mut keep, |t| entries.iter().any(|e| e == t));
            narrow(&mut keep, |t| built.iter().any(|b| b == t));
        }
        if keep.len() > 1
            && let Some(destination) = self.destination
        {
            let (server, builds) = (self.server, &mut self.builds);
            narrow(&mut keep, |t| {
                *builds
                    .entry(t.to_string())
                    .or_insert_with(|| server.builds_for(t, destination))
            });
        }
        self.decisions.insert(key, (keep.clone(), 1));
        keep
    }

    /// Log each set of owners a rule narrowed, once per [`Server::listed_sources`].
    fn log_decisions(&self) {
        for (owners, (keep, files)) in &self.decisions {
            if keep.len() < owners.len() {
                self.server.log(&format!(
                    "shared sources: {files} file(s) of {owners:?} listed under {keep:?} \
                     (scheme={:?} destination={:?})",
                    self.scheme, self.destination
                ));
            }
        }
    }
}

/// Keep the `targets` that `rule` accepts, unless it accepts none of them.
fn narrow(targets: &mut Vec<String>, mut rule: impl FnMut(&str) -> bool) {
    let kept: Vec<String> = targets.iter().filter(|t| rule(t)).cloned().collect();
    if !kept.is_empty() {
        *targets = kept;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommandLine, LogLevel, ResolvedConfig, Server, Value, derived_data, editor_sdk_for,
        file_uri, parse_flags, path_from_uri, target_name_from_uri, write_config,
    };
    use std::collections::BTreeMap;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicU8;
    use std::sync::{Arc, Mutex};

    /// The protocol output of a server under test, kept where the test can
    /// read it. On stdout its frames would run into cargo's test lines.
    #[derive(Clone, Default)]
    struct Sent(Arc<Mutex<Vec<u8>>>);

    impl Sent {
        fn writer(&self) -> Box<dyn Write + Send> {
            Box::new(self.clone())
        }

        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    impl Write for Sent {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_projects_command_line_reaches_resolution_and_the_prepare_build() {
        let project = format!(
            "{}/fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj",
            env!("SWEETPAD_LIB_DIR")
        );
        let flags = parse_flags(&["--project".to_string(), project.clone()]);
        let command_line = CommandLine {
            xcconfig: Some(PathBuf::from("/work/ci.xcconfig")),
            overrides: vec![
                (
                    "SWIFT_ACTIVE_COMPILATION_CONDITIONS".into(),
                    "STAGING".into(),
                ),
                ("CODE_SIGNING_ALLOWED".into(), "YES".into()),
            ],
        };
        let server = Server::build(
            ResolvedConfig::from_flags(PathBuf::from(&project), &flags),
            None,
            Arc::new(AtomicU8::new(LogLevel::Info as u8)),
            command_line.clone(),
            Sent::default().writer(),
        )
        .unwrap();

        let opts = server.options_for("SweetpadCIMac", "macosx", "arm64");
        assert_eq!(opts.xcconfig, command_line.xcconfig);
        assert_eq!(opts.overrides, command_line.overrides);

        // The prepare build takes them too, ahead of the settings prepare
        // fixes for itself: a prepare never signs.
        let (cmd, _) = server.prepare_command("SweetpadCIMac");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.windows(2)
                .any(|w| w == ["-xcconfig", "/work/ci.xcconfig"]),
            "{args:?}"
        );
        let at = |arg: &str| args.iter().position(|a| a == arg);
        let staging = at("SWIFT_ACTIVE_COMPILATION_CONDITIONS=STAGING").expect("the setting");
        let allowed = at("CODE_SIGNING_ALLOWED=YES").expect("the setting");
        let unsigned = at("CODE_SIGNING_ALLOWED=NO").expect("prepare's own setting");
        assert!(staging < unsigned && allowed < unsigned, "{args:?}");
    }

    /// A copy of the synthetic fixture project, with the embedded
    /// `project.xcworkspace` Xcode writes into every bundle.
    fn project_with_embedded_workspace(tag: &str) -> (crate::scratch::ScratchDir, PathBuf) {
        let scratch = crate::scratch::ScratchDir::new(tag).unwrap();
        let project = scratch.join("SweetpadCIApp.xcodeproj");
        std::fs::create_dir_all(project.join("project.xcworkspace")).unwrap();
        std::fs::copy(
            format!(
                "{}/fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj/project.pbxproj",
                env!("SWEETPAD_LIB_DIR")
            ),
            project.join("project.pbxproj"),
        )
        .unwrap();
        (scratch, project)
    }

    /// A server pointed at a project's embedded workspace serves the project:
    /// its index store is in the `<Project>-<hash of the project>` folder
    /// `xcodebuild -workspace Foo.xcodeproj/project.xcworkspace` builds into,
    /// not a `project-<hash>` folder nothing writes.
    #[test]
    fn an_embedded_workspace_root_serves_its_project() {
        let (_scratch, project) = project_with_embedded_workspace("sweetpad-bsp-stub");
        let stub = project.join("project.xcworkspace");
        let flags = parse_flags(&["--workspace".to_string(), stub.display().to_string()]);
        let server = Server::build(
            ResolvedConfig::from_flags(stub, &flags),
            None,
            Arc::new(AtomicU8::new(LogLevel::Info as u8)),
            CommandLine::default(),
            Sent::default().writer(),
        )
        .unwrap();
        assert_eq!(server.project_path, project);
        assert_eq!(server.projects, std::slice::from_ref(&project));
        assert!(!server.is_workspace());
        if sweetpad_lib::host::home().is_some() {
            let store = server.initialize()["data"]["indexStorePath"]
                .as_str()
                .unwrap()
                .to_string();
            let folder = derived_data::ContainerKey::of(&project).folder_name();
            assert!(store.contains(&format!("/{folder}/")), "{store}");
        }
    }

    /// `bsp init` on a project's embedded workspace writes `buildServer.json`
    /// beside the `.xcodeproj`, where sourcekit-lsp looks, with the server
    /// pointed at the project.
    #[test]
    fn an_embedded_workspace_config_is_written_beside_its_project() {
        let (scratch, project) = project_with_embedded_workspace("sweetpad-bsp-stub-config");
        let stub = project.join("project.xcworkspace");
        write_config(
            &["--workspace".to_string(), stub.display().to_string()],
            &["bsp"],
        )
        .unwrap();
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(scratch.join("buildServer.json")).unwrap(),
        )
        .unwrap();
        let argv: Vec<&str> = written["argv"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let canonical = std::fs::canonicalize(&project).unwrap();
        assert!(
            argv.windows(2)
                .any(|w| w == ["--project", &*canonical.to_string_lossy()]),
            "{argv:?}"
        );
        assert!(!project.join("buildServer.json").exists());
    }

    /// The extension writes `sweetpad.build.args` into `bsp.json` as
    /// `buildArgs`. Their settings and `-xcconfig` reach resolution and the
    /// prepare build under the caller's, follow the file when it changes, and
    /// a file without them has none.
    #[test]
    fn the_command_line_in_bsp_json_is_read_and_follows_the_file() {
        let project = format!(
            "{}/fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj",
            env!("SWEETPAD_LIB_DIR")
        );
        let scratch = crate::scratch::ScratchDir::new("sweetpad-bsp-json").unwrap();
        let config = scratch.join("bsp.json");
        let write = |body: serde_json::Value| {
            std::fs::write(&config, body.to_string()).unwrap();
        };
        write(serde_json::json!({
            "workspacePath": *scratch,
            "projectPath": project,
            "buildArgs": [
                "SWIFT_ACTIVE_COMPILATION_CONDITIONS=STAGING",
                "-destination",
                "platform=macOS",
                "-xcconfig",
                "ci.xcconfig",
                "A=b=c",
            ],
        }));
        let sent = Sent::default();
        let server = Server::build(
            ResolvedConfig::from_file(&config, &BTreeMap::new()).unwrap(),
            Some(config.clone()),
            Arc::new(AtomicU8::new(LogLevel::Info as u8)),
            CommandLine {
                xcconfig: None,
                overrides: vec![("A".into(), "typed".into())],
            },
            sent.writer(),
        )
        .unwrap();
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());

        let opts = server.options_for("SweetpadCIMac", "macosx", "arm64");
        assert_eq!(
            opts.xcconfig,
            Some(sweetpad_lib::project::standardize(&scratch).join("ci.xcconfig"))
        );
        assert_eq!(
            opts.overrides,
            [
                pair("SWIFT_ACTIVE_COMPILATION_CONDITIONS", "STAGING"),
                pair("A", "b=c"),
                pair("A", "typed"),
            ]
        );
        let (cmd, _) = server.prepare_command("SweetpadCIMac");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.iter()
                .any(|a| a == "SWIFT_ACTIVE_COMPILATION_CONDITIONS=STAGING"),
            "{args:?}"
        );

        write(serde_json::json!({
            "workspacePath": *scratch,
            "projectPath": project,
            "buildArgs": ["SWIFT_ACTIVE_COMPILATION_CONDITIONS=BETA"],
        }));
        server.reload_from_file(&config);
        let opts = server.options_for("SweetpadCIMac", "macosx", "arm64");
        assert_eq!(opts.xcconfig, None);
        assert_eq!(
            opts.overrides,
            [
                pair("SWIFT_ACTIVE_COMPILATION_CONDITIONS", "BETA"),
                pair("A", "typed"),
            ]
        );
        // The change tells the client to pull the targets' options again.
        let told = sent.text();
        assert!(
            told.starts_with("Content-Length: ") && told.contains(r#""buildTarget/didChange""#),
            "{told}"
        );

        write(serde_json::json!({ "workspacePath": *scratch, "projectPath": project }));
        server.reload_from_file(&config);
        let opts = server.options_for("SweetpadCIMac", "macosx", "arm64");
        assert_eq!(opts.overrides, [pair("A", "typed")]);
    }

    /// The extension's builds run `xcodebuild` in the workspace folder, which
    /// `xcodebuild` knows by its physical path. Through a symlinked folder, a
    /// relative `-xcconfig ../ci.xcconfig` in `buildArgs` is the real
    /// folder's sibling, the way the CLI reads its arguments. A
    /// `-derivedDataPath` there leaves DerivedData to `derivedDataPath`.
    #[test]
    fn build_args_paths_are_read_from_the_physical_workspace_folder() {
        let project = format!(
            "{}/fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj",
            env!("SWEETPAD_LIB_DIR")
        );
        let scratch = crate::scratch::ScratchDir::new("sweetpad-bsp-json-link").unwrap();
        let real = scratch.join("real/app");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(scratch.join("elsewhere")).unwrap();
        let link = scratch.join("elsewhere/link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let config = scratch.join("bsp.json");
        std::fs::write(
            &config,
            serde_json::json!({
                "workspacePath": link,
                "projectPath": project,
                "buildArgs": ["-xcconfig", "../ci.xcconfig", "-derivedDataPath", "dd"],
            })
            .to_string(),
        )
        .unwrap();
        let resolved = ResolvedConfig::from_file(&config, &BTreeMap::new()).unwrap();
        let physical = sweetpad_lib::project::standardize(&real);
        assert_eq!(
            resolved.command_line.xcconfig,
            Some(physical.parent().unwrap().join("ci.xcconfig"))
        );
        assert_eq!(resolved.derived_data_path, None);
    }

    /// A `buildArgs` that ends with a flag waiting for its value fails the
    /// extension's builds. The index warns and reads the rest, the copy of the
    /// flag before it included, the way the CLI reads the same arguments.
    #[test]
    fn a_trailing_flag_in_bsp_json_is_warned_about_and_left_out() {
        let project = format!(
            "{}/fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj",
            env!("SWEETPAD_LIB_DIR")
        );
        let scratch = crate::scratch::ScratchDir::new("sweetpad-bsp-dangling").unwrap();
        let config = scratch.join("bsp.json");
        let log = scratch.join("bsp.log");
        std::fs::write(
            &config,
            serde_json::json!({
                "workspacePath": *scratch,
                "projectPath": project,
                "logPath": log,
                "buildArgs": ["-xcconfig", "ci.xcconfig", "FOO=1", "-xcconfig"],
            })
            .to_string(),
        )
        .unwrap();
        let server = Server::build(
            ResolvedConfig::from_file(&config, &BTreeMap::new()).unwrap(),
            Some(config.clone()),
            Arc::new(AtomicU8::new(LogLevel::Info as u8)),
            CommandLine::default(),
            Sent::default().writer(),
        )
        .unwrap();

        let opts = server.options_for("SweetpadCIMac", "macosx", "arm64");
        assert_eq!(
            opts.xcconfig,
            Some(sweetpad_lib::project::standardize(&scratch).join("ci.xcconfig"))
        );
        assert_eq!(opts.overrides, [("FOO".to_string(), "1".to_string())]);
        let logged = std::fs::read_to_string(&log).unwrap();
        assert!(
            logged.contains(
                "ignoring '-xcconfig' at the end of buildArgs: it has no value, and \
                 xcodebuild refuses it"
            ),
            "{logged}"
        );
    }

    /// A server over the shared-sources fixture, `bsp.json` written with
    /// `scheme` and `destination`, and the `bsp.json` to rewrite it through.
    fn shared_sources_server(
        scratch: &Path,
        scheme: Option<&str>,
        destination: Option<&str>,
    ) -> (Server, Sent, PathBuf) {
        let config = scratch.join("bsp.json");
        write_shared_sources_config(&config, scheme, destination);
        let sent = Sent::default();
        let server = Server::build(
            ResolvedConfig::from_file(&config, &BTreeMap::new()).unwrap(),
            Some(config.clone()),
            Arc::new(AtomicU8::new(LogLevel::Info as u8)),
            CommandLine::default(),
            sent.writer(),
        )
        .unwrap();
        (server, sent, config)
    }

    fn write_shared_sources_config(config: &Path, scheme: Option<&str>, destination: Option<&str>) {
        let project = format!(
            "{}/fixtures/_synthetic-shared-sources/project/SharedSources.xcodeproj",
            env!("SWEETPAD_LIB_DIR")
        );
        let body = serde_json::json!({
            "workspacePath": config.parent().unwrap(),
            "projectPath": project,
            "scheme": scheme,
            "destinationPlatform": destination,
        });
        std::fs::write(config, body.to_string()).unwrap();
    }

    /// The targets `buildTarget/sources` lists a file ending in `file` under,
    /// sorted.
    fn listed_under(server: &Server, file: &str) -> Vec<String> {
        let reply = server.sources(None);
        let mut targets: Vec<String> = reply["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| {
                item["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s["uri"].as_str().unwrap().ends_with(file))
            })
            .map(|item| target_name_from_uri(item["target"]["uri"].as_str().unwrap()))
            .collect();
        targets.sort();
        targets
    }

    /// sourcekit-lsp reads a file through the first target by URI that lists
    /// it, so a file an iOS and a watchOS app share is listed only under the
    /// one being built: the selected scheme's, then the selected
    /// destination's. A rule that picks neither leaves both.
    #[test]
    fn a_shared_file_is_listed_under_the_target_being_built() {
        let scratch = crate::scratch::ScratchDir::new("sweetpad-bsp-shared").unwrap();
        let cases: [(Option<&str>, Option<&str>, &[&str]); 7] = [
            (None, None, &["WatchApp", "iOSApp"]),
            // Its own entry, over the watchOS app it embeds and so also builds.
            (Some("iOSApp"), None, &["iOSApp"]),
            // A scheme Xcode autocreates builds the target it is named after.
            (Some("WatchApp"), None, &["WatchApp"]),
            (Some("iOSApp"), Some("watchsimulator"), &["iOSApp"]),
            (None, Some("watchsimulator"), &["WatchApp"]),
            // A device destination picks the target its simulator would.
            (None, Some("iphoneos"), &["iOSApp"]),
            (Some("Elsewhere"), Some("watchos"), &["WatchApp"]),
        ];
        for (scheme, destination, expected) in cases {
            let (server, _, _) = shared_sources_server(&scratch, scheme, destination);
            assert_eq!(
                listed_under(&server, "/Shared/Shared.swift"),
                expected,
                "scheme={scheme:?} destination={destination:?}"
            );
            // A file only one target compiles stays listed under it.
            assert_eq!(
                listed_under(&server, "/Watch/WatchMain.swift"),
                ["WatchApp"]
            );
            assert_eq!(listed_under(&server, "/Phone/PhoneApp.swift"), ["iOSApp"]);
        }
    }

    /// A destination picked while the server runs moves the shared file,
    /// tells the client to pull the targets again, and answers the requests
    /// that name no target from the target now listing it.
    #[test]
    fn a_new_destination_moves_a_shared_file_to_its_target() {
        let scratch = crate::scratch::ScratchDir::new("sweetpad-bsp-shared-live").unwrap();
        let (server, sent, config) = shared_sources_server(&scratch, None, Some("iphonesimulator"));
        assert_eq!(listed_under(&server, "/Shared/Shared.swift"), ["iOSApp"]);

        write_shared_sources_config(&config, None, Some("watchsimulator"));
        server.reload_from_file(&config);
        assert!(
            sent.text().contains(r#""buildTarget/didChange""#),
            "{}",
            sent.text()
        );
        assert_eq!(listed_under(&server, "/Shared/Shared.swift"), ["WatchApp"]);

        let shared = format!(
            "{}/fixtures/_synthetic-shared-sources/project/Shared/Shared.swift",
            env!("SWEETPAD_LIB_DIR")
        );
        let document =
            serde_json::json!({ "textDocument": { "uri": file_uri(Path::new(&shared)) } });
        let owners = server.inverse_sources(Some(&document));
        assert_eq!(
            owners["targets"],
            serde_json::json!([{ "uri": "sweetpad://target/WatchApp" }])
        );
        let options = server.source_kit_options(Some(&document));
        let args: Vec<&str> = options["compilerArguments"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let module = args
            .iter()
            .position(|a| *a == "-module-name")
            .map(|i| args[i + 1]);
        assert_eq!(module, Some("WatchApp"), "{args:?}");
    }

    /// A target that builds for several platforms reads as the selected
    /// destination's platform when it builds for it, as a target listing
    /// `macosx` builds natively for My Mac, and follows the destination as it
    /// changes. With a destination it doesn't build for, or none, it reads as
    /// the platform it authors, and `--sdk` wins over both.
    #[test]
    fn a_multiplatform_target_reads_as_the_selected_destination() {
        let project = format!(
            "{}/fixtures/_synthetic-multiplatform/project/MultiPlatformApp.xcodeproj",
            env!("SWEETPAD_LIB_DIR")
        );
        let scratch = crate::scratch::ScratchDir::new("sweetpad-bsp-destination").unwrap();
        let config = scratch.join("bsp.json");
        let write = |destination: Option<&str>| {
            let body = serde_json::json!({
                "workspacePath": *scratch,
                "projectPath": project,
                "destinationPlatform": destination,
            });
            std::fs::write(&config, body.to_string()).unwrap();
        };
        let server_for = |destination: Option<&str>, sdk: Option<&str>| {
            write(destination);
            let flags: BTreeMap<String, String> = sdk
                .map(|s| ("sdk".to_string(), s.to_string()))
                .into_iter()
                .collect();
            Server::build(
                ResolvedConfig::from_file(&config, &flags).unwrap(),
                Some(config.clone()),
                Arc::new(AtomicU8::new(LogLevel::Info as u8)),
                CommandLine::default(),
                Sent::default().writer(),
            )
            .unwrap()
        };
        for (destination, expected) in [
            (None, "iphonesimulator"),
            (Some("macosx"), "macosx"),
            (Some("iphoneos"), "iphonesimulator"),
            (Some("watchsimulator"), "iphonesimulator"),
        ] {
            let sdk = server_for(destination, None)
                .editor_platform("MultiPlatformApp")
                .sdk;
            assert_eq!(sdk, expected, "destination={destination:?}");
        }
        let sdk = server_for(Some("macosx"), Some("iphonesimulator"))
            .editor_platform("MultiPlatformApp")
            .sdk;
        assert_eq!(sdk, "iphonesimulator");

        let server = server_for(Some("iphonesimulator"), None);
        write(Some("macosx"));
        server.reload_from_file(&config);
        let probe = format!(
            "{}/fixtures/_synthetic-multiplatform/project/Sources/Probe.swift",
            env!("SWEETPAD_LIB_DIR")
        );
        let args = server
            .compiler_arguments("MultiPlatformApp", Path::new(&probe))
            .unwrap();
        let triple = args
            .iter()
            .position(|a| a == "-target")
            .map(|i| &args[i + 1]);
        assert!(
            triple.is_some_and(|t| t.contains("-apple-macos")),
            "{args:?}"
        );

        // An iOS-only target runs on a Mac as Designed for iPad, an iOS build.
        let (server, _, _) = shared_sources_server(&scratch, None, Some("macosx"));
        assert_eq!(server.editor_platform("iOSApp").sdk, "iphonesimulator");
        assert_eq!(server.editor_platform("WatchApp").sdk, "watchsimulator");
    }

    /// A Catalyst target reads as Catalyst for a Mac destination, iOS code on
    /// the macOS SDK. Which targets build that way follows the selected
    /// scheme, as the build does: a framework the "Designed for iPad" app
    /// embeds builds for `iphoneos` with it, and as Catalyst with the
    /// Catalyst app or on its own.
    #[test]
    fn a_catalyst_target_reads_as_catalyst_for_a_mac_destination() {
        let root = format!(
            "{}/fixtures/_synthetic-destination-platforms/xcode-27.0.0/project/DestPlatforms",
            env!("SWEETPAD_LIB_DIR")
        );
        let scratch = crate::scratch::ScratchDir::new("sweetpad-bsp-catalyst").unwrap();
        let config = scratch.join("bsp.json");
        let server_for = |scheme: Option<&str>, destination: Option<&str>| {
            let body = serde_json::json!({
                "workspacePath": *scratch,
                "projectPath": format!("{root}/DestPlatforms.xcodeproj"),
                "scheme": scheme,
                "destinationPlatform": destination,
            });
            std::fs::write(&config, body.to_string()).unwrap();
            Server::build(
                ResolvedConfig::from_file(&config, &BTreeMap::new()).unwrap(),
                Some(config.clone()),
                Arc::new(AtomicU8::new(LogLevel::Info as u8)),
                CommandLine::default(),
                Sent::default().writer(),
            )
            .unwrap()
        };
        let read = |server: &Server, target: &str| {
            let platform = server.editor_platform(target);
            (platform.sdk, platform.catalyst)
        };
        let catalyst = ("macosx".to_string(), true);
        let ios = ("iphonesimulator".to_string(), false);

        let server = server_for(Some("CatApp"), Some("macosx"));
        assert_eq!(read(&server, "CatApp"), catalyst);
        assert_eq!(read(&server, "IPadKit"), catalyst);
        assert_eq!(read(&server, "IPadApp"), ios);
        assert_eq!(read(&server, "MacHelper"), ("macosx".to_string(), false));
        let source = format!("{root}/Sources/Kit/Kit.swift");
        let args = server
            .compiler_arguments("IPadKit", Path::new(&source))
            .unwrap();
        let triple = args
            .iter()
            .position(|a| a == "-target")
            .map(|i| &args[i + 1]);
        assert!(triple.is_some_and(|t| t.ends_with("-macabi")), "{args:?}");

        let server = server_for(Some("IPadApp"), Some("macosx"));
        assert_eq!(read(&server, "IPadKit"), ios);
        assert_eq!(read(&server, "CatApp"), catalyst);

        let server = server_for(None, Some("macosx"));
        assert_eq!(read(&server, "IPadKit"), catalyst);

        let server = server_for(Some("CatApp"), Some("iphonesimulator"));
        assert_eq!(read(&server, "CatApp"), ios);
    }

    /// A `-configuration` in `buildArgs` replaces the one the extension picks
    /// on its builds' command line, so the index resolves that configuration,
    /// at startup and when the file changes, and the prepare build takes it.
    #[test]
    fn a_configuration_in_build_args_is_the_one_the_index_resolves() {
        let project = format!(
            "{}/fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj",
            env!("SWEETPAD_LIB_DIR")
        );
        let scratch = crate::scratch::ScratchDir::new("sweetpad-bsp-json-config").unwrap();
        let config = scratch.join("bsp.json");
        let write = |build_args: &[&str]| {
            let body = serde_json::json!({
                "workspacePath": *scratch,
                "projectPath": project,
                "configuration": "Debug",
                "buildArgs": build_args,
            });
            std::fs::write(&config, body.to_string()).unwrap();
        };
        write(&["-quiet", "-configuration", "Release"]);
        let sent = Sent::default();
        let server = Server::build(
            ResolvedConfig::from_file(&config, &BTreeMap::new()).unwrap(),
            Some(config.clone()),
            Arc::new(AtomicU8::new(LogLevel::Info as u8)),
            CommandLine::default(),
            sent.writer(),
        )
        .unwrap();
        let resolved = |server: &Server| {
            let opts = server.options_for("SweetpadCIMac", "macosx", "arm64");
            let settings = crate::build_settings::resolve_build_settings(&opts)
                .unwrap()
                .pop()
                .unwrap()
                .settings;
            (opts.configuration, settings["CONFIGURATION"].clone())
        };
        assert_eq!(resolved(&server), ("Release".into(), "Release".into()));
        let (cmd, _) = server.prepare_command("SweetpadCIMac");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.windows(2).any(|w| w == ["-configuration", "Release"]),
            "{args:?}"
        );

        write(&["-quiet"]);
        server.reload_from_file(&config);
        assert_eq!(resolved(&server), ("Debug".into(), "Debug".into()));
    }

    #[test]
    fn path_from_uri_survives_unencoded_non_ascii_after_percent() {
        // A `%` followed within two bytes by a multi-byte char used to panic
        // (str slice off a char boundary) and kill the whole BSP server.
        assert_eq!(
            path_from_uri("file:///tmp/100%€/x.swift"),
            std::path::PathBuf::from("/tmp/100%€/x.swift")
        );
        // Normal decoding still works, and per-digit hex validation keeps a
        // stray `%+5` literal instead of decoding it.
        assert_eq!(
            path_from_uri("file:///a%20b/c.swift"),
            std::path::PathBuf::from("/a b/c.swift")
        );
        assert_eq!(
            path_from_uri("file:///x%+5y"),
            std::path::PathBuf::from("/x%+5y")
        );
    }

    #[test]
    fn editor_sdk_from_concrete_sdkroot() {
        assert_eq!(editor_sdk_for("iphoneos", ""), "iphonesimulator");
        assert_eq!(editor_sdk_for("macosx", ""), "macosx");
        assert_eq!(editor_sdk_for("appletvos", ""), "appletvsimulator");
        assert_eq!(editor_sdk_for("watchos", ""), "watchsimulator");
        assert_eq!(editor_sdk_for("xros", ""), "xrsimulator");
    }

    #[test]
    fn editor_sdk_from_supported_platforms_when_auto() {
        // The IceCubesApp case: SDKROOT = auto, platform comes from SUPPORTED_PLATFORMS.
        assert_eq!(
            editor_sdk_for("auto", "iphoneos iphonesimulator xros xrsimulator"),
            "iphonesimulator"
        );
        assert_eq!(editor_sdk_for("auto", "xros xrsimulator"), "xrsimulator");
        // Empty SDKROOT behaves like auto.
        assert_eq!(
            editor_sdk_for("", "appletvos appletvsimulator"),
            "appletvsimulator"
        );
        // No usable info → macOS default (never `auto`, which breaks stdlib loading).
        assert_eq!(editor_sdk_for("auto", ""), "macosx");
    }
}
