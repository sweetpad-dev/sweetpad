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
use sweetpad_core::xcodebuild_args::{self, has_flag};

use crate::cli::output::Output;
use crate::cli::resolve::Container;
use crate::cli::{
    CliError, CommandResult, Context, ErrorKind, Render, Rendered, exits, resolve, simctl, swiftpm,
    xcodebuild,
};

/// The test flags, declared `global` at the `test` resource so they parse on
/// either side of the (optional) `run` token: `sweetpad test --failed` and
/// `sweetpad test run --failed` are the same invocation.
#[derive(Debug, clap::Args)]
#[allow(clippy::struct_excessive_bools)] // independent CLI toggles, not a state machine
pub struct TestArgs {
    #[command(flatten)]
    pub target: crate::cli::BuildTargetArgs,

    /// Test on this Mac ('--on mac' is the same thing).
    #[arg(long, global = true, help_heading = crate::cli::TARGET_SELECTION)]
    pub mac: bool,

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
/// build`. The targeting flags past the container are among them: the
/// retained bundle is one per project, whatever the run's scheme,
/// configuration or destination was.
#[derive(Debug, clap::Args)]
#[allow(clippy::struct_excessive_bools)] // mirrors TestArgs' toggles, none of them read here
pub struct HiddenRunArgs {
    #[command(flatten)]
    pub target: crate::cli::HiddenTargetArgs,
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

    /// Export only the attachments of the tests that failed.
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
    let typed = crate::cli::flag_typed;
    match action {
        Some(Action::Attachments(opts)) => {
            refuse_run_flags(
                &read_refused_flags(args, typed),
                &read_reason("attachments"),
            )?;
            return attachments(ctx, args, opts);
        }
        Some(Action::Output(opts)) => {
            refuse_run_flags(&read_refused_flags(args, typed), &read_reason("output"))?;
            return output(ctx, args, opts);
        }
        Some(Action::Build(_) | Action::Run) | None => {}
    }
    crate::cli::mac_as_on(&mut ctx.targeting, args.mac)?;
    if let Some(Action::Build(_)) = action {
        return build(ctx, args);
    }
    // A Swift package's tail goes to 'swift test', which takes neither
    // xcodebuild flag, so there is no copy to refuse or leave out.
    let package = matches!(
        resolve::container_silently(ctx),
        Some(Container::SwiftPackage(_))
    );
    let twins = if package {
        Vec::new()
    } else {
        flag_twins(args)
    };
    if let Some((ours, theirs)) = twins
        .iter()
        .find(|(_, theirs)| has_flag(&args.passthrough, theirs))
    {
        return Err(CliError::new(format!(
            "'{ours}' passes '{theirs}' itself, and xcodebuild takes it only once; give one \
             or the other"
        ))
        .kind(ErrorKind::Usage));
    }
    let (passthrough, left_out) = without_file_twins(
        &twins,
        &ctx.xcodebuild_args(xcodebuild::Action::Test, &args.passthrough)?,
    );
    if ctx.out.is_verbose() {
        for note in &left_out {
            ctx.out.note(note);
        }
    }
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

/// Refuse the run flags in `given` by name (see [`crate::cli::refuse_flags`]).
/// `why` follows "…apply to a test run" in the message.
fn refuse_run_flags(given: &[&str], why: &str) -> Result<(), CliError> {
    crate::cli::refuse_flags(given, "a test run", why)
}

/// The `xcodebuild` flags a typed `test` flag passes itself, as `(sweetpad
/// flag, xcodebuild flag)`: '--coverage' passes '-enableCodeCoverage YES' and
/// '--retry-flaky N' passes '-test-iterations N'. `xcodebuild` takes each
/// only once, so [`run`] refuses a typed copy after '--' and
/// [`without_file_twins`] leaves out the project file's.
fn flag_twins(args: &TestArgs) -> Vec<(&'static str, &'static str)> {
    [
        ("--coverage", "-enableCodeCoverage", args.coverage),
        (
            "--retry-flaky",
            "-test-iterations",
            args.retry_flaky.is_some(),
        ),
    ]
    .into_iter()
    .filter_map(|(ours, theirs, given)| given.then_some((ours, theirs)))
    .collect()
}

/// `passthrough` without the project file's copy of each of `twins` and its
/// value, and the notes naming what it left out: the typed flag replaces the
/// copy, as a typed `--` tail replaces a single-use flag in the file. The tail
/// holds none by now ([`run`] refused them), so any copy left is the file's.
/// The arguments are read with [`xcodebuild_args::read`], so the value of a
/// flag that takes one stays with it: `-xcconfig -enableCodeCoverage` names a
/// file, not a copy.
fn without_file_twins(
    twins: &[(&'static str, &'static str)],
    passthrough: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::with_capacity(passthrough.len());
    let mut notes = Vec::new();
    for arg in xcodebuild_args::read(passthrough) {
        let Some((ours, theirs)) = twins.iter().find(|(_, theirs)| arg.word == *theirs) else {
            kept.extend(arg.words().map(String::from));
            continue;
        };
        let value = arg.value.unwrap_or_default();
        notes.push(format!(
            "leaving out sweetpad.toml's '{theirs} {value}': '{ours}' passes its own, and \
             xcodebuild takes '{theirs}' only once"
        ));
    }
    (kept, notes)
}

/// Why `test <verb>` refuses a run flag.
fn read_reason(verb: &str) -> String {
    format!(": 'test {verb}' reads the last run's result bundle and runs nothing")
}

/// The flags on `args` that shape a test run and mean nothing to reading one
/// back. `--only-testing` and `--result-bundle` are not among them: they pick
/// the tests and the bundle to read. The targeting flags count as
/// [`crate::cli::typed_target_flags`] says.
fn read_refused_flags(args: &TestArgs, typed: impl Fn(&str) -> bool) -> Vec<&'static str> {
    let mut given = crate::cli::typed_target_flags(&args.target, args.mac, typed);
    given.extend(
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
        .filter_map(|(flag, given)| given.then_some(flag)),
    );
    given
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
    /// The 'app logs --exits' command for the run's destination, which a
    /// failure that says something vanished points at when no exit was
    /// found for it. `None` when no failure says so, or the destination keeps
    /// no exit log to read.
    exits_command: Option<String>,
    /// Per failure, the 'test attachments' command that exports the crash log
    /// XCTest attached to it, for a failure that says `xctest` crashed.
    crash_log_commands: Vec<Option<String>>,
    coverage: Option<f64>,
    result_bundle: String,
}

/// launchd's account of the exit behind a failure (CLI_DESIGN §9m), with the
/// crash report the system wrote when the exit was a crash.
struct Termination {
    exit: exits::Exit,
    crash_report: Option<PathBuf>,
    /// The test the crash report's backtrace puts the crash in, as
    /// '-only-testing' names it (see [`crashed_in`]).
    crashed_in: Option<String>,
}

impl Termination {
    /// `app terminated: …`, `test runner terminated: …` or `xctest
    /// terminated: …`, by whose exit it was.
    fn line(&self) -> String {
        let who = if self.exit.bundle_id.ends_with(".xctrunner") {
            "test runner"
        } else if self.in_xctest() {
            "xctest"
        } else {
            "app"
        };
        format!("{who} terminated: {}", self.exit.summary())
    }

    /// Whether the exit is `xctest`'s, read from the crash log XCTest
    /// attached ([`attached_crashes`]). Nothing else has an exit with no
    /// bundle id.
    fn in_xctest(&self) -> bool {
        self.exit.bundle_id.is_empty()
    }

    /// Whether `other` is the same crash: one host going down fails every
    /// test that was running in it, and each failure finds the same exit.
    fn same_crash(&self, other: &Self) -> bool {
        self.exit.is_crash()
            && self.exit.pid.is_some()
            && self.exit.pid == other.exit.pid
            && self.exit.bundle_id == other.exit.bundle_id
            && self.exit.time == other.exit.time
    }
}

/// The failed tests whose exit is the same crash as failure `i`'s, `i`
/// included, as '-only-testing' names them.
fn sharing_the_crash(
    failures: &[xcodebuild::TestFailure],
    terminations: &[Option<Termination>],
    i: usize,
) -> Vec<String> {
    let Some(Some(own)) = terminations.get(i) else {
        return Vec::new();
    };
    failures
        .iter()
        .zip(terminations)
        .filter(|(_, t)| t.as_ref().is_some_and(|t| own.same_crash(t)))
        .map(|(f, _)| f.selector())
        .collect()
}

/// What the crash behind failure `i` says about whose code crashed: the test
/// its backtrace is in, when that is another test, or, when the crash failed
/// several tests and the backtrace is in none of them, that which one caused
/// it can't be told.
fn crash_line(
    failures: &[xcodebuild::TestFailure],
    terminations: &[Option<Termination>],
    i: usize,
) -> Option<String> {
    let termination = terminations.get(i)?.as_ref()?;
    if !termination.exit.is_crash() {
        return None;
    }
    if let Some(culprit) = &termination.crashed_in {
        let own = failures.get(i)?.selector();
        return (culprit.trim_end_matches("()") != own.trim_end_matches("()"))
            .then(|| format!("the crash report's backtrace is in {culprit}, not this test"));
    }
    let shared = sharing_the_crash(failures, terminations, i).len();
    if shared < 2 {
        return None;
    }
    Some(if termination.crash_report.is_some() {
        format!("{shared} tests failed with this one crash, and its backtrace is in none of them")
    } else {
        format!(
            "{shared} tests failed with this one crash, and without its crash report which one \
             caused it can't be told"
        )
    })
}

impl TestReport {
    /// What the report says about failure `i` beyond its exit: that a crash
    /// has no crash report, and why that can be, or, for a failure that says
    /// something vanished and has no exit, where else to look for one.
    fn note(&self, i: usize) -> Option<String> {
        match self.terminations.get(i).and_then(Option::as_ref) {
            // The exit came from the copy XCTest attached, and the report it
            // copied is not on disk.
            Some(t) if t.in_xctest() && t.crash_report.is_none() => {
                let export = self.crash_log_commands.get(i)?.as_deref()?;
                Some(format!(
                    "no crash report was found in DiagnosticReports; {export} exports the copy \
                     XCTest attached"
                ))
            }
            Some(t) if t.exit.is_crash() && t.crash_report.is_none() => Some(format!(
                "no crash report was found; {}",
                exits::REPORT_LIMIT
            )),
            Some(_) => None,
            None => {
                let (cause, _) = vanishing(self.summary.test_failures.get(i)?)?;
                if cause == Vanished::Xctest {
                    let export = self.crash_log_commands.get(i)?.as_deref()?;
                    return Some(format!(
                        "the tests ran in 'xctest', which launchd logs no exit for; {export} \
                         exports the crash log XCTest attached"
                    ));
                }
                let exits = self.exits_command.as_deref()?;
                Some(format!("couldn't find launchd's exit record; try {exits}"))
            }
        }
    }

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
            if let Some(crash) = crash_line(&self.summary.test_failures, &self.terminations, i) {
                lines.push(format!("      {crash}"));
            }
            if let Some(note) = self.note(i) {
                lines.push(format!("      {note}"));
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
                    if let Some(culprit) = &t.crashed_in {
                        failure["crashedIn"] = culprit.as_str().into();
                    } else {
                        let shared =
                            sharing_the_crash(&self.summary.test_failures, &self.terminations, i);
                        if shared.len() > 1 {
                            failure["crashCandidates"] = shared.into();
                        }
                    }
                }
                if let Some(note) = self.note(i) {
                    failure["note"] = note.into();
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
    let final_bundle = bundle_path(&resolved.container, args.result_bundle);

    // `--failed`: the selectors come from the *previous* run's retained
    // bundle, read before anything touches it.
    let only: Vec<String> = if args.failed {
        let failed = xcodebuild::failed_tests(&final_bundle)?;
        // xcodebuild takes a failure outside any test as a selector, runs no
        // test, and reports success, so it is left out and said so.
        let outside = failed
            .outside_tests
            .iter()
            .map(|name| format!("'{name}'"))
            .collect::<Vec<_>>()
            .join(", ");
        if failed.selectors.is_empty() {
            return Err(CliError::new(if outside.is_empty() {
                "the previous run recorded no failures to rerun (or no previous run exists)"
                    .to_string()
            } else {
                format!(
                    "the previous run failed outside any test ({outside}), so '--failed' has no \
                     test to rerun; run 'sweetpad test' without it to rerun them all"
                )
            }));
        }
        if !outside.is_empty() {
            let (records, it) = if failed.outside_tests.len() == 1 {
                ("it records", "it")
            } else {
                ("they record", "them")
            };
            ctx.out.note(&format!(
                "not rerunning {outside}: {records} the test process failing outside any \
                 test, and no '-only-testing' selector can name {it}"
            ));
        }
        failed.selectors
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
        // A JUnit report lists every test, and only the test tree has the
        // passed ones, so `--junit` pays for reading it on a green run too.
        match xcodebuild::test_summary(&run_bundle, args.junit.is_some()) {
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
    // The run's build step is the project's last build, so it is recorded the
    // way 'build' records its own, for 'build diagnostics' to read back. Tests
    // only run once it succeeds.
    if outcome.parsed {
        xcodebuild::record_build(
            &resolved.container,
            Some(&target.scheme),
            ran_tests,
            &outcome.diagnostics,
        );
    }
    if !ran_tests {
        if args.result_bundle.is_some() {
            let _ = std::fs::remove_dir_all(&final_bundle);
            let _ = std::fs::rename(&run_bundle, &final_bundle);
        } else {
            let _ = std::fs::remove_dir_all(&run_bundle);
        }
        let device = xcodebuild::device_tip(&plan.command().0, &outcome.diagnostics);
        let err = build_step_failure(
            &xcodebuild::project_artifact(&resolved.container, "-test.log"),
            outcome,
            !Output::streams_share_a_file(),
        );
        // A device's own reason for failing outranks the list of every error.
        let tip = device.or_else(|| err.tip_text().map(str::to_string));
        return Err(err.tip(tip));
    }
    // The scratch run is the project's latest real result: promote it to the
    // retained slot.
    let _ = std::fs::remove_dir_all(&final_bundle);
    let bundle = if std::fs::rename(&run_bundle, &final_bundle).is_ok() {
        final_bundle
    } else {
        run_bundle
    };
    let read_summary = summary.is_some();
    if !read_summary {
        ctx.out
            .warn("could not read the result bundle's summary; counts show as 0");
    }
    let summary = summary.unwrap_or_default();
    let passed = outcome.passed;

    let coverage = args
        .coverage
        .then(|| xcodebuild::coverage_percent(&bundle))
        .flatten();

    let (terminations, exit_log) = if passed {
        (Vec::new(), None)
    } else {
        let run = RunContext {
            resolved: &resolved,
            target: &target,
            passthrough: args.passthrough,
            bundle: &bundle,
            started: run_started,
        };
        terminations(&run, &summary)
    };

    // Written once the exits are known, so a failure's body can say what
    // ended the app.
    if let Some(junit) = args.junit.map(sweetpad_lib::project::absolutize) {
        write_junit(&junit, &target.scheme, &summary, &terminations)?;
        if read_summary && summary.test_cases.is_none() {
            ctx.out.warn(
                "could not read the result bundle's test tree, so the JUnit report lists only \
                 the failed tests",
            );
        }
        ctx.out.note(&format!("junit report: {}", junit.display()));
    }

    let exits_command =
        exit_log.map(|log| super::app::follow_up(ctx, "app logs", &log.exits_args()));
    let crash_log_commands = summary
        .test_failures
        .iter()
        .map(|failure| {
            let (cause, _) = vanishing(failure)?;
            (cause == Vanished::Xctest).then(|| {
                crash_log_command(
                    ctx,
                    args.result_bundle.is_some().then_some(&*bundle),
                    failure,
                )
            })
        })
        .collect();
    let report = TestReport {
        passed,
        summary,
        terminations,
        exits_command,
        crash_log_commands,
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

/// The 'test attachments' command that exports what `failure`'s test attached,
/// the crash log among it, from `bundle` when the run named its own.
fn crash_log_command(
    ctx: &Context,
    bundle: Option<&Path>,
    failure: &xcodebuild::TestFailure,
) -> String {
    let bundle = bundle.map(|b| b.display().to_string());
    let selector = failure.selector();
    let mut rest = Vec::new();
    if let Some(bundle) = &bundle {
        rest.extend(["--result-bundle", bundle.as_str()]);
    }
    rest.extend(["--only-testing", selector.as_str()]);
    super::app::read_back_follow_up(ctx, "test attachments", &rest)
}

/// The error for a run that died before any test executed — nearly always a
/// failed compile. The parsed diagnostics are the diagnosis, so they ride in
/// the error object and their first error becomes the message; the transcript
/// is parked at `log`, the project's artifact slot for it, and named, rather
/// than quoted in full. A run with nothing parseable (a bad destination, a
/// signing refusal) keeps the tail, which is then the only account of what
/// happened.
///
/// A human run streamed the errors to stdout. `stderr_apart` says stderr is a
/// different file (`2>err.log`), where they are not in front of this error, so
/// it repeats the first few ([`xcodebuild::repeated_errors`]) rather than
/// leaving the stream to explain it, and points at `build diagnostics` for the
/// rest, as a failed `build` does.
fn build_step_failure(
    log: &Path,
    outcome: xcodebuild::TestRunOutcome,
    stderr_apart: bool,
) -> CliError {
    // A blocked build is not a compile failure: no diagnostic describes it and
    // no edit fixes it, so the flag that unblocks it is the whole answer.
    if let Some(hint) = outcome.blocker {
        return CliError::new(format!("the build is blocked, not broken: {hint}"))
            .kind(crate::cli::ErrorKind::BuildFailure)
            .context("running the tests");
    }
    let streamed_error = xcodebuild::streamed_an_error(outcome.streamed, &outcome.diagnostics);
    let shown = streamed_error && !stderr_apart;
    let repeated = streamed_error && stderr_apart;
    let log = outcome
        .transcript
        .as_deref()
        .and_then(|text| xcodebuild::record_failure_transcript(log, text));
    let detail = match xcodebuild::diagnostics_summary(&outcome.diagnostics) {
        Some(_) if repeated => xcodebuild::repeated_errors(&outcome.diagnostics),
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
    .tip(repeated.then(|| xcodebuild::DIAGNOSTICS_TIP.to_string()))
    .context("running the tests");
    if shown { err.shown() } else { err }
}

/// What a failure message says vanished mid-test, in the wording Xcode 27
/// uses for it.
#[derive(Debug, PartialEq, Eq)]
enum Vanished {
    /// The app it names: `<bundle id> crashed`, `<bundle id> crashed in
    /// <symbol>`, `Failed to application <bundle id> is not running`.
    Named(String),
    /// The app under test, when an `Application … is not running` names no
    /// bundle id.
    App,
    /// The process running the tests: a UI test's `.xctrunner`, or the host
    /// app a unit test runs in (see [`RUNNER_VANISHED`]).
    Runner,
    /// `xctest`, which runs a unit-test bundle that has no host app, named as
    /// the process that crashed (`Crash: xctest at static xctest.main()`).
    /// xcodebuild starts it rather than launchd, so launchd logs no exit for it.
    Xctest,
}

/// How XCTest's harness words the process running the tests going away, each
/// as the start of the message. A unit test's host app is that process, so a
/// host that crashes reads `Crash: <App> at <frame>. <library>: <reason>`, or
/// `The test runner crashed while preparing to run tests: …` when it was the
/// first test to run, and one that calls `exit` reads `The test runner exited
/// with code 3 before finishing running tests. …`. A Swift Testing test runs
/// in the same host and vanishes in the same words. A UI test's runner killed
/// outright reads `Test crashed with signal kill.` Those were each seen in a
/// run on Xcode 27; the others are read from the strings in its XCTest
/// harness.
const RUNNER_VANISHED: [&str; 7] = [
    "Crash: ",
    "Test crashed with signal ",
    "The test runner crashed before establishing connection: ",
    "The test runner crashed while preparing to run tests: ",
    "The test runner exited with code ",
    "Lost connection to the test runner",
    "Lost connection to test process",
];

/// The wordings in [`RUNNER_VANISHED`] that go on to name the process running
/// the tests, the host app's executable or `xctest`.
const RUNNER_NAMED: [&str; 3] = [
    "Crash: ",
    "The test runner crashed before establishing connection: ",
    "The test runner crashed while preparing to run tests: ",
];

/// The same, where the harness writes the whole message and nothing follows.
const RUNNER_VANISHED_EXACTLY: [&str; 3] = [
    "Test crashed",
    "Test runner crashed.",
    "The test runner exited",
];

/// What `message` says vanished, when it is one of the ways Xcode words that.
/// An assertion that merely mentions a crash (`XCTFail("the helper
/// crashed")`, recorded as `failed - the helper crashed`) is not one of them.
fn vanished(message: &str) -> Option<Vanished> {
    let bundle_id = |token: &str| {
        (token.contains('.')
            && token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')))
        .then(|| token.to_string())
    };
    // XCTest gives up on a host that keeps crashing, or one that crashed
    // before any test ran, and says why in the host's own words: `Exceeded
    // max restart count of 2. (Underlying Error: Crash: <App> at <frame>. …)`,
    // `Early unexpected exit, operation never finished bootstrapping - no
    // restart will be attempted. (Underlying Error: The test runner crashed
    // before establishing connection: <App>)`.
    if let Some(underlying) = message
        .strip_prefix("Exceeded max restart count ")
        .or_else(|| message.strip_prefix("Early unexpected exit, "))
        .and_then(|rest| rest.split_once("(Underlying Error: "))
        .map(|(_, cause)| cause.strip_suffix(')').unwrap_or(cause))
    {
        return vanished(underlying);
    }
    let crashed = message
        .strip_suffix(" crashed")
        .or_else(|| message.split_once(" crashed in ").map(|(head, _)| head));
    if let Some(named) = crashed.and_then(bundle_id) {
        return Some(Vanished::Named(named));
    }
    // `Application <id> is not running`, `Application for <id> is not
    // running.`, or `Failed to application <id> is not running`.
    if let Some((head, _)) = message.split_once(" is not running") {
        let mut words = head.rsplit(' ');
        let named = words.next().and_then(bundle_id);
        let mut before = words.next();
        if before == Some("for") {
            before = words.next();
        }
        if before.is_some_and(|w| w.eq_ignore_ascii_case("application")) {
            return Some(named.map_or(Vanished::App, Vanished::Named));
        }
    }
    let hostless = RUNNER_NAMED
        .iter()
        .filter_map(|p| message.strip_prefix(p))
        .any(|named| {
            named
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
                .next()
                == Some("xctest")
        });
    if hostless {
        return Some(Vanished::Xctest);
    }
    let runner = RUNNER_VANISHED.iter().any(|p| message.starts_with(p))
        || RUNNER_VANISHED_EXACTLY.contains(&message);
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

/// How long after a failure on a simulator the exit behind it may be logged.
/// launchd writes the line when it reaps the process, which can trail the
/// moment the test noticed. XCTest restarts a crashed unit-test host, and in a
/// measured run the restart's clean exit landed 1.5s after the failure, so the
/// margin stays well under that. A Mac gets none (see [`ExitLog::after_failure`]).
const EXIT_AFTER_FAILURE: f64 = 0.5;

/// The longest the exit query may take before the report goes out without
/// termination reasons.
const EXIT_QUERY_TIMEOUT: Duration = Duration::from_secs(15);

/// How long to wait before the one more look [`find_exits`] takes. launchd's
/// line is readable within a second of the exit, so a second query mostly
/// recovers from the first one failing or running out of time. Four other
/// readers dumping the simulator's log, as `simctl diagnose` does, stretched a
/// 1.5s query to 13s, and the one after it took 7s.
const EXIT_RETRY_WAIT: Duration = Duration::from_secs(2);

/// At most this many failures are timed, since each costs an `xcresulttool`
/// read of the test's activity log. The same bound holds for the crash logs
/// read from the result bundle ([`attached_crashes`]).
const MAX_TIMED_FAILURES: usize = 20;

/// For each failure with a message that says the app or the test runner
/// vanished, the exit launchd logged for it: the last exit of that process
/// between the test's start and the moment that message was recorded (just
/// after it, on a simulator: [`ExitLog::after_failure`]). A failure that says
/// `xctest` crashed gets the exit in the crash log attached to its test
/// instead ([`attached_crashes`]). Best effort and bounded: an unreadable log,
/// an unsupported destination, or a failure with no activity log leaves that
/// failure without one. Also the log that was searched, when there was a
/// failure to search it for.
fn terminations(
    run: &RunContext,
    summary: &xcodebuild::TestSummary,
) -> (Vec<Option<Termination>>, Option<ExitLog>) {
    let failures = &summary.test_failures;
    let causes: Vec<Option<(Vanished, &str)>> = failures.iter().map(vanishing).collect();
    if causes.iter().all(Option::is_none) {
        return (failures.iter().map(|_| None).collect(), None);
    }
    let tests = run_tests(summary);
    let attached = attached_crashes(run, failures, &causes, &tests);
    let Some(log) = ExitLog::of(&run.target.destination) else {
        return (attached, None);
    };
    let run_start = epoch_seconds(run.started) - EXIT_QUERY_LEAD;
    let mut budget = MAX_TIMED_FAILURES;
    let windows: Vec<Option<ExitWindow>> = causes
        .into_iter()
        .zip(failures)
        .map(|(cause, failure)| {
            let (cause, message) = cause?;
            if cause == Vanished::Xctest {
                return None;
            }
            budget = budget.checked_sub(1)?;
            let times =
                xcodebuild::failure_times(run.bundle, &failure.test_identifier_string, message)?;
            Some(ExitWindow {
                cause,
                from: times.started.map_or(run_start, |t| t - 1.0),
                until: times.failed + log.after_failure(),
            })
        })
        .collect();
    // Resolved on first need: only a failure that names no bundle id needs it.
    let mut app_id: Option<Option<String>> = None;
    let mut app_id = || app_id.get_or_insert_with(|| app_bundle_id(run)).clone();
    let terminations = find_exits(
        &windows,
        &mut || exits_during(run, &log),
        &mut || std::thread::sleep(EXIT_RETRY_WAIT),
        &mut app_id,
    )
    .into_iter()
    .zip(attached)
    .map(|(exit, attached)| {
        if attached.is_some() {
            return attached;
        }
        let exit = exit?;
        let crash_report = exit
            .is_crash()
            .then(|| exits::crash_report(&exit, Some(run.started)))
            .flatten();
        let crashed_in = crash_report
            .as_deref()
            .and_then(|report| crashed_in(&exits::crashed_thread_symbols(report), &tests));
        Some(Termination {
            exit,
            crash_report,
            crashed_in,
        })
    })
    .collect();
    (terminations, Some(log))
}

/// For each failure that says `xctest` crashed ([`Vanished::Xctest`]), the
/// exit recorded in the crash log XCTest attached to its test, with the test
/// the crashed thread is in and the report the system saved, which the
/// attachment copies. launchd logs no exit for `xctest`, and the saved
/// report's header names no bundle id, so the attachment is what ties a crash
/// to its test. A test retried after a crash can hold several, and the latest
/// is read. At most [`MAX_TIMED_FAILURES`] tests are read.
fn attached_crashes(
    run: &RunContext,
    failures: &[xcodebuild::TestFailure],
    causes: &[Option<(Vanished, &str)>],
    tests: &[String],
) -> Vec<Option<Termination>> {
    let mut budget = MAX_TIMED_FAILURES;
    failures
        .iter()
        .zip(causes)
        .map(|(failure, cause)| {
            if !matches!(cause, Some((Vanished::Xctest, _))) {
                return None;
            }
            budget = budget.checked_sub(1)?;
            let test_id = if failure.test_identifier_url.is_empty() {
                &failure.test_identifier_string
            } else {
                &failure.test_identifier_url
            };
            let (text, exit) = xcodebuild::attached_crash_logs(run.bundle, test_id)
                .into_iter()
                .rev()
                .find_map(|text| exits::xctest_exit(&text).map(|exit| (text, exit)))?;
            let crashed_in = exits::crashed_thread_symbols_in(&text)
                .and_then(|frames| crashed_in(&frames, tests));
            Some(Termination {
                exit,
                crash_report: exits::saved_report(&text, Some(run.started)),
                crashed_in,
            })
        })
        .collect()
}

/// The run's tests as '-only-testing' names them, for [`crashed_in`] to find
/// in a backtrace: every case in the test tree, or, when it couldn't be read,
/// just the failed ones.
fn run_tests(summary: &xcodebuild::TestSummary) -> Vec<String> {
    summary.test_cases.as_ref().map_or_else(
        || {
            summary
                .test_failures
                .iter()
                .map(xcodebuild::TestFailure::selector)
                .collect()
        },
        |cases| cases.iter().map(|c| c.identifier.clone()).collect(),
    )
}

/// The test a crashed thread's backtrace is in: the innermost frame that is
/// one of `tests` (as '-only-testing' names them), or a closure inside one.
///
/// XCTest fails every test that was running when the host crashed, all with
/// the same message, and a crash in work a test left running after it passed
/// fails whichever tests run at that moment instead. Neither says whose code
/// crashed. The crash report's backtrace does, when it runs through the test.
fn crashed_in(frames: &[String], tests: &[String]) -> Option<String> {
    frames.iter().find_map(|symbol| {
        let function = frame_function(symbol)?;
        tests
            .iter()
            .find(|test| test_function(test) == function)
            .cloned()
    })
}

/// A backtrace frame's function as `Type.method`: the last word of a
/// demangled Swift symbol without its argument list (`closure #1 in
/// Suite.test(n:)` is `Suite.test`), or an Objective-C method's class and
/// selector (`-[AppTests testGreeting]` is `AppTests.testGreeting`).
fn frame_function(symbol: &str) -> Option<String> {
    if let Some(method) = symbol
        .strip_prefix("-[")
        .or_else(|| symbol.strip_prefix("+["))
    {
        let (class, name) = method.strip_suffix(']')?.split_once(' ')?;
        return Some(format!("{class}.{name}"));
    }
    let last = symbol.rsplit(' ').next()?;
    let (name, _) = last.split_once('(')?;
    (!name.is_empty()).then(|| name.to_string())
}

/// A test's function as [`frame_function`] gives it: the '-only-testing'
/// selector without its target and argument list, so
/// `AppTests/Suite/test(n:)` is `Suite.test`.
fn test_function(selector: &str) -> String {
    let within = selector.split_once('/').map_or(selector, |(_, test)| test);
    let name = within.split_once('(').map_or(within, |(name, _)| name);
    name.replace('/', ".")
}

/// Where to look for the exit behind one failure: whose it is, and the part
/// of the run it happened in, in seconds since the epoch.
struct ExitWindow {
    cause: Vanished,
    from: f64,
    until: f64,
}

impl ExitWindow {
    /// The last exit in `found` of the process the cause names, inside the
    /// window. `app_id` gives the bundle id of the app under test.
    fn exit_in(
        &self,
        found: &[exits::Exit],
        app_id: &mut dyn FnMut() -> Option<String>,
    ) -> Option<exits::Exit> {
        let last_of = |matches: &dyn Fn(&exits::Exit) -> bool| {
            found
                .iter()
                .filter(|e| {
                    e.epoch_seconds()
                        .is_some_and(|t| t >= self.from && t <= self.until)
                })
                .rfind(|e| matches(e))
                .cloned()
        };
        match &self.cause {
            Vanished::Named(id) => last_of(&|e| e.bundle_id == *id),
            Vanished::App => app_id().and_then(|id| last_of(&|e| e.bundle_id == id)),
            // A UI test's runner is its own `.xctrunner` app; a unit test's is
            // the host app, so that is the fallback.
            Vanished::Runner => last_of(&|e| e.bundle_id.ends_with(".xctrunner"))
                .or_else(|| app_id().and_then(|id| last_of(&|e| e.bundle_id == id))),
            // launchd logs no exit for `xctest`, and an app's exit in the
            // window is some other test target's host.
            Vanished::Xctest => None,
        }
    }
}

/// The exit behind each failure that has a window, from the exits `lookup`
/// reads (`None` when it fails). When one of them finds no exit, `wait` runs
/// and the lookup is made once more for those still without one. A run with
/// no window to fill never looks at all.
fn find_exits(
    windows: &[Option<ExitWindow>],
    lookup: &mut dyn FnMut() -> Option<Vec<exits::Exit>>,
    wait: &mut dyn FnMut(),
    app_id: &mut dyn FnMut() -> Option<String>,
) -> Vec<Option<exits::Exit>> {
    let mut exits: Vec<Option<exits::Exit>> = windows.iter().map(|_| None).collect();
    if windows.iter().all(Option::is_none) {
        return exits;
    }
    let unfilled = |exits: &[Option<exits::Exit>]| {
        windows
            .iter()
            .zip(exits)
            .any(|(window, exit)| window.is_some() && exit.is_none())
    };
    for attempt in 0..2 {
        if attempt > 0 {
            if !unfilled(&exits) {
                break;
            }
            wait();
        }
        let Some(found) = lookup() else {
            continue;
        };
        for (window, exit) in windows.iter().zip(&mut exits) {
            if exit.is_none()
                && let Some(window) = window
            {
                *exit = window.exit_in(&found, app_id);
            }
        }
    }
    exits
}

/// Where a test destination's app exits are logged.
enum ExitLog {
    Mac,
    Simulator(String),
}

impl ExitLog {
    /// The log `destination`'s app exits land in; `None` for a destination
    /// with none to read (a device).
    fn of(destination: &str) -> Option<Self> {
        let spec = sweetpad_lib::destination::DestinationSpec::parse(destination);
        if spec.is_macos() {
            return Some(Self::Mac);
        }
        if !spec.is_simulator() || spec.generic {
            return None;
        }
        let udid = if let Some(id) = spec.id.clone() {
            id
        } else {
            let sims = simctl::list().ok()?;
            sweetpad_core::devices::simctl::find_named(&sims, &spec)?
                .udid
                .clone()
        };
        Some(Self::Simulator(udid))
    }

    fn source(&self) -> exits::Source<'_> {
        match self {
            Self::Mac => exits::Source::Mac,
            Self::Simulator(udid) => exits::Source::Simulator(udid),
        }
    }

    /// How long after a failure the exit behind it may land: a simulator's
    /// [`EXIT_AFTER_FAILURE`], and nothing on a Mac. There XCTest records a
    /// failure only after the exit behind it, 0.3s after a host's `exit` and
    /// 2s to 6.3s after a crash (it waits for the crash report), and the host
    /// it restarts to finish the run exited 0.4s to 0.8s after the failure. A
    /// simulator's margin would take the restarted host's exit for the crash,
    /// and no margin is needed.
    fn after_failure(&self) -> f64 {
        match self {
            Self::Mac => 0.0,
            Self::Simulator(_) => EXIT_AFTER_FAILURE,
        }
    }

    /// The 'app logs --exits' flags that read this log.
    fn exits_args(&self) -> Vec<&str> {
        match self {
            Self::Mac => vec!["--exits", "--mac"],
            Self::Simulator(udid) => vec!["--exits", "--on", udid],
        }
    }
}

/// Every app exit launchd logged in `log` since the run started, plus the
/// crashes only a crash report records. `None` when the query failed.
fn exits_during(run: &RunContext, log: &ExitLog) -> Option<Vec<exits::Exit>> {
    let source = log.source();
    let window = exits::Window::Between {
        start: epoch_seconds(run.started) - EXIT_QUERY_LEAD,
        end: epoch_seconds(SystemTime::now()) + 1.0,
    };
    let launchd = exits::query(&source, &[], &window, EXIT_QUERY_TIMEOUT).ok()?;
    let reports = exits::crash_reports(&source, &[], Some(run.started));
    Some(
        exits::merge(launchd, Vec::new(), reports)
            .into_iter()
            .map(|(exit, _)| exit)
            .collect(),
    )
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
    xcodebuild::located(&plan)
        .ok()
        .map(|located| located.app.bundle_id)
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

/// The `.xcresult` a run writes or a read-back verb reads: the one `given`
/// names, else the project's retained slot. A named path is made absolute
/// with its `.` and `..` collapsed, the form every report prints, so
/// `--result-bundle ../x.xcresult` reads back as one clean path rather than
/// as the cwd with `/../x.xcresult` on the end.
fn bundle_path(container: &Container, given: Option<&Path>) -> PathBuf {
    given.map_or_else(
        || retained_bundle_path(container),
        sweetpad_lib::project::absolutize,
    )
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
    /// A failure's evidence: its test failed, or xcresulttool marks the file as
    /// recorded against a failure. Xcode 27 leaves the mark off what a failed
    /// test attached itself, and off an iOS simulator UI test's crash log too,
    /// though it sets it on the crash log of a macOS test whose host crashed.
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

    // A named directory is reported the way [`bundle_path`] reports a bundle.
    let output_dir = opts.output_dir.as_deref().map_or_else(
        || {
            let ours = export_dir_path(&container);
            // Our own slot holds one export at a time, so a stale file from a
            // previous run can never be mistaken for this one. A directory the
            // caller named is theirs, and is added to rather than emptied.
            let _ = std::fs::remove_dir_all(&ours);
            ours
        },
        sweetpad_lib::project::absolutize,
    );

    // xcresulttool exports under UUID names and *duplicates* into a populated
    // directory (`name (1).png`), so it always gets a fresh directory of its
    // own; the files are renamed out of it afterwards.
    let staging = output_dir.join(".sweetpad-export");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .map_err(|e| CliError::new(format!("failed to create {}: {e}", staging.display())))?;

    let export = match xcodebuild::export_attachments(&bundle, &staging) {
        Ok(export) => export,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    let mut exported = export.attachments;
    let attached_any = !exported.is_empty();
    if opts.only_failures {
        // A failed test's files, and any file xcresulttool marks as recorded
        // against a failure. On Xcode 27 the mark alone would keep nothing
        // from an iOS simulator UI test, crash log included, and only the
        // crash log from a macOS test whose host crashed.
        exported.retain(|a| a.failure || a.failed_test);
    }
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

    let note =
        (!found_any).then(|| empty_note(opts.only_failures, export.failed_tests, attached_any));
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
            failure: item.failure || item.failed_test,
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
    let bundle = bundle_path(&container, args.result_bundle.as_deref());
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
    /// Whether the run said it ran its tests serially. Carried for the machine
    /// modes only: a parallel run's workers each write a stream of their own,
    /// so it changes nothing about which test a line is under.
    serial: bool,
    /// Whether a stream's case markers overlapped, which leaves some lines
    /// under the wrong test (see [`OutputReport::overlap_warning`]).
    overlapped: bool,
    note: Option<String>,
}

impl OutputReport {
    /// The warning an overlap earns, pointing at the kept streams: they hold
    /// the output in the order it was written, which the per-test slices of
    /// an overlapping stream don't.
    fn overlap_warning(&self) -> Option<String> {
        self.overlapped.then(|| {
            let where_ = self
                .sources
                .first()
                .and_then(|p| p.parent())
                .map_or_else(String::new, |d| {
                    format!("; read {} for the output as written", d.display())
                });
            format!(
                "tests overlapped in one test process, so some lines may be listed under the \
                 wrong test{where_}"
            )
        })
    }
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
        if let Some(warning) = self.overlap_warning() {
            out.warn(&warning);
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
            "overlapped": self.overlapped,
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

    let note = output_note(
        found_any,
        args.only_testing.is_empty(),
        run.between_tests,
        &run.sources,
    );
    // Output written outside a test case is XCTest's own bookkeeping and the
    // app's `os_log` chatter — noise, next to the lines a test meant to write.
    // It earns a place only when nothing was attributed at all, which is what
    // a framework this parser does not recognise looks like: then it is the
    // only account of what ran, and dropping it would lose the run entirely.
    // Otherwise the note counts what was written while the tests ran.
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
        overlapped: run.overlapped,
        note,
    }))
}

/// What `test output` says under its listing: that no test wrote anything, or,
/// when some did and `every_test` says no '--only-testing' narrowed the
/// report, that it leaves out `outside` lines written between tests and which
/// directory of `sources` has them.
fn output_note(
    found_any: bool,
    every_test: bool,
    outside: usize,
    sources: &[PathBuf],
) -> Option<String> {
    if !found_any {
        return Some(
            "no test wrote to stdout or stderr; a test's own 'print' lands here, while \
             XCTest's assertions and a UI test's screenshots do not"
                .to_string(),
        );
    }
    if !every_test || outside == 0 {
        return None;
    }
    let (lines, verb) = if outside == 1 {
        ("line", "is")
    } else {
        ("lines", "are")
    };
    let where_ = sources
        .first()
        .and_then(|p| p.parent())
        .map_or_else(String::new, |d| {
            format!("; read {} for the output as written", d.display())
        });
    Some(format!(
        "{outside} {lines} written outside any test {verb} not listed{where_}"
    ))
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
///
/// Under `--only-failures` the note goes by what the bundle says:
/// `failed_tests` is how many tests the run's test tree lists as failed
/// (`None` when it could not be read), and `attached_any` whether any test
/// attached anything.
fn empty_note(only_failures: bool, failed_tests: Option<usize>, attached_any: bool) -> String {
    const DISCARDED: &str = "XCTAttachment.lifetime defaults to .deleteOnSuccess, so a \
                             passing test's attachments are discarded — set \
                             'attachment.lifetime = .keepAlways' to keep them";
    let discarded = || format!("the run attached nothing that survived: {DISCARDED}");
    if !only_failures {
        return discarded();
    }
    let rest = if attached_any {
        "; drop '--only-failures' to export what the passing tests attached"
    } else {
        ""
    };
    match failed_tests {
        Some(0) if attached_any => format!("the run had no failing tests{rest}"),
        Some(0) => format!("the run had no failing tests and kept no attachments: {DISCARDED}"),
        Some(1) => format!("the run's one failing test attached nothing{rest}"),
        Some(n) => format!("none of the run's {n} failing tests attached anything{rest}"),
        None if attached_any => "xcresulttool marked no attachment as recorded against a \
                                 failure, and sweetpad could not read which tests failed from \
                                 the result bundle; drop '--only-failures' to export everything \
                                 the run attached"
            .to_string(),
        None => discarded(),
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

/// Write the run's JUnit XML report to `path` (see [`junit_xml`]).
fn write_junit(
    path: &Path,
    scheme: &str,
    summary: &xcodebuild::TestSummary,
    terminations: &[Option<Termination>],
) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, junit_xml(scheme, summary, terminations))
        .map_err(|e| CliError::new(format!("failed to write {}: {e}", path.display())))
}

/// One `<testcase>` of a JUnit report.
struct JunitCase<'a> {
    /// As `-only-testing` takes it, which [`junit_names`] splits.
    selector: String,
    outcome: xcodebuild::CaseOutcome,
    duration: Option<f64>,
    /// Every failure message of a failed test, or why a skipped one skipped.
    messages: Vec<&'a str>,
    /// What ended the app or the test runner under a failed test, as the
    /// `app terminated: …` line the summary prints under it, and the line on
    /// whose code crashed when the summary prints one.
    ended: Option<String>,
}

/// The report's test cases: every case in the test tree, a failed one with
/// its messages as the summary gives them and what ended its app when
/// `terminations` (per summary failure) has it, then any failure the tree does
/// not list. With no tree to read, only the summary's failures.
fn junit_cases<'a>(
    summary: &'a xcodebuild::TestSummary,
    terminations: &[Option<Termination>],
) -> Vec<JunitCase<'a>> {
    use xcodebuild::CaseOutcome;
    let same = |a: &str, b: &str| a.trim_end_matches("()") == b.trim_end_matches("()");
    let failures: Vec<(String, &xcodebuild::TestFailure, Option<String>)> = summary
        .test_failures
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let ended = terminations.get(i).and_then(Option::as_ref).map(|t| {
                let mut ended = t.line();
                if let Some(crash) = crash_line(&summary.test_failures, terminations, i) {
                    ended = format!("{ended}\n{crash}");
                }
                ended
            });
            (f.selector(), f, ended)
        })
        .collect();
    let mut cases: Vec<JunitCase> = summary
        .test_cases
        .iter()
        .flatten()
        .map(|case| {
            let failure = failures.iter().find(|(s, ..)| same(s, &case.identifier));
            let (outcome, messages) = match (failure, case.outcome) {
                (Some((_, f, _)), _) => (CaseOutcome::Failed, f.messages().collect()),
                (None, CaseOutcome::Failed) => (
                    CaseOutcome::Failed,
                    case.messages.iter().map(String::as_str).collect(),
                ),
                (None, CaseOutcome::Skipped) => (
                    CaseOutcome::Skipped,
                    case.skip_message.iter().map(String::as_str).collect(),
                ),
                (None, CaseOutcome::Passed) => (CaseOutcome::Passed, Vec::new()),
            };
            JunitCase {
                selector: case.identifier.clone(),
                outcome,
                duration: case.duration,
                messages,
                ended: failure.and_then(|(.., ended)| ended.clone()),
            }
        })
        .collect();
    for (selector, f, ended) in failures {
        if !cases.iter().any(|c| same(&c.selector, &selector)) {
            cases.push(JunitCase {
                selector,
                outcome: CaseOutcome::Failed,
                duration: None,
                messages: f.messages().collect(),
                ended,
            });
        }
    }
    cases
}

/// A JUnit XML report of the run: one `<testcase>` per test, passed and
/// skipped ones included, since GitLab counts the cases and ignores the
/// suite's totals. A failure's `message` is the first line of its first
/// message, and the element's body holds every message in full, a blank line
/// between two, then the `app terminated: …` line when `terminations` has one
/// for it; an attribute can't carry a line break the way text can. With no
/// test tree only the failures are listed, under the summary's totals.
fn junit_xml(
    scheme: &str,
    summary: &xcodebuild::TestSummary,
    terminations: &[Option<Termination>],
) -> String {
    use std::fmt::Write as _;
    use xcodebuild::CaseOutcome;
    let cases = junit_cases(summary, terminations);
    let count = |outcome| cases.iter().filter(|c| c.outcome == outcome).count();
    let widen = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
    let (tests, failures, skipped) = if summary.test_cases.is_some() {
        (
            cases.len(),
            count(CaseOutcome::Failed),
            count(CaseOutcome::Skipped),
        )
    } else {
        (
            widen(summary.total_test_count),
            widen(summary.failed_tests),
            widen(summary.skipped_tests),
        )
    };
    let totals = format!("tests=\"{tests}\" failures=\"{failures}\" skipped=\"{skipped}\"");
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(xml, "<testsuites {totals}>");
    let _ = writeln!(xml, "  <testsuite name=\"{}\" {totals}>", xml_attr(scheme));
    for case in &cases {
        let (classname, name) = junit_names(&case.selector, scheme);
        let time = case
            .duration
            .map_or_else(String::new, |secs| format!(" time=\"{secs:.3}\""));
        let _ = write!(
            xml,
            "    <testcase classname=\"{}\" name=\"{}\"{time}",
            xml_attr(&classname),
            xml_attr(name)
        );
        match (case.outcome, case.messages.first()) {
            (CaseOutcome::Passed, _) => xml.push_str("/>\n"),
            (CaseOutcome::Skipped, None) => xml.push_str(">\n      <skipped/>\n    </testcase>\n"),
            (CaseOutcome::Skipped, Some(reason)) => {
                let _ = writeln!(
                    xml,
                    ">\n      <skipped message=\"{}\"/>\n    </testcase>",
                    xml_attr(reason)
                );
            }
            (CaseOutcome::Failed, first) => {
                let headline = first.and_then(|m| m.lines().next()).unwrap_or_default();
                let body: Vec<&str> = case
                    .messages
                    .iter()
                    .copied()
                    .chain(case.ended.as_deref())
                    .collect();
                let _ = writeln!(
                    xml,
                    ">\n      <failure message=\"{}\">{}</failure>\n    </testcase>",
                    xml_attr(headline),
                    xml_text(&body.join("\n\n"))
                );
            }
        }
    }
    xml.push_str("  </testsuite>\n</testsuites>\n");
    xml
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

/// `s` as XML character data: markup characters escaped, and each control
/// character XML 1.0 cannot carry at all (an ANSI escape's ESC, say) replaced
/// by U+FFFD.
fn xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if c < ' ' || matches!(c, '\u{FFFE}' | '\u{FFFF}') => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

/// `s` as a double-quoted XML attribute value: [`xml_text`], with quotes
/// escaped and line breaks and tabs as character references, since a parser
/// reads a literal one inside an attribute as a space.
fn xml_attr(s: &str) -> String {
    xml_text(s)
        .replace('"', "&quot;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
        .replace('\t', "&#9;")
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
    use crate::cli::testdir::TempDir;
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
    fn coverage_and_retry_flaky_replace_the_files_copy_of_their_flag() {
        // xcodebuild takes '-enableCodeCoverage' and '-test-iterations' once,
        // and '--coverage'/'--retry-flaky' pass their own.
        let (args, _) = parse_test(&["--coverage", "--retry-flaky", "3"]);
        let twins = flag_twins(&args);
        assert_eq!(
            twins,
            [
                ("--coverage", "-enableCodeCoverage"),
                ("--retry-flaky", "-test-iterations")
            ]
        );
        let file = [
            "-enableCodeCoverage",
            "NO",
            "-skipMacroValidation",
            "-test-iterations",
            "5",
        ]
        .map(String::from)
        .to_vec();
        let (kept, notes) = without_file_twins(&twins, &file);
        assert_eq!(kept, ["-skipMacroValidation"]);
        assert_eq!(
            notes[0],
            "leaving out sweetpad.toml's '-enableCodeCoverage NO': '--coverage' passes its own, \
             and xcodebuild takes '-enableCodeCoverage' only once"
        );
        assert_eq!(notes.len(), 2);
        // Without the typed flags, the file's copies stay.
        let (args, _) = parse_test(&[]);
        let (kept, notes) = without_file_twins(&flag_twins(&args), &file);
        assert_eq!(kept, file);
        assert!(notes.is_empty());

        // A value spelled like a twin is its flag's, as xcodebuild reads it,
        // and each twin's own value leaves with it.
        let file = [
            "-xcconfig",
            "-enableCodeCoverage",
            "-jobs",
            "-test-iterations",
            "-test-iterations",
            "5",
        ]
        .map(String::from)
        .to_vec();
        let (kept, notes) = without_file_twins(&twins, &file);
        assert_eq!(
            kept,
            [
                "-xcconfig",
                "-enableCodeCoverage",
                "-jobs",
                "-test-iterations"
            ]
        );
        assert_eq!(
            notes,
            [
                "leaving out sweetpad.toml's '-test-iterations 5': '--retry-flaky' passes its own, \
                 and xcodebuild takes '-test-iterations' only once"
            ]
        );
        for (_, theirs) in &twins {
            assert!(xcodebuild_args::takes_value(theirs), "{theirs}");
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
            assert_eq!(
                read_refused_flags(&args, |_| true),
                ["--failed", "--junit"],
                "{verb}"
            );
            let (args, _) = parse_test(&["--coverage", verb, "--", "-quiet"]);
            assert_eq!(
                read_refused_flags(&args, |_| true),
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
                read_refused_flags(&args, |_| true),
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
        let err = refuse_run_flags(&read_refused_flags(&args, |_| true), &read_reason("output"))
            .expect_err("--failed was not refused");
        assert_eq!(
            err.to_string(),
            "--failed applies to a test run: 'test output' reads the last run's result bundle \
             and runs nothing"
        );
        // A refused flag is a usage error, like the ones clap reports itself.
        assert_eq!(err.error_kind(), crate::cli::ErrorKind::Usage);
        assert_eq!(err.error_kind().exit_code(), 2);
    }

    #[test]
    fn the_read_back_verbs_refuse_a_destination_and_leave_it_out_of_their_help() {
        use clap::CommandFactory;
        // The bundle is the project's, whatever the run tested on, so a
        // destination picks nothing here, on either side of the verb.
        for verb in ["attachments", "output"] {
            let (args, _) = parse_test(&[verb, "--mac"]);
            assert_eq!(read_refused_flags(&args, |_| true), ["--mac"], "{verb}");
            let (args, _) = parse_test(&["--on", "booted", verb, "--destination", "id=X"]);
            assert_eq!(
                read_refused_flags(&args, |_| true),
                ["--on", "--destination"],
                "{verb}"
            );
            // Set by 'SWEETPAD_ON' or 'SWEETPAD_DESTINATION', not typed.
            assert!(read_refused_flags(&args, |_| false).is_empty(), "{verb}");

            let mut root = crate::cli::Cli::command();
            root.build();
            let help = root
                .find_subcommand_mut("test")
                .and_then(|test| test.find_subcommand_mut(verb))
                .expect("the verb")
                .render_long_help()
                .to_string();
            for flag in ["--mac", "--on", "--destination"] {
                assert!(!help.contains(&format!("{flag} ")), "{verb} lists {flag}");
                assert!(!help.contains(&format!("{flag}\n")), "{verb} lists {flag}");
            }
            assert!(help.contains("--result-bundle <PATH>"), "{verb}");
        }
        let (args, _) = parse_test(&["output", "--on", "mac", "--mac"]);
        let err = refuse_run_flags(&read_refused_flags(&args, |_| true), &read_reason("output"))
            .expect_err("--mac and --on were not refused");
        assert_eq!(
            err.to_string(),
            "--mac and --on apply to a test run: 'test output' reads the last run's result \
             bundle and runs nothing"
        );
        // A run still takes them.
        let (args, _) = parse_test(&["--on", "booted"]);
        assert_eq!(args.target.on.as_deref(), Some("booted"));
        let (args, _) = parse_test(&["run", "--mac"]);
        assert!(args.mac);
    }

    #[test]
    fn the_read_back_verbs_refuse_a_scheme_configuration_and_sdk() {
        use clap::CommandFactory;
        // The bundle is the project's whatever the run's scheme, configuration
        // or SDK was, so none of them picks one to read.
        for verb in ["attachments", "output"] {
            let (args, _) = parse_test(&[
                "--scheme",
                "App",
                verb,
                "--configuration",
                "Release",
                "--sdk",
                "macosx",
            ]);
            assert_eq!(
                read_refused_flags(&args, |_| true),
                ["--scheme", "--configuration", "--sdk"],
                "{verb}"
            );
            // Set by 'SWEETPAD_SCHEME', 'SWEETPAD_CONFIGURATION' or
            // 'SWEETPAD_SDK', not typed.
            assert!(read_refused_flags(&args, |_| false).is_empty(), "{verb}");

            let mut root = crate::cli::Cli::command();
            root.build();
            let help = root
                .find_subcommand_mut("test")
                .and_then(|test| test.find_subcommand_mut(verb))
                .expect("the verb")
                .render_long_help()
                .to_string();
            for flag in ["--scheme <", "--configuration <", "--sdk <"] {
                assert!(!help.contains(flag), "{verb} lists {flag}");
            }
            // The container still picks the project whose bundle is read.
            assert!(help.contains("--project <PROJECT>"), "{verb}");
        }
        let (args, _) = parse_test(&["output", "--scheme", "App"]);
        let err = refuse_run_flags(&read_refused_flags(&args, |_| true), &read_reason("output"))
            .expect_err("--scheme was not refused");
        assert_eq!(
            err.to_string(),
            "--scheme applies to a test run: 'test output' reads the last run's result bundle \
             and runs nothing"
        );
        // A run still takes them.
        let (args, _) = parse_test(&["run", "--scheme", "App", "--sdk", "macosx"]);
        assert_eq!(args.target.scheme.scheme.as_deref(), Some("App"));
        assert_eq!(args.target.sdk.as_deref(), Some("macosx"));
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
                assert!(read_refused_flags(&args, |_| true).is_empty(), "{argv:?}");
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
    fn a_named_bundle_reads_back_as_one_clean_absolute_path() {
        // `std::path::absolute` keeps a `..`, which would print
        // `--result-bundle ../x.xcresult` as the cwd with `/../x.xcresult` on
        // the end.
        let c = Container::Project(PathBuf::from("/work/App.xcodeproj"));
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            bundle_path(&c, Some(Path::new("../x.xcresult"))),
            cwd.parent().unwrap().join("x.xcresult")
        );
        assert_eq!(
            bundle_path(&c, Some(Path::new("/r/./a/../x.xcresult"))),
            PathBuf::from("/r/x.xcresult")
        );
        assert_eq!(bundle_path(&c, None), retained_bundle_path(&c));
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
        // The log slot is in a directory of the test's own rather than the
        // user's state.
        let dir = crate::cli::testdir::TempDir::new("sweetpad-test-log");
        let slot = dir.join("results/App-test.log");
        let outcome = xcodebuild::TestRunOutcome {
            passed: false,
            tail: Some(transcript.clone()),
            diagnostics: crate::cli::buildlog::diagnostics_from_transcript(&transcript),
            transcript: Some(transcript.clone()),
            blocker: None,
            streamed: false,
            parsed: true,
        };
        let err = build_step_failure(&slot, outcome, false);

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
        assert_eq!(log, slot);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), transcript);
    }

    #[test]
    fn a_failure_with_nothing_parseable_keeps_the_tail() {
        // No diagnostic means no better account exists — dropping the tail here
        // would leave the caller with a bare "failed before any test ran".
        let log = Path::new("/work/App-test.log");
        let outcome = xcodebuild::TestRunOutcome {
            passed: false,
            tail: Some("xcodebuild: error: Unable to find a destination".to_string()),
            diagnostics: Vec::new(),
            transcript: None,
            blocker: None,
            streamed: true,
            parsed: true,
        };
        let err = build_step_failure(log, outcome, false);
        assert!(err.to_string().contains("Unable to find a destination"));
        assert!(err.json().get("diagnostics").is_none());
        // Nothing on the stream explained it, so the terminal needs this error.
        assert!(!err.is_shown());
    }

    #[test]
    fn a_streamed_compile_error_is_not_restated_after_the_banner() {
        let log = Path::new("/work/App-test.log");
        let outcome = |streamed, blocker: Option<&str>| xcodebuild::TestRunOutcome {
            passed: false,
            tail: None,
            diagnostics: crate::cli::buildlog::diagnostics_from_transcript(
                "/work/App/Picker.swift:4:11: error: cannot find 'Missing' in scope\n",
            ),
            transcript: None,
            blocker: blocker.map(str::to_string),
            streamed,
            parsed: true,
        };
        let err = build_step_failure(log, outcome(true, None), false);
        assert!(err.is_shown());
        // The machine-readable object and the exit code still carry it.
        assert_eq!(err.json()["diagnostics"].as_array().map(Vec::len), Some(1));
        assert_eq!(err.error_kind().exit_code(), 3);

        // The captured modes showed nothing, and a blocker's hint says what
        // the stream did not.
        assert!(!build_step_failure(log, outcome(false, None), false).is_shown());
        assert!(!build_step_failure(log, outcome(true, Some("approve it")), false).is_shown());
    }

    /// With stderr in another file than the stream, the error carries the
    /// streamed errors itself, and points at 'build diagnostics' for the rest,
    /// since a test run whose build failed records it as 'build' does.
    #[test]
    fn a_streamed_compile_error_is_repeated_on_a_stderr_apart() {
        let log = Path::new("/work/App-test.log");
        let transcript = (1..=5)
            .map(|n| format!("/work/App/Picker.swift:{n}:11: error: cannot find 'M{n}' in scope\n"))
            .collect::<Vec<_>>()
            .concat();
        let outcome = xcodebuild::TestRunOutcome {
            passed: false,
            tail: None,
            diagnostics: crate::cli::buildlog::diagnostics_from_transcript(&transcript),
            transcript: None,
            blocker: None,
            streamed: true,
            parsed: true,
        };
        let err = build_step_failure(log, outcome, true);
        assert!(!err.is_shown());
        assert_eq!(
            err.to_string(),
            "running the tests: xcodebuild test failed before any test ran:\n  \
             error: /work/App/Picker.swift:1:11: cannot find 'M1' in scope\n  \
             error: /work/App/Picker.swift:2:11: cannot find 'M2' in scope\n  \
             error: /work/App/Picker.swift:3:11: cannot find 'M3' in scope\n  \
             and 2 more error(s)"
        );
        assert_eq!(err.tip_text(), Some(xcodebuild::DIAGNOSTICS_TIP));
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
        let dir = TempDir::new("sweetpad-att");
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
        // Every case looks identical on disk, and none is guessable: a
        // default that discards evidence, a green run, a red run whose
        // failing tests attached nothing.
        let all = empty_note(false, Some(3), false);
        assert!(all.contains(".keepAlways"), "{all}");
        assert!(all.contains("deleteOnSuccess"), "{all}");
        assert_eq!(
            empty_note(true, Some(0), true),
            "the run had no failing tests; drop '--only-failures' to export what the passing \
             tests attached"
        );
        let green = empty_note(true, Some(0), false);
        assert!(green.starts_with("the run had no failing tests and kept no attachments: "));
        assert!(green.contains(".keepAlways"), "{green}");
        // A red run never reads as one that may have passed.
        assert_eq!(
            empty_note(true, Some(5), true),
            "none of the run's 5 failing tests attached anything; drop '--only-failures' to \
             export what the passing tests attached"
        );
        assert_eq!(
            empty_note(true, Some(1), false),
            "the run's one failing test attached nothing"
        );
        // Without the tree only xcresulttool's own mark was there to go by.
        let unread = empty_note(true, None, true);
        assert!(
            unread.contains("could not read which tests failed"),
            "{unread}"
        );
        assert_eq!(empty_note(true, None, false), all);
        // Backticks render literally in a terminal.
        for (only_failures, failed, attached) in [
            (false, None, false),
            (true, Some(0), true),
            (true, Some(0), false),
            (true, Some(2), true),
            (true, None, true),
        ] {
            let note = empty_note(only_failures, failed, attached);
            assert!(!note.contains('`'), "{note}");
            assert!(!note.contains("may have passed"), "{note}");
        }
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
        let root = TempDir::new("sweetpad-rename");
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
                failed_test: false,
                timestamp: 1.0,
            },
            xcodebuild::ExportedAttachment {
                test: "ATests/testTwo()".into(),
                identifier: "AppTests/ATests/testTwo".into(),
                file: staging.join("missing"),
                suggested_name: "gone.png".into(),
                failure: false,
                failed_test: false,
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
    }

    #[test]
    fn a_failed_tests_attachments_are_marked_as_failure_evidence() {
        // Xcode 27 leaves xcresulttool's mark off what a failed test attached
        // itself, and off an iOS simulator UI test's crash log too, so the
        // mark alone would leave '(failure)' off them. The test tree's verdict
        // decides, and a mark still counts on its own.
        let root = TempDir::new("sweetpad-marked");
        let staging = root.join(".sweetpad-export");
        std::fs::create_dir_all(&staging).unwrap();
        let attachment = |test: &str, file: &str, failure, failed_test| {
            std::fs::write(staging.join(file), file).unwrap();
            xcodebuild::ExportedAttachment {
                test: format!("ATests/{test}()"),
                identifier: format!("AppTests/ATests/{test}"),
                file: staging.join(file),
                suggested_name: format!("{file}.txt"),
                failure,
                failed_test,
                timestamp: 1.0,
            }
        };
        let exported = vec![
            attachment("testCrashes", "crash-log", false, true),
            attachment("testMarked", "marked", true, false),
            attachment("testPasses", "note", false, false),
        ];
        let tests = rename_into_place(exported, &root).unwrap();
        let marks: Vec<(&str, bool)> = tests
            .iter()
            .flat_map(|t| &t.files)
            .map(|f| (f.name.as_str(), f.failure))
            .collect();
        assert_eq!(
            marks,
            [
                ("crash-log.txt", true),
                ("marked.txt", true),
                ("note.txt", false)
            ]
        );
        let report = AttachmentsReport {
            output_dir: root.to_path_buf(),
            tests,
            recorded_at: None,
            note: None,
        };
        assert_eq!(report.json()["tests"][0]["attachments"][0]["failure"], true);
        assert_eq!(
            report.json()["tests"][2]["attachments"][0]["failure"],
            false
        );
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
                started: None,
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
    fn test_output_warns_only_when_the_markers_overlapped() {
        let report = |serial, overlapped| OutputReport {
            tests: Vec::new(),
            unattributed: None,
            sources: vec![PathBuf::from("/state/App-output/AppTests.txt")],
            recorded_at: None,
            serial,
            overlapped,
            note: None,
        };
        // Parallel workers each write a stream of their own, so a run that
        // didn't say it was serial is read the same way as one that did.
        assert_eq!(report(false, false).overlap_warning(), None);
        assert_eq!(report(true, false).overlap_warning(), None);
        assert_eq!(
            report(false, true).overlap_warning().as_deref(),
            Some(
                "tests overlapped in one test process, so some lines may be listed under the \
                 wrong test; read /state/App-output for the output as written"
            )
        );
        let json = report(false, true).json();
        assert_eq!(json["overlapped"], true);
        assert_eq!(json["serial"], false);
    }

    #[test]
    fn lines_between_tests_are_counted_when_they_are_not_listed() {
        let sources = [PathBuf::from("/state/App-output/AppTests.txt")];
        assert_eq!(
            output_note(true, true, 2, &sources).as_deref(),
            Some(
                "2 lines written outside any test are not listed; read /state/App-output for \
                 the output as written"
            )
        );
        assert_eq!(
            output_note(true, true, 1, &sources).as_deref(),
            Some(
                "1 line written outside any test is not listed; read /state/App-output for the \
                 output as written"
            )
        );
        // Nothing between tests, or a report narrowed to some tests, says
        // nothing.
        assert_eq!(output_note(true, true, 0, &sources), None);
        assert_eq!(output_note(true, false, 2, &sources), None);
        // With nothing attributed, the lines are listed and the note says why
        // no test is.
        assert!(
            output_note(false, true, 2, &sources)
                .is_some_and(|n| n.starts_with("no test wrote to stdout or stderr"))
        );
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
    fn a_macos_host_that_vanished_is_the_runner() {
        // Xcode 27 on macOS 27, a hosted unit test: the host crashed as the
        // first test ran, as a later one, as a Swift Testing test, or called
        // `exit`, and XCTest gave up on a host that crashed again each time.
        for message in [
            "The test runner crashed while preparing to run tests: SweetpadB5Mac at \
             +[XCTFailableInvocation invokeStandardConventionInvocation:completion:]. \
             libsystem_c.dylib: abort() called",
            "Crash: SweetpadB5Mac at +[XCTFailableInvocation \
             invokeStandardConventionInvocation:completion:]. libsystem_c.dylib: abort() called",
            "Crash: SweetpadB5Mac at specialized static \
             Runner._applyScopingTraits(for:testCase:_:). libsystem_c.dylib: abort() called",
            "The test runner exited with code 3 before finishing running tests. This may be \
             due to your code calling 'exit', consider adding a symbolic breakpoint on 'exit' \
             to debug.",
            "Exceeded max restart count of 2. (Underlying Error: Crash: SweetpadB5Mac at \
             specialized static Runner._applyScopingTraits(for:testCase:_:). \
             libsystem_c.dylib: abort() called)",
            "Early unexpected exit, operation never finished bootstrapping - no restart will \
             be attempted. (Underlying Error: The test runner crashed before establishing \
             connection: SweetpadB6TestMac)",
            "Lost connection to the test runner",
        ] {
            assert_eq!(vanished(message), Some(Vanished::Runner), "{message}");
        }
        assert_eq!(
            vanished("dev.sweetpad.exitprobe.app crashed in -[ProbeView boom]"),
            Some(Vanished::Named("dev.sweetpad.exitprobe.app".into()))
        );
    }

    #[test]
    fn a_hostless_bundle_that_crashed_names_xctest() {
        // Xcode 27 on macOS 27, a unit-test bundle with no host app: a test
        // that crashed, and XCTest's own crash after a nested run.
        for message in [
            "Crash: xctest at static xctest.main()",
            "Crash: xctest at static xctest.main(). libsystem_c.dylib: abort() called",
            "Exceeded max restart count of 2. (Underlying Error: Crash: xctest at static \
             xctest.main())",
            "The test runner crashed before establishing connection: xctest",
        ] {
            assert_eq!(vanished(message), Some(Vanished::Xctest), "{message}");
        }
        // A host app whose name only starts like it is still the host.
        assert_eq!(
            vanished("Crash: xctestHost at -[AppDelegate boom]"),
            Some(Vanished::Runner)
        );
    }

    #[test]
    fn a_crash_in_xctest_never_takes_a_host_apps_exit() {
        // A hostless target crashed while a hosted one's app exited in the
        // same window, as in a macOS run of both targets at once.
        const HOST: &str = "dev.sweetpad.b8test.mac";
        let found = vec![exit_at(
            HOST,
            64890,
            "2026-09-27 14:56:07.500000+0200",
            "exited due to exit(0), ran for 2600ms",
        )];
        let at = found[0].epoch_seconds().expect("a time");
        let window = |cause| ExitWindow {
            cause,
            from: at - 5.0,
            until: at + 5.0,
        };
        let host = &mut || Some(HOST.to_string());
        assert!(window(Vanished::Runner).exit_in(&found, host).is_some());
        assert!(window(Vanished::Xctest).exit_in(&found, host).is_none());
    }

    #[test]
    fn a_failure_that_only_mentions_a_crash_is_not_one() {
        // An XCTFail and a Swift Testing issue whose text says "crashed", as
        // Xcode 27 recorded them, and other messages that share a word with
        // Xcode's but not its wording.
        for message in [
            "failed - simulated: the helper crashed",
            "Issue recorded: simulated: the helper crashed",
            "Expectation failed: worker.crashed == false",
            "failed - the server is not running",
            "Critical process com.apple.backboardd crashed in main",
            "Exceeded max restart count of 2. (Underlying Error: the helper crashed)",
            "Early unexpected exit, (Underlying Error: the helper crashed)",
            "Lost connection to testmanagerd",
            "Warning: The test runner exited with code 1 while the state machine was in state \
             Finished.",
        ] {
            assert_eq!(vanished(message), None, "{message}");
        }
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
            exits_command: None,
            crash_log_commands: Vec::new(),
            coverage: None,
            result_bundle: "/tmp/ExitProbe.xcresult".into(),
        }
    }

    #[test]
    fn a_vanished_app_with_no_exit_found_says_where_to_look() {
        // Both lookups came back without the exit, so the failure would say
        // only that the app is gone.
        let mut report = failed_report(vec![None, None]);
        report.exits_command = Some("'sweetpad app logs --exits --on F13C004A'".into());
        let note =
            "couldn't find launchd's exit record; try 'sweetpad app logs --exits --on F13C004A'";
        assert_eq!(report.failure_lines()[1], format!("      {note}"));
        let json = report.json();
        assert_eq!(json["failures"][0]["note"], note);
        assert!(json["failures"][0].get("terminationReason").is_none());
        // An ordinary failure has no exit to look for.
        assert!(json["failures"][1].get("note").is_none());
        assert_eq!(report.failure_lines().len(), 3);

        // A destination with no exit log to read (a device) has nowhere to
        // point.
        let device = failed_report(vec![None, None]);
        assert_eq!(device.failure_lines().len(), 2);
        assert!(device.json()["failures"][0].get("note").is_none());
    }

    #[test]
    fn a_crash_in_xctest_points_at_its_crash_log() {
        let mut report = failed_report(vec![None, None]);
        report.summary.test_failures[0] = xcodebuild::TestFailure {
            test_name: "testACrash()".into(),
            target_name: "B8TestHostless".into(),
            failure_text: "Crash: xctest at static xctest.main()".into(),
            test_identifier_string: "CrashTests/testACrash()".into(),
            ..Default::default()
        };
        report.exits_command = Some("'sweetpad app logs --exits --mac'".into());
        report.crash_log_commands = vec![
            Some(
                "'sweetpad test attachments --only-testing B8TestHostless/CrashTests/testACrash'"
                    .into(),
            ),
            None,
        ];
        let note = "the tests ran in 'xctest', which launchd logs no exit for; 'sweetpad test \
                    attachments --only-testing B8TestHostless/CrashTests/testACrash' exports the \
                    crash log XCTest attached";
        assert_eq!(report.failure_lines()[1], format!("      {note}"));
        assert_eq!(report.json()["failures"][0]["note"], note);
    }

    #[test]
    fn a_crash_in_xctest_takes_its_exit_from_the_attached_crash_log() {
        const TEST: &str = "B9MiscHostless/CrashTests/testBFatal";
        let saved = "/Users/me/Library/Logs/DiagnosticReports/xctest-2026-09-27-160417.ips";
        // What the crash log XCTest attached to a hostless bundle's
        // `fatalError` records, Xcode 27 on macOS 27.
        let exit = exits::Exit {
            time: "2026-09-27 16:04:16.3634+0200".into(),
            bundle_id: String::new(),
            pid: Some(13933),
            cause: exits::Cause::Signal {
                name: "SIGTRAP".into(),
                sent_by: Some("exc handler[13933]".into()),
            },
            explanation: None,
            exception: Some("EXC_BREAKPOINT".into()),
            ran_for_ms: Some(812),
            message: "Trace/BPT trap: 5".into(),
            origin: exits::Origin::CrashReport,
        };
        let mut report = failed_report(vec![
            Some(Termination {
                exit,
                crash_report: Some(PathBuf::from(saved)),
                crashed_in: Some(TEST.into()),
            }),
            None,
        ]);
        report.summary.test_failures[0] = xcodebuild::TestFailure {
            test_name: "testBFatal()".into(),
            target_name: "B9MiscHostless".into(),
            failure_text: "Crash: xctest at static xctest.main()".into(),
            test_identifier_string: "CrashTests/testBFatal()".into(),
            ..Default::default()
        };
        let export = format!("'sweetpad test attachments --only-testing {TEST}'");
        report.crash_log_commands = vec![Some(export.clone()), None];

        let lines = report.failure_lines();
        assert_eq!(
            lines[1],
            "      xctest terminated: crashed with SIGTRAP (sent by exc handler[13933]; \
             EXC_BREAKPOINT)"
        );
        // The crash is in this test, and the saved report is named, so
        // nothing more is said.
        assert_eq!(lines.len(), 3);
        let json = report.json();
        let failure = &json["failures"][0];
        assert_eq!(failure["crashedIn"], TEST);
        assert!(failure["terminationReason"]["bundleId"].is_null());
        assert_eq!(failure["terminationReason"]["source"], "crashReport");
        assert_eq!(failure["terminationReason"]["crashReport"], saved);
        assert!(failure.get("note").is_none());

        // With the saved report gone, the attached copy is the only one.
        if let Some(Some(t)) = report.terminations.get_mut(0) {
            t.crash_report = None;
        }
        let note = format!(
            "no crash report was found in DiagnosticReports; {export} exports the copy XCTest \
             attached"
        );
        assert_eq!(report.failure_lines()[2], format!("      {note}"));
        assert_eq!(report.json()["failures"][0]["note"], note);
    }

    #[test]
    fn the_exit_hint_reads_the_log_the_run_searched() {
        assert_eq!(
            ExitLog::Simulator("F13C004A".into()).exits_args(),
            ["--exits", "--on", "F13C004A"]
        );
        assert_eq!(ExitLog::Mac.exits_args(), ["--exits", "--mac"]);
    }

    #[test]
    fn a_faults_sender_and_exception_read_apart() {
        // launchd's line for a fault, with the exception its crash report adds.
        let line = serde_json::json!({
            "timestamp": "2026-09-26 20:37:03.524000+0200",
            "subsystem": "user/503/UIKitApplication:dev.sweetpad.ci.app[1a2b][rb-legacy] [91599]",
            "eventMessage": "exited due to SIGSEGV | sent by exc handler[91599], ran for 371ms",
        })
        .to_string();
        let mut exit = exits::parse_ndjson_line(&line, &[]).expect("an exit");
        exit.exception = Some("EXC_BAD_ACCESS KERN_INVALID_ADDRESS at 0x10".into());
        let termination = Termination {
            exit,
            crash_report: Some(PathBuf::from("/tmp/App.ips")),
            crashed_in: None,
        };
        assert_eq!(
            termination.line(),
            "app terminated: crashed with SIGSEGV (sent by exc handler[91599]; EXC_BAD_ACCESS \
             KERN_INVALID_ADDRESS at 0x10)"
        );
        let json = failed_report(vec![Some(termination)]).json();
        let reason = &json["failures"][0]["terminationReason"];
        assert_eq!(reason["sentBy"], "exc handler[91599]");
        assert_eq!(
            reason["exception"],
            "EXC_BAD_ACCESS KERN_INVALID_ADDRESS at 0x10"
        );
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
            crashed_in: None,
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
    fn a_crash_with_no_crash_report_says_why_there_may_be_none() {
        // Past its limit for an app, macOS saves no more of that app's crash
        // reports, so launchd's line is all a suite that keeps crashing it
        // gets: no report path, and no EXC_* detail.
        const APP: &str = "dev.sweetpad.exitprobe.app";
        let crash = || {
            exit_at(
                APP,
                79175,
                "2026-09-26 20:45:07.612000+0200",
                "exited due to SIGTRAP | sent by exc handler[79175], ran for 4517ms",
            )
        };
        let note = "no crash report was found; macOS may have reached its limit of crash \
                    reports for this app";
        let report = failed_report(vec![
            Some(Termination {
                exit: crash(),
                crash_report: None,
                crashed_in: None,
            }),
            None,
        ]);
        assert_eq!(
            report.failure_lines()[1..3],
            [
                "      app terminated: crashed with SIGTRAP (sent by exc handler[79175])"
                    .to_string(),
                format!("      {note}"),
            ]
        );
        let json = report.json();
        assert_eq!(json["failures"][0]["note"], note);
        assert!(json["failures"][1].get("note").is_none());

        // A crash whose report was found has nothing missing, and a kill never
        // had a report to miss.
        let reported = failed_report(vec![Some(Termination {
            exit: crash(),
            crash_report: Some(PathBuf::from("/tmp/App.ips")),
            crashed_in: None,
        })]);
        let killed = failed_report(vec![Some(Termination {
            exit: exit_at(
                APP,
                79644,
                "2026-09-26 17:54:45.694421+0200",
                "exited due to SIGKILL, ran for 5820ms",
            ),
            crash_report: None,
            crashed_in: None,
        })]);
        for report in [reported, killed] {
            let lines = report.failure_lines();
            assert_eq!(lines.len(), 3, "{lines:?}");
            assert!(report.json()["failures"][0].get("note").is_none());
        }
    }

    #[test]
    fn a_backtrace_frame_names_the_test_it_runs_through() {
        let tests = [
            "AppTests/ParallelSuite/a_recordsCrashed()".to_string(),
            "AppTests/ParallelSuite/b_crashesHost()".to_string(),
            "AppTests/Outer/Inner/param(n:)".to_string(),
            "AppTests/free()".to_string(),
            "AppTests/ArithmeticTests/testArithmetic".to_string(),
        ];
        let named = |symbol: &str| crashed_in(&[symbol.to_string()], &tests);
        // Frames from Xcode 27 crash reports of a host a Swift Testing test
        // crashed, directly and through a closure it scheduled.
        assert_eq!(
            named("ParallelSuite.b_crashesHost()").as_deref(),
            Some("AppTests/ParallelSuite/b_crashesHost()")
        );
        assert_eq!(
            named("closure #1 in ParallelSuite.b_crashesHost()").as_deref(),
            Some("AppTests/ParallelSuite/b_crashesHost()")
        );
        assert_eq!(
            named("Outer.Inner.param(n:)").as_deref(),
            Some("AppTests/Outer/Inner/param(n:)")
        );
        assert_eq!(named("free()").as_deref(), Some("AppTests/free()"));
        assert_eq!(
            named("@objc ArithmeticTests.testArithmetic()").as_deref(),
            Some("AppTests/ArithmeticTests/testArithmetic")
        );
        assert_eq!(
            named("-[ArithmeticTests testArithmetic]").as_deref(),
            Some("AppTests/ArithmeticTests/testArithmetic")
        );
        // The Swift Testing macro's thunk, a helper of the same name on
        // another type, and a frame with no function name.
        for symbol in [
            "static ParallelSuite.$s7AppTests13ParallelSuiteV13b_crashesHost0C0fMp_@Sendable ()",
            "Helper.b_crashesHost()",
            "__semwait_signal",
        ] {
            assert_eq!(named(symbol), None, "{symbol}");
        }
        // The innermost frame that is a test decides.
        let frames = [
            "_assertionFailure(_:_:file:line:flags:)".to_string(),
            "Helper.boom()".to_string(),
            "ParallelSuite.b_crashesHost()".to_string(),
            "ParallelSuite.a_recordsCrashed()".to_string(),
        ];
        assert_eq!(
            crashed_in(&frames, &tests).as_deref(),
            Some("AppTests/ParallelSuite/b_crashesHost()")
        );
    }

    /// Two Swift Testing tests of one suite running at once in a macOS host
    /// that crashed, as Xcode 27 recorded them: both failed, with the same
    /// message, and each failure found the host's one exit.
    fn parallel_crash(crashed_in: Option<&str>, crash_report: bool) -> TestReport {
        let failure = |name: &str| xcodebuild::TestFailure {
            test_name: format!("{name}()"),
            target_name: "AppTests".into(),
            failure_text: "Crash: App at specialized static \
                           Runner._applyScopingTraits(for:testCase:_:)"
                .into(),
            test_identifier_string: format!("ParallelSuite/{name}()"),
            test_identifier_url: format!(
                "test://com.apple.xcode/App/AppTests/ParallelSuite/{name}()"
            ),
            ..Default::default()
        };
        let termination = || Termination {
            exit: exit_at(
                "dev.sweetpad.app",
                18626,
                "2026-09-27 01:20:24.112000+0200",
                "exited due to SIGTRAP | sent by exc handler[18626], ran for 2504ms",
            ),
            crash_report: crash_report.then(|| PathBuf::from("/tmp/App.ips")),
            crashed_in: crashed_in.map(str::to_string),
        };
        TestReport {
            passed: false,
            summary: xcodebuild::TestSummary {
                result: "Failed".into(),
                total_test_count: 3,
                failed_tests: 2,
                test_failures: vec![failure("a_recordsCrashed"), failure("b_crashesHost")],
                ..Default::default()
            },
            terminations: vec![Some(termination()), Some(termination())],
            exits_command: None,
            crash_log_commands: Vec::new(),
            coverage: None,
            result_bundle: "/tmp/App.xcresult".into(),
        }
    }

    #[test]
    fn a_crash_that_failed_the_tests_beside_it_names_the_one_it_was_in() {
        const B: &str = "AppTests/ParallelSuite/b_crashesHost()";
        const C: &str = "AppTests/ParallelSuite/c_passes()";
        let report = parallel_crash(Some(B), true);
        let lines = report.failure_lines();
        assert_eq!(
            lines[2],
            format!("      the crash report's backtrace is in {B}, not this test")
        );
        // The test it was in gets no such line.
        assert_eq!(lines.len(), 5, "{lines:?}");
        assert!(lines[3].starts_with(&format!("  ✗ {B}: Crash: App")));
        let json = report.json();
        assert_eq!(json["failures"][0]["crashedIn"], B);
        assert_eq!(json["failures"][1]["crashedIn"], B);
        assert!(json["failures"][0].get("crashCandidates").is_none());

        // A crash in work a passed test left running fails only the test
        // running at the time, and its backtrace names the test that passed.
        let report = parallel_crash(Some(C), true);
        let lines = report.failure_lines();
        assert_eq!(
            lines[2],
            format!("      the crash report's backtrace is in {C}, not this test")
        );
        assert_eq!(
            lines[5],
            format!("      the crash report's backtrace is in {C}, not this test")
        );
    }

    #[test]
    fn a_crash_no_backtrace_pins_down_names_every_test_it_failed() {
        let candidates = serde_json::json!([
            "AppTests/ParallelSuite/a_recordsCrashed()",
            "AppTests/ParallelSuite/b_crashesHost()",
        ]);
        let report = parallel_crash(None, true);
        let lines = report.failure_lines();
        let line = "      2 tests failed with this one crash, and its backtrace is in none of them";
        assert_eq!(lines[2], line);
        assert_eq!(lines[5], line);
        let json = report.json();
        assert_eq!(json["failures"][0]["crashCandidates"], candidates);
        assert_eq!(json["failures"][1]["crashCandidates"], candidates);
        assert!(json["failures"][0].get("crashedIn").is_none());

        let report = parallel_crash(None, false);
        assert_eq!(
            report.failure_lines()[2],
            "      2 tests failed with this one crash, and without its crash report which one \
             caused it can't be told"
        );
        assert_eq!(report.json()["failures"][1]["crashCandidates"], candidates);

        // A crash that failed one test says nothing more about it.
        let mut report = parallel_crash(None, true);
        report.terminations[1] = None;
        assert_eq!(report.failure_lines().len(), 3);
        assert!(
            report.json()["failures"][0]
                .get("crashCandidates")
                .is_none()
        );
    }

    /// launchd's exit line for `bundle_id` at `time`, as `log show` gives it.
    fn exit_at(bundle_id: &str, pid: u32, time: &str, message: &str) -> exits::Exit {
        let line = serde_json::json!({
            "timestamp": time,
            "subsystem": format!("user/503/UIKitApplication:{bundle_id}[18a4][rb-legacy] [{pid}]"),
            "eventMessage": message,
        })
        .to_string();
        exits::parse_ndjson_line(&line, &[]).expect("an exit")
    }

    /// A UI test whose app crashed: the previous test's app killed as this
    /// one relaunched it, then the crash, then the runner's own exit.
    fn crash_run() -> (Vec<exits::Exit>, ExitWindow) {
        const APP: &str = "dev.sweetpad.ci.app";
        let found = vec![
            exit_at(
                APP,
                79101,
                "2026-09-26 20:23:06.537712+0200",
                "exited due to SIGKILL, ran for 5820ms",
            ),
            exit_at(
                APP,
                79175,
                "2026-09-26 20:23:11.299962+0200",
                "exited due to SIGTRAP | sent by exc handler[79175], ran for 4517ms",
            ),
            exit_at(
                "dev.sweetpad.ci.uitests.xctrunner",
                79090,
                "2026-09-26 20:23:21.581374+0200",
                "exited due to exit(1), ran for 27595ms",
            ),
        ];
        let at = |i: usize| found[i].epoch_seconds().expect("a time");
        // The test started just before the kill; XCTest noticed the crash
        // 3.6s after it, well before the runner went.
        let window = ExitWindow {
            cause: Vanished::Named(APP.into()),
            from: at(0) - 1.0,
            until: at(1) + 3.6 + EXIT_AFTER_FAILURE,
        };
        (found, window)
    }

    #[test]
    fn a_failure_takes_its_apps_last_exit_inside_the_window() {
        let (found, window) = crash_run();
        let exit = window
            .exit_in(&found, &mut || None)
            .expect("the crash was not found");
        assert_eq!(exit.pid, Some(79175));
        // The runner's exit lands after the window, and the test runner is
        // not whose exit an app's crash asks for anyway.
        let runner = ExitWindow {
            cause: Vanished::Runner,
            ..window
        };
        assert!(runner.exit_in(&found, &mut || None).is_none());
    }

    #[test]
    fn a_macos_failure_window_closes_at_the_failure() {
        // Two hosted unit-test runs on macOS 27. The host crashed, or called
        // `exit(3)`, and XCTest recorded the failure 4.5s or 0.3s later, then
        // restarted the host to run the next test, which passed; that host
        // exited cleanly just under half a second after the failure.
        const HOST: &str = "dev.sweetpad.b5.mac";
        for (exit, restarted, failed) in [
            (
                exit_at(
                    HOST,
                    34271,
                    "2026-09-27 00:01:43.425430+0200",
                    "exited due to SIGABRT | sent by SweetpadB5Mac[34271], ran for 542ms",
                ),
                exit_at(
                    HOST,
                    34334,
                    "2026-09-27 00:01:48.356533+0200",
                    "exited due to exit(0), ran for 415ms",
                ),
                1_790_460_107.891,
            ),
            (
                exit_at(
                    HOST,
                    35018,
                    "2026-09-27 00:02:40.512247+0200",
                    "exited due to exit(3), ran for 505ms",
                ),
                exit_at(
                    HOST,
                    35032,
                    "2026-09-27 00:02:41.246614+0200",
                    "exited due to exit(0), ran for 460ms",
                ),
                1_790_460_160.794,
            ),
        ] {
            let found = vec![exit.clone(), restarted.clone()];
            // The host is the runner. A unit test records no start, so the
            // window opens at the run's.
            let window = |after: f64| ExitWindow {
                cause: Vanished::Runner,
                from: failed - 30.0,
                until: failed + after,
            };
            let host = &mut || Some(HOST.to_string());
            let got = window(ExitLog::Mac.after_failure()).exit_in(&found, host);
            assert_eq!(got.and_then(|e| e.pid), exit.pid);
            // A simulator's margin would reach the restarted host's exit.
            let simulator = ExitLog::Simulator("U".into()).after_failure();
            let got = window(simulator).exit_in(&found, host);
            assert_eq!(got.and_then(|e| e.pid), restarted.pid);
        }
    }

    #[test]
    fn a_vanished_app_with_no_exit_found_gets_one_more_look() {
        // The first query failed (a timeout, say) or came back without the
        // crash; one wait and one more query recover it.
        for first in [None, Some(Vec::new())] {
            let (found, window) = crash_run();
            let mut answers = vec![first, Some(found)].into_iter();
            let (mut lookups, mut waits) = (0, 0);
            let exits = find_exits(
                &[Some(window), None],
                &mut || {
                    lookups += 1;
                    answers.next().flatten()
                },
                &mut || waits += 1,
                &mut || None,
            );
            assert_eq!((lookups, waits), (2, 1));
            assert_eq!(exits[0].as_ref().and_then(|e| e.pid), Some(79175));
            assert!(exits[1].is_none());
        }

        // Once is all it gets.
        let (_, window) = crash_run();
        let (mut lookups, mut waits) = (0, 0);
        let exits = find_exits(
            &[Some(window)],
            &mut || {
                lookups += 1;
                None
            },
            &mut || waits += 1,
            &mut || None,
        );
        assert_eq!((lookups, waits), (2, 1));
        assert!(exits[0].is_none());
    }

    #[test]
    fn a_run_whose_exits_are_all_found_never_waits() {
        let (found, window) = crash_run();
        let (mut lookups, mut waits) = (0, 0);
        let exits = find_exits(
            &[Some(window)],
            &mut || {
                lookups += 1;
                Some(found.clone())
            },
            &mut || waits += 1,
            &mut || None,
        );
        assert_eq!((lookups, waits), (1, 0));
        assert!(exits[0].is_some());

        // Nor does a run with no failure that says something vanished: it
        // never looks at all.
        let (mut lookups, mut waits) = (0, 0);
        let exits = find_exits(
            &[None, None],
            &mut || {
                lookups += 1;
                None
            },
            &mut || waits += 1,
            &mut || None,
        );
        assert_eq!((lookups, waits), (0, 0));
        assert!(exits.iter().all(Option::is_none));
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
        // With no test tree to read, the failures are all there is to list,
        // and the suite keeps the summary's totals.
        let dir = TempDir::new("sweetpad-junit");
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
            test_cases: None,
        };
        write_junit(&path, "App", &summary, &[]).unwrap();
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains("tests=\"3\" failures=\"1\""));
        assert!(
            xml.contains("<testcase classname=\"AppTests.Suite\" name=\"testA&lt;&gt;\">"),
            "{xml}"
        );
        assert!(
            xml.contains(
                "<failure message=\"x &amp; y &quot;broke&quot;\">x &amp; y \"broke\"</failure>"
            ),
            "{xml}"
        );
        assert_eq!(xml.matches("<testcase ").count(), 1, "{xml}");
    }

    /// The fixture's run as the summary and the test tree give it: a pass, a
    /// skip, two failures, one of them with a second message, both multi-line.
    fn junit_summary() -> xcodebuild::TestSummary {
        use xcodebuild::{CaseOutcome, TestCase};
        let case = |identifier: &str, outcome, duration, messages: &[&str]| TestCase {
            identifier: identifier.into(),
            outcome,
            duration,
            messages: messages.iter().map(|m| (*m).to_string()).collect(),
            skip_message: (outcome == CaseOutcome::Skipped)
                .then(|| "Test skipped - not on this simulator".to_string()),
        };
        let two = [
            "XCTAssertEqual failed: (\"hello\") is not equal to (\"world\") - first <failure> & \
             \"quoted\"",
            "XCTAssertTrue failed - second failure\nwith a second line",
        ];
        xcodebuild::TestSummary {
            result: "Failed".into(),
            total_test_count: 4,
            passed_tests: 1,
            failed_tests: 2,
            skipped_tests: 1,
            test_failures: vec![
                xcodebuild::TestFailure {
                    test_name: "testTwoFailures()".into(),
                    target_name: "SweetpadCIAppTests".into(),
                    failure_text: two[0].into(),
                    test_identifier_string: "AppTests/testTwoFailures()".into(),
                    test_identifier_url: "test://com.apple.xcode/SweetpadCIApp/\
                                          SweetpadCIAppTests/AppTests/testTwoFailures"
                        .into(),
                    other_messages: vec![two[1].into()],
                },
                xcodebuild::TestFailure {
                    test_name: "suiteGreeting()".into(),
                    target_name: "SweetpadCIAppTests".into(),
                    failure_text: "Expectation failed: n == 1\nn → 2".into(),
                    test_identifier_string: "GreetingSuite/suiteGreeting()".into(),
                    test_identifier_url: "test://com.apple.xcode/SweetpadCIApp/\
                                          SweetpadCIAppTests/GreetingSuite/suiteGreeting()"
                        .into(),
                    ..Default::default()
                },
            ],
            test_cases: Some(vec![
                case(
                    "SweetpadCIAppTests/AppTests/testArithmetic",
                    CaseOutcome::Passed,
                    Some(0.003_32),
                    &[],
                ),
                case(
                    "SweetpadCIAppTests/AppTests/testSkipped",
                    CaseOutcome::Skipped,
                    Some(0.006_89),
                    &[],
                ),
                case(
                    "SweetpadCIAppTests/AppTests/testTwoFailures",
                    CaseOutcome::Failed,
                    Some(0.038_12),
                    &two,
                ),
                case(
                    "SweetpadCIAppTests/GreetingSuite/suiteGreeting()",
                    CaseOutcome::Failed,
                    None,
                    &["Expectation failed: n == 1\nn → 2"],
                ),
            ]),
        }
    }

    #[test]
    fn junit_lists_every_test_the_run_recorded() {
        // GitLab counts the <testcase> elements and ignores the suite's
        // totals, so a report of failures alone reads as a smaller run.
        let xml = junit_xml("SweetpadCIApp", &junit_summary(), &[]);
        assert_eq!(xml.matches("<testcase ").count(), 4, "{xml}");
        assert!(xml.contains("<testsuites tests=\"4\" failures=\"2\" skipped=\"1\">"));
        assert!(
            xml.contains(
                "    <testcase classname=\"SweetpadCIAppTests.AppTests\" name=\"testArithmetic\" \
                 time=\"0.003\"/>\n"
            ),
            "{xml}"
        );
        assert!(
            xml.contains(
                "    <testcase classname=\"SweetpadCIAppTests.AppTests\" name=\"testSkipped\" \
                 time=\"0.007\">\n      <skipped message=\"Test skipped - not on this \
                 simulator\"/>\n    </testcase>\n"
            ),
            "{xml}"
        );
        // A case the tree gives no time goes without the attribute.
        assert!(
            xml.contains(
                "<testcase classname=\"SweetpadCIAppTests.GreetingSuite\" name=\"suiteGreeting()\">"
            ),
            "{xml}"
        );
    }

    #[test]
    fn a_junit_failure_carries_every_message_in_its_body() {
        // An XML parser turns a raw line break inside an attribute into a
        // space, so `message` holds the first line and the body holds the text.
        let xml = junit_xml("SweetpadCIApp", &junit_summary(), &[]);
        assert!(
            xml.contains(
                "<testcase classname=\"SweetpadCIAppTests.AppTests\" name=\"testTwoFailures\" \
                 time=\"0.038\">\n      <failure message=\"XCTAssertEqual failed: \
                 (&quot;hello&quot;) is not equal to (&quot;world&quot;) - first \
                 &lt;failure&gt; &amp; &quot;quoted&quot;\">XCTAssertEqual failed: (\"hello\") \
                 is not equal to (\"world\") - first &lt;failure&gt; &amp; \"quoted\"\n\n\
                 XCTAssertTrue failed - second failure\nwith a second line</failure>\n    \
                 </testcase>\n"
            ),
            "{xml}"
        );
        assert!(
            xml.contains(
                "<failure message=\"Expectation failed: n == 1\">Expectation failed: n == 1\nn → \
                 2</failure>"
            ),
            "{xml}"
        );
        // No `message` spans a line.
        for attribute in xml.split("message=\"").skip(1) {
            let value = attribute.split('"').next().unwrap_or_default();
            assert!(!value.contains('\n'), "{value}");
        }
    }

    #[test]
    fn a_failure_the_tree_does_not_list_still_reaches_the_report() {
        let mut summary = junit_summary();
        summary.test_failures.push(xcodebuild::TestFailure {
            test_name: "testGone()".into(),
            target_name: "SweetpadCIAppUITests".into(),
            failure_text: "Lost connection to the test runner".into(),
            test_identifier_string: "AppUITests/testGone()".into(),
            ..Default::default()
        });
        let xml = junit_xml("SweetpadCIApp", &summary, &[]);
        assert!(xml.contains("<testsuites tests=\"5\" failures=\"3\" skipped=\"1\">"));
        assert!(
            xml.contains(
                "<testcase classname=\"SweetpadCIAppUITests.AppUITests\" name=\"testGone\">\n      \
                 <failure message=\"Lost connection to the test runner\">"
            ),
            "{xml}"
        );
    }

    #[test]
    fn a_junit_failure_says_what_ended_its_app() {
        // The summary prints the exit under the failure, and a CI reader of
        // the report has only the report.
        let mut summary = junit_summary();
        summary.test_failures.push(xcodebuild::TestFailure {
            test_name: "testGone()".into(),
            target_name: "SweetpadCIAppUITests".into(),
            failure_text: "Lost connection to the test runner".into(),
            test_identifier_string: "AppUITests/testGone()".into(),
            ..Default::default()
        });
        let mut crash = exit_at(
            "dev.sweetpad.ci.app",
            91599,
            "2026-09-26 20:37:03.524000+0200",
            "exited due to SIGSEGV | sent by exc handler[91599], ran for 371ms",
        );
        crash.exception = Some("EXC_BAD_ACCESS KERN_INVALID_ADDRESS at 0x10".into());
        let runner = exit_at(
            "dev.sweetpad.ci.uitests.xctrunner",
            79090,
            "2026-09-26 20:23:21.581374+0200",
            "exited due to exit(1), ran for 27595ms",
        );
        let terminations = [
            Some(Termination {
                exit: crash,
                crash_report: None,
                crashed_in: None,
            }),
            None,
            Some(Termination {
                exit: runner,
                crash_report: None,
                crashed_in: None,
            }),
        ];
        let xml = junit_xml("SweetpadCIApp", &summary, &terminations);
        // After every message, a blank line apart, and never in `message`.
        assert!(
            xml.contains(
                "XCTAssertTrue failed - second failure\nwith a second line\n\napp terminated: \
                 crashed with SIGSEGV (sent by exc handler[91599]; EXC_BAD_ACCESS \
                 KERN_INVALID_ADDRESS at 0x10)</failure>"
            ),
            "{xml}"
        );
        assert!(
            xml.contains(
                "<failure message=\"Lost connection to the test runner\">Lost connection to the \
                 test runner\n\ntest runner terminated: exited with status 1</failure>"
            ),
            "{xml}"
        );
        // A failure with no exit keeps its messages alone.
        assert!(
            xml.contains(
                "<failure message=\"Expectation failed: n == 1\">Expectation failed: n == 1\nn → \
                 2</failure>"
            ),
            "{xml}"
        );
        assert_eq!(xml.matches("terminated: ").count(), 2, "{xml}");
    }

    #[test]
    fn xml_escaping_keeps_what_a_parser_would_lose() {
        assert_eq!(xml_text("a < b && c > d"), "a &lt; b &amp;&amp; c &gt; d");
        // XML 1.0 has no way to write most control characters, escaped or not.
        assert_eq!(xml_text("\u{1b}[31mred\u{0}"), "\u{FFFD}[31mred\u{FFFD}");
        assert_eq!(xml_text("tab\there\nline"), "tab\there\nline");
        assert_eq!(
            xml_attr("say \"hi\"\n\tthen go"),
            "say &quot;hi&quot;&#10;&#9;then go"
        );
    }
}
