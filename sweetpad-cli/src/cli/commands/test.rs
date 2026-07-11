//! `sweetpad test …` — run tests via `xcodebuild test`, the sibling of
//! `build`. Streams output in human mode; emits a parsed pass/fail summary
//! under `--json` (and per-test events under `-o ndjson`), read back from the
//! `.xcresult` bundle. The bundle is **retained** (state dir, or
//! `--result-bundle PATH`) so failure attachments stay inspectable and
//! `test run --failed` can rerun just the last run's failures.

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

    /// Extra arguments passed to xcodebuild verbatim (after `--`).
    #[arg(last = true, value_name = "XCODEBUILD_ARGS", global = true)]
    pub passthrough: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Run the resolved scheme's tests (the default action: `sweetpad test`).
    Run,
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

pub fn run(ctx: &mut Context, args: &TestArgs) -> CommandResult {
    ctx.targeting = args.target.clone().into();
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
        let detail = outcome
            .tail
            .map_or_else(String::new, |tail| format!(":\n{tail}"));
        return Err(CliError::new(format!(
            "xcodebuild test failed before any test ran{detail}"
        ))
        .kind(crate::cli::ErrorKind::BuildFailure)
        .context("running the tests"));
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

/// Where a project's latest `.xcresult` is retained: one slot per project in
/// the state dir (stem + key hash, so two `App.xcodeproj`s never share it).
fn retained_bundle_path(container: &Container) -> PathBuf {
    xcodebuild::project_artifact(container, ".xcresult")
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
