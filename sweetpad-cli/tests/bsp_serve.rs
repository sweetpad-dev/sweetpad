//! `bsp serve` with nothing naming its container serves the project in its
//! working directory, so a `buildServer.json` the extension never wrote, like
//! one committed to a repository, still starts the server.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

use common::TempDir;
use serde_json::{Value, json};

const PBXPROJ: &str = r#"// !$*UTF8*$!
{
	archiveVersion = 1;
	classes = {
	};
	objectVersion = 77;
	objects = {
		C1 = {isa = XCBuildConfiguration; buildSettings = {}; name = Debug; };
		L1 = {isa = XCConfigurationList; buildConfigurations = (C1, ); defaultConfigurationIsVisible = 0; defaultConfigurationName = Debug; };
		G1 = {isa = PBXGroup; children = (); sourceTree = "<group>"; };
		P1 = {isa = PBXProject; buildConfigurationList = L1; mainGroup = G1; targets = (); };
	};
	rootObject = P1;
}
"#;

/// `sweetpad bsp serve` started in `dir`, with a state folder of its own, so
/// no folder is registered in the extension's discovery index.
fn serve(dir: &Path, state: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(["bsp", "serve"])
        .current_dir(dir)
        .env("XDG_STATE_HOME", state)
        .env("SWEETPAD_BSP_LOG", state.join("bsp.log"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn send(stdin: &mut impl Write, message: &Value) {
    let body = message.to_string();
    write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    stdin.flush().unwrap();
}

/// The next message, or `None` when the server closed its output.
fn receive(out: &mut BufReader<ChildStdout>) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if out.read_line(&mut line).unwrap() == 0 {
            return None;
        }
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    out.read_exact(&mut body).unwrap();
    Some(serde_json::from_slice(&body).unwrap())
}

#[test]
fn serve_with_no_config_serves_the_project_in_its_working_directory() {
    let dir = TempDir::new("sweetpad-bsp-serve");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("App.xcodeproj")).unwrap();
    std::fs::write(dir.join("App.xcodeproj/project.pbxproj"), PBXPROJ).unwrap();
    let state = TempDir::new("sweetpad-bsp-serve-state");
    let mut child = serve(&dir, &state);
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let root = format!("file://{}", dir.display());
    send(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "build/initialize", "params": {
            "displayName": "test", "version": "1", "bspVersion": "2.1.0", "rootUri": root,
            "capabilities": {"languageIds": ["swift"]}}}),
    );
    let mut response = None;
    while let Some(message) = receive(&mut out) {
        if message["id"] == 1 {
            response = Some(message);
            break;
        }
    }
    send(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "build/shutdown"}),
    );
    send(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "method": "build/exit"}),
    );
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    let response = response.unwrap_or_else(|| {
        panic!(
            "no answer to build/initialize: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(response.get("result").is_some(), "{response}");
}

#[test]
fn serve_with_no_config_and_no_project_says_what_names_one() {
    let dir = TempDir::new("sweetpad-bsp-serve-empty");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let state = TempDir::new("sweetpad-bsp-serve-empty-state");
    let output = serve(&dir, &state).wait_with_output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no .xcworkspace or .xcodeproj"), "{stderr}");
}
