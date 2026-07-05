//! Thin wrapper over `xcodebuild` for the build/run commands: assembling the
//! argument vector (mirroring the VS Code extension's proven invocation) and
//! reading back the build settings needed to locate and launch the built app.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

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
    /// recover per-file commands). Only set for simulator builds under `--hot`.
    pub hot: bool,
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
        // Only the raw `-v` human passthrough leaves output unparsed; every
        // parsing mode (including ndjson under `-v`) records the artifact.
        let mut parsed = true;
        let ok = if out.is_ndjson() {
            let (ok, s) = buildlog::run_ndjson("xcodebuild", &args, cwd.as_deref(), out)?;
            diagnostics.clone_from(&s.diagnostics);
            stats = Some(s);
            ok
        } else if out.is_json() {
            let run = process::run_captured("xcodebuild", &args, cwd.as_deref())?;
            diagnostics = buildlog::diagnostics_from_transcript(&run.combined);
            if !run.success {
                failure_detail = format!(":\n{}", run.tail);
            }
            run.success
        } else if out.is_verbose() {
            // Raw passthrough is unparsed — no artifact for this mode.
            parsed = false;
            process::run("xcodebuild", &args, cwd.as_deref(), false)?
        } else {
            let (ok, d) =
                buildlog::run_collecting("xcodebuild", &args, cwd.as_deref(), out, "Building")?;
            diagnostics = d;
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
            Err(
                CliError::new(format!(
                    "xcodebuild exited with a non-zero status{failure_detail}"
                ))
                .kind(ErrorKind::BuildFailure)
                .context("building the project"),
            )
        }
    }
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
            let (ok, _stats) = buildlog::run_ndjson("xcodebuild", &args, cwd.as_deref(), out)?;
            TestRunOutcome {
                passed: ok,
                tail: None,
            }
        } else if out.is_json() {
            let run = process::run_captured("xcodebuild", &args, cwd.as_deref())?;
            TestRunOutcome {
                passed: run.success,
                tail: (!run.success).then_some(run.tail),
            }
        } else if out.is_verbose() {
            let ok = process::run("xcodebuild", &args, cwd.as_deref(), false)
                .context("running the tests")?;
            TestRunOutcome {
                passed: ok,
                tail: None,
            }
        } else {
            let ok = buildlog::run("xcodebuild", &args, cwd.as_deref(), out, "Testing")
                .context("running the tests")?;
            TestRunOutcome {
                passed: ok,
                tail: None,
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
    let stem = container
        .path()
        .file_stem().map_or_else(|| "project".to_string(), |s| s.to_string_lossy().into_owned());
    let name = format!("{stem}-{:016x}{suffix}", fnv1a64(container.key().as_bytes()));
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
pub(crate) fn record_build_diagnostics(container: &Container, ok: bool, diagnostics: &[serde_json::Value]) {
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
fn shell_quote(arg: &str) -> String {
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
        && let Some(id) = node.get("nodeIdentifier").and_then(serde_json::Value::as_str)
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
#[derive(Debug)]
pub struct AppBundle {
    pub path: PathBuf,
    pub bundle_id: String,
    /// `TARGET_BUILD_DIR/EXECUTABLE_PATH` — the binary to run for a macOS app.
    pub executable: PathBuf,
}

/// Pick the launchable app from resolved settings. Candidates are targets
/// that build a `.app` wrapper and declare a bundle id; among them, one whose
/// `SUPPORTED_PLATFORMS` covers the destination's platform wins — in an
/// iOS + watchOS scheme the watch companion builds *first* (dependency
/// order), and blind first-pick would install the watch app onto the iPhone
/// simulator. Targets that don't state their platforms (or an unmappable/
/// absent destination) fall back to first-candidate order.
pub fn app_bundle(
    settings: &[TargetBuildSettings],
    destination: Option<&str>,
) -> Result<AppBundle, CliError> {
    let wanted = destination.and_then(destination_sdk_token);
    let mut fallback: Option<AppBundle> = None;
    for t in settings {
        let wrapper = t
            .settings
            .get("WRAPPER_NAME")
            .or_else(|| t.settings.get("FULL_PRODUCT_NAME"));
        let (Some(build_dir), Some(wrapper), Some(bundle_id)) = (
            t.settings.get("TARGET_BUILD_DIR"),
            wrapper,
            t.settings.get("PRODUCT_BUNDLE_IDENTIFIER"),
        ) else {
            continue;
        };
        if !Path::new(wrapper)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("app"))
        {
            continue;
        }
        let build_dir = Path::new(build_dir);
        let executable = t
            .settings
            .get("EXECUTABLE_PATH")
            .map_or_else(|| build_dir.join(wrapper), |rel| build_dir.join(rel));
        let bundle = AppBundle {
            path: build_dir.join(wrapper),
            bundle_id: bundle_id.clone(),
            executable,
        };
        let supported = t.settings.get("SUPPORTED_PLATFORMS");
        match (wanted, supported) {
            // The target states its platforms and covers the destination —
            // a definitive pick.
            (Some(tok), Some(platforms)) if platforms.split_whitespace().any(|p| p == tok) => {
                return Ok(bundle);
            }
            // States its platforms and the destination is not among them —
            // not this app (the watch-companion case).
            (Some(_), Some(_)) => {}
            // No filter requested, or the target doesn't say — candidate in
            // declaration order.
            _ => {
                if fallback.is_none() {
                    fallback = Some(bundle);
                }
            }
        }
    }
    fallback.ok_or_else(|| {
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
        };
        let args = plan.args();
        assert!(args.contains(&"OTHER_LDFLAGS=$(inherited) -Xlinker -interposable".to_string()));
        assert!(args.contains(&"EMIT_FRONTEND_COMMAND_LINES=YES".to_string()));
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
