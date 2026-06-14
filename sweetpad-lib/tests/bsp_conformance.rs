//! Layer 1 of the BSP measurement loop (see `DOCS.md` §8 (BSP server)): protocol
//! conformance. Drives the `bsp-server bsp` server with a scripted JSON-RPC
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
    format!(
        "{}/fixtures/_synthetic-multimodule/project/MultiModule.xcodeproj",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn b_swift_uri() -> String {
    format!(
        "file://{}/fixtures/_synthetic-multimodule/project/ModuleB/b.swift",
        env!("CARGO_MANIFEST_DIR")
    )
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

/// Run a full scripted session against the server, returning responses by id.
fn run_session(messages: &[Value], project: &str) -> Vec<Value> {
    run_session_args(messages, project, &[])
}

/// As [`run_session`], with extra CLI flags after `--project` (e.g.
/// `--derived-data-path`) so a test can exercise the flag-driven config.
fn run_session_args(messages: &[Value], project: &str, extra: &[&str]) -> Vec<Value> {
    let mut input = Vec::new();
    for m in messages {
        input.extend(frame(m));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_bsp-server"))
        .args(["bsp", "--project", project])
        .args(extra)
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
    // stdin dropped → EOF; the server also exits on the `build/exit` message.
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

fn error_for(frames: &[Value], id: i64) -> Option<&Value> {
    frames
        .iter()
        .find(|f| f.get("id").and_then(Value::as_i64) == Some(id))
        .and_then(|f| f.get("error"))
}

/// The string compiler arguments from a `textDocument/sourceKitOptions` reply.
fn sourcekit_args(frames: &[Value], id: i64) -> Vec<String> {
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
    let langs = init
        .pointer("/capabilities/languageIds")
        .and_then(Value::as_array)
        .expect("languageIds");
    assert!(
        langs.iter().any(|l| l == "swift"),
        "swift not advertised: {init}"
    );

    // buildTargets: both modules present.
    let targets = result_for(&frames, 2)
        .and_then(|r| r.get("targets"))
        .and_then(Value::as_array)
        .expect("targets");
    let names: Vec<&str> = targets
        .iter()
        .filter_map(|t| t.get("displayName").and_then(Value::as_str))
        .collect();
    assert!(
        names.contains(&"ModuleA") && names.contains(&"ModuleB"),
        "missing targets: {names:?}"
    );

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
    assert_eq!(
        deps,
        vec!["sweetpad://target/ModuleA"],
        "ModuleB should depend on ModuleA"
    );
    // ModuleA depends on nothing.
    let module_a_target = targets
        .iter()
        .find(|t| t.get("displayName").and_then(Value::as_str) == Some("ModuleA"))
        .expect("ModuleA target");
    assert_eq!(
        module_a_target
            .get("dependencies")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "ModuleA should have no dependencies"
    );

    // sources(ModuleB): includes b.swift.
    let items = result_for(&frames, 3)
        .and_then(|r| r.get("items"))
        .and_then(Value::as_array)
        .expect("items");
    let b_listed = items.iter().any(|it| {
        it.get("sources")
            .and_then(Value::as_array)
            .is_some_and(|ss| {
                ss.iter().any(|s| {
                    s.get("uri")
                        .and_then(Value::as_str)
                        .is_some_and(|u| u.ends_with("/ModuleB/b.swift"))
                })
            })
    });
    assert!(b_listed, "b.swift not in ModuleB sources: {items:?}");

    // inverseSources(b.swift) → ModuleB (the round-trip).
    let inverse = result_for(&frames, 4)
        .and_then(|r| r.get("targets"))
        .and_then(Value::as_array)
        .expect("inverse");
    let owners: Vec<&str> = inverse
        .iter()
        .filter_map(|t| t.get("uri").and_then(Value::as_str))
        .collect();
    assert!(
        owners.contains(&"sweetpad://target/ModuleB"),
        "inverseSources didn't map b.swift to ModuleB: {owners:?}"
    );

    // sourceKitOptions(b.swift): editor args — search paths in, explicit-module out.
    let args = result_for(&frames, 5)
        .and_then(|r| r.get("compilerArguments"))
        .and_then(Value::as_array)
        .expect("compilerArguments");
    let args: Vec<&str> = args.iter().filter_map(Value::as_str).collect();
    assert!(!args.is_empty(), "empty compilerArguments");
    assert!(
        args.contains(&"-I"),
        "no search paths in editor args: {args:?}"
    );
    assert!(
        !args.contains(&"-explicit-module-build"),
        "explicit-module flag leaked into editor args"
    );

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
    assert!(
        args.windows(2).any(|w| w == ["-x", "objective-c"]),
        "no -x objective-c: {args:?}"
    );
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
        args.windows(2)
            .any(|w| w[0] == "-F" && w[1].ends_with("/PackageFrameworks")),
        "no PackageFrameworks framework search path: {args:?}"
    );
    assert!(
        args.iter().any(|a| a.ends_with("/AppSources/App.swift")),
        "App.swift not an input: {args:?}"
    );
}

// The fixtures below the conformance smoke test pin the rest of the protocol
// surface — the shapes sourcekit-lsp's decoder requires and the per-file
// dialect gating — each driven through the real server, hermetically (embedded
// catalog / active Xcode, no `xcodebuild`). They're syntactic oracles: the
// expectation is the structure of the reply, not a real type-check (that's the
// `BSP_ORACLE`-gated Layer 0/2 tests).

fn multimodule_dir() -> String {
    format!(
        "{}/fixtures/_synthetic-multimodule/project",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn objc_project() -> String {
    format!(
        "{}/fixtures/_synthetic-objc-headers/project/ObjCHeaders.xcodeproj",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// `build/initialize` advertises the exact capability surface sourcekit-lsp
/// decodes: the `sourceKit` data kind with both providers on, the BSP version,
/// and all five language ids. A missing field here silently disables the server.
#[test]
fn bsp_initialize_full_capabilities() {
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let init = result_for(&run_session(&messages, &project()), 1)
        .cloned()
        .expect("initialize result");
    assert_eq!(
        init.get("bspVersion").and_then(Value::as_str),
        Some("2.2.0"),
        "bspVersion: {init}"
    );
    assert_eq!(
        init.get("dataKind").and_then(Value::as_str),
        Some("sourceKit"),
        "dataKind: {init}"
    );
    assert_eq!(
        init.pointer("/data/sourceKitOptionsProvider")
            .and_then(Value::as_bool),
        Some(true),
        "sourceKitOptionsProvider must be advertised: {init}"
    );
    assert_eq!(
        init.pointer("/data/prepareProvider")
            .and_then(Value::as_bool),
        Some(true),
        "prepareProvider must be advertised so background indexing delegates prepare: {init}"
    );
    let langs: Vec<&str> = init
        .pointer("/capabilities/languageIds")
        .and_then(Value::as_array)
        .expect("languageIds")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for lang in ["swift", "objective-c", "objective-cpp", "c", "cpp"] {
        assert!(
            langs.contains(&lang),
            "languageIds missing {lang}: {langs:?}"
        );
    }
}

/// With a `--derived-data-path`, `build/initialize` advertises that build's
/// index store, so sourcekit-lsp can navigate the index-while-building data.
/// The paths are pure geometry off the given DerivedData (no build needed).
#[test]
fn bsp_initialize_advertises_index_store() {
    let dd = "/tmp/sweetpad-bsp-idx-oracle";
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session_args(&messages, &project(), &["--derived-data-path", dd]);
    let init = result_for(&frames, 1).cloned().expect("initialize result");
    let store = init
        .pointer("/data/indexStorePath")
        .and_then(Value::as_str)
        .expect("indexStorePath");
    let db = init
        .pointer("/data/indexDatabasePath")
        .and_then(Value::as_str)
        .expect("indexDatabasePath");
    assert_eq!(
        store,
        format!("{dd}/Index.noindex/DataStore"),
        "indexStorePath"
    );
    assert_eq!(
        db,
        format!("{dd}/Index.noindex/IndexDatabase"),
        "indexDatabasePath"
    );
}

/// Every `workspace/buildTargets` entry carries the fields sourcekit-lsp needs:
/// a `sweetpad://target/<name>` id, the project base directory, the five
/// language ids, and capabilities marking it compilable (not testable/runnable
/// from the editor).
#[test]
fn bsp_build_targets_shape() {
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"workspace/buildTargets"}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &project());
    let base = format!("file://{}", multimodule_dir());
    let targets = result_for(&frames, 2)
        .and_then(|r| r.get("targets"))
        .and_then(Value::as_array)
        .expect("targets");
    assert!(!targets.is_empty(), "no targets");
    for t in targets {
        let name = t
            .get("displayName")
            .and_then(Value::as_str)
            .expect("displayName");
        assert_eq!(
            t.pointer("/id/uri").and_then(Value::as_str),
            Some(format!("sweetpad://target/{name}").as_str()),
            "target id uri for {name}: {t}"
        );
        assert_eq!(
            t.get("baseDirectory").and_then(Value::as_str),
            Some(base.as_str()),
            "baseDirectory for {name}"
        );
        assert_eq!(
            t.pointer("/capabilities/canCompile")
                .and_then(Value::as_bool),
            Some(true),
            "{name} must be compilable"
        );
        for cap in ["canTest", "canRun", "canDebug"] {
            assert_eq!(
                t.pointer(&format!("/capabilities/{cap}"))
                    .and_then(Value::as_bool),
                Some(false),
                "{name}.{cap} must be false (the editor doesn't run targets)"
            );
        }
        let langs = t.get("languageIds").and_then(Value::as_array).map(Vec::len);
        assert_eq!(langs, Some(5), "{name} should advertise all 5 language ids");
    }
}

/// `buildTarget/sources` filters to the requested target and tags each source
/// as an on-disk (`generated: false`), file-kind (`kind: 1`) entry.
#[test]
fn bsp_sources_metadata_and_filtering() {
    let module_a = json!({ "uri": "sweetpad://target/ModuleA" });
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"buildTarget/sources","params":{"targets":[module_a]}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &project());
    let items = result_for(&frames, 2)
        .and_then(|r| r.get("items"))
        .and_then(Value::as_array)
        .expect("items");
    assert_eq!(
        items.len(),
        1,
        "asking for ModuleA must return exactly one item: {items:?}"
    );
    assert_eq!(
        items[0].pointer("/target/uri").and_then(Value::as_str),
        Some("sweetpad://target/ModuleA")
    );
    let sources = items[0]
        .get("sources")
        .and_then(Value::as_array)
        .expect("sources");
    assert!(!sources.is_empty(), "ModuleA has no sources");
    for s in sources {
        assert_eq!(
            s.get("kind").and_then(Value::as_i64),
            Some(1),
            "source kind must be 1 (file): {s}"
        );
        assert_eq!(
            s.get("generated").and_then(Value::as_bool),
            Some(false),
            "source must be on-disk: {s}"
        );
    }
    assert!(
        sources.iter().any(|s| s
            .get("uri")
            .and_then(Value::as_str)
            .is_some_and(|u| u.ends_with("/ModuleA/a.swift"))),
        "a.swift not in ModuleA sources: {sources:?}"
    );
}

/// `buildTarget/sources` with no `targets` defaults to every target — so a
/// client can enumerate all sources in one call.
#[test]
fn bsp_sources_all_targets_when_unspecified() {
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"buildTarget/sources"}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &project());
    let items = result_for(&frames, 2)
        .and_then(|r| r.get("items"))
        .and_then(Value::as_array)
        .expect("items");
    let owners: Vec<&str> = items
        .iter()
        .filter_map(|it| it.pointer("/target/uri").and_then(Value::as_str))
        .collect();
    assert!(
        owners.contains(&"sweetpad://target/ModuleA"),
        "ModuleA missing: {owners:?}"
    );
    assert!(
        owners.contains(&"sweetpad://target/ModuleB"),
        "ModuleB missing: {owners:?}"
    );
}

/// `sources` ↔ `inverseSources` is an exact bijection on the fixture: each
/// module's own file maps back to exactly that module, nothing else.
#[test]
fn bsp_inverse_sources_exact_roundtrip() {
    let dir = multimodule_dir();
    let a_uri = format!("file://{dir}/ModuleA/a.swift");
    let b_uri = format!("file://{dir}/ModuleB/b.swift");
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"buildTarget/inverseSources","params":{"textDocument":{"uri":a_uri}}}),
        json!({"jsonrpc":"2.0","id":3,"method":"buildTarget/inverseSources","params":{"textDocument":{"uri":b_uri}}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &project());
    let owners = |id: i64| -> Vec<String> {
        result_for(&frames, id)
            .and_then(|r| r.get("targets"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.get("uri").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(
        owners(2),
        vec!["sweetpad://target/ModuleA"],
        "a.swift must map to exactly ModuleA"
    );
    assert_eq!(
        owners(3),
        vec!["sweetpad://target/ModuleB"],
        "b.swift must map to exactly ModuleB"
    );
}

/// A Swift `sourceKitOptions` reply carries the module-resolution surface
/// (`-module-name`, `-sdk`, `-target`), the working directory, the file as an
/// input, and — crucially — none of the build-only flags an editor front end
/// can't honour (`-c`, explicit-module plumbing, emit/codegen actions).
#[test]
fn bsp_swift_options_editor_shape() {
    let dir = multimodule_dir();
    let b_uri = format!("file://{dir}/ModuleB/b.swift");
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":5,"method":"textDocument/sourceKitOptions","params":{"textDocument":{"uri":b_uri},"target":{"uri":"sweetpad://target/ModuleB"}}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &project());
    let args = sourcekit_args(&frames, 5);
    assert!(
        args.windows(2).any(|w| w == ["-module-name", "ModuleB"]),
        "no `-module-name ModuleB`: {args:?}"
    );
    assert!(args.iter().any(|a| a == "-sdk"), "no -sdk: {args:?}");
    assert!(args.iter().any(|a| a == "-target"), "no -target: {args:?}");
    assert!(
        args.iter().any(|a| a.ends_with("/ModuleB/b.swift")),
        "b.swift not an input: {args:?}"
    );
    // Build-only flags the editor invocation must drop. Each is present in the
    // real build argv (verified via `compiler-args` against this fixture), so a
    // regression that stops stripping would surface here, not pass vacuously.
    for stripped in [
        "-explicit-module-build",
        "-emit-dependencies",
        "-emit-const-values",
        "-incremental",
        "-enable-batch-mode",
    ] {
        assert!(
            !args.iter().any(|a| a == stripped),
            "build-only flag {stripped} leaked into editor args: {args:?}"
        );
    }
    let wd = result_for(&frames, 5)
        .and_then(|r| r.get("workingDirectory"))
        .and_then(Value::as_str);
    assert_eq!(
        wd,
        Some(dir.as_str()),
        "workingDirectory must be the project dir"
    );
}

/// An unknown request gets a JSON-RPC `-32601` (method not found) rather than a
/// dropped frame that would wedge the client waiting on a reply.
#[test]
fn bsp_unknown_method_errors() {
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":42,"method":"some/unsupportedMethod","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &project());
    let err = error_for(&frames, 42).expect("error reply for unknown method");
    assert_eq!(
        err.get("code").and_then(Value::as_i64),
        Some(-32601),
        "wrong error code: {err}"
    );
}

/// `inverseSources` for a file in no target returns an empty target list (not
/// an error), and `sourceKitOptions` for an unowned file returns a null result.
#[test]
fn bsp_unowned_file_handled_gracefully() {
    let foreign = "file:///definitely/not/in/project/foreign.swift";
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"buildTarget/inverseSources","params":{"textDocument":{"uri":foreign}}}),
        json!({"jsonrpc":"2.0","id":3,"method":"textDocument/sourceKitOptions","params":{"textDocument":{"uri":foreign}}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &project());
    let owners = result_for(&frames, 2)
        .and_then(|r| r.get("targets"))
        .and_then(Value::as_array);
    assert_eq!(
        owners.map(Vec::len),
        Some(0),
        "inverseSources of a foreign file must be empty: {owners:?}"
    );
    assert_eq!(
        result_for(&frames, 3),
        Some(&Value::Null),
        "sourceKitOptions of an unowned file must be null"
    );
}

/// `workspace/waitForBuildSystemUpdates` is answered with an empty object, so
/// sourcekit-lsp's startup barrier completes.
#[test]
fn bsp_wait_for_build_system_updates() {
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"workspace/waitForBuildSystemUpdates"}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &project());
    assert_eq!(
        result_for(&frames, 2),
        Some(&json!({})),
        "waitForBuildSystemUpdates must answer with {{}}"
    );
}

/// `sourceKitOptions` without an explicit `target` resolves the owning target
/// by source membership (a sourcekit-lsp request often omits it) — for both a
/// Swift file (whole-module args) and a clang file (its own dialect).
#[test]
fn bsp_source_kit_options_resolves_target_by_membership() {
    let dir = multimodule_dir();
    let b_uri = format!("file://{dir}/ModuleB/b.swift");
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":5,"method":"textDocument/sourceKitOptions","params":{"textDocument":{"uri":b_uri}}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let swift_args = sourcekit_args(&run_session(&messages, &project()), 5);
    assert!(
        swift_args
            .windows(2)
            .any(|w| w == ["-module-name", "ModuleB"]),
        "membership fallback should resolve ModuleB for b.swift: {swift_args:?}"
    );
    assert!(
        swift_args.iter().any(|a| a.ends_with("/ModuleB/b.swift")),
        "b.swift not an input: {swift_args:?}"
    );

    let objc_dir = format!(
        "{}/fixtures/_synthetic-objc-headers/project",
        env!("CARGO_MANIFEST_DIR")
    );
    let widget = format!("file://{objc_dir}/widget.m");
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":5,"method":"textDocument/sourceKitOptions","params":{"textDocument":{"uri":widget}}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let clang_args = sourcekit_args(&run_session(&messages, &objc_project()), 5);
    assert!(
        clang_args.windows(2).any(|w| w == ["-x", "objective-c"]),
        "membership fallback should resolve ObjCHeaders for widget.m: {clang_args:?}"
    );
}

/// `buildTarget/sources` enumerates clang sources too (not just Swift): the
/// static-library fixture's `widget.m` is listed with file-kind/on-disk tags.
#[test]
fn bsp_sources_lists_clang_sources() {
    let messages = vec![
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"buildTarget/sources","params":{"targets":[{"uri":"sweetpad://target/ObjCHeaders"}]}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let frames = run_session(&messages, &objc_project());
    let items = result_for(&frames, 2)
        .and_then(|r| r.get("items"))
        .and_then(Value::as_array)
        .expect("items");
    let widget = items
        .iter()
        .flat_map(|it| {
            it.get("sources")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
        })
        .find(|s| {
            s.get("uri")
                .and_then(Value::as_str)
                .is_some_and(|u| u.ends_with("/widget.m"))
        })
        .expect("widget.m not listed in ObjCHeaders sources");
    assert_eq!(
        widget.get("kind").and_then(Value::as_i64),
        Some(1),
        "widget.m kind must be 1: {widget}"
    );
    assert_eq!(
        widget.get("generated").and_then(Value::as_bool),
        Some(false),
        "widget.m must be on-disk: {widget}"
    );
}

/// Per-file `sourceKitOptions` dialect gating — the correctness-critical
/// per-file case (DOCS.md §8 (BSP server): "a `.mm` needs the C++ dialect/flags, a `.m` must
/// not"). For one clang target, each source extension must select its own clang
/// `-x` dialect, ObjC flags must reach only ObjC inputs, and C++ flags only
/// C++ inputs. Driven through the server against on-disk probe files; resolution
/// is by extension, so no build is required.
#[test]
fn bsp_per_file_clang_dialect_matrix() {
    // ObjC-only marker (an ObjC dispatch define) vs a C++-only warning. Both are
    // long-standing entries in Apple's clang xcspec, gated by source language.
    const OBJC_ONLY: &str = "-DOBJC_OLD_DISPATCH_PROTOTYPES=1";
    const CXX_ONLY: &str = "-Winvalid-offsetof";

    let dir = format!(
        "{}/fixtures/_synthetic-objc-headers/project",
        env!("CARGO_MANIFEST_DIR")
    );
    let proj = objc_project();

    // (file, expected `-x` dialect, has ObjC flags, has C++ flags)
    let cases = [
        ("widget.m", "objective-c", true, false),
        ("dialect_probe.mm", "objective-c++", true, true),
        ("dialect_probe.cpp", "c++", false, true),
        ("dialect_probe.c", "c", false, false),
    ];
    for (file, dialect, objc, cxx) in cases {
        let uri = format!("file://{dir}/{file}");
        let messages = vec![
            json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
            json!({"jsonrpc":"2.0","method":"build/initialized"}),
            json!({"jsonrpc":"2.0","id":5,"method":"textDocument/sourceKitOptions","params":{"textDocument":{"uri":uri},"target":{"uri":"sweetpad://target/ObjCHeaders"}}}),
            json!({"jsonrpc":"2.0","method":"build/exit"}),
        ];
        let args = sourcekit_args(&run_session(&messages, &proj), 5);
        assert!(
            args.windows(2).any(|w| w == ["-x", dialect]),
            "{file} should select `-x {dialect}`: {args:?}"
        );
        assert_eq!(
            args.iter().any(|a| a == OBJC_ONLY),
            objc,
            "{file}: ObjC flag presence should be {objc} (marker {OBJC_ONLY}): {args:?}"
        );
        assert_eq!(
            args.iter().any(|a| a == CXX_ONLY),
            cxx,
            "{file}: C++ flag presence should be {cxx} (marker {CXX_ONLY}): {args:?}"
        );
    }
}

/// The server must start from the `bsp.json` named by `--config`, written with
/// the extension's schema, where `workspacePath` is the VS Code workspace
/// *folder* (not an Xcode container) and `projectPath` is the real
/// `.xcodeproj`. Regression test: the config resolver used to prefer
/// `workspacePath`, open the folder as a project, and exit before replying.
#[test]
fn bsp_starts_from_extension_bsp_json() {
    let workspace = std::env::temp_dir().join(format!("sweetpad-bsp-json-{}", std::process::id()));
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let config = json!({
        "name": "sweetpad",
        "workspacePath": workspace.to_string_lossy(),
        "projectPath": project(),
        "scheme": null,
        "configuration": "Debug",
        "derivedDataPath": null,
        "developerDir": null,
    });
    let config_path = workspace.join("bsp.json");
    std::fs::write(&config_path, config.to_string()).expect("write bsp.json");

    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"workspace/buildTargets"}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let mut input = Vec::new();
    for m in &messages {
        input.extend(frame(m));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_bsp-server"))
        .arg("bsp")
        .arg("--config")
        .arg(&config_path)
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
    let status = child.wait().expect("wait");
    let _ = std::fs::remove_dir_all(&workspace);

    assert!(status.success(), "server exited non-zero: {status:?}");
    let frames = parse_frames(&out);
    assert!(
        result_for(&frames, 1).is_some(),
        "no initialize result; frames: {frames:?}"
    );
    let names: Vec<&str> = result_for(&frames, 2)
        .and_then(|r| r.get("targets"))
        .and_then(Value::as_array)
        .map(|ts| {
            ts.iter()
                .filter_map(|t| t.get("displayName").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        names.contains(&"ModuleA") && names.contains(&"ModuleB"),
        "targets should come from projectPath, not workspacePath: {names:?}"
    );
}

/// When `buildServer.json` carries no `--config` (an older or hand-written stub),
/// the server must still find its config by discovering it from the cwd via the
/// extension's discovery index (`projects.json`, keyed by canonical workspace
/// path, with a `bspConfig` pointer).
#[test]
fn bsp_discovers_config_from_cwd_index() {
    let root = std::env::temp_dir().join(format!("sweetpad-bsp-cwd-{}", std::process::id()));
    let workspace = root.join("workspace");
    let state_home = root.join("xdg-state");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    // The bsp.json lives out of the project tree, in the per-project state dir.
    let config_dir = state_home.join("sweetpad").join("projects").join("hash");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    let config_path = config_dir.join("bsp.json");
    let config = json!({
        "name": "sweetpad",
        "workspacePath": workspace.to_string_lossy(),
        "projectPath": project(),
        "configuration": "Debug",
    });
    std::fs::write(&config_path, config.to_string()).expect("write bsp.json");

    // The index maps the canonical workspace path to that bsp.json.
    let index_dir = state_home.join("sweetpad");
    let key = std::fs::canonicalize(&workspace).unwrap();
    let index = json!({
        "version": 1,
        "projects": { key.to_str().unwrap(): { "bspConfig": config_path.to_string_lossy() } },
    });
    std::fs::write(index_dir.join("projects.json"), index.to_string()).expect("write index");

    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"workspace/buildTargets"}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let mut input = Vec::new();
    for m in &messages {
        input.extend(frame(m));
    }
    // No --config: discovery falls back to the cwd + index.
    let mut child = Command::new(env!("CARGO_BIN_EXE_bsp-server"))
        .arg("bsp")
        .current_dir(&workspace)
        .env("XDG_STATE_HOME", &state_home)
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
    let status = child.wait().expect("wait");
    let _ = std::fs::remove_dir_all(&root);

    assert!(status.success(), "server exited non-zero: {status:?}");
    let frames = parse_frames(&out);
    assert!(
        result_for(&frames, 1).is_some(),
        "no initialize result; frames: {frames:?}"
    );
    let names: Vec<&str> = result_for(&frames, 2)
        .and_then(|r| r.get("targets"))
        .and_then(Value::as_array)
        .map(|ts| {
            ts.iter()
                .filter_map(|t| t.get("displayName").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        names.contains(&"ModuleA") && names.contains(&"ModuleB"),
        "targets should be resolved from the cwd-discovered config: {names:?}"
    );
}

/// A frame whose body isn't valid JSON must get a `-32700` parse-error reply
/// (id null) instead of being silently dropped, and the session must continue.
#[test]
fn bsp_replies_parse_error_on_malformed_frame() {
    let proj = project();
    let bad_body = "{not json";
    let mut input = format!("Content-Length: {}\r\n\r\n{bad_body}", bad_body.len()).into_bytes();
    for m in [
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ] {
        input.extend(frame(&m));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_bsp-server"))
        .args(["bsp", "--project", &proj])
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

    let frames = parse_frames(&out);
    let parse_error = frames.iter().find(|f| {
        f.get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_i64)
            == Some(-32700)
    });
    assert!(parse_error.is_some(), "expected a -32700 reply: {frames:?}");
    assert!(
        result_for(&frames, 1).is_some(),
        "session should continue after a parse error: {frames:?}"
    );
}

/// `Content-Length` matching is case-insensitive (clients aren't required to
/// send the canonical casing), and a malformed length is a clean non-zero
/// exit, not a panic.
#[test]
fn bsp_framing_header_robustness() {
    let proj = project();

    // Lowercase header: the frame must still be read and answered.
    let body = json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}).to_string();
    let mut input = format!("content-length: {}\r\n\r\n{body}", body.len()).into_bytes();
    input.extend(frame(&json!({"jsonrpc":"2.0","method":"build/exit"})));
    let mut child = Command::new(env!("CARGO_BIN_EXE_bsp-server"))
        .args(["bsp", "--project", &proj])
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
    assert!(
        result_for(&parse_frames(&out), 1).is_some(),
        "lowercase content-length header should be accepted"
    );

    // Unparseable length: the frame boundary is unrecoverable — exit non-zero
    // without panicking (a panic would abort with a signal, not a code).
    let mut child = Command::new(env!("CARGO_BIN_EXE_bsp-server"))
        .args(["bsp", "--project", &proj])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn bsp server");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Content-Length: nope\r\n\r\n{}")
        .expect("write stdin");
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code(),
        Some(1),
        "expected clean error exit: {status:?}"
    );
}

/// A frame declaring a giant `Content-Length` must be rejected before the
/// body buffer is allocated — previously `vec![0u8; len]` aborted the whole
/// process on allocation failure (or committed gigabytes and hung waiting
/// for a body that never comes). Expect a clean error exit, not a signal.
#[test]
fn bsp_rejects_oversized_content_length() {
    let proj = project();
    let mut child = Command::new(env!("CARGO_BIN_EXE_bsp-server"))
        .args(["bsp", "--project", &proj])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn bsp server");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Content-Length: 10000000000\r\n\r\n{}")
        .expect("write stdin");
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code(),
        Some(1),
        "expected clean error exit (a None code means a signal/abort): {status:?}"
    );
}
