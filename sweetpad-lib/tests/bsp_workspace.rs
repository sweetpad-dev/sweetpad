//! Multi-project `.xcworkspace` resolution: the BSP server must resolve a source
//! file to whichever *member* project of a workspace declares its target, not
//! just a single `.xcodeproj`. Driven against the committed CocoaPods fixture,
//! whose `App.xcworkspace` references two projects — `App.xcodeproj` (the app
//! sources) and `Pods/Pods.xcodeproj` (the vendored pod sources).
//!
//! The assertions: `workspace/buildTargets` lists targets from *both* member
//! projects, and `textDocument/sourceKitOptions` returns non-empty editor args
//! for a file in each — proving the server picked the right member project per
//! file. Fast and hermetic: scripted JSON-RPC, no `sourcekit-lsp`, no build.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn fixture(rel: &str) -> String {
    format!(
        "{}/fixtures/_synthetic-cocoapods/project/{rel}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn file_uri(rel: &str) -> String {
    format!("file://{}", fixture(rel))
}

fn frame(msg: &Value) -> Vec<u8> {
    let body = msg.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// Split a stream of `Content-Length`-framed messages into JSON values.
fn parse_frames(out: &[u8]) -> Vec<Value> {
    let text = String::from_utf8_lossy(out);
    let mut frames = Vec::new();
    let mut rest: &str = &text;
    while let Some(hdr) = rest.find("Content-Length:") {
        rest = &rest[hdr + "Content-Length:".len()..];
        let Some(sep) = rest.find("\r\n\r\n") else {
            break;
        };
        let len: usize = rest[..sep].trim().parse().unwrap_or(0);
        let start = sep + 4;
        let end = (start + len).min(rest.len());
        if let Ok(v) = serde_json::from_str::<Value>(&rest[start..end]) {
            frames.push(v);
        }
        rest = &rest[end..];
    }
    frames
}

/// Drive a scripted session against `bsp --workspace <ws>`, returning responses.
fn run_workspace_session(messages: &[Value], workspace: &str) -> Vec<Value> {
    let mut input = Vec::new();
    for m in messages {
        input.extend(frame(m));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_bsp-server"))
        .args(["bsp", "--workspace", workspace])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn bsp server");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&input)
        .expect("write stdin");
    let mut out = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut out)
        .expect("read stdout");
    let _ = child.wait();
    parse_frames(&out)
}

fn result_for(frames: &[Value], id: i64) -> Option<&Value> {
    frames
        .iter()
        .find(|f| f.get("id").and_then(Value::as_i64) == Some(id))
        .and_then(|f| f.get("result"))
}

fn options_args(frames: &[Value], id: i64) -> Vec<String> {
    result_for(frames, id)
        .and_then(|r| r.get("compilerArguments"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn workspace_resolves_files_across_member_projects() {
    let workspace = fixture("App.xcworkspace");
    if !Path::new(&workspace).exists() {
        eprintln!("skipping: cocoapods workspace fixture absent at {workspace}");
        return;
    }
    let app_file = file_uri("App/Sources/AppMain.swift");
    let pod_file = file_uri("Pods/SwiftyJSON/Source/SwiftyJSON/SwiftyJSON.swift");

    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{"rootUri":format!("file://{}", fixture(""))}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"workspace/buildTargets"}),
        json!({"jsonrpc":"2.0","id":5,"method":"textDocument/sourceKitOptions","params":{"textDocument":{"uri":app_file}}}),
        json!({"jsonrpc":"2.0","id":6,"method":"textDocument/sourceKitOptions","params":{"textDocument":{"uri":pod_file}}}),
        json!({"jsonrpc":"2.0","id":9,"method":"build/shutdown"}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_workspace_session(&messages, &workspace);

    // buildTargets spans both member projects: the App target (App.xcodeproj) and
    // at least one pod target (Pods.xcodeproj).
    let targets = result_for(&frames, 2)
        .and_then(|r| r.get("targets"))
        .and_then(Value::as_array)
        .expect("buildTargets result");
    let names: Vec<String> = targets
        .iter()
        .filter_map(|t| {
            t.get("displayName")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    assert!(
        names.iter().any(|n| n == "App"),
        "App target (App.xcodeproj) missing: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n.contains("SwiftyJSON") || n.contains("Pods")),
        "no Pods.xcodeproj target listed — the workspace's second project wasn't seen: {names:?}"
    );

    // A file in each member project resolves to non-empty editor args — the server
    // picked the right project per file.
    let app_args = options_args(&frames, 5);
    assert!(
        app_args.iter().any(|a| a == "-module-name"),
        "App/Sources/AppMain.swift (App.xcodeproj) did not resolve: {app_args:?}"
    );
    let pod_args = options_args(&frames, 6);
    assert!(
        pod_args.iter().any(|a| a == "-module-name"),
        "Pods SwiftyJSON.swift (Pods.xcodeproj) did not resolve — cross-project resolution failed: {pod_args:?}"
    );
}
