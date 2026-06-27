//! The injection *server* the CLI runs: it binds `:8887`, accepts the in-app
//! client's connection, completes the handshake, and then ships recompiled
//! dylibs via `.load` whenever the watcher reports a save. Responses
//! (`.injected` / `.failed` / `.unhide`) are read on a background thread and
//! surfaced through the session's logger.

use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::protocol::{self, command, response};
use super::recompiler::Recompiler;

/// A thread-safe sink for human status lines (so the reader thread and the
/// watcher can report through the run session's terminal).
pub type Logger = Arc<dyn Fn(&str) + Send + Sync>;

/// The recompile capability the server needs, behind a trait so tests can drive
/// the socket protocol with a stub (no toolchain). Production uses [`Recompiler`].
pub(crate) trait Recompile: Send + Sync {
    fn recompile(&self, file: &std::path::Path) -> Result<std::path::PathBuf, String>;
    fn xcode_app_path(&self) -> String;
}

impl Recompile for Recompiler {
    fn recompile(&self, file: &std::path::Path) -> Result<std::path::PathBuf, String> {
        Recompiler::recompile(self, file)
    }
    fn xcode_app_path(&self) -> String {
        Recompiler::xcode_app_path(self)
    }
}

/// A running injection server bound to `:8887` for one `--hot` session.
pub struct InjectServer {
    recompiler: Arc<dyn Recompile>,
    log: Logger,
    /// The write half of the client connection, populated once it connects.
    conn: Arc<Mutex<Option<TcpStream>>>,
    connected: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    /// `.injected` / `.failed` counts (for the `--hot-selfcheck` CI assertion).
    injected: Arc<AtomicUsize>,
    failed: Arc<AtomicUsize>,
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
        Self::serve(listener, recompiler, log)
    }

    /// Drive an already-bound listener: create the shared state and spawn the
    /// accept loop that serves each client in turn. Split from [`start`] so tests
    /// can bind an ephemeral port and drive the protocol with a stub recompiler.
    fn serve(
        listener: TcpListener,
        recompiler: Arc<dyn Recompile>,
        log: Logger,
    ) -> Result<InjectServer, String> {
        let conn: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
        let connected = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let injected = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(AtomicUsize::new(0));

        let server = InjectServer {
            recompiler: Arc::clone(&recompiler),
            log: Arc::clone(&log),
            conn: Arc::clone(&conn),
            connected: Arc::clone(&connected),
            stop: Arc::clone(&stop),
            injected: Arc::clone(&injected),
            failed: Arc::clone(&failed),
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
                        serve_client(
                            stream,
                            &recompiler,
                            &log,
                            &conn,
                            &connected,
                            &stop,
                            &injected,
                            &failed,
                        );
                        connected.store(false, Ordering::Relaxed);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => {
                        log(&format!("[hot] Accept failed: {e}"));
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

    /// Recompile `file`, ship it via `.load`, and report the outcome — a save stays
    /// on one updating line. The in-progress `… recompiling…` message (the terminal
    /// logger draws any line ending in `…` in place) is overwritten by the
    /// `.injected` / `.failed` result, awaited here (the reader thread
    /// [`response_loop`] tallies it) so the final line can name the file too.
    pub fn inject(&self, file: &std::path::Path) {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        if !self.is_connected() {
            (self.log)(&format!(
                "[hot] ✗ {name} not connected (press r to relaunch)"
            ));
            return;
        }
        // In-progress line: the filename leads (so the line starts capitalized and the
        // save is acknowledged) and it ends with `…`, so the logger draws it in place
        // and the outcome below overwrites it.
        (self.log)(&format!("[hot] » {name} recompiling…"));
        let dylib = match self.recompiler.recompile(file) {
            Ok(p) => p,
            Err(e) => {
                (self.log)(&format!("[hot] ✗ {name} recompile failed: {e}"));
                return;
            }
        };
        let baseline = self.result_counts();
        {
            let mut guard = self.conn.lock().unwrap();
            let Some(stream) = guard.as_mut() else {
                (self.log)(&format!("[hot] ✗ {name} connection lost"));
                return;
            };
            if let Err(e) =
                protocol::write_command(stream, command::LOAD, Some(&dylib.to_string_lossy()))
            {
                (self.log)(&format!("[hot] ✗ {name} send failed: {e}"));
                return;
            }
        }
        // Await the app's verdict so the final line can name the file too.
        match self.wait_for_result(baseline, Duration::from_secs(15)) {
            Some(true) => (self.log)(&format!("[hot] ✓ {name} injected")),
            Some(false) => (self.log)(&format!("[hot] ✗ {name} injection rejected")),
            None => (self.log)(&format!("[hot] ✗ {name} timed out")),
        }
    }

    /// `(injected, failed)` response counts so far.
    #[must_use]
    pub fn result_counts(&self) -> (usize, usize) {
        (
            self.injected.load(Ordering::Relaxed),
            self.failed.load(Ordering::Relaxed),
        )
    }

    /// Block until the client connects, or `timeout` elapses. Used by
    /// `--hot-selfcheck` (CI) to sequence the injection after the app is up.
    #[must_use]
    pub fn wait_connected(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.is_connected() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.is_connected()
    }

    /// Wait for the next `.injected`/`.failed` past `(injected, failed)` baseline.
    /// Returns `Some(true)` on a new `.injected`, `Some(false)` on `.failed`, or
    /// `None` on timeout. For the CI self-check.
    #[must_use]
    pub fn wait_for_result(&self, baseline: (usize, usize), timeout: Duration) -> Option<bool> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let (inj, fail) = self.result_counts();
            if inj > baseline.0 {
                return Some(true);
            }
            if fail > baseline.1 {
                return Some(false);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
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
#[allow(clippy::too_many_arguments)] // shared session state threaded into the worker
fn serve_client(
    mut stream: TcpStream,
    recompiler: &Arc<dyn Recompile>,
    log: &Logger,
    conn: &Arc<Mutex<Option<TcpStream>>>,
    connected: &AtomicBool,
    stop: &AtomicBool,
    injected: &AtomicUsize,
    failed: &AtomicUsize,
) {
    // An accept off a non-blocking listener may inherit non-blocking; force
    // blocking + a read timeout for the handshake.
    if stream.set_nonblocking(false).is_err()
        || stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .is_err()
    {
        log("[hot] Socket setup failed");
        return;
    }
    if let Err(e) = read_handshake(&mut stream, log) {
        log(&format!("[hot] Handshake failed: {e}"));
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
            log(&format!("[hot] Socket clone failed: {e}"));
            return;
        }
    };
    *conn.lock().unwrap() = Some(stream);
    connected.store(true, Ordering::Relaxed);
    log("[hot] Connected — edit a Swift file to inject");
    response_loop(reader, log, stop, injected, failed);
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
            "[hot] Client protocol {version} != expected {}; continuing",
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
                log(&format!("[hot] Client: {platform} {arch}"));
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
fn response_loop(
    mut reader: TcpStream,
    log: &Logger,
    stop: &AtomicBool,
    injected: &AtomicUsize,
    failed: &AtomicUsize,
) {
    // A long timeout so a slow load still resolves; we poll `stop` between reads.
    let _ = reader.set_read_timeout(Some(Duration::from_secs(2)));
    while !stop.load(Ordering::Relaxed) {
        match protocol::read_int(&mut reader) {
            // Tallied for `inject`'s wait and the self-check; `inject` prints the
            // outcome (with the filename), so the loop doesn't log it here.
            Ok(Some(response::INJECTED)) => {
                injected.fetch_add(1, Ordering::Relaxed);
            }
            Ok(Some(response::FAILED)) => {
                failed.fetch_add(1, Ordering::Relaxed);
            }
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

#[cfg(test)]
mod tests {
    //! Drive the real server over a loopback socket with a fake in-app client,
    //! exercising the handshake → `.load` → `.injected`/`.failed` protocol with a
    //! stub recompiler (no Xcode, no simulator). Each test binds an ephemeral
    //! port via [`InjectServer::serve`]; the ~2s handshake drain dominates runtime.
    use super::*;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    /// A recompiler stand-in: returns a canned result, never touches a toolchain.
    struct StubRecompiler {
        result: Result<PathBuf, String>,
    }
    impl Recompile for StubRecompiler {
        fn recompile(&self, _file: &Path) -> Result<PathBuf, String> {
            self.result.clone()
        }
        fn xcode_app_path(&self) -> String {
            "/Applications/Xcode.app".to_string()
        }
    }

    /// A logger that records every status line for assertions.
    fn capturing_logger() -> (Logger, Arc<Mutex<Vec<String>>>) {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let inner = Arc::clone(&sink);
        let log: Logger = Arc::new(move |m: &str| inner.lock().unwrap().push(m.to_string()));
        (log, sink)
    }

    /// Bind an ephemeral port and start the server on it with `stub`.
    fn start_test_server(
        stub: StubRecompiler,
    ) -> (Arc<InjectServer>, u16, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (log, sink) = capturing_logger();
        let server = Arc::new(InjectServer::serve(listener, Arc::new(stub), log).unwrap());
        (server, port, sink)
    }

    /// Perform the in-app client's connect handshake, then read the server's
    /// `.xcodePath` reply (so the stream is positioned to read further commands).
    fn connect_and_handshake(port: u16, version: i32) -> TcpStream {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        protocol::write_int(&mut s, version).unwrap();
        protocol::write_string(&mut s, "/Users/test").unwrap(); // home dir
        protocol::write_int(&mut s, response::PLATFORM).unwrap();
        protocol::write_string(&mut s, "iPhoneSimulator").unwrap();
        protocol::write_string(&mut s, "arm64").unwrap();
        protocol::write_int(&mut s, response::PROJECT_ROOT).unwrap();
        protocol::write_string(&mut s, "/work/App").unwrap();
        s.flush().unwrap();
        // The server drains the handshake, then sends `.xcodePath`. Read it.
        let code = loop {
            if let Some(c) = protocol::read_int(&mut s).unwrap() {
                break c;
            }
        };
        assert_eq!(code, command::XCODE_PATH);
        let _ = protocol::read_string(&mut s).unwrap();
        s
    }

    fn read_command(s: &mut TcpStream) -> i32 {
        loop {
            if let Some(c) = protocol::read_int(s).unwrap() {
                return c;
            }
        }
    }

    #[test]
    fn full_handshake_then_load_and_injected() {
        let dylib = std::env::temp_dir().join("eval_injection_unit.dylib");
        let (server, port, sink) = start_test_server(StubRecompiler {
            result: Ok(dylib.clone()),
        });

        let client = std::thread::spawn(move || {
            let mut s = connect_and_handshake(port, protocol::INJECTION_VERSION);
            assert_eq!(read_command(&mut s), command::LOAD);
            let path = protocol::read_string(&mut s).unwrap();
            protocol::write_int(&mut s, response::INJECTED).unwrap();
            s.flush().unwrap();
            path
        });

        assert!(
            server.wait_connected(Duration::from_secs(5)),
            "client should connect"
        );
        let baseline = server.result_counts();
        server.inject(Path::new("/work/App/ContentView.swift"));
        assert_eq!(
            server.wait_for_result(baseline, Duration::from_secs(5)),
            Some(true),
            "server should observe .injected"
        );

        assert_eq!(client.join().unwrap(), dylib.to_string_lossy());
        assert_eq!(server.result_counts().0, 1);
        server.shutdown();

        let log = sink.lock().unwrap().join("\n");
        assert!(log.contains("recompiling"), "log: {log}");
        assert!(log.contains("injected"), "log: {log}");
    }

    #[test]
    fn failed_response_is_counted() {
        let (server, port, _sink) = start_test_server(StubRecompiler {
            result: Ok(std::env::temp_dir().join("x.dylib")),
        });
        let client = std::thread::spawn(move || {
            let mut s = connect_and_handshake(port, protocol::INJECTION_VERSION);
            assert_eq!(read_command(&mut s), command::LOAD);
            let _ = protocol::read_string(&mut s).unwrap();
            protocol::write_int(&mut s, response::FAILED).unwrap();
            s.flush().unwrap();
        });
        assert!(server.wait_connected(Duration::from_secs(5)));
        let baseline = server.result_counts();
        server.inject(Path::new("/work/App/ContentView.swift"));
        assert_eq!(
            server.wait_for_result(baseline, Duration::from_secs(5)),
            Some(false)
        );
        client.join().unwrap();
        server.shutdown();
    }

    #[test]
    fn recompile_failure_skips_load_and_is_logged() {
        let (server, port, sink) = start_test_server(StubRecompiler {
            result: Err("boom".to_string()),
        });
        let client = std::thread::spawn(move || {
            let mut s = connect_and_handshake(port, protocol::INJECTION_VERSION);
            s.set_read_timeout(Some(Duration::from_millis(500)))
                .unwrap();
            // No `.load` should arrive when the recompile fails.
            matches!(protocol::read_int(&mut s), Ok(None) | Err(_))
        });
        assert!(server.wait_connected(Duration::from_secs(5)));
        server.inject(Path::new("/work/App/Broken.swift"));
        assert_eq!(
            server.wait_for_result((0, 0), Duration::from_millis(800)),
            None,
            "a failed recompile yields no inject result"
        );
        assert!(client.join().unwrap(), "server must not send .load");
        server.shutdown();
        assert!(
            sink.lock().unwrap().join("\n").contains("boom"),
            "the recompile error should be surfaced"
        );
    }

    #[test]
    fn inject_before_connect_is_a_noop() {
        let (server, _port, sink) = start_test_server(StubRecompiler {
            result: Ok(std::env::temp_dir().join("x.dylib")),
        });
        server.inject(Path::new("/work/App/ContentView.swift")); // no client yet
        server.shutdown();
        let log = sink.lock().unwrap().join("\n");
        // The skip names the file and explains why nothing was injected.
        assert!(
            log.contains("ContentView.swift"),
            "the file should be named: {log}"
        );
        assert!(
            log.contains("not connected"),
            "should explain the skip when no app is connected: {log}"
        );
    }

    #[test]
    fn unexpected_protocol_version_still_connects() {
        let (server, port, sink) = start_test_server(StubRecompiler {
            result: Ok(std::env::temp_dir().join("x.dylib")),
        });
        let client = std::thread::spawn(move || {
            let _s = connect_and_handshake(port, protocol::INJECTION_VERSION + 999);
            std::thread::sleep(Duration::from_millis(300));
        });
        assert!(
            server.wait_connected(Duration::from_secs(5)),
            "should still connect despite a version mismatch"
        );
        client.join().unwrap();
        server.shutdown();
        assert!(
            sink.lock().unwrap().join("\n").contains("protocol"),
            "the version mismatch should be noted"
        );
    }
}
