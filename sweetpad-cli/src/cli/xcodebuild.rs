//! Thin wrapper over `xcodebuild` for the build/run commands: assembling the
//! argument vector (mirroring the VS Code extension's proven invocation) and
//! reading back the build settings needed to locate and launch the built app.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sweetpad_core::app_locator::path_value;
pub use sweetpad_core::app_locator::{AppBundle, CommandLineSettings, Located, ProductKind};
use sweetpad_core::build_settings::BuildSettingsOptions;
use sweetpad_core::xcodebuild_args::{self, dangling_flag, last_value};
use sweetpad_lib::destination::DestinationSpec;

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

/// The `xcodebuild` action a command's passthrough goes to, which decides the
/// arguments sweetpad passes beside it. The commands that resolve settings
/// in-process instead of spawning `xcodebuild` (`settings show`, the `app`
/// verbs that find a built product, the editor's index) model a build, so
/// they name [`Action::Build`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Build,
    BuildForTesting,
    Test,
    Archive,
    Clean,
}

impl Action {
    fn as_arg(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::BuildForTesting => "build-for-testing",
            Self::Test => "test",
            Self::Archive => "archive",
            Self::Clean => "clean",
        }
    }
}

impl From<BuildAction> for Action {
    fn from(action: BuildAction) -> Self {
        match action {
            BuildAction::Build => Self::Build,
            BuildAction::BuildForTesting => Self::BuildForTesting,
        }
    }
}

/// An `xcodebuild` flag sweetpad passes itself: the flag, who passes it, what
/// it sets, and the sweetpad flag that sets it instead.
type OwnedFlag = (&'static str, &'static str, &'static str, &'static str);

/// The flags sweetpad passes itself on every action. `xcodebuild` refuses a
/// second copy of each ("option '-scheme' may only be provided once"). A
/// `-destination` is not here: `xcodebuild` takes several, and builds or tests
/// for each one.
const OWNED_FLAGS: [OwnedFlag; 5] = [
    ("-scheme", "sweetpad", "the scheme", "--scheme"),
    (
        "-configuration",
        "sweetpad",
        "the configuration",
        "--configuration",
    ),
    ("-sdk", "sweetpad", "the SDK", "--sdk"),
    ("-workspace", "sweetpad", "the workspace", "--workspace"),
    ("-project", "sweetpad", "the project", "--project"),
];

/// The flag only `test` passes itself. A build takes a typed
/// `-resultBundlePath` in place of its own (see [`BuildPlan::result_bundle`]),
/// since nothing reads the build's bundle back.
const OWNED_BY_TEST: [OwnedFlag; 1] = [(
    "-resultBundlePath",
    "'sweetpad test'",
    "the result bundle",
    "--result-bundle",
)];

/// The flags only `archive` passes itself. The archive step takes
/// `-archivePath` once and fails on `-exportOptionsPlist`; the export step,
/// which names both of the others, never sees the tail.
const OWNED_BY_ARCHIVE: [OwnedFlag; 3] = [
    (
        "-archivePath",
        "'sweetpad archive'",
        "the archive path",
        "--output-file",
    ),
    (
        "-exportPath",
        "'sweetpad archive'",
        "the export directory",
        "--output-file",
    ),
    (
        "-exportOptionsPlist",
        "'sweetpad archive'",
        "the export options",
        "--export-options",
    ),
];

/// Refuse a typed `--` argument that names what sweetpad passes `xcodebuild`
/// itself for `action`, naming the sweetpad flag to use instead. The typed
/// flags alone decide it, so it is a usage error, raised before the command
/// resolves its scheme or destination. Only the exact token counts:
/// `xcodebuild` reads `-scheme=App` as something other than a scheme.
///
/// # Errors
/// A usage error naming the first such argument.
pub fn refuse_owned_flags(action: Action, tail: &[String]) -> Result<(), CliError> {
    match xcodebuild_args::read(tail).find_map(|arg| owned_flag(action, arg.word)) {
        Some(owned) => Err(
            CliError::new(format!("{} after '--'", instead_of_owned(owned))).kind(ErrorKind::Usage),
        ),
        None => Ok(()),
    }
}

/// The flag sweetpad passes `xcodebuild` itself for `action` that `arg` is.
#[must_use]
pub fn owned_flag(action: Action, arg: &str) -> Option<&'static OwnedFlag> {
    let specific: &[OwnedFlag] = match action {
        Action::Test => &OWNED_BY_TEST,
        Action::Archive => &OWNED_BY_ARCHIVE,
        Action::Build | Action::BuildForTesting | Action::Clean => &[],
    };
    OWNED_FLAGS.iter().chain(specific).find(|f| arg == f.0)
}

/// What to pass in place of an owned flag: "sweetpad sets the scheme itself;
/// pass '--scheme' instead of '-scheme'".
#[must_use]
pub fn instead_of_owned((flag, who, what, instead): &OwnedFlag) -> String {
    format!("{who} sets {what} itself; pass '{instead}' instead of '{flag}'")
}

/// Refuse a typed `--` tail that ends with a flag still waiting for its
/// value. `xcodebuild` refuses it too ("option '-xcconfig' requires an
/// argument"), but the commands that read the tail without spawning it
/// (`settings show`, the `app` verbs that find a built product) would
/// otherwise read an earlier copy of the flag.
///
/// # Errors
/// A usage error naming the flag.
pub fn refuse_dangling_flag(tail: &[String]) -> Result<(), CliError> {
    match dangling_flag(tail) {
        Some(flag) => {
            Err(CliError::new(format!("'{flag}' after '--' needs a value")).kind(ErrorKind::Usage))
        }
        None => Ok(()),
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
    /// Hot-reload build: add [`sweetpad_core::hot::build_settings`] for the
    /// destination's SDK, `-Xlinker -interposable` (so dyld can swap symbols)
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
    ///
    /// A `-resultBundlePath` in the passthrough replaces it: the build writes
    /// that bundle instead and keeps it (see [`Self::typed_result_bundle`]).
    pub result_bundle: Option<PathBuf>,
    /// Extra arguments passed through to xcodebuild verbatim (everything after
    /// `--` on the command line) — the escape hatch for flags/settings the CLI
    /// doesn't model.
    pub passthrough: &'a [String],
}

/// The typed result bundles this process's builds were about to write, for
/// [`BuildPlan::prepare_result_bundle`] to tell a bundle it may replace from
/// one that was there first.
static TYPED_BUNDLES_WRITTEN: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

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
        if let Some(bundle) = self.own_result_bundle() {
            args.push("-resultBundlePath".into());
            args.push(bundle.display().to_string());
        }
        args.extend(container_args(self.container));
        if self.hot
            && let Some(sdk) = self
                .destination
                .and_then(sweetpad_core::hot::sdk_for_destination)
        {
            // Build settings (KEY=VALUE) after the action, shared with the VS
            // Code extension. A sandbox declared in an entitlements file the
            // stripped copy doesn't replace is beyond build settings: the mac
            // preflight catches that case with instructions.
            args.extend(
                sweetpad_core::hot::build_settings(sdk, self.hot_entitlements)
                    .into_iter()
                    .map(|(key, value)| format!("{key}={value}")),
            );
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

    /// The `-resultBundlePath` the passthrough gives, as `xcodebuild` resolves
    /// it from [`working_dir`]. It replaces [`Self::result_bundle`], so a
    /// build writes its bundle where the caller asked, and the activity log
    /// the editor's index reads is written all the same.
    fn typed_result_bundle(&self) -> Option<PathBuf> {
        passthrough_path(self.passthrough, "-resultBundlePath", self.container)
    }

    /// The slot sweetpad passes as `-resultBundlePath`, unless the passthrough
    /// names its own.
    fn own_result_bundle(&self) -> Option<&PathBuf> {
        self.result_bundle
            .as_ref()
            .filter(|_| self.typed_result_bundle().is_none())
    }

    /// Clear the result-bundle slot: `xcodebuild` refuses to write into a path
    /// that already exists, and the slot is reused across builds. Call this
    /// immediately before spawning — never on the `--show-command` path, which
    /// must not touch state.
    ///
    /// A typed bundle is the caller's, so it is cleared only when an earlier
    /// build of this same process wrote it (the next round of `build
    /// --watch`, or a rebuild in the run session). One that was there before
    /// the command started stays, for `xcodebuild` to refuse.
    pub fn prepare_result_bundle(&self) {
        if let Some(typed) = self.typed_result_bundle() {
            let mut written = TYPED_BUNDLES_WRITTEN
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if written.contains(&typed) {
                let _ = std::fs::remove_dir_all(&typed);
            } else if !typed.exists() {
                written.push(typed);
            }
            return;
        }
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
                    &project_artifact(self.container, "-build.log"),
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
            record_build(self.container, Some(self.scheme), ok, &diagnostics);
        }
        if ok {
            // One tally for every parsing mode, so the `-o json` envelope and
            // the `-o ndjson` result line carry the same fields.
            Ok(parsed.then(|| buildlog::BuildStats::tally(&diagnostics, start.elapsed())))
        } else {
            // Classified here, the one chokepoint every build goes through, so
            // `build start` and `app run`'s build step both exit 3 on a failed
            // compile instead of the generic 1.
            Err(build_failure(
                &parts,
                diagnostics,
                blocker,
                streamed,
                !Output::streams_share_a_file(),
                &failure_detail,
            )
            .context("building the project"))
        }
    }
}

/// Where a failed build's error, repeating only its first few errors, sends
/// the reader for the rest: the record every parsed build leaves behind.
pub(crate) const DIAGNOSTICS_TIP: &str =
    "run 'sweetpad build diagnostics' to see every error and warning";

/// The error a failed build reports, for [`BuildPlan::run`] and for the
/// interactive `app run` session, which spawns xcodebuild itself. A blocked
/// build ([`buildlog::BlockerWatch`]) leads with the way past it, and stays on
/// the terminal even under a streamed log, since no diagnostic in that log
/// explains it. `detail` is what a captured run appends to the headline, and
/// `streamed` says whether the beautified log already rendered the errors.
/// It rendered them to stdout, so they sit just above this error only while
/// stderr writes to the same file. `stderr_apart` says it does not
/// (`2>err.log`, or stdout piped away; see [`Output::streams_share_a_file`]),
/// and the error then repeats the first few ([`repeated_errors`]) and points
/// at `build diagnostics` for the rest instead of being
/// [`shown`](CliError::shown).
pub(crate) fn build_failure(
    args: &[String],
    diagnostics: Vec<serde_json::Value>,
    blocker: Option<String>,
    streamed: bool,
    stderr_apart: bool,
    detail: &str,
) -> CliError {
    let streamed_error = blocker.is_none() && streamed_an_error(streamed, &diagnostics);
    let shown = streamed_error && !stderr_apart;
    let repeated = (streamed_error && stderr_apart).then(|| repeated_errors(&diagnostics));
    let headline = blocker.map_or_else(
        || {
            format!(
                "xcodebuild exited with a non-zero status{}",
                repeated.as_deref().unwrap_or(detail)
            )
        },
        |hint| format!("the build is blocked, not broken: {hint}"),
    );
    // A device's own reason for failing outranks the list of every error.
    let tip = device_tip(args, &diagnostics)
        .or_else(|| repeated.is_some().then(|| DIAGNOSTICS_TIP.to_string()));
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
    let spec = xcodebuild_args::values(args, "-destination")
        .map(DestinationSpec::parse)
        .find(DestinationSpec::is_device)?;
    let device = spec.id.or(spec.name).map_or_else(String::new, |d| {
        // The tip is single-quoted, so a name that needs quoting gets double
        // quotes inside it.
        if shell_quote(&d) == d {
            format!(" {d}")
        } else {
            format!(" \"{d}\"")
        }
    });
    Some(format!(
        "run 'sweetpad device info{device}' to see why the device isn't ready"
    ))
}

/// `-workspace <path>` / `-project <path>`; nothing for a Swift package (it's
/// driven from the package directory). Shared with the `dependency` command's
/// `-resolvePackageDependencies` invocation.
///
/// `xcodebuild` runs in [`working_dir`], so a relative container is named by
/// its file name there: a `--project link/App.xcodeproj` typed in `link`'s
/// parent runs `xcodebuild -project App.xcodeproj` in `link`. An absolute one
/// reads the same from anywhere and stays as given.
pub(crate) fn container_args(container: &Container) -> Vec<String> {
    let path = container.path();
    let named = if path.is_absolute() {
        path
    } else {
        path.file_name().map_or(path, Path::new)
    };
    let named = named.display().to_string();
    match container {
        Container::Workspace(_) => vec!["-workspace".into(), named],
        Container::Project(_) => vec!["-project".into(), named],
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
    /// The diagnostics the run's build step printed, parsed from its output up
    /// to the first test line, so a run that died in its build step reports
    /// the compile errors as data rather than as a log, and one whose tests
    /// ran records its build's warnings. Empty under `-v`, whose raw
    /// passthrough is not parsed.
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
    /// The output was parsed for diagnostics, as in every mode but `-v`'s raw
    /// passthrough, so the run's build has a record to write for `build
    /// diagnostics`, as [`BuildPlan::run`] does.
    pub parsed: bool,
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
                parsed: true,
            }
        } else if out.is_json() {
            let run = process::run_captured("xcodebuild", &args, cwd.as_deref())?;
            TestRunOutcome {
                passed: run.success,
                tail: (!run.success).then_some(run.tail),
                blocker: (!run.success)
                    .then(|| buildlog::blocker_from_transcript(&run.combined))
                    .flatten(),
                diagnostics: buildlog::diagnostics_from_transcript(&run.combined),
                transcript: (!run.success).then_some(run.combined),
                streamed: false,
                parsed: true,
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
                parsed: false,
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
                parsed: true,
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
    record_build(container, None, ok, diagnostics);
}

/// [`record_build_diagnostics`], naming the scheme the build ran, so a later
/// command that has none can say which one it was (a typed `--scheme` is not
/// remembered). `test` records its build step through this too.
pub(crate) fn record_build(
    container: &Container,
    scheme: Option<&str>,
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
        "scheme": scheme,
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

/// Park a failed run's raw transcript at `path`, the project's artifact slot
/// for it ([`project_artifact`]), and return the path, so an error can name the
/// log instead of quoting it. The captured (`--json`) modes send nothing to the
/// terminal, so without this the transcript only survives inside the error
/// message — the thing that makes the message unreadable. Best-effort: `None`
/// when the write fails, and the caller falls back to the tail.
pub(crate) fn record_failure_transcript(path: &Path, text: &str) -> Option<std::path::PathBuf> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, text).ok().map(|()| path.to_path_buf())
}

/// What a captured (`--json`) build failure appends to its headline: the
/// summarized diagnostics plus `log`, the path the transcript is parked at, or
/// the raw tail when nothing parsed.
///
/// Empty for a `blocked` run, which parks nothing. The blocker headline
/// replaces this detail wholesale, so a transcript written here would be a file
/// no message goes on to name — unfindable on disk, and several KB of package
/// resolution that the blocker already accounts for better.
fn captured_failure_detail(
    log: &Path,
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
            let log = record_failure_transcript(log, combined)
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

/// How many of the errors a log streamed to stdout [`repeated_errors`] repeats.
const REPEATED_ERRORS: usize = 3;

/// The errors a beautified log streamed to stdout, for an error that stderr
/// prints somewhere else (`2>err.log`), where those lines are not in front of
/// it: `:` and then the first [`REPEATED_ERRORS`] distinct errors, one per
/// indented line as `build diagnostics` prints them, then a count of the rest,
/// which that command reads back. Only the first line of a message is kept,
/// as in [`diagnostics_summary`].
pub(crate) fn repeated_errors(diagnostics: &[serde_json::Value]) -> String {
    use std::fmt::Write as _;

    let mut errors: Vec<(Option<&str>, &str)> = Vec::new();
    for d in diagnostics.iter().filter(|d| d["severity"] == "error") {
        let message = d["message"]
            .as_str()
            .and_then(|m| m.lines().next())
            .map_or("(no message)", |line| line.trim_end_matches(':'));
        let error = (d["location"].as_str(), message);
        if !errors.contains(&error) {
            errors.push(error);
        }
    }
    let mut detail = String::from(":");
    for (location, message) in errors.iter().take(REPEATED_ERRORS) {
        let line = buildlog::diagnostic_line(&buildlog::DiagKind::Error, *location, message, false);
        let _ = write!(detail, "\n  {line}");
    }
    if errors.len() > REPEATED_ERRORS {
        let _ = write!(
            detail,
            "\n  and {} more error(s)",
            errors.len() - REPEATED_ERRORS
        );
    }
    detail
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

/// The scheme the project's last recorded build ran, if the record names one.
#[must_use]
pub fn last_build_scheme(container: &Container) -> Option<String> {
    last_build_diagnostics(container)?
        .get("scheme")?
        .as_str()
        .map(str::to_string)
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
    /// Every test case the run recorded, passed ones included, in tree order.
    /// The summary lists only failures, so these come from the test tree, and
    /// only when [`test_summary`] reads it and the tree can be read.
    #[serde(skip)]
    pub test_cases: Option<Vec<TestCase>>,
}

/// How a test case ended, as the test tree records it. An expected failure
/// counts as passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseOutcome {
    Passed,
    Failed,
    Skipped,
}

impl CaseOutcome {
    fn from_result(result: Option<&str>) -> Self {
        match result {
            Some(r) if r.eq_ignore_ascii_case("failed") => Self::Failed,
            Some(r) if r.eq_ignore_ascii_case("skipped") => Self::Skipped,
            _ => Self::Passed,
        }
    }
}

/// One test case of a run, from its test tree.
#[derive(Debug)]
pub struct TestCase {
    /// As `-only-testing:` takes it (`Target/Class/method`).
    pub identifier: String,
    pub outcome: CaseOutcome,
    /// How long it ran, in seconds, when the tree says.
    pub duration: Option<f64>,
    /// Each distinct failure message it recorded, in order.
    pub messages: Vec<String>,
    /// Why it was skipped, when it was and the tree says why.
    pub skip_message: Option<String>,
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
    outcome: CaseOutcome,
    duration: Option<f64>,
    /// Each distinct failure message recorded under the case, in tree order.
    messages: Vec<&'a str>,
    /// The first `Skip Message` recorded under the case.
    skip_message: Option<&'a str>,
}

impl TreeCase<'_> {
    /// Whether the case is a test an `-only-testing:` selector can name. Every
    /// test has a `test://` URL, and a tree too old to carry URLs names its
    /// tests without spaces. The case XCTest records for a test process that
    /// failed outside any test has neither ([`FailedTests::outside_tests`]).
    fn is_a_test(&self) -> bool {
        match self.url {
            Some(url) => url.starts_with("test://"),
            None => !self.identifier.contains(char::is_whitespace),
        }
    }
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
                messages_under(node, "Failure Message", &mut messages);
                let mut skipped = Vec::new();
                messages_under(node, "Skip Message", &mut skipped);
                out.push(TreeCase {
                    target,
                    identifier,
                    url: field("nodeIdentifierURL"),
                    outcome: CaseOutcome::from_result(field("result")),
                    duration: node
                        .get("durationInSeconds")
                        .and_then(serde_json::Value::as_f64),
                    messages,
                    skip_message: skipped.first().copied(),
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

/// The messages of the `kind` nodes (`Failure Message`, `Skip Message`) under
/// `node`, whether a test case holds them itself or through the runs below it
/// (a parameterized test's `Arguments`, a retried test's `Repetition`). A
/// retry records the same message again, so each is kept once.
fn messages_under<'a>(node: &'a serde_json::Value, kind: &str, out: &mut Vec<&'a str>) {
    let children = node.get("children").and_then(serde_json::Value::as_array);
    for child in children.into_iter().flatten() {
        if child.get("nodeType").and_then(serde_json::Value::as_str) == Some(kind) {
            if let Some(message) = child.get("name").and_then(serde_json::Value::as_str)
                && !out.contains(&message)
            {
                out.push(message);
            }
        } else {
            messages_under(child, kind, out);
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

/// The failures of a run's test tree, as `--failed` reruns them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FailedTests {
    /// Each failed test, as `-only-testing:` takes it.
    pub selectors: Vec<String>,
    /// The failures that name no test, as the tree names them. A test
    /// process that fails outside any test is recorded as a case of its own,
    /// `<App> (<pid>) encountered an error`, under a `System Failures` suite
    /// and with no `test://` URL. Given that as a selector, xcodebuild runs
    /// no test and reports success.
    pub outside_tests: Vec<String>,
}

/// What `--failed` reruns from `bundle`, read from its test tree. A missing
/// bundle yields nothing ("no previous run").
pub fn failed_tests(bundle: &Path) -> Result<FailedTests, CliError> {
    if !bundle.exists() {
        return Ok(FailedTests::default());
    }
    let root = test_tree(bundle).context("reading the previous run's failures")?;
    Ok(failed_selectors(&root))
}

fn failed_selectors(root: &serde_json::Value) -> FailedTests {
    let mut cases = Vec::new();
    tree_cases(root, None, &mut cases);
    let mut failed = FailedTests::default();
    for case in cases.iter().filter(|c| c.outcome == CaseOutcome::Failed) {
        if case.is_a_test() {
            let selector = test_selector(case.target, case.identifier, case.url);
            failed.selectors.push(selector);
        } else {
            failed.outside_tests.push(case.identifier.to_string());
        }
    }
    for list in [&mut failed.selectors, &mut failed.outside_tests] {
        list.sort();
        list.dedup();
    }
    failed
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
    /// The tests that failed, as `-only-testing:` takes them with any `()`
    /// trimmed, so a name spelled without its URL still matches.
    failed: BTreeSet<String>,
}

impl TestTargets {
    fn from_tree(root: &serde_json::Value) -> Self {
        let mut cases = Vec::new();
        tree_cases(root, None, &mut cases);
        let mut by_test: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut by_url = BTreeMap::new();
        let mut failed = BTreeSet::new();
        for case in cases {
            if case.outcome == CaseOutcome::Failed {
                let selector = test_selector(case.target, case.identifier, case.url);
                failed.insert(selector.trim_end_matches("()").to_string());
            }
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
        Self {
            by_test,
            by_url,
            failed,
        }
    }

    /// Whether the test `identifier` names (as [`Self::identifier`] gives it)
    /// failed.
    fn failed(&self, identifier: &str) -> bool {
        self.failed.contains(identifier.trim_end_matches("()"))
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
///
/// Whenever it reads the tree, it also fills in [`TestSummary::test_cases`].
/// `every_case` asks for those on a green run too, which costs it the tree
/// read it otherwise skips.
pub fn test_summary(bundle: &Path, every_case: bool) -> Result<TestSummary, CliError> {
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
    if (every_case || !summary.test_failures.is_empty())
        && let Ok(root) = test_tree(bundle)
    {
        add_other_messages(&mut summary, &root);
        summary.test_cases = Some(test_cases(&root));
    }
    Ok(summary)
}

/// Every test case in a test tree, in tree order, named as `-only-testing:`
/// takes it.
fn test_cases(root: &serde_json::Value) -> Vec<TestCase> {
    let mut cases = Vec::new();
    tree_cases(root, None, &mut cases);
    cases
        .into_iter()
        .map(|c| TestCase {
            identifier: test_selector(c.target, c.identifier, c.url),
            outcome: c.outcome,
            duration: c.duration,
            messages: c.messages.into_iter().map(str::to_string).collect(),
            skip_message: c.skip_message.map(str::to_string),
        })
        .collect()
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
    /// Recorded against a test failure rather than a passing step, as
    /// xcresulttool marks it. Xcode 27 marks the crash log of a macOS test
    /// whose host crashed, but nothing an iOS simulator UI test recorded, its
    /// crash log included, and nothing a failed test attached itself.
    pub failure: bool,
    /// The test that recorded it failed, as the test tree says.
    pub failed_test: bool,
    /// Seconds since the epoch — the run order the test recorded them in,
    /// which the manifest's own order does not preserve.
    pub timestamp: f64,
}

/// What [`export_attachments`] staged, and how many of the run's tests failed.
pub struct AttachmentExport {
    pub attachments: Vec<ExportedAttachment>,
    /// The failed tests the test tree lists; `None` when it can't be read.
    pub failed_tests: Option<usize>,
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

/// Export all of a `.xcresult`'s attachments into `staging` and read the
/// manifest back, flattened to one entry per file. `staging` must be empty: a
/// second export into a populated directory writes `name (1).png` duplicates
/// rather than replacing what is there.
///
/// xcresulttool's own `--only-failures` is never passed. It keeps only the
/// files it marks as recorded against a failure. On Xcode 27 that is at most
/// the crash log of a macOS test whose host crashed: an iOS simulator UI
/// test's files go unmarked, its crash log and screen recording included, so
/// the flag exports nothing from a red run of UI tests. Each file says instead
/// whether its test failed, for the caller to pick by.
pub fn export_attachments(bundle: &Path, staging: &Path) -> Result<AttachmentExport, CliError> {
    let bundle_arg = bundle.to_string_lossy();
    let staging_arg = staging.to_string_lossy();
    let argv = [
        "xcresulttool",
        "export",
        "attachments",
        "--path",
        &bundle_arg,
        "--output-path",
        &staging_arg,
    ];
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
    // bundle; the tree adds the target and whether the test failed. When it
    // can't be read, a test is named without a target, counts as passed, and
    // its files are exported all the same.
    let targets = test_tree(bundle).map(|root| TestTargets::from_tree(&root));
    let failed_tests = targets.as_ref().ok().map(|t| t.failed.len());
    let targets = targets.unwrap_or_default();
    Ok(AttachmentExport {
        attachments: parse_attachment_manifest(&manifest_json, staging, &targets)?,
        failed_tests,
    })
}

/// The text of each crash log XCTest attached to test `test_id` in `bundle`,
/// oldest first. `test_id` is the test's `test://` URL or the id
/// `xcresulttool` takes as `--test-id`. On macOS, XCTest waits for the crash
/// report of a test process that crashed and attaches it to the failing test
/// as `Crash Log <date>.ips`. Only those files are exported, into a scratch
/// directory that goes when this returns. An export that fails gives none.
#[must_use]
pub fn attached_crash_logs(bundle: &Path, test_id: &str) -> Vec<String> {
    let Ok(staging) = sweetpad_core::scratch::ScratchDir::new("sweetpad-crash-log") else {
        return Vec::new();
    };
    let bundle_arg = bundle.to_string_lossy();
    let staging_arg = staging.to_string_lossy();
    let argv = [
        "xcresulttool",
        "export",
        "attachments",
        "--path",
        &bundle_arg,
        "--output-path",
        &staging_arg,
        "--test-id",
        test_id,
        "--filter",
        "Crash Log*",
    ];
    if process::capture("xcrun", &argv, None).is_err() {
        return Vec::new();
    }
    let Some(entries) = std::fs::read_to_string(staging.join("manifest.json"))
        .ok()
        .and_then(|json| serde_json::from_str::<Vec<AttachmentManifestEntry>>(&json).ok())
    else {
        return Vec::new();
    };
    let mut logs: Vec<ManifestAttachment> =
        entries.into_iter().flat_map(|e| e.attachments).collect();
    logs.sort_by(|a, b| a.timestamp.total_cmp(&b.timestamp));
    logs.iter()
        .filter_map(|a| std::fs::read_to_string(staging.join(&a.exported_file_name)).ok())
        .collect()
}

/// Flatten an attachment manifest to one entry per file, each test named by
/// the target `targets` puts it under and marked failed when it did.
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
            let failed_test = targets.failed(&identifier);
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
                    failed_test,
                    timestamp: a.timestamp,
                })
        })
        .collect())
}

/// What the tests themselves wrote, recovered from a `.xcresult`'s diagnostics.
pub struct RunOutput {
    /// Per test, in the order the tests started. Each test process writes a
    /// stream of its own, and a parallel run has one per worker, so the
    /// streams are merged on [`TestOutput::started`]. Only tests that wrote
    /// something appear.
    pub tests: Vec<TestOutput>,
    /// Output written outside any test case — setup, teardown, and any
    /// framework whose markers this parser does not recognise. Kept rather
    /// than dropped, so nothing the run wrote goes missing without a word.
    pub unattributed: String,
    /// How many non-blank lines of [`Self::unattributed`] a stream wrote
    /// while its tests ran: after its first case started and before its last
    /// one ended. What a hosted app logs as it launches comes before that,
    /// and a stream with no case in it has none.
    pub between_tests: usize,
    /// The files it was read from, for the part that doesn't fit in a payload.
    pub sources: Vec<PathBuf>,
    /// Whether every target reported running its tests serially. A parallel
    /// run's workers each write a stream of their own, one test at a time, so
    /// this alone says nothing about attribution; [`Self::overlapped`] does.
    pub serial: bool,
    /// Whether the case markers in some stream crossed rather than nested
    /// (see [`split_output`]), which leaves some lines under the wrong test.
    pub overlapped: bool,
}

pub struct TestOutput {
    /// `Class/method`, the shape the attachment manifest uses (the marker's
    /// own `Module.Class` spelling is normalized here).
    pub test: String,
    /// `Target/Class/method`, the shape `--only-testing` takes.
    pub identifier: String,
    pub output: String,
    /// When the test started, in seconds on the test process's clock read as
    /// UTC, so only good for ordering. The stream dates each suite's start,
    /// and a test starts once the tests before it in its suite have taken
    /// their time. `None` before the stream's first suite banner.
    pub started: Option<f64>,
}

/// One of XCTest's case markers, read apart.
struct CaseMarker<'a> {
    /// The class's module, when the marker qualifies it (an Objective-C
    /// class is not).
    module: Option<&'a str>,
    /// `Class/method`.
    test: String,
    started: bool,
    /// How long the test took, from an end marker's `(0.405 seconds)`.
    seconds: Option<f64>,
}

/// XCTest brackets each test's console output with these, on the test
/// process's own stdout: `Test Case '-[Module.Class method]' started.` … then
/// `passed`/`failed`/`skipped`. Everything between is what that test wrote.
/// Only XCTest's form counts ([`sweetpad_core::test_markers`]): Swift Testing
/// runs a process's tests at the same time, so its lines bracket nothing.
fn parse_case_marker(line: &str) -> Option<CaseMarker<'_>> {
    use sweetpad_core::test_markers::{self, Form, Status};
    let marker = test_markers::parse_case(line).filter(|m| m.form == Form::XCTest)?;
    // The marker spells the class module-qualified; the result bundle
    // does not, and names a nested class without its outer types.
    let class = marker.class?.rsplit('.').next()?;
    Some(CaseMarker {
        module: marker.module,
        test: format!("{class}/{}", marker.name),
        started: marker.status == Status::Started,
        seconds: marker.seconds.and_then(|s| s.parse().ok()),
    })
}

/// When a suite banner (`Test Suite 'AppTests' started at 2026-09-27
/// 14:56:05.763.`) says its suite started, as [`TestOutput::started`] counts.
fn suite_started(line: &str) -> Option<f64> {
    let (_, time) = line.rsplit_once("' started at ")?;
    crate::cli::exits::zoneless_seconds(time.strip_suffix('.').unwrap_or(time))
}

/// Split one test process's stdout into per-test slices, each named by the
/// target `targets` says ran it. Only tests that wrote something are added.
///
/// A line goes to the innermost test open when it was written. XCTest runs a
/// test on the thread that asked for it, so a test that runs another case
/// inside itself (`InnerTests(selector:).run()`) writes the inner case's
/// markers between its own: the inner case's lines sit between the inner
/// markers, and the outer test's lines resume once the inner case ends. A test
/// whose end never comes crashed and took its process with it. XCTest
/// restarts the process, so that test's lines run up to the next start.
///
/// Returns whether the markers crossed and how many lines outside any test
/// fell while the tests ran (see [`Split`]).
fn split_output(
    text: &str,
    targets: &TestTargets,
    into: &mut Vec<TestOutput>,
    unattributed: &mut String,
) -> Split {
    struct Open<'a> {
        module: Option<&'a str>,
        test: String,
        /// Its entry in `into`.
        entry: usize,
        ends: bool,
    }
    let lines: Vec<&str> = text.lines().collect();
    let markers: Vec<Option<CaseMarker>> = lines.iter().map(|l| parse_case_marker(l)).collect();
    let ends = ends_later(&markers);
    let first_start = markers
        .iter()
        .position(|m| m.as_ref().is_some_and(|m| m.started));
    let last_end = markers
        .iter()
        .rposition(|m| m.as_ref().is_some_and(|m| !m.started));
    let while_testing =
        |i: usize| first_start.is_some_and(|s| s < i) && last_end.is_some_and(|e| i < e);
    let first = into.len();
    let mut open: Vec<Open> = Vec::new();
    let mut split = Split::default();
    let mut after_suite = false;
    // When the next test to start does, by the suite banners and the
    // durations of the tests since. Rounding in the durations can run it past
    // the next banner, so no test starts before the one ahead of it.
    let mut clock: Option<f64> = None;
    let mut last_start: Option<f64> = None;
    for (i, ((line, marker), ends)) in lines.iter().zip(markers).zip(ends).enumerate() {
        if let Some(marker) = marker {
            after_suite = false;
            if marker.started {
                // An open test that never ends crashed, and this case runs in
                // the process XCTest restarted.
                while open.last().is_some_and(|o| !o.ends) {
                    open.pop();
                }
                let target = targets.target_of(&marker.test, marker.module);
                let started = match (clock, last_start) {
                    (Some(now), Some(last)) => Some(now.max(last)),
                    (now, last) => now.or(last),
                };
                last_start = started;
                into.push(TestOutput {
                    identifier: test_selector(target, &marker.test, None),
                    test: marker.test.clone(),
                    output: String::new(),
                    started,
                });
                open.push(Open {
                    module: marker.module,
                    test: marker.test,
                    entry: into.len() - 1,
                    ends,
                });
            } else {
                match open
                    .iter()
                    .rposition(|o| o.module == marker.module && o.test == marker.test)
                {
                    Some(at) => {
                        split.crossed |= at + 1 != open.len();
                        open.remove(at);
                        // A nested case's time is part of the outer test's.
                        if open.is_empty() {
                            clock = clock.zip(marker.seconds).map(|(t, s)| t + s);
                        }
                    }
                    None => split.crossed = true,
                }
            }
            continue;
        }
        // Suite banners, and the `Executed N tests` line under a suite's
        // end, are structure, not output. Keeping them would bury the
        // handful of real lines outside any test.
        if line.starts_with("Test Suite '") {
            let started = suite_started(line);
            if started.is_some() {
                clock = started;
            }
            after_suite = !line.contains("' started at ");
            continue;
        }
        if std::mem::take(&mut after_suite) && line.starts_with("\t Executed ") {
            continue;
        }
        let sink = if let Some(o) = open.last() {
            &mut into[o.entry].output
        } else {
            if while_testing(i) && !line.trim().is_empty() {
                split.between_tests += 1;
            }
            &mut *unattributed
        };
        sink.push_str(line);
        sink.push('\n');
    }
    let mut added = into.split_off(first);
    added.retain(|t| !t.output.trim().is_empty());
    into.append(&mut added);
    split
}

/// What [`split_output`] makes of a stream besides its per-test slices.
#[derive(Debug, Default, PartialEq, Eq)]
struct Split {
    /// Whether the markers crossed: a test that ends while a case that
    /// started after it is still open, or with none open, as two cases
    /// running at once in the one process write them. The lines between
    /// crossed markers could be either test's, so some land under the wrong
    /// one.
    crossed: bool,
    /// How many non-blank lines outside any test came after the stream's
    /// first case start and before its last case end.
    between_tests: usize,
}

/// For each of `markers`, whether it starts a test that ends later in the
/// stream: the next marker for the same test is its end rather than another
/// start. A test that crashed has none.
fn ends_later(markers: &[Option<CaseMarker>]) -> Vec<bool> {
    let mut ends = vec![false; markers.len()];
    let mut started: BTreeMap<(Option<&str>, &str), usize> = BTreeMap::new();
    for (i, marker) in markers.iter().enumerate() {
        let Some(marker) = marker else {
            continue;
        };
        let key = (marker.module, marker.test.as_str());
        if marker.started {
            started.insert(key, i);
        } else if let Some(start) = started.remove(&key) {
            ends[start] = true;
        }
    }
    ends
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
    let mut overlapped = false;
    let mut between_tests = 0;
    for path in &streams {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let split = split_output(&text, &targets, &mut tests, &mut unattributed);
        overlapped |= split.crossed;
        between_tests += split.between_tests;
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
    in_start_order(&mut tests);
    // Asserted only from the line that says so: an absent or differently
    // worded log leaves this unclaimed rather than guessed at.
    let serial = !schedules.is_empty()
        && schedules.iter().all(|p| {
            std::fs::read_to_string(p).is_ok_and(|s| s.contains("Parallelization disabled"))
        });

    Ok(RunOutput {
        tests,
        unattributed,
        between_tests,
        sources,
        serial,
        overlapped,
    })
}

/// Merge the tests of every stream by when each started. A stream is one
/// process's tests in the order it ran them, and the sort is stable, so it
/// keeps that order and only interleaves the streams.
fn in_start_order(tests: &mut [TestOutput]) {
    tests.sort_by(|a, b| {
        let at = |t: &TestOutput| t.started.unwrap_or(f64::NEG_INFINITY);
        at(a).total_cmp(&at(b))
    });
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

/// The part of the project file's `[xcodebuild] args` that `action` takes:
/// all of it but the flags `xcodebuild` fails on for that action, and their
/// values, with a note per flag left out. The file applies to every action, so
/// a test-only flag in it would otherwise break every build. The settings, the
/// `-xcconfig` and the package flags stay, so a `clean` still resolves the
/// products where the build put them. `tail` is what the invocation typed
/// after `--`, which can give the `-resultBundlePath` a `-resultStreamPath`
/// needs.
///
/// Both are read with [`xcodebuild_args::read`]: a flag that takes a
/// value takes the next argument, dashes and all, so `-xcconfig
/// -enableCodeCoverage` keeps a file named '-enableCodeCoverage', and a
/// `-derivedDataPath -resultBundlePath` names no result bundle.
///
/// Only the file's arguments are filtered: a flag typed for this run is the
/// caller's, and `xcodebuild` says why it refuses one.
#[must_use]
pub fn for_action(
    action: Action,
    configured: &[String],
    tail: &[String],
) -> (Vec<String>, Vec<String>) {
    let bundle_given = matches!(
        action,
        Action::Build | Action::BuildForTesting | Action::Test
    ) || last_value(tail, "-resultBundlePath").is_some();
    let mut kept = Vec::with_capacity(configured.len());
    let mut notes = Vec::new();
    let testing = matches!(action, Action::Test | Action::BuildForTesting);
    for arg in xcodebuild_args::read(configured) {
        let why = if !testing && TEST_ONLY_FLAGS.contains(&arg.word) {
            ", as a flag only testing takes"
        } else if !bundle_given && arg.word == "-resultStreamPath" {
            " without a '-resultBundlePath' to stream into"
        } else {
            kept.extend(arg.words().map(String::from));
            continue;
        };
        let value = arg.value.map_or_else(String::new, |v| format!(" {v}"));
        notes.push(format!(
            "leaving out sweetpad.toml's '{}{value}': 'xcodebuild {}' fails on it{why}",
            arg.word,
            action.as_arg()
        ));
    }
    (kept, notes)
}

/// The flags `xcodebuild` takes only when testing ("The flag
/// -enableCodeCoverage is only supported when testing"), as Xcode 27 refuses
/// them: `build`, `archive` and `clean` fail on them, and `build-for-testing`
/// takes them. Each takes a value, which [`for_action`] leaves out with it
/// because [`xcodebuild_args::VALUE_FLAGS`] lists the flag. The other testing flags
/// (`-test-iterations`, `-parallel-testing-enabled`, `-only-testing:`, …) are
/// accepted by every action. `-test-repetition-relaunch-enabled` fails every
/// action, `test` included, unless an iteration flag comes with it, and is
/// accepted by every action when one does.
const TEST_ONLY_FLAGS: [&str; 5] = [
    "-enableCodeCoverage",
    "-testPlan",
    "-testLanguage",
    "-testRegion",
    "-testProductsPath",
];

/// The value after the last `flag` in a passthrough, read by [`last_value`],
/// which the BSP server reads the extension's arguments with too, as a path,
/// joined onto [`working_dir`] when relative, the way `xcodebuild` running
/// there reads it (see [`path_value`]). A `-derivedDataPath build/dd` typed
/// in a nested source directory names a directory beside the project, not
/// below the caller.
fn passthrough_path(passthrough: &[String], flag: &str, container: &Container) -> Option<PathBuf> {
    path_value(passthrough, flag, working_dir(container).as_deref())
}

/// What `passthrough` adds to the build settings `xcodebuild` resolves:
/// [`CommandLineSettings::of`] reading it from the directory `xcodebuild`
/// runs in for `container` ([`working_dir`]), so a relative `-derivedDataPath`
/// or `-xcconfig` names what the build reads.
#[must_use]
pub fn command_line_settings(passthrough: &[String], container: &Container) -> CommandLineSettings {
    CommandLineSettings::of(passthrough, working_dir(container).as_deref())
}

/// Locate the app a build of `plan` writes, through the in-process resolver
/// (the engine behind `settings show`), with no `xcodebuild` spawn, and with
/// the passthrough's [`CommandLineSettings`]. Swift packages build no `.app`,
/// so they have nothing to locate here.
///
/// `app`'s `RunPlan` locates its install/launch bundle through this same
/// function, and so does `build -o json`'s `productPath`: one locator, so the
/// CLI cannot report one bundle and install another. The `app` verbs install
/// and launch `.app` bundles, so a scheme whose product is a command-line tool
/// has nothing for them.
pub fn located(plan: &BuildPlan<'_>) -> Result<Located, CliError> {
    let located =
        sweetpad_core::app_locator::locate(settings_options(plan)?).map_err(CliError::new)?;
    if located.kind != ProductKind::App {
        return Err(CliError::new(
            "could not find a launchable .app in the resolved build settings",
        ));
    }
    Ok(located)
}

/// The resolved settings of `target` in a build of `plan`, resolved the way
/// [`located`] resolves the app's. `None` when the build doesn't resolve it.
pub fn target_settings(
    plan: &BuildPlan<'_>,
    target: &str,
) -> Result<Option<BTreeMap<String, String>>, CliError> {
    let resolved = sweetpad_core::build_settings::resolve_build_settings(&settings_options(plan)?)
        .map_err(CliError::new)?;
    Ok(resolved
        .into_iter()
        .find(|t| t.target == target)
        .map(|t| t.settings))
}

/// The resolver options [`located`] resolves `plan` with.
fn settings_options(plan: &BuildPlan<'_>) -> Result<BuildSettingsOptions, CliError> {
    let (project, workspace) = match plan.container {
        Container::Project(p) => (Some(p.clone()), None),
        Container::Workspace(p) => (None, Some(p.clone())),
        Container::SwiftPackage(_) => {
            return Err(CliError::new("Swift packages have no .app bundle"));
        }
    };
    let command_line = command_line_settings(plan.passthrough, plan.container);
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
        // The build takes them too: a `PRODUCT_BUNDLE_IDENTIFIER=` or
        // `PRODUCT_NAME=`, on the command line or in an `-xcconfig`, changes
        // what gets installed and launched.
        xcconfig: command_line.xcconfig,
        xcode: None,
        xcspec_root: None,
        sdksettings_root: None,
        catalog_cache: None,
        derived_data_path: command_line.derived_data_path,
        overrides: command_line.overrides,
        // Callers install, launch, and report what this resolves, so it has to
        // name the bundle `xcodebuild` actually wrote, including when the user
        // has moved Derived Data in Xcode (issue #306).
        read_xcode_locations: true,
        keys: None,
    };
    Ok(opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::resolve::Container;
    use std::path::PathBuf;
    use sweetpad_lib::project::standardize;

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

    fn split(text: &str) -> (Split, Vec<TestOutput>, String) {
        let (mut tests, mut rest) = (Vec::new(), String::new());
        let split = split_output(text, &TestTargets::default(), &mut tests, &mut rest);
        (split, tests, rest)
    }

    /// XCTest ends a skipped test with `skipped` (Xcode 27.0), so what comes
    /// after it is no longer that test's.
    #[test]
    fn a_skipped_test_ends_at_its_skipped_marker() {
        let (found, tests, rest) = split(
            "Test Case '-[M.A testSkipped]' started.\n\
             before skip\n\
             /src/A.swift:19: -[M.A testSkipped] : Test skipped - not today\n\
             Test Case '-[M.A testSkipped]' skipped (0.002 seconds).\n\
             between\n\
             Test Case '-[M.A testB]' started.\n\
             b1\n\
             Test Case '-[M.A testB]' passed (0.1 seconds).\n",
        );
        assert!(!found.crossed);
        assert_eq!(tests[0].test, "A/testSkipped");
        assert_eq!(
            tests[0].output,
            "before skip\n/src/A.swift:19: -[M.A testSkipped] : Test skipped - not today\n"
        );
        assert_eq!(tests[1].output, "b1\n");
        assert_eq!(rest, "between\n");
    }

    #[test]
    fn only_markers_that_cross_count_as_overlapping() {
        assert!(!split(STREAM).0.crossed);

        // A test that crashed never ends; the next one's start closes it, and
        // each line is still under the test that wrote it.
        let (found, tests, _) = split(
            "Test Case '-[M.A testA]' started.\n\
             a1\n\
             Test Case '-[M.A testB]' started.\n\
             b1\n\
             Test Case '-[M.A testB]' passed (0.1 seconds).\n",
        );
        assert!(!found.crossed);
        assert_eq!(tests[0].output.trim(), "a1");
        assert_eq!(tests[1].output.trim(), "b1");

        // One test run inside another nests, and each line has one owner.
        let (found, tests, _) = split(
            "Test Case '-[M.A testA]' started.\n\
             a1\n\
             Test Case '-[M.A testB]' started.\n\
             b1\n\
             Test Case '-[M.A testB]' passed (0.1 seconds).\n\
             a2\n\
             Test Case '-[M.A testA]' passed (0.1 seconds).\n",
        );
        assert!(!found.crossed);
        assert_eq!(tests[0].output, "a1\na2\n");
        assert_eq!(tests[1].output, "b1\n");

        // Two tests open at once, each ending while the other is open: the
        // lines after the second start can't be told apart.
        let (found, ..) = split(
            "Test Case '-[M.A testA]' started.\n\
             Test Case '-[M.A testB]' started.\n\
             Test Case '-[M.A testA]' passed (0.1 seconds).\n\
             Test Case '-[M.A testB]' passed (0.1 seconds).\n",
        );
        assert!(found.crossed);
        // So does an end with nothing open.
        assert!(
            split("Test Case '-[M.A testA]' passed (0.1 seconds).\n")
                .0
                .crossed
        );
    }

    /// A real stream from a macOS unit-test bundle on Xcode 27: a test that
    /// runs another case inside itself, which XCTest warns about and then
    /// crashes on, and the restart XCTest makes after it.
    const NESTED_STREAM: &str = "\
Test Suite 'NestedTests' started at 2026-09-27 14:56:05.373.
Test Case '-[B8TestHostless.NestedTests testOuter]' started.
outer before
Test Case '-[B8TestHostless.InnerTests testInner]' started.
WARNING: Starting test case -[InnerTests testInner] while test case -[NestedTests testOuter] is still running
inner line
Test Case '-[B8TestHostless.InnerTests testInner]' passed (0.000 seconds).
outer after
WARNING: Test case -[NestedTests testOuter] finished which isn't running
Test Case '-[B8TestHostless.NestedTests testOuter]' passed (0.001 seconds).
2026-09-27 14:56:05.374526+0200 xctest[64888:23067538] [general] *** Assertion failure in -[XCTRunnerIDESession testCaseDidFinish:], XCTRunnerIDESession.m:680
libc++abi: terminate_handler unexpectedly threw an exception
Test Suite 'NestedTests' started at 2026-09-27 14:56:11.408.
Test Suite 'NestedTests' passed at 2026-09-27 14:56:11.409.
\t Executed 0 tests, with 0 failures (0 unexpected) in 0.000 (0.001) seconds
";

    #[test]
    fn a_line_goes_to_the_innermost_test_running() {
        let (found, tests, rest) = split(NESTED_STREAM);
        assert!(!found.crossed);
        let names: Vec<&str> = tests.iter().map(|t| t.test.as_str()).collect();
        assert_eq!(names, ["NestedTests/testOuter", "InnerTests/testInner"]);
        // The outer test's lines on both sides of the inner run are its own.
        assert_eq!(
            tests[0].output,
            "outer before\n\
             outer after\n\
             WARNING: Test case -[NestedTests testOuter] finished which isn't running\n"
        );
        assert!(
            tests[1].output.ends_with("inner line\n"),
            "{}",
            tests[1].output
        );
        // What the process wrote after its last test ended is outside any
        // test, while the suite's closing summary is structure.
        assert!(rest.contains("*** Assertion failure"), "{rest}");
        assert!(rest.contains("libc++abi"), "{rest}");
        assert!(!rest.contains("Executed"), "{rest}");
        // They came after the last test ended, so none fell between tests.
        assert_eq!(found.between_tests, 0);
    }

    #[test]
    fn only_lines_while_the_tests_ran_count_as_between_them() {
        // A hosted app's launch logging comes before the first test, and a
        // class's teardown between two tests.
        let (found, tests, rest) = split(
            "2026-09-27 14:56:04.900083+0200 B8TestMacApp[64890:23067600] [Connection] Unable \
             to get synchronousRemoteObjectProxy\n\
             Test Suite 'A' started at 2026-09-27 14:56:05.229.\n\
             Test Case '-[M.A testA]' started.\n\
             a\n\
             Test Case '-[M.A testA]' passed (0.001 seconds).\n\
             Test Suite 'A' passed at 2026-09-27 14:56:05.232.\n\
             \t Executed 1 test, with 0 failures (0 unexpected) in 0.001 (0.003) seconds\n\
             class teardown\n\
             \n\
             Test Case '-[M.B testB]' started.\n\
             b\n\
             Test Case '-[M.B testB]' passed (0.001 seconds).\n\
             after the last test\n",
        );
        assert_eq!(tests.len(), 2);
        assert!(rest.contains("synchronousRemoteObjectProxy"), "{rest}");
        assert!(rest.contains("after the last test"), "{rest}");
        assert_eq!(found.between_tests, 1);
        // A stream with no case in it, like the one holding only xcodebuild's
        // request for the result bundle, has no tests to be between.
        let (found, _, rest) = split(
            "\n\n*** If you believe this error represents a bug, please attach the result \
             bundle at /tmp/App.xcresult\n",
        );
        assert!(rest.contains("If you believe"), "{rest}");
        assert_eq!(found.between_tests, 0);
    }

    #[test]
    fn a_parallel_runs_workers_are_merged_by_when_each_test_started() {
        // Two workers' real streams from a macOS run with three workers.
        let first = "\
Test Suite 'BetaTests' started at 2026-09-27 14:56:05.760.
Test Case '-[B8TestHostless.BetaTests testOne]' started.
beta one
Test Case '-[B8TestHostless.BetaTests testOne]' passed (0.405 seconds).
Test Case '-[B8TestHostless.BetaTests testTwo]' started.
beta two
Test Case '-[B8TestHostless.BetaTests testTwo]' passed (0.407 seconds).
Test Suite 'BetaTests' passed at 2026-09-27 14:56:06.573.
\t Executed 2 tests, with 0 failures (0 unexpected) in 0.812 (0.814) seconds
Test Suite 'AlphaTests' started at 2026-09-27 14:56:06.579.
Test Case '-[B8TestHostless.AlphaTests testOne]' started.
alpha one
Test Case '-[B8TestHostless.AlphaTests testOne]' passed (0.412 seconds).
Test Case '-[B8TestHostless.AlphaTests testTwo]' started.
alpha two
Test Case '-[B8TestHostless.AlphaTests testTwo]' passed (0.404 seconds).
Test Suite 'AlphaTests' passed at 2026-09-27 14:56:07.395.
";
        let second = "\
Test Suite 'GammaTests' started at 2026-09-27 14:56:05.763.
Test Case '-[B8TestHostless.GammaTests testOne]' started.
gamma one
Test Case '-[B8TestHostless.GammaTests testOne]' passed (0.405 seconds).
Test Case '-[B8TestHostless.GammaTests testTwo]' started.
gamma two
Test Case '-[B8TestHostless.GammaTests testTwo]' passed (0.410 seconds).
Test Suite 'GammaTests' passed at 2026-09-27 14:56:06.580.
";
        let (mut tests, mut rest) = (Vec::new(), String::new());
        for stream in [first, second] {
            split_output(stream, &TestTargets::default(), &mut tests, &mut rest);
        }
        in_start_order(&mut tests);
        let names: Vec<&str> = tests.iter().map(|t| t.test.as_str()).collect();
        assert_eq!(
            names,
            [
                "BetaTests/testOne",
                "GammaTests/testOne",
                "BetaTests/testTwo",
                "GammaTests/testTwo",
                "AlphaTests/testOne",
                "AlphaTests/testTwo",
            ]
        );
        assert!(rest.is_empty(), "{rest}");
    }

    #[test]
    fn a_tests_start_counts_only_the_tests_before_it() {
        let started = |text: &str| {
            let (_, tests, _) = split(text);
            tests
                .iter()
                .map(|t| (t.test.clone(), t.started))
                .collect::<Vec<_>>()
        };
        let banner =
            crate::cli::exits::zoneless_seconds("2026-09-27 14:56:05.000").expect("parses");
        // An inner case's time is inside the outer test's, so only the outer
        // one moves the clock on.
        let tests = started(
            "Test Suite 'A' started at 2026-09-27 14:56:05.000.\n\
             Test Case '-[M.A testOuter]' started.\n\
             outer\n\
             Test Case '-[M.B testInner]' started.\n\
             inner\n\
             Test Case '-[M.B testInner]' passed (0.250 seconds).\n\
             Test Case '-[M.A testOuter]' passed (1.000 seconds).\n\
             Test Case '-[M.A testNext]' started.\n\
             next\n\
             Test Case '-[M.A testNext]' passed (0.100 seconds).\n",
        );
        let at = |i: usize| tests[i].1.expect("dated") - banner;
        assert!(at(0).abs() < 1e-6 && at(1).abs() < 1e-6, "{tests:?}");
        assert!((at(2) - 1.0).abs() < 1e-6, "{tests:?}");

        // Rounded durations can run past the next banner; the next test
        // still starts no earlier than the one before it.
        let tests = started(
            "Test Suite 'A' started at 2026-09-27 14:56:05.000.\n\
             Test Case '-[M.A testA]' started.\n\
             a\n\
             Test Case '-[M.A testA]' passed (0.406 seconds).\n\
             Test Suite 'B' started at 2026-09-27 14:56:05.405.\n\
             Test Case '-[M.B testB]' started.\n\
             b\n\
             Test Case '-[M.B testB]' passed (0.001 seconds).\n",
        );
        assert!(tests[1].1 >= tests[0].1, "{tests:?}");
        // A stream with no banner has nothing to date its tests by.
        let tests = started("Test Case '-[M.A testA]' started.\na\n");
        assert_eq!(tests[0].1, None);
    }

    #[test]
    fn a_crashed_tests_lines_run_to_the_restart() {
        // A real stream: the crash, XCTest's restart, and the next test.
        let (found, tests, rest) = split(
            "Test Suite 'CrashTests' started at 2026-09-27 15:00:10.509.\n\
             Test Case '-[B8TestHostless.CrashTests testACrash]' started.\n\
             about to crash\n\
             B8TestHostless/CrashTests.swift:10: Fatal error: b8 crash\n\
             \n\
             Restarting after unexpected exit, crash, or test timeout; summary will include \
             totals from previous launches.\n\
             \n\
             Test Suite 'Selected tests' started at 2026-09-27 15:00:12.408.\n\
             Test Suite 'CrashTests' started at 2026-09-27 15:00:12.409.\n\
             Test Case '-[B8TestHostless.CrashTests testBAfter]' started.\n\
             after the crash\n\
             Test Case '-[B8TestHostless.CrashTests testBAfter]' passed (0.001 seconds).\n\
             Test Suite 'CrashTests' passed at 2026-09-27 15:00:12.410.\n\
             \t Executed 1 test, with 0 failures (0 unexpected) in 0.001 (0.001) seconds\n\
             between\n",
        );
        assert!(!found.crossed);
        assert!(tests[0].output.contains("Fatal error: b8 crash"));
        assert!(tests[0].output.contains("Restarting after unexpected exit"));
        assert_eq!(tests[1].output, "after the crash\n");
        // The crashed test is not left open to take what comes after.
        assert_eq!(rest, "between\n");

        // A test run again after its crash ends only the second time.
        let (found, tests, rest) = split(
            "Test Case '-[M.A testA]' started.\n\
             first\n\
             Test Case '-[M.A testA]' started.\n\
             second\n\
             Test Case '-[M.A testA]' passed (0.1 seconds).\n\
             after\n",
        );
        assert!(!found.crossed);
        assert_eq!(tests[0].output, "first\n");
        assert_eq!(tests[1].output, "second\n");
        assert_eq!(rest, "after\n");
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
        // parking arm is the one a blocked build otherwise lands in. The log
        // slot is in a directory of the test's own rather than the user's
        // state.
        let dir = crate::cli::testdir::TempDir::new("sweetpad-parked-log");
        let log = dir.join("results/Blocked-build.log");
        let diagnostics = vec![diag("warning", Some("A.swift:1:1"), "unused variable 'x'")];

        let detail = captured_failure_detail(
            &log,
            "the whole transcript",
            "the tail",
            &diagnostics,
            /* blocked */ true,
        );
        assert!(detail.is_empty(), "{detail}");
        assert!(!log.exists(), "a blocked build parked {}", log.display());

        // The same failure, not blocked: the transcript is parked and named.
        let detail = captured_failure_detail(
            &log,
            "the whole transcript",
            "the tail",
            &diagnostics,
            /* blocked */ false,
        );
        assert!(detail.contains("unused variable 'x'"), "{detail}");
        assert!(
            detail.contains(&format!("full log: {}", log.display())),
            "{detail}"
        );
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "the whole transcript"
        );
        std::fs::remove_file(&log).unwrap();

        // Nothing parsed and not blocked: the tail is the only account there is.
        let detail = captured_failure_detail(&log, "transcript", "the tail", &[], false);
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

    /// The repeat of a streamed log's errors keeps each distinct error once,
    /// the first line of its message, and no warning, and counts what it
    /// leaves out.
    #[test]
    fn the_repeated_errors_are_the_first_distinct_ones() {
        let diagnostics = vec![
            diag("warning", Some("A.swift:1:1"), "unused variable 'x'"),
            diag("error", Some("B.swift:9:3"), "cannot find 'foo' in scope"),
            diag("error", Some("B.swift:9:3"), "cannot find 'foo' in scope"),
            diag(
                "error",
                None,
                "xcodebuild: Unable to find a device matching the provided destination:\n\
                 \t\t{ platform:iOS }",
            ),
            diag("error", Some("C.swift:2:1"), "expected '}'"),
            diag("error", Some("D.swift:4:1"), "expected ')'"),
        ];
        assert_eq!(
            repeated_errors(&diagnostics),
            ":\n  \
             error: B.swift:9:3: cannot find 'foo' in scope\n  \
             error: xcodebuild: Unable to find a device matching the provided destination\n  \
             error: C.swift:2:1: expected '}'\n  \
             and 1 more error(s)"
        );
        assert_eq!(
            repeated_errors(&diagnostics[..2]),
            ":\n  error: B.swift:9:3: cannot find 'foo' in scope"
        );
    }

    /// A streamed compile error is left to the stream while stderr shares its
    /// file, and repeated, with 'build diagnostics' as the tip, once it does
    /// not. Neither changes what the machine modes read.
    #[test]
    fn a_build_failure_repeats_streamed_errors_only_on_a_stderr_apart() {
        let diagnostics = || {
            vec![diag(
                "error",
                Some("B.swift:9:3"),
                "cannot find 'foo' in scope",
            )]
        };
        let args = vec!["build".to_string()];

        let beside = build_failure(&args, diagnostics(), None, true, false, "");
        assert!(beside.is_shown());
        assert_eq!(
            beside.to_string(),
            "xcodebuild exited with a non-zero status"
        );
        assert_eq!(beside.tip_text(), None);

        let apart = build_failure(&args, diagnostics(), None, true, true, "");
        assert!(!apart.is_shown());
        assert_eq!(
            apart.to_string(),
            "xcodebuild exited with a non-zero status:\n  \
             error: B.swift:9:3: cannot find 'foo' in scope"
        );
        assert_eq!(
            apart.tip_text(),
            Some("run 'sweetpad build diagnostics' to see every error and warning")
        );
        assert_eq!(apart.json()["diagnostics"], beside.json()["diagnostics"]);

        // A captured run was never streamed, so it keeps its own detail.
        let captured = build_failure(&args, diagnostics(), None, false, true, ": the detail");
        assert_eq!(
            captured.to_string(),
            "xcodebuild exited with a non-zero status: the detail"
        );
        assert_eq!(captured.tip_text(), None);
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

    fn build_plan<'a>(c: &'a Container, bundle: &Path, passthrough: &'a [String]) -> BuildPlan<'a> {
        BuildPlan {
            action: BuildAction::Build,
            container: c,
            scheme: "App",
            configuration: "Debug",
            destination: Some("platform=macOS"),
            passthrough,
            sdk: None,
            clean: false,
            hot: false,
            hot_entitlements: None,
            result_bundle: Some(bundle.to_path_buf()),
        }
    }

    #[test]
    fn a_typed_result_bundle_replaces_the_builds_own() {
        // xcodebuild refuses a second '-resultBundlePath', and nothing reads
        // the build's bundle back, so the typed one is passed alone. The
        // activity log the editor's index reads is written either way.
        let c = project();
        let slot = PathBuf::from("/state/App-build.xcresult");
        let tail = vec!["-resultBundlePath".to_string(), "mine.xcresult".to_string()];
        let args = build_plan(&c, &slot, &tail).args();
        assert_eq!(
            args.iter().filter(|a| *a == "-resultBundlePath").count(),
            1,
            "{args:?}"
        );
        assert!(!args.contains(&slot.display().to_string()), "{args:?}");
        assert!(args.ends_with(&tail), "{args:?}");
    }

    #[test]
    fn a_typed_result_bundle_is_replaced_only_after_this_process_wrote_it() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-typed-bundle");
        let c = Container::Project(dir.join("App.xcodeproj"));
        let slot = dir.join("slot.xcresult");
        // A bundle that was there first is the caller's: it stays, and
        // xcodebuild says it exists.
        let theirs = dir.join("theirs.xcresult");
        std::fs::create_dir_all(theirs.join("Data")).unwrap();
        let tail = vec![
            "-resultBundlePath".to_string(),
            "theirs.xcresult".to_string(),
        ];
        build_plan(&c, &slot, &tail).prepare_result_bundle();
        assert!(theirs.join("Data").exists());
        // One this process's first build wrote is replaced on the next build,
        // as a 'build --watch' round or a run-session rebuild needs.
        let tail = vec!["-resultBundlePath".to_string(), "ours.xcresult".to_string()];
        let plan = build_plan(&c, &slot, &tail);
        plan.prepare_result_bundle();
        std::fs::create_dir_all(dir.join("ours.xcresult/Data")).unwrap();
        plan.prepare_result_bundle();
        assert!(!dir.join("ours.xcresult").exists());
        // The slot sweetpad would have used is left alone throughout.
        std::fs::create_dir_all(&slot).unwrap();
        plan.prepare_result_bundle();
        assert!(slot.exists());
    }

    #[test]
    fn a_tail_naming_what_sweetpad_passes_is_a_usage_error_naming_its_flag() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
        for (action, tail, instead) in [
            (Action::Build, &["-scheme", "App"][..], "--scheme"),
            (
                Action::BuildForTesting,
                &["-configuration", "Release"],
                "--configuration",
            ),
            (Action::Test, &["-sdk", "macosx"], "--sdk"),
            (
                Action::Archive,
                &["-workspace", "A.xcworkspace"],
                "--workspace",
            ),
            (
                Action::Build,
                &["FOO=1", "-project", "A.xcodeproj"],
                "--project",
            ),
            (
                Action::Test,
                &["-resultBundlePath", "r.xcresult"],
                "--result-bundle",
            ),
            (
                Action::Archive,
                &["-archivePath", "A.xcarchive"],
                "--output-file",
            ),
            (Action::Archive, &["-exportPath", "out"], "--output-file"),
            (
                Action::Archive,
                &["-exportOptionsPlist", "E.plist"],
                "--export-options",
            ),
        ] {
            let err = refuse_owned_flags(action, &s(tail)).expect_err(&format!("{tail:?}"));
            assert_eq!(err.error_kind(), ErrorKind::Usage, "{tail:?}");
            assert!(
                err.to_string()
                    .contains(&format!("pass '{instead}' instead")),
                "{tail:?}: {err}"
            );
        }
        for (action, tail) in [
            // A build writes a typed bundle in place of its own.
            (Action::Build, &["-resultBundlePath", "r.xcresult"][..]),
            (
                Action::BuildForTesting,
                &["-resultBundlePath", "r.xcresult"],
            ),
            // xcodebuild takes several destinations.
            (Action::Test, &["-destination", "platform=macOS"]),
            // Only the exact token is the flag.
            (Action::Build, &["-scheme=App"]),
            // A flag's value is not a flag.
            (Action::Build, &["-xcconfig", "-scheme"]),
            // Archive's own paths are its alone.
            (Action::Build, &["-archivePath", "A.xcarchive"]),
            (Action::Test, &["-exportPath", "out"]),
        ] {
            assert!(refuse_owned_flags(action, &s(tail)).is_ok(), "{tail:?}");
        }
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

    /// `xcodebuild` runs in the container's directory, so a relative
    /// container is named from there. Typed as it is, `link/App.xcodeproj`
    /// would name `link/link/App.xcodeproj`.
    #[test]
    fn a_relative_container_is_named_from_the_directory_xcodebuild_runs_in() {
        for (container, args, cwd) in [
            (
                Container::Project(PathBuf::from("link/App.xcodeproj")),
                ["-project", "App.xcodeproj"],
                Some("link"),
            ),
            (
                Container::Workspace(PathBuf::from("../ios/App.xcworkspace")),
                ["-workspace", "App.xcworkspace"],
                Some("../ios"),
            ),
            (
                Container::Project(PathBuf::from("App.xcodeproj")),
                ["-project", "App.xcodeproj"],
                None,
            ),
            (
                Container::Project(PathBuf::from("/work/ios/App.xcodeproj")),
                ["-project", "/work/ios/App.xcodeproj"],
                Some("/work/ios"),
            ),
        ] {
            assert_eq!(container_args(&container), args, "{container:?}");
            assert_eq!(working_dir(&container), cwd.map(PathBuf::from));
        }
        let package = Container::SwiftPackage(PathBuf::from("pkg/Package.swift"));
        assert!(container_args(&package).is_empty());
    }

    #[test]
    fn a_relative_derived_data_path_resolves_where_xcodebuild_runs() {
        let args = |dir: &str| vec!["-derivedDataPath".to_string(), dir.to_string()];
        let dd = |args: &[String], container: &Container| {
            command_line_settings(args, container).derived_data_path
        };
        let nested = Container::Project(PathBuf::from("/work/ios/App.xcodeproj"));
        // xcodebuild runs from the project's directory, whatever the caller's.
        assert_eq!(
            dd(&args("build/dd"), &nested),
            Some(PathBuf::from("/work/ios/build/dd"))
        );
        assert_eq!(
            dd(&args("/tmp/dd"), &nested),
            Some(PathBuf::from("/tmp/dd"))
        );
        // A project named relative to the cwd runs xcodebuild in the cwd.
        let here = Container::Project(PathBuf::from("App.xcodeproj"));
        assert_eq!(
            dd(&args("dd"), &here),
            Some(standardize(Path::new(".")).join("dd"))
        );
        assert_eq!(dd(&[], &nested), None);
    }

    /// `xcodebuild` reads a relative path against the physical directory it
    /// runs in. Measured on Xcode 27 from a `link` symlinked to `real/app`,
    /// with `-project` relative and absolute through `link`:
    /// `-derivedDataPath dd` reported `BUILD_DIR = …/real/app/dd/Build/Products`,
    /// and `../dd` reported `…/real/dd/Build/Products`.
    #[test]
    fn a_relative_path_joins_the_real_directory_of_a_symlinked_project() {
        let root = sweetpad_core::scratch::ScratchDir::new("sweetpad-cli-dd-link").unwrap();
        std::fs::create_dir_all(root.join("real/app")).unwrap();
        std::os::unix::fs::symlink(root.join("real/app"), root.join("link")).unwrap();
        let container = Container::Project(root.join("link/App.xcodeproj"));
        let read = |dir: &str| {
            let args = vec!["-derivedDataPath".to_string(), dir.to_string()];
            command_line_settings(&args, &container).derived_data_path
        };
        let physical = standardize(&root.join("real"));
        assert_eq!(read("dd"), Some(physical.join("app/dd")));
        assert_eq!(read("../dd"), Some(physical.join("dd")));
        assert_eq!(
            read(&root.join("link/dd").display().to_string()),
            Some(root.join("link/dd"))
        );
    }

    /// Every build-settings caller reads the build's `-derivedDataPath`,
    /// `-xcconfig` and `KEY=VALUE` arguments the same way: paths from the
    /// directory xcodebuild runs in, settings in order.
    #[test]
    fn the_command_line_settings_are_read_the_way_xcodebuild_reads_them() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
        let nested = Container::Project(PathBuf::from("/work/ios/App.xcodeproj"));
        let read = command_line_settings(
            &s(&[
                "-xcconfig",
                "Config/Over.xcconfig",
                "-derivedDataPath",
                "dd",
                "-destination",
                "OS=17.0,platform=iOS Simulator",
                "PRODUCT_BUNDLE_IDENTIFIER=com.example.x",
                "-skipMacroValidation",
                "SWIFT_VERSION=6.0",
            ]),
            &nested,
        );
        assert_eq!(
            read,
            CommandLineSettings {
                derived_data_path: Some(PathBuf::from("/work/ios/dd")),
                xcconfig: Some(PathBuf::from("/work/ios/Config/Over.xcconfig")),
                overrides: vec![
                    (
                        "PRODUCT_BUNDLE_IDENTIFIER".to_string(),
                        "com.example.x".to_string()
                    ),
                    ("SWIFT_VERSION".to_string(), "6.0".to_string()),
                ],
            }
        );
        // An absolute path stays as typed, and none given is none.
        let absolute = command_line_settings(&s(&["-xcconfig", "/x/Over.xcconfig"]), &nested);
        assert_eq!(absolute.xcconfig, Some(PathBuf::from("/x/Over.xcconfig")));
        assert_eq!(
            command_line_settings(&[], &nested),
            CommandLineSettings::default()
        );
    }

    /// The CLI and the BSP server read a flag's value with one helper, so they
    /// agree on which copy counts, and a tail that ends waiting for a value is
    /// refused before either reads it.
    #[test]
    fn a_flag_left_without_its_value_is_refused_and_never_read() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
        let project = Container::Project(PathBuf::from("/work/ios/App.xcodeproj"));

        let tail = s(&["-xcconfig", "a.xcconfig", "FOO=1", "-xcconfig"]);
        let err = refuse_dangling_flag(&tail).expect_err("a dangling flag");
        assert_eq!(err.error_kind(), ErrorKind::Usage);
        assert_eq!(err.to_string(), "'-xcconfig' after '--' needs a value");
        assert_eq!(
            command_line_settings(&tail, &project).xcconfig,
            sweetpad_core::xcodebuild_args::last_value(&tail, "-xcconfig")
                .map(|p| PathBuf::from("/work/ios").join(p)),
        );
        assert_eq!(
            command_line_settings(&tail, &project).xcconfig,
            Some(PathBuf::from("/work/ios/a.xcconfig"))
        );

        // A value spelled like a flag is a value, as xcodebuild reads it.
        let tail = s(&["-derivedDataPath", "-xcconfig"]);
        assert!(refuse_dangling_flag(&tail).is_ok());
        let read = command_line_settings(&tail, &project);
        assert_eq!(
            read.derived_data_path,
            Some(PathBuf::from("/work/ios/-xcconfig"))
        );
        assert_eq!(read.xcconfig, None);

        for tail in [&[][..], &["-quiet"], &["-xcconfig", "a.xcconfig", "-quiet"]] {
            assert!(refuse_dangling_flag(&s(tail)).is_ok(), "{tail:?}");
        }
    }

    /// The settings stay as typed, a relative build location included: the
    /// resolver reads that against each target's project directory.
    #[test]
    fn a_passthroughs_settings_are_read_as_typed() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
        let project = Container::Project(PathBuf::from("/work/ios/App.xcodeproj"));
        let read = command_line_settings(
            &s(&["SYMROOT=build", "OBJROOT=/tmp/obj", "PRODUCT_NAME=Renamed"]),
            &project,
        );
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert_eq!(
            read.overrides,
            [
                pair("SYMROOT", "build"),
                pair("OBJROOT", "/tmp/obj"),
                pair("PRODUCT_NAME", "Renamed"),
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
        // The options the locator builds, with the catalog it parses out of
        // the active Xcode cached in a directory of the test's own rather than
        // the user's `~/.cache/sweetpad`.
        let cache = crate::cli::testdir::TempDir::new("sweetpad-locator-catalog");
        let mut opts = settings_options(&plan).unwrap();
        opts.catalog_cache = Some(cache.join("catalog.bin"));
        sweetpad_core::app_locator::locate(opts).unwrap().app
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

    /// Xcode 27 builds `SYMROOT=../x/sym2` into `<parent>/x/sym2/Debug`, and
    /// `-showBuildSettings` spells `TARGET_BUILD_DIR` that way too: the
    /// relative value is read against the project's directory and folded.
    /// `SYMROOT=build` is `<project dir>/build`.
    #[test]
    fn a_relocated_product_path_has_no_dot_dot() {
        let container = fixture_app();
        let project_dir = sweetpad_lib::project::standardize(&working_dir(&container).unwrap());
        let beside = located(&container, &["SYMROOT=build".to_string()]);
        assert_eq!(
            beside.path,
            project_dir.join("build/Debug/SweetpadCIMac.app"),
            "{}",
            beside.path.display()
        );
        let app = located(&container, &["SYMROOT=../x/sym2".to_string()]);
        let parent = project_dir.parent().unwrap();
        assert_eq!(
            app.path,
            parent.join("x/sym2/Debug/SweetpadCIMac.app"),
            "{}",
            app.path.display()
        );
        assert_eq!(
            app.executable,
            app.path.join("Contents/MacOS/SweetpadCIMac")
        );
    }

    #[test]
    fn each_action_takes_the_files_args_but_the_flags_it_fails_on() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
        let file = s(&[
            "SYMROOT=build",
            "-enableCodeCoverage",
            "YES",
            "-xcconfig",
            "ci.xcconfig",
            "-testPlan",
            "Smoke",
            "-skipMacroValidation",
            "-resultStreamPath",
            "stream.json",
            "-test-iterations",
            "3",
            "-clonedSourcePackagesDirPath",
            "pkgs",
        ]);
        let (kept, notes) = for_action(Action::Clean, &file, &[]);
        assert_eq!(
            kept,
            [
                "SYMROOT=build",
                "-xcconfig",
                "ci.xcconfig",
                "-skipMacroValidation",
                "-test-iterations",
                "3",
                "-clonedSourcePackagesDirPath",
                "pkgs",
            ]
        );
        assert_eq!(
            notes,
            [
                "leaving out sweetpad.toml's '-enableCodeCoverage YES': 'xcodebuild clean' fails \
                 on it, as a flag only testing takes",
                "leaving out sweetpad.toml's '-testPlan Smoke': 'xcodebuild clean' fails on it, \
                 as a flag only testing takes",
                "leaving out sweetpad.toml's '-resultStreamPath stream.json': 'xcodebuild clean' \
                 fails on it without a '-resultBundlePath' to stream into",
            ]
        );
        // A build passes its own result bundle, so the stream stays; the
        // test-only flags go.
        let (kept, notes) = for_action(Action::Build, &file, &[]);
        assert!(kept.contains(&"-resultStreamPath".to_string()), "{kept:?}");
        assert!(
            !kept.contains(&"-enableCodeCoverage".to_string()),
            "{kept:?}"
        );
        assert!(!kept.contains(&"-testPlan".to_string()), "{kept:?}");
        assert_eq!(notes.len(), 2, "{notes:?}");
        // An archive has a bundle only when one is typed.
        let (kept, _) = for_action(Action::Archive, &file, &[]);
        assert!(!kept.contains(&"-resultStreamPath".to_string()), "{kept:?}");
        let (kept, _) = for_action(Action::Archive, &file, &s(&["-resultBundlePath", "r"]));
        assert!(kept.contains(&"-resultStreamPath".to_string()), "{kept:?}");
        // The testing actions take the whole file.
        for action in [Action::Test, Action::BuildForTesting] {
            assert_eq!(for_action(action, &file, &[]), (file.clone(), Vec::new()));
        }
    }

    /// A flag's value spelled like a flag an action leaves out is the value,
    /// as `xcodebuild` reads it, in the file and in the tail.
    #[test]
    fn an_action_reads_a_flags_value_as_its_value() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
        let file = s(&[
            "-xcconfig",
            "-enableCodeCoverage",
            "FOO=1",
            "-clonedSourcePackagesDirPath",
            "-resultStreamPath",
            "-testPlan",
            "Smoke",
        ]);
        let (kept, notes) = for_action(Action::Clean, &file, &[]);
        assert_eq!(
            kept,
            [
                "-xcconfig",
                "-enableCodeCoverage",
                "FOO=1",
                "-clonedSourcePackagesDirPath",
                "-resultStreamPath",
            ]
        );
        assert_eq!(
            notes,
            [
                "leaving out sweetpad.toml's '-testPlan Smoke': 'xcodebuild clean' fails on it, \
                 as a flag only testing takes"
            ]
        );
        assert!(crate::cli::config::effective_xcodebuild_args(&kept, &[]).is_ok());

        // A '-resultBundlePath' that is another flag's value names no bundle.
        let file = s(&["-resultStreamPath", "stream.json"]);
        let tail = s(&["-derivedDataPath", "-resultBundlePath"]);
        let (kept, _) = for_action(Action::Archive, &file, &tail);
        assert!(kept.is_empty(), "{kept:?}");
        let tail = s(&["-resultBundlePath", "-derivedDataPath"]);
        let (kept, _) = for_action(Action::Archive, &file, &tail);
        assert_eq!(kept, file);
    }

    #[test]
    fn the_flags_an_action_leaves_out_are_ones_the_file_may_carry() {
        // The file's refusals run on what the action keeps, so a flag that
        // was both left out and refused would slip past them on that action.
        for flag in TEST_ONLY_FLAGS.iter().chain(&["-resultStreamPath"]) {
            let file = [(*flag).to_string(), "v".to_string()];
            assert!(
                crate::cli::config::effective_xcodebuild_args(&file, &[]).is_ok(),
                "{flag}"
            );
            // Each takes a value, which leaves with it.
            assert!(xcodebuild_args::takes_value(flag), "{flag}");
            assert!(for_action(Action::Clean, &file, &[]).0.is_empty(), "{flag}");
        }
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
            failed_selectors(&tree()).selectors,
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
        assert_eq!(printed, failed_selectors(&tree()).selectors);
    }

    /// A macOS host app that crashed at launch, on Xcode 27: XCTest records the
    /// test process as a case of its own, which names no test. Handed to
    /// xcodebuild as '-only-testing:SweetpadB6TestMacTests/SweetpadB6TestMac
    /// (16050) encountered an error', it ran no test and reported
    /// `** TEST SUCCEEDED **`.
    #[test]
    fn a_failure_outside_any_test_is_not_rerun_as_one() {
        fn bare(identifier: &str) -> TreeCase<'_> {
            TreeCase {
                target: Some("AppTests"),
                identifier,
                url: None,
                outcome: CaseOutcome::Failed,
                duration: None,
                messages: Vec::new(),
                skip_message: None,
            }
        }
        let tree: serde_json::Value = serde_json::from_str(
            r#"{ "testNodes": [
  { "name": "SweetpadB6TestMac", "nodeType": "Test Plan", "result": "Failed", "children": [
    { "name": "SweetpadB6TestMacTests", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadB6Test/SweetpadB6TestMacTests", "nodeType": "Unit test bundle", "result": "Failed", "children": [
      { "name": "System Failures", "nodeType": "Test Suite", "result": "Failed", "children": [
        { "name": "SweetpadB6TestMac (16050) encountered an error", "nodeIdentifier": "SweetpadB6TestMac (16050) encountered an error", "nodeType": "Test Case", "result": "Failed", "children": [
          { "name": "Early unexpected exit, operation never finished bootstrapping - no restart will be attempted. (Underlying Error: The test runner crashed before establishing connection: SweetpadB6TestMac)", "nodeType": "Failure Message" }
        ] }
      ] },
      { "name": "ParallelSuite", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadB6Test/SweetpadB6TestMacTests/ParallelSuite", "nodeType": "Test Suite", "result": "Failed", "children": [
        { "name": "a()", "nodeIdentifier": "ParallelSuite/a()", "nodeIdentifierURL": "test://com.apple.xcode/SweetpadB6Test/SweetpadB6TestMacTests/ParallelSuite/a()", "nodeType": "Test Case", "result": "Failed" }
      ] }
    ] }
  ] }
] }"#,
        )
        .unwrap();
        assert_eq!(
            failed_selectors(&tree),
            FailedTests {
                selectors: vec!["SweetpadB6TestMacTests/ParallelSuite/a()".to_string()],
                outside_tests: vec!["SweetpadB6TestMac (16050) encountered an error".to_string()],
            }
        );

        // A tree with no URLs at all still names each test without a space.
        assert!(bare("AppTests/testGreeting()").is_a_test());
        assert!(!bare("App (36652) encountered an error").is_a_test());
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
    fn every_test_case_is_listed_with_its_outcome_and_time() {
        // Trimmed from an Xcode 27 run of the fixture: a pass, a skip, a
        // failure with two messages, and a Swift Testing failure.
        let recorded = serde_json::json!({ "testNodes": [
            { "name": "SweetpadCIApp", "nodeType": "Test Plan", "children": [
                { "name": "SweetpadCIAppTests", "nodeType": "Unit test bundle", "children": [
                    { "name": "AppTests", "nodeType": "Test Suite", "children": [
                        { "name": "testArithmetic()", "nodeIdentifier": "AppTests/testArithmetic()",
                          "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/AppTests/testArithmetic",
                          "nodeType": "Test Case", "result": "Passed",
                          "duration": "0,0033s", "durationInSeconds": 0.003_320_097_923_278_808_6 },
                        { "name": "testSkipped()", "nodeIdentifier": "AppTests/testSkipped()",
                          "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/AppTests/testSkipped",
                          "nodeType": "Test Case", "result": "Skipped", "durationInSeconds": 0.006_891,
                          "children": [ { "name": "Test skipped - not on this simulator", "nodeType": "Skip Message" } ] },
                        { "name": "testTwoFailures()", "nodeIdentifier": "AppTests/testTwoFailures()",
                          "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/AppTests/testTwoFailures",
                          "nodeType": "Test Case", "result": "Failed", "durationInSeconds": 0.038_118,
                          "children": [
                            { "name": "XCTAssertEqual failed: (\"hello\") is not equal to (\"world\")", "nodeType": "Failure Message" },
                            { "name": "XCTAssertTrue failed - second failure\nwith a second line", "nodeType": "Failure Message" } ] } ] },
                    { "name": "GreetingSuite", "nodeType": "Test Suite", "children": [
                        { "name": "suiteGreeting()", "nodeIdentifier": "GreetingSuite/suiteGreeting()",
                          "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/suiteGreeting()",
                          "nodeType": "Test Case", "result": "Failed", "durationInSeconds": 0.005_311,
                          "children": [ { "name": "Expectation failed: n == 1\nn → 2", "nodeType": "Failure Message" } ] },
                        { "name": "expected()", "nodeIdentifier": "GreetingSuite/expected()",
                          "nodeIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/GreetingSuite/expected()",
                          "nodeType": "Test Case", "result": "Expected Failure" } ] } ] } ] }
        ] });
        let cases = test_cases(&recorded);
        let listed: Vec<(&str, CaseOutcome, Option<f64>)> = cases
            .iter()
            .map(|c| (c.identifier.as_str(), c.outcome, c.duration))
            .collect();
        assert_eq!(
            listed,
            [
                (
                    "SweetpadCIAppTests/AppTests/testArithmetic",
                    CaseOutcome::Passed,
                    Some(0.003_320_097_923_278_808_6)
                ),
                (
                    "SweetpadCIAppTests/AppTests/testSkipped",
                    CaseOutcome::Skipped,
                    Some(0.006_891)
                ),
                (
                    "SweetpadCIAppTests/AppTests/testTwoFailures",
                    CaseOutcome::Failed,
                    Some(0.038_118)
                ),
                (
                    "SweetpadCIAppTests/GreetingSuite/suiteGreeting()",
                    CaseOutcome::Failed,
                    Some(0.005_311)
                ),
                // An expected failure is a pass, and a case may carry no time.
                (
                    "SweetpadCIAppTests/GreetingSuite/expected()",
                    CaseOutcome::Passed,
                    None
                ),
            ]
        );
        assert_eq!(
            cases[1].skip_message.as_deref(),
            Some("Test skipped - not on this simulator")
        );
        assert_eq!(
            cases[2].messages,
            [
                "XCTAssertEqual failed: (\"hello\") is not equal to (\"world\")",
                "XCTAssertTrue failed - second failure\nwith a second line",
            ]
        );
        assert!(cases[0].messages.is_empty() && cases[0].skip_message.is_none());
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
    fn a_failed_tests_attachments_say_so_whatever_the_manifest_marks() {
        // On an iOS simulator, Xcode 27 marks no attachment as recorded
        // against a failure, not even the crash log and screen recording of a
        // UI test whose app crashed. The tree is what says the test failed.
        // Trimmed from that run.
        let manifest = r#"[
          { "testIdentifier": "AppUITests/testLaunchShowsNothing()",
            "testIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppUITests/AppUITests/testLaunchShowsNothing",
            "attachments": [
              { "exportedFileName": "1721F03E.ips", "isAssociatedWithFailure": false,
                "suggestedHumanReadableName": "Crash Log 2026-09-26 at 08.09.04 PM.ips", "timestamp": 1790446144.877 },
              { "exportedFileName": "6A3B5911.mp4", "isAssociatedWithFailure": false,
                "suggestedHumanReadableName": "Screen Recording 2026-09-26 at 08.08.49 PM.mp4", "timestamp": 1790446129.731 } ] },
          { "testIdentifier": "GreetingSuite/suiteGreeting()",
            "attachments": [ { "exportedFileName": "8B93D9DB.txt", "timestamp": 1790441857.306 } ] },
          { "testIdentifier": "AppTests/testArithmetic()",
            "testIdentifierURL": "test://com.apple.xcode/SweetpadCIApp/SweetpadCIAppTests/AppTests/testArithmetic",
            "attachments": [
              { "exportedFileName": "FC88EDEC.txt", "isAssociatedWithFailure": false, "timestamp": 1790441857.045 },
              { "exportedFileName": "0A8B57AD.txt", "isAssociatedWithFailure": true, "timestamp": 1790441857.046 } ] },
          { "testIdentifier": "Gone/testElsewhere()",
            "attachments": [ { "exportedFileName": "D00DFEED.txt", "timestamp": 1790441879.133 } ] }
        ]"#;
        let targets = TestTargets::from_tree(&tree());
        assert_eq!(targets.failed.len(), 7);
        let exported =
            parse_attachment_manifest(manifest, Path::new("/staging"), &targets).unwrap();
        let marks: Vec<(&str, bool, bool)> = exported
            .iter()
            .map(|a| (a.identifier.as_str(), a.failed_test, a.failure))
            .collect();
        assert_eq!(
            marks,
            [
                (
                    "SweetpadCIAppUITests/AppUITests/testLaunchShowsNothing",
                    true,
                    false
                ),
                (
                    "SweetpadCIAppUITests/AppUITests/testLaunchShowsNothing",
                    true,
                    false
                ),
                // Found without a URL, where the name drops the `()` the
                // tree's own spelling keeps.
                (
                    "SweetpadCIAppTests/GreetingSuite/suiteGreeting",
                    true,
                    false
                ),
                // A passing test, one of whose files is marked anyway.
                ("SweetpadCIAppTests/AppTests/testArithmetic", false, false),
                ("SweetpadCIAppTests/AppTests/testArithmetic", false, true),
                // A test the tree doesn't list counts as passed.
                ("Gone/testElsewhere", false, false),
            ]
        );
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
