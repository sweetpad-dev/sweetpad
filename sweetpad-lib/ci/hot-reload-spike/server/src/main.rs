//! Milestone-1 hot-reload socket spike — the injection *server* half, in Rust.
//!
//! Proves the two load-bearing assumptions of `app run --hot` (CLI_DESIGN §9d):
//!   1. transport — a Rust server can speak the InjectionNext `:8887` protocol
//!      well enough that the in-app client completes its handshake, and
//!   2. build+load — a dylib we compile + link from a single changed source
//!      actually injects (the client replies `.injected`, not `.failed`).
//!
//! This is deliberately throwaway: no file watcher, no flag, no clap, no cache.
//! On the first client connection it reads the handshake, recompiles
//! `ContentView.swift` once (via the live build-log command — path A), links a
//! dylib, sends `.load`, and exits 0 iff the client replies `.injected`.
//!
//! Pure std on purpose so CI can `cargo build` it without fetching crates.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const PORT: u16 = 8887; // HOTRELOADING_PORT
const INJECTION_VERSION: i32 = 4001;

// InjectionCommand (server -> app), from InjectionNextC/include/InjectionClient.h
const CMD_LOAD: i32 = 1;
const CMD_XCODE_PATH: i32 = 3;

// InjectionResponse (app -> server)
const RSP_PLATFORM: i32 = 0;
const RSP_INJECTED: i32 = 1;
const RSP_FAILED: i32 = 2;
const RSP_TMP_PATH: i32 = 3;
const RSP_UNHIDE: i32 = 4;
const RSP_PROJECT_ROOT: i32 = 5;
const RSP_DETAIL: i32 = 6;
const RSP_BAZEL_TARGET: i32 = 7;
const RSP_EXECUTABLE: i32 = 8;

/// Inputs passed by the harness via env (keeps the binary argument-free).
struct Config {
    build_log: PathBuf,
    source: PathBuf,
    developer_dir: String,
    out_dir: PathBuf,
}

impl Config {
    fn from_env() -> Config {
        let get = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("missing env {k}"));
        Config {
            build_log: PathBuf::from(get("SPIKE_BUILD_LOG")),
            source: PathBuf::from(get("SPIKE_SOURCE")),
            developer_dir: get("SPIKE_DEVELOPER_DIR"),
            out_dir: PathBuf::from(
                std::env::var("SPIKE_OUT_DIR").unwrap_or_else(|_| "/tmp/hot-reload-spike".into()),
            ),
        }
    }
}

fn log(msg: &str) {
    println!("[spike-server] {msg}");
    let _ = std::io::stdout().flush();
}

fn main() {
    let cfg = Config::from_env();
    std::fs::create_dir_all(&cfg.out_dir).expect("create out dir");

    // Watchdog: never let a non-connecting client hang the CI job.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        log("❌ watchdog: no result within 60s (did the client connect?)");
        std::process::exit(2);
    });

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)).expect("bind :8887");
    log(&format!("listening on 127.0.0.1:{PORT}; waiting for the app to connect…"));

    // One client is enough for the spike.
    let (stream, peer) = listener.accept().expect("accept");
    log(&format!("client connected from {peer}"));
    // Generous for the first handshake bytes; tightened to 2s mid-handshake to
    // detect when the client has finished pushing and is awaiting commands.
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set read timeout");

    match run(stream, &cfg) {
        Ok(()) => {
            log("✅ injection confirmed (.injected received)");
            std::process::exit(0);
        }
        Err(e) => {
            log(&format!("❌ spike failed: {e}"));
            std::process::exit(1);
        }
    }
}

fn run(mut s: TcpStream, cfg: &Config) -> Result<(), String> {
    read_handshake(&mut s)?;

    // Tell the client which Xcode toolchain to reload against (best-effort).
    write_command(&mut s, CMD_XCODE_PATH, Some(&xcode_app_path(&cfg.developer_dir)))?;

    let dylib = recompile(cfg)?;
    log(&format!("sending .load {}", dylib.display()));
    write_command(&mut s, CMD_LOAD, Some(&dylib.to_string_lossy()))?;

    await_injected(&mut s)
}

// ---- protocol framing: native little-endian i32; string/data = i32 len + bytes ----

fn write_int(s: &mut TcpStream, v: i32) -> Result<(), String> {
    s.write_all(&v.to_le_bytes()).map_err(|e| format!("write int: {e}"))
}

fn write_string(s: &mut TcpStream, v: &str) -> Result<(), String> {
    write_int(s, v.len() as i32)?;
    s.write_all(v.as_bytes()).map_err(|e| format!("write string: {e}"))
}

fn write_command(s: &mut TcpStream, cmd: i32, str_arg: Option<&str>) -> Result<(), String> {
    write_int(s, cmd)?;
    if let Some(v) = str_arg {
        write_string(s, v)?;
    }
    Ok(())
}

/// Read exactly `n` bytes, distinguishing a clean timeout from EOF/error.
fn read_exact_timed(s: &mut TcpStream, buf: &mut [u8]) -> Result<bool, String> {
    let mut filled = 0;
    while filled < buf.len() {
        match s.read(&mut buf[filled..]) {
            Ok(0) => return Err("connection closed".into()),
            Ok(n) => filled += n,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Ok(false) // timeout
            }
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    Ok(true)
}

fn read_int(s: &mut TcpStream) -> Result<Option<i32>, String> {
    let mut b = [0u8; 4];
    if read_exact_timed(s, &mut b)? {
        Ok(Some(i32::from_le_bytes(b)))
    } else {
        Ok(None) // timeout
    }
}

fn read_string(s: &mut TcpStream) -> Result<String, String> {
    let len = loop {
        if let Some(v) = read_int(s)? {
            break v;
        }
    };
    if len < 0 {
        return Err("EOF reading string length".into());
    }
    let mut buf = vec![0u8; len as usize];
    let mut filled = 0;
    while filled < buf.len() {
        match s.read(&mut buf[filled..]) {
            Ok(0) => return Err("connection closed mid-string".into()),
            Ok(n) => filled += n,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(format!("read string body: {e}")),
        }
    }
    String::from_utf8(buf).map_err(|e| format!("utf8: {e}"))
}

/// Read the unsolicited handshake the client pushes on connect (InjectionNext.swift
/// `runInBackground`): version, home dir, then a short stream of responses. We read
/// responses until a read times out (the client then blocks waiting for our commands).
fn read_handshake(s: &mut TcpStream) -> Result<(), String> {
    let version = read_int(s)?.ok_or("timed out waiting for version")?;
    log(&format!("handshake: version {version}"));
    if version != INJECTION_VERSION {
        log(&format!("⚠️ version {version} != expected {INJECTION_VERSION}; continuing anyway"));
    }
    let home = read_string(s)?;
    log(&format!("handshake: home {home}"));

    // The remaining responses arrive back-to-back; a short timeout marks the end.
    s.set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| format!("set timeout: {e}"))?;

    loop {
        let Some(code) = read_int(s)? else {
            break; // timeout => client has finished pushing and is awaiting commands
        };
        match code {
            RSP_PLATFORM => {
                let platform = read_string(s)?;
                let arch = read_string(s)?; // sent as a bare trailing string
                log(&format!("handshake: platform {platform} arch {arch}"));
            }
            RSP_PROJECT_ROOT => log(&format!("handshake: projectRoot {}", read_string(s)?)),
            RSP_TMP_PATH => log(&format!("handshake: tmpPath {}", read_string(s)?)),
            RSP_EXECUTABLE => log(&format!("handshake: executable {}", read_string(s)?)),
            RSP_DETAIL => log(&format!("handshake: detail {}", read_string(s)?)),
            RSP_BAZEL_TARGET => log(&format!("handshake: bazelTarget {}", read_string(s)?)),
            other => log(&format!("handshake: unexpected response code {other}; ignoring")),
        }
    }
    log("handshake complete");
    Ok(())
}

/// Wait (longer) for the load result. `loadAndPatch` failure sends `.unhide`
/// then `.failed`; success sends `.injected`.
fn await_injected(s: &mut TcpStream) -> Result<(), String> {
    s.set_read_timeout(Some(Duration::from_secs(45)))
        .map_err(|e| format!("set timeout: {e}"))?;
    loop {
        let code = read_int(s)?.ok_or("timed out waiting for load result")?;
        match code {
            RSP_INJECTED => return Ok(()),
            RSP_FAILED => return Err("client reported .failed (dylib did not patch)".into()),
            RSP_UNHIDE => log("client reported .unhide (precedes .failed)"),
            RSP_DETAIL => log(&format!("client detail: {}", read_string(s)?)),
            other => log(&format!("post-load response {other}; ignoring")),
        }
    }
}

// ---- recompile (path A): live build-log command -> single-file .o -> .dylib ----

fn xcode_app_path(developer_dir: &str) -> String {
    // developer_dir is …/Xcode.app/Contents/Developer; the client wants …/Xcode.app.
    developer_dir
        .strip_suffix("/Contents/Developer")
        .unwrap_or(developer_dir)
        .to_string()
}

fn recompile(cfg: &Config) -> Result<PathBuf, String> {
    let source = cfg.source.to_string_lossy().to_string();
    let source_name = cfg
        .source
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("bad source path")?;

    let log_text = std::fs::read_to_string(&cfg.build_log)
        .map_err(|e| format!("read build log {}: {e}", cfg.build_log.display()))?;

    // Pick the frontend invocation where our source is the actual -primary-file
    // (a batch-mode build also emits lines where it's only a secondary input).
    let is_primary_line = |l: &str| {
        l.split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|w| w[0] == "-primary-file" && (w[1].ends_with(source_name) || w[1] == source))
    };
    let line = log_text
        .lines()
        .find(|l| l.contains("-primary-file") && is_primary_line(l))
        .or_else(|| {
            log_text
                .lines()
                .find(|l| l.contains("swift-frontend") && l.contains(source_name))
        })
        .ok_or_else(|| format!("no swift-frontend -primary-file command for {source_name} in build log"))?;
    log(&format!("matched frontend command ({} chars)", line.len()));

    // CI paths have no spaces, so a whitespace split is sufficient for the spike.
    // xcodebuild's transcript shell-escapes punctuation (e.g. `-enforce-exclusivity\=checked`);
    // since we exec the argv directly (no shell), strip those escapes per token.
    let tokens: Vec<String> = line.split_whitespace().map(unescape).collect();
    let object = cfg.out_dir.join("eval.o");
    let compile = single_file_command(&tokens, &source, &object)?;
    log(&format!("compile: {}", compile.join(" ")));
    run_tool(&compile, "compile")?;

    let dylib = cfg.out_dir.join("eval.dylib");
    let link = link_command(&tokens, &object, &dylib)?;
    log(&format!("link: {}", link.join(" ")));
    run_tool(&link, "link")?;

    Ok(dylib)
}

/// Flags whose following token is per-build output geometry we must drop.
const DROP_WITH_NEXT: &[&str] = &[
    "-o",
    "-output-file-map",
    "-supplementary-output-file-map",
    "-serialize-diagnostics-path",
    "-emit-dependencies-path",
    "-emit-reference-dependencies-path",
    "-emit-module-path",
    "-emit-module-doc-path",
    "-emit-module-source-info-path",
    "-emit-objc-header-path",
    "-index-store-path",
    "-index-unit-output-path",
    "-pch-output-dir",
];

const DROP_STANDALONE: &[&str] = &["-frontend-parseable-output", "-emit-module"];

/// Rewrite the recovered frontend command to compile *only* our source into one
/// object: drop output geometry, keep a single `-primary-file <source>` (other
/// primaries become plain secondary inputs), append `-c -o <object>`.
fn single_file_command(tokens: &[String], source: &str, object: &std::path::Path) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    let mut kept_primary = false;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        if DROP_WITH_NEXT.contains(&t) {
            i += 2;
            continue;
        }
        if DROP_STANDALONE.contains(&t) {
            i += 1;
            continue;
        }
        if t == "-primary-file" {
            let file = tokens.get(i + 1).ok_or("dangling -primary-file")?;
            if file.ends_with(source) || source.ends_with(file.as_str()) {
                out.push("-primary-file".into());
                out.push(file.clone());
                kept_primary = true;
            } else {
                // Demote other primaries to secondary inputs (keep the filename).
                out.push(file.clone());
            }
            i += 2;
            continue;
        }
        out.push(t.to_string());
        i += 1;
    }
    if !kept_primary {
        return Err(format!("source {source} was not a -primary-file in the command"));
    }
    // Ensure object output mode + our single output path.
    if !out.iter().any(|t| t == "-c" || t == "-emit-object") {
        out.push("-c".into());
    }
    out.push("-o".into());
    out.push(object.to_string_lossy().to_string());

    // If the frontend path isn't absolute, route through xcrun.
    if !out[0].contains('/') {
        let mut wrapped = vec!["xcrun".to_string()];
        wrapped.extend(out);
        out = wrapped;
    }
    Ok(out)
}

/// Strip shell-escaping backslashes from a transcript token (`a\=b` -> `a=b`).
fn unescape(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut chars = t.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn token_after<'a>(tokens: &'a [String], flag: &str) -> Option<&'a str> {
    tokens.iter().position(|t| t == flag).and_then(|i| tokens.get(i + 1)).map(|s| s.as_str())
}

/// Build the `clang` link line for a loadable simulator dylib, reusing the
/// build's own `-target`/`-sdk` so the ABI matches.
fn link_command(tokens: &[String], object: &std::path::Path, dylib: &std::path::Path) -> Result<Vec<String>, String> {
    let triple = token_after(tokens, "-target").ok_or("no -target in frontend command")?;
    let sdk = token_after(tokens, "-sdk")
        .map(|s| s.to_string())
        .or_else(|| {
            Command::new("xcrun")
                .args(["--sdk", "iphonesimulator", "--show-sdk-path"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .ok_or("could not resolve an SDK path")?;

    Ok(vec![
        "xcrun".into(),
        "clang".into(),
        "-target".into(),
        triple.into(),
        "-isysroot".into(),
        sdk,
        "-dynamiclib".into(),
        "-undefined".into(),
        "dynamic_lookup".into(),
        "-Xlinker".into(),
        "-interposable".into(),
        object.to_string_lossy().to_string(),
        "-o".into(),
        dylib.to_string_lossy().to_string(),
    ])
}

fn run_tool(argv: &[String], what: &str) -> Result<(), String> {
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .output()
        .map_err(|e| format!("spawn {what}: {e}"))?;
    if !out.stderr.is_empty() {
        log(&format!("{what} stderr:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{what} exited with {}", out.status))
    }
}
