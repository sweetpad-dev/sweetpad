//! `sweetpad test …` — run tests via `xcodebuild test`, the sibling of
//! `build`. Streams output in human mode; emits a parsed pass/fail summary
//! under `--json`, read back from the `.xcresult` bundle.

use std::time::{SystemTime, UNIX_EPOCH};

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::{CommandResult, Context, Render, Rendered, resolve, swiftpm, xcodebuild};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Run the resolved scheme's tests.
    Run {
        /// Run only this test identifier (Target[/Class[/method]]); repeatable.
        #[arg(long = "only-testing")]
        only_testing: Vec<String>,
        /// Skip this test identifier; repeatable.
        #[arg(long = "skip-testing")]
        skip_testing: Vec<String>,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Run {
            only_testing,
            skip_testing,
        } => test(ctx, only_testing, skip_testing),
    }
}

/// The xcodebuild test result: a pass/fail summary line and failure list in
/// human mode, the parsed counts + failures as JSON `data`.
struct TestReport {
    passed: bool,
    summary: xcodebuild::TestSummary,
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

fn test(ctx: &mut Context, only_testing: &[String], skip_testing: &[String]) -> CommandResult {
    // Tests resolve their own context (testing overrides, falling back to build).
    let resolved = resolve::resolve_testing(ctx)?;

    // Swift packages run tests with the `swift` toolchain — no simulator
    // destination, no `.xcresult` bundle to read a summary from.
    if matches!(resolved.container, resolve::Container::SwiftPackage(_)) {
        return spm_test(ctx, &resolved, only_testing, skip_testing);
    }

    let target = resolve::build_target(ctx, &resolved)?;
    resolve::remember_testing(ctx, &resolved, &target);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let bundle = std::env::temp_dir().join(format!(
        "sweetpad-test-{}-{nanos}.xcresult",
        std::process::id()
    ));

    let plan = xcodebuild::TestPlan {
        container: &resolved.container,
        scheme: &target.scheme,
        configuration: &target.configuration,
        destination: Some(&target.destination),
        only_testing,
        skip_testing,
        result_bundle: &bundle,
    };

    if !ctx.out.is_json() {
        ctx.out.note(&format!(
            "testing {} for {}",
            target.scheme, target.destination
        ));
    }

    // Human mode beautifies output; JSON stays quiet so stdout holds only the
    // enveloped summary the dispatcher renders from the returned payload.
    let passed = plan.run(&ctx.out)?;
    let summary = xcodebuild::test_summary(&bundle)?;
    let _ = std::fs::remove_dir_all(&bundle);

    let report = TestReport { passed, summary };
    if passed {
        Ok(Rendered::data(report))
    } else {
        // A red suite still renders its summary, but exits 3 (build/test failure).
        Ok(Rendered::data_with_exit(report, 3))
    }
}

/// Run a Swift package's tests via `swift test`. Unlike xcodebuild there's no
/// `.xcresult` to parse, so the result is just the pass/fail flag.
fn spm_test(
    ctx: &mut Context,
    resolved: &resolve::Resolved,
    only_testing: &[String],
    skip_testing: &[String],
) -> CommandResult {
    let configuration = resolved
        .configuration
        .clone()
        .unwrap_or_else(|| "Debug".to_string());

    if !ctx.out.is_json() {
        ctx.out.note(&format!(
            "testing Swift package ({configuration}) with swift test"
        ));
    }

    let passed = swiftpm::test(
        &resolved.container,
        &configuration,
        only_testing,
        skip_testing,
        ctx.out.is_json(),
    )?;

    let report = SpmTestReport { passed };
    if passed {
        Ok(Rendered::data(report))
    } else {
        Ok(Rendered::data_with_exit(report, 3))
    }
}
