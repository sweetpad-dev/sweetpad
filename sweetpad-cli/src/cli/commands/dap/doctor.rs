//! `sweetpad dap doctor`: can this machine serve a debug session? It checks
//! that lldb-dap resolves (Xcode's through `xcrun`, or `SWEETPAD_LLDB_DAP`),
//! that the selected Xcode is one whose LLDB has the commands the adapter
//! drives, and finally starts lldb-dap and asks it to initialize.

use std::io::BufReader;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::json;
use sweetpad_core::framing;

use crate::cli::output::Output;
use crate::cli::{CommandResult, Context, Render, Rendered, process};

/// The oldest Xcode whose toolchain ships lldb-dap and LLDB's `device`
/// commands.
const MIN_XCODE: u32 = 16;

struct Check {
    ok: bool,
    what: String,
}

struct DapDoctor {
    lldb_dap: Option<String>,
    checks: Vec<Check>,
}

impl Render for DapDoctor {
    fn human(&self, out: &Output) {
        out.line(&format!(
            "lldb-dap: {}",
            self.lldb_dap.as_deref().unwrap_or("not found")
        ));
        for c in &self.checks {
            out.line(&format!("  {} {}", if c.ok { "✓" } else { "✗" }, c.what));
        }
    }

    fn json(&self) -> serde_json::Value {
        let checks: Vec<serde_json::Value> = self
            .checks
            .iter()
            .map(|c| json!({ "ok": c.ok, "check": c.what }))
            .collect();
        json!({ "lldbDap": self.lldb_dap, "checks": checks })
    }
}

#[allow(clippy::unnecessary_wraps)] // uniform CommandResult across the dap actions
pub(super) fn run(_ctx: &mut Context) -> CommandResult {
    let mut checks = Vec::new();
    let (program, args) = super::lldb_dap_command();
    let overridden = args.is_empty();
    let lldb_dap = if overridden {
        Some(program.clone()).filter(|p| std::path::Path::new(p).exists())
    } else {
        process::capture("xcrun", &["--find", "lldb-dap"], None)
            .ok()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
    };
    checks.push(Check {
        ok: lldb_dap.is_some(),
        what: match (&lldb_dap, overridden) {
            (Some(_), true) => "SWEETPAD_LLDB_DAP names an existing file".to_string(),
            (Some(_), false) => "'xcrun lldb-dap' resolves".to_string(),
            (None, true) => {
                format!("SWEETPAD_LLDB_DAP names an existing file ({program} is missing)")
            }
            (None, false) => format!(
                "'xcrun lldb-dap' resolves (it ships with Xcode {MIN_XCODE} and later; select \
                 one with 'xcode-select --switch' or DEVELOPER_DIR, or point SWEETPAD_LLDB_DAP \
                 at an lldb-dap)"
            ),
        },
    });
    if !overridden {
        checks.push(xcode_check());
    }
    if lldb_dap.is_some() {
        checks.push(probe(&program, &args));
    }
    let failed = checks.iter().any(|c| !c.ok);
    let report = DapDoctor { lldb_dap, checks };
    Ok(if failed {
        Rendered::data_with_exit(report, 1)
    } else {
        Rendered::data(report)
    })
}

/// The selected Xcode's major version, against [`MIN_XCODE`].
fn xcode_check() -> Check {
    let version = process::capture("xcodebuild", &["-version"], None)
        .ok()
        .and_then(|out| out.lines().next().map(str::to_string));
    let major = version
        .as_deref()
        .and_then(|v| v.strip_prefix("Xcode "))
        .and_then(|v| v.split('.').next())
        .and_then(|m| m.trim().parse::<u32>().ok());
    match (version, major) {
        (Some(v), Some(m)) if m >= MIN_XCODE => Check {
            ok: true,
            what: format!("{v} is new enough"),
        },
        (Some(v), _) => Check {
            ok: false,
            what: format!("{v} is new enough (debugging needs Xcode {MIN_XCODE} or later)"),
        },
        (None, _) => Check {
            ok: false,
            what: "'xcodebuild -version' answers (select an Xcode with 'xcode-select --switch')"
                .to_string(),
        },
    }
}

/// The definitive check: start lldb-dap, send `initialize`, and wait for its
/// answer.
fn probe(program: &str, args: &[String]) -> Check {
    let what = "lldb-dap answers 'initialize'";
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(e) => {
            return Check {
                ok: false,
                what: format!("{what} (it didn't start: {e})"),
            };
        }
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return Check {
            ok: false,
            what: format!("{what} (it started without pipes)"),
        };
    };
    let request = json!({
        "seq": 1,
        "type": "request",
        "command": "initialize",
        "arguments": {
            "clientID": "sweetpad-doctor",
            "adapterID": "sweetpad",
            "pathFormat": "path",
        },
    });
    let _ = framing::write_message(&mut stdin, &request.to_string());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let _ = tx.send(framing::read_message(&mut reader));
    });
    let answer = rx.recv_timeout(Duration::from_secs(15));
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let ok = matches!(&answer, Ok(Ok(Some(body)))
        if serde_json::from_str::<serde_json::Value>(body)
            .is_ok_and(|m| m["command"] == "initialize" && m["success"] == true));
    Check {
        ok,
        what: if ok {
            what.to_string()
        } else {
            format!("{what} (no answer within 15s)")
        },
    }
}
