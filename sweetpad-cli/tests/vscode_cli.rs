//! End-to-end oracle for the `sweetpad vscode` command: a mock extension
//! control server on a Unix socket (Content-Length-framed JSON-RPC 2.0, like
//! the real `cli-server`), a discovery index (`projects.json` under a private
//! `XDG_STATE_HOME`) pointing at it, and the `sweetpad` binary execed against
//! both — pinning discovery, the request wire shape, output formatting, the
//! error envelope, and the exit codes (0 ok / 1 RPC error / 2 client error)
//! the JS CLI established.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

fn temp_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("sweetpad-vscode-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Read one Content-Length-framed message (the request) from the stream.
fn read_frame(reader: &mut impl BufRead) -> Value {
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            len = value.trim().parse().unwrap();
        }
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).unwrap();
    serde_json::from_slice(&buf).unwrap()
}

fn write_frame(writer: &mut impl Write, body: &Value) {
    let body = body.to_string();
    write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    writer.flush().unwrap();
}

/// Start a one-shot mock server: accept one connection, read one request,
/// answer with `respond(request)`. Returns the request it saw.
fn spawn_server(
    socket: &Path,
    respond: impl FnOnce(&Value) -> Value + Send + 'static,
) -> std::thread::JoinHandle<Value> {
    let listener = UnixListener::bind(socket).unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let request = read_frame(&mut reader);
        let mut writer = stream;
        write_frame(&mut writer, &respond(&request));
        request
    })
}

/// The private `XDG_STATE_HOME` for a project dir, so the test never touches the
/// developer's real `~/.local/state`.
fn state_home(root: &Path) -> PathBuf {
    root.join("xdg-state")
}

/// Register `root` in the discovery index advertising `socket` as its control
/// server, keyed by the canonical project path the way the extension writes it.
fn register_project(root: &Path, socket: &Path) {
    let dir = state_home(root).join("sweetpad");
    std::fs::create_dir_all(&dir).unwrap();
    let key = std::fs::canonicalize(root).unwrap();
    let index = json!({
        "version": 1,
        "projects": { key.to_str().unwrap(): { "control": { "socket": socket } } },
    });
    std::fs::write(dir.join("projects.json"), index.to_string()).unwrap();
}

fn run_cli(cwd: &Path, root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .arg("vscode")
        .args(args)
        .current_dir(cwd)
        .env("XDG_STATE_HOME", state_home(root))
        .output()
        .expect("run sweetpad vscode")
}

fn stderr_envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stderr).unwrap_or_else(|_| {
        panic!(
            "stderr is not JSON: {:?}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn success_round_trip_pretty_and_request_shape() {
    let dir = temp_dir("ok");
    let socket = dir.join("srv.sock");
    register_project(&dir, &socket);
    let server = spawn_server(
        &socket,
        |req| json!({ "jsonrpc": "2.0", "id": req["id"], "result": { "schemes": ["App"] } }),
    );

    // Run from a nested directory: discovery must walk up to the project root.
    let nested = dir.join("Sources/Deep");
    std::fs::create_dir_all(&nested).unwrap();
    let output = run_cli(&nested, &dir, &["scheme.list"]);

    let request = server.join().unwrap();
    assert_eq!(request["jsonrpc"], "2.0");
    assert_eq!(request["method"], "scheme.list");
    assert_eq!(request["params"], json!({}));

    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Pretty by default (multi-line, 2-space indent).
    assert!(stdout.contains("\n  "), "expected pretty JSON: {stdout:?}");
    let parsed: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed, json!({ "schemes": ["App"] }));
}

#[test]
fn raw_minifies_and_flags_reach_the_wire() {
    let dir = temp_dir("raw");
    let socket = dir.join("srv.sock");
    register_project(&dir, &socket);
    let server = spawn_server(
        &socket,
        |req| json!({ "jsonrpc": "2.0", "id": req["id"], "result": req["params"] }),
    );

    let output = run_cli(
        &dir,
        &dir,
        &["build.wait", "b1", "--timeout", "5s", "--raw"],
    );

    let request = server.join().unwrap();
    assert_eq!(
        request["params"],
        json!({ "buildId": "b1", "timeoutMs": 5000 })
    );
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.trim(), r#"{"buildId":"b1","timeoutMs":5000}"#);
}

#[test]
fn string_results_print_bare() {
    let dir = temp_dir("str");
    let socket = dir.join("srv.sock");
    register_project(&dir, &socket);
    spawn_server(
        &socket,
        |req| json!({ "jsonrpc": "2.0", "id": req["id"], "result": "/Users/me/Proj.xcworkspace" }),
    );

    let output = run_cli(&dir, &dir, &["meta.workspacePath"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "/Users/me/Proj.xcworkspace"
    );
}

#[test]
fn rpc_errors_exit_1_with_the_servers_stable_code() {
    let dir = temp_dir("err");
    let socket = dir.join("srv.sock");
    register_project(&dir, &socket);
    spawn_server(&socket, |req| {
        json!({
            "jsonrpc": "2.0",
            "id": req["id"],
            "error": {
                "code": -32000,
                "message": "no scheme selected",
                "data": { "code": "NO_SCHEME", "hint": "run scheme.set first" }
            }
        })
    });

    let output = run_cli(&dir, &dir, &["scheme.get"]);
    assert_eq!(output.status.code(), Some(1));
    let envelope = stderr_envelope(&output);
    assert_eq!(envelope["ok"], json!(false));
    assert_eq!(envelope["error"]["code"], json!("NO_SCHEME"));
    assert_eq!(envelope["error"]["message"], json!("no scheme selected"));
    assert_eq!(envelope["error"]["hint"], json!("run scheme.set first"));
    assert_eq!(envelope["error"]["data"]["code"], json!("NO_SCHEME"));
}

#[test]
fn missing_project_and_dead_socket_exit_2() {
    // No index registered (the private state home has none).
    let dir = temp_dir("none");
    let output = run_cli(&dir, &dir, &["scheme.list"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr_envelope(&output)["error"]["code"],
        json!("NO_SERVER")
    );

    // The index points at a socket nobody listens on.
    let dir = temp_dir("dead");
    register_project(&dir, &dir.join("gone.sock"));
    let output = run_cli(&dir, &dir, &["scheme.list"]);
    assert_eq!(output.status.code(), Some(2));
    let envelope = stderr_envelope(&output);
    assert_eq!(envelope["error"]["code"], json!("CLI_ERROR"));
    let message = envelope["error"]["message"].as_str().unwrap();
    assert!(message.contains("ENOENT"), "message: {message}");
}

#[test]
fn usage_paths_exit_2() {
    let dir = temp_dir("usage");
    // No method / bare word / --help all print the USAGE envelope.
    for args in [&[][..], &["schemes"][..], &["--help"][..]] {
        let output = run_cli(&dir, &dir, args);
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        let envelope = stderr_envelope(&output);
        assert_eq!(envelope["error"]["code"], json!("USAGE"));
        assert!(
            envelope["error"]["message"]
                .as_str()
                .unwrap()
                .contains("sweetpad vscode <method>"),
            "args: {args:?}"
        );
    }

    // The top-level binary itself: unknown command → 2, --help → 0.
    let unknown = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .arg("frobnicate")
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
    let help = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout).unwrap().contains("vscode"));
}
