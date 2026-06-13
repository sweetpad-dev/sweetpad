//! Thin wrapper over `xcodebuild` for the build/run commands: assembling the
//! argument vector (mirroring the VS Code extension's proven invocation) and
//! reading back the build settings needed to locate and launch the built app.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::cli::resolve::Container;
use crate::cli::{process, CliError};

/// Everything needed to invoke `xcodebuild build` for a resolved target.
pub struct BuildPlan<'a> {
    pub container: &'a Container,
    pub scheme: &'a str,
    pub configuration: &'a str,
    /// Raw `-destination` specifier, e.g. `platform=iOS Simulator,id=<udid>`.
    pub destination: Option<&'a str>,
    pub clean: bool,
}

impl BuildPlan<'_> {
    /// The `xcodebuild` argument vector: `[clean] build -scheme … -configuration
    /// … [-destination …] [-workspace|-project …]`.
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
        args.extend(container_args(self.container));
        args
    }

    /// Run the build, streaming xcodebuild output to the terminal.
    pub fn run(&self) -> Result<(), CliError> {
        let parts = self.args();
        let args: Vec<&str> = parts.iter().map(String::as_str).collect();
        process::stream("xcodebuild", &args, working_dir(self.container).as_deref())
    }
}

/// `-workspace <path>` / `-project <path>`; nothing for a Swift package (it's
/// driven from the package directory).
fn container_args(container: &Container) -> Vec<String> {
    match container {
        Container::Workspace(p) => vec!["-workspace".into(), p.display().to_string()],
        Container::Project(p) => vec!["-project".into(), p.display().to_string()],
        Container::SwiftPackage(_) => Vec::new(),
    }
}

/// Directory to run xcodebuild from: the container's parent (or the package
/// directory for SPM).
fn working_dir(container: &Container) -> Option<PathBuf> {
    container.path().parent().map(Path::to_path_buf)
}

/// Everything needed to invoke `xcodebuild test` for a resolved target.
pub struct TestPlan<'a> {
    pub container: &'a Container,
    pub scheme: &'a str,
    pub configuration: &'a str,
    pub destination: Option<&'a str>,
    /// `-only-testing:` selectors (Target/Class/method); empty runs everything.
    pub only_testing: &'a [String],
    /// `-skip-testing:` selectors.
    pub skip_testing: &'a [String],
    /// Where xcodebuild writes the `.xcresult` bundle (parsed for the summary).
    pub result_bundle: &'a Path,
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
        args.extend(container_args(self.container));
        for t in self.only_testing {
            args.push(format!("-only-testing:{t}"));
        }
        for t in self.skip_testing {
            args.push(format!("-skip-testing:{t}"));
        }
        args
    }

    /// Run the tests. `quiet` discards xcodebuild's stdout (for `--json`, where
    /// only the parsed summary is emitted). Returns whether the run passed; a
    /// test failure is a `false`, not an error.
    pub fn run(&self, quiet: bool) -> Result<bool, CliError> {
        let parts = self.args();
        let args: Vec<&str> = parts.iter().map(String::as_str).collect();
        process::run("xcodebuild", &args, working_dir(self.container).as_deref(), quiet)
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
    )?;
    let json = out
        .find('{')
        .map(|i| &out[i..])
        .ok_or_else(|| CliError::new("xcresulttool produced no JSON summary"))?;
    serde_json::from_str(json).map_err(|e| CliError::new(format!("parsing test summary: {e}")))
}

/// One target's settings from `xcodebuild -showBuildSettings -json`.
#[derive(Debug, Deserialize)]
pub struct TargetBuildSettings {
    pub target: String,
    #[serde(rename = "buildSettings")]
    pub settings: BTreeMap<String, String>,
}

/// Read build settings via `xcodebuild -showBuildSettings -json`. Used at run
/// time to locate the built `.app` and its bundle id (the in-process resolver
/// needs an Xcode catalog; for the launch path we ask xcodebuild directly,
/// matching the values the actual build produced).
pub fn show_settings(
    container: &Container,
    scheme: &str,
    configuration: &str,
    destination: Option<&str>,
) -> Result<Vec<TargetBuildSettings>, CliError> {
    let mut parts: Vec<String> = vec![
        "-showBuildSettings".into(),
        "-json".into(),
        "-scheme".into(),
        scheme.into(),
        "-configuration".into(),
        configuration.into(),
    ];
    if let Some(dest) = destination {
        parts.push("-destination".into());
        parts.push(dest.into());
    }
    parts.extend(container_args(container));

    let args: Vec<&str> = parts.iter().map(String::as_str).collect();
    let stdout = process::capture("xcodebuild", &args, working_dir(container).as_deref())?;

    // xcodebuild can print warnings before the JSON; slice from the first `[`.
    let json = stdout
        .find('[')
        .map(|i| &stdout[i..])
        .ok_or_else(|| CliError::new("xcodebuild -showBuildSettings produced no JSON"))?;
    serde_json::from_str(json)
        .map_err(|e| CliError::new(format!("parsing build settings: {e}")))
}

/// The launchable app produced by a build: the `.app` path and its bundle id.
#[derive(Debug)]
pub struct AppBundle {
    pub path: PathBuf,
    pub bundle_id: String,
}

/// Pick the launchable app from resolved settings: the first target that builds
/// a `.app` wrapper and declares a bundle id.
pub fn app_bundle(settings: &[TargetBuildSettings]) -> Result<AppBundle, CliError> {
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
        if Path::new(wrapper)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("app"))
        {
            return Ok(AppBundle {
                path: Path::new(build_dir).join(wrapper),
                bundle_id: bundle_id.clone(),
            });
        }
    }
    Err(CliError::new(
        "could not find a launchable .app in the resolved build settings",
    ))
}
