//! Thin wrapper over `xcodebuild` for the build/run commands: assembling the
//! argument vector (mirroring the VS Code extension's proven invocation) and
//! reading back the build settings needed to locate and launch the built app.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sweetpad_core::build_settings::{BuildSettingsOptions, resolve_build_settings};

use crate::cli::output::Output;
use crate::cli::resolve::Container;
use crate::cli::{CliError, ErrorContext, ErrorKind, buildlog, process};

/// Everything needed to invoke `xcodebuild build` for a resolved target.
pub struct BuildPlan<'a> {
    pub container: &'a Container,
    pub scheme: &'a str,
    pub configuration: &'a str,
    /// Raw `-destination` specifier, e.g. `platform=iOS Simulator,id=<udid>`.
    pub destination: Option<&'a str>,
    /// `-sdk` override (`--sdk` / config / `context select sdk`); `None` lets
    /// the destination imply it.
    pub sdk: Option<&'a str>,
    pub clean: bool,
    /// Hot-reload build: add `-Xlinker -interposable` (so dyld can swap symbols)
    /// and `EMIT_FRONTEND_COMMAND_LINES=YES` (so the build-log recompiler can
    /// recover per-file commands). A macOS destination additionally disables the
    /// hardened runtime and App Sandbox so the product is injectable. Set for
    /// simulator and macOS builds under `--hot`.
    pub hot: bool,
    /// Entitlements override for a hot macOS build (CLI_DESIGN §9d zero-config
    /// sandbox stripping): the ephemeral sandbox-stripped plist (or a
    /// `--hot-entitlements` file) that `CODE_SIGN_ENTITLEMENTS=…` points the
    /// signing at. Only emitted for a hot macOS build.
    pub hot_entitlements: Option<&'a Path>,
    /// Extra arguments passed through to xcodebuild verbatim (everything after
    /// `--` on the command line) — the escape hatch for flags/settings the CLI
    /// doesn't model.
    pub passthrough: &'a [String],
}

impl BuildPlan<'_> {
    /// The `xcodebuild` argument vector: `[clean] build -scheme … -configuration
    /// … [-destination …] [-sdk …] [-workspace|-project …]`.
    fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        if self.clean {
            args.push("clean".into());
        }
        args.push("build".into());
        args.push("-scheme".into());
        args.push(self.scheme.into());
        args.push("-configuration".into());
        args.push(self.configuration.into());
        if let Some(dest) = self.destination {
            args.push("-destination".into());
            args.push(dest.into());
        }
        if let Some(sdk) = self.sdk {
            args.push("-sdk".into());
            args.push(sdk.into());
        }
        args.extend(container_args(self.container));
        if self.hot {
            // Build settings (KEY=VALUE) after the action; `$(inherited)` keeps
            // any project OTHER_LDFLAGS. Mirrors the VS Code extension + the
            // validated spike fixture.
            args.push("OTHER_LDFLAGS=$(inherited) -Xlinker -interposable".into());
            args.push("EMIT_FRONTEND_COMMAND_LINES=YES".into());
            // A native macOS app must be injectable: the hardened runtime makes
            // dyld strip `DYLD_INSERT_LIBRARIES` and library validation reject
            // the ad-hoc recompiled dylibs, and the App Sandbox blocks both the
            // client's socket and dlopen from outside the container. Command-line
            // settings outrank project ones, so the hot Debug product is built
            // without either protection. (A sandbox declared in an explicit
            // entitlements file is beyond build settings — the mac preflight
            // catches that case with instructions.)
            if self.destination.is_some_and(is_macos_destination) {
                args.push("ENABLE_HARDENED_RUNTIME=NO".into());
                args.push("ENABLE_APP_SANDBOX=NO".into());
                // An explicit entitlements plist outranks those settings at
                // signing time; the ephemeral stripped copy wins it back.
                if let Some(entitlements) = self.hot_entitlements {
                    args.push(format!("CODE_SIGN_ENTITLEMENTS={}", entitlements.display()));
                }
            }
        }
        args.extend(self.passthrough.iter().cloned());
        args
    }

    /// The `(argv, cwd)` for this build, exposed so the interactive `app run`
    /// session can spawn xcodebuild itself (interruptibly) instead of going
    /// through [`run`].
    #[must_use]
    pub fn command(&self) -> (Vec<String>, Option<PathBuf>) {
        (self.args(), working_dir(self.container))
    }

    /// Run the build. Human mode beautifies xcodebuild's output via
    /// [`buildlog`]; `-v` passes it through raw; `--json` captures both child
    /// streams (nothing interleaves with the envelope) and folds the tail of
    /// the transcript into the error on failure; `-o ndjson` streams one event
    /// per line (the returned stats ride into the terminal result event).
    /// Every mode but `-v` records the parsed diagnostics as the project's
    /// last-build artifact for `build diagnostics`.
    pub fn run(&self, out: &Output) -> Result<Option<buildlog::StreamStats>, CliError> {
        let parts = self.args();
        let args: Vec<&str> = parts.iter().map(String::as_str).collect();
        let cwd = working_dir(self.container);
        let mut failure_detail = String::new();
        let mut stats = None;
        let mut diagnostics = Vec::new();
        let mut blocker = None;
        // Only the raw `-v` human passthrough leaves output unparsed; every
        // parsing mode (including ndjson under `-v`) records the artifact.
        let mut parsed = true;
        let ok = if out.is_ndjson() {
            let (ok, s) = buildlog::run_ndjson("xcodebuild", &args, cwd.as_deref(), out)?;
            diagnostics.clone_from(&s.diagnostics);
            blocker.clone_from(&s.blocker);
            stats = Some(s);
            ok
        } else if out.is_json() {
            let run = process::run_captured("xcodebuild", &args, cwd.as_deref())?;
            diagnostics = buildlog::diagnostics_from_transcript(&run.combined);
            if !run.success {
                blocker = buildlog::blocker_from_transcript(&run.combined);
                failure_detail = captured_failure_detail(
                    self.container,
                    &run.combined,
                    &run.tail,
                    &diagnostics,
                    blocker.is_some(),
                );
            }
            run.success
        } else if out.is_verbose() {
            // Raw passthrough is unparsed — no artifact for this mode.
            parsed = false;
            process::run("xcodebuild", &args, cwd.as_deref(), false)?
        } else {
            let (ok, d, b) =
                buildlog::run_collecting("xcodebuild", &args, cwd.as_deref(), out, "Building")?;
            diagnostics = d;
            blocker = b;
            ok
        };
        if parsed {
            record_build_diagnostics(self.container, ok, &diagnostics);
        }
        if ok {
            Ok(stats)
        } else {
            // Classified here, the one chokepoint every build goes through, so
            // `build start` and `app run`'s build step both exit 3 on a failed
            // compile instead of the generic 1.
            let headline = blocker.map_or_else(
                || format!("xcodebuild exited with a non-zero status{failure_detail}"),
                |hint| format!("the build is blocked, not broken: {hint}"),
            );
            Err(CliError::new(headline)
                .kind(ErrorKind::BuildFailure)
                .diagnostics(diagnostics)
                .context("building the project"))
        }
    }
}

/// Whether a `-destination` specifier targets native macOS (the platform whose
/// hot builds need the injectability settings).
fn is_macos_destination(spec: &str) -> bool {
    spec.split(',')
        .find_map(|kv| kv.trim().strip_prefix("platform="))
        .is_some_and(|p| p.trim() == "macOS")
}

/// `-workspace <path>` / `-project <path>`; nothing for a Swift package (it's
/// driven from the package directory). Shared with the `dependency` command's
/// `-resolvePackageDependencies` invocation.
pub(crate) fn container_args(container: &Container) -> Vec<String> {
    match container {
        Container::Workspace(p) => vec!["-workspace".into(), p.display().to_string()],
        Container::Project(p) => vec!["-project".into(), p.display().to_string()],
        Container::SwiftPackage(_) => Vec::new(),
    }
}

/// Directory to run xcodebuild from: the container's parent (or the package
/// directory for SPM). A relative container like `App.xcodeproj` has an empty
/// parent — that means "the current directory", so return `None` rather than
/// trying to `chdir("")` (which fails the spawn and looks like a missing tool).
pub(crate) fn working_dir(container: &Container) -> Option<PathBuf> {
    container
        .path()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

/// Everything needed to invoke `xcodebuild test` for a resolved target.
pub struct TestPlan<'a> {
    pub container: &'a Container,
    pub scheme: &'a str,
    pub configuration: &'a str,
    pub destination: Option<&'a str>,
    /// `-sdk` override; `None` lets the destination imply it.
    pub sdk: Option<&'a str>,
    /// `-only-testing:` selectors (Target/Class/method); empty runs everything.
    pub only_testing: &'a [String],
    /// `-skip-testing:` selectors.
    pub skip_testing: &'a [String],
    /// Where xcodebuild writes the `.xcresult` bundle (parsed for the summary).
    pub result_bundle: &'a Path,
    /// Retry failing tests, running each up to N times
    /// (`-retry-tests-on-failure -test-iterations N`).
    pub retry_flaky: Option<u32>,
    /// Collect code coverage (`-enableCodeCoverage YES`).
    pub coverage: bool,
    /// Extra xcodebuild arguments passed through verbatim (after `--`).
    pub passthrough: &'a [String],
}

/// What a [`TestPlan::run`] produced: the raw pass/fail, plus — in the
/// captured (`--json`) mode — the transcript tail, so a run that failed
/// before any test ran can surface *why* instead of a vacuous zero-count
/// summary.
pub struct TestRunOutcome {
    pub passed: bool,
    pub tail: Option<String>,
    /// Diagnostics parsed from the run's output, so a run that died in its
    /// build step reports the compile errors as data rather than as a log.
    /// Empty in the modes that already showed them to the user (`-v`, the
    /// beautified human stream).
    pub diagnostics: Vec<serde_json::Value>,
    /// The whole captured transcript (`--json` only, where nothing reached the
    /// terminal), for [`record_failure_transcript`].
    pub transcript: Option<String>,
    /// Set when the run was blocked rather than broken — a policy gate no
    /// compile error describes (see [`buildlog::BlockerWatch`]).
    pub blocker: Option<String>,
}

impl TestPlan<'_> {
    fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "test".into(),
            "-scheme".into(),
            self.scheme.into(),
            "-configuration".into(),
            self.configuration.into(),
            "-resultBundlePath".into(),
            self.result_bundle.display().to_string(),
        ];
        if let Some(dest) = self.destination {
            args.push("-destination".into());
            args.push(dest.into());
        }
        if let Some(sdk) = self.sdk {
            args.push("-sdk".into());
            args.push(sdk.into());
        }
        if let Some(iterations) = self.retry_flaky {
            args.push("-retry-tests-on-failure".into());
            args.push("-test-iterations".into());
            args.push(iterations.to_string());
        }
        if self.coverage {
            args.push("-enableCodeCoverage".into());
            args.push("YES".into());
        }
        args.extend(container_args(self.container));
        for t in self.only_testing {
            args.push(format!("-only-testing:{t}"));
        }
        for t in self.skip_testing {
            args.push(format!("-skip-testing:{t}"));
        }
        args.extend(self.passthrough.iter().cloned());
        args
    }

    /// The `(argv, cwd)` for this test run — for `--show-command`.
    #[must_use]
    pub fn command(&self) -> (Vec<String>, Option<PathBuf>) {
        (self.args(), working_dir(self.container))
    }

    /// Run the tests. `--json` captures both child streams (stdout holds only
    /// the enveloped summary), `-o ndjson` streams per-test events, `-v` is
    /// raw, otherwise xcodebuild output is beautified. A test failure is
    /// `passed: false`, not an error; whether the run got far enough to
    /// produce a usable result bundle is the *caller's* judgment (it owns the
    /// bundle lifecycle) — the tail rides back for its error message.
    pub fn run(&self, out: &Output) -> Result<TestRunOutcome, CliError> {
        let parts = self.args();
        let args: Vec<&str> = parts.iter().map(String::as_str).collect();
        let cwd = working_dir(self.container);
        let outcome = if out.is_ndjson() {
            let (ok, stats) = buildlog::run_ndjson("xcodebuild", &args, cwd.as_deref(), out)?;
            TestRunOutcome {
                passed: ok,
                tail: None,
                diagnostics: stats.diagnostics,
                transcript: None,
                blocker: stats.blocker,
            }
        } else if out.is_json() {
            let run = process::run_captured("xcodebuild", &args, cwd.as_deref())?;
            let diagnostics = if run.success {
                Vec::new()
            } else {
                buildlog::diagnostics_from_transcript(&run.combined)
            };
            TestRunOutcome {
                passed: run.success,
                tail: (!run.success).then_some(run.tail),
                blocker: (!run.success)
                    .then(|| buildlog::blocker_from_transcript(&run.combined))
                    .flatten(),
                diagnostics,
                transcript: (!run.success).then_some(run.combined),
            }
        } else if out.is_verbose() {
            let ok = process::run("xcodebuild", &args, cwd.as_deref(), false)
                .context("running the tests")?;
            TestRunOutcome {
                passed: ok,
                tail: None,
                diagnostics: Vec::new(),
                transcript: None,
                blocker: None,
            }
        } else {
            let (ok, diagnostics, blocker) =
                buildlog::run_collecting("xcodebuild", &args, cwd.as_deref(), out, "Testing")
                    .context("running the tests")?;
            TestRunOutcome {
                passed: ok,
                tail: None,
                diagnostics,
                transcript: None,
                blocker,
            }
        };
        Ok(outcome)
    }
}

/// One per-project artifact slot in the state dir
/// (`…/sweetpad/results/<stem>-<hash><suffix>`): the container stem keeps it
/// findable, the key hash keeps two same-named projects apart. FNV-1a rather
/// than `DefaultHasher`, whose algorithm is unspecified across Rust releases —
/// a toolchain bump must not orphan every retained bundle and diagnostics
/// artifact.
pub(crate) fn project_artifact(container: &Container, suffix: &str) -> std::path::PathBuf {
    let stem = container.path().file_stem().map_or_else(
        || "project".to_string(),
        |s| s.to_string_lossy().into_owned(),
    );
    let name = format!(
        "{stem}-{:016x}{suffix}",
        fnv1a64(container.key().as_bytes())
    );
    sweetpad_core::paths::state_dir().map_or_else(
        || std::env::temp_dir().join(&name),
        |d| d.join("sweetpad").join("results").join(&name),
    )
}

/// FNV-1a, 64-bit: tiny, dependency-free, and stable forever — the properties
/// an on-disk slot name needs (`DefaultHasher` guarantees none of them).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Persist the last build's parsed diagnostics for `build diagnostics`
/// — agents stop re-running builds just to re-read the errors. Best-effort: a
/// write failure never fails the build.
pub(crate) fn record_build_diagnostics(
    container: &Container,
    ok: bool,
    diagnostics: &[serde_json::Value],
) {
    let path = project_artifact(container, "-build.json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let errors = diagnostics
        .iter()
        .filter(|d| d["severity"] == "error")
        .count();
    let warnings = diagnostics
        .iter()
        .filter(|d| d["severity"] == "warning")
        .count();
    let record = serde_json::json!({
        "ok": ok,
        "errors": errors,
        "warnings": warnings,
        "diagnostics": diagnostics,
        "finishedAtEpochMs": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or_default(),
    });
    if let Ok(text) = serde_json::to_string_pretty(&record) {
        let _ = std::fs::write(&path, text);
    }
}

/// Park a failed run's raw transcript in the project's artifact slot and return
/// the path, so an error can name the log instead of quoting it. The captured
/// (`--json`) modes send nothing to the terminal, so without this the transcript
/// only survives inside the error message — the thing that makes the message
/// unreadable. Best-effort: `None` when the write fails, and the caller falls
/// back to the tail.
pub(crate) fn record_failure_transcript(
    container: &Container,
    suffix: &str,
    text: &str,
) -> Option<std::path::PathBuf> {
    let path = project_artifact(container, suffix);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, text).ok().map(|()| path)
}

/// What a captured (`--json`) build failure appends to its headline: the
/// summarized diagnostics plus the path the transcript was parked at, or the
/// raw tail when nothing parsed.
///
/// Empty for a `blocked` run, which parks nothing. The blocker headline
/// replaces this detail wholesale, so a transcript written here would be a file
/// no message goes on to name — unfindable on disk, and several KB of package
/// resolution that the blocker already accounts for better.
fn captured_failure_detail(
    container: &Container,
    combined: &str,
    tail: &str,
    diagnostics: &[serde_json::Value],
    blocked: bool,
) -> String {
    if blocked {
        return String::new();
    }
    match diagnostics_summary(diagnostics) {
        Some(summary) => {
            let log = record_failure_transcript(container, "-build.log", combined)
                .map_or_else(String::new, |p| format!("; full log: {}", p.display()));
            format!(": {summary}{log}")
        }
        None => format!(":\n{tail}"),
    }
}

/// Summarize diagnostics for a one-line error message: the first error (falling
/// back to the first diagnostic of any severity) plus a count of the rest, so
/// the headline names the actual cause and the full set rides in the error
/// object's `diagnostics`.
pub(crate) fn diagnostics_summary(diagnostics: &[serde_json::Value]) -> Option<String> {
    let errors: Vec<&serde_json::Value> = diagnostics
        .iter()
        .filter(|d| d["severity"] == "error")
        .collect();
    let (first, total) = match errors.first() {
        Some(first) => (*first, errors.len()),
        None => (diagnostics.first()?, diagnostics.len()),
    };
    let location = first["location"]
        .as_str()
        .map(|l| format!("{l}: "))
        .unwrap_or_default();
    let message = first["message"].as_str().unwrap_or("(no message)");
    let more = match total {
        0 | 1 => String::new(),
        n => format!(" (and {} more)", n - 1),
    };
    Some(format!("{location}{message}{more}"))
}

/// Read the project's last-build diagnostics artifact, if a build recorded one.
#[must_use]
pub fn last_build_diagnostics(container: &Container) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(project_artifact(container, "-build.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// The `--show-command` payload: the exact invocation that would run, shown
/// shell-quoted in human mode or as `{command, cwd}` in the envelope — so users
/// can graduate to raw xcodebuild and agents can plan.
pub struct CommandPreview {
    pub program: &'static str,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

impl crate::cli::Render for CommandPreview {
    fn human(&self, out: &Output) {
        let mut line = String::from(self.program);
        for arg in &self.args {
            line.push(' ');
            line.push_str(&shell_quote(arg));
        }
        out.line(&line);
        if let Some(cwd) = &self.cwd {
            out.note(&format!("in {}", cwd.display()));
        }
    }

    fn json(&self) -> serde_json::Value {
        let mut command = vec![self.program.to_string()];
        command.extend(self.args.iter().cloned());
        serde_json::json!({
            "command": command,
            "cwd": self.cwd.as_ref().map(|p| p.display().to_string()),
        })
    }
}

/// Single-quote an argument for display when it needs it (spaces, quotes,
/// shell metacharacters) — the standard `'…'` with `'\''` escapes.
pub(crate) fn shell_quote(arg: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c);
    if !arg.is_empty() && arg.chars().all(plain) {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

/// Parsed `xcrun xcresulttool get test-results summary` output.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TestSummary {
    pub result: String,
    pub total_test_count: u32,
    pub passed_tests: u32,
    pub failed_tests: u32,
    pub skipped_tests: u32,
    pub test_failures: Vec<TestFailure>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TestFailure {
    pub test_name: String,
    pub target_name: String,
    pub failure_text: String,
}

/// The `-only-testing:` selectors for every test that failed in `bundle`,
/// read from `xcrun xcresulttool get test-results tests` (Xcode 16+): the test
/// tree is walked for failed test cases and their identifiers
/// (`Target/Class/method`) returned. A missing bundle yields an empty list
/// ("no previous run").
pub fn failed_test_selectors(bundle: &Path) -> Result<Vec<String>, CliError> {
    if !bundle.exists() {
        return Ok(Vec::new());
    }
    let out = process::capture(
        "xcrun",
        &[
            "xcresulttool",
            "get",
            "test-results",
            "tests",
            "--path",
            &bundle.to_string_lossy(),
        ],
        None,
    )
    .context("reading the previous run's failures")?;
    let json = out
        .find('{')
        .map(|i| &out[i..])
        .ok_or_else(|| CliError::new("xcresulttool produced no JSON test tree"))?;
    let root: serde_json::Value =
        serde_json::from_str(json).map_err(|e| CliError::new(format!("parsing test tree: {e}")))?;
    let mut selectors = Vec::new();
    collect_failed(&root, &mut selectors);
    selectors.sort();
    selectors.dedup();
    Ok(selectors)
}

/// Recursive walk of the xcresulttool test tree (`testNodes`/`children`),
/// collecting failed test cases' identifiers. XCTest identifiers carry a
/// trailing `()` that `-only-testing:` doesn't accept — trimmed here.
fn collect_failed(node: &serde_json::Value, out: &mut Vec<String>) {
    for key in ["testNodes", "children"] {
        if let Some(nodes) = node.get(key).and_then(serde_json::Value::as_array) {
            for n in nodes {
                collect_failed(n, out);
            }
        }
    }
    let is_case = node.get("nodeType").and_then(serde_json::Value::as_str) == Some("Test Case");
    let failed = node
        .get("result")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|r| r.eq_ignore_ascii_case("failed"));
    if is_case
        && failed
        && let Some(id) = node
            .get("nodeIdentifier")
            .and_then(serde_json::Value::as_str)
    {
        out.push(id.trim_end_matches("()").to_string());
    }
}

/// The overall line-coverage fraction (0.0–1.0) from a coverage-enabled
/// `.xcresult`, via `xcrun xccov view --report --json`. `None` when coverage
/// wasn't collected or the report can't be read.
#[must_use]
pub fn coverage_percent(bundle: &Path) -> Option<f64> {
    let out = process::capture(
        "xcrun",
        &[
            "xccov",
            "view",
            "--report",
            "--json",
            &bundle.to_string_lossy(),
        ],
        None,
    )
    .ok()?;
    // Skip any leading non-JSON, like the sibling `xcresulttool` readers do —
    // a preamble line on stdout would otherwise silently read as "no coverage".
    let json = out.find('{').map(|i| &out[i..])?;
    let json: serde_json::Value = serde_json::from_str(json).ok()?;
    json.get("lineCoverage").and_then(serde_json::Value::as_f64)
}

/// Read a test summary from a `.xcresult` bundle via `xcresulttool` (Xcode 16+).
pub fn test_summary(bundle: &Path) -> Result<TestSummary, CliError> {
    let out = process::capture(
        "xcrun",
        &[
            "xcresulttool",
            "get",
            "test-results",
            "summary",
            "--path",
            &bundle.to_string_lossy(),
        ],
        None,
    )
    .context("reading the test results")?;
    parse_summary(&out)
}

/// Parse the `xcresulttool` summary JSON (skipping any leading non-JSON).
fn parse_summary(out: &str) -> Result<TestSummary, CliError> {
    let json = out
        .find('{')
        .map(|i| &out[i..])
        .ok_or_else(|| CliError::new("xcresulttool produced no JSON summary"))?;
    serde_json::from_str(json).map_err(|e| CliError::new(format!("parsing test summary: {e}")))
}

/// One file a test attached during its run, as exported from a `.xcresult`.
/// `file` is the staged export (named by UUID); `suggested_name` is what the
/// test called it, which is the only name a reader can use.
pub struct ExportedAttachment {
    /// The owning test, as `xcresulttool` identifies it (`Class/method()`).
    pub test: String,
    pub file: PathBuf,
    pub suggested_name: String,
    /// Recorded against a test failure rather than a passing step.
    pub failure: bool,
    /// Seconds since the epoch — the run order the test recorded them in,
    /// which the manifest's own order does not preserve.
    pub timestamp: f64,
}

/// The `manifest.json` `xcresulttool export attachments` writes beside the
/// exported files: one entry per test, mapping UUID filenames back to the
/// names the test gave them. Without this join the export is unusable.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachmentManifestEntry {
    test_identifier: String,
    #[serde(default)]
    attachments: Vec<ManifestAttachment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ManifestAttachment {
    exported_file_name: String,
    suggested_human_readable_name: String,
    is_associated_with_failure: bool,
    timestamp: f64,
}

impl Default for ManifestAttachment {
    fn default() -> Self {
        Self {
            exported_file_name: String::new(),
            suggested_human_readable_name: String::new(),
            is_associated_with_failure: false,
            timestamp: 0.0,
        }
    }
}

/// Export a `.xcresult`'s attachments into `staging` and read the manifest
/// back, flattened to one entry per file. `staging` must be empty: a second
/// export into a populated directory writes `name (1).png` duplicates rather
/// than replacing what is there.
pub fn export_attachments(
    bundle: &Path,
    staging: &Path,
    only_failures: bool,
) -> Result<Vec<ExportedAttachment>, CliError> {
    let mut owned = vec![
        "xcresulttool",
        "export",
        "attachments",
        "--path",
        &bundle.to_string_lossy(),
        "--output-path",
        &staging.to_string_lossy(),
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    if only_failures {
        owned.push("--only-failures".to_string());
    }
    let argv: Vec<&str> = owned.iter().map(String::as_str).collect();
    // The command's own stdout just narrates each file; the manifest is the
    // part worth reading.
    process::capture("xcrun", &argv, None).context("exporting the test attachments")?;

    let manifest = staging.join("manifest.json");
    let manifest_json = std::fs::read_to_string(&manifest).map_err(|e| {
        CliError::new(format!(
            "xcresulttool wrote no attachment manifest at {}: {e}",
            manifest.display()
        ))
    })?;
    let entries: Vec<AttachmentManifestEntry> = serde_json::from_str(&manifest_json)
        .map_err(|e| CliError::new(format!("parsing the attachment manifest: {e}")))?;

    Ok(entries
        .into_iter()
        .flat_map(|entry| {
            let test = entry.test_identifier;
            entry
                .attachments
                .into_iter()
                .map(move |a| ExportedAttachment {
                    test: test.clone(),
                    file: staging.join(&a.exported_file_name),
                    suggested_name: if a.suggested_human_readable_name.is_empty() {
                        a.exported_file_name
                    } else {
                        a.suggested_human_readable_name
                    },
                    failure: a.is_associated_with_failure,
                    timestamp: a.timestamp,
                })
        })
        .collect())
}

/// What the tests themselves wrote, recovered from a `.xcresult`'s diagnostics.
pub struct RunOutput {
    /// Per test, in the order the run executed them. Only tests that wrote
    /// something appear.
    pub tests: Vec<TestOutput>,
    /// Output written outside any test case — setup, teardown, and any
    /// framework whose markers this parser does not recognise. Kept rather
    /// than dropped, so nothing the run wrote goes missing without a word.
    pub unattributed: String,
    /// The files it was read from, for the part that doesn't fit in a payload.
    pub sources: Vec<PathBuf>,
    /// Whether every target reported running its tests serially. Attribution
    /// keys on `started`/`passed` markers bracketing a test's output, which
    /// interleaved parallel workers would scramble.
    pub serial: bool,
}

pub struct TestOutput {
    /// `Class/method`, the shape `--only-testing` and the attachment manifest
    /// both use (the marker's own `Module.Class` spelling is normalized here).
    pub test: String,
    pub output: String,
}

/// XCTest brackets each test's console output with these, on the test
/// process's own stdout: `Test Case '-[Module.Class method]' started.` … then
/// `passed`/`failed`. Everything between is what that test wrote.
fn parse_case_marker(line: &str) -> Option<(String, bool)> {
    let rest = line.strip_prefix("Test Case '-[")?;
    let (inner, tail) = rest.split_once("]' ")?;
    let (class, method) = inner.split_once(' ')?;
    let started = tail.starts_with("started");
    if !started && !tail.starts_with("passed") && !tail.starts_with("failed") {
        return None;
    }
    // The marker spells the class module-qualified; every other identifier in
    // the CLI (and in the result bundle) does not.
    let class = class.rsplit('.').next().unwrap_or(class);
    Some((format!("{class}/{method}"), started))
}

/// Split one test process's stdout into per-test slices.
fn split_output(text: &str, into: &mut Vec<TestOutput>, unattributed: &mut String) {
    let mut current: Option<(String, String)> = None;
    for line in text.lines() {
        if let Some((test, started)) = parse_case_marker(line) {
            if let Some((name, body)) = current.take()
                && !body.trim().is_empty()
            {
                into.push(TestOutput {
                    test: name,
                    output: body,
                });
            }
            if started {
                current = Some((test, String::new()));
            }
            continue;
        }
        // Suite banners are structure, not output; keeping them would bury
        // the handful of real lines in the unattributed bucket.
        if line.starts_with("Test Suite '") {
            continue;
        }
        let sink = match current.as_mut() {
            Some((_, body)) => body,
            None => &mut *unattributed,
        };
        sink.push_str(line);
        sink.push('\n');
    }
    if let Some((name, body)) = current
        && !body.trim().is_empty()
    {
        into.push(TestOutput {
            test: name,
            output: body,
        });
    }
}

/// Export a `.xcresult`'s diagnostics into `staging` and read back what the
/// tests printed. The bundle keeps this per test *process*, not per test, so
/// it is sliced here on XCTest's own case markers.
///
/// Only the test processes' streams are read: the same export also holds the
/// app-under-test's `os_log` firehose (`StandardOutputAndStandardError-<bundle
/// id>.txt`), which is a different thing and can run to tens of megabytes.
/// Those streams are copied into `keep` — a report that truncates has to name
/// something that outlives the scratch directory the export landed in.
pub fn export_run_output(
    bundle: &Path,
    staging: &Path,
    keep: &Path,
) -> Result<RunOutput, CliError> {
    process::capture(
        "xcrun",
        &[
            "xcresulttool",
            "export",
            "diagnostics",
            "--path",
            &bundle.to_string_lossy(),
            "--output-path",
            &staging.to_string_lossy(),
        ],
        None,
    )
    .context("exporting the test diagnostics")?;

    let mut streams = Vec::new();
    let mut schedules = Vec::new();
    collect_diagnostic_files(staging, &mut streams, &mut schedules);
    streams.sort();

    let _ = std::fs::remove_dir_all(keep);
    let _ = std::fs::create_dir_all(keep);
    let mut tests = Vec::new();
    let mut unattributed = String::new();
    let mut sources = Vec::new();
    for path in &streams {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        split_output(&text, &mut tests, &mut unattributed);
        let label = stream_label(path, sources.len());
        let mut kept = keep.join(format!("{label}.txt"));
        // Two targets resolving to one label would silently cost a stream.
        if sources.contains(&kept) {
            kept = keep.join(format!("{label}-{}.txt", sources.len()));
        }
        if std::fs::write(&kept, &text).is_ok() {
            sources.push(kept);
        }
    }
    // Asserted only from the line that says so: an absent or differently
    // worded log leaves this unclaimed rather than guessed at.
    let serial = !schedules.is_empty()
        && schedules.iter().all(|p| {
            std::fs::read_to_string(p).is_ok_and(|s| s.contains("Parallelization disabled"))
        });

    Ok(RunOutput {
        tests,
        unattributed,
        sources,
        serial,
    })
}

/// A name for one test process's kept stream. Every stream is called
/// `StandardOutputAndStandardError.txt`; what tells two apart is the directory
/// above, which leads with the test target (`ReflowTests-<UUID>-…`). The file's
/// own name is skipped for exactly that reason — matching it would give every
/// target the same label, and one stream would overwrite the other.
fn stream_label(path: &Path, index: usize) -> String {
    path.parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .filter_map(|a| a.file_name().and_then(|n| n.to_str()))
        .find_map(|name| {
            let (target, _) = name.split_once('-')?;
            (!target.is_empty()).then(|| target.to_string())
        })
        .unwrap_or_else(|| format!("target-{index}"))
}

/// Walk the diagnostics export for the two files worth reading: each test
/// process's own stdout, and the scheduling log that says whether the run was
/// serial. Directory names in the export carry spaces and UUIDs, so it is
/// walked rather than globbed.
fn collect_diagnostic_files(dir: &Path, streams: &mut Vec<PathBuf>, schedules: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_diagnostic_files(&path, streams, schedules);
        } else {
            match path.file_name().and_then(|n| n.to_str()) {
                // Exactly this name is the test process; a `-<bundle id>`
                // suffix is some other process the run happened to capture.
                Some("StandardOutputAndStandardError.txt") => streams.push(path),
                Some("scheduling.log") => schedules.push(path),
                _ => {}
            }
        }
    }
}

/// One target's resolved build settings, the shape [`app_bundle`] reads to
/// locate the built product. Field names mirror `xcodebuild -showBuildSettings
/// -json` so the values can be deserialized straight from that format in tests.
#[derive(Debug, Deserialize)]
pub struct TargetBuildSettings {
    pub target: String,
    #[serde(rename = "buildSettings")]
    pub settings: BTreeMap<String, String>,
}

/// The launchable app produced by a build: the `.app` path, its bundle id, and
/// the executable inside it (used to launch macOS apps directly).
#[derive(Debug, Clone)]
pub struct AppBundle {
    pub path: PathBuf,
    pub bundle_id: String,
    /// `TARGET_BUILD_DIR/EXECUTABLE_PATH` — the binary to run for a macOS app.
    pub executable: PathBuf,
}

/// Pick the launchable app's *target* from resolved settings. Candidates are
/// targets that build a `.app` wrapper and declare a bundle id; among them,
/// one whose `SUPPORTED_PLATFORMS` covers the destination's platform wins —
/// in an iOS + watchOS scheme the watch companion builds *first* (dependency
/// order), and blind first-pick would install the watch app onto the iPhone
/// simulator. Targets that don't state their platforms (or an unmappable/
/// absent destination) fall back to first-candidate order — and when the
/// filter rejects *every* candidate (Mac Catalyst declaring `iphoneos` under
/// a `platform=macOS` destination), the first `.app` still wins over a
/// nothing-to-launch error.
pub fn app_target<'a>(
    settings: &'a [TargetBuildSettings],
    destination: Option<&str>,
) -> Result<&'a TargetBuildSettings, CliError> {
    let wanted = destination.and_then(destination_sdk_token);
    let mut fallback: Option<&TargetBuildSettings> = None;
    let mut first_app: Option<&TargetBuildSettings> = None;
    for t in settings {
        if bundle_of(t).is_none() {
            continue;
        }
        if first_app.is_none() {
            first_app = Some(t);
        }
        let supported = t.settings.get("SUPPORTED_PLATFORMS");
        match (wanted, supported) {
            // The target states its platforms and covers the destination —
            // a definitive pick.
            (Some(tok), Some(platforms)) if platforms.split_whitespace().any(|p| p == tok) => {
                return Ok(t);
            }
            // States its platforms and the destination is not among them —
            // not this app (the watch-companion case).
            (Some(_), Some(_)) => {}
            // No filter requested, or the target doesn't say — candidate in
            // declaration order.
            _ => {
                if fallback.is_none() {
                    fallback = Some(t);
                }
            }
        }
    }
    fallback.or(first_app).ok_or_else(|| {
        CliError::new("could not find a launchable .app in the resolved build settings")
    })
}

/// The launchable bundle a target's resolved settings describe, if it builds
/// a `.app` wrapper with a bundle id — the candidacy test behind
/// [`app_target`].
fn bundle_of(t: &TargetBuildSettings) -> Option<AppBundle> {
    let wrapper = t
        .settings
        .get("WRAPPER_NAME")
        .or_else(|| t.settings.get("FULL_PRODUCT_NAME"))?;
    let build_dir = t.settings.get("TARGET_BUILD_DIR")?;
    let bundle_id = t.settings.get("PRODUCT_BUNDLE_IDENTIFIER")?;
    if !Path::new(wrapper)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("app"))
    {
        return None;
    }
    let build_dir = Path::new(build_dir);
    let executable = t
        .settings
        .get("EXECUTABLE_PATH")
        .map_or_else(|| build_dir.join(wrapper), |rel| build_dir.join(rel));
    Some(AppBundle {
        path: build_dir.join(wrapper),
        bundle_id: bundle_id.clone(),
        executable,
    })
}

/// The `-derivedDataPath` a passthrough hands `xcodebuild`, if any — the
/// product locator has to look where the build actually put the bundle.
/// Product-relocating build settings the locator can't model (`SYMROOT=`,
/// `OBJROOT=`, `CONFIGURATION_BUILD_DIR=`) are refused loudly: looking in the
/// default DerivedData would name whatever stale `.app` an earlier plain build
/// left there.
fn passthrough_derived_data(passthrough: &[String]) -> Result<Option<PathBuf>, CliError> {
    let mut derived_data = None;
    let mut iter = passthrough.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == "-derivedDataPath" {
            derived_data = iter.peek().map(PathBuf::from);
        } else if let Some((key, _)) = arg.split_once('=')
            && matches!(key, "SYMROOT" | "OBJROOT" | "CONFIGURATION_BUILD_DIR")
        {
            return Err(CliError::new(format!(
                "`-- {key}=…` relocates the built product where the app locator can't \
                 follow; use `-- -derivedDataPath <dir>` instead"
            )));
        }
    }
    Ok(derived_data)
}

/// Resolve every target's build settings for a plan through the in-process
/// resolver (the engine behind `settings show`), with no `xcodebuild` spawn —
/// including a passthrough `-derivedDataPath`. Feed the result to
/// [`app_bundle`] to name the product a build of this plan writes. Swift
/// packages build no `.app`, so they have nothing to resolve here.
///
/// `app`'s `RunPlan` resolves the same way for its install/launch path; both
/// locators must agree on the products dir or the CLI reports one bundle and
/// installs another.
pub fn resolved_settings(plan: &BuildPlan<'_>) -> Result<Vec<TargetBuildSettings>, CliError> {
    let (project, workspace) = match plan.container {
        Container::Project(p) => (Some(p.clone()), None),
        Container::Workspace(p) => (None, Some(p.clone())),
        Container::SwiftPackage(_) => {
            return Err(CliError::new("Swift packages have no .app bundle"));
        }
    };
    let opts = BuildSettingsOptions {
        project,
        workspace,
        scheme: Some(plan.scheme.to_string()),
        target: None,
        configuration: plan.configuration.to_string(),
        // Must match the build's own -sdk (if any), or TARGET_BUILD_DIR points
        // at a different products dir than the one just built.
        sdk: plan.sdk.unwrap_or_default().to_string(),
        arch: String::new(),
        destination: plan
            .destination
            .and_then(sweetpad_lib::destination::parse_destination_arg),
        xcconfig: None,
        xcode: None,
        xcspec_root: None,
        sdksettings_root: None,
        catalog_cache: None,
        derived_data_path: passthrough_derived_data(plan.passthrough)?,
        // Callers install, launch, and report what this resolves, so it has to
        // name the bundle `xcodebuild` actually wrote — including when the user
        // has moved Derived Data in Xcode (issue #306).
        read_xcode_locations: true,
        keys: None,
    };
    let resolved = resolve_build_settings(&opts).map_err(CliError::new)?;
    Ok(resolved
        .into_iter()
        .map(|t| TargetBuildSettings {
            target: t.target,
            settings: t.settings,
        })
        .collect())
}

/// Pick the launchable app from resolved settings — [`app_target`]'s bundle.
pub fn app_bundle(
    settings: &[TargetBuildSettings],
    destination: Option<&str>,
) -> Result<AppBundle, CliError> {
    let target = app_target(settings, destination)?;
    bundle_of(target).ok_or_else(|| {
        CliError::new("could not find a launchable .app in the resolved build settings")
    })
}

/// The SDK token a `-destination platform=…` implies, as spelled in
/// `SUPPORTED_PLATFORMS` (e.g. `iOS Simulator` → `iphonesimulator`).
fn destination_sdk_token(spec: &str) -> Option<&'static str> {
    let platform = spec
        .split(',')
        .find_map(|kv| kv.trim().strip_prefix("platform="))?;
    Some(match platform {
        "iOS Simulator" => "iphonesimulator",
        "iOS" => "iphoneos",
        "macOS" => "macosx",
        "watchOS Simulator" => "watchsimulator",
        "watchOS" => "watchos",
        "tvOS Simulator" => "appletvsimulator",
        "tvOS" => "appletvos",
        "visionOS Simulator" => "xrsimulator",
        "visionOS" => "xros",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::resolve::Container;
    use std::path::PathBuf;

    fn project() -> Container {
        Container::Project(PathBuf::from("/work/App.xcodeproj"))
    }

    /// A real excerpt: the markers, an `os_log` line the app emitted onto the
    /// same stream, and one test's own `print` between its own markers.
    const STREAM: &str = "\
2026-08-09 16:22:38.996503+0200 Reflow[48456:7376915] [General] Failed to send CA Event
Test Suite 'All tests' started at 2026-08-09 16:22:39.033.
Test Case '-[ReflowTests.ReflowEngineTests testFast]' started.
Test Case '-[ReflowTests.ReflowEngineTests testFast]' passed (0.004 seconds).
Test Case '-[ReflowTests.ReflowEngineTests testBook]' started.
BOOK REFLOW: 300 source pages -> 486 pages in 26.9s
Test Case '-[ReflowTests.ReflowEngineTests testBook]' passed (26.988 seconds).
Test Case '-[ReflowTests.ReflowEngineTests testBroken]' started.
about to fail
Test Case '-[ReflowTests.ReflowEngineTests testBroken]' failed (0.1 seconds).
Test Suite 'All tests' passed at 2026-08-09 16:24:00.
";

    #[test]
    fn a_tests_own_output_is_recovered_from_the_stream() {
        let (mut tests, mut rest) = (Vec::new(), String::new());
        split_output(STREAM, &mut tests, &mut rest);

        // Only tests that wrote something appear, under the identifier shape
        // the rest of the CLI uses — not the marker's `Module.Class`.
        let names: Vec<&str> = tests.iter().map(|t| t.test.as_str()).collect();
        assert_eq!(
            names,
            ["ReflowEngineTests/testBook", "ReflowEngineTests/testBroken"]
        );
        assert_eq!(
            tests[0].output.trim(),
            "BOOK REFLOW: 300 source pages -> 486 pages in 26.9s"
        );
        // A failing test's output is bracketed by `failed`, not `passed`.
        assert_eq!(tests[1].output.trim(), "about to fail");

        // Output written outside any case is kept, and suite banners are not
        // mistaken for it.
        assert!(rest.contains("Failed to send CA Event"), "{rest}");
        assert!(!rest.contains("Test Suite"), "{rest}");
        assert!(!rest.contains("BOOK REFLOW"), "{rest}");
    }

    #[test]
    fn an_unrecognized_framework_loses_nothing() {
        // Swift Testing's markers are not XCTest's. Nothing is attributed,
        // and the run's output survives whole rather than vanishing.
        let text = "◇ Test example() started.\nmeasured 42ms\n✔ Test example() passed.\n";
        let (mut tests, mut rest) = (Vec::new(), String::new());
        split_output(text, &mut tests, &mut rest);
        assert!(tests.is_empty());
        assert!(rest.contains("measured 42ms"), "{rest}");
    }

    #[test]
    fn each_test_targets_stream_keeps_its_own_name() {
        // Every stream is named StandardOutputAndStandardError.txt; only the
        // directory above distinguishes them, so a label taken from the file
        // would collide and one target's output would overwrite another's.
        let a = Path::new(
            "/x/0_Test_iPhone 17_Diagnostics/ReflowTests-9C70-Configuration-Test Scheme \
             Action-Iteration-1/ReflowTests-859C/StandardOutputAndStandardError.txt",
        );
        let b = Path::new(
            "/x/0_Test_iPhone 17_Diagnostics/ReflowUITests-82D4-Configuration-Test Scheme \
             Action-Iteration-1/ReflowUITests-1E01/StandardOutputAndStandardError.txt",
        );
        assert_eq!(stream_label(a, 0), "ReflowTests");
        assert_eq!(stream_label(b, 1), "ReflowUITests");
        assert_ne!(stream_label(a, 0), stream_label(b, 1));
        // A path with nothing to read a target from still names something.
        assert_eq!(
            stream_label(Path::new("/StandardOutputAndStandardError.txt"), 3),
            "target-3"
        );
    }

    fn diag(severity: &str, location: Option<&str>, message: &str) -> serde_json::Value {
        serde_json::json!({
            "event": "diagnostic",
            "severity": severity,
            "location": location,
            "message": message,
        })
    }

    #[test]
    fn a_blocked_build_parks_no_transcript() {
        // A blocked build's headline is the blocker, and it drops this detail
        // wholesale — so parking a log here leaves a file on disk that nothing
        // ever names. A blocker with warnings alongside it is the case that
        // exercises it: `diagnostics_summary` answers on any severity, so the
        // parking arm is the one a blocked build otherwise lands in.
        let container = Container::Project(PathBuf::from("/work/Blocked.xcodeproj"));
        let log = project_artifact(&container, "-build.log");
        let _ = std::fs::remove_file(&log);
        let diagnostics = vec![diag("warning", Some("A.swift:1:1"), "unused variable 'x'")];

        let detail = captured_failure_detail(
            &container,
            "the whole transcript",
            "the tail",
            &diagnostics,
            /* blocked */ true,
        );
        assert!(detail.is_empty(), "{detail}");
        assert!(!log.exists(), "a blocked build parked {}", log.display());

        // The same failure, not blocked: the transcript is parked and named.
        let detail = captured_failure_detail(
            &container,
            "the whole transcript",
            "the tail",
            &diagnostics,
            /* blocked */ false,
        );
        assert!(detail.contains("unused variable 'x'"), "{detail}");
        assert!(detail.contains("full log: "), "{detail}");
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "the whole transcript"
        );
        let _ = std::fs::remove_file(&log);

        // Nothing parsed and not blocked: the tail is the only account there is.
        let detail = captured_failure_detail(&container, "transcript", "the tail", &[], false);
        assert_eq!(detail, ":\nthe tail");
        assert!(!log.exists(), "nothing to summarize parked a log anyway");
    }

    #[test]
    fn the_summary_leads_with_the_first_error_not_the_first_diagnostic() {
        // A failed build usually emits warnings before the error that stopped
        // it; leading with the warning would name the wrong cause.
        let diagnostics = vec![
            diag("warning", Some("A.swift:1:1"), "unused variable 'x'"),
            diag("error", Some("B.swift:9:3"), "cannot find 'foo' in scope"),
            diag("error", None, "Build input file cannot be found"),
        ];
        assert_eq!(
            diagnostics_summary(&diagnostics).as_deref(),
            Some("B.swift:9:3: cannot find 'foo' in scope (and 1 more)")
        );
    }

    #[test]
    fn a_lone_error_gets_no_count_and_a_locationless_one_no_prefix() {
        let one = vec![diag("error", None, "Build input file cannot be found")];
        assert_eq!(
            diagnostics_summary(&one).as_deref(),
            Some("Build input file cannot be found")
        );
    }

    #[test]
    fn warnings_alone_still_summarize_and_nothing_parsed_is_none() {
        // xcodebuild can fail with no error diagnostic at all (a bad
        // destination, a signing refusal) — then there is nothing to lead with
        // and the caller keeps the transcript tail instead.
        let warn = vec![diag("warning", Some("A.swift:1:1"), "unused variable 'x'")];
        assert_eq!(
            diagnostics_summary(&warn).as_deref(),
            Some("A.swift:1:1: unused variable 'x'")
        );
        assert_eq!(diagnostics_summary(&[]), None);
    }

    /// Build `TargetBuildSettings` from a `-showBuildSettings -json` payload,
    /// skipping any preamble — a convenience for the `app_bundle` tests.
    fn parse_settings(stdout: &str) -> Vec<TargetBuildSettings> {
        let json = &stdout[stdout.find('[').expect("no JSON array")..];
        serde_json::from_str(json).expect("invalid build settings JSON")
    }

    #[test]
    fn build_args_for_project() {
        let c = project();
        let plan = BuildPlan {
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=iOS Simulator,id=UDID"),
            passthrough: &[],
            sdk: None,
            clean: true,
            hot: false,
            hot_entitlements: None,
        };
        assert_eq!(
            plan.args(),
            vec![
                "clean",
                "build",
                "-scheme",
                "App",
                "-configuration",
                "Debug",
                "-destination",
                "platform=iOS Simulator,id=UDID",
                "-project",
                "/work/App.xcodeproj",
            ]
        );
    }

    #[test]
    fn hot_build_appends_interposable_and_frontend_settings() {
        let c = project();
        let plan = BuildPlan {
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=iOS Simulator,id=UDID"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: true,
            hot_entitlements: None,
        };
        let args = plan.args();
        assert!(args.contains(&"OTHER_LDFLAGS=$(inherited) -Xlinker -interposable".to_string()));
        assert!(args.contains(&"EMIT_FRONTEND_COMMAND_LINES=YES".to_string()));
        // The injectability settings are macOS-only: a simulator app needs
        // neither (the sim enforces no hardened runtime / sandbox on dlopen).
        assert!(
            !args
                .iter()
                .any(|a| a.starts_with("ENABLE_HARDENED_RUNTIME"))
        );
        assert!(!args.iter().any(|a| a.starts_with("ENABLE_APP_SANDBOX")));
    }

    #[test]
    fn hot_mac_build_disables_hardened_runtime_and_sandbox() {
        let c = project();
        let plan = BuildPlan {
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=macOS"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: true,
            hot_entitlements: None,
        };
        let args = plan.args();
        assert!(args.contains(&"ENABLE_HARDENED_RUNTIME=NO".to_string()));
        assert!(args.contains(&"ENABLE_APP_SANDBOX=NO".to_string()));
        // A non-hot mac build keeps the project's own protections.
        let cold = BuildPlan {
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=macOS"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: false,
            hot_entitlements: None,
        };
        assert!(!cold.args().iter().any(|a| {
            a.starts_with("ENABLE_HARDENED_RUNTIME") || a.starts_with("ENABLE_APP_SANDBOX")
        }));
    }

    #[test]
    fn hot_mac_build_signs_with_the_stripped_entitlements() {
        let c = project();
        let stripped = Path::new("/cache/hot/Debug-nosandbox.entitlements");
        let plan = BuildPlan {
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=macOS"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: true,
            hot_entitlements: Some(stripped),
        };
        let args = plan.args();
        let expected = "CODE_SIGN_ENTITLEMENTS=/cache/hot/Debug-nosandbox.entitlements";
        // The override rides right after the sandbox settings it completes.
        let sandbox_at = args.iter().position(|a| a == "ENABLE_APP_SANDBOX=NO");
        let override_at = args.iter().position(|a| a == expected);
        assert!(sandbox_at.is_some() && override_at > sandbox_at, "{args:?}");

        // Simulator hot builds never sign with it — the strip is a macOS
        // concern (and the caller never sets it for simulators anyway).
        let sim = BuildPlan {
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=iOS Simulator,id=UDID"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: true,
            hot_entitlements: Some(stripped),
        };
        assert!(
            !sim.args()
                .iter()
                .any(|a| a.starts_with("CODE_SIGN_ENTITLEMENTS"))
        );
    }

    #[test]
    fn build_args_workspace_omits_clean_and_destination() {
        let c = Container::Workspace(PathBuf::from("/work/App.xcworkspace"));
        let plan = BuildPlan {
            container: &c,
            scheme: "App",
            configuration: "Release",
            destination: None,
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: false,
            hot_entitlements: None,
        };
        assert_eq!(
            plan.args(),
            vec![
                "build",
                "-scheme",
                "App",
                "-configuration",
                "Release",
                "-workspace",
                "/work/App.xcworkspace"
            ]
        );
    }

    #[test]
    fn test_args_include_selectors_and_bundle() {
        let c = project();
        let bundle = PathBuf::from("/tmp/r.xcresult");
        let only = vec!["AppTests/LoginTests".to_string()];
        let skip = vec!["AppTests/FlakyTests/testJitter".to_string()];
        let plan = TestPlan {
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=iOS Simulator,id=UDID"),
            only_testing: &only,
            skip_testing: &skip,
            result_bundle: &bundle,
            sdk: None,
            retry_flaky: None,
            coverage: false,
            passthrough: &[],
        };
        assert_eq!(
            plan.args(),
            vec![
                "test",
                "-scheme",
                "App",
                "-configuration",
                "Debug",
                "-resultBundlePath",
                "/tmp/r.xcresult",
                "-destination",
                "platform=iOS Simulator,id=UDID",
                "-project",
                "/work/App.xcodeproj",
                "-only-testing:AppTests/LoginTests",
                "-skip-testing:AppTests/FlakyTests/testJitter",
            ]
        );
    }

    #[test]
    fn parses_settings_skipping_preamble() {
        let stdout =
            "warning: blah\n[{\"target\":\"App\",\"buildSettings\":{\"PRODUCT_NAME\":\"App\"}}]";
        let parsed = parse_settings(stdout);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].target, "App");
        assert_eq!(parsed[0].settings.get("PRODUCT_NAME").unwrap(), "App");
    }

    #[test]
    fn app_bundle_picks_the_app_target() {
        let stdout = r#"[
          {"target":"AppTests","buildSettings":{"TARGET_BUILD_DIR":"/d","WRAPPER_NAME":"AppTests.xctest","PRODUCT_BUNDLE_IDENTIFIER":"com.x.tests"}},
          {"target":"App","buildSettings":{"TARGET_BUILD_DIR":"/d","WRAPPER_NAME":"App.app","PRODUCT_BUNDLE_IDENTIFIER":"com.x.app"}}
        ]"#;
        let settings = parse_settings(stdout);
        let app = app_bundle(&settings, None).unwrap();
        assert_eq!(app.path, PathBuf::from("/d/App.app"));
        assert_eq!(app.bundle_id, "com.x.app");
    }

    #[test]
    fn app_bundle_resolves_macos_executable() {
        let stdout = r#"[{"target":"App","buildSettings":{
            "TARGET_BUILD_DIR":"/d","WRAPPER_NAME":"App.app",
            "EXECUTABLE_PATH":"App.app/Contents/MacOS/App","PRODUCT_BUNDLE_IDENTIFIER":"com.x.app"}}]"#;
        let settings = parse_settings(stdout);
        let app = app_bundle(&settings, None).unwrap();
        assert_eq!(
            app.executable,
            PathBuf::from("/d/App.app/Contents/MacOS/App")
        );
    }

    #[test]
    fn working_dir_is_none_for_relative_container() {
        // A relative project path must not produce an empty cwd (which would
        // make the spawn fail and look like a missing xcodebuild).
        assert_eq!(
            working_dir(&Container::Project(PathBuf::from("App.xcodeproj"))),
            None
        );
        assert_eq!(
            working_dir(&Container::Project(PathBuf::from("/work/App.xcodeproj"))),
            Some(PathBuf::from("/work"))
        );
    }

    #[test]
    fn app_bundle_errors_without_app() {
        let settings = parse_settings(
            r#"[{"target":"Lib","buildSettings":{"TARGET_BUILD_DIR":"/d","WRAPPER_NAME":"Lib.framework","PRODUCT_BUNDLE_IDENTIFIER":"com.x.lib"}}]"#,
        );
        assert!(app_bundle(&settings, None).is_err());
    }

    #[test]
    fn app_bundle_prefers_the_destination_platform() {
        // Dependency order builds the watch companion first; the destination
        // platform must pick the iOS app anyway.
        let stdout = r#"[
          {"target":"WatchApp","buildSettings":{"TARGET_BUILD_DIR":"/w","WRAPPER_NAME":"Watch App.app","PRODUCT_BUNDLE_IDENTIFIER":"com.x.watch","SUPPORTED_PLATFORMS":"watchos watchsimulator"}},
          {"target":"App","buildSettings":{"TARGET_BUILD_DIR":"/d","WRAPPER_NAME":"App.app","PRODUCT_BUNDLE_IDENTIFIER":"com.x.app","SUPPORTED_PLATFORMS":"iphoneos iphonesimulator"}}
        ]"#;
        let settings = parse_settings(stdout);
        let ios = app_bundle(&settings, Some("platform=iOS Simulator,id=U")).unwrap();
        assert_eq!(ios.bundle_id, "com.x.app");
        let watch = app_bundle(&settings, Some("platform=watchOS Simulator,id=U")).unwrap();
        assert_eq!(watch.bundle_id, "com.x.watch");
        // No destination (or targets without SUPPORTED_PLATFORMS) keeps the
        // declaration-order pick.
        let first = app_bundle(&settings, None).unwrap();
        assert_eq!(first.bundle_id, "com.x.watch");
    }

    #[test]
    fn artifact_hash_is_stable_fnv1a() {
        // Pinned FNV-1a test vectors: the slot names must never change across
        // toolchains (that would orphan every retained artifact).
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn parses_test_summary() {
        let out = "Some log line\n{\"result\":\"Failed\",\"totalTestCount\":5,\"passedTests\":4,\"failedTests\":1,\"skippedTests\":0,\"testFailures\":[{\"testName\":\"testX\",\"targetName\":\"AppTests\",\"failureText\":\"boom\"}]}";
        let s = parse_summary(out).unwrap();
        assert_eq!(
            (s.total_test_count, s.passed_tests, s.failed_tests),
            (5, 4, 1)
        );
        assert_eq!(s.test_failures[0].test_name, "testX");
    }
}
