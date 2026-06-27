//! v2 of the BSP loop: `buildTarget/prepare` builds a target's dependency
//! modules on demand, so cross-module `import`s resolve with **no prior build**.
//!
//! This drives the `bsp-server bsp` server directly (no sourcekit-lsp): from a
//! clean DerivedData, it sends `buildTarget/prepare` for `ModuleB` and asserts
//! (a) the server answers the request and (b) `ModuleA`'s `.swiftmodule` now
//! exists in the products dir our search paths point at — i.e. preparation
//! produced the dependency module.
//!
//! Opt-in: runs `xcodebuild`, so gated on `BSP_ORACLE=1` (+ Xcode 26.5).

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const XCODE: &str = "/Applications/Xcode-26.5.0.app";

fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

#[test]
fn prepare_builds_dependency_module_from_clean_deriveddata() {
    if std::env::var("BSP_ORACLE").is_err() {
        eprintln!("skipping: set BSP_ORACLE=1 to run the BSP prepare oracle");
        return;
    }
    if !Path::new(XCODE).exists() {
        eprintln!("skipping: {XCODE} not installed");
        return;
    }

    let root = env!("SWEETPAD_LIB_DIR");
    let project = format!("{root}/fixtures/_synthetic-multimodule/project/MultiModule.xcodeproj");
    let dd = std::env::temp_dir().join(format!("sweetpad-bsp-prep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dd);
    let dep_module = dd.join("Build/Products/Debug/ModuleA.swiftmodule");
    let log = std::env::temp_dir().join(format!("sweetpad-bsp-prep-log-{}", std::process::id()));
    let _ = std::fs::remove_file(&log);

    let mut child = Command::new(env!("CARGO_BIN_EXE_bsp-server"))
        .args(["bsp", "--project", &project, "--xcode"])
        .arg(format!("{XCODE}/Contents/Developer"))
        .arg("--derived-data-path")
        .arg(&dd)
        .env("SWEETPAD_BSP_LOG", &log)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let buf = Arc::new(Mutex::new(String::new()));
    let buf_reader = Arc::clone(&buf);
    let reader = std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut chunk) {
            if n == 0 {
                break;
            }
            buf_reader
                .lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
    });

    stdin
        .write_all(&frame(
            r#"{"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}"#,
        ))
        .unwrap();
    stdin
        .write_all(&frame(r#"{"jsonrpc":"2.0","method":"build/initialized"}"#))
        .unwrap();
    // prepare ModuleB → must build its dependency ModuleA's module.
    stdin.write_all(&frame(r#"{"jsonrpc":"2.0","id":10,"method":"buildTarget/prepare","params":{"targets":[{"uri":"sweetpad://target/ModuleB"}]}}"#)).unwrap();
    stdin.flush().unwrap();

    // Wait (up to 3 min) for the prepare response.
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut replied = false;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        if buf.lock().unwrap().contains(r#""id":10"#) {
            replied = true;
            break;
        }
    }
    let _ = stdin.write_all(&frame(r#"{"jsonrpc":"2.0","method":"build/exit"}"#));
    let _ = stdin.flush();
    drop(stdin);
    let _ = child.wait();
    let _ = reader.join();

    let module_exists = dep_module.exists();
    let log_text = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dd);
    let _ = std::fs::remove_file(&log);

    assert!(replied, "server never answered buildTarget/prepare");
    assert!(
        module_exists,
        "prepare did not produce the dependency module at {}",
        dep_module.display()
    );
    // v3 fast path: the pure-Swift closure is emitted by swiftc, not xcodebuild.
    assert!(
        log_text.contains("emitted module ModuleA"),
        "expected the swiftc self-build fast path; log:\n{log_text}"
    );
    assert!(
        !log_text.contains("building scheme"),
        "should not have fallen back to xcodebuild for a pure-Swift closure; log:\n{log_text}"
    );
}
