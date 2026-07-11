//! The `sweetpad vscode` command — a thin, generic JSON-RPC client for the
//! SweetPad VS Code extension's control server.
//!
//! It carries no per-method knowledge. The first dotted token is the JSON-RPC
//! method; every `--flag` becomes a field of the request's `params` object
//! (bare `--flag` is boolean true, `--flag value` is parsed as JSON with a
//! string fallback); the request goes straight to the server. The server's
//! method catalog — discoverable at runtime via `meta.usage` / `meta.schema` —
//! is the single source of truth for which methods exist and which params they
//! take, so this client never needs updating when the server grows a method.
//! Flag names mirror the catalog's param names one for one.
//!
//! Discovery works like `git`: walk up from the cwd to the nearest ancestor
//! registered in the host-wide index the extension maintains
//! (`~/.local/state/sweetpad/projects.json`, keyed by canonical workspace path,
//! last-writer-wins across VS Code windows), read its Unix socket, then send a
//! single `Content-Length`-framed JSON-RPC 2.0 request over the socket.
//!
//! Output is pretty JSON by default, minified with `--raw`, and a bare string
//! result is printed as-is so it pipes cleanly. Errors go to stderr as
//! `{"ok":false,"error":{...}}`. Exit codes: 0 ok, 1 RPC error, 2 client/usage
//! error. `--help` prints a short pointer at the server's `meta.usage` /
//! `meta.schema` — the client keeps no method list of its own.

use std::collections::BTreeMap;
use std::io::{BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use sweetpad_core::framing::{read_message, write_message};

/// Client-side read deadline. The server bounds every method itself (e.g.
/// `build.wait` caps around 30s, and long-running work returns immediately and
/// is polled rather than blocked on), so one fixed ceiling covers them all —
/// the client needs no per-method timeout knowledge.
const TIMEOUT_MS: u64 = 6 * 60 * 1000;

/// Run `sweetpad vscode <method> [--flag value...]` and return the process exit
/// code: 0 success, 1 the server answered with an RPC error, 2 client-side
/// failure (usage, no server, connect/timeout).
#[must_use]
pub fn run(args: &[String]) -> u8 {
    let parsed = match parse_argv(args) {
        Ok(parsed) => parsed,
        Err(message) => return emit_failure(&usage_error(&message), false),
    };
    if parsed.help {
        println!("{HELP}");
        return 0;
    }
    let raw = parsed.raw;
    if parsed.method.is_none() {
        // A human typing `sweetpad vscode` (or a bare word) gets the usage
        // text, not a machine envelope — scripts always pass a dotted method.
        eprintln!("{HELP}");
        return 2;
    }

    let socket = match resolve_socket() {
        Ok(socket) => socket,
        Err(envelope) => return emit_failure(&envelope, raw),
    };
    match call_rpc(&socket, &parsed) {
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
    flags: BTreeMap<String, FlagValue>,
    raw: bool,
    help: bool,
}

/// `--raw` and `--help`/`-h` are consumed by the client; the first dotted token
/// is the method; every other `--flag` is a param. A positional token after the
/// method is rejected — the generic client takes named flags only.
fn parse_argv(args: &[String]) -> Result<ParsedArgv, String> {
    let mut result = ParsedArgv::default();
    let mut method_seen = false;
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
            } else if let Some(next) = args
                .get(i + 1)
                .filter(|n| !n.starts_with('-') && (method_seen || !method_shaped(n)))
            {
                result
                    .flags
                    .insert(body.to_string(), FlagValue::Str(next.clone()));
                i += 2;
            } else {
                result.flags.insert(body.to_string(), FlagValue::True);
                i += 1;
            }
        } else if !method_seen {
            // First non-flag token is the dotted method name; a bare word
            // (no dot) leaves `method` unset, which the dispatcher turns into
            // usage.
            method_seen = true;
            if arg.contains('.') {
                result.method = Some(arg.to_string());
            }
            i += 1;
        } else {
            return Err(format!(
                "Unexpected positional argument '{arg}'; the vscode client takes named --flags \
                 only (a value starting with '-' needs the --flag=value form).",
            ));
        }
    }
    Ok(result)
}

/// Whether a token has the dotted-method shape (`resource.verb`, identifier
/// segments only). Used so a boolean `--flag` typed *before* the method
/// doesn't swallow the method token as its value — `sweetpad vscode --booted
/// destination.list` must call `destination.list`, not send `{booted:
/// "destination.list"}` to nothing (or, worse, to a later token's method).
fn method_shaped(token: &str) -> bool {
    token.contains('.')
        && token.split('.').all(|s| {
            !s.is_empty()
                && s.starts_with(|c: char| c.is_ascii_alphabetic())
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

/// Build the JSON-RPC `params` object from the parsed flags. Each flag name
/// becomes a field — matching the server catalog's param names one for one —
/// so the client needs no per-method mapping. A bare `--flag` is boolean true;
/// a `--flag value` is JSON-parsed (numbers, booleans, arrays, objects, null)
/// and falls back to the raw string when the value isn't valid JSON.
fn build_params(parsed: &ParsedArgv) -> Value {
    let mut map = Map::new();
    for (key, value) in &parsed.flags {
        let json = match value {
            FlagValue::True => Value::Bool(true),
            FlagValue::Str(raw) => {
                serde_json::from_str(raw).unwrap_or_else(|_| Value::from(raw.as_str()))
            }
        };
        map.insert(key.clone(), json);
    }
    Value::Object(map)
}

// ---------------------------------------------------------------------------
// server discovery + JSON-RPC transport
// ---------------------------------------------------------------------------

fn put_opt(map: &mut Map<String, Value>, key: &str, value: Option<impl Into<Value>>) {
    if let Some(value) = value {
        map.insert(key.to_string(), value.into());
    }
}

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
    // The same envelope shape as the standalone CLI (`{schema, ok, error}`),
    // so one consumer parses both namespaces; the `error.code` vocabulary
    // stays the RPC server's.
    json!({ "schema": 1, "ok": false, "error": error })
}

/// The CLI talks to the single control server the extension advertised for this
/// project in the host-wide index (last-writer-wins across windows). Walk up from
/// cwd to the nearest registered ancestor and read its control socket; the connect
/// itself surfaces a dead server as ECONNREFUSED.
fn resolve_socket() -> Result<String, Value> {
    let no_server = |message: &str| error_envelope("NO_SERVER", message, None, None);
    let cwd = std::env::current_dir()
        .map_err(|_| no_server("Cannot determine the current working directory."))?;
    sweetpad_core::paths::lookup_index_entry(&cwd)
        .as_ref()
        .and_then(|entry| entry.get("control"))
        .and_then(|control| control.get("socket"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            no_server(
                "No running SweetPad server for this project. Enable sweetpad.cliServer.enabled and open the project in VS Code.",
            )
        })
}

/// One-shot JSON-RPC 2.0 call over the Unix socket using `Content-Length`
/// framing. `Ok(None)` when the response carries no `result` field.
fn call_rpc(socket: &str, parsed: &ParsedArgv) -> Result<Option<Value>, Failure> {
    let stream = UnixStream::connect(socket).map_err(|e| connect_failure(socket, &e))?;
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": parsed.method.as_deref().unwrap_or_default(),
        "params": build_params(parsed),
    });
    let mut writer = &stream;
    write_message(&mut writer, &request.to_string()).map_err(|e| failure(2, "CLI_ERROR", &e))?;
    writer.flush().ok();

    let deadline = Instant::now() + Duration::from_millis(TIMEOUT_MS);
    let timeout = || {
        failure(
            2,
            "CLI_ERROR",
            &format!("Request timed out after {TIMEOUT_MS}ms"),
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

/// The single help surface. The client carries no method list of its own, so
/// `--help` is deliberately a short pointer at the server's self-describing
/// `meta.*` calls — where users and agents should start.
const HELP: &str = "\
sweetpad vscode — a generic JSON-RPC client for the running SweetPad VS Code extension.

It has no built-in list of methods; the extension describes its own API, so start there:

  sweetpad vscode meta.usage       every method, one line each
  sweetpad vscode meta.schema      full params (JSON schema) for every method

Confirm which server you're talking to (it's resolved from the current directory):

  sweetpad vscode meta.version         extension + protocol version
  sweetpad vscode meta.workspacePath   which workspace it owns

Then call any method, passing its params as --flags (names come from meta.schema):

  sweetpad vscode state.get
  sweetpad vscode scheme.set --name MyApp
  sweetpad vscode build.start --command launch

Output is pretty JSON; add --raw to minify. Exit codes: 0 ok, 1 RPC error, 2 usage / no server.";

/// A usage/client error envelope (exit 2), always pointing back at `--help`.
fn usage_error(message: &str) -> Value {
    error_envelope(
        "USAGE",
        message,
        Some("Run `sweetpad vscode --help` to get started."),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> ParsedArgv {
        let owned: Vec<String> = args.iter().map(ToString::to_string).collect();
        parse_argv(&owned).expect("parse_argv")
    }

    fn params_for(args: &[&str]) -> Value {
        build_params(&argv(args))
    }

    #[test]
    fn first_dotted_token_is_the_method() {
        let r = argv(&["scheme.list"]);
        assert_eq!(r.method.as_deref(), Some("scheme.list"));
        assert!(r.flags.is_empty());
        assert!(!r.help && !r.raw);
    }

    #[test]
    fn bare_first_token_leaves_method_unset() {
        // A word with no dot isn't a method; the dispatcher turns it into usage.
        let r = argv(&["schemes"]);
        assert_eq!(r.method, None);
        assert!(r.flags.is_empty());
    }

    #[test]
    fn positional_after_method_is_rejected() {
        let owned: Vec<String> = ["scheme.set", "MyApp"]
            .iter()
            .map(ToString::to_string)
            .collect();
        let err = parse_argv(&owned).unwrap_err();
        assert!(err.contains("positional"), "message: {err}");
    }

    #[test]
    fn flag_forms_parse() {
        let r = argv(&["build.wait", "--buildId", "b1", "--timeoutMs=5000"]);
        assert_eq!(r.flags["buildId"], FlagValue::Str("b1".into()));
        assert_eq!(r.flags["timeoutMs"], FlagValue::Str("5000".into()));

        let r = argv(&["destination.list", "--booted"]);
        assert_eq!(r.flags["booted"], FlagValue::True);
    }

    #[test]
    fn boolean_flag_before_the_method_does_not_swallow_it() {
        // `--booted` is a toggle here — the dotted token is the method, not
        // its value.
        let r = argv(&["--booted", "destination.list"]);
        assert_eq!(r.method.as_deref(), Some("destination.list"));
        assert_eq!(r.flags["booted"], FlagValue::True);

        // After the method, values that merely contain a dot still work.
        let r = argv(&["scheme.set", "--name", "My.App"]);
        assert_eq!(r.method.as_deref(), Some("scheme.set"));
        assert_eq!(r.flags["name"], FlagValue::Str("My.App".into()));
    }

    #[test]
    fn help_and_raw_are_reserved() {
        assert!(argv(&["--help"]).help);
        assert!(argv(&["-h"]).help);
        let r = argv(&["--raw", "meta.version"]);
        assert!(r.raw);
        assert_eq!(r.method.as_deref(), Some("meta.version"));
        assert!(r.flags.is_empty());
    }

    #[test]
    fn params_are_empty_without_flags() {
        assert_eq!(params_for(&["scheme.list"]), json!({}));
    }

    #[test]
    fn params_coerce_json_with_string_fallback() {
        // Plain strings stay strings; numbers/booleans/JSON literals coerce.
        assert_eq!(
            params_for(&["scheme.set", "--name", "MyApp"]),
            json!({ "name": "MyApp" })
        );
        assert_eq!(
            params_for(&["build.list", "--limit", "5"]),
            json!({ "limit": 5 })
        );
        assert_eq!(
            params_for(&["destination.list", "--type", "simulator", "--booted"]),
            json!({ "type": "simulator", "booted": true })
        );
        assert_eq!(
            params_for(&["build.wait", "--buildId", "b1", "--timeoutMs", "5000"]),
            json!({ "buildId": "b1", "timeoutMs": 5000 })
        );
    }

    #[test]
    fn params_pass_through_json_arrays_objects_and_bare_flags() {
        assert_eq!(
            params_for(&[
                "simulator.launchApp",
                "--udid",
                "u1",
                "--bundleId",
                "com.app",
                "--args",
                "[\"-x\"]",
                "--waitForDebugger",
            ]),
            json!({ "udid": "u1", "bundleId": "com.app", "args": ["-x"], "waitForDebugger": true })
        );
        assert_eq!(
            params_for(&["workspaceState.set", "--key", "k", "--value", "{\"a\":1}"]),
            json!({ "key": "k", "value": { "a": 1 } })
        );
        assert_eq!(
            params_for(&["workspaceState.set", "--key", "k", "--value", "plain"]),
            json!({ "key": "k", "value": "plain" })
        );
    }
}
