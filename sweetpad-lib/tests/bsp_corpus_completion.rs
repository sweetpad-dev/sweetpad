//! Corpus-scale completion oracle — the "does the editor actually work" measure
//! from DOCS.md §8 (BSP server), generalized from the single synthetic fixture
//! (`bsp_lsp_e2e.rs`) to the real OSS corpus. It turns the hand-wavy "works on
//! ~X% of projects" into a **measured** figure.
//!
//! Per project it: builds the main scheme once into a throwaway DerivedData
//! (so the dependency modules + generated inputs exist), writes a
//! `buildServer.json` pointing a real headless `sourcekit-lsp` at our `bsp`
//! server, then opens a bounded sample of the project's own source files and
//! pulls diagnostics. The headline metric is the **clean-file rate**: the
//! fraction of files with zero *module/header-resolution* diagnostics — the
//! failures caused by wrong or missing compiler args (`no such module`, a header
//! `file not found`, …), which are unambiguously the build server's fault. A
//! `sourcekit-lsp` internal/stdlib-load error is bucketed separately (retried
//! once): a degraded experience, but an environment rough edge, not an arg bug.
//!
//! Build-first is deliberate: it isolates "are our args/search-paths correct?"
//! from "does prepare work?" (the latter is `bsp_lsp_e2e.rs`'s prepare variant).
//!
//! Opt-in (`BSP_CORPUS=1`): builds real projects with `xcodebuild` and runs
//! `sourcekit-lsp`, so it's slow and needs the cloned corpus (`corpus/<slug>/`,
//! recreated by `scripts/01_clone_corpus.py`) plus Xcode 26.5. Knobs:
//! `BSP_CORPUS_ONLY=<slug[,slug]>` to scope to some projects,
//! `BSP_CORPUS_SAMPLE=<n>` to cap files per project (default 30).

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const XCODE: &str = "/Applications/Xcode-26.5.0.app";
const DEFAULT_SAMPLE: usize = 30;

/// A corpus project the harness can drive: the `.xcodeproj` to point the server
/// at, the scheme that builds the module(s) we measure, and the `xcodebuild`
/// destination to build for (must match the platform our args resolve against —
/// macOS frameworks resolve against `macosx`, so we build for macOS).
struct CorpusProject {
    slug: &'static str,
    xcodeproj: &'static str,
    scheme: &'static str,
    destination: &'static str,
    /// Project dir relative to the slug dir: `"."` for a corpus clone
    /// (`corpus/<slug>/`), `"project"` for a committed synthetic fixture
    /// (`fixtures/<slug>/project/`). A `_synthetic-` slug roots under `fixtures/`.
    project_root: &'static str,
    /// When set, build the `.xcworkspace` (CocoaPods needs the workspace so the
    /// Pods build) while the BSP still points at `xcodeproj`.
    workspace: Option<&'static str>,
    /// Filename substrings that must always be measured even if even-sampling
    /// skipped them — the probe files that reference generated/Pod symbols.
    forced_probes: &'static [&'static str],
    /// A minimal fixture that must analyze fully clean, so *any* error-severity
    /// diagnostic (e.g. an unresolved generated symbol — an in-scope error, not
    /// "no such module") counts as our failure, not just module-resolution ones.
    strict: bool,
    /// Build with `build-for-testing` (vs `build`) and measure the **test**
    /// targets' files too — so a test file's `import XCTest` / `@testable import`
    /// resolution is exercised instead of skipped.
    build_for_testing: bool,
}

/// The corpus clones (build headlessly, measure arg/module-resolution quality)
/// plus the committed synthetic fixtures that probe the generated-source and
/// CocoaPods surfaces the clones don't exercise.
const PROJECTS: &[CorpusProject] = &[
    // A unit-test target: `ProbeTests.swift` uses `import XCTest` + `@testable
    // import Lib`. Built `build-for-testing` (so `Lib.swiftmodule` exists), it
    // resolves only if the BSP gives the test file the unit-test `-F` search
    // paths — the test-target editor path every other project skips.
    CorpusProject {
        slug: "_synthetic-tests",
        xcodeproj: "TestProbe.xcodeproj",
        scheme: "Lib",
        destination: "platform=macOS",
        project_root: "project",
        workspace: None,
        forced_probes: &["Probe"],
        strict: true,
        build_for_testing: true,
    },
    CorpusProject {
        slug: "kingfisher",
        xcodeproj: "Kingfisher.xcodeproj",
        scheme: "Kingfisher",
        destination: "platform=macOS",
        project_root: ".",
        workspace: None,
        forced_probes: &[],
        strict: false,
        build_for_testing: false,
    },
    CorpusProject {
        slug: "alamofire",
        xcodeproj: "Alamofire.xcodeproj",
        scheme: "Alamofire macOS",
        destination: "platform=macOS",
        project_root: ".",
        workspace: None,
        forced_probes: &[],
        strict: false,
        build_for_testing: false,
    },
    // The hard cases: real apps with many cross-module targets (local packages,
    // app extensions, ObjC+Swift, generated sources). They may not build
    // headlessly — the harness reports that honestly rather than hiding it.
    CorpusProject {
        slug: "ice-cubes",
        xcodeproj: "IceCubesApp.xcodeproj",
        scheme: "IceCubesApp",
        destination: "generic/platform=iOS Simulator",
        project_root: ".",
        workspace: None,
        forced_probes: &[],
        strict: false,
        build_for_testing: false,
    },
    CorpusProject {
        slug: "netnewswire",
        xcodeproj: "NetNewsWire.xcodeproj",
        scheme: "NetNewsWire",
        destination: "platform=macOS",
        project_root: ".",
        workspace: None,
        forced_probes: &[],
        strict: false,
        build_for_testing: false,
    },
    // Committed synthetic fixtures (fixtures/_synthetic-*/project/), each with a
    // Probe*.swift that references a build-time-generated symbol. They build
    // clean (xcodebuild generates the symbol), so any editor error on the probe
    // means our BSP didn't surface the generated source / Pod search path.
    CorpusProject {
        slug: "_synthetic-coredata",
        xcodeproj: "CoreDataGen.xcodeproj",
        scheme: "CoreDataGen",
        destination: "platform=macOS",
        project_root: "project",
        workspace: None,
        forced_probes: &["Probe"],
        strict: true,
        build_for_testing: false,
    },
    CorpusProject {
        slug: "_synthetic-assetsym",
        xcodeproj: "AssetSym.xcodeproj",
        scheme: "AssetSym",
        destination: "platform=macOS",
        project_root: "project",
        workspace: None,
        forced_probes: &["Probe"],
        strict: true,
        build_for_testing: false,
    },
    CorpusProject {
        slug: "_synthetic-strcat",
        xcodeproj: "StringCatGen.xcodeproj",
        scheme: "StringCatGen",
        destination: "platform=macOS",
        project_root: "project",
        workspace: None,
        forced_probes: &["Probe"],
        strict: true,
        build_for_testing: false,
    },
    CorpusProject {
        slug: "_synthetic-intents",
        xcodeproj: "IntentsGen.xcodeproj",
        scheme: "IntentsGen",
        destination: "platform=macOS",
        project_root: "project",
        workspace: None,
        forced_probes: &["Probe"],
        strict: true,
        build_for_testing: false,
    },
    CorpusProject {
        slug: "_synthetic-cocoapods",
        xcodeproj: "App.xcodeproj",
        scheme: "App",
        destination: "generic/platform=iOS Simulator",
        project_root: "project",
        workspace: Some("App.xcworkspace"),
        forced_probes: &["Probe"],
        strict: true,
        build_for_testing: false,
    },
    // A third-party Swift macro: `Probe.swift` uses `#stringify` from the bundled
    // `SweetMacro` package, whose implementation lives only in a `.macro` plugin
    // executable Xcode builds into the host products dir. The reference resolves
    // only if the BSP emits `-load-plugin-executable <plugin>#<module>` for it.
    CorpusProject {
        slug: "_synthetic-macro",
        xcodeproj: "MacroProbe.xcodeproj",
        scheme: "MacroProbe",
        destination: "platform=macOS",
        project_root: "project",
        workspace: None,
        forced_probes: &["Probe"],
        strict: true,
        build_for_testing: false,
    },
    // A multiplatform `SDKROOT = auto` app (the IceCubesApp shape, miniaturized):
    // one target, `SUPPORTED_PLATFORMS = iphoneos iphonesimulator macosx`. Built
    // for the iOS simulator — the editor picks `iphonesimulator`, so the BSP must
    // bind `auto` to that SDK and emit a matching `-target` or the stdlib won't
    // load. The committed regression for the SDKROOT=auto class, end to end.
    CorpusProject {
        slug: "_synthetic-multiplatform",
        xcodeproj: "MultiPlatformApp.xcodeproj",
        scheme: "MultiPlatformApp",
        destination: "generic/platform=iOS Simulator",
        project_root: "project",
        workspace: None,
        forced_probes: &["Probe"],
        strict: true,
        build_for_testing: false,
    },
    // Real-world generated-source validation against a Tuist example (`Model.swift`
    // uses the generated Core Data class `User`). Needs a one-time
    // `tuist generate --path corpus/_tuist-src/examples/xcode/generated_ios_app_with_coredata`
    // (the .xcodeproj is gitignored, like the OSS clones), so the harness skips it
    // when absent. iOS-simulator → also subject to the stdlib-load rough edge
    // (see `is_internal_error`).
    CorpusProject {
        slug: "_tuist-src/examples/xcode/generated_ios_app_with_coredata",
        xcodeproj: "App.xcodeproj",
        scheme: "App",
        destination: "generic/platform=iOS Simulator",
        project_root: ".",
        workspace: None,
        forced_probes: &["Model"],
        strict: true,
        build_for_testing: false,
    },
];

/// Wall-clock cap for a single project's build (`BSP_CORPUS_BUILD_TIMEOUT`,
/// seconds). Bounds a pathological/headless-incompatible project.
fn build_timeout() -> Duration {
    let secs = std::env::var("BSP_CORPUS_BUILD_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(420);
    Duration::from_secs(secs)
}

/// Run a build with a wall-clock cap, killing it on timeout so a hung build
/// can't wedge the harness. stdout is discarded; stderr goes to `errlog` (read
/// for a tail on failure). Returns `Ok(success)`.
fn run_build(cmd: &mut Command, timeout: Duration, errlog: &Path) -> Result<bool, String> {
    let sink = std::fs::File::create(errlog).map_err(|e| format!("errlog: {e}"))?;
    let mut child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::from(sink))
        .spawn()
        .map_err(|e| format!("launch: {e}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("timed out".into());
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(e) => return Err(format!("wait: {e}")),
        }
    }
}

fn developer_dir() -> String {
    format!("{XCODE}/Contents/Developer")
}

fn tool(name: &str) -> String {
    format!(
        "{}/Toolchains/XcodeDefault.xctoolchain/usr/bin/{name}",
        developer_dir()
    )
}

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("corpus")
}

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn lsp_frame(msg: &Value) -> Vec<u8> {
    let body = msg.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
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

/// A diagnostic that means a module or header couldn't be resolved — i.e. the
/// build server fed `sourcekit-lsp` arguments missing a search path / module
/// input. High precision: these are the server's fault, not the project's code.
fn is_resolution_failure(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("no such module")
        || m.contains("could not build module")
        || m.contains("missing required module")
        || m.contains("cannot load underlying module")
        || m.contains("module map file")
        || m.contains("umbrella header")
        || (m.contains("file not found") && (m.contains(".h") || m.contains("module")))
}

/// What a project measurement produced.
struct Report {
    slug: String,
    /// Set when the build/setup couldn't complete; measurement was skipped.
    skipped: Option<String>,
    candidates: usize,
    sampled: usize,
    clean: usize,
    /// Files with at least one resolution failure (the headline failures).
    failed: usize,
    /// Files where `sourcekit-lsp` returned an internal error (e.g. "Loading the
    /// standard library failed") that the standalone-compiler cross-check
    /// *cleared* of being our fault — the genuine rough edge, not an arg bug.
    internal_errors: usize,
    /// Internal-error files the cross-check reclassified as our fault (a subset of
    /// `failed`): a standalone `swiftc` also failed to load the stdlib with our
    /// args. The de-exoneration signal — what the old "it's all upstream" bucket
    /// would have hidden.
    reclassified: usize,
    /// Clean files that still carried some error-severity diagnostic — broader,
    /// lower-precision context (may be the project's own latent errors).
    any_errors: usize,
    /// Example (file, message) pairs for resolution failures, internal errors,
    /// and other project errors — for auditing each bucket.
    samples: Vec<(String, String)>,
    internal_samples: Vec<(String, String)>,
    error_samples: Vec<(String, String)>,
}

/// How one file's diagnostics classify, in priority order: a missing-module/
/// header arg bug, a `sourcekit-lsp` internal error, or clean (carrying at most
/// an unrelated project error).
enum Class {
    Resolution(String),
    Internal(String),
    Clean(Option<String>),
}

impl Class {
    fn is_internal(&self) -> bool {
        matches!(self, Class::Internal(_))
    }
}

/// A *candidate* `sourcekit-lsp` internal failure (e.g. "Loading the standard
/// library failed") — a diagnostic that is not yet charged to anyone, because it
/// can be either a genuine sourcekit-lsp rough edge OR our own bad `-sdk`/
/// `-target` wearing an internal-error mask.
///
/// The classifier never trusts this verdict on its own. A file that lands here is
/// retried once (filtering the build-time module race) and then cross-checked:
/// its real BSP args are driven through a standalone `swiftc -typecheck`
/// (`args_fail_to_load_stdlib`). If the compiler also fails to load the stdlib
/// with our args, the fault is ours and the file is reclassified as a resolution
/// failure; only a clean standalone load leaves it in the internal bucket. That
/// cross-check is the guard the earlier "it's all upstream #2328" assumption
/// lacked — a multiplatform `SDKROOT = auto` mis-binding produced the *identical*
/// stdlib-load message and was wrongly exonerated until a standalone compile
/// separated our bug from the real rough edge.
fn is_internal_error(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("internal sourcekit error")
        || m.contains("loading the standard library failed")
        || m.contains("request failed")
}

fn classify(items: &[Value]) -> Class {
    let mut internal: Option<String> = None;
    let mut other: Option<String> = None;
    for d in items {
        let msg = d.get("message").and_then(Value::as_str).unwrap_or("");
        if is_resolution_failure(msg) {
            return Class::Resolution(msg.to_string());
        }
        if is_internal_error(msg) {
            internal.get_or_insert_with(|| msg.to_string());
        } else if d.get("severity").and_then(Value::as_i64) == Some(1) && other.is_none() {
            other = Some(msg.to_string());
        }
    }
    internal.map_or(Class::Clean(other), Class::Internal)
}

/// The project's own Swift source files that physically exist, via the same
/// queries the BSP server answers (`project::open` + `target_source_files`).
///
/// Restricted to **non-test** targets: a test bundle's modules (XCTest + its
/// test-only deps) only resolve once the *test* target is built, which the
/// framework/app scheme doesn't do — so measuring test files here would charge
/// the build server for a target we didn't prepare. Measuring test targets is a
/// separable concern (build-for-testing), left for a later iteration.
fn swift_sources(xcodeproj: &Path, include_tests: bool) -> Vec<PathBuf> {
    let Ok(project) = sweetpad::project::open(xcodeproj) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for t in &project.targets {
        let is_test = t
            .product_type
            .as_deref()
            .is_some_and(|pt| pt.contains("-test"));
        if is_test && !include_tests {
            continue;
        }
        let srcs = sweetpad::project::target_source_files(xcodeproj, &t.name).unwrap_or_default();
        for p in srcs {
            if p.extension().and_then(|e| e.to_str()) == Some("swift")
                && p.exists()
                && seen.insert(p.clone())
            {
                files.push(p);
            }
        }
    }
    files.sort();
    files
}

/// Evenly sample up to `cap` items so the sample spans the file list rather than
/// just the alphabetical head.
fn sample<T: Clone>(items: &[T], cap: usize) -> Vec<T> {
    if items.len() <= cap {
        return items.to_vec();
    }
    (0..cap)
        .map(|i| items[i * items.len() / cap].clone())
        .collect()
}

fn measure_project(p: &CorpusProject, sample_cap: usize) -> Report {
    let mut report = Report {
        slug: p.slug.into(),
        skipped: None,
        candidates: 0,
        sampled: 0,
        clean: 0,
        failed: 0,
        internal_errors: 0,
        reclassified: 0,
        any_errors: 0,
        samples: Vec::new(),
        internal_samples: Vec::new(),
        error_samples: Vec::new(),
    };
    // Corpus clones live under corpus/<slug>/; committed synthetic fixtures under
    // fixtures/<slug>/project/. The project dir is also the sourcekit-lsp root
    // (where buildServer.json goes).
    let base = if p.slug.starts_with("_synthetic-") {
        fixtures_root()
    } else {
        corpus_root()
    };
    let project_dir = base.join(p.slug).join(p.project_root);
    let xcodeproj = project_dir.join(p.xcodeproj);
    if !xcodeproj.exists() {
        report.skipped = Some(format!(
            "no project at {} (corpus clone / fixture missing)",
            xcodeproj.display()
        ));
        return report;
    }

    // A slug can contain '/' (a nested Tuist example path), so flatten it for the
    // throwaway temp file names.
    let tag = p.slug.replace('/', "-");
    let dd = std::env::temp_dir().join(format!("sweetpad-corpus-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dd);

    // Build once so the module graph + generated inputs exist where our search
    // paths point.
    let errlog = std::env::temp_dir().join(format!(
        "sweetpad-corpus-{}-{}.err",
        tag,
        std::process::id()
    ));
    let timeout = build_timeout();
    eprintln!(
        "[{}] building scheme {:?} for {} (≤{}s) …",
        p.slug,
        p.scheme,
        p.destination,
        timeout.as_secs()
    );
    let mut cmd = Command::new("xcodebuild");
    cmd.env("DEVELOPER_DIR", developer_dir())
        .arg(if p.build_for_testing {
            "build-for-testing"
        } else {
            "build"
        });
    // CocoaPods needs the workspace built so the Pods build; everything else
    // builds the project directly.
    if let Some(ws) = p.workspace {
        cmd.arg("-workspace").arg(project_dir.join(ws));
    } else {
        cmd.arg("-project").arg(&xcodeproj);
    }
    cmd.args([
        "-scheme",
        p.scheme,
        "-configuration",
        "Debug",
        "-destination",
        p.destination,
        "-derivedDataPath",
    ])
    .arg(&dd)
    .args([
        "CODE_SIGNING_ALLOWED=NO",
        "-skipMacroValidation",
        "-skipPackagePluginValidation",
    ]);
    match run_build(&mut cmd, timeout, &errlog) {
        Ok(true) => {}
        Ok(false) => {
            let log = std::fs::read_to_string(&errlog).unwrap_or_default();
            let tail: Vec<&str> = log
                .lines()
                .filter(|l| !l.trim().is_empty())
                .rev()
                .take(3)
                .collect();
            report.skipped = Some(format!(
                "build failed: {}",
                tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
            ));
            let _ = std::fs::remove_dir_all(&dd);
            let _ = std::fs::remove_file(&errlog);
            return report;
        }
        Err(e) => {
            report.skipped = Some(format!("build {e}"));
            let _ = std::fs::remove_dir_all(&dd);
            let _ = std::fs::remove_file(&errlog);
            return report;
        }
    }
    let _ = std::fs::remove_file(&errlog);

    // Point sourcekit-lsp at our server (config dog-foods the real command).
    let build_server = project_dir.join("buildServer.json");
    let cfg = Command::new(env!("CARGO_BIN_EXE_sweetpad-lib"))
        .args(["config", "--project"])
        .arg(&xcodeproj)
        .args(["--xcode", XCODE, "--derived-data-path"])
        .arg(&dd)
        .arg("--output")
        .arg(&build_server)
        .stderr(Stdio::null())
        .status();
    if !cfg.map(|s| s.success()).unwrap_or(false) {
        report.skipped = Some("config (buildServer.json) failed".into());
        let _ = std::fs::remove_dir_all(&dd);
        return report;
    }

    let candidates = swift_sources(&xcodeproj, p.build_for_testing);
    report.candidates = candidates.len();
    let mut files = sample(&candidates, sample_cap);
    // Always measure the forced-probe files (the ones referencing generated/Pod
    // symbols) even if even-sampling skipped them — else the very thing under
    // test could go unmeasured.
    for c in &candidates {
        let name = c.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.forced_probes.iter().any(|pat| name.contains(pat)) && !files.contains(c) {
            files.push(c.clone());
        }
    }
    report.sampled = files.len();
    if files.is_empty() {
        report.skipped = Some("no swift sources discovered".into());
        let _ = std::fs::remove_dir_all(&dd);
        let _ = std::fs::remove_file(&build_server);
        return report;
    }

    measure_files(&project_dir, &xcodeproj, &dd, &files, p.strict, &mut report);

    let _ = std::fs::remove_dir_all(&dd);
    let _ = std::fs::remove_file(&build_server);
    report
}

/// Split a `Content-Length`-framed BSP stream into JSON values.
fn bsp_frames(out: &[u8]) -> Vec<Value> {
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

/// The `compilerArguments` our BSP server returns for `file` (via a one-shot
/// `textDocument/sourceKitOptions`), or `None` when the resolver failed for the
/// owning target. These are the exact args sourcekit-lsp is handed, so they also
/// feed the standalone cross-check that de-exonerates internal errors.
fn bsp_file_args(xcodeproj: &Path, dd: &Path, file: &Path) -> Option<Vec<String>> {
    let uri = format!("file://{}", file.to_string_lossy());
    let msgs = [
        json!({"jsonrpc":"2.0","id":1,"method":"build/initialize","params":{}}),
        json!({"jsonrpc":"2.0","method":"build/initialized"}),
        json!({"jsonrpc":"2.0","id":5,"method":"textDocument/sourceKitOptions","params":{"textDocument":{"uri":uri}}}),
        json!({"jsonrpc":"2.0","method":"build/exit"}),
    ];
    let mut input = Vec::new();
    for m in &msgs {
        input.extend(lsp_frame(m));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_sweetpad-lib"))
        .args(["bsp", "--project"])
        .arg(xcodeproj)
        .args(["--derived-data-path"])
        .arg(dd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&input);
    }
    let mut out = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut out);
    }
    let _ = child.wait();
    bsp_frames(&out).iter().find_map(|f| {
        if f.get("id").and_then(Value::as_i64) != Some(5) {
            return None;
        }
        f.pointer("/result/compilerArguments")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
    })
}

/// Whether our BSP server returns non-empty `sourceKitOptions` for `file`. A
/// `null`/empty reply means our resolver failed for the owning target — and
/// sourcekit-lsp would silently fall back, emitting no diagnostics, which the
/// per-file measurement would otherwise misread as "clean". So a strict fixture
/// gates on this directly, closing that false-clean hole.
fn bsp_serves_options(xcodeproj: &Path, dd: &Path, file: &Path) -> bool {
    bsp_file_args(xcodeproj, dd, file).is_some_and(|a| !a.is_empty())
}

/// Cross-check a candidate internal/stdlib-load error: feed the file's own BSP
/// args to a standalone `swiftc -typecheck`. The editor args already drop the
/// build-only output flags, so this is a pure parse + module load. If the
/// compiler *also* fails to load the standard library, our `-sdk`/`-target` are
/// the cause (a resolution bug, not the sourcekit-lsp rough edge) and the caller
/// charges it to us; a clean load — or only unrelated type errors — exonerates
/// the args and leaves the file in the internal bucket.
fn args_fail_to_load_stdlib(args: &[String]) -> bool {
    let Ok(out) = Command::new(tool("swiftc"))
        .env("DEVELOPER_DIR", developer_dir())
        .arg("-typecheck")
        .args(args)
        .output()
    else {
        return false;
    };
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    err.contains("unable to load standard library")
        || err.contains("loading the standard library failed")
        || err.contains("failed to load module 'swift'")
}

/// Drive one `sourcekit-lsp` session over the sampled files, pulling diagnostics
/// per file and tallying resolution failures into `report`. For a strict fixture
/// each file is first gated on our BSP actually serving options (a null reply is
/// a resolver failure that sourcekit-lsp would otherwise mask).
#[allow(clippy::too_many_lines)] // a linear LSP-driving loop reads clearer in one piece
fn measure_files(
    root: &Path,
    xcodeproj: &Path,
    dd: &Path,
    files: &[PathBuf],
    strict: bool,
    report: &mut Report,
) {
    let mut lsp = Command::new(tool("sourcekit-lsp"))
        .env("DEVELOPER_DIR", developer_dir())
        .current_dir(root)
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
    let send = |stdin: &mut std::process::ChildStdin, msg: &Value| {
        let _ = stdin.write_all(&lsp_frame(msg));
        let _ = stdin.flush();
    };
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

    let root_uri = format!("file://{}", root.to_string_lossy());
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{
                "processId":std::process::id(),"rootUri":root_uri,
                "capabilities":{"textDocument":{"diagnostic":{"dynamicRegistration":false}}},
                "initializationOptions":{}
            }
        }),
    );
    let _ = wait_for_id(&rx, 1, 30);
    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );

    let pull =
        |stdin: &mut std::process::ChildStdin, uri: &str, id: i64, secs: u64| -> Vec<Value> {
            send(
                stdin,
                &json!({
                    "jsonrpc":"2.0","id":id,"method":"textDocument/diagnostic",
                    "params":{"textDocument":{"uri":uri}}
                }),
            );
            wait_for_id(&rx, id, secs)
                .and_then(|r| {
                    r.pointer("/result/items")
                        .and_then(Value::as_array)
                        .cloned()
                })
                .unwrap_or_default()
        };

    for (i, file) in files.iter().enumerate() {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let uri = format!("file://{}", file.to_string_lossy());
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        // For a strict fixture, a null BSP reply is itself the failure (sourcekit
        // would fall back and report nothing — a false clean), so gate on it.
        if strict && !bsp_serves_options(xcodeproj, dd, file) {
            report.failed += 1;
            if report.samples.len() < 8 {
                report.samples.push((
                    name,
                    "BSP returned no sourceKitOptions (resolver failed for the target)".into(),
                ));
            }
            continue;
        }
        send(
            &mut stdin,
            &json!({
                "jsonrpc":"2.0","method":"textDocument/didOpen",
                "params":{"textDocument":{"uri":uri,"languageId":"swift","version":1,"text":text}}
            }),
        );
        // The first file pays the module-graph load (slow); later files are fast.
        let seq = i64::try_from(i).unwrap_or(0);
        let first_timeout = if i == 0 { 90 } else { 30 };
        let mut items = pull(&mut stdin, &uri, 1000 + seq, first_timeout);
        // An internal/stdlib-load error is often the stdlib explicit module still
        // building — retry once after a beat and trust the retry, so a race isn't
        // charged as a real failure.
        if classify(&items).is_internal() {
            std::thread::sleep(Duration::from_secs(3));
            items = pull(&mut stdin, &uri, 5000 + seq, 30);
        }
        match classify(&items) {
            Class::Resolution(msg) => {
                report.failed += 1;
                if report.samples.len() < 8 {
                    report.samples.push((name, msg));
                }
            }
            Class::Internal(msg) => {
                // De-exonerate: keep this in the internal bucket only when a
                // standalone compile with the file's own BSP args loads the
                // stdlib cleanly. If that compile also fails to load the stdlib,
                // our `-sdk`/`-target` are wrong and it is our resolution failure.
                let our_fault = bsp_file_args(xcodeproj, dd, file)
                    .is_some_and(|args| args_fail_to_load_stdlib(&args));
                if our_fault {
                    report.failed += 1;
                    report.reclassified += 1;
                    if report.samples.len() < 8 {
                        report.samples.push((
                            name,
                            format!("[reclassified from internal] standalone swiftc also fails to load the stdlib with our args: {msg}"),
                        ));
                    }
                } else {
                    report.internal_errors += 1;
                    if report.internal_samples.len() < 6 {
                        report.internal_samples.push((name, msg));
                    }
                }
            }
            // In strict mode (a minimal synthetic fixture that must be fully
            // clean) any error is our gap — count it as a failure, not "clean".
            Class::Clean(Some(msg)) if strict => {
                report.failed += 1;
                if report.samples.len() < 8 {
                    report.samples.push((name, msg));
                }
            }
            Class::Clean(other) => {
                report.clean += 1;
                if let Some(msg) = other {
                    report.any_errors += 1;
                    if report.error_samples.len() < 6 {
                        report.error_samples.push((name, msg));
                    }
                }
            }
        }
    }

    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":9999,"method":"shutdown"}),
    );
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"exit"}));
    drop(stdin);
    let _ = lsp.wait();
    let _ = reader.join();
}

#[test]
#[allow(clippy::cast_precision_loss)] // counts are small; report percentages don't need f64 exactness
fn bsp_corpus_completion() {
    if std::env::var("BSP_CORPUS").is_err() {
        eprintln!("skipping: set BSP_CORPUS=1 to run the corpus-scale completion oracle");
        return;
    }
    if !Path::new(&tool("sourcekit-lsp")).exists() {
        eprintln!("skipping: sourcekit-lsp not found under {XCODE}");
        return;
    }

    let only: Option<Vec<String>> = std::env::var("BSP_CORPUS_ONLY")
        .ok()
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect());
    let sample_cap = std::env::var("BSP_CORPUS_SAMPLE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_SAMPLE);

    let mut reports = Vec::new();
    for p in PROJECTS {
        if let Some(only) = &only
            && !only.iter().any(|s| s == p.slug)
        {
            continue;
        }
        reports.push(measure_project(p, sample_cap));
    }

    // Report. The clean-file rate is the measurement — printed, not asserted
    // (we're establishing the number, not gating on a threshold yet).
    eprintln!("\n===== BSP corpus completion (sample cap {sample_cap}/project) =====");
    let (mut tot_sampled, mut tot_clean, mut measured) = (0usize, 0usize, 0usize);
    for r in &reports {
        if let Some(reason) = &r.skipped {
            eprintln!("  {:<14} SKIPPED — {reason}", r.slug);
            continue;
        }
        measured += 1;
        tot_sampled += r.sampled;
        tot_clean += r.clean;
        let pct = if r.sampled > 0 {
            100.0 * r.clean as f64 / r.sampled as f64
        } else {
            0.0
        };
        eprintln!(
            "  {:<14} clean {:>3}/{:<3} ({pct:>5.1}%)  resolution-fail {:<3} (incl {:<2} reclassified)  internal-err {:<3} proj-error {:<3}  [of {} candidates]",
            r.slug,
            r.clean,
            r.sampled,
            r.failed,
            r.reclassified,
            r.internal_errors,
            r.any_errors,
            r.candidates
        );
        for (file, msg) in &r.samples {
            eprintln!("                  ↳ resolution-fail {file}: {msg}");
        }
        for (file, msg) in &r.internal_samples {
            eprintln!("                  · internal-error {file}: {msg}");
        }
        for (file, msg) in &r.error_samples {
            eprintln!("                  · proj-error {file}: {msg}");
        }
    }
    if tot_sampled > 0 {
        eprintln!(
            "  {:-<14} TOTAL clean {tot_clean}/{tot_sampled} ({:.1}%) across {measured} project(s)",
            "",
            100.0 * tot_clean as f64 / tot_sampled as f64
        );
    }
    eprintln!("=================================================================\n");

    assert!(
        measured > 0,
        "no corpus project could be built + measured; see SKIPPED reasons above"
    );
    // Guard the harness itself: a *resolution* failure on every file means our
    // args point nowhere (wrong DerivedData / server not launched) — a setup
    // fault, not a real-world signal. (An all-internal-error project is a real,
    // if degraded, result and must not trip this.)
    let dead = reports
        .iter()
        .find(|r| r.skipped.is_none() && r.sampled > 0 && r.failed == r.sampled);
    assert!(
        dead.is_none(),
        "project {:?} had a resolution failure on all {} files — likely a harness/setup fault, not a quality measure",
        dead.map_or("", |r| r.slug.as_str()),
        dead.map_or(0, |r| r.sampled),
    );
}
