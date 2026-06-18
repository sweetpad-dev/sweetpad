//! Thin wrapper over `xcodebuild` for the build/run commands: assembling the
//! argument vector (mirroring the VS Code extension's proven invocation) and
//! reading back the build settings needed to locate and launch the built app.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::cli::output::Output;
use crate::cli::resolve::Container;
use crate::cli::{CliError, ErrorContext, buildlog, process};

/// Everything needed to invoke `xcodebuild build` for a resolved target.
pub struct BuildPlan<'a> {
    pub container: &'a Container,
    pub scheme: &'a str,
    pub configuration: &'a str,
    /// Raw `-destination` specifier, e.g. `platform=iOS Simulator,id=<udid>`.
    pub destination: Option<&'a str>,
    pub clean: bool,
    /// Hot-reload build: add `-Xlinker -interposable` (so dyld can swap symbols)
    /// and `EMIT_FRONTEND_COMMAND_LINES=YES` (so the build-log recompiler can
    /// recover per-file commands). Only set for simulator builds under `--hot`.
    pub hot: bool,
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
        if self.hot {
            // Build settings (KEY=VALUE) after the action; `$(inherited)` keeps
            // any project OTHER_LDFLAGS. Mirrors the VS Code extension + the
            // validated spike fixture.
            args.push("OTHER_LDFLAGS=$(inherited) -Xlinker -interposable".into());
            args.push("EMIT_FRONTEND_COMMAND_LINES=YES".into());
        }
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
    /// [`buildlog`]; `-v` passes it through raw; `--json` stays quiet.
    pub fn run(&self, out: &Output) -> Result<(), CliError> {
        let parts = self.args();
        let args: Vec<&str> = parts.iter().map(String::as_str).collect();
        let cwd = working_dir(self.container);
        let ok = if out.is_json() {
            process::run("xcodebuild", &args, cwd.as_deref(), true)?
        } else if out.is_verbose() {
            process::run("xcodebuild", &args, cwd.as_deref(), false)?
        } else {
            buildlog::run("xcodebuild", &args, cwd.as_deref(), out)?
        };
        if ok {
            Ok(())
        } else {
            Err(CliError::new("xcodebuild exited with a non-zero status")
                .context("building the project"))
        }
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
/// directory for SPM). A relative container like `App.xcodeproj` has an empty
/// parent — that means "the current directory", so return `None` rather than
/// trying to `chdir("")` (which fails the spawn and looks like a missing tool).
fn working_dir(container: &Container) -> Option<PathBuf> {
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

    /// Run the tests. `--json` stays quiet (only the parsed summary is emitted),
    /// `-v` is raw, otherwise xcodebuild output is beautified. Returns whether
    /// the run passed; a test failure is `false`, not an error.
    pub fn run(&self, out: &Output) -> Result<bool, CliError> {
        let parts = self.args();
        let args: Vec<&str> = parts.iter().map(String::as_str).collect();
        let cwd = working_dir(self.container);
        let result = if out.is_json() {
            process::run("xcodebuild", &args, cwd.as_deref(), true)
        } else if out.is_verbose() {
            process::run("xcodebuild", &args, cwd.as_deref(), false)
        } else {
            buildlog::run("xcodebuild", &args, cwd.as_deref(), out)
        };
        result.context("running the tests")
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
            let build_dir = Path::new(build_dir);
            let executable = t
                .settings
                .get("EXECUTABLE_PATH")
                .map_or_else(|| build_dir.join(wrapper), |rel| build_dir.join(rel));
            return Ok(AppBundle {
                path: build_dir.join(wrapper),
                bundle_id: bundle_id.clone(),
                executable,
            });
        }
    }
    Err(CliError::new(
        "could not find a launchable .app in the resolved build settings",
    ))
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
        let app = app_bundle(&settings).unwrap();
        assert_eq!(app.path, PathBuf::from("/d/App.app"));
        assert_eq!(app.bundle_id, "com.x.app");
    }

    #[test]
    fn app_bundle_resolves_macos_executable() {
        let stdout = r#"[{"target":"App","buildSettings":{
            "TARGET_BUILD_DIR":"/d","WRAPPER_NAME":"App.app",
            "EXECUTABLE_PATH":"App.app/Contents/MacOS/App","PRODUCT_BUNDLE_IDENTIFIER":"com.x.app"}}]"#;
        let settings = parse_settings(stdout);
        let app = app_bundle(&settings).unwrap();
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
        assert!(app_bundle(&settings).is_err());
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
