//! v2 of the BSP loop: `buildTarget/prepare` builds a target's dependency
//! modules on demand, so cross-module `import`s resolve with **no prior build**.
//!
//! These drive the `bsp-server bsp` server directly (no sourcekit-lsp), from a
//! clean DerivedData:
//!
//! * a pure-Swift closure is prepared by the `swiftc` fast path, and the
//!   dependency `.swiftmodule` lands in the products dir our search paths name;
//! * preparing an ObjC target publishes its header maps **and** pushes
//!   `buildTarget/didChange`, so a client that pulled options against the cold
//!   tree learns to ask again (GitHub #238 — without the push, the arguments a
//!   client cached before the build stand, and its ObjC imports never resolve);
//! * the startup warm-up reaches a target **no scheme builds**, which needs a
//!   `-target` build with the output roots named explicitly.
//!
//! Opt-in: runs `xcodebuild`, so gated on `BSP_ORACLE=1` (+ Xcode 26.5).

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const XCODE: &str = "/Applications/Xcode-26.5.0.app";

/// Long enough for a cold `xcodebuild` on a small fixture, short enough that a
/// wedged server fails the run rather than hanging it.
const BUILD_TIMEOUT: Duration = Duration::from_secs(300);

fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

fn fixture(name: &str, proj: &str) -> String {
    format!(
        "{}/fixtures/{name}/project/{proj}",
        env!("SWEETPAD_LIB_DIR")
    )
}

/// Whether the gated preconditions hold; prints why when they don't.
fn gated() -> bool {
    if std::env::var("BSP_ORACLE").is_err() {
        eprintln!("skipping: set BSP_ORACLE=1 to run the BSP prepare oracle");
        return false;
    }
    if !Path::new(XCODE).exists() {
        eprintln!("skipping: {XCODE} not installed");
        return false;
    }
    true
}

/// A running BSP server plus everything it has written to stdout so far, so a
/// test can drive it by frames and wait on what comes back.
struct Session {
    child: Child,
    stdin: std::process::ChildStdin,
    out: Arc<Mutex<String>>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Session {
    fn start(project: &str, dd: &Path, log: &Path) -> Session {
        let mut child = Command::new(env!("CARGO_BIN_EXE_bsp-server"))
            .args(["bsp", "--project", project, "--xcode"])
            .arg(format!("{XCODE}/Contents/Developer"))
            .arg("--derived-data-path")
            .arg(dd)
            .env("SWEETPAD_BSP_LOG", log)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn server");
        let stdin = child.stdin.take().expect("stdin");
        let mut stdout = child.stdout.take().expect("stdout");
        let out = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&out);
        let reader = std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                sink.lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&chunk[..n]));
            }
        });
        Session {
            child,
            stdin,
            out,
            reader: Some(reader),
        }
    }

    fn send(&mut self, body: &str) {
        self.stdin.write_all(&frame(body)).expect("write frame");
        self.stdin.flush().expect("flush");
    }

    fn text(&self) -> String {
        self.out.lock().unwrap().clone()
    }

    /// Poll until `needle` appears in the server's output, returning whether it
    /// did before `timeout`.
    fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.text().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }

    fn shutdown(mut self) -> String {
        let _ = self
            .stdin
            .write_all(&frame(r#"{"jsonrpc":"2.0","method":"build/exit"}"#));
        let _ = self.stdin.flush();
        drop(self.stdin);
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.out.lock().unwrap().clone()
    }
}

/// The `compilerArguments` of the last `textDocument/sourceKitOptions` reply in
/// `transcript` — read as raw text, since the frames are concatenated rather
/// than parsed one at a time.
fn last_compiler_arguments(transcript: &str) -> String {
    let key = "\"compilerArguments\":";
    let start = transcript
        .rfind(key)
        .unwrap_or_else(|| panic!("no sourceKitOptions reply in:\n{transcript}"));
    let rest = &transcript[start + key.len()..];
    let end = rest.find(']').unwrap_or(rest.len());
    rest[..=end.min(rest.len() - 1)].to_string()
}

#[test]
fn prepare_builds_dependency_module_from_clean_deriveddata() {
    if !gated() {
        return;
    }
    let project = fixture("_synthetic-multimodule", "MultiModule.xcodeproj");
    let dd = std::env::temp_dir().join(format!("sweetpad-bsp-prep-{}", std::process::id()));
    let log = std::env::temp_dir().join(format!("sweetpad-bsp-prep-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dd);
    let _ = std::fs::remove_file(&log);
    let dep_module = dd.join("Build/Products/Debug/ModuleA.swiftmodule");

    let mut session = Session::start(&project, &dd, &log);
    session.send(r#"{"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}"#);
    session.send(r#"{"jsonrpc":"2.0","method":"build/initialized"}"#);
    // prepare ModuleB → must build its dependency ModuleA's module.
    session.send(
        r#"{"jsonrpc":"2.0","id":10,"method":"buildTarget/prepare","params":{"targets":[{"uri":"sweetpad://target/ModuleB"}]}}"#,
    );
    let replied = session.wait_for(r#""id":10"#, BUILD_TIMEOUT);
    session.shutdown();

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

/// GitHub #238, end to end on the server: an ObjC target's arguments gain the
/// header maps a prepare puts on disk, and the client is *told* to come back for
/// them.
///
/// `build/initialized` is deliberately not sent — it starts the warm-up, which
/// would prepare the target before this can observe the cold tree.
#[test]
fn prepare_publishes_header_maps_and_notifies() {
    if !gated() {
        return;
    }
    let project = fixture("_synthetic-headermaps", "HeaderMaps.xcodeproj");
    let widget = format!(
        "{}/fixtures/_synthetic-headermaps/project/Top/Widget.m",
        env!("SWEETPAD_LIB_DIR")
    );
    let dd = std::env::temp_dir().join(format!("sweetpad-bsp-hm-{}", std::process::id()));
    let log = std::env::temp_dir().join(format!("sweetpad-bsp-hm-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dd);
    let _ = std::fs::remove_file(&log);

    let options = |id: u32| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"textDocument/sourceKitOptions","params":{{"textDocument":{{"uri":"file://{widget}"}},"target":{{"uri":"sweetpad://target/HeaderMaps"}},"language":"objective-c"}}}}"#
        )
    };

    let mut session = Session::start(&project, &dd, &log);
    session.send(r#"{"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}"#);
    session.send(&options(2));
    assert!(
        session.wait_for(r#""id":2"#, Duration::from_secs(120)),
        "no sourceKitOptions reply for the cold tree"
    );
    let cold = last_compiler_arguments(&session.text());

    session.send(
        r#"{"jsonrpc":"2.0","id":3,"method":"buildTarget/prepare","params":{"targets":[{"uri":"sweetpad://target/HeaderMaps"}]}}"#,
    );
    let replied = session.wait_for(r#""id":3"#, BUILD_TIMEOUT);
    let notified = session.wait_for("buildTarget/didChange", Duration::from_secs(10));

    session.send(&options(4));
    let re_replied = session.wait_for(r#""id":4"#, Duration::from_secs(120));
    let warm = last_compiler_arguments(&session.text());
    let transcript = session.shutdown();

    let log_text = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dd);
    let _ = std::fs::remove_file(&log);

    assert!(replied, "server never answered buildTarget/prepare");
    assert!(re_replied, "server never re-answered sourceKitOptions");
    assert!(
        !cold.contains(".hmap"),
        "named header maps from a tree with none:\n{cold}"
    );
    assert!(
        notified,
        "prepare did not push buildTarget/didChange, so a client that pulled options \
         against the cold tree keeps them; log:\n{log_text}\ntranscript:\n{transcript}"
    );
    for expected in [
        "HeaderMaps-project-headers.hmap",
        "HeaderMaps-own-target-headers.hmap",
        "HeaderMaps-generated-files.hmap",
        "DerivedSources",
    ] {
        assert!(
            warm.contains(expected),
            "prepared arguments are missing {expected}:\n{warm}"
        );
    }
}

/// A target no scheme builds still gets prepared — by `-target`, with the output
/// roots named so its header maps land in the DerivedData the editor arguments
/// point at rather than in the project's own `build/`. Also the warm-up itself:
/// nothing here ever sends `buildTarget/prepare`.
#[test]
fn startup_warmup_prepares_a_target_no_scheme_builds() {
    if !gated() {
        return;
    }
    let project = fixture("_synthetic-headermaps", "HeaderMaps.xcodeproj");
    let dd = std::env::temp_dir().join(format!("sweetpad-bsp-orphan-{}", std::process::id()));
    let log = std::env::temp_dir().join(format!("sweetpad-bsp-orphan-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dd);
    let _ = std::fs::remove_file(&log);
    let orphan_hmap = dd.join(
        "Build/Intermediates.noindex/HeaderMaps.build/Debug/HeaderMapsOrphan.build/\
         HeaderMapsOrphan-project-headers.hmap",
    );

    let mut session = Session::start(&project, &dd, &log);
    session.send(r#"{"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}"#);
    session.send(r#"{"jsonrpc":"2.0","method":"build/initialized"}"#);

    let deadline = Instant::now() + BUILD_TIMEOUT;
    let mut appeared = false;
    while Instant::now() < deadline {
        if orphan_hmap.exists() {
            appeared = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    session.shutdown();

    let log_text = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dd);
    let _ = std::fs::remove_file(&log);

    assert!(
        appeared,
        "warm-up left HeaderMapsOrphan unprepared — expected {}\nlog:\n{log_text}",
        orphan_hmap.display()
    );
    assert!(
        log_text.contains("building target HeaderMapsOrphan"),
        "expected the -target path for a target no scheme builds; log:\n{log_text}"
    );
}

/// A second prepare over unchanged project files must not re-spawn `xcodebuild`:
/// with one worker and no coalescing, a client that asks per opened file
/// serializes a fresh build behind every one of them.
#[test]
fn a_repeat_prepare_over_unchanged_inputs_is_skipped() {
    if !gated() {
        return;
    }
    let project = fixture("_synthetic-headermaps", "HeaderMaps.xcodeproj");
    let dd = std::env::temp_dir().join(format!("sweetpad-bsp-coalesce-{}", std::process::id()));
    let log =
        std::env::temp_dir().join(format!("sweetpad-bsp-coalesce-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dd);
    let _ = std::fs::remove_file(&log);

    let prepare = |id: u32| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"buildTarget/prepare","params":{{"targets":[{{"uri":"sweetpad://target/HeaderMaps"}}]}}}}"#
        )
    };
    let mut session = Session::start(&project, &dd, &log);
    session.send(r#"{"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}"#);
    session.send(&prepare(2));
    let first = session.wait_for(r#""id":2"#, BUILD_TIMEOUT);
    session.send(&prepare(3));
    let second = session.wait_for(r#""id":3"#, Duration::from_secs(60));
    session.shutdown();

    let log_text = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dd);
    let _ = std::fs::remove_file(&log);

    assert!(first && second, "both prepares must be answered");
    assert_eq!(
        log_text.matches("prepare: building").count(),
        1,
        "the repeat re-ran xcodebuild; log:\n{log_text}"
    );
    assert!(
        log_text.contains("prepare: HeaderMaps already current; skipping"),
        "the repeat was not recognised as current; log:\n{log_text}"
    );
}
