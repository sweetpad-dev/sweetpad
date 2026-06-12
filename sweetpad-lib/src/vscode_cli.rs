//! The `sweetpad vscode` command — a one-shot JSON-RPC client for the SweetPad
//! VS Code extension's CLI control server.
//!
//! A port of the extension's former bundled JS CLI (`src/cli/` → `out/cli.js`),
//! kept behavior-compatible: same method → params mapping, same output shape
//! (pretty JSON, `--raw` to minify, bare strings printed as-is), same error
//! envelope (`{"ok":false,"error":{...}}` on stderr) and exit codes (0 ok,
//! 1 RPC error, 2 client/usage error).
//!
//! Discovery works like `git`: walk up from the cwd to the nearest `.sweetpad/`
//! directory, read its `cli.json` for the running server's Unix socket
//! (last-writer-wins across VS Code windows), then send a single
//! `Content-Length`-framed JSON-RPC 2.0 request over the socket.

use std::collections::BTreeMap;
use std::io::{BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::framing::{read_message, write_message};

const SWEETPAD_DIR: &str = ".sweetpad";
const DEFAULT_TIMEOUT_MS: u64 = 6 * 60 * 1000;

/// Run `sweetpad vscode <method> [args...]` and return the process exit code:
/// 0 success, 1 the server answered with an RPC error, 2 client-side failure
/// (no server, connect/timeout, usage).
#[must_use]
pub fn run(args: &[String]) -> u8 {
    let parsed = parse_argv(args);
    let raw = parsed.raw;
    if parsed.help || parsed.method.is_none() {
        return emit_failure(&usage_envelope(), raw);
    }
    let mapped = method_params(&parsed);

    let socket = match resolve_socket() {
        Ok(socket) => socket,
        Err(envelope) => return emit_failure(&envelope, raw),
    };
    match call_rpc(&socket, &mapped) {
        Ok(Some(result)) => {
            println!("{}", render(&result, raw));
            0
        }
        Ok(None) => 0,
        Err(failure) => {
            eprintln!("{}", render(&failure.envelope, raw));
            failure.code
        }
    }
}

fn emit_failure(envelope: &Value, raw: bool) -> u8 {
    eprintln!("{}", render(envelope, raw));
    2
}

/// JSON for humans by default, `--raw` to minify; a bare string result is
/// printed as-is so it can be piped without quotes.
fn render(value: &Value, raw: bool) -> String {
    match value {
        Value::String(s) => s.clone(),
        v if raw => v.to_string(),
        v => serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
    }
}

// ---------------------------------------------------------------------------
// argv parsing
// ---------------------------------------------------------------------------

/// `--flag value` / `--flag=value` carry a string; a trailing `--flag` (or one
/// followed by another `-…` token) is a boolean toggle.
#[derive(Debug, Clone, PartialEq)]
enum FlagValue {
    True,
    Str(String),
}

#[derive(Debug, Default)]
struct ParsedArgv {
    /// Full dotted method name (e.g. "scheme.list"); a bare first token leaves
    /// this `None` and the dispatcher prints usage.
    method: Option<String>,
    positionals: Vec<String>,
    flags: BTreeMap<String, FlagValue>,
    raw: bool,
    help: bool,
}

fn parse_argv(args: &[String]) -> ParsedArgv {
    let mut result = ParsedArgv::default();
    let mut first_seen = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--help" || arg == "-h" {
            result.help = true;
            i += 1;
        } else if arg == "--raw" {
            result.raw = true;
            i += 1;
        } else if let Some(body) = arg.strip_prefix("--") {
            if let Some((key, value)) = body.split_once('=') {
                result
                    .flags
                    .insert(key.to_string(), FlagValue::Str(value.to_string()));
                i += 1;
            } else if let Some(next) = args.get(i + 1).filter(|n| !n.starts_with('-')) {
                result
                    .flags
                    .insert(body.to_string(), FlagValue::Str(next.clone()));
                i += 2;
            } else {
                result.flags.insert(body.to_string(), FlagValue::True);
                i += 1;
            }
        } else if !first_seen {
            // First non-flag token is the dotted method name; a bare word
            // leaves `method` unset (→ usage). Everything after is a positional.
            first_seen = true;
            if arg.contains('.') {
                result.method = Some(arg.to_string());
            }
            i += 1;
        } else {
            result.positionals.push(arg.to_string());
            i += 1;
        }
    }
    result
}

fn str_flag<'a>(parsed: &'a ParsedArgv, key: &str) -> Option<&'a str> {
    match parsed.flags.get(key) {
        Some(FlagValue::Str(s)) => Some(s),
        _ => None,
    }
}

fn bool_flag(parsed: &ParsedArgv, key: &str) -> Option<bool> {
    match parsed.flags.get(key) {
        Some(FlagValue::True) => Some(true),
        Some(FlagValue::Str(s)) if s == "true" => Some(true),
        Some(FlagValue::Str(s)) if s == "false" => Some(false),
        _ => None,
    }
}

fn num_flag(parsed: &ParsedArgv, key: &str) -> Option<f64> {
    str_flag(parsed, key)?
        .parse()
        .ok()
        .filter(|n: &f64| n.is_finite())
}

/// `--flag` set as a bare boolean (not `--flag=true` — matches the JS CLI's
/// `=== true` checks for `--wait-for-debugger` / `--no-terminate-existing`).
fn is_true(parsed: &ParsedArgv, key: &str) -> bool {
    matches!(parsed.flags.get(key), Some(FlagValue::True))
}

/// "30" / "30s" / "5m" / "1h" / "1h30m" / "2h15m10s" → seconds. Bare numbers =
/// seconds. `None` for anything else.
fn parse_duration(input: &str) -> Option<f64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_bare_number(trimmed) {
        return trimmed.parse().ok();
    }
    // ^(\d+h)?(\d+m)?(\d+s)?$ with at least one component.
    let mut rest = trimmed;
    let mut total = 0.0_f64;
    let mut any = false;
    for (unit, mult) in [('h', 3600.0), ('m', 60.0), ('s', 1.0)] {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && rest[digits..].starts_with(unit) {
            total += rest[..digits].parse::<f64>().ok()? * mult;
            rest = &rest[digits + 1..];
            any = true;
        }
    }
    (any && rest.is_empty()).then_some(total)
}

/// ^\d+(\.\d+)?$
fn is_bare_number(s: &str) -> bool {
    let (int_part, frac) = s.split_once('.').map_or((s, None), |(i, f)| (i, Some(f)));
    let all_digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    all_digits(int_part) && frac.is_none_or(all_digits)
}

// ---------------------------------------------------------------------------
// method → JSON-RPC params mapping (mirrors the JS CLI's `methodParams`)
// ---------------------------------------------------------------------------

struct Mapped {
    method: String,
    params: Value,
    timeout_ms: u64,
}

// One match arm per RPC method, mirroring the JS CLI's `methodParams` switch —
// a flat catalog reads better than the indirection needed to shorten it.
#[allow(clippy::too_many_lines)]
fn method_params(parsed: &ParsedArgv) -> Mapped {
    let method = parsed.method.clone().unwrap_or_default();
    let first = parsed.positionals.first().map(String::as_str);
    let second = parsed.positionals.get(1).map(String::as_str);
    let mut timeout_ms = DEFAULT_TIMEOUT_MS;

    let params = match method.as_str() {
        "meta.schema" => object(|p| put_opt(p, "method", first)),

        "scheme.set" | "buildConfig.set" | "scheme.reveal" => object(|p| put_opt(p, "name", first)),
        "destination.set" | "simulator.start" | "simulator.stop" => {
            object(|p| put_opt(p, "id", first))
        }
        "workspace.use" => object(|p| put_opt(p, "path", first)),
        "workspaceState.get"
        | "workspaceState.delete"
        | "vscodeSettings.get"
        | "vscodeSettings.inspect" => object(|p| put_opt(p, "key", first)),
        "build.status" | "build.logs" | "build.diagnostics" => {
            object(|p| put_opt(p, "buildId", first))
        }

        "destination.list" => object(|p| {
            put_opt(p, "type", str_flag(parsed, "type"));
            put_opt(p, "platform", str_flag(parsed, "platform"));
            put_opt(p, "booted", bool_flag(parsed, "booted"));
        }),
        "simulator.list" => object(|p| {
            put_opt(p, "state", str_flag(parsed, "state"));
            put_opt(p, "available", bool_flag(parsed, "available"));
        }),
        "build.list" => object(|p| put_opt(p, "limit", num_flag(parsed, "limit").map(json_num))),
        "workspace.detect" => {
            object(|p| put_opt(p, "depth", num_flag(parsed, "depth").map(json_num)))
        }
        "logs.tail" => object(|p| {
            put_opt(p, "lines", num_flag(parsed, "lines").map(json_num));
            put_opt(p, "level", str_flag(parsed, "level"));
        }),

        "simulator.install" => object(|p| {
            put_opt(p, "udid", first);
            put_opt(p, "appPath", second);
        }),
        "simulator.uninstall" | "simulator.terminateApp" => object(|p| {
            put_opt(p, "udid", first);
            put_opt(p, "bundleId", second);
        }),
        "simulator.openUrl" => object(|p| {
            put_opt(p, "udid", first);
            put_opt(p, "url", second);
        }),
        "simulator.screenshot" => object(|p| {
            put_opt(p, "udid", first);
            put_opt(p, "path", str_flag(parsed, "path"));
        }),
        "simulator.launchApp" => object(|p| {
            put_opt(p, "udid", first);
            put_opt(p, "bundleId", second);
            put_launch_extras(p, parsed);
            if is_true(parsed, "wait-for-debugger") {
                p.insert("waitForDebugger".into(), Value::Bool(true));
            }
        }),

        "device.install" => object(|p| {
            put_opt(p, "deviceId", first);
            put_opt(p, "appPath", second);
        }),
        "device.terminate" => object(|p| {
            put_opt(p, "deviceId", first);
            put_opt(p, "bundleId", second);
        }),
        "device.launch" => object(|p| {
            put_opt(p, "deviceId", first);
            put_opt(p, "bundleId", second);
            put_launch_extras(p, parsed);
            if is_true(parsed, "no-terminate-existing") {
                p.insert("terminateExisting".into(), Value::Bool(false));
            }
        }),

        "buildSettings.get" => object(|p| {
            put_xcworkspace_query(p, parsed);
            if let Some(csv) = str_flag(parsed, "keys") {
                let keys: Vec<Value> = csv
                    .split(',')
                    .map(str::trim)
                    .filter(|k| !k.is_empty())
                    .map(Value::from)
                    .collect();
                p.insert("keys".into(), Value::Array(keys));
            }
        }),
        "appPath.find" | "bundleId.get" => object(|p| put_xcworkspace_query(p, parsed)),
        "xcodebuild.list" => object(|p| put_opt(p, "xcworkspace", str_flag(parsed, "xcworkspace"))),

        "build.start" => object(|p| {
            p.insert("command".into(), Value::from(first.unwrap_or("build")));
            p.insert(
                "debug".into(),
                Value::Bool(bool_flag(parsed, "debug") == Some(true)),
            );
            let caller = str_flag(parsed, "caller")
                .map(str::to_string)
                .or_else(|| std::env::var("SWEETPAD_CALLER").ok());
            put_opt(p, "caller", caller.filter(|c| !c.is_empty()));
        }),
        "build.wait" => {
            let timeout_sec = str_flag(parsed, "timeout").and_then(parse_duration);
            if let Some(sec) = timeout_sec {
                // Pad the client timeout past the server's so a slow
                // round-trip doesn't trip us first.
                timeout_ms = round_ms(sec) + 10_000;
            }
            object(|p| {
                put_opt(p, "buildId", first);
                put_opt(
                    p,
                    "timeoutMs",
                    timeout_sec.map(|s| Value::from(round_ms(s))),
                );
            })
        }

        "workspaceState.set" => object(|p| {
            put_opt(p, "key", first);
            put_opt(p, "value", str_flag(parsed, "value").map(parse_value_flag));
        }),
        "vscodeSettings.set" => object(|p| {
            put_opt(p, "key", first);
            put_opt(p, "value", str_flag(parsed, "value").map(parse_value_flag));
            put_opt(p, "target", str_flag(parsed, "target"));
        }),

        "vscode.executeCommand" => object(|p| {
            put_opt(p, "command", first);
            if let Some(Value::Array(args)) = parse_json_flag(str_flag(parsed, "args-json")) {
                p.insert("args".into(), Value::Array(args));
            } else if parsed.positionals.len() > 1 {
                let rest: Vec<Value> = parsed.positionals[1..]
                    .iter()
                    .map(|a| Value::from(a.as_str()))
                    .collect();
                p.insert("args".into(), Value::Array(rest));
            }
        }),

        // meta.usage / meta.version / meta.workspacePath / state.get /
        // scheme.list / destination.get / buildConfig.* / build.stop /
        // simulator.refresh / derivedData.path / workspace.recent /
        // workspaceState.keys / vscodeSettings.list — and any method this
        // mapping doesn't know — pass through with empty params.
        _ => Value::Object(Map::new()),
    };

    Mapped {
        method,
        params,
        timeout_ms,
    }
}

fn object(build: impl FnOnce(&mut Map<String, Value>)) -> Value {
    let mut map = Map::new();
    build(&mut map);
    Value::Object(map)
}

fn put_opt(map: &mut Map<String, Value>, key: &str, value: Option<impl Into<Value>>) {
    if let Some(value) = value {
        map.insert(key.to_string(), value.into());
    }
}

/// `--scheme/--configuration/--sdk/--xcworkspace`, shared by the xcodebuild-
/// style queries (`buildSettings.get`, `appPath.find`, `bundleId.get`).
fn put_xcworkspace_query(map: &mut Map<String, Value>, parsed: &ParsedArgv) {
    for key in ["scheme", "configuration", "sdk", "xcworkspace"] {
        put_opt(map, key, str_flag(parsed, key));
    }
}

/// `--args-json '[...]'` / `--env-json '{...}'`, shared by
/// `simulator.launchApp` and `device.launch`.
fn put_launch_extras(map: &mut Map<String, Value>, parsed: &ParsedArgv) {
    if let Some(Value::Array(args)) = parse_json_flag(str_flag(parsed, "args-json")) {
        map.insert("args".into(), Value::Array(args));
    }
    if let Some(Value::Object(env)) = parse_json_flag(str_flag(parsed, "env-json")) {
        map.insert("env".into(), Value::Object(env));
    }
}

/// `--value '{"a":1}'`, `--value true`, or `--value "plain string"` all work.
fn parse_value_flag(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::from(raw))
}

fn parse_json_flag(raw: Option<&str>) -> Option<Value> {
    serde_json::from_str(raw?).ok()
}

/// Seconds → whole milliseconds (the wire carries integers, like JS
/// `Math.round(sec * 1000)`).
fn round_ms(sec: f64) -> u64 {
    let ms = (sec * 1000.0).round().max(0.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        ms as u64
    }
}

/// A float that is a whole number serializes as a JSON integer (`5`, not
/// `5.0`) — what the JS CLI put on the wire.
fn json_num(n: f64) -> Value {
    const MAX_SAFE: f64 = 9_007_199_254_740_992.0; // 2^53
    if n.fract() == 0.0 && n.abs() <= MAX_SAFE {
        #[allow(clippy::cast_possible_truncation)]
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

// ---------------------------------------------------------------------------
// server discovery + JSON-RPC transport
// ---------------------------------------------------------------------------

struct Failure {
    code: u8,
    envelope: Value,
}

fn failure(code: u8, error_code: &str, message: &str) -> Failure {
    Failure {
        code,
        envelope: error_envelope(error_code, message, None, None),
    }
}

fn error_envelope(code: &str, message: &str, hint: Option<&str>, data: Option<Value>) -> Value {
    let mut error = Map::new();
    error.insert("code".into(), Value::from(code));
    error.insert("message".into(), Value::from(message));
    put_opt(&mut error, "hint", hint);
    put_opt(&mut error, "data", data);
    json!({ "ok": false, "error": error })
}

/// The CLI talks to the single control server advertised in `.sweetpad/cli.json`
/// (last-writer-wins across windows). Read its socket; the connect itself
/// surfaces a dead server as ECONNREFUSED.
fn resolve_socket() -> Result<String, Value> {
    let no_server = |message: &str| error_envelope("NO_SERVER", message, None, None);
    let root = std::env::current_dir()
        .ok()
        .and_then(|cwd| find_project_root(&cwd))
        .ok_or_else(|| no_server("No .sweetpad project found from the current directory."))?;
    std::fs::read_to_string(root.join(SWEETPAD_DIR).join("cli.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|meta| meta.get("socket")?.as_str().map(str::to_string))
        .ok_or_else(|| {
            no_server(
                "No running SweetPad server (.sweetpad/cli.json not found). Enable sweetpad.cliServer.enabled and open the project in VS Code.",
            )
        })
}

/// Walk up from `start` to the nearest ancestor containing a `.sweetpad`
/// directory — how the CLI finds the project it's run inside, like `git`
/// finds `.git`.
fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(SWEETPAD_DIR).is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// One-shot JSON-RPC 2.0 call over the Unix socket using `Content-Length`
/// framing. `Ok(None)` when the response carries no `result` field.
fn call_rpc(socket: &str, mapped: &Mapped) -> Result<Option<Value>, Failure> {
    let stream = UnixStream::connect(socket).map_err(|e| connect_failure(socket, &e))?;
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": mapped.method,
        "params": mapped.params,
    });
    let mut writer = &stream;
    write_message(&mut writer, &request.to_string()).map_err(|e| failure(2, "CLI_ERROR", &e))?;
    writer.flush().ok();

    let deadline = Instant::now() + Duration::from_millis(mapped.timeout_ms);
    let timeout = || {
        failure(
            2,
            "CLI_ERROR",
            &format!("Request timed out after {}ms", mapped.timeout_ms),
        )
    };
    let mut reader = BufReader::new(stream);
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(timeout());
        };
        reader.get_ref().set_read_timeout(Some(remaining)).ok();
        let body = match read_message(&mut reader) {
            Ok(Some(body)) => body,
            Ok(None) => {
                return Err(failure(
                    2,
                    "CLI_ERROR",
                    "Connection closed before a response was received",
                ));
            }
            // The codec stringifies I/O errors, so a timed-out read is told
            // apart from a real failure by checking the deadline.
            Err(_) if Instant::now() >= deadline => return Err(timeout()),
            Err(e) => return Err(failure(2, "CLI_ERROR", &e)),
        };
        let Ok(message) = serde_json::from_str::<Value>(&body) else {
            continue;
        };
        // Skip anything that isn't the response to our request (the server
        // may interleave notifications on the same connection).
        if message.get("id") != Some(&Value::from(1)) {
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(rpc_failure(error));
        }
        return Ok(message.get("result").cloned());
    }
}

fn connect_failure(socket: &str, err: &std::io::Error) -> Failure {
    use std::io::ErrorKind;
    let code = match err.kind() {
        ErrorKind::NotFound => Some("ENOENT"),
        ErrorKind::ConnectionRefused => Some("ECONNREFUSED"),
        _ => None,
    };
    let message = match code {
        Some(code) => {
            format!(
                "Cannot connect to server at {socket} ({code}). Is the SweetPad RPC server running?"
            )
        }
        None => err.to_string(),
    };
    failure(2, "CLI_ERROR", &message)
}

/// Server-side failure: keep the stable string code the server put in
/// `error.data.code` (fall back to "RPC_ERROR"), plus its hint and data.
fn rpc_failure(error: &Value) -> Failure {
    let message = error.get("message").and_then(Value::as_str).unwrap_or("");
    let data = error.get("data").filter(|d| !d.is_null());
    let code = data
        .and_then(|d| d.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("RPC_ERROR");
    let hint = data.and_then(|d| d.get("hint")).and_then(Value::as_str);
    Failure {
        code: 1,
        envelope: error_envelope(code, message, hint, data.cloned()),
    }
}

fn usage_envelope() -> Value {
    let usage = "\
sweetpad vscode — JSON-RPC client for the SweetPad VS Code extension

Usage:
  sweetpad vscode <method> [args...] [--raw]

Methods (canonical dot-form; arguments listed after the method name):
  meta.usage
  meta.schema [<method>]
  meta.version
  meta.workspacePath

  state.get

  scheme.list
  scheme.get
  scheme.set <name>
  scheme.reveal <name>

  destination.list [--type <t>] [--platform <p>] [--booted]
  destination.get
  destination.set <id>

  simulator.list [--state Booted] [--available]
  simulator.start <id-or-udid>
  simulator.stop <id-or-udid>
  simulator.refresh
  simulator.install <udid> <appPath>
  simulator.uninstall <udid> <bundleId>
  simulator.launchApp <udid> <bundleId> [--args-json '[...]'] [--env-json '{...}'] [--wait-for-debugger]
  simulator.terminateApp <udid> <bundleId>
  simulator.openUrl <udid> <url>
  simulator.screenshot <udid> [--path <p>]

  device.install <deviceId> <appPath>
  device.launch <deviceId> <bundleId> [--args-json '[...]'] [--env-json '{...}'] [--no-terminate-existing]
  device.terminate <deviceId> <bundleId>

  buildConfig.list
  buildConfig.get
  buildConfig.set <name>

  buildSettings.get [--scheme <s>] [--configuration <c>] [--sdk <s>] [--xcworkspace <p>] [--keys K1,K2]
  xcodebuild.list [--xcworkspace <p>]
  appPath.find [--scheme <s>] [--configuration <c>] [--sdk <s>] [--xcworkspace <p>]
  bundleId.get [--scheme <s>] [--configuration <c>] [--sdk <s>] [--xcworkspace <p>]
  derivedData.path

  build.start <cmd> [--debug] [--caller <label>]
  build.stop
  build.wait [<id>] [--timeout <30s|5m|1h>]
  build.status [<id>]
  build.logs [<id>]
  build.diagnostics [<id>]
  build.list [--limit N]

  workspace.detect [--depth N]
  workspace.use <path>
  workspace.recent

  workspaceState.get <key>
  workspaceState.set <key> --value <json|string>
  workspaceState.keys
  workspaceState.delete <key>

  vscode.executeCommand <command> [...args]                      # or --args-json '[...]'
  vscodeSettings.get <key>
  vscodeSettings.set <key> --value <json|string> [--target global|workspace|workspaceFolder]
  vscodeSettings.inspect <key>
  vscodeSettings.list

  logs.tail [--lines N] [--level debug|info|warning|error]

Flags:
  --raw                    minify JSON output
  --timeout <30s|5m|1h>    duration for build.wait (capped server-side ~30s)
  --caller <label>         label build originator (also SWEETPAD_CALLER env)";
    error_envelope("USAGE", usage, None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> ParsedArgv {
        let owned: Vec<String> = args.iter().map(ToString::to_string).collect();
        parse_argv(&owned)
    }

    fn params_for(args: &[&str]) -> Value {
        method_params(&argv(args)).params
    }

    #[test]
    fn argv_first_dotted_token_is_the_method() {
        let r = argv(&["scheme.list"]);
        assert_eq!(r.method.as_deref(), Some("scheme.list"));
        assert!(r.positionals.is_empty());
        assert!(!r.help);
    }

    #[test]
    fn argv_collects_remaining_tokens_as_positionals() {
        let r = argv(&["simulator.install", "udid-abc", "/path/to/app.app"]);
        assert_eq!(r.method.as_deref(), Some("simulator.install"));
        assert_eq!(r.positionals, ["udid-abc", "/path/to/app.app"]);
    }

    #[test]
    fn argv_bare_first_token_leaves_method_unset() {
        let r = argv(&["servers", "list"]);
        assert_eq!(r.method, None);
        assert_eq!(r.positionals, ["list"]);
    }

    #[test]
    fn argv_flag_forms() {
        let r = argv(&["build.wait", "b1", "--timeout=300"]);
        assert_eq!(r.positionals, ["b1"]);
        assert_eq!(r.flags["timeout"], FlagValue::Str("300".into()));

        let r = argv(&["build.list", "--limit", "5"]);
        assert_eq!(r.flags["limit"], FlagValue::Str("5".into()));

        let r = argv(&["build.start", "build", "--debug"]);
        assert_eq!(r.positionals, ["build"]);
        assert_eq!(r.flags["debug"], FlagValue::True);
    }

    #[test]
    fn argv_help_and_raw() {
        assert!(argv(&["--help"]).help);
        assert!(argv(&["-h"]).help);
        let r = argv(&["--raw", "meta.version"]);
        assert!(r.raw);
        assert_eq!(r.method.as_deref(), Some("meta.version"));
    }

    #[test]
    fn duration_parses_bare_seconds_and_units() {
        assert_eq!(parse_duration("30"), Some(30.0));
        assert_eq!(parse_duration("0"), Some(0.0));
        assert_eq!(parse_duration("1.5"), Some(1.5));
        assert_eq!(parse_duration("30s"), Some(30.0));
        assert_eq!(parse_duration("5m"), Some(300.0));
        assert_eq!(parse_duration("1h"), Some(3600.0));
        assert_eq!(parse_duration("1h30m"), Some(5400.0));
        assert_eq!(parse_duration("2h15m10s"), Some(8110.0));
        assert_eq!(parse_duration("90m"), Some(5400.0));
        assert_eq!(parse_duration("  30s  "), Some(30.0));
    }

    #[test]
    fn duration_rejects_invalid_input() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration("30x"), None);
        assert_eq!(parse_duration("m30"), None);
    }

    #[test]
    fn maps_positional_only_methods() {
        assert_eq!(
            params_for(&["scheme.set", "MyApp"]),
            json!({"name": "MyApp"})
        );
        assert_eq!(params_for(&["scheme.set"]), json!({}));
        assert_eq!(
            params_for(&["destination.set", "id1"]),
            json!({"id": "id1"})
        );
        assert_eq!(params_for(&["scheme.list"]), json!({}));
        assert_eq!(
            params_for(&["meta.schema", "scheme.list"]),
            json!({"method": "scheme.list"})
        );
    }

    #[test]
    fn maps_flag_built_methods() {
        assert_eq!(
            params_for(&["destination.list", "--type", "simulator", "--booted"]),
            json!({"type": "simulator", "booted": true})
        );
        assert_eq!(
            params_for(&["build.list", "--limit", "5"]),
            json!({"limit": 5})
        );
        assert_eq!(
            params_for(&["buildSettings.get", "--scheme", "App", "--keys", "A, B,,C"]),
            json!({"scheme": "App", "keys": ["A", "B", "C"]})
        );
    }

    #[test]
    fn maps_build_start_and_wait() {
        let mapped = method_params(&argv(&["build.start", "--debug"]));
        assert_eq!(mapped.params, json!({"command": "build", "debug": true}));

        let mapped = method_params(&argv(&["build.wait", "b1", "--timeout", "5s"]));
        assert_eq!(mapped.params, json!({"buildId": "b1", "timeoutMs": 5000}));
        assert_eq!(mapped.timeout_ms, 15_000);

        let mapped = method_params(&argv(&["build.wait"]));
        assert_eq!(mapped.params, json!({}));
        assert_eq!(mapped.timeout_ms, DEFAULT_TIMEOUT_MS);
    }

    #[test]
    fn maps_launch_extras_and_values() {
        assert_eq!(
            params_for(&[
                "simulator.launchApp",
                "u1",
                "com.app",
                "--args-json",
                "[\"-x\"]",
                "--wait-for-debugger"
            ]),
            json!({"udid": "u1", "bundleId": "com.app", "args": ["-x"], "waitForDebugger": true})
        );
        assert_eq!(
            params_for(&["workspaceState.set", "k", "--value", "{\"a\":1}"]),
            json!({"key": "k", "value": {"a": 1}})
        );
        assert_eq!(
            params_for(&["workspaceState.set", "k", "--value", "plain"]),
            json!({"key": "k", "value": "plain"})
        );
        assert_eq!(
            params_for(&["vscode.executeCommand", "cmd.id", "a1", "a2"]),
            json!({"command": "cmd.id", "args": ["a1", "a2"]})
        );
    }

    #[test]
    fn unknown_dotted_methods_pass_through_with_empty_params() {
        let mapped = method_params(&argv(&["future.method", "ignored"]));
        assert_eq!(mapped.method, "future.method");
        assert_eq!(mapped.params, json!({}));
    }
}
