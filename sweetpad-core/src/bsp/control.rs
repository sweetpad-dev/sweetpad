//! The telemetry channel to the SweetPad extension. The BSP server reads all of
//! its config — including the Unix socket path to bind — from the `bsp.json`
//! named by `--config` (written by the extension into the host state dir). It
//! binds that socket, serves `bsp/log` and `bsp/status` to any extension that
//! connects, and accepts `bsp/setLogLevel` back. Config never flows over this
//! channel; it lives entirely in `bsp.json`.

use std::io::BufReader;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::framing::{read_message, write_message};

/// Verbosity of the `bsp/log` stream pushed to the extension. Gates only the
/// telemetry stream — the `SWEETPAD_BSP_LOG` file always gets everything.
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

    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "off" => LogLevel::Off,
            "error" => LogLevel::Error,
            "debug" => LogLevel::Debug,
            _ => LogLevel::Info,
        }
    }
}

/// The telemetry socket the BSP binds — at the path the extension assigned in
/// `bsp.json` — and the connected extensions it pushes `bsp/log` / `bsp/status`
/// to. Each accepted connection also feeds `bsp/setLogLevel` back through the
/// `on_set_level` callback.
pub(crate) struct TelemetryServer {
    clients: Mutex<Vec<UnixStream>>,
    socket: PathBuf,
}

impl TelemetryServer {
    /// Bind `socket` and start accepting extension connections. Reclaims a stale
    /// socket file (nothing listening there); returns `None` if another live
    /// server already owns the path — a second BSP for the same project — so the
    /// caller simply runs without telemetry. `on_set_level` fires for each
    /// incoming `bsp/setLogLevel`.
    pub(crate) fn bind(
        socket: &Path,
        on_set_level: impl Fn(&str) + Send + Sync + 'static,
    ) -> Option<Arc<TelemetryServer>> {
        // A leftover socket file is either a live peer (another BSP owns this
        // project — stand down) or a crashed one's stale file (reclaim it).
        if socket.exists() {
            if UnixStream::connect(socket).is_ok() {
                return None;
            }
            let _ = std::fs::remove_file(socket);
        }
        let listener = UnixListener::bind(socket).ok()?;
        let server = Arc::new(TelemetryServer {
            clients: Mutex::new(Vec::new()),
            socket: socket.to_path_buf(),
        });
        let accept = Arc::clone(&server);
        let on_set_level = Arc::new(on_set_level);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                // Keep a write half for broadcasts; the read half drains
                // `bsp/setLogLevel` until the extension disconnects. The
                // write timeout is load-bearing: `broadcast` runs on the
                // request thread, so a connected client that stops reading
                // (suspended extension host, full socket buffer) would
                // otherwise block a broadcast forever — wedging the whole
                // server. A timed-out write fails and drops the client.
                if let Ok(write_half) = stream.try_clone()
                    && let Ok(mut clients) = accept.clients.lock()
                {
                    let _ = write_half.set_write_timeout(Some(Duration::from_secs(1)));
                    clients.push(write_half);
                }
                let on_set_level = Arc::clone(&on_set_level);
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream);
                    while let Ok(Some(msg)) = read_message(&mut reader) {
                        if let Ok(val) = serde_json::from_str::<Value>(&msg)
                            && val.get("method").and_then(Value::as_str) == Some("bsp/setLogLevel")
                            && let Some(level) = val
                                .get("params")
                                .and_then(|p| p.get("level"))
                                .and_then(Value::as_str)
                        {
                            on_set_level(level);
                        }
                    }
                });
            }
        });
        Some(server)
    }

    /// Push a JSON-RPC notification to every connected extension, dropping any
    /// that errors on write — a disconnected client, or one whose write timed
    /// out (see the timeout set at accept; this runs on the request thread, so
    /// a stalled client must cost at most one bounded write, never a wedge).
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn broadcast(&self, method: &str, params: Value) {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string();
        if let Ok(mut clients) = self.clients.lock() {
            clients.retain_mut(|c| write_message(c, &body).is_ok());
        }
    }

    /// Remove the socket file on a clean shutdown. (A crash leaves it behind; the
    /// next server reclaims it via the connect-probe in [`Self::bind`].)
    pub(crate) fn shutdown(&self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}
