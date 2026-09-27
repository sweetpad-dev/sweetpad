//! Layer 2 of the BSP measurement loop (see `DOCS.md` §8 (BSP server)): end-to-end through a
//! real, headless `sourcekit-lsp`. This is the closest thing to "does the editor
//! actually work" — it exercises the whole stack: `sourcekit-lsp` discovers our
//! `buildServer.json`, launches `bsp-server bsp`, asks it for a file's
//! `sourceKitOptions`, and analyzes the file with those args.
//!
//! The assertion: opening `ModuleB/b.swift` (which `import ModuleA`) produces **no
//! module-resolution diagnostics** — i.e. our BSP server fed `sourcekit-lsp`
//! arguments that resolve the cross-module import.
//!
//! Opt-in (`BSP_ORACLE=1`): it copies the fixture to a temp dir, builds it with
//! `xcodebuild`, and runs `sourcekit-lsp` from the Xcode `BSP_ORACLE_XCODE`
//! names, else the selected one ([`oracle_xcode`]). The build and
//! `sourcekit-lsp`, with the server it launches, get a `TMPDIR` in that temp
//! dir.

mod oracle_xcode;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use oracle_xcode::OracleXcode;
use serde_json::{Value, json};
use sweetpad_core::scratch::ScratchDir;

/// The Xcode to run against and its `sourcekit-lsp`, when `BSP_ORACLE` is set
/// and that Xcode has one; prints why when it doesn't.
fn gated(what: &str) -> Option<(OracleXcode, PathBuf)> {
    if std::env::var("BSP_ORACLE").is_err() {
        eprintln!("skipping: set BSP_ORACLE=1 to run the {what}");
        return None;
    }
    let xcode = oracle_xcode::find()?;
    let sourcekit_lsp = xcode.tool("sourcekit-lsp");
    if !sourcekit_lsp.exists() {
        eprintln!("skipping: {} not found", sourcekit_lsp.display());
        return None;
    }
    Some((xcode, sourcekit_lsp))
}

/// A `TMPDIR` inside `root`, which goes with it.
fn tmpdir(root: &Path) -> PathBuf {
    let tmp = root.join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    tmp
}

fn lsp_frame(msg: &Value) -> Vec<u8> {
    let body = msg.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// Pull the target uri from a `textDocument/definition` result
/// (`Location` | `Location[]` | `LocationLink[]`).
fn definition_uri(result: Option<&Value>) -> Option<String> {
    let v = result?;
    if v.is_null() {
        return None;
    }
    let loc = if let Some(arr) = v.as_array() {
        arr.first()?
    } else {
        v
    };
    loc.get("uri")
        .or_else(|| loc.get("targetUri"))
        .and_then(Value::as_str)
        .map(String::from)
}

/// Copy the committed fixture into `dst` (so the build artifacts + buildServer.json
/// don't touch the tracked tree).
fn copy_fixture(dst: &Path) {
    let src =
        PathBuf::from(env!("SWEETPAD_LIB_DIR")).join("fixtures/_synthetic-multimodule/project");
    let status = Command::new("cp")
        .arg("-R")
        .arg(&src)
        .arg(dst)
        .status()
        .expect("cp fixture");
    assert!(status.success(), "failed to copy fixture");
}

/// Read one `Content-Length`-framed LSP message; `None` on EOF.
fn read_lsp(reader: &mut impl BufRead) -> Option<Value> {
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            len = v.trim().parse().ok()?;
        }
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).ok()?;
    serde_json::from_str(&String::from_utf8_lossy(&buf)).ok()
}

#[test]
#[allow(clippy::too_many_lines)] // a linear end-to-end harness reads clearer in one piece
fn bsp_lsp_e2e() {
    let Some((xcode, sourcekit_lsp)) = gated("sourcekit-lsp end-to-end oracle") else {
        return;
    };

    // Isolate everything in a temp copy of the fixture.
    let root = ScratchDir::new("sweetpad-lsp").unwrap();
    copy_fixture(&root);
    let project_dir = root.join("project");
    let xcodeproj = project_dir.join("MultiModule.xcodeproj");
    let dd = root.join("dd");
    let tmp = tmpdir(&root);

    // Build so ModuleA.swiftmodule exists where the args point.
    let build = xcode
        .command("xcodebuild", &tmp)
        .args(["build", "-project"])
        .arg(&xcodeproj)
        .args([
            "-scheme",
            "ModuleB",
            "-configuration",
            "Debug",
            "-destination",
            "platform=macOS",
            "-derivedDataPath",
        ])
        .arg(&dd)
        .arg("CODE_SIGNING_ALLOWED=NO")
        .output()
        .expect("xcodebuild");
    assert!(
        build.status.success(),
        "fixture build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    // Point sourcekit-lsp at our server by generating buildServer.json with the
    // real `config` command (dog-foods it end-to-end).
    let bsp_bin = env!("CARGO_BIN_EXE_bsp-server");
    let config = Command::new(bsp_bin)
        .args(["config", "--project"])
        .arg(&xcodeproj)
        .arg("--xcode")
        .arg(&xcode.path)
        .arg("--derived-data-path")
        .arg(&dd)
        .arg("--output")
        .arg(project_dir.join("buildServer.json"))
        .status()
        .expect("run config");
    assert!(config.success(), "config command failed");

    // Launch sourcekit-lsp.
    let mut lsp = xcode
        .command(&sourcekit_lsp, &tmp)
        .current_dir(&project_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sourcekit-lsp");

    let mut stdin = lsp.stdin.take().unwrap();
    let stdout = lsp.stdout.take().unwrap();
    // Reader thread → channel of messages (diagnostics arrive asynchronously).
    let (tx, rx) = mpsc::channel::<Value>();
    let reader = std::thread::spawn(move || {
        let mut r = BufReader::new(stdout);
        while let Some(msg) = read_lsp(&mut r) {
            if tx.send(msg).is_err() {
                break;
            }
        }
    });

    let root_uri = format!("file://{}", project_dir.to_string_lossy());
    let b_path = project_dir.join("ModuleB/b.swift");
    let b_uri = format!("file://{}", b_path.to_string_lossy());
    let b_text = std::fs::read_to_string(&b_path).unwrap();

    let send = |stdin: &mut std::process::ChildStdin, msg: &Value| {
        let _ = stdin.write_all(&lsp_frame(msg));
        let _ = stdin.flush();
    };

    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"processId":std::process::id(),"rootUri":root_uri,"capabilities":{},"initializationOptions":{}}
        }),
    );
    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":b_uri,"languageId":"swift","version":1,"text":b_text}}
        }),
    );

    // Collect diagnostics for b.swift within a window.
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut diags_for_b: Option<Vec<Value>> = None;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(msg) => {
                if msg.get("method").and_then(Value::as_str)
                    == Some("textDocument/publishDiagnostics")
                {
                    let uri = msg
                        .pointer("/params/uri")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if uri.ends_with("/ModuleB/b.swift") {
                        let d = msg
                            .pointer("/params/diagnostics")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default();
                        diags_for_b = Some(d);
                        break;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Positive cross-module navigation: jump-to-definition on `Greeter` (line 3,
    // `    Greeter().greet()`) should resolve into ModuleA's a.swift via the
    // index store we advertised. Best-effort — sourcekit-lsp indexes
    // asynchronously, so a miss within the window is logged, not failed; a hit
    // must land in ModuleA.
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0","id":50,"method":"textDocument/definition",
            "params":{"textDocument":{"uri":b_uri},"position":{"line":3,"character":4}}
        }),
    );
    let mut def_uri: Option<String> = None;
    let def_deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < def_deadline {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(msg) if msg.get("id").and_then(Value::as_i64) == Some(50) => {
                def_uri = definition_uri(msg.get("result"));
                break;
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Shut down.
    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":99,"method":"shutdown"}),
    );
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(stdin);
    let _ = lsp.wait();
    let _ = reader.join();

    match &def_uri {
        Some(u) => {
            eprintln!("definition(Greeter) -> {u}");
            assert!(
                u.contains("/ModuleA/") || u.ends_with("a.swift"),
                "definition resolved outside ModuleA: {u}"
            );
        }
        None => {
            eprintln!("definition(Greeter): no result within window (index async) — not failing");
        }
    }

    let diags = diags_for_b.expect("no diagnostics published for b.swift within the window");
    let module_errors: Vec<&str> = diags
        .iter()
        .filter_map(|d| d.get("message").and_then(Value::as_str))
        .filter(|m| is_module_resolution_error(m))
        .collect();
    eprintln!(
        "b.swift diagnostics: {} total, {} module-resolution",
        diags.len(),
        module_errors.len()
    );
    assert!(
        module_errors.is_empty(),
        "sourcekit-lsp couldn't resolve the cross-module import via our BSP server: {module_errors:?}"
    );
}

fn is_module_resolution_error(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("no such module")
        || m.contains("cannot find")
        || m.contains("could not build module")
}

/// v2 end-to-end: the headline promise — cross-module `import` resolves with **no
/// prior build**. From a clean DerivedData, background indexing has sourcekit-lsp
/// call `buildTarget/prepare` on our server, which builds the dependency module
/// on demand. Drive it deterministically: open `b.swift`, `workspace/synchronize`
/// (blocks until prepare + indexing finish), then pull diagnostics — zero
/// module-resolution errors.
#[test]
#[allow(clippy::too_many_lines)]
fn prepare_resolves_cross_module_without_prior_build() {
    let Some((xcode, sourcekit_lsp)) = gated("sourcekit-lsp prepare end-to-end oracle") else {
        return;
    };

    let root = ScratchDir::new("sweetpad-lsp-prep").unwrap();
    copy_fixture(&root);
    let project_dir = root.join("project");
    let xcodeproj = project_dir.join("MultiModule.xcodeproj");
    let dd = root.join("dd"); // clean — deliberately NOT built up front
    let tmp = tmpdir(&root);

    // buildServer.json → our server, pointed at the clean DerivedData; prepare
    // will build the dependency module into it on demand.
    let bsp_bin = env!("CARGO_BIN_EXE_bsp-server");
    let config = Command::new(bsp_bin)
        .args(["config", "--project"])
        .arg(&xcodeproj)
        .arg("--xcode")
        .arg(&xcode.path)
        .arg("--derived-data-path")
        .arg(&dd)
        .arg("--output")
        .arg(project_dir.join("buildServer.json"))
        .status()
        .expect("run config");
    assert!(config.success(), "config command failed");

    let mut lsp = xcode
        .command(&sourcekit_lsp, &tmp)
        .current_dir(&project_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sourcekit-lsp");
    let mut stdin = lsp.stdin.take().unwrap();
    let stdout = lsp.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel::<Value>();
    let reader = std::thread::spawn(move || {
        let mut r = BufReader::new(stdout);
        while let Some(msg) = read_lsp(&mut r) {
            if tx.send(msg).is_err() {
                break;
            }
        }
    });

    let root_uri = format!("file://{}", project_dir.to_string_lossy());
    let b_path = project_dir.join("ModuleB/b.swift");
    let b_uri = format!("file://{}", b_path.to_string_lossy());
    let b_text = std::fs::read_to_string(&b_path).unwrap();
    let send = |stdin: &mut std::process::ChildStdin, msg: &Value| {
        let _ = stdin.write_all(&lsp_frame(msg));
        let _ = stdin.flush();
    };

    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{
                "processId":std::process::id(),"rootUri":root_uri,
                "capabilities":{
                    "textDocument":{"diagnostic":{"dynamicRegistration":false}},
                    "workspace":{"diagnostics":{"refreshSupport":true}}
                },
                // Background indexing drives prepare; default-on in 6.1+ but explicit here.
                "initializationOptions":{"backgroundIndexing":true,"backgroundPreparationMode":"enabled"}
            }
        }),
    );
    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":b_uri,"languageId":"swift","version":1,"text":b_text}}
        }),
    );
    let wait_for_id = |rx: &mpsc::Receiver<Value>, want: i64, secs: u64| -> Option<Value> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(msg) if msg.get("id").and_then(Value::as_i64) == Some(want) => return Some(msg),
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            }
        }
        None
    };

    // Block until background work (prepare + index) settles. The build runs here.
    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":10,"method":"workspace/synchronize","params":{"index":true}}),
    );
    let synchronized = wait_for_id(&rx, 10, 180).is_some();
    // b.swift was first compiled at didOpen — before prepare built ModuleA — so
    // its diagnostics are cached as "no such module". A real editor re-pulls on
    // `workspace/diagnostic/refresh`; force the equivalent fresh compile with a
    // no-op version bump, then pull diagnostics.
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0","method":"textDocument/didChange",
            "params":{"textDocument":{"uri":b_uri,"version":2},"contentChanges":[{"text":b_text}]}
        }),
    );
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0","id":20,"method":"textDocument/diagnostic",
            "params":{"textDocument":{"uri":b_uri}}
        }),
    );
    let diag_report = wait_for_id(&rx, 20, 60);

    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":99,"method":"shutdown"}),
    );
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(stdin);
    let _ = lsp.wait();
    let _ = reader.join();
    // Confirm prepare actually built the dependency into the clean DerivedData.
    let dep_built = dd.join("Build/Products/Debug/ModuleA.swiftmodule").exists();

    assert!(
        synchronized,
        "workspace/synchronize never returned (prepare/index didn't settle)"
    );
    assert!(
        dep_built,
        "prepare did not build ModuleA into the clean DerivedData"
    );
    let report = diag_report.expect("no textDocument/diagnostic response");
    // Pull-diagnostic result is a full report: { kind: "full", items: [...] }.
    let items = report
        .pointer("/result/items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let module_errors: Vec<&str> = items
        .iter()
        .filter_map(|d| d.get("message").and_then(Value::as_str))
        .filter(|m| is_module_resolution_error(m))
        .collect();
    eprintln!(
        "b.swift (no prior build) diagnostics: {} total, {} module-resolution",
        items.len(),
        module_errors.len()
    );
    assert!(
        module_errors.is_empty(),
        "cross-module import did not resolve after prepare (no prior build): {module_errors:?}"
    );
}
