//! `sweetpad test …` — run tests via `xcodebuild test`, the sibling of
//! `build`. Streams output in human mode; emits a parsed pass/fail summary
//! under `--json` (and per-test events under `-o ndjson`), read back from the
//! `.xcresult` bundle. The bundle is **retained** (state dir, or
//! `--result-bundle PATH`) so the run's own record stays readable afterwards:
//! `test run --failed` reruns just the last run's failures, `test attachments`
//! exports what those tests attached, and `test output` shows what they
//! printed. A verdict is what the summary carries; the evidence behind it lives
//! in the bundle. `test build` compiles the test targets without running them;
//! it is a build rather than a run, so it writes `build`'s record and leaves the
//! retained bundle alone.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::resolve::Container;
use crate::cli::{
    CliError, CommandResult, Context, Render, Rendered, exits, resolve, simctl, swiftpm, xcodebuild,
};

/// The test flags, declared `global` at the `test` resource so they parse on
/// either side of the (optional) `run` token: `sweetpad test --failed` and
/// `sweetpad test run --failed` are the same invocation.
#[derive(Debug, clap::Args)]
#[allow(clippy::struct_excessive_bools)] // independent CLI toggles, not a state machine
pub struct TestArgs {
    #[command(flatten)]
    pub target: crate::cli::BuildTargetArgs,

    /// Run only this test identifier (Target[/Class[/method]]); repeatable.
    #[arg(long = "only-testing", global = true)]
    pub only_testing: Vec<String>,

    /// Skip this test identifier; repeatable.
    #[arg(long = "skip-testing", global = true)]
    pub skip_testing: Vec<String>,

    /// Rerun only the tests that failed in the previous run (read from the
    /// retained result bundle).
    #[arg(long, conflicts_with = "only_testing", global = true)]
    pub failed: bool,

    /// Where to write the .xcresult bundle (default: retained per project
    /// in the state dir, replacing the previous run's).
    #[arg(long, value_name = "PATH", global = true)]
    pub result_bundle: Option<PathBuf>,

    /// Also write a JUnit XML report to PATH (for CI).
    #[arg(long, value_name = "PATH", global = true)]
    pub junit: Option<PathBuf>,

    /// Rerun the tests on every Swift save (Ctrl-C stops).
    #[arg(long, global = true, conflicts_with = "show_command")]
    pub watch: bool,

    /// Retry failing tests, running each up to N times before calling it
    /// failed (xcodebuild's -retry-tests-on-failure).
    #[arg(long = "retry-flaky", value_name = "N", global = true)]
    pub retry_flaky: Option<u32>,

    /// Collect code coverage; the summary rides in the report (via xccov).
    #[arg(long, global = true)]
    pub coverage: bool,

    /// Print the exact xcodebuild invocation that would run, then exit.
    #[arg(long, global = true)]
    pub show_command: bool,

    /// Extra arguments passed to xcodebuild verbatim (after '--').
    #[arg(last = true, value_name = "XCODEBUILD_ARGS", global = true)]
    pub passthrough: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Run the resolved scheme's tests (the default action: 'sweetpad test').
    Run,
    /// Compile the scheme's test targets without running them (xcodebuild's
    /// 'build-for-testing').
    ///
    /// Its output matches 'sweetpad build', and 'sweetpad build diagnostics'
    /// reads its errors back afterwards.
    Build(BuildArgs),
    /// Export the last run's attachments (screenshots, UI dumps) as files.
    Attachments(AttachmentsArgs),
    /// Show what the last run's tests printed, per test.
    Output(OutputArgs),
}

/// The run flags a build refuses, redeclared hidden on `test build` under the
/// same ids. A subcommand's own arg keeps the resource's global one from
/// propagating into it, so its help leaves them out; a stray one still parses,
/// and its value reaches [`TestArgs`] for [`build`] to refuse.
#[derive(Debug, clap::Args)]
pub struct BuildArgs {
    /// Recompile the test targets on every Swift save (Ctrl-C stops).
    #[arg(long, conflicts_with = "show_command")]
    pub watch: bool,

    #[arg(long = "only-testing", hide = true)]
    pub only_testing: Vec<String>,
    #[arg(long = "skip-testing", hide = true)]
    pub skip_testing: Vec<String>,
    #[arg(long, hide = true)]
    pub failed: bool,
    #[arg(long, hide = true)]
    pub result_bundle: Option<PathBuf>,
    #[arg(long, hide = true)]
    pub junit: Option<PathBuf>,
    #[arg(long = "retry-flaky", hide = true)]
    pub retry_flaky: Option<u32>,
    #[arg(long, hide = true)]
    pub coverage: bool,
}

/// The run flags that reading the last run back refuses, redeclared hidden on
/// `test attachments` and `test output` the way [`BuildArgs`] does for `test
/// build`.
#[derive(Debug, clap::Args)]
#[allow(clippy::struct_excessive_bools)] // mirrors TestArgs' toggles, none of them read here
pub struct HiddenRunArgs {
    #[arg(long = "skip-testing", hide = true)]
    pub skip_testing: Vec<String>,
    #[arg(long, hide = true)]
    pub failed: bool,
    #[arg(long, hide = true)]
    pub junit: Option<PathBuf>,
    #[arg(long, hide = true)]
    pub watch: bool,
    #[arg(long = "retry-flaky", hide = true)]
    pub retry_flaky: Option<u32>,
    #[arg(long, hide = true)]
    pub coverage: bool,
    #[arg(long, hide = true)]
    pub show_command: bool,
    #[arg(last = true, hide = true)]
    pub passthrough: Vec<String>,
}

/// The flags of `test output`. '--only-testing' and '--result-bundle' keep the
/// ids of the run's own, so their values land on [`TestArgs`] and read the
/// same as they do on a run.
#[derive(Debug, clap::Args)]
pub struct OutputArgs {
    /// Show each test's output in full instead of its last few KB.
    #[arg(long)]
    pub full: bool,

    /// Show only this test's output (Target[/Class[/method]]); repeatable.
    #[arg(long = "only-testing")]
    pub only_testing: Vec<String>,

    /// The .xcresult bundle to read (default: the one the last 'sweetpad
    /// test' kept for this project).
    #[arg(long, value_name = "PATH")]
    pub result_bundle: Option<PathBuf>,

    #[command(flatten)]
    pub run: HiddenRunArgs,
}

/// The flags of `test attachments`, with '--only-testing' and
/// '--result-bundle' declared as on [`OutputArgs`].
#[derive(Debug, clap::Args)]
pub struct AttachmentsArgs {
    /// Where to write the files (default: a directory beside the retained
    /// result bundle, replacing the previous export).
    #[arg(long, value_name = "DIR")]
    pub output_dir: Option<PathBuf>,

    /// Export only the attachments recorded against a failing test.
    #[arg(long)]
    pub only_failures: bool,

    /// Export only this test's attachments (Target[/Class[/method]]);
    /// repeatable.
    #[arg(long = "only-testing")]
    pub only_testing: Vec<String>,

    /// The .xcresult bundle to read (default: the one the last 'sweetpad
    /// test' kept for this project).
    #[arg(long, value_name = "PATH")]
    pub result_bundle: Option<PathBuf>,

    #[command(flatten)]
    pub run: HiddenRunArgs,
}

/// The flags of one `test run`, bundled so helpers don't take eight params.
struct RunArgs<'a> {
    only_testing: &'a [String],
    skip_testing: &'a [String],
    failed: bool,
    result_bundle: Option<&'a Path>,
    junit: Option<&'a Path>,
    retry_flaky: Option<u32>,
    coverage: bool,
    show_command: bool,
    passthrough: &'a [String],
}

pub fn run(ctx: &mut Context, args: &TestArgs, action: Option<&Action>) -> CommandResult {
    ctx.targeting = args.target.clone().into();
    match action {
        Some(Action::Attachments(opts)) => {
            refuse_run_flags(&read_refused_flags(args), &read_reason("attachments"))?;
            return attachments(ctx, args, opts);
        }
        Some(Action::Output(opts)) => {
            refuse_run_flags(&read_refused_flags(args), &read_reason("output"))?;
            return output(ctx, args, opts);
        }
        Some(Action::Build(_)) => return build(ctx, args),
        Some(Action::Run) | None => {}
    }
    let passthrough = ctx.xcodebuild_args(&args.passthrough)?;
    let run_args = RunArgs {
        only_testing: &args.only_testing,
        skip_testing: &args.skip_testing,
        failed: args.failed,
        result_bundle: args.result_bundle.as_deref(),
        junit: args.junit.as_deref(),
        retry_flaky: args.retry_flaky,
        coverage: args.coverage,
        show_command: args.show_command,
        passthrough: &passthrough,
    };
    if args.watch {
        let resolved = resolve::resolve_testing(ctx)?;
        let root = resolved
            .container
            .path()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        return super::watch_swift(ctx, &root, |ctx| test(ctx, &run_args));
    }
    test(ctx, &run_args)
}

/// `test build`: compile what `test run` would run, and run none of it. The
/// build is `build`'s own (see [`super::build::for_testing`]); what this adds
/// is refusing the flags that only shape a run, which would otherwise parse —
/// they are resource-global — and be silently dropped.
fn build(ctx: &mut Context, args: &TestArgs) -> CommandResult {
    refuse_run_flags(
        &run_only_flags(args),
        ", not a build: 'test build' compiles every test target in the scheme and runs none",
    )?;
    super::build::for_testing(ctx, args.watch, args.show_command, &args.passthrough)
}

/// Refuse the run flags in `given` by name, since each one parsed on a verb it
/// means nothing to and dropping it would not do what was asked. `why` follows
/// "…apply to a test run" in the message.
fn refuse_run_flags(given: &[&str], why: &str) -> Result<(), CliError> {
    let Some((last, rest)) = given.split_last() else {
        return Ok(());
    };
    let (flags, verb) = if rest.is_empty() {
        ((*last).to_string(), "applies")
    } else {
        (format!("{} and {last}", rest.join(", ")), "apply")
    };
    Err(CliError::new(format!("{flags} {verb} to a test run{why}")))
}

/// Why `test <verb>` refuses a run flag.
fn read_reason(verb: &str) -> String {
    format!(": 'test {verb}' reads the last run's result bundle and runs nothing")
}

/// The flags on `args` that shape a test run and mean nothing to reading one
/// back. `--only-testing` and `--result-bundle` are not among them: they pick
/// the tests and the bundle to read.
fn read_refused_flags(args: &TestArgs) -> Vec<&'static str> {
    [
        ("--skip-testing", !args.skip_testing.is_empty()),
        ("--failed", args.failed),
        ("--junit", args.junit.is_some()),
        ("--watch", args.watch),
        ("--retry-flaky", args.retry_flaky.is_some()),
        ("--coverage", args.coverage),
        ("--show-command", args.show_command),
        ("'-- XCODEBUILD_ARGS'", !args.passthrough.is_empty()),
    ]
    .into_iter()
    .filter_map(|(flag, given)| given.then_some(flag))
    .collect()
}

/// The flags on `args` that shape a test run and mean nothing to a build.
/// `--only-testing` is among them because `build-for-testing` compiles every
/// target the scheme tests whatever the filter says, so no narrowing is on
/// offer.
fn run_only_flags(args: &TestArgs) -> Vec<&'static str> {
    [
        ("--only-testing", !args.only_testing.is_empty()),
        ("--skip-testing", !args.skip_testing.is_empty()),
        ("--failed", args.failed),
        ("--result-bundle", args.result_bundle.is_some()),
        ("--junit", args.junit.is_some()),
        ("--retry-flaky", args.retry_flaky.is_some()),
        ("--coverage", args.coverage),
    ]
    .into_iter()
    .filter_map(|(flag, given)| given.then_some(flag))
    .collect()
}

/// The xcodebuild test result: a pass/fail summary line and failure list in
/// human mode, the parsed counts + failures + retained bundle path as JSON.
struct TestReport {
    passed: bool,
    summary: xcodebuild::TestSummary,
    /// What ended the app or the test runner, per failure (same order as
    /// `summary.test_failures`), for the failures that say one vanished.
    terminations: Vec<Option<Termination>>,
    coverage: Option<f64>,
    result_bundle: String,
}

/// launchd's account of the exit behind a failure (CLI_DESIGN §9m), with the
/// crash report the system wrote when the exit was a crash.
struct Termination {
    exit: exits::Exit,
    crash_report: Option<PathBuf>,
}

impl Termination {
    /// `app terminated: …` or `test runner terminated: …`, by whose exit it was.
    fn line(&self) -> String {
        let who = if self.exit.bundle_id.ends_with(".xctrunner") {
            "test runner"
        } else {
            "app"
        };
        format!("{who} terminated: {}", self.exit.summary())
    }
}

impl TestReport {
    /// One `✗` line per failed test, naming it and its first failure, with
    /// its other failures and what the report adds about it indented below.
    fn failure_lines(&self) -> Vec<String> {
        // A Swift Testing expectation continues with one line per operand
        // (`n → 2`). They sit under the message they belong to, deeper than
        // the lines that start a message.
        fn push_message(lines: &mut Vec<String>, head: &str, message: &str) {
            let mut message = message.lines();
            lines.push(format!("{head}{}", message.next().unwrap_or_default()));
            lines.extend(message.map(|line| format!("        {line}")));
        }
        let mut lines = Vec::new();
        for (i, f) in self.summary.test_failures.iter().enumerate() {
            push_message(
                &mut lines,
                &format!("  ✗ {}: ", f.selector()),
                &f.failure_text,
            );
            for other in &f.other_messages {
                push_message(&mut lines, "      ", other);
            }
            if let Some(Some(termination)) = self.terminations.get(i) {
                lines.push(format!("      {}", termination.line()));
            }
        }
        lines
    }
}

impl Render for TestReport {
    fn human(&self, out: &Output) {
        out.line(&format!(
            "{} passed, {} failed, {} skipped ({} total)",
            self.summary.passed_tests,
            self.summary.failed_tests,
            self.summary.skipped_tests,
            self.summary.total_test_count
        ));
        for line in self.failure_lines() {
            out.line(&line);
        }
        if let Some(coverage) = self.coverage {
            out.line(&format!("coverage: {:.1}%", coverage * 100.0));
        }
        out.note(&format!("result bundle: {}", self.result_bundle));
    }

    fn json(&self) -> serde_json::Value {
        let failures: Vec<serde_json::Value> = self
            .summary
            .test_failures
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let mut failure = serde_json::json!({
                    "test": f.test_name,
                    "target": f.target_name,
                    "identifier": f.selector(),
                    "message": f.failure_text,
                    "messages": f.messages().collect::<Vec<_>>(),
                });
                if let Some(Some(t)) = self.terminations.get(i) {
                    failure["terminationReason"] = t.exit.json(t.crash_report.as_deref());
                }
                failure
            })
            .collect();
        serde_json::json!({
            "passed": self.passed,
            "total": self.summary.total_test_count,
            "passedTests": self.summary.passed_tests,
            "failedTests": self.summary.failed_tests,
            "skippedTests": self.summary.skipped_tests,
            "failures": failures,
            "lineCoverage": self.coverage,
            "resultBundle": self.result_bundle,
        })
    }
}

/// A Swift package test result — `swift test` gives no `.xcresult`, so the only
/// machine-readable fact is the pass/fail flag (no human summary line).
struct SpmTestReport {
    passed: bool,
}

impl Render for SpmTestReport {
    fn human(&self, _out: &Output) {}

    fn json(&self) -> serde_json::Value {
        serde_json::json!({ "passed": self.passed })
    }
}

#[allow(clippy::too_many_lines)] // one linear run: resolve, guard, run, promote, report
fn test(ctx: &mut Context, args: &RunArgs) -> CommandResult {
    // Tests resolve their own context (testing overrides, falling back to build).
    let mut resolved = resolve::resolve_testing(ctx)?;

    // Swift packages run tests with the `swift` toolchain — no simulator
    // destination, no `.xcresult` bundle to retain, rerun from, or report on;
    // no `-retry-tests-on-failure` equivalent either.
    if matches!(resolved.container, resolve::Container::SwiftPackage(_)) {
        if args.failed || args.result_bundle.is_some() || args.junit.is_some() {
            return Err(CliError::new(
                "--failed/--result-bundle/--junit need an .xcresult bundle; 'swift test' \
                 produces none for a Swift package",
            ));
        }
        if args.retry_flaky.is_some() {
            return Err(CliError::new(
                "--retry-flaky is xcodebuild's -retry-tests-on-failure; 'swift test' has no \
                 equivalent for a Swift package",
            ));
        }
        return spm_test(ctx, &resolved, args);
    }

    let target = resolve::build_target(ctx, &mut resolved, !args.show_command)?;

    // xcodebuild resolves `-resultBundlePath` against the *container's* parent
    // (its cwd), while the CLI's own exists/summary/rename steps resolve
    // against the CLI's cwd — absolutize so a relative `--result-bundle`
    // means the same directory on both sides.
    let final_bundle = args.result_bundle.map_or_else(
        || retained_bundle_path(&resolved.container),
        |p| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()),
    );

    // `--failed`: the selectors come from the *previous* run's retained
    // bundle, read before anything touches it.
    let only: Vec<String> = if args.failed {
        let selectors = xcodebuild::failed_test_selectors(&final_bundle)?;
        if selectors.is_empty() {
            return Err(CliError::new(
                "the previous run recorded no failures to rerun (or no previous run exists)",
            ));
        }
        selectors
    } else if args.only_testing.is_empty() {
        // A pinned testing target (config `[….testing] target` or `context
        // select target --testing`) narrows the run when nothing explicit is
        // given.
        resolve::testing_target(ctx, &resolved.container)
            .map(|t| vec![t])
            .unwrap_or_default()
    } else {
        args.only_testing.to_vec()
    };

    // The run writes into a scratch sibling; it replaces the retained slot
    // only once it actually holds test results — a rerun that dies in its
    // build step must not destroy the previous run's failure set (`--failed`).
    let run_bundle = final_bundle.with_extension("new.xcresult");

    let plan = xcodebuild::TestPlan {
        container: &resolved.container,
        scheme: &target.scheme,
        configuration: &target.configuration,
        destination: Some(&target.destination),
        sdk: resolved.sdk.as_deref(),
        only_testing: &only,
        skip_testing: args.skip_testing,
        result_bundle: &run_bundle,
        retry_flaky: args.retry_flaky,
        coverage: args.coverage,
        skip_test_diagnostics: xcodebuild::skips_test_diagnostics(
            sweetpad_lib::xcode::active_install().major_version(),
            args.passthrough,
        ),
        passthrough: args.passthrough,
    };

    // A dry run prints and exits before any state or bundle is touched.
    if args.show_command {
        let (command_args, cwd) = plan.command();
        return Ok(Rendered::data(xcodebuild::CommandPreview {
            program: "xcodebuild",
            args: command_args,
            cwd,
        }));
    }
    // Remember the picks — never a `--on`-sourced destination (one-off).
    resolve::remember_testing(ctx, &resolved, &target, ctx.targeting.on.is_none());

    // xcodebuild refuses to overwrite an existing bundle.
    let _ = std::fs::remove_dir_all(&run_bundle);
    if let Some(parent) = run_bundle.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    ctx.out.note(&format!(
        "testing {} for {}",
        target.scheme, target.destination
    ));

    let run_started = SystemTime::now();
    // Human mode beautifies output; JSON stays quiet so stdout holds only the
    // enveloped summary the dispatcher renders from the returned payload.
    let outcome = plan.run(&ctx.out)?;
    let summary = if run_bundle.exists() {
        match xcodebuild::test_summary(&run_bundle) {
            Ok(s) => Some(s),
            // The bundle exists but can't be read (xcresulttool format drift,
            // a transient xcrun failure). For a green run that is *not* "no
            // tests ran" — keep the results and surface the real error.
            Err(e) if outcome.passed => {
                let _ = std::fs::remove_dir_all(&final_bundle);
                let bundle = if std::fs::rename(&run_bundle, &final_bundle).is_ok() {
                    &final_bundle
                } else {
                    &run_bundle
                };
                return Err(e.context(format!(
                    "the tests passed but the result bundle at {} could not be read",
                    bundle.display()
                )));
            }
            Err(_) => None,
        }
    } else {
        None
    };

    // No tests ran: xcodebuild died before/inside its build step (it usually
    // still writes a bundle, so "bundle exists" proves nothing). Surface the
    // real cause instead of a vacuous `0 passed, 0 failed` summary — and keep
    // the previous retained bundle so `--failed` still works.
    // A green xcodebuild exit means the tests ran even if the summary can't be
    // read back (xcresulttool hiccup / older syntax) — misclassifying that as
    // "failed before any test ran" would delete a perfectly good bundle.
    let ran_tests = outcome.passed || summary.as_ref().is_some_and(|s| s.total_test_count > 0);
    if !ran_tests {
        if args.result_bundle.is_some() {
            let _ = std::fs::remove_dir_all(&final_bundle);
            let _ = std::fs::rename(&run_bundle, &final_bundle);
        } else {
            let _ = std::fs::remove_dir_all(&run_bundle);
        }
        let tip = xcodebuild::device_tip(&plan.command().0, &outcome.diagnostics);
        return Err(build_step_failure(&resolved.container, outcome).tip(tip));
    }
    // The scratch run is the project's latest real result: promote it to the
    // retained slot.
    let _ = std::fs::remove_dir_all(&final_bundle);
    let bundle = if std::fs::rename(&run_bundle, &final_bundle).is_ok() {
        final_bundle
    } else {
        run_bundle
    };
    if summary.is_none() {
        ctx.out
            .warn("could not read the result bundle's summary; counts show as 0");
    }
    let summary = summary.unwrap_or_default();
    let passed = outcome.passed;

    if let Some(junit) = args.junit {
        write_junit(junit, &target.scheme, &summary)?;
        ctx.out.note(&format!("junit report: {}", junit.display()));
    }

    let coverage = args
        .coverage
        .then(|| xcodebuild::coverage_percent(&bundle))
        .flatten();

    let terminations = if passed {
        Vec::new()
    } else {
        let run = RunContext {
            resolved: &resolved,
            target: &target,
            passthrough: args.passthrough,
            bundle: &bundle,
            started: run_started,
        };
        terminations(&run, &summary.test_failures)
    };
    let report = TestReport {
        passed,
        summary,
        terminations,
        coverage,
        result_bundle: bundle.display().to_string(),
    };
    if passed {
        Ok(Rendered::data(report))
    } else {
        // A red suite still renders its summary, but exits 3 (build/test failure).
        Ok(Rendered::data_with_exit(report, 3))
    }
}

/// The error for a run that died before any test executed — nearly always a
/// failed compile. The parsed diagnostics are the diagnosis, so they ride in
/// the error object and their first error becomes the message; the transcript
/// is parked in the project's artifact slot and named, rather than quoted in
/// full. A run with nothing parseable (a bad destination, a signing refusal)
/// keeps the tail, which is then the only account of what happened.
fn build_step_failure(container: &Container, outcome: xcodebuild::TestRunOutcome) -> CliError {
    // A blocked build is not a compile failure: no diagnostic describes it and
    // no edit fixes it, so the flag that unblocks it is the whole answer.
    if let Some(hint) = outcome.blocker {
        return CliError::new(format!("the build is blocked, not broken: {hint}"))
            .kind(crate::cli::ErrorKind::BuildFailure)
            .context("running the tests");
    }
    let shown = xcodebuild::streamed_an_error(outcome.streamed, &outcome.diagnostics);
    let log = outcome
        .transcript
        .as_deref()
        .and_then(|text| xcodebuild::record_failure_transcript(container, "-test.log", text));
    let detail = match xcodebuild::diagnostics_summary(&outcome.diagnostics) {
        Some(summary) => {
            let log = log.map_or_else(String::new, |p| format!("; full log: {}", p.display()));
            format!(": {summary}{log}")
        }
        None => outcome
            .tail
            .map_or_else(String::new, |tail| format!(":\n{tail}")),
    };
    let err = CliError::new(format!(
        "xcodebuild test failed before any test ran{detail}"
    ))
    .kind(crate::cli::ErrorKind::BuildFailure)
    .diagnostics(outcome.diagnostics)
    .context("running the tests");
    if shown { err.shown() } else { err }
}

/// What a failure message says vanished mid-test, in the wording Xcode 27
/// uses for it.
#[derive(Debug, PartialEq, Eq)]
enum Vanished {
    /// The app it names: `<bundle id> crashed`, `Failed to application <bundle
    /// id> is not running`.
    Named(String),
    /// The app under test, when an `… is not running` names no bundle id.
    App,
    /// The process running the tests: `Test crashed with signal kill.` (a UI
    /// test runner), `Crash: <App> at <frame>` (a unit test's host app), `Lost
    /// connection to the test runner`, `… test runner exited …`.
    Runner,
}

fn vanished(message: &str) -> Option<Vanished> {
    let bundle_id = |token: &str| {
        (token.contains('.')
            && token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')))
        .then(|| token.to_string())
    };
    if let Some(named) = message.strip_suffix(" crashed").and_then(bundle_id) {
        return Some(Vanished::Named(named));
    }
    if let Some((head, _)) = message.split_once(" is not running") {
        let named = head.rsplit(' ').next().and_then(bundle_id);
        return Some(named.map_or(Vanished::App, Vanished::Named));
    }
    let lower = message.to_lowercase();
    let runner = message.starts_with("Crash: ")
        || lower.contains("crashed")
        || lower.contains("lost connection to the test runner")
        || lower.contains("test runner exited");
    runner.then_some(Vanished::Runner)
}

/// The first of a failure's messages that says something vanished, and what.
/// It need not be the message the summary gave: an assertion that failed
/// before the app crashed is recorded first.
fn vanishing(failure: &xcodebuild::TestFailure) -> Option<(Vanished, &str)> {
    failure
        .messages()
        .find_map(|message| vanished(message).map(|cause| (cause, message)))
}

/// The run [`terminations`] searches: what was tested, where the result bundle
/// is, and when xcodebuild started.
struct RunContext<'a> {
    resolved: &'a resolve::Resolved,
    target: &'a resolve::BuildTarget,
    passthrough: &'a [String],
    bundle: &'a Path,
    started: SystemTime,
}

/// How far before the run's start the exit query reaches, so a skew between
/// launchd's clock and ours cannot cut off an early exit.
const EXIT_QUERY_LEAD: f64 = 2.0;

/// How long after a failure the exit behind it may be logged. launchd writes
/// the line when it reaps the process, which can trail the moment the test
/// noticed. XCTest restarts a crashed unit-test host, and in a measured run the
/// restart's clean exit landed 1.5s after the failure, so the margin stays well
/// under that.
const EXIT_AFTER_FAILURE: f64 = 0.5;

/// The longest the exit query may take before the report goes out without
/// termination reasons.
const EXIT_QUERY_TIMEOUT: Duration = Duration::from_secs(15);

/// At most this many failures are timed, since each costs an `xcresulttool`
/// read of the test's activity log.
const MAX_TIMED_FAILURES: usize = 20;

/// For each failure with a message that says the app or the test runner
/// vanished, the exit launchd logged for it: the last exit of that process
/// between the test's start and just after that message was recorded. Best
/// effort and bounded: an unreadable log, an unsupported destination, or a
/// failure with no activity log leaves that failure without one.
fn terminations(
    run: &RunContext,
    failures: &[xcodebuild::TestFailure],
) -> Vec<Option<Termination>> {
    let causes: Vec<Option<(Vanished, &str)>> = failures.iter().map(vanishing).collect();
    let mut found = Vec::new();
    if causes.iter().any(Option::is_some) {
        found = exits_during(run).unwrap_or_default();
    }
    if found.is_empty() {
        return failures.iter().map(|_| None).collect();
    }
    let run_start = epoch_seconds(run.started) - EXIT_QUERY_LEAD;
    // Resolved on first need: only a failure that names no bundle id needs it.
    let mut app_id: Option<Option<String>> = None;
    let mut app_id = || app_id.get_or_insert_with(|| app_bundle_id(run)).clone();
    let mut budget = MAX_TIMED_FAILURES;
    causes
        .into_iter()
        .zip(failures)
        .map(|(cause, failure)| {
            let (cause, message) = cause?;
            budget = budget.checked_sub(1)?;
            let times =
                xcodebuild::failure_times(run.bundle, &failure.test_identifier_string, message)?;
            let from = times.started.map_or(run_start, |t| t - 1.0);
            let until = times.failed + EXIT_AFTER_FAILURE;
            let last_of = |matches: &dyn Fn(&exits::Exit) -> bool| {
                found
                    .iter()
                    .filter(|e| e.epoch_seconds().is_some_and(|t| t >= from && t <= until))
                    .rfind(|e| matches(e))
                    .cloned()
            };
            let exit = match cause {
                Vanished::Named(id) => last_of(&|e| e.bundle_id == id),
                Vanished::App => app_id().and_then(|id| last_of(&|e| e.bundle_id == id)),
                // A UI test's runner is its own `.xctrunner` app; a unit
                // test's is the host app, so that is the fallback.
                Vanished::Runner => last_of(&|e| e.bundle_id.ends_with(".xctrunner"))
                    .or_else(|| app_id().and_then(|id| last_of(&|e| e.bundle_id == id))),
            }?;
            let crash_report = exit
                .is_crash()
                .then(|| exits::crash_report(&exit, Some(run.started)))
                .flatten();
            Some(Termination { exit, crash_report })
        })
        .collect()
}

/// Every app exit launchd logged on the test destination since the run
/// started. `None` for a destination with no exit records to read (a device)
/// or a query that failed.
fn exits_during(run: &RunContext) -> Option<Vec<exits::Exit>> {
    let destination = &run.target.destination;
    let field = |key: &str| {
        destination
            .split(',')
            .find_map(|kv| kv.trim().strip_prefix(key))
            .map(str::to_string)
    };
    let platform = field("platform=")?;
    let udid;
    let source = if platform == "macOS" {
        exits::Source::Mac
    } else if platform.ends_with(" Simulator") {
        udid = if let Some(id) = field("id=") {
            id
        } else {
            let name = field("name=")?;
            let sims = simctl::list().ok()?;
            simctl::find(&sims, &name)?.udid.clone()
        };
        exits::Source::Simulator(&udid)
    } else {
        return None;
    };
    let window = exits::Window::Between {
        start: epoch_seconds(run.started) - EXIT_QUERY_LEAD,
        end: epoch_seconds(SystemTime::now()) + 1.0,
    };
    exits::query(&source, &[], &window, EXIT_QUERY_TIMEOUT).ok()
}

/// The bundle id of the app the scheme builds, through the in-process
/// build-settings resolver (no xcodebuild spawn).
fn app_bundle_id(run: &RunContext) -> Option<String> {
    let plan = xcodebuild::BuildPlan {
        container: &run.resolved.container,
        scheme: &run.target.scheme,
        configuration: &run.target.configuration,
        destination: Some(&run.target.destination),
        sdk: run.resolved.sdk.as_deref(),
        clean: false,
        hot: false,
        hot_entitlements: None,
        result_bundle: None,
        passthrough: run.passthrough,
        action: xcodebuild::BuildAction::Build,
    };
    let settings = xcodebuild::resolved_settings(&plan).ok()?;
    xcodebuild::app_bundle(&settings, Some(&run.target.destination))
        .ok()
        .map(|app| app.bundle_id)
}

fn epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Where a project's latest `.xcresult` is retained: one slot per project in
/// the state dir (stem + key hash, so two `App.xcodeproj`s never share it).
fn retained_bundle_path(container: &Container) -> PathBuf {
    xcodebuild::project_artifact(container, ".xcresult")
}

/// Where an export lands by default: a directory beside the retained bundle,
/// so `test attachments` works with no arguments and writes nothing into the
/// working directory.
fn export_dir_path(container: &Container) -> PathBuf {
    xcodebuild::project_artifact(container, "-attachments")
}

/// What `test attachments` wrote: the files, grouped by the test that recorded
/// them and ordered as that test recorded them.
struct AttachmentsReport {
    output_dir: PathBuf,
    tests: Vec<TestAttachments>,
    /// When the source run was recorded, as seconds since the epoch. A
    /// screenshot looks authoritative whatever its age, so the age of the
    /// evidence travels with it.
    recorded_at: Option<f64>,
    /// Why an empty export is empty — the answer is never obvious.
    note: Option<String>,
}

struct TestAttachments {
    /// As the manifest names it (`Class/method()`).
    test: String,
    /// As `-only-testing` takes it (`Target/Class/method`).
    identifier: String,
    files: Vec<ExportedFile>,
}

struct ExportedFile {
    name: String,
    path: PathBuf,
    failure: bool,
    timestamp: f64,
}

impl AttachmentsReport {
    fn count(&self) -> usize {
        self.tests.iter().map(|t| t.files.len()).sum()
    }
}

impl Render for AttachmentsReport {
    fn human(&self, out: &Output) {
        let count = self.count();
        let age = self
            .recorded_at
            .and_then(age_phrase)
            .map_or_else(String::new, |age| format!(" (recorded {age} ago)"));
        out.line(&format!(
            "{count} attachment{} from {} test{}{age}",
            if count == 1 { "" } else { "s" },
            self.tests.len(),
            if self.tests.len() == 1 { "" } else { "s" }
        ));
        for test in &self.tests {
            out.line(&format!("  {}", test.identifier));
            for file in &test.files {
                out.line(&format!(
                    "    {}{}",
                    file.name,
                    if file.failure { "  (failure)" } else { "" }
                ));
            }
        }
        if count > 0 {
            out.note(&format!("written to {}", self.output_dir.display()));
        }
        if let Some(note) = &self.note {
            out.note(note);
        }
    }

    fn json(&self) -> serde_json::Value {
        let tests: Vec<serde_json::Value> = self
            .tests
            .iter()
            .map(|t| {
                let files: Vec<serde_json::Value> = t
                    .files
                    .iter()
                    .map(|f| {
                        serde_json::json!({
                            "name": f.name,
                            "path": f.path.display().to_string(),
                            "failure": f.failure,
                            "timestamp": f.timestamp,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "test": t.test,
                    "identifier": t.identifier,
                    "attachments": files,
                })
            })
            .collect();
        serde_json::json!({
            "outputDir": self.output_dir.display().to_string(),
            "count": self.count(),
            "recordedAt": self.recorded_at,
            "tests": tests,
            "note": self.note,
        })
    }
}

/// `test attachments`: export what the last run's tests attached — screenshots,
/// UI-hierarchy dumps, generated fixtures — out of the `.xcresult` and into
/// files a reader can open. The bundle stores them under UUID filenames, so
/// the export is joined against the manifest and renamed back to what each
/// test called the file.
fn attachments(ctx: &mut Context, args: &TestArgs, opts: &AttachmentsArgs) -> CommandResult {
    let (container, bundle) = last_run_bundle(ctx, args)?;

    let output_dir = opts.output_dir.clone().unwrap_or_else(|| {
        let ours = export_dir_path(&container);
        // Our own slot holds one export at a time, so a stale file from a
        // previous run can never be mistaken for this one. A directory the
        // caller named is theirs, and is added to rather than emptied.
        let _ = std::fs::remove_dir_all(&ours);
        ours
    });

    // xcresulttool exports under UUID names and *duplicates* into a populated
    // directory (`name (1).png`), so it always gets a fresh directory of its
    // own; the files are renamed out of it afterwards.
    let staging = output_dir.join(".sweetpad-export");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .map_err(|e| CliError::new(format!("failed to create {}: {e}", staging.display())))?;

    let exported = xcodebuild::export_attachments(&bundle, &staging, opts.only_failures);
    let mut exported = match exported {
        Ok(list) => list,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    let found_any = !exported.is_empty();
    if !args.only_testing.is_empty() {
        // Read off the full set before filtering: what a failed match needs to
        // report is already here, and re-exporting to recover it would both
        // cost a second xcresulttool run and strand its files on the way out.
        let tests = tests_with_attachments(&exported);
        exported.retain(|a| {
            args.only_testing
                .iter()
                .any(|sel| selector_matches(sel, &a.identifier) || selector_matches(sel, &a.test))
        });
        if exported.is_empty() && found_any {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(no_selector_match(&tests, &args.only_testing));
        }
    }

    // The run order is the timestamp order: the manifest lists a test's
    // attachments in neither the order they were taken nor a stable one.
    exported.sort_by(|a, b| {
        a.identifier
            .cmp(&b.identifier)
            .then(a.timestamp.total_cmp(&b.timestamp))
    });

    let renamed = rename_into_place(exported, &output_dir);
    // Every path out of the rename drops the staging directory. A failure
    // partway through would otherwise strand it — half-emptied, and inside a
    // directory the caller named and expects to hold only their files.
    let _ = std::fs::remove_dir_all(&staging);
    let mut tests = renamed?;

    // Tests read in the order the run executed them, which their first
    // attachment dates — a suite that numbers its screenshots across tests
    // reads as one sequence again.
    tests.sort_by(|a, b| {
        let stamp = |t: &TestAttachments| t.files.first().map_or(f64::MAX, |f| f.timestamp);
        stamp(a).total_cmp(&stamp(b))
    });

    let note = (!found_any).then(|| empty_note(opts.only_failures));
    Ok(Rendered::data(AttachmentsReport {
        output_dir,
        tests,
        recorded_at: recorded_at(&bundle),
        note,
    }))
}

/// Move each exported file out of the staging directory into its own test's
/// directory under `output_dir`, renamed from the UUID the export gave it back
/// to what the test called it. Grouping relies on the caller having sorted by
/// test, so one test's files land in one entry.
fn rename_into_place(
    exported: Vec<xcodebuild::ExportedAttachment>,
    output_dir: &Path,
) -> Result<Vec<TestAttachments>, CliError> {
    let mut tests: Vec<TestAttachments> = Vec::new();
    for item in exported {
        let dir = output_dir.join(test_dir_name(&item.identifier));
        std::fs::create_dir_all(&dir)
            .map_err(|e| CliError::new(format!("failed to create {}: {e}", dir.display())))?;
        let path = unique_path(&dir, &clean_name(&item.suggested_name));
        std::fs::rename(&item.file, &path).map_err(|e| {
            CliError::new(format!(
                "failed to move the exported attachment to {}: {e}",
                path.display()
            ))
        })?;
        let file = ExportedFile {
            name: path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            path,
            failure: item.failure,
            timestamp: item.timestamp,
        };
        match tests.last_mut() {
            Some(last) if last.identifier == item.identifier => last.files.push(file),
            _ => tests.push(TestAttachments {
                test: item.test,
                identifier: item.identifier,
                files: vec![file],
            }),
        }
    }
    Ok(tests)
}

/// The `.xcresult` the read-back verbs work from: whichever bundle the last
/// run left behind, or the one `--result-bundle` names.
fn last_run_bundle(ctx: &Context, args: &TestArgs) -> Result<(Container, PathBuf), CliError> {
    let container = resolve::container(ctx)?;
    let bundle = args.result_bundle.as_deref().map_or_else(
        || retained_bundle_path(&container),
        |p| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()),
    );
    if !bundle.exists() {
        return Err(CliError::new(format!(
            "no result bundle at {} — run 'sweetpad test' first, or name one with \
             '--result-bundle PATH'",
            bundle.display()
        )));
    }
    Ok((container, bundle))
}

/// How much of one test's output rides in the report before it is cut back to
/// its tail. Sized from what tests actually write: a unit test's `print`
/// output runs to hundreds of bytes, while a UI test's automation trace runs
/// to tens of KB and would swamp both a terminal and a JSON payload.
const OUTPUT_CAP: usize = 4096;

/// What `test output` recovered: the console output of each test that wrote
/// any, plus anything written outside a test.
struct OutputReport {
    tests: Vec<TestOutputEntry>,
    unattributed: Option<String>,
    sources: Vec<PathBuf>,
    recorded_at: Option<f64>,
    /// Attribution brackets a test's output between its own markers, which
    /// only holds while one test runs at a time.
    serial: bool,
    note: Option<String>,
}

struct TestOutputEntry {
    test: String,
    identifier: String,
    output: String,
    truncated: bool,
}

impl Render for OutputReport {
    fn human(&self, out: &Output) {
        for entry in &self.tests {
            out.line(&entry.identifier);
            if entry.truncated {
                out.line("    …");
            }
            for line in entry.output.lines() {
                out.line(&format!("    {line}"));
            }
        }
        if let Some(text) = &self.unattributed {
            out.line("outside any test");
            for line in text.lines() {
                out.line(&format!("    {line}"));
            }
        }
        let age = self
            .recorded_at
            .and_then(age_phrase)
            .map_or_else(String::new, |age| format!(" (recorded {age} ago)"));
        out.line(&format!(
            "{} test{} wrote output{age}",
            self.tests.len(),
            if self.tests.len() == 1 { "" } else { "s" }
        ));
        if self.tests.iter().any(|t| t.truncated) {
            let where_ = self
                .sources
                .first()
                .and_then(|p| p.parent())
                .map_or_else(String::new, |d| format!(", or read {}", d.display()));
            out.note(&format!(
                "output was cut to its last {OUTPUT_CAP} bytes; pass '--full' for all of it{where_}"
            ));
        }
        if !self.serial {
            out.warn(
                "the run did not report itself as serial — output is attributed by the \
                 markers around each test, which parallel workers interleave",
            );
        }
        if let Some(note) = &self.note {
            out.note(note);
        }
    }

    fn json(&self) -> serde_json::Value {
        let tests: Vec<serde_json::Value> = self
            .tests
            .iter()
            .map(|t| {
                serde_json::json!({
                    "test": t.test,
                    "identifier": t.identifier,
                    "output": t.output,
                    "truncated": t.truncated,
                })
            })
            .collect();
        serde_json::json!({
            "tests": tests,
            "unattributed": self.unattributed,
            "sources": self.sources.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "recordedAt": self.recorded_at,
            "serial": self.serial,
            "note": self.note,
        })
    }
}

/// `test output`: what the last run's tests printed. A passing test's own
/// output — a benchmark's timing, a generated fixture's size — is the product
/// of the test rather than an aside, and the pass/fail summary says nothing
/// about it. The bundle keeps it per test *process*, so it is sliced back to
/// per test here.
fn output(ctx: &mut Context, args: &TestArgs, opts: &OutputArgs) -> CommandResult {
    let (container, bundle) = last_run_bundle(ctx, args)?;

    // The diagnostics export is all-or-nothing and writes the app-under-test's
    // log alongside the streams worth reading, so it lands in a scratch
    // directory that is removed once the few KB that matter are parsed.
    let staging = xcodebuild::project_artifact(&container, "-output.staging");
    let kept = xcodebuild::project_artifact(&container, "-output");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .map_err(|e| CliError::new(format!("failed to create {}: {e}", staging.display())))?;
    let run = xcodebuild::export_run_output(&bundle, &staging, &kept);
    let _ = std::fs::remove_dir_all(&staging);
    let run = run?;

    let found_any = !run.tests.is_empty();
    let mut wrote: Vec<String> = run.tests.iter().map(|t| t.identifier.clone()).collect();
    let tests = select_output(run.tests, &args.only_testing, opts.full);

    if tests.is_empty() && found_any && !args.only_testing.is_empty() {
        wrote.sort();
        wrote.dedup();
        return Err(no_output_match(&wrote, &args.only_testing));
    }

    let note = (!found_any).then(|| {
        "no test wrote to stdout or stderr; a test's own 'print' lands here, while \
         XCTest's assertions and a UI test's screenshots do not"
            .to_string()
    });
    // Output written outside a test case is XCTest's own bookkeeping and the
    // app's `os_log` chatter — noise, next to the lines a test meant to write.
    // It earns a place only when nothing was attributed at all, which is what
    // a framework this parser does not recognise looks like: then it is the
    // only account of what ran, and dropping it would lose the run entirely.
    let unattributed = tests
        .is_empty()
        .then(|| run.unattributed.trim().to_string())
        .filter(|s| !s.is_empty());
    Ok(Rendered::data(OutputReport {
        tests,
        unattributed,
        sources: run.sources,
        recorded_at: recorded_at(&bundle),
        serial: run.serial,
        note,
    }))
}

/// The tests the report shows: those `only_testing` selects that wrote
/// anything, each cut back to its tail unless `full`.
///
/// Whether a test wrote anything is decided on its whole output, before the cap
/// is applied, so cutting can only shorten an entry and never drop the test.
fn select_output(
    tests: Vec<xcodebuild::TestOutput>,
    only_testing: &[String],
    full: bool,
) -> Vec<TestOutputEntry> {
    tests
        .into_iter()
        .filter(|t| {
            only_testing.is_empty()
                || only_testing.iter().any(|sel| {
                    selector_matches(sel, &t.identifier) || selector_matches(sel, &t.test)
                })
        })
        .filter(|t| !t.output.trim().is_empty())
        .map(|t| {
            // Trimmed before the cut so the kept tail ends on a real line: a
            // tail made of the run's trailing blank lines would render as an
            // entry with nothing under it.
            let source = t.output.trim_end();
            let (output, truncated) = if full {
                (source.to_string(), false)
            } else {
                cut_to_tail(source, OUTPUT_CAP)
            };
            TestOutputEntry {
                test: t.test,
                identifier: t.identifier,
                output,
                truncated,
            }
        })
        .collect()
}

/// Cut `text` back to its last `cap` bytes on a line boundary. The tail is
/// kept because output that ran long is nearly always a log, and a log's end
/// is where the thing being diagnosed happened.
fn cut_to_tail(text: &str, cap: usize) -> (String, bool) {
    if text.len() <= cap {
        return (text.to_string(), false);
    }
    // The cap counts bytes, so it can land inside a character; walk forward to
    // a boundary before slicing.
    let mut start = text.len() - cap;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    // Then forward again to the next line break, so the cut never leaves half
    // a line — but only while that leaves something. A test that printed one
    // long line (a JSON blob, a serialized payload) has no break left in its
    // tail, and advancing past the end there would report the test as silent.
    if let Some(i) = text[start..].find('\n')
        && start + i + 1 < text.len()
    {
        start += i + 1;
    }
    (text[start..].to_string(), true)
}

/// Why an export came back empty. A run that attached nothing looks identical
/// to one whose attachments were discarded, and the discard is the default:
/// `XCTAttachment.lifetime` is `.deleteOnSuccess` unless a test says otherwise.
fn empty_note(only_failures: bool) -> String {
    if only_failures {
        "no attachments were recorded against a failure (the run may have passed); drop \
         '--only-failures' to export everything the run attached"
            .to_string()
    } else {
        "the run attached nothing that survived: XCTAttachment.lifetime defaults to \
         .deleteOnSuccess, so a passing test's attachments are discarded — set \
         'attachment.lifetime = .keepAlways' to keep them"
            .to_string()
    }
}

/// The distinct tests an export covers, which is what a failed `--only-testing`
/// match needs to name.
fn tests_with_attachments(exported: &[xcodebuild::ExportedAttachment]) -> Vec<String> {
    let mut tests: Vec<String> = exported.iter().map(|a| a.identifier.clone()).collect();
    tests.sort();
    tests.dedup();
    tests
}

/// `--only-testing` matched none of the tests that attached anything, so those
/// are named instead, in the form the selector takes.
fn no_selector_match(tests: &[String], selectors: &[String]) -> CliError {
    let known = match tests.len() {
        n if n > 5 => format!("{}, and {} more", tests[..5].join(", "), n - 5),
        _ => tests.join(", "),
    };
    CliError::new(format!(
        "no attachments matched {}; tests with attachments: {known}",
        selectors.join(", ")
    ))
}

/// `--only-testing` matched none of the tests that wrote output, so those are
/// named instead, in the form the selector takes.
fn no_output_match(wrote: &[String], selectors: &[String]) -> CliError {
    let known = match wrote.len() {
        n if n > 5 => format!("{}, and {} more", wrote[..5].join(", "), n - 5),
        _ => wrote.join(", "),
    };
    CliError::new(format!(
        "no output from {}; tests that wrote output: {known}",
        selectors.join(", ")
    ))
}

/// Match an `--only-testing` selector against the manifest's test identifier.
/// The manifest identifies a test as `Class/method`, while a selector may
/// carry the leading target (`Target/Class/method`) that `-only-testing` takes,
/// so a selector also matches with its first component dropped.
fn selector_matches(selector: &str, identifier: &str) -> bool {
    let id = identifier.trim_end_matches("()");
    let sel = selector.trim_end_matches("()");
    let hit = |s: &str| !s.is_empty() && (id == s || id.starts_with(&format!("{s}/")));
    hit(sel) || sel.split_once('/').is_some_and(|(_, rest)| hit(rest))
}

/// The directory one test's attachments land in: its identifier as a single
/// path component, with a trailing `()` trimmed.
fn test_dir_name(identifier: &str) -> String {
    let name: String = identifier
        .trim_end_matches("()")
        .chars()
        .map(|c| if c == '/' || c == '\\' { '.' } else { c })
        .collect();
    let name = name.trim_start_matches('.').trim();
    if name.is_empty() {
        "unknown-test".to_string()
    } else {
        name.to_string()
    }
}

/// Strip the `_<run>_<UUID>` uniquifier XCTest appends to an attachment's own
/// name, recovering what the test called the file. A name without that shape
/// is kept as it is.
fn clean_name(suggested: &str) -> String {
    let path = Path::new(suggested);
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return suggested.to_string();
    };
    let mut parts: Vec<&str> = stem.split('_').collect();
    if parts.len() >= 2 && parts.last().is_some_and(|p| is_uuid(p)) {
        parts.pop();
        // The index between the name and the UUID is XCTest's, not the test's.
        if parts.len() >= 2
            && parts
                .last()
                .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        {
            parts.pop();
        }
    }
    let stem = if parts.is_empty() {
        stem.to_string()
    } else {
        parts.join("_")
    };
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{stem}.{ext}"),
        None => stem,
    }
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

/// A free path in `dir` for `name`. Attachment names are not unique — the
/// uniquifier just stripped from them exists for that reason — so a taken
/// name gets a counter rather than silently overwriting the earlier file.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map_or_else(String::new, |e| format!(".{e}"));
    // Bounded: a name colliding this many times is a runaway, and looping
    // forever over a full directory would be a worse answer than overwriting.
    (2..10_000)
        .map(|n| dir.join(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(candidate)
}

/// When the run behind `bundle` was recorded, in seconds since the epoch.
fn recorded_at(bundle: &Path) -> Option<f64> {
    std::fs::metadata(bundle)
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs_f64())
}

/// How long ago `stamp` was, coarsely — enough to notice that the evidence
/// predates the change being checked. `None` when it isn't in the past.
fn age_phrase(stamp: f64) -> Option<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    let secs = now - stamp;
    if secs < 60.0 {
        return (secs >= 0.0).then(|| "less than a minute".to_string());
    }
    let (value, unit) = if secs < 3600.0 {
        (secs / 60.0, "minute")
    } else if secs < 86_400.0 {
        (secs / 3600.0, "hour")
    } else {
        (secs / 86_400.0, "day")
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // coarse by design
    let value = value as u64;
    Some(format!(
        "{value} {unit}{}",
        if value == 1 { "" } else { "s" }
    ))
}

/// Write a minimal JUnit XML report from the parsed summary: totals on the
/// suite, one `<testcase>` per recorded failure (the summary carries failures
/// individually and the rest as counts).
fn write_junit(
    path: &Path,
    scheme: &str,
    summary: &xcodebuild::TestSummary,
) -> Result<(), CliError> {
    use std::fmt::Write as _;
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        xml,
        "<testsuites tests=\"{}\" failures=\"{}\" skipped=\"{}\">",
        summary.total_test_count, summary.failed_tests, summary.skipped_tests
    );
    let _ = writeln!(
        xml,
        "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" skipped=\"{}\">",
        xml_escape(scheme),
        summary.total_test_count,
        summary.failed_tests,
        summary.skipped_tests
    );
    for f in &summary.test_failures {
        let selector = f.selector();
        let (classname, name) = junit_names(&selector, scheme);
        let _ = writeln!(
            xml,
            "    <testcase classname=\"{}\" name=\"{}\">\n      <failure message=\"{}\"/>\n    </testcase>",
            xml_escape(&classname),
            xml_escape(name),
            xml_escape(&f.failure_text)
        );
    }
    xml.push_str("  </testsuite>\n</testsuites>\n");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, xml)
        .map_err(|e| CliError::new(format!("failed to write {}: {e}", path.display())))
}

/// A test's JUnit `classname` and `name`, cut from its `-only-testing`
/// selector at the last `/`: `Target.Class` and `method`, or the target alone
/// for a Swift Testing function outside any suite. Report viewers read
/// `classname` as a dotted `package.Class` (Jenkins groups by the part before
/// the last dot), so each target groups its own classes. A selector with no
/// `/` in it takes the scheme as its class.
fn junit_names<'a>(selector: &'a str, scheme: &str) -> (String, &'a str) {
    match selector.rsplit_once('/') {
        Some((class, name)) => (class.replace('/', "."), name),
        None => (scheme.to_string(), selector),
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Run a Swift package's tests via `swift test`. Unlike xcodebuild there's no
/// `.xcresult` to parse, so the result is just the pass/fail flag. Honors the
/// dry run (`--show-command` previews instead of executing), `--coverage`
/// (`--enable-code-coverage`), and `--` passthrough.
fn spm_test(ctx: &mut Context, resolved: &resolve::Resolved, args: &RunArgs) -> CommandResult {
    let configuration = resolved
        .configuration
        .clone()
        .unwrap_or_else(|| "Debug".to_string());

    if args.show_command {
        return Ok(Rendered::data(xcodebuild::CommandPreview {
            program: "swift",
            args: swiftpm::test_args(
                &configuration,
                args.only_testing,
                args.skip_testing,
                args.coverage,
                args.passthrough,
            ),
            cwd: swiftpm::package_dir(&resolved.container),
        }));
    }

    ctx.out.note(&format!(
        "testing Swift package ({configuration}) with swift test"
    ));

    let passed = swiftpm::test(
        &resolved.container,
        &configuration,
        args.only_testing,
        args.skip_testing,
        args.coverage,
        ctx.out.is_json() || ctx.out.is_ndjson(),
        args.passthrough,
    )?;

    let report = SpmTestReport { passed };
    if passed {
        Ok(Rendered::data(report))
    } else {
        Ok(Rendered::data_with_exit(report, 3))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use std::path::PathBuf;

    /// Parse a `sweetpad test …` command line down to its resource flags and
    /// action.
    fn parse_test(argv: &[&str]) -> (TestArgs, Option<Action>) {
        use clap::Parser;
        let cli = crate::cli::Cli::try_parse_from(["sweetpad", "test"].iter().chain(argv))
            .unwrap_or_else(|e| panic!("`test {}` rejected: {e}", argv.join(" ")));
        match cli.resource {
            Some(crate::cli::Resource::Test { args, action }) => (args, action),
            other => panic!("parsed as {other:?}"),
        }
    }

    #[test]
    fn test_build_is_a_verb_that_takes_the_build_flags() {
        // The target selection, the dry run, `--watch`, and the `--` tail all
        // parse, on either side of the verb, and none of them reads as a run
        // flag.
        let (args, action) = parse_test(&[
            "--scheme",
            "App",
            "build",
            "--on",
            "booted",
            "--show-command",
            "--",
            "-skipMacroValidation",
        ]);
        assert!(matches!(action, Some(Action::Build(_))));
        assert!(args.show_command);
        assert_eq!(args.passthrough, ["-skipMacroValidation"]);
        assert!(run_only_flags(&args).is_empty());

        let (args, _) = parse_test(&["build", "--watch"]);
        assert!(args.watch);
        assert!(run_only_flags(&args).is_empty());
    }

    #[test]
    fn test_build_refuses_the_flags_that_only_shape_a_run() {
        // They are resource-global, so they parse after `build`; each one has
        // to be named rather than dropped. `--only-testing` especially, since
        // it reads like it would narrow the build and cannot.
        let (args, _) = parse_test(&["build", "--only-testing", "AppTests"]);
        assert_eq!(run_only_flags(&args), ["--only-testing"]);

        let (args, _) = parse_test(&[
            "build",
            "--skip-testing",
            "AppUITests",
            "--failed",
            "--result-bundle",
            "r.xcresult",
            "--junit",
            "j.xml",
            "--retry-flaky",
            "2",
            "--coverage",
        ]);
        assert_eq!(
            run_only_flags(&args),
            [
                "--skip-testing",
                "--failed",
                "--result-bundle",
                "--junit",
                "--retry-flaky",
                "--coverage"
            ]
        );
    }

    #[test]
    fn the_read_back_verbs_refuse_the_flags_that_only_shape_a_run() {
        // Hidden on these verbs, but they still parse, on either side of the
        // verb, and land on the resource's args to be named in the refusal.
        for verb in ["attachments", "output"] {
            let (args, _) = parse_test(&[verb, "--failed", "--junit", "j.xml"]);
            assert_eq!(read_refused_flags(&args), ["--failed", "--junit"], "{verb}");
            let (args, _) = parse_test(&["--coverage", verb, "--", "-quiet"]);
            assert_eq!(
                read_refused_flags(&args),
                ["--coverage", "'-- XCODEBUILD_ARGS'"],
                "{verb}"
            );
            let (args, _) = parse_test(&[
                verb,
                "--skip-testing",
                "A",
                "--watch",
                "--retry-flaky",
                "2",
                "--show-command",
            ]);
            assert_eq!(
                read_refused_flags(&args),
                [
                    "--skip-testing",
                    "--watch",
                    "--retry-flaky",
                    "--show-command"
                ],
                "{verb}"
            );
        }
        let (args, _) = parse_test(&["output", "--failed"]);
        let err = refuse_run_flags(&read_refused_flags(&args), &read_reason("output"))
            .expect_err("--failed was not refused");
        assert_eq!(
            err.to_string(),
            "--failed applies to a test run: 'test output' reads the last run's result bundle \
             and runs nothing"
        );
    }

    #[test]
    fn the_read_back_verbs_take_the_tests_and_the_bundle_to_read() {
        // Declared on each verb, and still read off the resource's args, from
        // either side of the verb.
        for verb in ["attachments", "output"] {
            for argv in [
                [
                    verb,
                    "--only-testing",
                    "AppTests",
                    "--result-bundle",
                    "r.xcresult",
                ],
                [
                    "--only-testing",
                    "AppTests",
                    "--result-bundle",
                    "r.xcresult",
                    verb,
                ],
            ] {
                let (args, _) = parse_test(&argv);
                assert_eq!(args.only_testing, ["AppTests"], "{argv:?}");
                assert_eq!(
                    args.result_bundle.as_deref(),
                    Some(Path::new("r.xcresult")),
                    "{argv:?}"
                );
                assert!(read_refused_flags(&args).is_empty(), "{argv:?}");
            }
        }
        // `test --failed` is still `test run --failed`.
        let (args, action) = parse_test(&["--failed"]);
        assert!(args.failed && action.is_none());
        let (args, action) = parse_test(&["run", "--failed"]);
        assert!(args.failed && matches!(action, Some(Action::Run)));
    }

    #[test]
    fn a_test_build_writes_the_build_slot_not_the_retained_run() {
        // `test build` hands its plan the build's result bundle. Were that the
        // retained run's slot, compiling the tests would erase the failures
        // `--failed`, `test output`, and `test attachments` read back.
        let c = Container::Project(PathBuf::from("/work/App.xcodeproj"));
        assert_ne!(
            xcodebuild::build_result_bundle(&c),
            retained_bundle_path(&c)
        );
    }

    #[test]
    fn retained_bundle_paths_are_stable_and_distinct() {
        let a = Container::Project(PathBuf::from("/work/App.xcodeproj"));
        let b = Container::Project(PathBuf::from("/other/App.xcodeproj"));
        let (pa, pb) = (retained_bundle_path(&a), retained_bundle_path(&b));
        // Same container → same slot; same stem elsewhere → a different slot.
        assert_eq!(pa, retained_bundle_path(&a));
        assert_ne!(pa, pb);
        assert!(pa.to_string_lossy().ends_with(".xcresult"));
        assert!(
            pa.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("App-")
        );
    }

    #[test]
    fn a_failed_build_reports_diagnostics_instead_of_the_transcript() {
        // The whole point: an agent reading `-o json` gets the one line that
        // matters plus structured diagnostics, not several KB of swiftc flags.
        let transcript = format!(
            "{}\n/work/App/Picker.swift:4:11: error: cannot find 'Missing' in scope\n{}",
            "CompileSwift normal arm64 -Xcc -I/a/very/long/include/path".repeat(40),
            "** TEST FAILED **"
        );
        let container = Container::Project(PathBuf::from("/work/App.xcodeproj"));
        let outcome = xcodebuild::TestRunOutcome {
            passed: false,
            tail: Some(transcript.clone()),
            diagnostics: crate::cli::buildlog::diagnostics_from_transcript(&transcript),
            transcript: Some(transcript.clone()),
            blocker: None,
            streamed: false,
        };
        let err = build_step_failure(&container, outcome);

        let json = err.json();
        assert_eq!(json["code"], "build_failure");
        let diagnostics = json["diagnostics"].as_array().expect("no diagnostics");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0]["message"], "cannot find 'Missing' in scope");

        // The message names the cause and the parked log, and stays far shorter
        // than the transcript it replaced.
        let message = err.to_string();
        assert!(
            message.contains("cannot find 'Missing' in scope"),
            "{message}"
        );
        assert!(message.contains("full log: "), "{message}");
        assert!(message.len() < transcript.len() / 4, "{message}");

        // The transcript is moved, not dropped.
        let log = message
            .rsplit_once("full log: ")
            .map(|(_, p)| PathBuf::from(p))
            .expect("no log path");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), transcript);
        let _ = std::fs::remove_file(&log);
    }

    #[test]
    fn a_failure_with_nothing_parseable_keeps_the_tail() {
        // No diagnostic means no better account exists — dropping the tail here
        // would leave the caller with a bare "failed before any test ran".
        let container = Container::Project(PathBuf::from("/work/App.xcodeproj"));
        let outcome = xcodebuild::TestRunOutcome {
            passed: false,
            tail: Some("xcodebuild: error: Unable to find a destination".to_string()),
            diagnostics: Vec::new(),
            transcript: None,
            blocker: None,
            streamed: true,
        };
        let err = build_step_failure(&container, outcome);
        assert!(err.to_string().contains("Unable to find a destination"));
        assert!(err.json().get("diagnostics").is_none());
        // Nothing on the stream explained it, so the terminal needs this error.
        assert!(!err.is_shown());
    }

    #[test]
    fn a_streamed_compile_error_is_not_restated_after_the_banner() {
        let container = Container::Project(PathBuf::from("/work/App.xcodeproj"));
        let outcome = |streamed, blocker: Option<&str>| xcodebuild::TestRunOutcome {
            passed: false,
            tail: None,
            diagnostics: crate::cli::buildlog::diagnostics_from_transcript(
                "/work/App/Picker.swift:4:11: error: cannot find 'Missing' in scope\n",
            ),
            transcript: None,
            blocker: blocker.map(str::to_string),
            streamed,
        };
        let err = build_step_failure(&container, outcome(true, None));
        assert!(err.is_shown());
        // The machine-readable object and the exit code still carry it.
        assert_eq!(err.json()["diagnostics"].as_array().map(Vec::len), Some(1));
        assert_eq!(err.error_kind().exit_code(), 3);

        // The captured modes showed nothing, and a blocker's hint says what
        // the stream did not.
        assert!(!build_step_failure(&container, outcome(false, None)).is_shown());
        assert!(!build_step_failure(&container, outcome(true, Some("approve it"))).is_shown());
    }

    #[test]
    fn an_attachment_keeps_the_name_its_test_gave_it() {
        // The whole point of the manifest join: the exported file is a UUID,
        // and only this name tells a reader which screenshot they are looking
        // at. XCTest's `_<run>_<UUID>` uniquifier is not part of that name.
        assert_eq!(
            clean_name("10-scrolled-back_0_AD33EE58-9A7C-47EC-A75E-F9EA6E2D8AFE.png"),
            "10-scrolled-back.png"
        );
        // A name of its own may contain underscores; only the suffix goes.
        assert_eq!(
            clean_name("my_shot_2_7B4971C1-82C2-439E-915F-48E2D15A43BD.png"),
            "my_shot.png"
        );
        // Nothing that isn't the suffix is stripped: a bare name, a name whose
        // trailing part merely looks numeric, and a UUID-shaped name that is
        // all the name there is.
        assert_eq!(clean_name("screenshot.png"), "screenshot.png");
        assert_eq!(clean_name("step_2.png"), "step_2.png");
        assert_eq!(
            clean_name("AD33EE58-9A7C-47EC-A75E-F9EA6E2D8AFE.png"),
            "AD33EE58-9A7C-47EC-A75E-F9EA6E2D8AFE.png"
        );
        // An attachment need not be an image, or have an extension at all.
        assert_eq!(
            clean_name("hierarchy_0_7B4971C1-82C2-439E-915F-48E2D15A43BD.txt"),
            "hierarchy.txt"
        );
        assert_eq!(
            clean_name("dump_0_7B4971C1-82C2-439E-915F-48E2D15A43BD"),
            "dump"
        );
    }

    #[test]
    fn only_a_real_uuid_counts_as_the_uniquifier() {
        assert!(is_uuid("AD33EE58-9A7C-47EC-A75E-F9EA6E2D8AFE"));
        assert!(!is_uuid("AD33EE58-9A7C-47EC-A75E-F9EA6E2D8AF"));
        assert!(!is_uuid("AD33EE58_9A7C_47EC_A75E_F9EA6E2D8AFE"));
        assert!(!is_uuid("ZD33EE58-9A7C-47EC-A75E-F9EA6E2D8AFE"));
    }

    #[test]
    fn a_selector_matches_a_test_the_manifest_names_by_class() {
        // xcresulttool identifies a test as Class/method, while -only-testing
        // takes Target/Class/method — a selector in either shape has to land.
        let id = "ReflowEngineTests/testResolvePages()";
        assert!(selector_matches("ReflowEngineTests", id));
        assert!(selector_matches("ReflowEngineTests/testResolvePages", id));
        assert!(selector_matches(
            "ReflowTests/ReflowEngineTests/testResolvePages",
            id
        ));
        // A different test in the same class, and a bare target name (which
        // the identifier does not carry), must not match — the second is why
        // an empty match reports the classes instead of returning nothing.
        assert!(!selector_matches(
            "ReflowEngineTests/testResolvePagesTwice",
            id
        ));
        assert!(!selector_matches("ReflowTests", id));
        assert!(!selector_matches("", id));
    }

    #[test]
    fn a_test_identifier_becomes_one_directory_component() {
        assert_eq!(
            test_dir_name("ReflowUITests/OpenTests/testOpensAPDF"),
            "ReflowUITests.OpenTests.testOpensAPDF"
        );
        assert_eq!(
            test_dir_name("ReflowTests/PageSuite/reflows()"),
            "ReflowTests.PageSuite.reflows"
        );
        // A separator inside the name must never escape into a nested path,
        // and a name that sanitizes away still needs somewhere to land.
        assert!(!test_dir_name("A/B/c()").contains('/'));
        assert!(!test_dir_name("A\\B").contains('\\'));
        assert_eq!(test_dir_name("()"), "unknown-test");
    }

    #[test]
    fn a_taken_name_never_overwrites_the_earlier_file() {
        // Attachment names are not unique — that is why XCTest appends a
        // uniquifier at all — so two files sharing one name must both survive.
        let dir = std::env::temp_dir().join(format!("sweetpad-att-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_path(&dir, "shot.png");
        assert_eq!(first.file_name().unwrap(), "shot.png");
        std::fs::write(&first, b"a").unwrap();
        let second = unique_path(&dir, "shot.png");
        assert_eq!(second.file_name().unwrap(), "shot-2.png");
        std::fs::write(&second, b"b").unwrap();
        assert_eq!(
            unique_path(&dir, "shot.png").file_name().unwrap(),
            "shot-3.png"
        );
        assert_eq!(std::fs::read(&first).unwrap(), b"a");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn attachments_name_each_test_the_way_a_rerun_takes_it() {
        // The heading is what `--only-testing` takes, and the JSON keeps the
        // manifest's own name under `test` beside it.
        let report = AttachmentsReport {
            output_dir: PathBuf::from("/out"),
            tests: vec![TestAttachments {
                test: "AppTests/testGreeting()".into(),
                identifier: "SweetpadCIAppTests/AppTests/testGreeting".into(),
                files: vec![ExportedFile {
                    name: "greeting-note.txt".into(),
                    path: PathBuf::from(
                        "/out/SweetpadCIAppTests.AppTests.testGreeting/greeting-note.txt",
                    ),
                    failure: false,
                    timestamp: 1.0,
                }],
            }],
            recorded_at: None,
            note: None,
        };
        let json = report.json();
        assert_eq!(json["tests"][0]["test"], "AppTests/testGreeting()");
        assert_eq!(
            json["tests"][0]["identifier"],
            "SweetpadCIAppTests/AppTests/testGreeting"
        );
    }

    #[test]
    fn a_selector_matching_no_attachments_names_the_tests_that_have_some() {
        let message = no_selector_match(
            &["SweetpadCIAppTests/AppTests/testGreeting".to_string()],
            &["SweetpadCIAppUITests".to_string()],
        )
        .to_string();
        assert_eq!(
            message,
            "no attachments matched SweetpadCIAppUITests; tests with attachments: \
             SweetpadCIAppTests/AppTests/testGreeting"
        );
        let many: Vec<String> = (0..8).map(|i| format!("T/C/test{i}")).collect();
        let message = no_selector_match(&many, &["X".to_string()]).to_string();
        assert!(message.contains("and 3 more"), "{message}");
    }

    #[test]
    fn an_empty_export_says_which_emptiness_it_is() {
        // Both cases look identical on disk, and neither is guessable: one is
        // "the run was green", the other is a default that discards evidence.
        let failures = empty_note(true);
        assert!(failures.contains("--only-failures"), "{failures}");
        let all = empty_note(false);
        assert!(all.contains(".keepAlways"), "{all}");
        assert!(all.contains("deleteOnSuccess"), "{all}");
        // Backticks render literally in a terminal.
        assert!(!failures.contains('`'), "{failures}");
        assert!(!all.contains('`'), "{all}");
    }

    #[test]
    fn long_output_is_cut_to_whole_lines_from_the_end() {
        // A log's end is where the thing being diagnosed happened, so the tail
        // is what survives — and it survives as whole lines.
        let text: String = (0..500).fold(String::new(), |mut s, i| {
            let _ = writeln!(s, "line {i}");
            s
        });
        let (cut, truncated) = cut_to_tail(&text, 100);
        assert!(truncated);
        assert!(cut.len() <= 100, "{}", cut.len());
        assert!(cut.ends_with("line 499\n"), "{cut}");
        assert!(cut.starts_with("line "), "{cut}");
        assert!(!cut.contains("line 0\n"));

        // Output that fits is returned whole and unmarked.
        let (whole, truncated) = cut_to_tail("short\n", 100);
        assert_eq!(whole, "short\n");
        assert!(!truncated);
    }

    #[test]
    fn a_rename_that_fails_partway_leaves_the_staging_directory_to_the_caller() {
        // The staging directory lives *inside* the caller's --output-dir, so a
        // failure that strands it leaves our scratch dir sitting in a directory
        // they expect to hold only their files. The cleanup has to sit on the
        // error path too, which is why the rename is its own function.
        let root = std::env::temp_dir().join(format!("sweetpad-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let staging = root.join(".sweetpad-export");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("aaaa"), "shot").unwrap();

        // The second attachment names a file that was never exported, so its
        // rename fails and the loop gives up with one file already moved.
        let exported = vec![
            xcodebuild::ExportedAttachment {
                test: "ATests/testOne()".into(),
                identifier: "AppTests/ATests/testOne".into(),
                file: staging.join("aaaa"),
                suggested_name: "shot.png".into(),
                failure: false,
                timestamp: 1.0,
            },
            xcodebuild::ExportedAttachment {
                test: "ATests/testTwo()".into(),
                identifier: "AppTests/ATests/testTwo".into(),
                file: staging.join("missing"),
                suggested_name: "gone.png".into(),
                failure: false,
                timestamp: 2.0,
            },
        ];
        let Err(err) = rename_into_place(exported, &root) else {
            panic!("renaming a file that was never exported should have failed")
        };
        assert!(err.to_string().contains("gone.png"), "{err}");
        // The contract the caller relies on: the failure comes back as a value,
        // so control reaches the one `remove_dir_all` that runs on every path.
        // Returning it out of the middle of `attachments` was what stranded the
        // directory — the cleanup sat below the `?`.
        assert!(staging.exists(), "the caller was given nothing to clean up");
        // The file that did move is where it was put, under its test's name.
        assert!(
            root.join("AppTests.ATests.testOne")
                .join("shot.png")
                .exists()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// [`select_output`] over `(Target/Class/method, output)` pairs, which is
    /// how a stream reads once it has been sliced per test.
    fn output_entries(
        tests: Vec<(&str, &str)>,
        only_testing: &[String],
        full: bool,
    ) -> Vec<TestOutputEntry> {
        let tests = tests
            .into_iter()
            .map(|(identifier, output)| xcodebuild::TestOutput {
                test: identifier
                    .split_once('/')
                    .map_or(identifier, |(_, test)| test)
                    .to_string(),
                identifier: identifier.to_string(),
                output: output.to_string(),
            })
            .collect();
        select_output(tests, only_testing, full)
    }

    #[test]
    fn one_long_line_is_cut_rather_than_erased() {
        // A test printing a single long line — a JSON blob, a serialized
        // payload — has no line break in its tail. Advancing to one past the
        // end would return nothing, and a test that wrote 10 KB would be
        // reported as having written nothing at all.
        let blob = format!("{}\n", "A".repeat(10_000));
        let (cut, truncated) = cut_to_tail(&blob, 4096);
        assert!(truncated);
        assert!(!cut.is_empty(), "a long line was erased instead of cut");
        assert!(cut.len() <= 4096, "{}", cut.len());
        assert!(blob.ends_with(&cut), "{cut}");

        // Same when whole lines precede it: the break is before the cap, so
        // there is still none left to advance to.
        let mixed = format!("hello\nworld\n{}\n", "B".repeat(10_000));
        let (cut, truncated) = cut_to_tail(&mixed, 4096);
        assert!(truncated);
        assert_eq!(cut.len(), 4096);
        assert!(cut.chars().all(|c| c == 'B' || c == '\n'), "{cut}");

        // And with no trailing newline anywhere.
        let (cut, truncated) = cut_to_tail(&"C".repeat(10_000), 4096);
        assert!(truncated);
        assert_eq!(cut, "C".repeat(4096));
    }

    #[test]
    fn a_test_that_printed_one_long_line_still_appears_in_the_report() {
        // The report-level half of the same bug: an entry cut to nothing was
        // dropped by the empty filter, so the run said "0 tests wrote output"
        // for a test that wrote plenty, and never offered `--full`.
        let printed = format!("{}\n", "A".repeat(10_000));
        let entries = output_entries(
            vec![("AppTests/BlobTests/testDump", printed.as_str())],
            &[],
            /* full */ false,
        );
        assert_eq!(entries.len(), 1, "the test was dropped from the report");
        assert_eq!(entries[0].identifier, "AppTests/BlobTests/testDump");
        assert!(entries[0].truncated);
        assert!(!entries[0].output.is_empty());

        // `--full` keeps all of it and marks nothing truncated.
        let entries = output_entries(
            vec![("AppTests/BlobTests/testDump", printed.as_str())],
            &[],
            /* full */ true,
        );
        assert_eq!(entries[0].output.len(), printed.trim_end().len());
        assert!(!entries[0].truncated);
    }

    #[test]
    fn a_test_that_printed_only_whitespace_is_left_out() {
        // The flip side: "wrote nothing" is decided on the whole output, so
        // blank output is still dropped — and dropping it is not something
        // truncation can do by accident.
        let entries = output_entries(
            vec![("AppTests/QuietTests/testNothing", "\n  \n\t\n")],
            &[],
            false,
        );
        assert!(entries.is_empty());
    }

    #[test]
    fn only_testing_selects_among_the_tests_that_wrote_output() {
        // Whatever the heading shows selects its test, down to the target
        // alone; a selector that starts at the class still lands too.
        for selector in [
            "AppUITests",
            "AppUITests/BTests/testTwo",
            "BTests",
            "BTests/testTwo",
        ] {
            let entries = output_entries(
                vec![
                    ("AppTests/ATests/testOne", "from a\n"),
                    ("AppUITests/BTests/testTwo", "from b\n"),
                ],
                &[selector.to_string()],
                false,
            );
            assert_eq!(entries.len(), 1, "{selector}");
            assert_eq!(entries[0].output, "from b", "{selector}");
        }
    }

    #[test]
    fn a_selector_matching_no_output_names_the_tests_that_wrote_some() {
        let err = no_output_match(
            &["AppTests/ATests/testOne".to_string()],
            &["AppTests/ATests/testTwo".to_string()],
        );
        let message = err.to_string();
        assert!(message.contains("AppTests/ATests/testTwo"), "{message}");
        assert!(message.contains("AppTests/ATests/testOne"), "{message}");
        let many: Vec<String> = (0..8).map(|i| format!("T/C/test{i}")).collect();
        let message = no_output_match(&many, &["X".to_string()]).to_string();
        assert!(message.contains("and 3 more"), "{message}");
    }

    #[test]
    fn cutting_never_splits_a_multibyte_character() {
        // The cap counts bytes and the text does not, so the cut lands inside
        // a character for some caps and not others. Every cap has to be safe,
        // and picking one by hand only ever proves the lucky case.
        let text: String = (0..40).fold(String::new(), |mut s, i| {
            let _ = writeln!(s, "é—→ line {i}");
            s
        });
        for cap in 1..=text.len() {
            let (cut, truncated) = cut_to_tail(&text, cap);
            assert_eq!(truncated, cap < text.len());
            assert!(text.ends_with(&cut), "cap {cap} produced a foreign tail");
        }
    }

    #[test]
    fn a_failure_that_says_something_vanished_names_whose_exit_to_read() {
        // Xcode 27's wording, each from a real run.
        let named = Some(Vanished::Named("dev.sweetpad.exitprobe.app".into()));
        assert_eq!(vanished("dev.sweetpad.exitprobe.app crashed"), named);
        assert_eq!(
            vanished("Failed to application dev.sweetpad.exitprobe.app is not running"),
            named
        );
        assert_eq!(
            vanished("Test crashed with signal kill."),
            Some(Vanished::Runner)
        );
        assert_eq!(
            vanished(
                "Crash: ExitProbe at +[XCTFailableInvocation \
                 invokeStandardConventionInvocation:completion:]"
            ),
            Some(Vanished::Runner)
        );
        assert_eq!(vanished("XCTAssertTrue failed"), None);
        assert_eq!(
            vanished("XCTAssertEqual failed: (\"1\") is not equal to (\"2\")"),
            None
        );
    }

    #[test]
    fn a_crash_recorded_after_another_failure_still_gets_its_exit() {
        // The summary gives the test's first message, and the crash can come
        // after it. The exit is timed by the message that names the app.
        let failure = xcodebuild::TestFailure {
            failure_text: "XCTAssertEqual failed".into(),
            other_messages: vec![
                "dev.sweetpad.ci.app crashed".into(),
                "XCTAssertTrue failed".into(),
            ],
            ..Default::default()
        };
        assert_eq!(
            vanishing(&failure),
            Some((
                Vanished::Named("dev.sweetpad.ci.app".into()),
                "dev.sweetpad.ci.app crashed"
            ))
        );
        let plain = xcodebuild::TestFailure {
            failure_text: "XCTAssertEqual failed".into(),
            other_messages: vec!["XCTAssertTrue failed".into()],
            ..Default::default()
        };
        assert_eq!(vanishing(&plain), None);
    }

    fn failed_report(terminations: Vec<Option<Termination>>) -> TestReport {
        TestReport {
            passed: false,
            summary: xcodebuild::TestSummary {
                result: "Failed".into(),
                total_test_count: 2,
                failed_tests: 2,
                test_failures: vec![
                    xcodebuild::TestFailure {
                        test_name: "testAppKilledFromOutside()".into(),
                        target_name: "ExitProbeUITests".into(),
                        failure_text: "Failed to application dev.sweetpad.exitprobe.app is not \
                                       running"
                            .into(),
                        test_identifier_string: "ProbeUITests/testAppKilledFromOutside()".into(),
                        ..Default::default()
                    },
                    xcodebuild::TestFailure {
                        test_name: "testPasses()".into(),
                        target_name: "ExitProbeTests".into(),
                        failure_text: "XCTAssertEqual failed".into(),
                        test_identifier_string: "ProbeTests/testPasses()".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            terminations,
            coverage: None,
            result_bundle: "/tmp/ExitProbe.xcresult".into(),
        }
    }

    #[test]
    fn a_termination_rides_on_its_failure_and_nowhere_else() {
        // `simctl terminate` of the app while a UI test waited on it.
        let line = serde_json::json!({
            "timestamp": "2026-09-26 17:54:45.694421+0200",
            "subsystem": "user/503/UIKitApplication:dev.sweetpad.exitprobe.app[8818][rb-legacy] \
                          [79644]",
            "eventMessage": "exited with exit reason (namespace: 10 code: 0xfbfbfbfb) - \
                             OS_REASON_SPRINGBOARD | <RBSTerminateContext| domain:10 \
                             code:0xFBFBFBFB explanation:Termination requested by simulator \
                             host\n\nProcessVisibility: Foreground\nProcessState: Running \
                             reportType:None maxTerminationResistance:Interactive>, ran for \
                             8372ms",
        })
        .to_string();
        let exit = exits::parse_ndjson_line(&line, &[]).expect("an exit");
        let termination = Termination {
            exit,
            crash_report: None,
        };
        assert_eq!(
            termination.line(),
            "app terminated: Termination requested by simulator host (OS_REASON_SPRINGBOARD \
             0xfbfbfbfb)"
        );
        let json = failed_report(vec![Some(termination), None]).json();
        let reason = &json["failures"][0]["terminationReason"];
        assert_eq!(reason["namespace"], 10);
        assert_eq!(reason["code"], "0xfbfbfbfb");
        assert_eq!(reason["reason"], "OS_REASON_SPRINGBOARD");
        assert_eq!(
            reason["explanation"],
            "Termination requested by simulator host"
        );
        assert!(reason["label"].is_null());
        // An ordinary failure gains no field, and neither does a report whose
        // lookup found nothing.
        assert!(json["failures"][1].get("terminationReason").is_none());
        let bare = failed_report(Vec::new()).json();
        assert!(bare["failures"][0].get("terminationReason").is_none());
    }

    #[test]
    fn a_multi_line_failure_stays_under_its_test() {
        // Swift Testing spells an expectation's operands out on the lines
        // after its first; at column 0 they read as a line of their own.
        let mut report = failed_report(Vec::new());
        report.summary.test_failures[1] = xcodebuild::TestFailure {
            test_name: "suiteGreeting()".into(),
            target_name: "SweetpadCIAppTests".into(),
            failure_text: "Expectation failed: n == 1\nn → 2".into(),
            test_identifier_string: "GreetingSuite/suiteGreeting()".into(),
            test_identifier_url: "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/\
                                  GreetingSuite/suiteGreeting()"
                .into(),
            ..Default::default()
        };
        assert_eq!(
            report.failure_lines(),
            [
                "  ✗ ExitProbeUITests/ProbeUITests/testAppKilledFromOutside: Failed to \
                 application dev.sweetpad.exitprobe.app is not running",
                "  ✗ SweetpadCIAppTests/GreetingSuite/suiteGreeting(): Expectation failed: n == 1",
                "        n → 2",
            ]
        );
    }

    #[test]
    fn every_failure_a_test_recorded_is_listed_under_it() {
        // The first message stays on the `✗` line; the rest follow it one
        // per line, each with its own continuation lines deeper still.
        let mut report = failed_report(Vec::new());
        report.summary.test_failures[1].other_messages = vec![
            "XCTAssertTrue failed - second failure".into(),
            "Expectation failed: m == 3\nm → 4".into(),
        ];
        assert_eq!(
            report.failure_lines()[1..],
            [
                "  ✗ ExitProbeTests/ProbeTests/testPasses: XCTAssertEqual failed",
                "      XCTAssertTrue failed - second failure",
                "      Expectation failed: m == 3",
                "        m → 4",
            ]
        );
        // JSON lists them all under `messages`, the first being `message`.
        let json = report.json();
        assert_eq!(json["failures"][1]["message"], "XCTAssertEqual failed");
        assert_eq!(
            json["failures"][1]["messages"],
            serde_json::json!([
                "XCTAssertEqual failed",
                "XCTAssertTrue failed - second failure",
                "Expectation failed: m == 3\nm → 4",
            ])
        );
        // A test with one failure lists just that one.
        assert_eq!(
            json["failures"][0]["messages"],
            serde_json::json!(["Failed to application dev.sweetpad.exitprobe.app is not running"])
        );
    }

    #[test]
    fn junit_names_a_test_by_its_target_class_and_method() {
        // `classname` is `Target.Class`, which report viewers split into a
        // package and a class, and `name` is the method as a rerun takes it.
        assert_eq!(
            junit_names("SweetpadCIAppTests/AppTests/testGreeting", "App"),
            ("SweetpadCIAppTests.AppTests".to_string(), "testGreeting")
        );
        assert_eq!(
            junit_names("SweetpadCIAppTests/GreetingSuite/Nested/inner()", "App"),
            (
                "SweetpadCIAppTests.GreetingSuite.Nested".to_string(),
                "inner()"
            )
        );
        // A Swift Testing function outside any suite has only its target.
        assert_eq!(
            junit_names("SweetpadCIAppTests/freeGreeting()", "App"),
            ("SweetpadCIAppTests".to_string(), "freeGreeting()")
        );
        assert_eq!(
            junit_names("testSignIn", "App"),
            ("App".to_string(), "testSignIn")
        );
    }

    #[test]
    fn junit_report_escapes_and_counts() {
        let dir = std::env::temp_dir().join(format!("sweetpad-junit-{}", std::process::id()));
        let path = dir.join("r.xml");
        let summary = xcodebuild::TestSummary {
            result: "Failed".into(),
            total_test_count: 3,
            passed_tests: 2,
            failed_tests: 1,
            skipped_tests: 0,
            test_failures: vec![xcodebuild::TestFailure {
                test_name: "testA<>()".into(),
                target_name: "AppTests".into(),
                failure_text: "x & y \"broke\"".into(),
                test_identifier_string: "Suite/testA<>()".into(),
                ..xcodebuild::TestFailure::default()
            }],
        };
        write_junit(&path, "App", &summary).unwrap();
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains("tests=\"3\" failures=\"1\""));
        assert!(
            xml.contains("<testcase classname=\"AppTests.Suite\" name=\"testA&lt;&gt;\">"),
            "{xml}"
        );
        assert!(xml.contains("x &amp; y &quot;broke&quot;"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
