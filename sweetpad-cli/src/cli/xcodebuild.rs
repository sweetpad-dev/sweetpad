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

/// The `xcodebuild` action a [`BuildPlan`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildAction {
    /// `build`: the targets the scheme builds for running.
    Build,
    /// `build-for-testing`: the targets the scheme tests, plus whatever they
    /// depend on, compiled without running a test.
    BuildForTesting,
}

impl BuildAction {
    fn as_arg(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::BuildForTesting => "build-for-testing",
        }
    }
}

/// Everything needed to invoke `xcodebuild build` (or `build-for-testing`) for
/// a resolved target.
pub struct BuildPlan<'a> {
    pub action: BuildAction,
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
    /// Where the run's `.xcresult` goes. Nothing here reads it: it is passed
    /// because `xcodebuild` writes the build's `.xcactivitylog` only when
    /// `-resultBundlePath` is given, and that log is the sole input
    /// `xcode-build-server` has for a file's compiler arguments. A build without
    /// it leaves the editor's index stale, so autocomplete degrades to "cannot
    /// find <Foundation/Foundation.h>" while the build itself succeeds.
    pub result_bundle: Option<PathBuf>,
    /// Extra arguments passed through to xcodebuild verbatim (everything after
    /// `--` on the command line) — the escape hatch for flags/settings the CLI
    /// doesn't model.
    pub passthrough: &'a [String],
}

impl BuildPlan<'_> {
    /// The `xcodebuild` argument vector: `[clean] build|build-for-testing
    /// -scheme … -configuration … [-destination …] [-sdk …]
    /// [-workspace|-project …]`.
    fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        if self.clean {
            args.push("clean".into());
        }
        args.push(self.action.as_arg().into());
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
        if let Some(bundle) = &self.result_bundle {
            args.push("-resultBundlePath".into());
            args.push(bundle.display().to_string());
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

    /// Clear the result-bundle slot: `xcodebuild` refuses to write into a path
    /// that already exists, and the slot is reused across builds. Call this
    /// immediately before spawning — never on the `--show-command` path, which
    /// must not touch state.
    pub fn prepare_result_bundle(&self) {
        let Some(bundle) = &self.result_bundle else {
            return;
        };
        let _ = std::fs::remove_dir_all(bundle);
        if let Some(parent) = bundle.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    /// Run the build. Human mode beautifies xcodebuild's output via
    /// [`buildlog`]; `-v` passes it through raw; `--json` captures both child
    /// streams (nothing interleaves with the envelope) and folds the tail of
    /// the transcript into the error on failure; `-o ndjson` streams one event
    /// per line. Every mode but `-v` records the parsed diagnostics as the
    /// project's last-build artifact for `build diagnostics`, and returns their
    /// [`BuildStats`](buildlog::BuildStats) for the terminal result.
    pub fn run(&self, out: &Output) -> Result<Option<buildlog::BuildStats>, CliError> {
        self.prepare_result_bundle();
        let parts = self.args();
        let args: Vec<&str> = parts.iter().map(String::as_str).collect();
        let cwd = working_dir(self.container);
        let start = std::time::Instant::now();
        let mut failure_detail = String::new();
        let mut diagnostics = Vec::new();
        let mut blocker = None;
        // Only the raw `-v` human passthrough leaves output unparsed; every
        // parsing mode (including ndjson under `-v`) records the artifact.
        let mut parsed = true;
        let mut streamed = false;
        let ok = if out.is_ndjson() {
            let (ok, d, b) = buildlog::run_ndjson("xcodebuild", &args, cwd.as_deref(), out)?;
            diagnostics = d;
            blocker = b;
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
            let (ok, d, b) = buildlog::run_collecting(
                "xcodebuild",
                &args,
                cwd.as_deref(),
                out,
                "Building",
                buildlog::ResultKind::BuildFailed,
            )?;
            diagnostics = d;
            blocker = b;
            streamed = true;
            ok
        };
        if parsed {
            record_build_diagnostics(self.container, ok, &diagnostics);
        }
        if ok {
            // One tally for every parsing mode, so the `-o json` envelope and
            // the `-o ndjson` result line carry the same fields.
            Ok(parsed.then(|| buildlog::BuildStats::tally(&diagnostics, start.elapsed())))
        } else {
            // Classified here, the one chokepoint every build goes through, so
            // `build start` and `app run`'s build step both exit 3 on a failed
            // compile instead of the generic 1.
            Err(
                build_failure(&parts, diagnostics, blocker, streamed, &failure_detail)
                    .context("building the project"),
            )
        }
    }
}

/// The error a failed build reports, for [`BuildPlan::run`] and for the
/// interactive `app run` session, which spawns xcodebuild itself. A blocked
/// build ([`buildlog::BlockerWatch`]) leads with the way past it, and stays on
/// the terminal even under a streamed log, since no diagnostic in that log
/// explains it. `detail` is what a captured run appends to the headline, and
/// `streamed` says whether the beautified log already rendered the errors.
pub(crate) fn build_failure(
    args: &[String],
    diagnostics: Vec<serde_json::Value>,
    blocker: Option<String>,
    streamed: bool,
    detail: &str,
) -> CliError {
    let shown = blocker.is_none() && streamed_an_error(streamed, &diagnostics);
    let headline = blocker.map_or_else(
        || format!("xcodebuild exited with a non-zero status{detail}"),
        |hint| format!("the build is blocked, not broken: {hint}"),
    );
    let tip = device_tip(args, &diagnostics);
    let err = CliError::new(headline)
        .kind(ErrorKind::BuildFailure)
        .diagnostics(diagnostics)
        .tip(tip);
    if shown { err.shown() } else { err }
}

/// Where to look next when xcodebuild could not use a physical device it was
/// asked to build for: `device info` connects to the device and names what to
/// fix (a lock, Developer Mode, pairing), which the destination error only
/// hints at. `None` for any other failure, and for a simulator destination.
/// The device is named by its `id=`, else its `name=`.
pub(crate) fn device_tip(args: &[String], diagnostics: &[serde_json::Value]) -> Option<String> {
    let destination_error = diagnostics.iter().any(|d| {
        d["severity"] == "error"
            && d["location"].is_null()
            && d["message"]
                .as_str()
                .is_some_and(buildlog::is_destination_error)
    });
    if !destination_error {
        return None;
    }
    let spec = args
        .windows(2)
        .filter(|pair| pair[0] == "-destination")
        .map(|pair| pair[1].as_str())
        .find(|spec| crate::cli::resolve::is_device_destination(spec))?;
    let key = |k: &str| {
        spec.split(',')
            .find_map(|kv| kv.trim().strip_prefix(k))
            .map(str::trim)
    };
    let device = key("id=")
        .or_else(|| key("name="))
        .map_or_else(String::new, |d| {
            // The tip is single-quoted, so a name that needs quoting gets double
            // quotes inside it.
            if shell_quote(d) == d {
                format!(" {d}")
            } else {
                format!(" \"{d}\"")
            }
        });
    Some(format!(
        "run 'sweetpad device info{device}' to see why the device isn't ready"
    ))
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
    /// Pass `-collect-test-diagnostics never` (see [`skips_test_diagnostics`]).
    pub skip_test_diagnostics: bool,
    /// Extra xcodebuild arguments passed through verbatim (after `--`).
    pub passthrough: &'a [String],
}

/// Whether a test run on Xcode `major` should turn off the diagnostics
/// xcodebuild collects after a failure. From Xcode 26 on, a failed run starts
/// `simctl diagnose --timeout=600`, a sysdiagnose-sized collection that holds
/// the run open for up to ten minutes with nothing on screen, so a red suite
/// looks hung. Older Xcodes collect nothing by default, so they get no flag.
/// A `-collect-test-diagnostics` in the passthrough keeps its own value.
#[must_use]
pub fn skips_test_diagnostics(major: u32, passthrough: &[String]) -> bool {
    major >= 26
        && !passthrough
            .iter()
            .any(|a| a.split('=').next() == Some("-collect-test-diagnostics"))
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
    /// Empty under `-v`, whose raw passthrough is not parsed.
    pub diagnostics: Vec<serde_json::Value>,
    /// The whole captured transcript (`--json` only, where nothing reached the
    /// terminal), for [`record_failure_transcript`].
    pub transcript: Option<String>,
    /// Set when the run was blocked rather than broken — a policy gate no
    /// compile error describes (see [`buildlog::BlockerWatch`]).
    pub blocker: Option<String>,
    /// The beautified human stream rendered the diagnostics as they arrived
    /// (see [`streamed_an_error`]).
    pub streamed: bool,
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
        if self.skip_test_diagnostics {
            args.push("-collect-test-diagnostics".into());
            args.push("never".into());
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
            let (ok, diagnostics, blocker) =
                buildlog::run_ndjson("xcodebuild", &args, cwd.as_deref(), out)?;
            TestRunOutcome {
                passed: ok,
                tail: None,
                diagnostics,
                transcript: None,
                blocker,
                streamed: false,
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
                streamed: false,
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
                streamed: false,
            }
        } else {
            let (ok, diagnostics, blocker) = buildlog::run_collecting(
                "xcodebuild",
                &args,
                cwd.as_deref(),
                out,
                "Testing",
                buildlog::ResultKind::TestFailed,
            )
            .context("running the tests")?;
            TestRunOutcome {
                passed: ok,
                tail: None,
                diagnostics,
                transcript: None,
                blocker,
                streamed: true,
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
/// The result-bundle slot a build writes into: one per project, in the state
/// dir. See [`BuildPlan::result_bundle`] for why a build asks for one at all.
#[must_use]
pub fn build_result_bundle(container: &Container) -> PathBuf {
    project_artifact(container, "-build.xcresult")
}

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
/// object's `diagnostics`. Only the diagnostic's first line is used: a
/// destination error carries xcodebuild's listing on the lines after it (see
/// [`buildlog::LogParser`]), and the colon that introduced them goes too.
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
    let message = first["message"]
        .as_str()
        .and_then(|m| m.lines().next())
        .map_or("(no message)", |line| line.trim_end_matches(':'));
    let more = match total {
        0 | 1 => String::new(),
        n => format!(" (and {} more)", n - 1),
    };
    Some(format!("{location}{message}{more}"))
}

/// Whether a failed run's own log already told the user why: the beautified
/// stream rendered an error as it arrived and closed on its `✗` banner, so a
/// trailing error would only restate it (see [`CliError::shown`]). A failure
/// with no parsed error keeps its trailing message, which is then the only
/// account of what went wrong.
pub(crate) fn streamed_an_error(streamed: bool, diagnostics: &[serde_json::Value]) -> bool {
    streamed && diagnostics.iter().any(|d| d["severity"] == "error")
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
    /// The test within its target, as the test tree names it
    /// (`Class/method()`, `Suite/function()`), and the id `xcresulttool` takes
    /// as `--test-id`.
    pub test_identifier_string: String,
    /// The same test as a `test://` URL, whose spelling settles the `()`
    /// (see [`test_selector`]).
    #[serde(rename = "testIdentifierURL")]
    pub test_identifier_url: String,
    /// The test's failure messages after `failure_text`, in the order it
    /// recorded them. The summary keeps one per test, so these come from the
    /// test tree ([`test_summary`]).
    #[serde(skip)]
    pub other_messages: Vec<String>,
}

/// When a failing test started and when one of its failures was recorded, in
/// seconds since the epoch.
#[derive(Debug, PartialEq)]
pub struct FailureTimes {
    /// The test's `Start Test at …` activity; unit tests record none.
    pub started: Option<f64>,
    pub failed: f64,
}

/// The [`FailureTimes`] of `failure_text` in test `test_id`, from the test's
/// activity log (`xcresulttool get test-results activities`). `None` when the
/// log is unreadable or holds no failure.
#[must_use]
pub fn failure_times(bundle: &Path, test_id: &str, failure_text: &str) -> Option<FailureTimes> {
    let out = process::capture(
        "xcrun",
        &[
            "xcresulttool",
            "get",
            "test-results",
            "activities",
            "--test-id",
            test_id,
            "--path",
            &bundle.to_string_lossy(),
        ],
        None,
    )
    .ok()?;
    parse_failure_times(&out, failure_text)
}

/// Read [`FailureTimes`] out of an activities export. A retried test records
/// one run per attempt, so the last run holding a failure wins. Within it the
/// activity titled with the failure's own text is the moment; a failure the
/// log titles differently falls back to the first activity marked failing.
fn parse_failure_times(out: &str, failure_text: &str) -> Option<FailureTimes> {
    fn find(
        nodes: &[serde_json::Value],
        matches: &dyn Fn(&serde_json::Value) -> bool,
    ) -> Option<f64> {
        nodes.iter().find_map(|node| {
            if matches(node) {
                return node.get("startTime").and_then(serde_json::Value::as_f64);
            }
            let children = node.get("childActivities")?.as_array()?;
            find(children, matches)
        })
    }
    let failing = |node: &serde_json::Value| {
        node.get("isAssociatedWithFailure")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    let titled = |node: &serde_json::Value| {
        failing(node) && node.get("title").and_then(serde_json::Value::as_str) == Some(failure_text)
    };
    let json = out.find('{').map(|i| &out[i..])?;
    let root: serde_json::Value = serde_json::from_str(json).ok()?;
    root.get("testRuns")?
        .as_array()?
        .iter()
        .rev()
        .find_map(|run| {
            let activities = run.get("activities")?.as_array()?;
            let failed = find(activities, &titled).or_else(|| find(activities, &failing))?;
            let started = activities
                .iter()
                .find(|a| {
                    a.get("title")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|t| t.starts_with("Start Test at "))
                })
                .and_then(|a| a.get("startTime").and_then(serde_json::Value::as_f64));
            Some(FailureTimes { started, failed })
        })
}

impl TestFailure {
    /// The failing test as `-only-testing:` takes it — what the summary
    /// prints, so one line can be copied straight into a rerun.
    #[must_use]
    pub fn selector(&self) -> String {
        let test = if self.test_identifier_string.is_empty() {
            &self.test_name
        } else {
            &self.test_identifier_string
        };
        test_selector(
            Some(self.target_name.as_str()),
            test,
            Some(self.test_identifier_url.as_str()).filter(|u| !u.is_empty()),
        )
    }

    /// Every failure message the test recorded: `failure_text`, then the rest.
    pub fn messages(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.failure_text.as_str())
            .chain(self.other_messages.iter().map(String::as_str))
    }
}

/// A test's `-only-testing:` identifier: its test target, then the test as the
/// result bundle names it within that target — `Class/method`, a Swift Testing
/// `Suite/function()`, or a bare `function()` declared outside any suite.
///
/// The bundle's own identifiers stop short of the target, and xcodebuild
/// refuses a rerun built from them alone. Only XCTest's spelling drops the
/// `()`: a Swift Testing test named without it selects nothing, silently. A
/// test's URL keeps the `()` exactly where it belongs, so it decides; with no
/// URL to go by, the test gets XCTest's spelling.
fn test_selector(target: Option<&str>, test: &str, url: Option<&str>) -> String {
    let test = if url.is_some_and(|u| u.ends_with("()")) {
        test
    } else {
        test.strip_suffix("()").unwrap_or(test)
    };
    match target.filter(|t| !t.is_empty()) {
        Some(target) => format!("{target}/{test}"),
        None => test.to_string(),
    }
}

/// One test case in a result bundle's test tree.
struct TreeCase<'a> {
    /// The unit or UI test bundle the case sits under. The tree names a
    /// bundle by its target, which is what `-only-testing:` takes, even when
    /// the product is renamed.
    target: Option<&'a str>,
    identifier: &'a str,
    url: Option<&'a str>,
    failed: bool,
    /// Each distinct failure message recorded under the case, in tree order.
    messages: Vec<&'a str>,
}

/// Walk the xcresulttool test tree (`testNodes`/`children`) for its test
/// cases, each carrying the test bundle it was found under.
fn tree_cases<'a>(
    node: &'a serde_json::Value,
    target: Option<&'a str>,
    out: &mut Vec<TreeCase<'a>>,
) {
    let field = |key: &str| node.get(key).and_then(serde_json::Value::as_str);
    let target = match field("nodeType") {
        Some("Unit test bundle" | "UI test bundle") => field("name").or(target),
        Some("Test Case") => {
            if let Some(identifier) = field("nodeIdentifier") {
                let mut messages = Vec::new();
                failure_messages(node, &mut messages);
                out.push(TreeCase {
                    target,
                    identifier,
                    url: field("nodeIdentifierURL"),
                    failed: field("result").is_some_and(|r| r.eq_ignore_ascii_case("failed")),
                    messages,
                });
            }
            target
        }
        _ => target,
    };
    for key in ["testNodes", "children"] {
        if let Some(nodes) = node.get(key).and_then(serde_json::Value::as_array) {
            for n in nodes {
                tree_cases(n, target, out);
            }
        }
    }
}

/// The `Failure Message` nodes under `node`, whether a test case holds them
/// itself or through the runs below it (a parameterized test's `Arguments`, a
/// retried test's `Repetition`). A retry records the same message again, so
/// each is kept once.
fn failure_messages<'a>(node: &'a serde_json::Value, out: &mut Vec<&'a str>) {
    let children = node.get("children").and_then(serde_json::Value::as_array);
    for child in children.into_iter().flatten() {
        if child.get("nodeType").and_then(serde_json::Value::as_str) == Some("Failure Message") {
            if let Some(message) = child.get("name").and_then(serde_json::Value::as_str)
                && !out.contains(&message)
            {
                out.push(message);
            }
        } else {
            failure_messages(child, out);
        }
    }
}

/// Give each failure in `summary` the rest of its test's failure messages
/// from the tree, found by the test's URL or else by its target and
/// identifier.
fn add_other_messages(summary: &mut TestSummary, root: &serde_json::Value) {
    let mut cases = Vec::new();
    tree_cases(root, None, &mut cases);
    for failure in &mut summary.test_failures {
        let case = cases
            .iter()
            .find(|c| c.url.is_some_and(|u| u == failure.test_identifier_url))
            .or_else(|| {
                cases.iter().find(|c| {
                    c.identifier == failure.test_identifier_string
                        && c.target == Some(failure.target_name.as_str())
                })
            });
        if let Some(case) = case {
            failure.other_messages = case
                .messages
                .iter()
                .filter(|m| **m != failure.failure_text)
                .map(|m| (*m).to_string())
                .collect();
        }
    }
}

/// Read a `.xcresult`'s test tree via `xcrun xcresulttool get test-results
/// tests` (Xcode 16+).
fn test_tree(bundle: &Path) -> Result<serde_json::Value, CliError> {
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
    )?;
    let json = out
        .find('{')
        .map(|i| &out[i..])
        .ok_or_else(|| CliError::new("xcresulttool produced no JSON test tree"))?;
    serde_json::from_str(json).map_err(|e| CliError::new(format!("parsing test tree: {e}")))
}

/// The `-only-testing:` selectors for every test that failed in `bundle`,
/// read from its test tree. A missing bundle yields an empty list ("no
/// previous run").
pub fn failed_test_selectors(bundle: &Path) -> Result<Vec<String>, CliError> {
    if !bundle.exists() {
        return Ok(Vec::new());
    }
    let root = test_tree(bundle).context("reading the previous run's failures")?;
    Ok(failed_selectors(&root))
}

fn failed_selectors(root: &serde_json::Value) -> Vec<String> {
    let mut cases = Vec::new();
    tree_cases(root, None, &mut cases);
    let mut selectors: Vec<String> = cases
        .iter()
        .filter(|c| c.failed)
        .map(|c| test_selector(c.target, c.identifier, c.url))
        .collect();
    selectors.sort();
    selectors.dedup();
    selectors
}

/// Which test target holds each test, keyed by `Class/method`. Only the test
/// tree names the target the way `-only-testing:` does: a test process's
/// markers name its module and its output directory names its product, and
/// either one differs from the target once renamed.
#[derive(Default)]
struct TestTargets {
    by_test: BTreeMap<String, Vec<String>>,
    /// The target under each test's `test://` URL, which stays unique where
    /// one test identifier sits in two targets.
    by_url: BTreeMap<String, String>,
}

impl TestTargets {
    fn from_tree(root: &serde_json::Value) -> Self {
        let mut cases = Vec::new();
        tree_cases(root, None, &mut cases);
        let mut by_test: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut by_url = BTreeMap::new();
        for case in cases {
            if let Some(target) = case.target {
                let targets = by_test
                    .entry(case.identifier.trim_end_matches("()").to_string())
                    .or_default();
                if !targets.iter().any(|t| t == target) {
                    targets.push(target.to_string());
                }
                if let Some(url) = case.url {
                    by_url.insert(url.to_string(), target.to_string());
                }
            }
        }
        Self { by_test, by_url }
    }

    /// `test` (`Class/method()`, `Suite/function()`, as the result bundle
    /// names it within its target) the way `-only-testing:` takes it, found
    /// by its URL or else by the identifier alone. A test the tree doesn't
    /// list keeps the bundle's name, with no target in front.
    fn identifier(&self, test: &str, url: Option<&str>) -> String {
        let target = url
            .and_then(|u| self.by_url.get(u))
            .map(String::as_str)
            .or_else(|| self.target_of(test.trim_end_matches("()"), None));
        test_selector(target, test, url)
    }

    /// The target that ran `test` (`Class/method`), whose marker named
    /// `module`. One source file compiled into two targets puts the same test
    /// in both, and the module each target builds under its own name tells
    /// them apart. A test the tree doesn't list is credited to the module
    /// itself, which is the target's name unless the product or module was
    /// renamed.
    fn target_of<'a>(&'a self, test: &str, module: Option<&'a str>) -> Option<&'a str> {
        let targets = self.by_test.get(test).map_or(&[][..], Vec::as_slice);
        let as_module =
            |target: &str| target.replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_");
        targets
            .iter()
            .find(|t| module.is_some_and(|m| as_module(t) == m))
            .or_else(|| targets.first())
            .map(String::as_str)
            .or(module)
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
///
/// The summary gives one failure message per test, where a test can record
/// several: an app that crashed mid-wait fails the wait as well. So a run with
/// failures also reads the test tree for the rest; when the tree can't be
/// read, each failure keeps the one message the summary gave it.
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
    let mut summary = parse_summary(&out)?;
    if !summary.test_failures.is_empty()
        && let Ok(root) = test_tree(bundle)
    {
        add_other_messages(&mut summary, &root);
    }
    Ok(summary)
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
    /// The same test as `-only-testing:` takes it (`Target/Class/method`),
    /// the name every other view of the run gives it.
    pub identifier: String,
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
    #[serde(default, rename = "testIdentifierURL")]
    test_identifier_url: Option<String>,
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
    // The manifest names a test from its class on, like the rest of the
    // bundle; the tree adds the target. When it can't be read, a test is
    // named without one and its files are exported all the same.
    let targets = test_tree(bundle)
        .map(|root| TestTargets::from_tree(&root))
        .unwrap_or_default();
    parse_attachment_manifest(&manifest_json, staging, &targets)
}

/// Flatten an attachment manifest to one entry per file, each test named by
/// the target `targets` puts it under.
fn parse_attachment_manifest(
    manifest_json: &str,
    staging: &Path,
    targets: &TestTargets,
) -> Result<Vec<ExportedAttachment>, CliError> {
    let entries: Vec<AttachmentManifestEntry> = serde_json::from_str(manifest_json)
        .map_err(|e| CliError::new(format!("parsing the attachment manifest: {e}")))?;

    Ok(entries
        .into_iter()
        .flat_map(|entry| {
            let identifier =
                targets.identifier(&entry.test_identifier, entry.test_identifier_url.as_deref());
            let test = entry.test_identifier;
            entry
                .attachments
                .into_iter()
                .map(move |a| ExportedAttachment {
                    test: test.clone(),
                    identifier: identifier.clone(),
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
    /// `Class/method`, the shape the attachment manifest uses (the marker's
    /// own `Module.Class` spelling is normalized here).
    pub test: String,
    /// `Target/Class/method`, the shape `--only-testing` takes.
    pub identifier: String,
    pub output: String,
}

/// One of XCTest's case markers, read apart.
struct CaseMarker<'a> {
    /// The class's module, when the marker qualifies it (an Objective-C
    /// class is not).
    module: Option<&'a str>,
    /// `Class/method`.
    test: String,
    started: bool,
}

/// XCTest brackets each test's console output with these, on the test
/// process's own stdout: `Test Case '-[Module.Class method]' started.` … then
/// `passed`/`failed`. Everything between is what that test wrote.
fn parse_case_marker(line: &str) -> Option<CaseMarker<'_>> {
    let rest = line.strip_prefix("Test Case '-[")?;
    let (inner, tail) = rest.split_once("]' ")?;
    let (qualified, method) = inner.split_once(' ')?;
    let started = tail.starts_with("started");
    if !started && !tail.starts_with("passed") && !tail.starts_with("failed") {
        return None;
    }
    // The marker spells the class module-qualified; the result bundle
    // does not.
    let class = qualified.rsplit('.').next().unwrap_or(qualified);
    Some(CaseMarker {
        module: qualified.split_once('.').map(|(module, _)| module),
        test: format!("{class}/{method}"),
        started,
    })
}

/// Split one test process's stdout into per-test slices, each named by the
/// target `targets` says ran it.
fn split_output(
    text: &str,
    targets: &TestTargets,
    into: &mut Vec<TestOutput>,
    unattributed: &mut String,
) {
    let mut current: Option<TestOutput> = None;
    for line in text.lines() {
        if let Some(marker) = parse_case_marker(line) {
            if let Some(done) = current.take()
                && !done.output.trim().is_empty()
            {
                into.push(done);
            }
            if marker.started {
                let target = targets.target_of(&marker.test, marker.module);
                current = Some(TestOutput {
                    identifier: test_selector(target, &marker.test, None),
                    test: marker.test,
                    output: String::new(),
                });
            }
            continue;
        }
        // Suite banners are structure, not output; keeping them would bury
        // the handful of real lines in the unattributed bucket.
        if line.starts_with("Test Suite '") {
            continue;
        }
        let sink = match current.as_mut() {
            Some(open) => &mut open.output,
            None => &mut *unattributed,
        };
        sink.push_str(line);
        sink.push('\n');
    }
    if let Some(done) = current
        && !done.output.trim().is_empty()
    {
        into.push(done);
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

    // The tree only names the output. When it can't be read, each test is
    // named by its module instead, and the output is kept all the same.
    let targets = test_tree(bundle)
        .map(|root| TestTargets::from_tree(&root))
        .unwrap_or_default();

    let _ = std::fs::remove_dir_all(keep);
    let _ = std::fs::create_dir_all(keep);
    let mut tests = Vec::new();
    let mut unattributed = String::new();
    let mut sources = Vec::new();
    for path in &streams {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        split_output(&text, &targets, &mut tests, &mut unattributed);
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
/// `xcodebuild` runs from [`working_dir`], so a relative path is joined onto
/// that directory rather than onto the caller's: from a nested source
/// directory the two differ, and the locator would name a bundle that isn't
/// there. A product-relocating build setting is refused (see
/// [`refuse_relocating_settings`]), with the passthrough's wording.
pub fn passthrough_derived_data(
    passthrough: &[String],
    container: &Container,
) -> Result<Option<PathBuf>, CliError> {
    refuse_relocating_settings(passthrough, &[])?;
    let mut derived_data = None;
    let mut iter = passthrough.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == "-derivedDataPath" {
            derived_data = iter.peek().map(|dir| {
                let dir = PathBuf::from(dir);
                match working_dir(container) {
                    Some(base) if dir.is_relative() => base.join(dir),
                    _ => dir,
                }
            });
        }
    }
    Ok(derived_data)
}

/// The `KEY=VALUE` build settings in a passthrough, in order: what
/// `xcodebuild` applies above every project layer. The value after a flag
/// that takes one is skipped, since `-destination platform=macOS` is a
/// specifier, not a setting named `platform`. A flag missing from
/// [`VALUE_FLAGS`] reads as a switch, which costs at most one setting read
/// from its value.
fn passthrough_settings(passthrough: &[String]) -> Vec<(String, String)> {
    let mut settings = Vec::new();
    let mut iter = passthrough.iter();
    while let Some(arg) = iter.next() {
        if VALUE_FLAGS.contains(&arg.as_str()) {
            iter.next();
        } else if let Some((key, value)) = arg.split_once('=')
            && key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            settings.push((key.to_string(), value.to_string()));
        }
    }
    settings
}

/// The `xcodebuild` flags that take the next argument as their value.
const VALUE_FLAGS: [&str; 22] = [
    "-project",
    "-workspace",
    "-target",
    "-scheme",
    "-configuration",
    "-sdk",
    "-arch",
    "-destination",
    "-destination-timeout",
    "-xcconfig",
    "-xctestrun",
    "-testPlan",
    "-toolchain",
    "-jobs",
    "-derivedDataPath",
    "-resultBundlePath",
    "-resultStreamPath",
    "-archivePath",
    "-exportPath",
    "-exportOptionsPlist",
    "-clonedSourcePackagesDirPath",
    "-packageCachePath",
];

/// The build settings that move the product somewhere the locator can't
/// model.
const RELOCATING_SETTINGS: [&str; 3] = ["SYMROOT", "OBJROOT", "CONFIGURATION_BUILD_DIR"];

/// Refuse a product-relocating build setting in `args`: looking in the default
/// DerivedData would name whatever stale `.app` an earlier plain build left
/// there. `from_file` is the project's `[xcodebuild] args`, which `args` starts
/// with. A setting found there is named as the file's, with a fix that works
/// for it, because the verbs that find a built product take no `--` tail to
/// fix it with. Public so `app`'s run plan can spend the refusal before a
/// build rather than after one, and `build` can say why it has no product.
pub fn refuse_relocating_settings(args: &[String], from_file: &[String]) -> Result<(), CliError> {
    let Some((arg, key)) = args.iter().find_map(|arg| {
        let (key, _) = arg.split_once('=')?;
        RELOCATING_SETTINGS.contains(&key).then_some((arg, key))
    }) else {
        return Ok(());
    };
    Err(CliError::new(if from_file.contains(arg) {
        format!(
            "sweetpad.toml: '{key}=…' in [xcodebuild] args relocates the built product where \
             the app locator can't follow; take it out and pass '-- -derivedDataPath <dir>' \
             to the build instead"
        )
    } else {
        format!(
            "'-- {key}=…' relocates the built product where the app locator can't follow; \
             use '-- -derivedDataPath <dir>' instead"
        )
    }))
}

/// Resolve every target's build settings for a plan through the in-process
/// resolver (the engine behind `settings show`), with no `xcodebuild` spawn —
/// including a passthrough's `-derivedDataPath` and its `KEY=VALUE` build
/// settings. Feed the result to
/// [`app_bundle`] to name the product a build of this plan writes. Swift
/// packages build no `.app`, so they have nothing to resolve here.
///
/// `app`'s `RunPlan` locates its install/launch bundle through this same
/// function: one locator, so the CLI cannot report one bundle and install
/// another.
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
        derived_data_path: passthrough_derived_data(plan.passthrough, plan.container)?,
        // The build takes them too: a `PRODUCT_BUNDLE_IDENTIFIER=` or
        // `PRODUCT_NAME=` changes what gets installed and launched.
        overrides: passthrough_settings(plan.passthrough),
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
        split_output(STREAM, &TestTargets::default(), &mut tests, &mut rest);

        // Only tests that wrote something appear, under the identifier shape
        // the result bundle uses — not the marker's `Module.Class`.
        let names: Vec<&str> = tests.iter().map(|t| t.test.as_str()).collect();
        assert_eq!(
            names,
            ["ReflowEngineTests/testBook", "ReflowEngineTests/testBroken"]
        );
        // With no test tree to name the target, the marker's module stands in.
        assert_eq!(
            tests[0].identifier,
            "ReflowTests/ReflowEngineTests/testBook"
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
        split_output(text, &TestTargets::default(), &mut tests, &mut rest);
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

    fn destination_args(specs: &[&str]) -> Vec<String> {
        let mut args = vec!["build".to_string()];
        for spec in specs {
            args.push("-destination".into());
            args.push((*spec).into());
        }
        args
    }

    const TIMEOUT: &str = "xcodebuild: Timed out waiting for all destinations matching the \
                           provided destination specifier to become available";

    #[test]
    fn a_device_destination_error_points_at_device_info() {
        let timeout = vec![diag("error", None, TIMEOUT)];
        assert_eq!(
            device_tip(
                &destination_args(&["platform=iOS,id=00008110-000559182E90401E"]),
                &timeout
            )
            .as_deref(),
            Some(
                "run 'sweetpad device info 00008110-000559182E90401E' to see why the device \
                 isn't ready"
            )
        );
        // A device passed through after sweetpad's own simulator destination
        // is still the one named, and a name stands in for a missing id.
        assert_eq!(
            device_tip(
                &destination_args(&[
                    "platform=iOS Simulator,id=SIM",
                    "platform=iOS,name=Iphone 13"
                ]),
                &timeout
            )
            .as_deref(),
            Some("run 'sweetpad device info \"Iphone 13\"' to see why the device isn't ready")
        );
    }

    #[test]
    fn only_a_device_destination_error_gets_the_tip() {
        let timeout = vec![diag("error", None, TIMEOUT)];
        for spec in [
            "platform=iOS Simulator,id=SIM",
            "platform=macOS",
            "generic/platform=iOS",
        ] {
            assert_eq!(
                device_tip(&destination_args(&[spec]), &timeout),
                None,
                "{spec}"
            );
        }
        let compile = vec![diag(
            "error",
            Some("A.swift:1:1"),
            "cannot find 'x' in scope",
        )];
        let device = destination_args(&["platform=iOS,id=00008110-000559182E90401E"]);
        assert_eq!(device_tip(&device, &compile), None);
        assert_eq!(device_tip(&device, &[]), None);
    }

    #[test]
    fn a_destination_errors_listing_stays_out_of_the_summary() {
        // The listing is the diagnostic's own detail; the headline keeps the
        // error's first line, without the colon that introduced the listing.
        let not_found = vec![diag(
            "error",
            None,
            "xcodebuild: Unable to find a device matching the provided destination specifier:\n  \
             { platform:iOS, id:00008110-000A1B2C3D4E5F60 }\n  \
             (26 other destinations omitted)",
        )];
        assert_eq!(
            diagnostics_summary(&not_found).as_deref(),
            Some("xcodebuild: Unable to find a device matching the provided destination specifier")
        );
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
            action: BuildAction::Build,
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=iOS Simulator,id=UDID"),
            passthrough: &[],
            sdk: None,
            clean: true,
            hot: false,
            hot_entitlements: None,
            result_bundle: None,
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
            action: BuildAction::Build,
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=iOS Simulator,id=UDID"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: true,
            hot_entitlements: None,
            result_bundle: None,
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
            action: BuildAction::Build,
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=macOS"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: true,
            hot_entitlements: None,
            result_bundle: None,
        };
        let args = plan.args();
        assert!(args.contains(&"ENABLE_HARDENED_RUNTIME=NO".to_string()));
        assert!(args.contains(&"ENABLE_APP_SANDBOX=NO".to_string()));
        // A non-hot mac build keeps the project's own protections.
        let cold = BuildPlan {
            action: BuildAction::Build,
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=macOS"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: false,
            hot_entitlements: None,
            result_bundle: None,
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
            action: BuildAction::Build,
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=macOS"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: true,
            hot_entitlements: Some(stripped),
            result_bundle: None,
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
            action: BuildAction::Build,
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=iOS Simulator,id=UDID"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: true,
            hot_entitlements: Some(stripped),
            result_bundle: None,
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
            action: BuildAction::Build,
            container: &c,
            scheme: "App",
            configuration: "Release",
            destination: None,
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: false,
            hot_entitlements: None,
            result_bundle: None,
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
    fn build_args_carry_the_result_bundle() {
        // Not cosmetic: `xcodebuild` writes the build's `.xcactivitylog` only
        // when asked for a result bundle, and that log is the only thing
        // `xcode-build-server` can read compiler arguments out of. Drop the flag
        // and every CLI build silently stops feeding the editor's index.
        let c = Container::Project(PathBuf::from("/work/App.xcodeproj"));
        let bundle = PathBuf::from("/state/App-build.xcresult");
        let plan = BuildPlan {
            action: BuildAction::Build,
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=macOS"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: false,
            hot_entitlements: None,
            result_bundle: Some(bundle.clone()),
        };
        let args = plan.args();
        let at = args
            .iter()
            .position(|a| a == "-resultBundlePath")
            .expect("build asks for a result bundle");
        assert_eq!(args[at + 1], bundle.display().to_string());
    }

    #[test]
    fn a_test_build_asks_for_build_for_testing_and_every_test_target() {
        // `test build` compiles what `test run` would run, so the plan swaps
        // the action and nothing else: the same result-bundle slot (the editor's
        // index reads this build's log too) and no `-only-testing`, which
        // `build-for-testing` ignores when deciding what to compile.
        let c = project();
        let bundle = PathBuf::from("/state/App-build.xcresult");
        let plan = BuildPlan {
            action: BuildAction::BuildForTesting,
            container: &c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=iOS Simulator,id=UDID"),
            passthrough: &[],
            sdk: None,
            clean: false,
            hot: false,
            hot_entitlements: None,
            result_bundle: Some(bundle),
        };
        assert_eq!(
            plan.args(),
            vec![
                "build-for-testing",
                "-scheme",
                "App",
                "-configuration",
                "Debug",
                "-destination",
                "platform=iOS Simulator,id=UDID",
                "-resultBundlePath",
                "/state/App-build.xcresult",
                "-project",
                "/work/App.xcodeproj",
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
            skip_test_diagnostics: true,
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
                "-collect-test-diagnostics",
                "never",
                "-project",
                "/work/App.xcodeproj",
                "-only-testing:AppTests/LoginTests",
                "-skip-testing:AppTests/FlakyTests/testJitter",
            ]
        );
    }

    #[test]
    fn a_failed_run_skips_the_diagnostics_wait_on_xcode_26_and_later() {
        assert!(skips_test_diagnostics(26, &[]));
        assert!(skips_test_diagnostics(27, &["-quiet".into()]));
        // Xcode 16 collects nothing by default, and may not know the flag.
        assert!(!skips_test_diagnostics(16, &[]));
        // An unreadable version is left alone rather than guessed at.
        assert!(!skips_test_diagnostics(0, &[]));
        // The caller's own choice wins, in either spelling.
        let own = ["-collect-test-diagnostics".to_string(), "on-failure".into()];
        assert!(!skips_test_diagnostics(27, &own));
        assert!(!skips_test_diagnostics(
            27,
            &["-collect-test-diagnostics=on-failure".into()]
        ));
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
    fn a_relative_derived_data_path_resolves_where_xcodebuild_runs() {
        let args = |dir: &str| vec!["-derivedDataPath".to_string(), dir.to_string()];
        let nested = Container::Project(PathBuf::from("/work/ios/App.xcodeproj"));
        // xcodebuild runs from the project's directory, whatever the caller's.
        assert_eq!(
            passthrough_derived_data(&args("build/dd"), &nested).unwrap(),
            Some(PathBuf::from("/work/ios/build/dd"))
        );
        assert_eq!(
            passthrough_derived_data(&args("/tmp/dd"), &nested).unwrap(),
            Some(PathBuf::from("/tmp/dd"))
        );
        // A project named relative to the cwd runs xcodebuild in the cwd.
        let here = Container::Project(PathBuf::from("App.xcodeproj"));
        assert_eq!(
            passthrough_derived_data(&args("dd"), &here).unwrap(),
            Some(PathBuf::from("dd"))
        );
        assert_eq!(passthrough_derived_data(&[], &nested).unwrap(), None);
    }

    #[test]
    fn a_relocating_setting_is_refused_in_the_words_of_where_it_came_from() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
        let file = s(&["SYMROOT=/tmp/out"]);
        let from_file = refuse_relocating_settings(&file, &file).unwrap_err();
        assert!(
            from_file
                .message
                .starts_with("sweetpad.toml: 'SYMROOT=…' in [xcodebuild] args"),
            "{}",
            from_file.message
        );
        assert!(!from_file.message.contains("'-- SYMROOT"));

        let typed = refuse_relocating_settings(&s(&["-quiet", "OBJROOT=/o"]), &[]).unwrap_err();
        assert!(
            typed.message.starts_with("'-- OBJROOT=…' relocates"),
            "{}",
            typed.message
        );
        // Merged as the file's args, then the tail: the tail's own setting is
        // still the tail's.
        let merged = s(&["-skipMacroValidation", "CONFIGURATION_BUILD_DIR=/c"]);
        let err = refuse_relocating_settings(&merged, &s(&["-skipMacroValidation"])).unwrap_err();
        assert!(err.message.starts_with("'-- CONFIGURATION_BUILD_DIR=…'"));
        for message in [&from_file.message, &typed.message, &err.message] {
            assert!(!message.contains('`'), "{message}");
        }

        assert!(refuse_relocating_settings(&s(&["TARGET_NAME=x"]), &[]).is_ok());
    }

    #[test]
    fn a_passthroughs_settings_are_its_assignments_not_its_flag_values() {
        let args: Vec<String> = [
            "-quiet",
            "PRODUCT_BUNDLE_IDENTIFIER=com.x.y",
            "-destination",
            "OS=17.0,platform=iOS Simulator",
            "-derivedDataPath",
            "A=b",
            "SWIFT_ACTIVE_COMPILATION_CONDITIONS=DEBUG STAGING",
            "-only-testing:App/T=1",
            "PRODUCT_NAME=",
        ]
        .iter()
        .map(|a| (*a).to_string())
        .collect();
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert_eq!(
            passthrough_settings(&args),
            [
                pair("PRODUCT_BUNDLE_IDENTIFIER", "com.x.y"),
                pair("SWIFT_ACTIVE_COMPILATION_CONDITIONS", "DEBUG STAGING"),
                pair("PRODUCT_NAME", ""),
            ]
        );
    }

    /// The fixture app's own project: a macOS app target whose bundle id and
    /// product name a command-line setting overrides.
    fn fixture_app() -> Container {
        Container::Project(
            PathBuf::from(env!("SWEETPAD_LIB_DIR"))
                .join("fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj"),
        )
    }

    fn located(container: &Container, passthrough: &[String]) -> AppBundle {
        let plan = BuildPlan {
            action: BuildAction::Build,
            container,
            scheme: "SweetpadCIMac",
            configuration: "Debug",
            destination: Some("platform=macOS"),
            sdk: None,
            clean: false,
            hot: false,
            hot_entitlements: None,
            result_bundle: None,
            passthrough,
        };
        app_bundle(&resolved_settings(&plan).unwrap(), plan.destination).unwrap()
    }

    #[test]
    fn the_locator_takes_the_passthroughs_build_settings() {
        let container = fixture_app();
        let plain = located(&container, &[]);
        assert_eq!(plain.bundle_id, "dev.sweetpad.ci.mac");

        let overridden = located(
            &container,
            &[
                "PRODUCT_BUNDLE_IDENTIFIER=com.example.override".to_string(),
                "PRODUCT_NAME=Renamed".to_string(),
            ],
        );
        assert_eq!(overridden.bundle_id, "com.example.override");
        assert_eq!(overridden.path.file_name().unwrap(), "Renamed.app");
        assert_eq!(overridden.path.parent(), plain.path.parent());
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

    #[test]
    fn a_failure_is_timed_by_the_activity_that_names_it() {
        // A UI test whose app aborted mid-wait (Xcode 27), trimmed to the
        // fields read: the crash is recorded inside the failing wait.
        let out = r#"{
          "testIdentifier" : "ProbeUITests/testAppAbortsMidTest()",
          "testRuns" : [ { "activities" : [
            { "isAssociatedWithFailure" : false, "startTime" : 1790436994.038,
              "title" : "Start Test at 2026-09-26 17:36:34.038" },
            { "isAssociatedWithFailure" : true, "startTime" : 1790436999.651,
              "title" : "Waiting 10.0s for \"probe\" StaticText to exist",
              "childActivities" : [
                { "isAssociatedWithFailure" : true, "startTime" : 1790437003.737,
                  "title" : "dev.sweetpad.exitprobe.app crashed" } ] },
            { "isAssociatedWithFailure" : true, "startTime" : 1790437019.985,
              "title" : "Failed to application dev.sweetpad.exitprobe.app is not running" }
          ] } ]
        }"#;
        assert_eq!(
            parse_failure_times(out, "dev.sweetpad.exitprobe.app crashed"),
            Some(FailureTimes {
                started: Some(1_790_436_994.038),
                failed: 1_790_437_003.737,
            })
        );
        // A failure titled differently falls back to the first failing activity.
        assert_eq!(
            parse_failure_times(out, "something else").map(|t| t.failed),
            Some(1_790_436_999.651)
        );
        // A hosted unit test records only the crash, with no start activity.
        let unit = r#"{ "testRuns" : [ { "activities" : [
            { "isAssociatedWithFailure" : true, "startTime" : 1790437907.543,
              "title" : "Crash: ExitProbe at +[XCTFailableInvocation invokeStandardConventionInvocation:completion:]" }
        ] } ] }"#;
        assert_eq!(
            parse_failure_times(
                unit,
                "Crash: ExitProbe at +[XCTFailableInvocation invokeStandardConventionInvocation:completion:]"
            ),
            Some(FailureTimes {
                started: None,
                failed: 1_790_437_907.543,
            })
        );
        assert_eq!(parse_failure_times("not json", "x"), None);
    }

    /// `xcresulttool get test-results tests` for the CI fixture app, extended
    /// with a UI test bundle and Swift Testing tests beside its XCTest class
    /// (durations and source locations dropped). Captured on Xcode 27.
    const TREE: &str = r#"{ "testNodes": [
  { "name": "SweetpadCIApp", "nodeType": "Test Plan", "result": "Failed", "children": [
    { "name": "SweetpadCIAppTests", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests", "nodeType": "Unit test bundle", "result": "Failed", "children": [
      { "name": "freeGreeting()", "nodeIdentifier": "freeGreeting()", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/freeGreeting()", "nodeType": "Test Case", "result": "Failed", "children": [
        { "name": "Expectation failed: \"free\" == \"bound\"", "nodeType": "Failure Message" }
      ] },
      { "name": "freePasses()", "nodeIdentifier": "freePasses()", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/freePasses()", "nodeType": "Test Case", "result": "Passed" },
      { "name": "AppTests", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/AppTests", "nodeType": "Test Suite", "result": "Failed", "children": [
        { "name": "testArithmetic()", "nodeIdentifier": "AppTests/testArithmetic()", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/AppTests/testArithmetic", "nodeType": "Test Case", "result": "Passed" },
        { "name": "testGreeting()", "nodeIdentifier": "AppTests/testGreeting()", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/AppTests/testGreeting", "nodeType": "Test Case", "result": "Failed", "children": [
          { "name": "XCTAssertEqual failed: (\"hello\") is not equal to (\"world\")", "nodeType": "Failure Message" }
        ] }
      ] },
      { "name": "GreetingSuite", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite", "nodeType": "Test Suite", "result": "Failed", "children": [
        { "name": "Nested", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/Nested", "nodeType": "Test Suite", "result": "Failed", "children": [
          { "name": "inner()", "nodeIdentifier": "GreetingSuite/Nested/inner()", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/Nested/inner()", "nodeType": "Test Case", "result": "Failed", "children": [
            { "name": "Expectation failed: Bool(false)\nfalse → ()", "nodeType": "Failure Message" }
          ] }
        ] },
        { "name": "suiteGreeting()", "nodeIdentifier": "GreetingSuite/suiteGreeting()", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/suiteGreeting()", "nodeType": "Test Case", "result": "Failed", "children": [
          { "name": "Expectation failed: 1 == 2", "nodeType": "Failure Message" }
        ] },
        { "name": "A display name", "nodeIdentifier": "GreetingSuite/named()", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/named()", "nodeType": "Test Case", "result": "Failed", "children": [
          { "name": "Expectation failed: Bool(false)\nfalse → ()", "nodeType": "Failure Message" }
        ] },
        { "name": "param(n:)", "nodeIdentifier": "GreetingSuite/param(n:)", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/param(n:)", "nodeType": "Test Case", "result": "Failed", "children": [
          { "name": "1", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/param(n:)?args=6b86b273ff34fce19d6b804eff5a3f5747ada4eaa22f1d49c01e52ddb7875b4b", "nodeType": "Arguments", "result": "Passed" },
          { "name": "2", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/param(n:)?args=d4735e3a265e16eee03f59718b9b5d03019c07d8b6c51f90da3a666eec13ab35", "nodeType": "Arguments", "result": "Failed", "children": [
            { "name": "Expectation failed: n == 1\nn → 2", "nodeType": "Failure Message" }
          ] }
        ] }
      ] }
    ] },
    { "name": "SweetpadCIAppUITests", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppUITests", "nodeType": "UI test bundle", "result": "Failed", "children": [
      { "name": "AppUITests", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppUITests/AppUITests", "nodeType": "Test Suite", "result": "Failed", "children": [
        { "name": "testLaunchShowsNothing()", "nodeIdentifier": "AppUITests/testLaunchShowsNothing()", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppUITests/AppUITests/testLaunchShowsNothing", "nodeType": "Test Case", "result": "Failed", "children": [
          { "name": "failed - deliberate UI failure", "nodeType": "Failure Message" }
        ] }
      ] }
    ] }
  ] }
] }"#;

    /// The failures `xcresulttool get test-results summary` reports for the
    /// same run as [`TREE`], one per framework and bundle.
    const SUMMARY: &str = r#"{"result": "Failed", "totalTestCount": 9, "passedTests": 2, "failedTests": 7, "testFailures": [
{"failureText": "Expectation failed: \"free\" == \"bound\"", "targetName": "SweetpadCIAppTests", "testIdentifier": 7, "testIdentifierString": "freeGreeting()", "testIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/freeGreeting()", "testName": "freeGreeting()"},
{"failureText": "Expectation failed: Bool(false)\nfalse → ()", "targetName": "SweetpadCIAppTests", "testIdentifier": 6, "testIdentifierString": "GreetingSuite/Nested/inner()", "testIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/Nested/inner()", "testName": "inner()"},
{"failureText": "Expectation failed: Bool(false)\nfalse → ()", "targetName": "SweetpadCIAppTests", "testIdentifier": 4, "testIdentifierString": "GreetingSuite/named()", "testIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/named()", "testName": "A display name"},
{"failureText": "Expectation failed: n == 1\nn → 2", "targetName": "SweetpadCIAppTests", "testIdentifier": 5, "testIdentifierString": "GreetingSuite/param(n:)", "testIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/param(n:)", "testName": "param(n:)"},
{"failureText": "Expectation failed: 1 == 2", "targetName": "SweetpadCIAppTests", "testIdentifier": 3, "testIdentifierString": "GreetingSuite/suiteGreeting()", "testIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/suiteGreeting()", "testName": "suiteGreeting()"},
{"failureText": "XCTAssertEqual failed: (\"hello\") is not equal to (\"world\")", "targetName": "SweetpadCIAppTests", "testIdentifier": 2, "testIdentifierString": "AppTests/testGreeting()", "testIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/AppTests/testGreeting", "testName": "testGreeting()"},
{"failureText": "failed - deliberate UI failure", "targetName": "SweetpadCIAppUITests", "testIdentifier": 9, "testIdentifierString": "AppUITests/testLaunchShowsNothing()", "testIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppUITests/AppUITests/testLaunchShowsNothing", "testName": "testLaunchShowsNothing()"}
]}"#;

    fn tree() -> serde_json::Value {
        serde_json::from_str(TREE).unwrap()
    }

    #[test]
    fn a_failed_rerun_names_each_test_by_its_target() {
        // The tree's own identifiers stop short of the target, and a rerun
        // built from them is refused outright ("isn't a member of the
        // specified test plan or scheme"). Each case takes the bundle it sits
        // under — unit or UI — and only XCTest's spelling drops the `()`,
        // since a Swift Testing test named without it selects nothing.
        assert_eq!(
            failed_selectors(&tree()),
            [
                "SweetpadCIAppTests/AppTests/testGreeting",
                "SweetpadCIAppTests/GreetingSuite/Nested/inner()",
                "SweetpadCIAppTests/GreetingSuite/named()",
                "SweetpadCIAppTests/GreetingSuite/param(n:)",
                "SweetpadCIAppTests/GreetingSuite/suiteGreeting()",
                "SweetpadCIAppTests/freeGreeting()",
                "SweetpadCIAppUITests/AppUITests/testLaunchShowsNothing",
            ]
        );
    }

    #[test]
    fn the_summary_names_a_failure_the_way_a_rerun_takes_it() {
        // One spelling for one test: each line the summary prints can be
        // copied into `--only-testing`, and is what `--failed` reruns.
        let summary = parse_summary(SUMMARY).unwrap();
        let mut printed: Vec<String> = summary
            .test_failures
            .iter()
            .map(TestFailure::selector)
            .collect();
        printed.sort();
        assert_eq!(printed, failed_selectors(&tree()));
    }

    #[test]
    fn a_failure_carries_every_message_its_test_recorded() {
        // Trimmed from Xcode 27 runs of the fixture: an XCTest case with two
        // failed assertions, retried once, and a UI test whose app crashed
        // mid-wait. The summary gives each of them its first message only.
        let url = "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/AppTests/testGreeting";
        let equal = "XCTAssertEqual failed: (\"hello\") is not equal to (\"world\")";
        let second = "XCTAssertTrue failed - second failure in the same test";
        let run = |name: &str| {
            serde_json::json!({ "name": name, "nodeIdentifierURL": url, "nodeType": "Repetition",
                "result": "Failed", "children": [
                    { "name": equal, "nodeType": "Failure Message" },
                    { "name": second, "nodeType": "Failure Message" } ] })
        };
        let recorded = serde_json::json!({ "testNodes": [
            { "name": "SweetpadCIAppTests", "nodeType": "Unit test bundle", "children": [
                { "name": "AppTests", "nodeType": "Test Suite", "children": [
                    { "name": "testGreeting()", "nodeIdentifier": "AppTests/testGreeting()",
                      "nodeIdentifierURL": url, "nodeType": "Test Case", "result": "Failed",
                      "children": [ run("First Run"), run("Retry 1") ] } ] } ] },
            { "name": "SweetpadCIAppUITests", "nodeType": "UI test bundle", "children": [
                { "name": "AppUITests", "nodeType": "Test Suite", "children": [
                    { "name": "testAppCrashesMidTest()",
                      "nodeIdentifier": "AppUITests/testAppCrashesMidTest()",
                      "nodeType": "Test Case", "result": "Failed", "children": [
                        { "name": "dev.sweetpad.ci.app crashed", "nodeType": "Failure Message" },
                        { "name": "XCTAssertTrue failed", "nodeType": "Failure Message" } ] } ] } ] }
        ] });
        let mut summary = parse_summary(
            &serde_json::json!({ "testFailures": [
            { "failureText": equal, "targetName": "SweetpadCIAppTests",
              "testIdentifierString": "AppTests/testGreeting()", "testIdentifierURL": url,
              "testName": "testGreeting()" },
            { "failureText": "dev.sweetpad.ci.app crashed", "targetName": "SweetpadCIAppUITests",
              "testIdentifierString": "AppUITests/testAppCrashesMidTest()",
              "testName": "testAppCrashesMidTest()" }
        ] })
            .to_string(),
        )
        .unwrap();
        add_other_messages(&mut summary, &recorded);
        // The retry recorded both messages again; each is listed once.
        assert_eq!(summary.test_failures[0].other_messages, [second]);
        assert_eq!(
            summary.test_failures[0].messages().collect::<Vec<_>>(),
            [equal, second]
        );
        // With no URL, the test is found by its target and identifier.
        assert_eq!(
            summary.test_failures[1].messages().collect::<Vec<_>>(),
            ["dev.sweetpad.ci.app crashed", "XCTAssertTrue failed"]
        );

        // A test that recorded one failure, parameterized or not, adds none.
        let mut summary = parse_summary(SUMMARY).unwrap();
        add_other_messages(&mut summary, &tree());
        assert!(
            summary
                .test_failures
                .iter()
                .all(|f| f.other_messages.is_empty())
        );
    }

    #[test]
    fn a_test_without_a_url_gets_the_xctest_spelling() {
        assert_eq!(
            test_selector(Some("AppTests"), "LoginTests/testSignIn()", None),
            "AppTests/LoginTests/testSignIn"
        );
        // A bundle the tree didn't name leaves the identifier as it is found.
        assert_eq!(
            test_selector(None, "LoginTests/testSignIn()", None),
            "LoginTests/testSignIn"
        );
        // A summary entry with no identifier string falls back to the name.
        let failure = TestFailure {
            test_name: "testSignIn()".into(),
            target_name: "AppTests".into(),
            ..TestFailure::default()
        };
        assert_eq!(failure.selector(), "AppTests/testSignIn");
    }

    #[test]
    fn an_attachment_is_filed_under_the_name_a_rerun_takes() {
        // The manifest's `testIdentifier` starts at the class, like the rest
        // of the bundle; its URL finds the test in the tree, which adds the
        // target. Trimmed from an export of the same run as [`TREE`].
        let manifest = r#"[
          { "testIdentifier": "AppTests/testGreeting()",
            "testIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/AppTests/testGreeting",
            "attachments": [ { "exportedFileName": "FC88EDEC.txt",
              "suggestedHumanReadableName": "greeting-note_0_01AF03AA-6BD7-4522-9FFF-BEAA0B4D2F0D.txt",
              "isAssociatedWithFailure": false, "timestamp": 1790441857.045 } ] },
          { "testIdentifier": "GreetingSuite/suiteGreeting()",
            "testIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/suiteGreeting()",
            "attachments": [ { "exportedFileName": "8B93D9DB.txt", "timestamp": 1790441857.306 } ] },
          { "testIdentifier": "AppUITests/testLaunchShowsNothing()",
            "attachments": [ { "exportedFileName": "B21A126E.mp4", "timestamp": 1790441864.451 } ] },
          { "testIdentifier": "Gone/testElsewhere()",
            "attachments": [ { "exportedFileName": "0A8B57AD.txt", "timestamp": 1790441879.133 } ] }
        ]"#;
        let staging = Path::new("/staging");
        let exported =
            parse_attachment_manifest(manifest, staging, &TestTargets::from_tree(&tree())).unwrap();
        let names: Vec<(&str, &str)> = exported
            .iter()
            .map(|a| (a.test.as_str(), a.identifier.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                (
                    "AppTests/testGreeting()",
                    "SweetpadCIAppTests/AppTests/testGreeting"
                ),
                (
                    "GreetingSuite/suiteGreeting()",
                    "SweetpadCIAppTests/GreetingSuite/suiteGreeting()"
                ),
                // An entry without a URL is found by its identifier alone.
                (
                    "AppUITests/testLaunchShowsNothing()",
                    "SweetpadCIAppUITests/AppUITests/testLaunchShowsNothing"
                ),
                // One the tree doesn't list keeps the bundle's own name.
                ("Gone/testElsewhere()", "Gone/testElsewhere"),
            ]
        );
        assert_eq!(exported[0].file, staging.join("FC88EDEC.txt"));
        assert_eq!(
            exported[0].suggested_name,
            "greeting-note_0_01AF03AA-6BD7-4522-9FFF-BEAA0B4D2F0D.txt"
        );
        // An attachment the test gave no name to keeps the exported one.
        assert_eq!(exported[1].suggested_name, "8B93D9DB.txt");
    }

    #[test]
    fn a_url_tells_apart_one_test_in_two_targets() {
        let targets = TestTargets::from_tree(&serde_json::json!({ "testNodes": [
            { "name": "AppTests", "nodeType": "Unit test bundle", "children": [
                { "name": "t()", "nodeIdentifier": "Shared/t()", "nodeIdentifierURL": "test://p/AppTests/Shared/t", "nodeType": "Test Case" }
            ] },
            { "name": "AppIntegrationTests", "nodeType": "Unit test bundle", "children": [
                { "name": "t()", "nodeIdentifier": "Shared/t()", "nodeIdentifierURL": "test://p/AppIntegrationTests/Shared/t", "nodeType": "Test Case" }
            ] }
        ] }));
        assert_eq!(
            targets.identifier("Shared/t()", Some("test://p/AppIntegrationTests/Shared/t")),
            "AppIntegrationTests/Shared/t"
        );
        assert_eq!(targets.identifier("Shared/t()", None), "AppTests/Shared/t");
    }

    /// The fixture's own test process stream, from the same run as [`TREE`].
    const FIXTURE_STREAM: &str = "\
Test Suite 'AppTests' started at 2026-09-26 17:26:12.841.
Test Case '-[SweetpadCIAppTests.AppTests testArithmetic]' started.
Test Case '-[SweetpadCIAppTests.AppTests testArithmetic]' passed (0.002 seconds).
Test Case '-[SweetpadCIAppTests.AppTests testGreeting]' started.
greeting under test
Test Case '-[SweetpadCIAppTests.AppTests testGreeting]' failed (0.223 seconds).
";

    #[test]
    fn a_tests_output_is_headed_the_way_a_rerun_takes_it() {
        let targets = TestTargets::from_tree(&tree());
        let (mut tests, mut rest) = (Vec::new(), String::new());
        split_output(FIXTURE_STREAM, &targets, &mut tests, &mut rest);
        assert_eq!(tests.len(), 1);
        assert_eq!(
            tests[0].identifier,
            "SweetpadCIAppTests/AppTests/testGreeting"
        );
        assert_eq!(tests[0].test, "AppTests/testGreeting");

        // A renamed product builds under its own module name, while
        // `-only-testing` still takes the target's: the tree is what knows it.
        let renamed = FIXTURE_STREAM.replace("SweetpadCIAppTests.", "Renamed_Tests.");
        let (mut tests, mut rest) = (Vec::new(), String::new());
        split_output(&renamed, &targets, &mut tests, &mut rest);
        assert_eq!(
            tests[0].identifier,
            "SweetpadCIAppTests/AppTests/testGreeting"
        );
    }

    #[test]
    fn one_test_in_two_targets_is_told_apart_by_its_module() {
        // A source file compiled into two test targets puts the same
        // Class/method in both; the marker's module picks the one that ran.
        let targets = TestTargets::from_tree(&serde_json::json!({ "testNodes": [
            { "name": "App Tests", "nodeType": "Unit test bundle", "children": [
                { "name": "t()", "nodeIdentifier": "Shared/t()", "nodeType": "Test Case" }
            ] },
            { "name": "AppIntegrationTests", "nodeType": "Unit test bundle", "children": [
                { "name": "t()", "nodeIdentifier": "Shared/t()", "nodeType": "Test Case" }
            ] }
        ] }));
        assert_eq!(
            targets.target_of("Shared/t", Some("AppIntegrationTests")),
            Some("AppIntegrationTests")
        );
        // A space is not an identifier character, so it builds as `App_Tests`.
        assert_eq!(
            targets.target_of("Shared/t", Some("App_Tests")),
            Some("App Tests")
        );
        // A test the tree doesn't list is named by its module.
        assert_eq!(targets.target_of("Other/t", Some("Mod")), Some("Mod"));
    }
}
