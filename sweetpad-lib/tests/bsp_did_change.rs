//! The change-watcher pushes `buildTarget/didChange` when the project file is
//! edited mid-session, so the client re-queries targets/sources without an LSP
//! restart. Hermetic: copies the multi-module fixture to a temp dir (so its
//! pbxproj can be mutated), drives the server with a short watch interval, edits
//! the pbxproj, and checks the notification arrives.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap().flatten() {
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

#[test]
fn buildtarget_did_change_on_pbxproj_edit() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/_synthetic-multimodule/project");
    let tmp = std::env::temp_dir().join(format!("sweetpad-bsp-didchange-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    copy_dir(&src, &tmp);
    let proj = tmp.join("MultiModule.xcodeproj");
    let pbxproj = proj.join("project.pbxproj");

    let mut child = Command::new(env!("CARGO_BIN_EXE_sweetpad-lib"))
        .args(["bsp", "--project", proj.to_str().unwrap()])
        .env("SWEETPAD_BSP_WATCH_MS", "100")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    // Accumulate the server's output off-thread (its stdout stays open while the
    // watcher runs, so a plain read-to-end would block).
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let buf_reader = Arc::clone(&buf);
    let reader = std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut chunk) {
            if n == 0 {
                break;
            }
            buf_reader.lock().unwrap().extend_from_slice(&chunk[..n]);
        }
    });

    stdin.write_all(&frame(r#"{"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}"#)).unwrap();
    stdin.write_all(&frame(r#"{"jsonrpc":"2.0","method":"build/initialized"}"#)).unwrap();
    stdin.flush().unwrap();

    // Let the watcher capture the baseline stamp, then edit the pbxproj — insert
    // a comment after the header line so it stays valid OpenStep yet changes
    // (len, mtime).
    std::thread::sleep(Duration::from_millis(300));
    let content = fs::read_to_string(&pbxproj).unwrap();
    fs::write(&pbxproj, content.replacen('\n', "\n// touched by test\n", 1)).unwrap();

    // Give the 100 ms poll time to notice and push the notification.
    std::thread::sleep(Duration::from_millis(600));
    let got = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();

    let _ = stdin.write_all(&frame(r#"{"jsonrpc":"2.0","method":"build/exit"}"#));
    let _ = stdin.flush();
    drop(stdin);
    let _ = child.wait();
    let _ = reader.join();
    let _ = fs::remove_dir_all(&tmp);

    assert!(
        got.contains("buildTarget/didChange"),
        "expected a buildTarget/didChange notification after editing the pbxproj; got:\n{got}"
    );
}
