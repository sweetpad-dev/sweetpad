//! The injection *server* the CLI runs: it binds `:8887`, accepts the in-app
//! client's connection, completes the handshake, and then ships recompiled
//! dylibs via `.load` whenever the watcher reports a save. Responses
//! (`.injected` / `.failed` / `.unhide`) are read on a background thread and
//! surfaced through the session's logger.

use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::protocol::{self, command, response};
use super::recompiler::Recompiler;

/// A thread-safe sink for human status lines (so the reader thread and the
/// watcher can report through the run session's terminal).
pub type Logger = Arc<dyn Fn(&str) + Send + Sync>;

/// A running injection server bound to `:8887` for one `--hot` session.
pub struct InjectServer {
    recompiler: Arc<Recompiler>,
    log: Logger,
    /// The write half of the client connection, populated once it connects.
    conn: Arc<Mutex<Option<TcpStream>>>,
    connected: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl InjectServer {
    /// Bind `:8887` and start accepting the (single) client in the background.
    /// Must be called *before* the app launches so the client's `+load` connect
    /// succeeds. Fails if the port is taken (another injection server running).
    pub fn start(recompiler: Arc<Recompiler>, log: Logger) -> Result<InjectServer, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, protocol::PORT)).map_err(|e| {
            format!(
                "cannot bind 127.0.0.1:{} for hot reload ({e}). Is InjectionNext.app \
                 or another hot-reload session already running?",
                protocol::PORT
            )
        })?;

        let conn: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
        let connected = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));

        let server = InjectServer {
            recompiler: Arc::clone(&recompiler),
            log: Arc::clone(&log),
            conn: Arc::clone(&conn),
            connected: Arc::clone(&connected),
            stop: Arc::clone(&stop),
        };

        // Non-blocking accept loop: serve each client in turn (a relaunch on `r`
        // reconnects), polling `stop` so the session can shut down cleanly.
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("hot-reload socket setup: {e}"))?;
        std::thread::spawn(move || {
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        serve_client(stream, &recompiler, &log, &conn, &connected, &stop);
                        connected.store(false, Ordering::Relaxed);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => {
                        log(&format!("[hot] accept failed: {e}"));
                        return;
                    }
                }
            }
        });

        Ok(server)
    }

    /// Whether the in-app client has connected yet.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Recompile `file` and ship it to the app via `.load`. The result
    /// (`.injected` / `.failed`) is reported asynchronously by the reader thread.
    pub fn inject(&self, file: &std::path::Path) {
        if !self.is_connected() {
            (self.log)("[hot] app not connected yet — skipping injection");
            return;
        }
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        (self.log)(&format!("[hot] ↻ {name} — recompiling…"));
        let dylib = match self.recompiler.recompile(file) {
            Ok(p) => p,
            Err(e) => {
                (self.log)(&format!("[hot] ✗ {name}: {e}"));
                return;
            }
        };
        let mut guard = self.conn.lock().unwrap();
        if let Some(stream) = guard.as_mut()
            && let Err(e) =
                protocol::write_command(stream, command::LOAD, Some(&dylib.to_string_lossy()))
        {
            (self.log)(&format!("[hot] ✗ failed to send {name}: {e}"));
        }
    }

    /// Tear the session down (stop the reader, drop the connection).
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(stream) = self.conn.lock().unwrap().take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Handshake one accepted client and serve it until its connection drops.
fn serve_client(
    mut stream: TcpStream,
    recompiler: &Arc<Recompiler>,
    log: &Logger,
    conn: &Arc<Mutex<Option<TcpStream>>>,
    connected: &AtomicBool,
    stop: &AtomicBool,
) {
    // An accept off a non-blocking listener may inherit non-blocking; force
    // blocking + a read timeout for the handshake.
    if stream.set_nonblocking(false).is_err()
        || stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .is_err()
    {
        log("[hot] socket setup failed");
        return;
    }
    if let Err(e) = read_handshake(&mut stream, log) {
        log(&format!("[hot] handshake failed: {e}"));
        return;
    }
    // Tell the client which Xcode to reload against (best-effort).
    let _ = protocol::write_command(
        &mut stream,
        command::XCODE_PATH,
        Some(&recompiler.xcode_app_path()),
    );
    // Split: a read clone for the response loop, the original for writes.
    let reader = match stream.try_clone() {
        Ok(r) => r,
        Err(e) => {
            log(&format!("[hot] socket clone failed: {e}"));
            return;
        }
    };
    *conn.lock().unwrap() = Some(stream);
    connected.store(true, Ordering::Relaxed);
    log("[hot] connected — edit a Swift file to inject");
    response_loop(reader, log, stop);
}

/// Read the handshake the client pushes on connect (`InjectionNext.swift`
/// `runInBackground`): version, home dir, then a short stream of responses,
/// drained until a read times out (the client then awaits our commands).
fn read_handshake(s: &mut TcpStream, log: &Logger) -> Result<(), String> {
    let version = protocol::read_int(s)
        .map_err(|e| e.to_string())?
        .ok_or("timed out waiting for version")?;
    if version != protocol::INJECTION_VERSION {
        log(&format!(
            "[hot] client protocol {version} != expected {}; continuing",
            protocol::INJECTION_VERSION
        ));
    }
    let _home = protocol::read_string(s).map_err(|e| e.to_string())?;

    s.set_read_timeout(Some(protocol::HANDSHAKE_DRAIN_TIMEOUT))
        .map_err(|e| e.to_string())?;
    loop {
        let Some(code) = protocol::read_int(s).map_err(|e| e.to_string())? else {
            break; // timeout => handshake drained
        };
        match code {
            response::PLATFORM => {
                let platform = protocol::read_string(s).map_err(|e| e.to_string())?;
                let arch = protocol::read_string(s).map_err(|e| e.to_string())?;
                log(&format!("[hot] client: {platform} {arch}"));
            }
            response::PROJECT_ROOT
            | response::TMP_PATH
            | response::EXECUTABLE
            | response::DETAIL
            | response::BAZEL_TARGET => {
                let _ = protocol::read_string(s).map_err(|e| e.to_string())?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Loop reading post-handshake responses and reporting load results.
fn response_loop(mut reader: TcpStream, log: &Logger, stop: &AtomicBool) {
    // A long timeout so a slow load still resolves; we poll `stop` between reads.
    let _ = reader.set_read_timeout(Some(Duration::from_secs(2)));
    while !stop.load(Ordering::Relaxed) {
        match protocol::read_int(&mut reader) {
            Ok(Some(response::INJECTED)) => log("[hot] ✓ injected"),
            Ok(Some(response::FAILED)) => log("[hot] ✗ injection failed (the patch did not apply)"),
            Ok(Some(response::DETAIL)) => {
                if let Ok(msg) = protocol::read_string(&mut reader) {
                    log(&format!("[hot] {msg}"));
                }
            }
            // .unhide (precedes .failed), other responses, and read timeouts:
            // nothing to report — keep polling.
            Ok(Some(_) | None) => {}
            Err(_) => break, // connection closed
        }
    }
}
