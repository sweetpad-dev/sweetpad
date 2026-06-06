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

/// SweetPad's XDG state root: `$XDG_STATE_HOME/sweetpad`, else
/// `~/.local/state/sweetpad` — mirrors `src/server/paths.ts` `getStateRoot`.
fn state_root() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME") {
        let p = PathBuf::from(xdg);
        if p.is_absolute() {
            return Some(p.join("sweetpad"));
        }
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/state/sweetpad"))
}

fn sockets_dir() -> Option<PathBuf> {
    Some(state_root()?.join("sockets"))
}

/// Find the control socket of the SweetPad server owning `cwd`. Scans the
/// `<name>.json` sidecars in the sockets dir, preferring the server whose
/// `workspacePath` contains `cwd` (longest match wins for nested workspaces),
/// and falling back to the sole server when there's exactly one. A returned
/// path may be stale — the connect surfaces that as a refused connection.
pub(crate) fn discover_socket(cwd: &Path) -> Option<PathBuf> {
    let dir = sockets_dir()?;
    let mut entries: Vec<(String, String)> = Vec::new();
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(sidecar) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if let (Some(name), Some(ws)) = (
            sidecar.get("name").and_then(Value::as_str),
            sidecar.get("workspacePath").and_then(Value::as_str),
        ) {
            entries.push((name.to_string(), ws.to_string()));
        }
    }
    pick_socket(&entries, cwd).map(|name| dir.join(format!("{name}.sock")))
}

/// Choose the server whose workspace contains `cwd` (longest match wins for
/// nested workspaces), else the sole server when there's exactly one. Pure (no
/// IO) so the matching policy is unit-testable.
fn pick_socket<'a>(entries: &'a [(String, String)], cwd: &Path) -> Option<&'a str> {
    let mut best: Option<(usize, &str)> = None;
    for (name, ws) in entries {
        if cwd.starts_with(ws) && best.as_ref().is_none_or(|(len, _)| ws.len() > *len) {
            best = Some((ws.len(), name.as_str()));
        }
    }
    best.map(|(_, name)| name).or_else(|| match entries {
        [(name, _)] => Some(name.as_str()),
        _ => None,
    })
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
    use std::path::Path;

    use super::pick_socket;

    fn entries(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(n, w)| ((*n).to_string(), (*w).to_string())).collect()
    }

    #[test]
    fn picks_the_workspace_that_contains_cwd() {
        let e = entries(&[("a", "/work/projA"), ("b", "/work/projB")]);
        assert_eq!(pick_socket(&e, Path::new("/work/projB/Sources")), Some("b"));
    }

    #[test]
    fn prefers_the_longest_containing_workspace() {
        let e = entries(&[("outer", "/work"), ("inner", "/work/nested")]);
        assert_eq!(pick_socket(&e, Path::new("/work/nested/app")), Some("inner"));
    }

    #[test]
    fn falls_back_to_the_sole_server_when_none_contain_cwd() {
        let e = entries(&[("only", "/elsewhere")]);
        assert_eq!(pick_socket(&e, Path::new("/work/app")), Some("only"));
    }

    #[test]
    fn no_match_among_several_is_none() {
        let e = entries(&[("a", "/x"), ("b", "/y")]);
        assert_eq!(pick_socket(&e, Path::new("/work/app")), None);
    }
}
