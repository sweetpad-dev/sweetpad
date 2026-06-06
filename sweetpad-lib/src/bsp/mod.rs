//! The Build Server Protocol server (`sweetpad-lib bsp`) — see `PLAN_BSP.md`.
//!
//! Speaks BSP (JSON-RPC over stdio) to `sourcekit-lsp`, answering the questions
//! that drive editor intelligence: what targets exist, what files each contains,
//! and the compiler arguments for a file. The argv comes from the resolver/
//! generator core (`build_settings::resolve_compiler_arguments`), so it's derived
//! from the project, not parsed out of a build log.
//!
//! This is the walking-skeleton scope: the core requests, per-**target** argv
//! (⚠️ per-file later — see `PLAN_BSP.md`), no `buildTarget/prepare` yet (v2).

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::build_context::BuildContext;
use crate::build_settings::{self, BuildSettingsOptions};
use crate::project;

/// Write a `buildServer.json` so `sourcekit-lsp` discovers and launches this
/// server. Its `argv` points at the current executable + the same `bsp` flags,
/// dropped into the workspace root (the `.xcodeproj`'s parent, or `--output`).
pub fn write_config(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let project = flags.get("project").ok_or("config: --project <path.xcodeproj> is required")?;
    let project_abs = std::fs::canonicalize(project).map_err(|e| format!("--project: {e}"))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    let mut server_argv = vec![
        exe.to_string_lossy().into_owned(),
        "bsp".into(),
        "--project".into(),
        project_abs.to_string_lossy().into_owned(),
    ];
    for (flag, key) in [("--xcode", "xcode"), ("--derived-data-path", "derived-data-path")] {
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
        || project_abs.parent().unwrap_or_else(|| Path::new(".")).join("buildServer.json"),
        PathBuf::from,
    );
    let body = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&out, format!("{body}\n")).map_err(|e| format!("write {}: {e}", out.display()))?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

/// Run the BSP server loop over stdin/stdout until EOF or `build/exit`.
pub fn run(args: &[String]) -> Result<(), String> {
    let server = Server::from_args(args)?;
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();

    while let Some(msg) = read_message(&mut reader)? {
        let Ok(req) = serde_json::from_str::<Value>(&msg) else {
            continue;
        };
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let id = req.get("id").cloned();
        let params = req.get("params");
        server.log(&format!("recv: {msg}"));
        match method {
            "build/initialize" => server.reply(&mut writer, id, server.initialize())?,
            "build/initialized" => {}
            "workspace/buildTargets" => server.reply(&mut writer, id, server.build_targets())?,
            "buildTarget/sources" => server.reply(&mut writer, id, server.sources(params))?,
            "buildTarget/inverseSources" => {
                server.reply(&mut writer, id, server.inverse_sources(params))?;
            }
            "textDocument/sourceKitOptions" => {
                server.reply(&mut writer, id, server.source_kit_options(params))?;
            }
            "build/shutdown" | "shutdown" => server.reply(&mut writer, id, Value::Null)?,
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
                    server.log(&format!("send: {resp}"));
                    write_message(&mut writer, &resp.to_string())?;
                }
            }
        }
    }
    Ok(())
}

struct Server {
    project_path: PathBuf,
    configuration: String,
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
}

const TARGET_SCHEME: &str = "sweetpad://target/";
const LANGUAGE_IDS: [&str; 5] = ["swift", "objective-c", "objective-cpp", "c", "cpp"];

impl Server {
    fn from_args(args: &[String]) -> Result<Self, String> {
        let flags = parse_flags(args);
        let project_path = flags
            .get("project")
            .map(PathBuf::from)
            .ok_or("bsp: --project <path.xcodeproj> is required")?;
        let ctx = BuildContext::open(&project_path).map_err(|e| format!("open project: {e}"))?;
        let targets: Vec<String> = ctx.project.targets.iter().map(|t| t.name.clone()).collect();
        let log = std::env::var_os("SWEETPAD_BSP_LOG")
            .and_then(|p| OpenOptions::new().create(true).append(true).open(p).ok())
            .map(Mutex::new);
        let server = Server {
            project_path,
            configuration: flags.get("configuration").cloned().unwrap_or_else(|| "Debug".into()),
            sdk: flags.get("sdk").cloned(),
            arch: flags.get("arch").cloned(),
            xcode: flags.get("xcode").map(PathBuf::from),
            derived_data_path: flags.get("derived-data-path").map(PathBuf::from),
            targets,
            log,
        };
        server.log(&format!(
            "start: project={} xcode={:?} dd={:?} targets={:?}",
            server.project_path.display(),
            server.xcode,
            server.derived_data_path,
            server.targets,
        ));
        Ok(server)
    }

    /// Append a timestamped line to the debug log (no-op unless `SWEETPAD_BSP_LOG`
    /// is set). Epoch-millis timestamps keep it dependency-free.
    fn log(&self, msg: &str) {
        if let Some(file) = &self.log
            && let Ok(mut file) = file.lock()
        {
            let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
            let _ = writeln!(file, "[{ms}] {msg}");
            let _ = file.flush();
        }
    }

    fn project_dir(&self) -> &Path {
        self.project_path.parent().unwrap_or_else(|| Path::new("."))
    }

    fn initialize(&self) -> Value {
        // Advertise the per-file options extension, and — when we can locate the
        // build's DerivedData — its index store, so sourcekit-lsp does project-wide
        // navigation (definition / references) from the index-while-building data.
        let mut data = json!({ "sourceKitOptionsProvider": true });
        if let Some(dd) = self.derived_data_dir() {
            data["indexStorePath"] = json!(dd.join("Index.noindex/DataStore").to_string_lossy());
            data["indexDatabasePath"] = json!(dd.join("Index.noindex/IndexDatabase").to_string_lossy());
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
        let abs = std::fs::canonicalize(&self.project_path).ok()?;
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
        let targets: Vec<Value> = self
            .targets
            .iter()
            .map(|name| {
                json!({
                    "id": target_id(name),
                    "displayName": name,
                    "baseDirectory": base,
                    "tags": [],
                    "languageIds": LANGUAGE_IDS,
                    "dependencies": [],
                    "capabilities": { "canCompile": true, "canTest": false, "canRun": false, "canDebug": false },
                })
            })
            .collect();
        json!({ "targets": targets })
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
        let owning: Vec<Value> = self
            .targets
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
                self.targets
                    .iter()
                    .find(|t| self.source_files(t).contains(&path))
                    .cloned()
            });

        let Some(target) = target else {
            self.log(&format!("sourceKitOptions: no target owns {}", path.display()));
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
                || self.targets.clone(),
                |arr| {
                    arr.iter()
                        .filter_map(|t| {
                            t.get("uri").and_then(Value::as_str).map(target_name_from_uri)
                        })
                        .collect()
                },
            )
    }

    fn source_files(&self, target: &str) -> Vec<PathBuf> {
        project::target_source_files(&self.project_path, target).unwrap_or_default()
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
                self.log(&format!("resolve failed: target={target} file={} err={e}", file.display()));
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
    /// dev build), defaulting to macOS. Arch is arm64 (Apple Silicon: simulator,
    /// device, and macOS are all arm64). `--sdk`/`--arch` flags override.
    fn editor_platform(&self, target: &str) -> (String, String) {
        if let (Some(sdk), Some(arch)) = (self.sdk.as_deref(), self.arch.as_deref()) {
            return (sdk.to_string(), arch.to_string());
        }
        // Read the target's *authored* SDKROOT (e.g. `iphoneos`): a real `--sdk`
        // replaces SDKROOT with that SDK's path, but a sentinel the catalog
        // doesn't know leaves it untouched. Map the platform to its simulator.
        let probe = self.options_for(target, "auto", "arm64");
        let sdkroot = build_settings::resolve_build_settings(&probe)
            .ok()
            .and_then(|mut t| {
                t.retain(|s| s.target == target);
                t.pop()
            })
            .and_then(|t| t.settings.get("SDKROOT").cloned())
            .unwrap_or_default()
            .to_lowercase();
        let sdk = if sdkroot.contains("iphone") {
            "iphonesimulator"
        } else if sdkroot.contains("appletv") {
            "appletvsimulator"
        } else if sdkroot.contains("watch") {
            "watchsimulator"
        } else if sdkroot.contains("xr") {
            "xrsimulator"
        } else {
            "macosx"
        };
        self.log(&format!("platform {target}: SDKROOT={sdkroot:?} -> sdk={sdk} arch=arm64"));
        (sdk.to_string(), "arm64".to_string())
    }

    fn options_for(&self, target: &str, sdk: &str, arch: &str) -> BuildSettingsOptions {
        BuildSettingsOptions {
            project: Some(self.project_path.clone()),
            workspace: None,
            scheme: None,
            target: Some(target.to_string()),
            configuration: self.configuration.clone(),
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
/// `sourcekit-lsp` in Layer 2 (PLAN_BSP.md).
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
    build_args.iter().filter(|a| !STRIP_FLAGS.contains(&a.as_str())).cloned().collect()
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

/// Read one `Content-Length`-framed JSON-RPC message. `Ok(None)` on clean EOF.
fn read_message(reader: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok();
        }
    }
    let len = content_length.ok_or("message without Content-Length")?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn write_message(writer: &mut impl Write, body: &str) -> Result<(), String> {
    write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len()).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}

impl Server {
    /// Write a JSON-RPC result response (skipped for notifications, which have no
    /// id) and log the full outgoing JSON.
    // `result` is owned: it's moved into the response object (`json!` needs an
    // owned `Value`); the id-less early return — a malformed request — only drops it.
    #[allow(clippy::needless_pass_by_value)]
    fn reply(&self, writer: &mut impl Write, id: Option<Value>, result: Value) -> Result<(), String> {
        let Some(id) = id else {
            return Ok(());
        };
        let resp = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        self.log(&format!("send: {resp}"));
        write_message(writer, &resp.to_string())
    }
}
