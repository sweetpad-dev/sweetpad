//! Layer 1 of the BSP measurement loop (see `PLAN_BSP.md`): protocol
//! conformance. Drives the `sweetpad-lib bsp` server with a scripted JSON-RPC
//! session (no `sourcekit-lsp`, no build) and asserts the structural invariants:
//! every target is listed, sources are returned, `sources` ↔ `inverseSources`
//! round-trips, and `sourceKitOptions` yields editor arguments.
//!
//! Fast and hermetic — reads only the committed multi-module fixture, resolves
//! against the embedded catalog, no Xcode or `xcodebuild` required.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn project() -> String {
    format!("{}/fixtures/_synthetic-multimodule/project/MultiModule.xcodeproj", env!("CARGO_MANIFEST_DIR"))
}

fn b_swift_uri() -> String {
    format!("file://{}/fixtures/_synthetic-multimodule/project/ModuleB/b.swift", env!("CARGO_MANIFEST_DIR"))
}

fn frame(msg: &Value) -> Vec<u8> {
    let body = msg.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// Split a stream of `Content-Length`-framed messages into JSON values.
fn parse_frames(out: &[u8]) -> Vec<Value> {
    // The server emits ASCII (uris are percent-encoded), so byte offsets from
    // `str` searches are char boundaries.
    let text = String::from_utf8_lossy(out);
    let mut frames = Vec::new();
    let mut rest: &str = &text;
    while let Some(hdr) = rest.find("Content-Length:") {
        rest = &rest[hdr + "Content-Length:".len()..];
        let Some(sep) = rest.find("\r\n\r\n") else { break };
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

/// Run a full scripted session against the server, returning responses by id.
fn run_session(messages: &[Value], project: &str) -> Vec<Value> {
    let mut input = Vec::new();
    for m in messages {
        input.extend(frame(m));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_sweetpad-lib"))
        .args(["bsp", "--project", project])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn bsp server");
    child.stdin.take().unwrap().write_all(&input).expect("write stdin");
    // stdin dropped → EOF; the server also exits on the `build/exit` message.
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).expect("read stdout");
    let _ = child.wait();
    parse_frames(&out)
}

fn result_for(frames: &[Value], id: i64) -> Option<&Value> {
    frames.iter().find(|f| f.get("id").and_then(Value::as_i64) == Some(id)).and_then(|f| f.get("result"))
}

#[test]
fn bsp_conformance() {
    let b_uri = b_swift_uri();
    let module_b = json!({ "uri": "sweetpad://target/ModuleB" });
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{"rootUri":format!("file://{}",project())}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"workspace/buildTargets"}),
        json!({"jsonrpc":"2.0","id":3,"method":"buildTarget/sources","params":{"targets":[module_b]}}),
        json!({"jsonrpc":"2.0","id":4,"method":"buildTarget/inverseSources","params":{"textDocument":{"uri":b_uri}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"textDocument/sourceKitOptions","params":{"textDocument":{"uri":b_uri},"target":module_b}}),
        json!({"jsonrpc":"2.0","id":6,"method":"build/shutdown"}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &project());

    // initialize: advertises the SourceKit options capability + swift language.
    let init = result_for(&frames, 1).expect("initialize result");
    let langs = init.pointer("/capabilities/languageIds").and_then(Value::as_array).expect("languageIds");
    assert!(langs.iter().any(|l| l == "swift"), "swift not advertised: {init}");

    // buildTargets: both modules present.
    let targets = result_for(&frames, 2).and_then(|r| r.get("targets")).and_then(Value::as_array).expect("targets");
    let names: Vec<&str> = targets.iter().filter_map(|t| t.get("displayName").and_then(Value::as_str)).collect();
    assert!(names.contains(&"ModuleA") && names.contains(&"ModuleB"), "missing targets: {names:?}");

    // buildTargets: the dependency edge ModuleB → ModuleA is reported (so
    // sourcekit-lsp knows the prepare order / transitive module set).
    let module_b_target = targets
        .iter()
        .find(|t| t.get("displayName").and_then(Value::as_str) == Some("ModuleB"))
        .expect("ModuleB target");
    let deps: Vec<&str> = module_b_target
        .get("dependencies")
        .and_then(Value::as_array)
        .expect("dependencies")
        .iter()
        .filter_map(|d| d.get("uri").and_then(Value::as_str))
        .collect();
    assert_eq!(deps, vec!["sweetpad://target/ModuleA"], "ModuleB should depend on ModuleA");
    // ModuleA depends on nothing.
    let module_a_target = targets
        .iter()
        .find(|t| t.get("displayName").and_then(Value::as_str) == Some("ModuleA"))
        .expect("ModuleA target");
    assert_eq!(
        module_a_target.get("dependencies").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "ModuleA should have no dependencies"
    );

    // sources(ModuleB): includes b.swift.
    let items = result_for(&frames, 3).and_then(|r| r.get("items")).and_then(Value::as_array).expect("items");
    let b_listed = items.iter().any(|it| {
        it.get("sources").and_then(Value::as_array).is_some_and(|ss| {
            ss.iter().any(|s| s.get("uri").and_then(Value::as_str).is_some_and(|u| u.ends_with("/ModuleB/b.swift")))
        })
    });
    assert!(b_listed, "b.swift not in ModuleB sources: {items:?}");

    // inverseSources(b.swift) → ModuleB (the round-trip).
    let inverse = result_for(&frames, 4).and_then(|r| r.get("targets")).and_then(Value::as_array).expect("inverse");
    let owners: Vec<&str> = inverse.iter().filter_map(|t| t.get("uri").and_then(Value::as_str)).collect();
    assert!(owners.contains(&"sweetpad://target/ModuleB"), "inverseSources didn't map b.swift to ModuleB: {owners:?}");

    // sourceKitOptions(b.swift): editor args — search paths in, explicit-module out.
    let args = result_for(&frames, 5)
        .and_then(|r| r.get("compilerArguments"))
        .and_then(Value::as_array)
        .expect("compilerArguments");
    let args: Vec<&str> = args.iter().filter_map(Value::as_str).collect();
    assert!(!args.is_empty(), "empty compilerArguments");
    assert!(args.contains(&"-I"), "no search paths in editor args: {args:?}");
    assert!(!args.contains(&"-explicit-module-build"), "explicit-module flag leaked into editor args");

    // shutdown is answered.
    assert!(result_for(&frames, 6).is_some(), "shutdown not answered");
}

/// Per-file `sourceKitOptions`: a `.m` file gets ObjC dialect (`-x objective-c`)
/// and the header search path it needs — both through the server, hermetically.
#[test]
fn bsp_per_file_clang_dialect() {
    let root = env!("CARGO_MANIFEST_DIR");
    let proj = format!("{root}/fixtures/_synthetic-objc-headers/project/ObjCHeaders.xcodeproj");
    let m_uri = format!("file://{root}/fixtures/_synthetic-objc-headers/project/widget.m");
    let target = json!({ "uri": "sweetpad://target/ObjCHeaders" });
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":5,"method":"textDocument/sourceKitOptions","params":{"textDocument":{"uri":m_uri},"target":target}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &proj);
    let args: Vec<&str> = result_for(&frames, 5)
        .and_then(|r| r.get("compilerArguments"))
        .and_then(Value::as_array)
        .expect("compilerArguments")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(args.windows(2).any(|w| w == ["-x", "objective-c"]), "no -x objective-c: {args:?}");
    assert!(args.contains(&"-I"), "no header search path: {args:?}");
}

/// A target consuming a Swift Package product gets the package-products
/// framework search path (`-F …/PackageFrameworks`), and its source maps to the
/// target — both through the server, hermetically (no build required).
#[test]
fn bsp_spm_package_search_path() {
    let root = env!("CARGO_MANIFEST_DIR");
    let proj = format!("{root}/fixtures/_synthetic-spm/project/SpmApp.xcodeproj");
    let app_uri = format!("file://{root}/fixtures/_synthetic-spm/project/AppSources/App.swift");
    let target = json!({ "uri": "sweetpad://target/SpmApp" });
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":5,"method":"textDocument/sourceKitOptions","params":{"textDocument":{"uri":app_uri},"target":target}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &proj);
    let args: Vec<&str> = result_for(&frames, 5)
        .and_then(|r| r.get("compilerArguments"))
        .and_then(Value::as_array)
        .expect("compilerArguments")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        args.windows(2).any(|w| w[0] == "-F" && w[1].ends_with("/PackageFrameworks")),
        "no PackageFrameworks framework search path: {args:?}"
    );
    assert!(args.iter().any(|a| a.ends_with("/AppSources/App.swift")), "App.swift not an input: {args:?}");
}
