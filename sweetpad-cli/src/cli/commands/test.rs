//! `sweetpad test …` — run tests via `xcodebuild test`, the sibling of
//! `build`. Streams output in human mode; emits a parsed pass/fail summary
//! under `--json` (and per-test events under `-o ndjson`), read back from the
//! `.xcresult` bundle. The bundle is **retained** (state dir, or
//! `--result-bundle PATH`) so `test run --failed` can rerun just the last run's
//! failures and `test attachments` can export what those tests attached —
//! screenshots and UI dumps, which for a UI test are the diagnosis the failure
//! message only points at.

use std::path::{Path, PathBuf};

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::resolve::Container;
use crate::cli::{
    CliError, CommandResult, Context, Render, Rendered, resolve, swiftpm, xcodebuild,
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
    /// Export the last run's attachments (screenshots, UI dumps) as files.
    Attachments(AttachmentsArgs),
}

/// The flags of `test attachments`. Which tests to export comes from the
/// global '--only-testing', so it reads the same as it does on a run.
#[derive(Debug, clap::Args)]
pub struct AttachmentsArgs {
    /// Where to write the files (default: a directory beside the retained
    /// result bundle, replacing the previous export).
    #[arg(long, value_name = "DIR")]
    pub output_dir: Option<PathBuf>,

    /// Export only the attachments recorded against a failing test.
    #[arg(long)]
    pub only_failures: bool,
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
    if let Some(Action::Attachments(opts)) = action {
        return attachments(ctx, args, opts);
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
        passthrough: &args.passthrough,
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

/// The xcodebuild test result: a pass/fail summary line and failure list in
/// human mode, the parsed counts + failures + retained bundle path as JSON.
struct TestReport {
    passed: bool,
    summary: xcodebuild::TestSummary,
    coverage: Option<f64>,
    result_bundle: String,
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
        for f in &self.summary.test_failures {
            out.line(&format!(
                "  ✗ {}/{}: {}",
                f.target_name, f.test_name, f.failure_text
            ));
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
            .map(|f| {
                serde_json::json!({
                    "test": f.test_name,
                    "target": f.target_name,
                    "message": f.failure_text,
                })
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
                "--failed/--result-bundle/--junit need an .xcresult bundle; `swift test` \
                 produces none for a Swift package",
            ));
        }
        if args.retry_flaky.is_some() {
            return Err(CliError::new(
                "--retry-flaky is xcodebuild's -retry-tests-on-failure; `swift test` has no \
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
        return Err(build_step_failure(&resolved.container, outcome));
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

    let report = TestReport {
        passed,
        summary,
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
    CliError::new(format!(
        "xcodebuild test failed before any test ran{detail}"
    ))
    .kind(crate::cli::ErrorKind::BuildFailure)
    .diagnostics(outcome.diagnostics)
    .context("running the tests")
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
    test: String,
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
            out.line(&format!("  {}", test.test));
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
                serde_json::json!({ "test": t.test, "attachments": files })
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
        exported.retain(|a| {
            args.only_testing
                .iter()
                .any(|sel| selector_matches(sel, &a.test))
        });
        if exported.is_empty() && found_any {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(no_selector_match(&bundle, &staging, &args.only_testing));
        }
    }

    // The run order is the timestamp order: the manifest lists a test's
    // attachments in neither the order they were taken nor a stable one.
    exported.sort_by(|a, b| {
        a.test
            .cmp(&b.test)
            .then(a.timestamp.total_cmp(&b.timestamp))
    });

    let mut tests: Vec<TestAttachments> = Vec::new();
    for item in exported {
        let dir = output_dir.join(test_dir_name(&item.test));
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
            Some(last) if last.test == item.test => last.files.push(file),
            _ => tests.push(TestAttachments {
                test: item.test,
                files: vec![file],
            }),
        }
    }
    let _ = std::fs::remove_dir_all(&staging);

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

/// `--only-testing` matched nothing. A selector naming a *target* is the usual
/// cause — the manifest knows tests by class — so the classes that do have
/// attachments are the correction, and they are far shorter than the full
/// identifier list.
fn no_selector_match(bundle: &Path, staging: &Path, selectors: &[String]) -> CliError {
    let mut classes: Vec<String> = xcodebuild::export_attachments(bundle, staging, false)
        .map(|list| {
            list.into_iter()
                .map(|a| {
                    a.test
                        .split_once('/')
                        .map_or(a.test.clone(), |(class, _)| class.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    classes.sort();
    classes.dedup();
    let known = match classes.len() {
        0 => String::new(),
        n if n > 5 => format!(
            "; classes with attachments: {}, and {} more",
            classes[..5].join(", "),
            n - 5
        ),
        _ => format!("; classes with attachments: {}", classes.join(", ")),
    };
    CliError::new(format!(
        "no attachments matched {} — a test here is identified by class, not target{known}",
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
/// path component, with the `()` that XCTest identifiers carry trimmed.
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
        let _ = writeln!(
            xml,
            "    <testcase classname=\"{}\" name=\"{}\">\n      <failure message=\"{}\"/>\n    </testcase>",
            xml_escape(&f.target_name),
            xml_escape(&f.test_name),
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
    use std::path::PathBuf;

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
        };
        let err = build_step_failure(&container, outcome);
        assert!(err.to_string().contains("Unable to find a destination"));
        assert!(err.json().get("diagnostics").is_none());
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
            test_dir_name("ReflowUITests/testOpensAPDF()"),
            "ReflowUITests.testOpensAPDF"
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
            }],
        };
        write_junit(&path, "App", &summary).unwrap();
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains("tests=\"3\" failures=\"1\""));
        assert!(xml.contains("name=\"testA&lt;&gt;()\""));
        assert!(xml.contains("x &amp; y &quot;broke&quot;"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
