//! The Build Server Protocol server (`sweetpad-lib bsp`) — see `DOCS.md` §8 (BSP server).
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
mod framing;

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::build_context::BuildContext;
use crate::build_settings::{self, BuildSettingsOptions};
use crate::project;
use control::{LogLevel, TelemetryServer};
use framing::{read_message, write_message};

/// Write a `buildServer.json` so `sourcekit-lsp` discovers and launches this
/// server. Its `argv` points at the current executable + the same `bsp` flags,
/// dropped into the workspace root (the `.xcodeproj`'s parent, or `--output`).
pub fn write_config(args: &[String]) -> Result<(), String> {
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
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    let mut server_argv = vec![
        exe.to_string_lossy().into_owned(),
        "bsp".into(),
        root_flag.into(),
        root_abs.to_string_lossy().into_owned(),
    ];
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
        "name": "sweetpad-lib",
        "version": env!("CARGO_PKG_VERSION"),
        "bspVersion": "2.2.0",
        "languages": LANGUAGE_IDS,
        "argv": server_argv,
    });

    let out = flags.get("output").map_or_else(
        || {
            root_abs
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("buildServer.json")
        },
        PathBuf::from,
    );
    let body = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&out, format!("{body}\n"))
        .map_err(|e| format!("write {}: {e}", out.display()))?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

/// Run the BSP server loop over stdin/stdout until EOF or `build/exit`.
pub fn run(args: &[String]) -> Result<(), String> {
    let server = Arc::new(Server::resolve(args)?);
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    // Each write locks stdout for one whole frame rather than holding the lock
    // across the loop, so the worker threads (change-watcher, prepare) can
    // interleave their messages between requests.
    let mut watching = false;

    // `buildTarget/prepare` runs `xcodebuild` (seconds-to-minutes), and
    // sourcekit-lsp blocks the requesting target's semantics until our response
    // arrives — so run it on a serialized worker and reply by id when the build
    // finishes, keeping the request loop responsive in the meantime.
    let (prepare_tx, prepare_rx) = mpsc::channel::<PrepareJob>();
    {
        let server = Arc::clone(&server);
        std::thread::spawn(move || {
            while let Ok(job) = prepare_rx.recv() {
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
                    if prepare_tx.send(PrepareJob { id, targets }).is_err() {
                        break; // worker gone
                    }
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
    server.shutdown_telemetry();
    Ok(())
}

struct Server {
    /// The root the server was pointed at: a `.xcodeproj` or a `.xcworkspace`.
    project_path: PathBuf,
    /// The member `.xcodeproj`s — `[project_path]` for a project root, or the
    /// workspace's project refs for a `.xcworkspace`. File→target and settings
    /// resolution iterate these, so each file in a multi-project workspace
    /// resolves against whichever member declares its target.
    projects: Vec<PathBuf>,
    /// Live-updatable config (configuration + scheme), swapped on `bsp/configChanged`.
    live: Mutex<LiveConfig>,
    /// `--sdk` / `--arch` overrides; `None` means infer the platform per target.
    sdk: Option<String>,
    arch: Option<String>,
    xcode: Option<PathBuf>,
    derived_data_path: Option<PathBuf>,
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
    /// The watched `.sweetpad/bsp.json` path. Live scheme/configuration changes
    /// the extension persists there are applied without a restart; immutable
    /// fields are fixed at startup. `None` in `--project` standalone mode.
    config_path: Option<PathBuf>,
    /// Verbosity of the `bsp/log` stream, retunable live via `bsp/setLogLevel`.
    log_level: Arc<AtomicU8>,
}

const TARGET_SCHEME: &str = "sweetpad://target/";
const LANGUAGE_IDS: [&str; 5] = ["swift", "objective-c", "objective-cpp", "c", "cpp"];

/// A queued `buildTarget/prepare`: the request `id` to answer once the build is
/// done, and the target names to prepare.
struct PrepareJob {
    id: Value,
    targets: Vec<String>,
}

/// The portion of config that can change while the server runs — pushed live by
/// the extension as `bsp/configChanged`. Everything else (project/xcode/derived
/// data) is fixed at startup; toolchain/DD changes warrant a restart instead.
#[derive(Clone, PartialEq)]
struct LiveConfig {
    configuration: String,
    scheme: Option<String>,
}

/// The inputs the server needs, however they were obtained — from `--project`
/// flags or the extension's `.sweetpad/bsp.json`.
struct ResolvedConfig {
    project_path: PathBuf,
    configuration: String,
    scheme: Option<String>,
    sdk: Option<String>,
    arch: Option<String>,
    xcode: Option<PathBuf>,
    derived_data_path: Option<PathBuf>,
    /// Debug log file: from `bsp.json`'s `logPath`, else `$SWEETPAD_BSP_LOG`.
    log_path: Option<PathBuf>,
    /// Telemetry socket to bind, assigned by the extension in `bsp.json`. `None`
    /// in `--project` standalone mode (no telemetry).
    socket: Option<PathBuf>,
}

/// The `SWEETPAD_BSP_LOG` env path (used by tests and the standalone paths).
fn env_log() -> Option<PathBuf> {
    std::env::var_os("SWEETPAD_BSP_LOG").map(PathBuf::from)
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
            sdk: flags.get("sdk").cloned(),
            arch: flags.get("arch").cloned(),
            xcode: flags.get("xcode").map(PathBuf::from),
            derived_data_path: flags.get("derived-data-path").map(PathBuf::from),
            log_path: env_log(),
            socket: None,
        }
    }

    /// Build from a `.sweetpad/bsp.json` object (the key schema the extension
    /// writes). Path fields may be written relative to the project root; they're
    /// resolved against `base`. Any explicit flag still wins, so it can be combined
    /// with targeted overrides.
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
        Ok(ResolvedConfig {
            project_path: resolve(project_path),
            configuration: flags
                .get("configuration")
                .cloned()
                .or_else(|| pull("configuration"))
                .unwrap_or_else(|| "Debug".into()),
            scheme: flags.get("scheme").cloned().or_else(|| pull("scheme")),
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
        })
    }

    /// Read and parse `.sweetpad/bsp.json` (written by the extension). Relative
    /// path fields resolve against the project root — the `.sweetpad` dir's
    /// parent — which is what the extension wrote them relative to.
    fn from_file(path: &Path, flags: &BTreeMap<String, String>) -> Result<Self, String> {
        let raw =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let value: Value =
            serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))?;
        let base = path
            .parent()
            .and_then(Path::parent)
            .unwrap_or_else(|| Path::new("."));
        Self::from_json(&value, base, flags)
    }
}

impl Server {
    /// Resolve the server config. An explicit `--project` stays self-contained
    /// (no config file, no telemetry — used by `sweetpad-lib config` and the
    /// tests); otherwise the sole source is `.sweetpad/bsp.json` (written by the
    /// extension), read here and watched for live changes.
    fn resolve(args: &[String]) -> Result<Self, String> {
        let flags = parse_flags(args);
        let log_level = Arc::new(AtomicU8::new(LogLevel::Info as u8));

        if let Some(root) = flags.get("workspace").or_else(|| flags.get("project")) {
            let config = ResolvedConfig::from_flags(PathBuf::from(root), &flags);
            return Self::build(config, None, log_level);
        }

        // Canonicalize to match the extension's realpath'd `workspacePath`
        // (so `/var` vs `/private/var` symlinks don't defeat the lookup).
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
        let config_file = control::discover_config_file(&cwd).ok_or(
            "no .sweetpad/bsp.json found from the working directory (the SweetPad extension or `sweetpad-lib config` writes it)",
        )?;
        let config = ResolvedConfig::from_file(&config_file, &flags)?;
        Self::build(config, Some(config_file), log_level)
    }

    fn build(
        config: ResolvedConfig,
        config_path: Option<PathBuf>,
        log_level: Arc<AtomicU8>,
    ) -> Result<Self, String> {
        // A `.xcworkspace` root expands to its member projects; a `.xcodeproj`
        // root is a one-element list. Targets are the union across members.
        let root = config.project_path.clone();
        let projects: Vec<PathBuf> =
            if root.extension().and_then(|e| e.to_str()) == Some("xcworkspace") {
                crate::workspace::open(&root)
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
            .as_ref()
            .and_then(|p| {
                // Create the parent dir (the per-workspace tmp state root) first so the
                // first write after a cold start doesn't silently drop because the dir
                // isn't there yet.
                if let Some(parent) = p.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                OpenOptions::new().create(true).append(true).open(p).ok()
            })
            .map(Mutex::new);
        let server = Server {
            project_path: config.project_path,
            projects,
            live: Mutex::new(LiveConfig {
                configuration: config.configuration,
                scheme: config.scheme,
            }),
            sdk: config.sdk,
            arch: config.arch,
            xcode: config.xcode,
            derived_data_path: config.derived_data_path,
            targets,
            log,
            telemetry: Mutex::new(None),
            config_path,
            log_level,
        };
        server.bind_telemetry(config.socket.as_deref());
        server.log(&format!(
            "start: project={} xcode={:?} dd={:?} telemetry={} config_watch={:?} targets={:?}",
            server.project_path.display(),
            server.xcode,
            server.derived_data_path,
            server.telemetry.lock().is_ok_and(|t| t.is_some()),
            server.config_path,
            server.targets,
        ));
        Ok(server)
    }

    /// The current build configuration (swapped live when `bsp.json` changes).
    fn configuration(&self) -> String {
        self.live
            .lock()
            .map_or_else(|_| "Debug".into(), |c| c.configuration.clone())
    }

    /// Swap the live config and, when it actually changed, tell the client to
    /// re-pull options via `buildTarget/didChange`. A missing `configuration`
    /// keeps the current value; the diff prevents redundant refresh storms.
    fn apply_config(&self, configuration: Option<&str>, scheme: Option<String>) {
        let next = {
            let Ok(mut live) = self.live.lock() else {
                return;
            };
            let updated = LiveConfig {
                configuration: configuration
                    .map_or_else(|| live.configuration.clone(), str::to_string),
                scheme,
            };
            if updated == *live {
                return;
            }
            *live = updated.clone();
            updated
        };
        self.log(&format!(
            "config changed: configuration={} scheme={:?}",
            next.configuration, next.scheme
        ));
        self.notify_targets_changed();
    }

    /// Re-read `.sweetpad/bsp.json` after a change: apply the volatile selection
    /// (configuration + scheme) live via [`Self::apply_config`], and bind the
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
        let configuration = value.get("configuration").and_then(Value::as_str);
        let scheme = value
            .get("scheme")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.apply_config(configuration, scheme);
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
        if let Some(dd) = self.derived_data_dir() {
            data["indexStorePath"] = json!(dd.join("Index.noindex/DataStore").to_string_lossy());
            data["indexDatabasePath"] =
                json!(dd.join("Index.noindex/IndexDatabase").to_string_lossy());
        }
        json!({
            "displayName": "sweetpad-lib",
            "version": env!("CARGO_PKG_VERSION"),
            "bspVersion": "2.2.0",
            "capabilities": { "languageIds": LANGUAGE_IDS },
            "dataKind": "sourceKit",
            "data": data,
        })
    }

    /// The build's DerivedData directory: the `--derived-data-path` override, else
    /// Xcode's default `~/Library/Developer/Xcode/DerivedData/<name>-<hash>`.
    fn derived_data_dir(&self) -> Option<PathBuf> {
        if let Some(dd) = &self.derived_data_path {
            return Some(dd.clone());
        }
        // Hash the container path *as opened* (absolute, symlinks intact):
        // Xcode keys DerivedData by the path it was launched on, so a project
        // under a symlinked root (`/tmp` → `/private/tmp`) must hash the
        // symlink spelling — `fs::canonicalize` here would compute a
        // different DerivedData dir than the one Xcode/xcodebuild populate.
        let abs = crate::project::absolutize(&self.project_path);
        let name = abs.file_stem()?.to_string_lossy().into_owned();
        let hash = crate::xcode_hash::derived_data_hash(&abs.to_string_lossy());
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library/Developer/Xcode/DerivedData")
                .join(format!("{name}-{hash}")),
        )
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

    /// Watch the project file (and the `.sweetpad/bsp.json` config, when present)
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
        let pbxprojs: Vec<PathBuf> = self
            .projects
            .iter()
            .map(|p| p.join("project.pbxproj"))
            .collect();
        let config = self.config_path.clone();
        std::thread::spawn(move || {
            let stamp_all =
                |paths: &[PathBuf]| paths.iter().map(|p| file_stamp(p)).collect::<Vec<_>>();
            let mut last_pbx = stamp_all(&pbxprojs);
            let mut last_cfg = config.as_deref().map(file_stamp);
            loop {
                std::thread::sleep(interval);
                let now_pbx = stamp_all(&pbxprojs);
                if now_pbx != last_pbx {
                    last_pbx = now_pbx;
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
        let changes: Vec<Value> = self
            .current_targets()
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
        for target in &job.targets {
            self.prepare_target(target);
        }
        self.push_status("ready", None);
        let resp = json!({ "jsonrpc": "2.0", "id": job.id, "result": {} });
        let _ = self.send(&resp);
    }

    /// Prepare `target` so its sources become semantically analyzable: build the
    /// modules it imports. The fast path emits each dependency's module with
    /// `swiftc` directly (no `xcodebuild` process, no link, single arch) when the
    /// whole closure — the target and its transitive deps — is pure Swift;
    /// anything else (packages, C-family, code-gen) falls back to a real
    /// `xcodebuild`, as does a self-build that unexpectedly fails.
    fn prepare_target(&self, target: &str) {
        let proj = self.project_for_target(target);
        // The self-build fast path is single-project: a workspace target's deps
        // can live in another member, so let xcodebuild handle the closure.
        let deps = project::transitive_dependencies(&proj, target).unwrap_or_default();
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
                return;
            }
            self.log(&format!(
                "prepare: {target} self-build failed; falling back to xcodebuild"
            ));
        }
        self.xcodebuild_prepare(target);
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
        let swiftc = self.developer_dir().map_or_else(
            || PathBuf::from("swiftc"),
            |dev| dev.join("Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc"),
        );
        let mut cmd = Command::new(&swiftc);
        if let Some(dev) = self.developer_dir() {
            cmd.env("DEVELOPER_DIR", dev);
        }
        cmd.arg("-emit-module")
            .arg("-emit-module-path")
            .arg(&module_path)
            .args(&args);
        match cmd.output() {
            Ok(out) if out.status.success() => {
                self.log(&format!(
                    "prepare: emitted module {module_name} -> {}",
                    module_path.display()
                ));
                true
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
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

    /// Build `target` via `xcodebuild` so its dependency `.swiftmodule`s and
    /// generated inputs land in the DerivedData our search paths point at —
    /// the fallback for closures the `swiftc` fast path can't emit.
    /// xcodebuild builds by **scheme** (a bare `-target` build doesn't populate
    /// the products dir), so a scheme that builds the target is required.
    fn xcodebuild_prepare(&self, target: &str) {
        let owning = self.project_for_target(target);
        let Some(scheme) = project::scheme_for_target(&owning, target) else {
            self.log(&format!(
                "prepare: no scheme builds target {target}; skipping"
            ));
            return;
        };
        let (sdk, _arch) = self.editor_platform(target);
        let destination = format!("generic/platform={}", platform_name(&sdk));
        let developer = self.developer_dir();
        let xcodebuild = developer.as_ref().map_or_else(
            || PathBuf::from("xcodebuild"),
            |dev| dev.join("usr/bin/xcodebuild"),
        );
        let mut cmd = Command::new(&xcodebuild);
        if let Some(dev) = &developer {
            cmd.env("DEVELOPER_DIR", dev);
        }
        cmd.arg("build");
        if self.is_workspace() {
            cmd.args(["-workspace".as_ref(), self.project_path.as_os_str()]);
        } else {
            cmd.args(["-project".as_ref(), owning.as_os_str()]);
        }
        cmd.args(["-scheme", &scheme])
            .args(["-configuration", &self.configuration()])
            .args(["-destination", &destination])
            // Prepare only needs modules, not a signed/launchable product, and
            // must not stall on validation prompts in a headless run.
            .args([
                "CODE_SIGNING_ALLOWED=NO",
                "-skipMacroValidation",
                "-skipPackagePluginValidation",
            ]);
        if let Some(dd) = &self.derived_data_path {
            // An explicit override needs `-derivedDataPath` (which xcodebuild only
            // accepts with `-scheme`); without it the default DerivedData already
            // matches our search paths.
            cmd.args(["-derivedDataPath".as_ref(), dd.as_os_str()]);
        }
        self.log(&format!(
            "prepare: building scheme {scheme} ({destination}) for target {target}"
        ));
        match cmd.output() {
            Ok(out) => {
                let code = out.status.code().unwrap_or(-1);
                if out.status.success() {
                    self.log(&format!("prepare: {target} build ok"));
                } else {
                    // Best-effort: log the tail and carry on (sourcekit-lsp uses
                    // whatever modules did build).
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let tail: String = stderr.lines().rev().take(8).collect::<Vec<_>>().join(" | ");
                    self.log(&format!("prepare: {target} build exit={code}: {tail}"));
                }
            }
            Err(e) => self.log(&format!(
                "prepare: {target} failed to launch xcodebuild: {e}"
            )),
        }
    }

    fn sources(&self, params: Option<&Value>) -> Value {
        let requested = self.requested_targets(params);
        let items: Vec<Value> = requested
            .iter()
            .map(|target| {
                let sources: Vec<Value> = self
                    .source_files(target)
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
        // Re-read the target list (not the startup snapshot) so files in a
        // target added after `buildTarget/didChange` resolve to an owner.
        let owning: Vec<Value> = self
            .current_targets()
            .iter()
            .filter(|t| self.source_files(t).contains(&path))
            .map(|t| target_id(t))
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

        // The owning target: the request's `target`, else the first whose source
        // list contains the file.
        let target = params
            .get("target")
            .and_then(|t| t.get("uri"))
            .and_then(Value::as_str)
            .map(target_name_from_uri)
            .or_else(|| {
                self.current_targets()
                    .into_iter()
                    .find(|t| self.source_files(t).contains(&path))
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

    fn source_files(&self, target: &str) -> Vec<PathBuf> {
        project::target_source_files(&self.project_for_target(target), target).unwrap_or_default()
    }

    /// The editor compiler arguments for `file` in `target`: the engine's
    /// per-file invocation (a clang file gated to its own language, a `.swift`
    /// file the whole module), reduced to an editor invocation (no build actions
    /// / explicit-module plumbing), with the inputs appended.
    fn compiler_arguments(&self, target: &str, file: &Path) -> Option<Vec<String>> {
        let (sdk, arch) = self.editor_platform(target);
        let opts = self.options_for(target, &sdk, &arch);
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

    /// The SDK + arch sourcekit-lsp should analyze `target` with. We infer the
    /// platform from the target's `SUPPORTED_PLATFORMS` and pick the **simulator**
    /// for device platforms (editor-friendly — no device/signing, and the usual
    /// dev build), defaulting to macOS. Arch defaults to the host's (simulator
    /// and macOS builds match the host: arm64 on Apple Silicon, x86_64 on
    /// Intel). `--sdk`/`--arch` flags override, each independently.
    fn editor_platform(&self, target: &str) -> (String, String) {
        let arch = self.arch.clone().unwrap_or_else(|| host_arch().to_string());
        if let Some(sdk) = self.sdk.as_deref() {
            return (sdk.to_string(), arch);
        }
        // Read the target's *authored* SDKROOT (e.g. `iphoneos`): a real `--sdk`
        // replaces SDKROOT with that SDK's path, but a sentinel the catalog
        // doesn't know leaves it untouched. Map the platform to its simulator.
        let probe = self.options_for(target, "auto", &arch);
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
        let sdkroot = read("SDKROOT");
        let supported = read("SUPPORTED_PLATFORMS");
        let sdk = editor_sdk_for(&sdkroot, &supported);
        self.log(&format!(
            "platform {target}: SDKROOT={sdkroot:?} platforms={supported:?} -> sdk={sdk} arch={arch}"
        ));
        (sdk.to_string(), arch)
    }

    fn options_for(&self, target: &str, sdk: &str, arch: &str) -> BuildSettingsOptions {
        BuildSettingsOptions {
            project: Some(self.project_for_target(target)),
            workspace: None,
            scheme: None,
            target: Some(target.to_string()),
            configuration: self.configuration(),
            sdk: sdk.to_string(),
            arch: arch.to_string(),
            destination: None,
            xcconfig: None,
            xcode: self.xcode.clone(),
            xcspec_root: None,
            sdksettings_root: None,
            catalog_cache: None,
            derived_data_path: self.derived_data_path.clone(),
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
    json!({ "uri": format!("{TARGET_SCHEME}{name}") })
}

fn target_name_from_uri(uri: &str) -> String {
    uri.strip_prefix(TARGET_SCHEME).unwrap_or(uri).to_string()
}

fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
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
    out
}

fn path_from_uri(uri: &str) -> PathBuf {
    let raw = uri.strip_prefix("file://").unwrap_or(uri);
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&raw[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&out).into_owned())
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
    match sdk {
        s if s.starts_with("iphonesimulator") => "iOS Simulator",
        s if s.starts_with("iphoneos") => "iOS",
        s if s.starts_with("appletvsimulator") => "tvOS Simulator",
        s if s.starts_with("appletvos") => "tvOS",
        s if s.starts_with("watchsimulator") => "watchOS Simulator",
        s if s.starts_with("watchos") => "watchOS",
        s if s.starts_with("xrsimulator") => "visionOS Simulator",
        s if s.starts_with("xros") => "visionOS",
        _ => "macOS",
    }
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

    /// Write one JSON-RPC message to stdout, holding the lock for the whole frame
    /// so the request loop and the watcher thread never interleave output.
    fn send(&self, msg: &Value) -> Result<(), String> {
        self.trace(&format!("send: {msg}"));
        let stdout = io::stdout();
        let mut writer = stdout.lock();
        write_message(&mut writer, &msg.to_string())
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

#[cfg(test)]
mod tests {
    use super::editor_sdk_for;

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
