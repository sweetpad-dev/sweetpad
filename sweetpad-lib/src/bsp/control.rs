//! The control channel to the SweetPad extension. Instead of taking the project
//! and toolchain on its command line, the BSP server discovers the extension's
//! Unix socket under its XDG state dir, connects, and pulls the resolved config
//! over JSON-RPC (`bsp.resolveConfig`). The same connection stays open so the
//! server can push `bsp/log` and `bsp/status` up, and receive `bsp/setLogLevel`.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::framing::{read_message, write_message};

/// Verbosity of the `bsp/log` stream pushed to the extension. Gates only the
/// control-channel stream — the `SWEETPAD_BSP_LOG` file always gets everything.
#[derive(Clone, Copy)]
pub(crate) enum LogLevel {
    Off = 0,
    Error = 1,
    Info = 2,
    Debug = 3,
}

impl LogLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LogLevel::Off => "off",
            LogLevel::Error => "error",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "off" => LogLevel::Off,
            "error" => LogLevel::Error,
            "debug" => LogLevel::Debug,
            _ => LogLevel::Info,
        }
    }
}

/// Walk up from `start` to the nearest ancestor that contains a `.sweetpad`
/// directory — the project root, found the way `git` finds `.git`.
fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start;
    loop {
        if dir.join(".sweetpad").is_dir() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// The extension server's control socket for the project containing `cwd`: read
/// the `.sweetpad/run/*.json` connection files and pick the active server's
/// socket, else any `kind: "extension"` one (BSP entries are ignored). A
/// returned path may be stale — the connect surfaces that as a refused
/// connection.
pub(crate) fn discover_socket(cwd: &Path) -> Option<PathBuf> {
    let root = find_project_root(cwd)?;
    let run = root.join(".sweetpad/run");
    let active = read_active_name(&root);
    let mut entries: Vec<(String, String, String)> = Vec::new();
    for entry in std::fs::read_dir(&run).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(conn) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if let (Some(name), Some(kind), Some(socket)) = (
            conn.get("name").and_then(Value::as_str),
            conn.get("kind").and_then(Value::as_str),
            conn.get("socket").and_then(Value::as_str),
        ) {
            entries.push((name.to_string(), kind.to_string(), socket.to_string()));
        }
    }
    pick_extension_socket(&entries, active.as_deref()).map(PathBuf::from)
}

/// The active server's recorded name, from `.sweetpad/active.json`.
fn read_active_name(root: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(root.join(".sweetpad/active.json")).ok()?;
    serde_json::from_str::<Value>(&raw).ok()?.get("server").and_then(Value::as_str).map(str::to_string)
}

/// Among `(name, kind, socket)` connection entries, the active server's socket
/// when it's an extension server, else the first `extension` socket. Pure (no
/// IO) so the selection policy is unit-testable.
fn pick_extension_socket(entries: &[(String, String, String)], active: Option<&str>) -> Option<String> {
    let mut fallback: Option<&str> = None;
    for (name, kind, socket) in entries {
        if kind != "extension" {
            continue;
        }
        if active == Some(name.as_str()) {
            return Some(socket.clone());
        }
        if fallback.is_none() {
            fallback = Some(socket.as_str());
        }
    }
    fallback.map(str::to_string)
}

/// A live JSON-RPC connection to the extension over its Unix control socket.
/// Sends are serialized through a mutex; a background thread drains incoming
/// notifications (currently `bsp/setLogLevel`).
pub(crate) struct ControlClient {
    write: Mutex<UnixStream>,
}

impl ControlClient {
    /// Connect, pull the resolved config via `bsp.resolveConfig`, then keep the
    /// connection open: spawn the notification reader and return the client plus
    /// the config `Value`. `log_level` is shared with the reader so the
    /// extension can retune the `bsp/log` stream live.
    pub(crate) fn connect_and_resolve(
        socket: &Path,
        log_level: Arc<AtomicU8>,
    ) -> Result<(Arc<ControlClient>, Value), String> {
        let stream = UnixStream::connect(socket).map_err(|e| format!("connect {}: {e}", socket.display()))?;
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let client = Arc::new(ControlClient { write: Mutex::new(stream) });

        let config = client.request(&mut reader, "bsp.resolveConfig", json!({}))?;

        std::thread::spawn(move || {
            while let Ok(Some(msg)) = read_message(&mut reader) {
                handle_incoming(&msg, &log_level);
            }
        });
        Ok((client, config))
    }

    /// One request/response round-trip on the connection, reading (and ignoring)
    /// any interleaved notifications until the matching response arrives. Uses a
    /// fixed id since requests are issued one-at-a-time during connect.
    // Callers pass an owned `json!(...)` temporary; `json!` serializes it by
    // reference, so clippy can't see the value as consumed.
    #[allow(clippy::needless_pass_by_value)]
    fn request(&self, reader: &mut impl BufRead, method: &str, params: Value) -> Result<Value, String> {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        {
            let mut w = self.write.lock().map_err(|_| "control write lock poisoned")?;
            write_message(&mut *w, &req.to_string())?;
        }
        loop {
            let Some(msg) = read_message(reader)? else {
                return Err("control connection closed before response".into());
            };
            let Ok(val) = serde_json::from_str::<Value>(&msg) else {
                continue;
            };
            if val.get("id").and_then(Value::as_i64) == Some(1) {
                if let Some(err) = val.get("error") {
                    return Err(format!("{method} failed: {err}"));
                }
                return Ok(val.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    /// Fire a notification at the extension (best-effort — a dropped control
    /// connection must not wedge the BSP loop).
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn notify(&self, method: &str, params: Value) {
        let notif = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        if let Ok(mut w) = self.write.lock() {
            let _ = write_message(&mut *w, &notif.to_string());
        }
    }
}

fn handle_incoming(msg: &str, log_level: &AtomicU8) {
    let Ok(val) = serde_json::from_str::<Value>(msg) else {
        return;
    };
    if val.get("method").and_then(Value::as_str) == Some("bsp/setLogLevel")
        && let Some(level) = val.get("params").and_then(|p| p.get("level")).and_then(Value::as_str)
    {
        log_level.store(LogLevel::parse(level) as u8, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::pick_extension_socket;

    fn entries(rows: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
        rows.iter().map(|(n, k, s)| ((*n).to_string(), (*k).to_string(), (*s).to_string())).collect()
    }

    #[test]
    fn prefers_the_active_extension_server() {
        let e = entries(&[("a", "extension", "/tmp/a.sock"), ("b", "extension", "/tmp/b.sock")]);
        assert_eq!(pick_extension_socket(&e, Some("b")).as_deref(), Some("/tmp/b.sock"));
    }

    #[test]
    fn falls_back_to_the_first_extension_when_no_active() {
        let e = entries(&[("a", "extension", "/tmp/a.sock"), ("b", "extension", "/tmp/b.sock")]);
        assert_eq!(pick_extension_socket(&e, None).as_deref(), Some("/tmp/a.sock"));
    }

    #[test]
    fn ignores_bsp_entries() {
        let e = entries(&[("x", "bsp", "/tmp/x.sock"), ("y", "extension", "/tmp/y.sock")]);
        assert_eq!(pick_extension_socket(&e, None).as_deref(), Some("/tmp/y.sock"));
    }

    #[test]
    fn none_when_no_extension_server() {
        let e = entries(&[("x", "bsp", "/tmp/x.sock")]);
        assert_eq!(pick_extension_socket(&e, None), None);
    }
}
