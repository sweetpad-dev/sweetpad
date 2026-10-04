//! `sweetpad dap` against a fake lldb-dap: the proxying an editor relies on,
//! with no Xcode involved. The fake answers every request, echoes the
//! arguments it received, and sends one reverse request, so the tests can
//! check what reached lldb-dap and what came back to the editor: the `seq`
//! renumbering, the trimmed capabilities, the rewritten requests, the
//! reverse-request routing, and a launch that fails before any build.

mod common;

use std::io::BufReader;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sweetpad_core::framing;

use common::TempDir;

/// The fake lldb-dap. Its reverse request carries seq 77, so the test can
/// see the editor's reply mapped back to it.
const FAKE: &str = r#"#!/usr/bin/env python3
import json, sys

def read():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            break
        key, value = line.decode().split(":", 1)
        headers[key.lower()] = value.strip()
    return json.loads(sys.stdin.buffer.read(int(headers["content-length"])))

def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()

def respond(request, body=None):
    send({"seq": 0, "type": "response", "request_seq": request["seq"],
          "command": request["command"], "success": True, "body": body or {}})

while True:
    message = read()
    if message is None:
        break
    if message["type"] == "request":
        command = message["command"]
        if command == "initialize":
            respond(message, {"supportsRestartRequest": True, "received": message["arguments"]})
        elif command in ("attach", "launch"):
            respond(message, {"received": message["arguments"]})
            send({"seq": 0, "type": "event", "event": "initialized"})
            send({"seq": 77, "type": "request", "command": "runInTerminal",
                  "arguments": {"args": ["true"]}})
        elif command == "disconnect":
            respond(message, {"received": message.get("arguments")})
            break
        else:
            respond(message)
    elif message["type"] == "response":
        send({"seq": 0, "type": "event", "event": "output",
              "body": {"category": "console",
                       "output": "reverse reply request_seq=%s\n" % message["request_seq"]}})
"#;

struct Editor {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
    seq: i64,
    last_seq: i64,
    _dirs: Vec<TempDir>,
}

impl Editor {
    /// Start `sweetpad dap` in `cwd` with the fake as its lldb-dap.
    fn start(tag: &str) -> (Self, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let home = TempDir::new(&format!("sweetpad-dap-{tag}-home"));
        let cwd = TempDir::new(&format!("sweetpad-dap-{tag}-cwd"));
        // Stop the project walk-up here.
        std::fs::create_dir_all(cwd.join(".git")).unwrap();
        let fake = home.join("lldb-dap");
        std::fs::write(&fake, FAKE).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
            .arg("dap")
            .current_dir(&cwd)
            .env("HOME", &home)
            .env("CFFIXED_USER_HOME", &home)
            .env("XDG_STATE_HOME", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("XDG_CACHE_HOME", &home)
            .env("SWEETPAD_LLDB_DAP", &fake)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, messages) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(body)) = framing::read_message(&mut reader) {
                if tx.send(serde_json::from_str(&body).unwrap()).is_err() {
                    return;
                }
            }
        });
        let cwd_path = cwd.to_path_buf();
        let editor = Editor {
            child,
            stdin,
            messages,
            seq: 0,
            last_seq: 0,
            _dirs: vec![home, cwd],
        };
        (editor, cwd_path)
    }

    fn send(&mut self, message: &Value) {
        framing::write_message(&mut self.stdin, &message.to_string()).unwrap();
    }

    fn request(&mut self, command: &str, arguments: &Value) -> i64 {
        self.seq += 1;
        let seq = self.seq;
        self.send(
            &json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments }),
        );
        seq
    }

    /// The next message matching `want`, checking every message's `seq` on
    /// the way.
    fn next(&mut self, what: &str, want: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = self
                .messages
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("no {what} within 20s"));
            let seq = message["seq"].as_i64().unwrap();
            assert!(
                seq > self.last_seq,
                "seq {seq} after {}: {message}",
                self.last_seq
            );
            self.last_seq = seq;
            if want(&message) {
                return message;
            }
        }
    }

    fn response(&mut self, seq: i64) -> Value {
        self.next("response", |m| {
            m["type"] == "response" && m["request_seq"] == seq
        })
    }

    /// Disconnect, and check the adapter exits on its own afterwards.
    fn disconnect(mut self) -> Value {
        let seq = self.request("disconnect", &json!({}));
        let response = self.response(seq);
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.child.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "the adapter kept running after disconnect"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(self.child.wait().unwrap().success());
        response
    }
}

fn initialize(editor: &mut Editor) -> Value {
    // No pathFormat: the protocol's default, which Xcode's lldb-dap insists on.
    let seq = editor.request(
        "initialize",
        &json!({ "clientID": "test", "linesStartAt1": true }),
    );
    editor.response(seq)
}

#[test]
fn proxies_an_attach_by_pid_with_its_own_seq_and_routes_reverse_requests() {
    let (mut editor, _) = Editor::start("attach");
    let init = initialize(&mut editor);
    assert_eq!(init["success"], true, "{init}");
    assert_eq!(init["body"]["received"]["pathFormat"], "path", "{init}");
    assert!(
        init["body"].get("supportsRestartRequest").is_none(),
        "restart isn't handled yet, so it mustn't be offered: {init}"
    );

    let seq = editor.request(
        "attach",
        &json!({ "type": "sweetpad", "request": "attach", "pid": 4242 }),
    );
    let attach = editor.response(seq);
    assert_eq!(attach["command"], "attach");
    assert_eq!(
        attach["body"]["received"],
        json!({ "pid": 4242 }),
        "{attach}"
    );
    editor.next("initialized", |m| m["event"] == "initialized");

    let reverse = editor.next("reverse request", |m| m["type"] == "request");
    assert_eq!(reverse["command"], "runInTerminal");
    let reply_to = reverse["seq"].clone();
    editor.seq += 1;
    let reply = json!({
        "seq": editor.seq, "type": "response", "request_seq": reply_to,
        "command": "runInTerminal", "success": true, "body": {},
    });
    editor.send(&reply);
    let routed = editor.next("the fake's report", |m| {
        m["event"] == "output"
            && m["body"]["output"]
                .as_str()
                .is_some_and(|o| o.starts_with("reverse reply"))
    });
    assert_eq!(routed["body"]["output"], "reverse reply request_seq=77\n");

    let disconnect = editor.disconnect();
    // An attach stays an attach: the app keeps running unless asked.
    assert_eq!(disconnect["body"]["received"], json!({}), "{disconnect}");
}

#[test]
fn passes_a_plain_lldb_dap_launch_through_untouched() {
    let (mut editor, _) = Editor::start("program");
    initialize(&mut editor);
    let arguments = json!({ "program": "/bin/echo", "args": ["hi"], "stopOnEntry": true });
    let seq = editor.request("launch", &arguments);
    let launch = editor.response(seq);
    assert_eq!(launch["command"], "launch");
    assert_eq!(launch["body"]["received"], arguments);
    editor.disconnect();
}

#[test]
fn a_launch_outside_a_project_fails_with_the_reason_and_the_session_ends_cleanly() {
    let (mut editor, cwd) = Editor::start("noproject");
    initialize(&mut editor);
    let seq = editor.request(
        "launch",
        &json!({ "type": "sweetpad", "request": "launch", "cwd": cwd, "scheme": "App" }),
    );
    let launch = editor.response(seq);
    assert_eq!(launch["success"], false, "{launch}");
    let message = launch["message"].as_str().unwrap();
    assert!(message.contains("no .xcworkspace"), "{message}");
    assert_eq!(launch["body"]["error"]["showUser"], true);
    editor.disconnect();
}

#[test]
fn a_bad_field_fails_the_launch_and_names_it() {
    let (mut editor, _) = Editor::start("badfield");
    initialize(&mut editor);
    let seq = editor.request("launch", &json!({ "type": "sweetpad", "args": "-Flag" }));
    let launch = editor.response(seq);
    assert_eq!(launch["success"], false, "{launch}");
    assert!(
        launch["message"].as_str().unwrap().contains("'args'"),
        "{launch}"
    );
    editor.disconnect();
}
