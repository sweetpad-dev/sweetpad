//! Building with Swift Build (`swbuild`), the open-source engine under Xcode's
//! build system, in place of `xcodebuild`.
//!
//! Swift Build takes no project files. It reads the project interchange
//! format (PIF) Xcode translates a workspace into, so a build writes the
//! workspace's PIF ([`sweetpad_lib::pif::pif`]), with the Swift packages it
//! uses as SwiftPM emits them, and its build parameters beside the build's
//! intermediates, then runs `swbuild build` on them.
//! swbuild reports the build as a stream of JSON messages, which
//! [`MessageParser`] turns into the [`buildlog`] events every output mode
//! already renders. Products land where `xcodebuild` would put them, so the
//! product locator finds the app.
//!
//! Off macOS there is no Xcode to take the SDKs from. They come from a Darwin
//! Swift SDK bundle (`xtool sdk install` makes one from an Xcode.app), with
//! its linker, and a few settings stand in for tools that only run on macOS.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde_json::{Value, json};
use sweetpad_lib::derived_data::{self, ContainerKey, Locations};
use sweetpad_lib::scheme::BuildFor;

use crate::cli::buildlog::{self, BuildProgress, DiagKind, Event, Parsed, ResultKind};
use crate::cli::output::Output;
use crate::cli::progress::Spinner;
use crate::cli::resolve::Container;
use crate::cli::xcodebuild::{self, BuildAction, BuildPlan, CommandPreview};
use crate::cli::{CliError, ErrorKind, process};

/// The engine a build runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Engine {
    Xcodebuild,
    #[value(name = "swift-build")]
    SwiftBuild,
}

impl Engine {
    /// `xcodebuild` on macOS, and Swift Build where there is no Xcode.
    #[must_use]
    pub fn for_host() -> Self {
        if cfg!(target_os = "macos") {
            Self::Xcodebuild
        } else {
            Self::SwiftBuild
        }
    }
}

/// The `swbuild` command a build of `plan` runs, for `--show-command`. It
/// names the PIF and parameters files the build writes, without writing them.
pub fn command(plan: &BuildPlan<'_>) -> Result<CommandPreview, CliError> {
    let invocation = Invocation::of(plan)?;
    Ok(CommandPreview {
        program: "swbuild",
        args: invocation.args,
        cwd: invocation.cwd,
    })
}

/// Build `plan` with Swift Build, rendering its progress the way an
/// `xcodebuild` build renders in every output mode, and recording the
/// diagnostics `build diagnostics` reads back.
pub fn run(plan: &BuildPlan<'_>, out: &Output) -> Result<Option<buildlog::BuildStats>, CliError> {
    let invocation = Invocation::of(plan)?;
    if plan.clean {
        let _ = std::fs::remove_dir_all(&invocation.locations.products);
        let _ = std::fs::remove_dir_all(&invocation.locations.intermediates);
    }
    invocation.write_inputs(out)?;
    let start = Instant::now();
    let program = invocation.program.to_string_lossy().into_owned();
    let args: Vec<&str> = invocation.args.iter().map(String::as_str).collect();
    let mut parser = MessageParser::default();
    let mut diagnostics = Vec::new();
    let mut transcript = String::new();
    let mut progress =
        (!out.is_json() && !out.is_ndjson()).then(|| BuildProgress::start(out, "Building"));
    let mut take = |parsed: &Parsed| {
        if matches!(parsed.event, Event::Diagnostic { .. })
            && let Some(json) = buildlog::event_json(&parsed.event)
        {
            diagnostics.push(json);
        }
        if out.is_ndjson() {
            if let Some(json) = buildlog::event_json(&parsed.event) {
                out.ndjson_event(&json);
            }
        } else if let Some(progress) = &mut progress
            && let Some(rendered) = progress.parsed(parsed)
        {
            out.line(&rendered);
        }
    };
    let ok = process::stream_lines_with_env(
        &program,
        &args,
        invocation.cwd.as_deref(),
        &invocation.env,
        |line| {
            transcript.push_str(line);
            transcript.push('\n');
            parser.push(line).iter().for_each(&mut take);
        },
    )?;
    if !ok
        && let Some(progress) = &mut progress
        && let Some(banner) = progress.close_failed(ResultKind::BuildFailed)
    {
        out.line(&banner);
    }
    xcodebuild::record_build(plan.container, Some(plan.scheme), ok, &diagnostics);
    if ok {
        return Ok(Some(buildlog::BuildStats::tally(
            &diagnostics,
            start.elapsed(),
        )));
    }
    let detail = if out.is_json() {
        let log = xcodebuild::project_artifact(plan.container, "-build.log");
        xcodebuild::record_failure_transcript(&log, &transcript)
            .map_or_else(String::new, |p| format!("; full log: {}", p.display()))
    } else {
        String::new()
    };
    Err(xcodebuild::tool_failure(
        "swbuild",
        &invocation.args,
        diagnostics,
        None,
        progress.is_some(),
        !Output::streams_share_a_file(),
        &detail,
    )
    .context("building the project"))
}

/// Everything a `swbuild build` needs: the command, its environment, and the
/// files it reads.
struct Invocation {
    program: PathBuf,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, Option<String>)>,
    container: PathBuf,
    pif: PathBuf,
    /// Where the root manifest for the workspace's Swift packages goes.
    packages: PathBuf,
    parameters_file: PathBuf,
    parameters: Value,
    locations: Locations,
    /// The folder of links that run this binary as the Xcode tools it stands
    /// in for, on a host without Xcode's.
    tools: Option<PathBuf>,
    /// The Darwin SDK's linker, which the linker stand-in runs.
    linker: Option<PathBuf>,
    /// Where the toolset a host without Xcode needs goes (see [`toolset`]).
    toolset: Option<PathBuf>,
}

/// Settings Xcode's own Swift Build resolves differently from the open-source
/// one, with no xcspec to say so. `SKIP_COPYING_TEST_FRAMEWORKS`: Xcode 27
/// leaves XCTest and Swift Testing out of a test host app, since the test
/// runner brings them in.
const XCODE_DEFAULTS: &[(&str, &str)] = &[("SKIP_COPYING_TEST_FRAMEWORKS", "YES")];

/// What a build needs where there is no Xcode on top of the Darwin SDK's
/// toolset, as a toolset, which Swift Build applies after that one and the
/// project's own flags:
/// - the compiler flag for cross-import overlays, like the one `StoreView`
///   is in (`_StoreKit_SwiftUI`), which Xcode's compiler loads by default
///   and the open-source one only when asked;
/// - the stand-in in `tools` for the SDK's linker (see
///   [`super::darwin_tools`]).
fn toolset(tools: &Path) -> String {
    let toolset = serde_json::json!({
        "schemaVersion": "1.0",
        "swiftCompiler": {
            "extraCLIOptions": ["-Xfrontend", "-enable-cross-import-overlays"]
        },
        "linker": { "path": tools.join("ld64.lld") },
    });
    format!("{toolset:#}\n")
}

impl Invocation {
    fn of(plan: &BuildPlan<'_>) -> Result<Self, CliError> {
        let container = match plan.container {
            Container::Workspace(path) | Container::Project(path) => path.clone(),
            Container::SwiftPackage(_) => {
                return Err(CliError::new(
                    "Swift Build builds Xcode projects here; a Swift package builds with 'swift build'",
                ));
            }
        };
        if plan.hot {
            return Err(CliError::new(
                "hot reload needs an xcodebuild build; pass '--engine xcodebuild'",
            )
            .kind(ErrorKind::Usage));
        }
        refuse_xcodebuild_flags(plan.passthrough)?;
        let settings = xcodebuild::command_line_settings(plan.passthrough, plan.container);
        let destination = plan
            .destination
            .and_then(sweetpad_lib::destination::parse_destination_arg)
            .ok_or_else(|| {
                CliError::new(format!(
                    "Swift Build needs a destination with a platform, like \
                     'generic/platform=iOS'; got '{}'",
                    plan.destination.unwrap_or_default()
                ))
                .kind(ErrorKind::TargetResolution)
            })?;
        let sdk = plan
            .sdk
            .map_or(destination.platform.clone(), str::to_string);
        let home = std::env::var("HOME").unwrap_or_default();
        let locations = derived_data::resolve(
            &ContainerKey::of(&container),
            &home,
            settings.derived_data_path.as_deref(),
            true,
        );
        let inputs = locations.intermediates.join("SweetPad");
        let pif = inputs.join("workspace.pif.json");
        let packages = inputs.join("SweetPadPackages");
        let parameters_file = inputs.join("build-parameters.json");

        let host = DarwinSdk::for_host()?;
        let tools = host.as_ref().map(|_| inputs.join("bin"));
        let toolset = host.as_ref().map(|_| inputs.join("toolset.json"));
        let mut overrides: BTreeMap<String, String> = XCODE_DEFAULTS
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        if let (Some(host), Some(toolset)) = (&host, &toolset) {
            overrides.extend(host.overrides(&sdk, &destination.arch, toolset));
        }
        if let Some(tools) = &tools {
            overrides.insert(
                "ASSETCATALOG_EXEC".to_string(),
                tools.join("actool").display().to_string(),
            );
        }
        overrides.extend(settings.overrides.iter().cloned());
        let parameters = parameters(
            plan.configuration,
            &sdk,
            &destination.arch,
            &locations,
            &overrides,
            settings.xcconfig.as_deref(),
        );

        let mut args = vec![
            "build".to_string(),
            "--configuration".to_string(),
            plan.configuration.to_string(),
            "--buildParametersFile".to_string(),
            parameters_file.display().to_string(),
        ];
        let build_for = match plan.action {
            BuildAction::Build => BuildFor::Running,
            BuildAction::BuildForTesting => BuildFor::Testing,
        };
        for target in scheme_targets(&container, plan.scheme, build_for)? {
            args.push("--target".to_string());
            args.push(target);
        }
        args.push(pif.display().to_string());
        Ok(Self {
            program: swbuild_program()?,
            args,
            cwd: xcodebuild::working_dir(plan.container),
            linker: host.as_ref().map(|h| h.bundle.join("toolset/bin/ld64.lld")),
            env: host
                .zip(tools.as_deref())
                .map(|(h, tools)| h.env(tools))
                .unwrap_or_default(),
            container,
            pif,
            packages,
            parameters_file,
            parameters,
            locations,
            tools,
            toolset,
        })
    }

    /// Write the workspace's PIF, with its Swift packages, and the build
    /// parameters, and link this binary in as the Xcode tools Swift Build
    /// needs where there is no Xcode.
    fn write_inputs(&self, out: &Output) -> Result<(), CliError> {
        let translate = |e: sweetpad_lib::project::Error| {
            CliError::new(format!(
                "couldn't translate {} for Swift Build: {e}",
                self.container.display()
            ))
        };
        let mut pif = sweetpad_lib::pif::pif(&self.container).map_err(translate)?;
        let packages = sweetpad_lib::pif::packages(&self.container).map_err(translate)?;
        if !packages.is_empty() {
            let dumped = self.package_pif(&packages, out)?;
            sweetpad_lib::pif::add_packages(&mut pif, dumped, &self.packages);
        }
        let write = |path: &Path, value: &Value| -> Result<(), CliError> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| {
                    CliError::new(format!("couldn't create {}: {e}", dir.display()))
                })?;
            }
            let text = serde_json::to_string(value).map_err(|e| CliError::new(e.to_string()))?;
            std::fs::write(path, text)
                .map_err(|e| CliError::new(format!("couldn't write {}: {e}", path.display())))
        };
        write(&self.pif, &Value::Array(pif))?;
        write(&self.parameters_file, &self.parameters)?;
        if let Some(tools) = &self.tools {
            link_tools(tools, self.linker.as_deref())?;
        }
        if let (Some(path), Some(tools)) = (&self.toolset, &self.tools) {
            write_changed(path, toolset(tools).as_bytes())?;
        }
        Ok(())
    }

    /// The PIF SwiftPM emits for the workspace's packages. Their root
    /// manifest goes beside the PIF with the workspace's `Package.resolved`,
    /// so the versions it pins are the ones built, and the checkouts go in
    /// `SourcePackages`, where Xcode keeps them.
    fn package_pif(
        &self,
        packages: &[sweetpad_lib::pif::Package],
        out: &Output,
    ) -> Result<Vec<Value>, CliError> {
        write_changed(
            &self.packages.join("Package.swift"),
            sweetpad_lib::pif::manifest(packages).as_bytes(),
        )?;
        if let Ok(resolved) = std::fs::read(resolved_file(&self.container)) {
            write_changed(&self.packages.join("Package.resolved"), &resolved)?;
        }
        let scratch = self.locations.folder.join("SourcePackages");
        // Xcode builds a dynamic library product as a framework in
        // `PackageFrameworks`, which an app embeds; SwiftPM's own builds
        // make a dylib unless asked otherwise.
        let args = [
            "package".as_ref(),
            "--package-path".as_ref(),
            self.packages.as_os_str(),
            "--scratch-path".as_ref(),
            scratch.as_os_str(),
            "--experimental-build-dylibs-as-frameworks".as_ref(),
            "dump-pif".as_ref(),
        ];
        let spinner = Spinner::start_timed(
            "Resolving packages",
            out.is_interactive() && !out.is_quiet(),
            out.use_color_stderr(),
        );
        // A git prompt for credentials would wait unseen behind the spinner.
        let output = Command::new("swift")
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| process::spawn_error("swift", &e));
        drop(spinner);
        let output = output?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let errors = swiftpm_errors(&stderr);
            let reason = if errors.is_empty() {
                format!("'swift package dump-pif' exited with {}", output.status)
            } else {
                errors.join("\n")
            };
            return Err(CliError::new(format!(
                "couldn't resolve the package dependencies: {reason}"
            )));
        }
        serde_json::from_slice(&output.stdout).map_err(|e| {
            CliError::new(format!(
                "couldn't read the PIF SwiftPM printed for the package dependencies: {e}"
            ))
        })
    }
}

/// SwiftPM's errors in what it printed, each with the indented lines under
/// it, like git's own reason for a failed clone.
fn swiftpm_errors(stderr: &str) -> Vec<&str> {
    let mut errors = Vec::new();
    let mut under_error = false;
    for line in stderr.lines() {
        if let Some(message) = line.strip_prefix("error: ") {
            errors.push(message);
            under_error = true;
        } else if line.contains(": error: ") {
            errors.push(line);
            under_error = true;
        } else if under_error && line.starts_with(char::is_whitespace) {
            errors.push(line);
        } else {
            under_error = false;
        }
    }
    errors
}

/// The `Package.resolved` Xcode pins a workspace's packages in.
fn resolved_file(container: &Path) -> PathBuf {
    let workspace = if container.extension().is_some_and(|e| e == "xcworkspace") {
        container.to_path_buf()
    } else {
        container.join("project.xcworkspace")
    };
    workspace.join("xcshareddata/swiftpm/Package.resolved")
}

/// Write `contents` to `path` unless it holds them already, which keeps the
/// file's time, and with it SwiftPM's cache of the manifest it evaluated.
fn write_changed(path: &Path, contents: &[u8]) -> Result<(), CliError> {
    if std::fs::read(path).is_ok_and(|old| old == contents) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| CliError::new(format!("couldn't create {}: {e}", dir.display())))?;
    }
    std::fs::write(path, contents)
        .map_err(|e| CliError::new(format!("couldn't write {}: {e}", path.display())))
}

/// Link this binary into `dir` under the names of the Xcode tools it stands
/// in for: `actool`, the asset catalog compiler (see [`super::actool`]),
/// `xcstringstool`, the String Catalog compiler (see [`super::xcstringstool`]),
/// and the small Darwin tools like `lipo` (see [`super::darwin_tools`]), with
/// the Darwin SDK's `linker` beside them for the linker stand-in to run.
fn link_tools(dir: &Path, linker: Option<&Path>) -> Result<(), CliError> {
    let fail = |e: std::io::Error| {
        CliError::new(format!(
            "couldn't link the stand-ins for Xcode's tools into {}: {e}",
            dir.display()
        ))
    };
    let exe = std::env::current_exe().map_err(fail)?;
    std::fs::create_dir_all(dir).map_err(fail)?;
    let tools = ["actool", "momc", "xcstringstool"]
        .iter()
        .chain(super::darwin_tools::TOOLS)
        .map(|tool| (*tool, exe.as_path()))
        .chain(linker.map(|l| (super::darwin_tools::SDK_LINKER, l)));
    for (name, target) in tools {
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, &path).map_err(fail)?;
        #[cfg(not(unix))]
        std::fs::copy(target, &path).map_err(fail)?;
    }
    Ok(())
}

/// The `swbuild` to run: `SWEETPAD_SWBUILD`, else the one on `PATH`. The
/// toolchain doesn't ship it, so a missing one gets a pointer to its source.
fn swbuild_program() -> Result<PathBuf, CliError> {
    if let Some(path) = std::env::var_os("SWEETPAD_SWBUILD") {
        return Ok(PathBuf::from(path));
    }
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("swbuild"))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            CliError::new("swbuild isn't on PATH")
                .kind(ErrorKind::ToolMissing)
                .tip(Some(
                    "build its swbuild and SWBBuildServiceBundle products from \
                     github.com/swiftlang/swift-build and keep them side by side, or point \
                     SWEETPAD_SWBUILD at a swbuild"
                        .to_string(),
                ))
        })
}

/// The arguments after `--` that Swift Build can take: build settings
/// (`KEY=VALUE`), `-derivedDataPath` and `-xcconfig`. Any other flag is
/// xcodebuild's own.
fn refuse_xcodebuild_flags(passthrough: &[String]) -> Result<(), CliError> {
    let mut args = passthrough.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-derivedDataPath" | "-xcconfig" => {
                args.next();
            }
            flag if flag.starts_with('-') => {
                return Err(CliError::new(format!(
                    "'{flag}' is an xcodebuild flag, and this build runs on Swift Build, which \
                     takes only build settings ('KEY=VALUE'), '-derivedDataPath' and '-xcconfig' \
                     after '--'"
                ))
                .kind(ErrorKind::Usage));
            }
            _ => {}
        }
    }
    Ok(())
}

/// The targets a scheme builds for `build_for`, by name, the way Swift Build
/// looks them up. A scheme with no file stands for the target of its name.
/// Building for testing adds what the test action runs, as `xcodebuild
/// build-for-testing` does: the default test plan's enabled targets, or
/// without plans the testables that aren't skipped. Entries from Swift
/// packages are left out, since the targets that use a package build it as a
/// dependency.
fn scheme_targets(
    container: &Path,
    scheme: &str,
    build_for: BuildFor,
) -> Result<Vec<String>, CliError> {
    let Some(file) = sweetpad_lib::scheme::locate(container, scheme) else {
        return Ok(vec![scheme.to_string()]);
    };
    let Ok(parsed) = sweetpad_lib::scheme::parse_file(&file) else {
        return Ok(vec![scheme.to_string()]);
    };
    let in_project = |container: &str| container.trim_end_matches('/').ends_with(".xcodeproj");
    let mut targets: Vec<String> = Vec::new();
    let mut add = |name: &str| {
        if !targets.iter().any(|t| t == name) {
            targets.push(name.to_string());
        }
    };
    for entry in parsed
        .build_entries
        .iter()
        .filter(|e| e.builds_for(build_for))
        .filter(|e| in_project(&e.buildable.container))
    {
        add(&entry.buildable.blueprint_name);
    }
    if build_for == BuildFor::Testing
        && let Some(test) = &parsed.test_action
    {
        let plan = test
            .test_plans
            .iter()
            .find(|p| p.default)
            .or(test.test_plans.first());
        if let Some(plan) = plan {
            // `container:` paths are relative to the folder the scheme's
            // project or workspace is in.
            let base = file
                .ancestors()
                .find(|a| {
                    a.extension()
                        .is_some_and(|e| e == "xcodeproj" || e == "xcworkspace")
                })
                .and_then(Path::parent)
                .unwrap_or(Path::new("."));
            let path = base.join(&plan.path);
            let plan_targets = sweetpad_lib::scheme::test_plan_targets(&path).ok_or_else(|| {
                CliError::new(format!(
                    "couldn't read test plan {} of scheme '{scheme}'",
                    path.display()
                ))
            })?;
            for target in plan_targets
                .iter()
                .filter(|t| t.enabled && in_project(&t.container))
            {
                add(&target.name);
            }
        } else {
            for testable in test
                .testables
                .iter()
                .filter(|t| !t.skipped && in_project(&t.buildable.container))
            {
                add(&testable.buildable.blueprint_name);
            }
        }
    }
    if targets.is_empty() {
        return Err(CliError::new(format!(
            "scheme '{scheme}' builds no project targets for this action"
        ))
        .kind(ErrorKind::TargetResolution));
    }
    Ok(targets)
}

/// Swift Build's build parameters: the configuration, the run destination,
/// where the build writes (the folders `xcodebuild` uses, so the product
/// locator finds the app), and the settings that override the project's.
fn parameters(
    configuration: &str,
    sdk: &str,
    arch: &str,
    locations: &Locations,
    overrides: &BTreeMap<String, String>,
    xcconfig: Option<&Path>,
) -> Value {
    let platform = sdk.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    let variant = if platform == "macosx" {
        "macos"
    } else {
        platform
    };
    let folder = &locations.folder;
    let mut overrides_json = json!({ "commandLine": { "table": overrides } });
    if let Some(xcconfig) = xcconfig {
        overrides_json["commandLineConfigPath"] = json!(xcconfig);
    }
    json!({
        "action": "build",
        "configurationName": configuration,
        "activeRunDestination": {
            "platform": platform,
            "sdk": sdk,
            "sdkVariant": variant,
            "targetArchitecture": arch,
            "supportedArchitectures": [arch],
            "disableOnlyActiveArch": false,
        },
        "arenaInfo": {
            "derivedDataPath": folder,
            "buildProductsPath": locations.products,
            "buildIntermediatesPath": locations.intermediates,
            "pchPath": locations.intermediates.join("PrecompiledHeaders"),
            "indexPCHPath": folder.join("Index.noindex/PrecompiledHeaders"),
            "indexDataStoreFolderPath": folder.join("Index.noindex/DataStore"),
            "indexEnableDataStore": false,
        },
        "overrides": overrides_json,
    })
}

/// A Darwin Swift SDK bundle: the Apple SDKs, Xcode's runtime libraries and a
/// linker that writes Mach-O, which stand in for Xcode where there is none.
struct DarwinSdk {
    bundle: PathBuf,
}

impl DarwinSdk {
    /// The bundle a build here needs: none on macOS, where Swift Build finds
    /// Xcode itself. Elsewhere `SWEETPAD_DARWIN_SDK`, else the one
    /// `xtool sdk install` writes.
    fn for_host() -> Result<Option<Self>, CliError> {
        if cfg!(target_os = "macos") {
            return Ok(None);
        }
        let bundle = std::env::var_os("SWEETPAD_DARWIN_SDK").map_or_else(
            || {
                PathBuf::from(std::env::var("HOME").unwrap_or_default())
                    .join(".swiftpm/swift-sdks/darwin.artifactbundle")
            },
            PathBuf::from,
        );
        if !bundle.join("toolset-swb.json").is_file() {
            return Err(CliError::new(format!(
                "no Darwin Swift SDK at {}, which a build for Apple platforms needs without Xcode",
                bundle.display()
            ))
            .kind(ErrorKind::ToolMissing)
            .tip(Some(
                "make one from an Xcode.app with 'xtool sdk install', or point \
                 SWEETPAD_DARWIN_SDK at one"
                    .to_string(),
            )));
        }
        Ok(Some(Self { bundle }))
    }

    /// The settings a build needs without Xcode's own tools.
    ///
    /// - `CODE_SIGNING_ALLOWED=NO`: the app is signed after the build, by a
    ///   tool that runs here.
    /// - `SWIFT_SDK_TOOLSETS`: the bundle's linker, which links for Apple
    ///   platforms where the toolchain's own refuses to, and `toolset`, with
    ///   what a build here needs on top (see [`toolset`]).
    /// - `LM_SKIP_METADATA_EXTRACTION`: App Intents metadata takes an
    ///   Xcode-only tool.
    /// - `DONT_RUN_SWIFT_STDLIB_TOOL`: its Mach-O reader runs only on macOS,
    ///   which matters only to an app that back-deploys Swift libraries.
    /// - `LIBRARY_SEARCH_PATHS`: the Swift compatibility libraries an older
    ///   deployment target links (`swiftCompatibility56`) are Xcode's, in the
    ///   bundle, and the toolchain here has none.
    /// - `GENERATE_INTERMEDIATE_TEXT_BASED_STUBS`: a framework's stub comes
    ///   from `tapi`, which is Xcode's; it only sharpens incremental relinks.
    /// - `STRINGS_FILE_INPUT_ENCODING`: Swift Build detects a `.strings`
    ///   file's encoding only on macOS, so a file that doesn't name one, like
    ///   a compiled String Catalog's, is read as UTF-8.
    /// - The Xcode version (see [`xcode_version_settings`]) and, for a
    ///   platform without its `version.plist`, its build as the SDK's, which
    ///   go into the app's Info.plist as `DTXcode`, `DTXcodeBuild` and
    ///   `DTPlatformBuild`.
    fn overrides(&self, sdk: &str, arch: &str, toolset: &Path) -> Vec<(String, String)> {
        let swift_libraries = self
            .bundle
            .join("Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/$(PLATFORM_NAME)");
        let mut versions = xcode_version_settings(&sweetpad_lib::derived_data::plist_strings(
            &self.bundle.join("version.plist"),
        ));
        if !self.platform_has_version(sdk) {
            versions.push((
                "PLATFORM_PRODUCT_BUILD_VERSION",
                "$(SDK_PRODUCT_BUILD_VERSION)".to_string(),
            ));
        }
        [
            ("SDKROOT", sdk.to_string()),
            (
                "LIBRARY_SEARCH_PATHS",
                format!("$(inherited) {}", swift_libraries.display()),
            ),
            ("ARCHS", arch.to_string()),
            ("ONLY_ACTIVE_ARCH", "YES".to_string()),
            ("CODE_SIGNING_ALLOWED", "NO".to_string()),
            (
                "SWIFT_SDK_TOOLSETS",
                format!(
                    "{} {}",
                    self.bundle.join("toolset-swb.json").display(),
                    toolset.display()
                ),
            ),
            ("LM_SKIP_METADATA_EXTRACTION", "YES".to_string()),
            ("GENERATE_INTERMEDIATE_TEXT_BASED_STUBS", "NO".to_string()),
            ("DONT_RUN_SWIFT_STDLIB_TOOL", "YES".to_string()),
            (
                "STRINGS_FILE_INPUT_ENCODING",
                "$(InputFileTextEncoding:default=utf-8)".to_string(),
            ),
        ]
        .into_iter()
        .chain(versions)
        .map(|(k, v)| (k.to_string(), v))
        .collect()
    }

    /// Whether the bundle has the `version.plist` of the platform `sdk` is
    /// for, which Swift Build reads the platform's build from.
    fn platform_has_version(&self, sdk: &str) -> bool {
        let platform = sdk.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
        std::fs::read_dir(self.bundle.join("Developer/Platforms"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| {
                path.file_stem()
                    .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(platform))
            })
            .is_some_and(|path| path.join("version.plist").is_file())
    }

    /// Swift Build takes the platforms from `XCODE_EXTRA_PLATFORM_FOLDERS`,
    /// with no Xcode to find them in, and looks tools like `dsymutil` and
    /// `xcstringstool` up on `PATH`, so the stand-ins in `tools` go first,
    /// then the bundle's.
    fn env(&self, tools: &Path) -> Vec<(String, Option<String>)> {
        let mut path = vec![tools.to_path_buf(), self.bundle.join("toolset/bin")];
        path.extend(
            std::env::var_os("PATH")
                .iter()
                .flat_map(std::env::split_paths),
        );
        vec![
            (
                "PATH".to_string(),
                std::env::join_paths(path)
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned()),
            ),
            (
                "XCODE_EXTRA_PLATFORM_FOLDERS".to_string(),
                Some(
                    self.bundle
                        .join("Developer/Platforms")
                        .display()
                        .to_string(),
                ),
            ),
            ("SDKROOT".to_string(), None),
            ("DEVELOPER_DIR".to_string(), None),
        ]
    }
}

/// The Xcode version settings Swift Build fills from Xcode.app's
/// `version.plist`, from that file's keys, as Swift Build computes them:
/// 26.4.1 is `XCODE_VERSION_ACTUAL` 2641. Without Xcode it reports 99.99.99,
/// so these come from the copy of the file beside the Darwin SDK's
/// `Developer` folder, where Xcode.app keeps it. None without one.
fn xcode_version_settings(version_plist: &[(String, String)]) -> Vec<(&'static str, String)> {
    let key = |name: &str| {
        version_plist
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let (Some(version), Some(build)) = (
        key("CFBundleShortVersionString"),
        key("ProductBuildVersion"),
    ) else {
        return Vec::new();
    };
    let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor, patch) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    vec![
        ("XCODE_VERSION_MAJOR", format!("{:04}", major * 100)),
        (
            "XCODE_VERSION_MINOR",
            format!("{:04}", major * 100 + minor * 10),
        ),
        (
            "XCODE_VERSION_ACTUAL",
            format!("{:04}", major * 100 + minor * 10 + patch),
        ),
        ("XCODE_PRODUCT_BUILD_VERSION", build.to_string()),
    ]
}

/// swbuild's output as [`buildlog`] events.
///
/// swbuild writes each message as its byte count on one line and the JSON
/// object on the next. Task starts become task events, diagnostics become
/// diagnostics, and the build's end becomes its banner. A task that fails
/// without an error of its own (a run script, say) becomes an error that
/// carries the task's output, which would otherwise go unseen, and so do the
/// warnings a successful link leaves in its output. Text lines are swbuild's
/// own errors, written before or after the build.
#[derive(Debug, Default)]
pub struct MessageParser {
    /// What each started task is, by task ID.
    tasks: HashMap<i64, Task>,
    /// Each task's output so far, by task ID.
    output: HashMap<i64, String>,
    /// Tasks that reported an error of their own, which explains their
    /// failure.
    diagnosed: HashSet<i64>,
    /// Whether any task has reported an error.
    errors: bool,
    /// The build has finished, so swbuild's closing `error: build failed`
    /// repeats what the banner says.
    finished: bool,
}

#[derive(Debug)]
struct Task {
    description: String,
    /// A task of the Swift driver's plan. It fails along with the compile
    /// it stands for, whose errors already say why.
    swift: bool,
    /// A link, whose output can hold warnings no diagnostic carries.
    link: bool,
}

impl MessageParser {
    /// The events one line of swbuild's output completes.
    pub fn push(&mut self, line: &str) -> Vec<Parsed> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.bytes().all(|b| b.is_ascii_digit()) {
            return Vec::new();
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(message) if message.is_object() => self.message(&message),
            _ => self.text(trimmed).into_iter().collect(),
        }
    }

    fn message(&mut self, m: &Value) -> Vec<Parsed> {
        let int = |key: &str| m.get(key).and_then(Value::as_i64);
        let text = |key: &str| m.get(key).and_then(Value::as_str).unwrap_or_default();
        match text("kind") {
            "taskStarted" => {
                let description = text("executionDescription").to_string();
                let event = task_event(text("ruleInfo"), &description);
                if let Some(id) = int("id") {
                    let rule_info = text("ruleInfo");
                    self.tasks.insert(
                        id,
                        Task {
                            description: description.clone(),
                            swift: rule_info.starts_with("Swift"),
                            link: rule_info.starts_with("Ld "),
                        },
                    );
                }
                vec![Parsed {
                    raw: description,
                    event,
                }]
            }
            "taskOutput" => {
                if let Some(id) = int("taskID") {
                    self.output.entry(id).or_default().push_str(text("data"));
                }
                Vec::new()
            }
            "taskComplete" => {
                let id = int("id");
                let task = id.and_then(|id| self.tasks.get(&id));
                let output = id
                    .and_then(|id| self.output.remove(&id))
                    .unwrap_or_default();
                if text("result") != "failed" {
                    return if task.is_some_and(|t| t.link) {
                        linker_warnings(&output)
                    } else {
                        Vec::new()
                    };
                }
                if id.is_some_and(|id| self.diagnosed.contains(&id))
                    || (self.errors && task.is_some_and(|t| t.swift))
                {
                    return Vec::new();
                }
                let description = task.map_or("A build task", |t| t.description.as_str());
                failed_task(description, &output)
            }
            kind @ ("error" | "warning" | "note" | "remark") => {
                if kind == "error" {
                    self.errors = true;
                    // `Linker command failed …` points at the project or the
                    // package and says nothing of why; any other error a task
                    // reports, in a file or not, explains its failure.
                    let at_project = m
                        .pointer("/location/path")
                        .and_then(Value::as_str)
                        .is_some_and(is_container);
                    if !at_project
                        && let Some(id) =
                            m.pointer("/locationContext/taskID").and_then(Value::as_i64)
                    {
                        self.diagnosed.insert(id);
                    }
                }
                let mut out = vec![diagnostic(m)];
                out.extend(
                    m.get("childDiagnostics")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(diagnostic),
                );
                out
            }
            "buildCompleted" => {
                self.finished = true;
                let kind = if text("result") == "ok" {
                    ResultKind::BuildSucceeded
                } else {
                    ResultKind::BuildFailed
                };
                vec![Parsed {
                    raw: String::new(),
                    event: Event::Result(kind),
                }]
            }
            _ => Vec::new(),
        }
    }

    fn text(&self, line: &str) -> Option<Parsed> {
        let (kind, message) = if let Some(message) = line.strip_prefix("error: ") {
            if self.finished && message == "build failed" {
                return None;
            }
            (DiagKind::Error, message)
        } else if let Some(message) = line.strip_prefix("warning: ") {
            (DiagKind::Warning, message)
        } else {
            return Some(Parsed {
                raw: line.to_string(),
                event: Event::Other(line.to_string()),
            });
        };
        Some(Parsed {
            raw: line.to_string(),
            event: Event::Diagnostic {
                kind,
                location: None,
                message: message.to_string(),
            },
        })
    }
}

/// The event a task's start stands for, by the rule it runs. The name is the
/// task's description less its verb and architecture: `Compile Foo.swift
/// (arm64)` is `Foo.swift`.
fn task_event(rule_info: &str, description: &str) -> Event {
    let name = description
        .split_once(' ')
        .map_or(description, |(_, rest)| rest);
    let name = match name.rsplit_once(" (") {
        Some((name, arch)) if arch.ends_with(')') && !arch.contains(' ') => name,
        _ => name,
    }
    .to_string();
    match rule_info.split_whitespace().next().unwrap_or_default() {
        // The Swift driver's plan lists each file as `Compiling Foo.swift`
        // before the compile itself runs as `Compile Foo.swift`; only the
        // second is a compile, so a file shows once.
        "SwiftCompile" | "CompileSwift" | "CompileC" if description.starts_with("Compile ") => {
            Event::Compile { name }
        }
        "Ld" => Event::Link { target: name },
        "CodeSign" => Event::CodeSign { name },
        "CpResource" | "CopySwiftLibs" | "Copy" | "Ditto" | "CopyPNGFile" | "CopyStringsFile"
        | "CpHeader" => Event::Copy { name },
        "ProcessInfoPlistFile" => Event::ProcessPlist { name },
        _ => Event::Other(description.to_string()),
    }
}

/// A diagnostic message as an event, with `path:line:column` when it points
/// into a file. A note that points nowhere is Swift Build's commentary
/// (`Building targets in dependency order`), shown only under `-v`.
fn diagnostic(m: &Value) -> Parsed {
    let kind = match m.get("kind").and_then(Value::as_str) {
        Some("error") => DiagKind::Error,
        Some("warning") => DiagKind::Warning,
        _ => DiagKind::Note,
    };
    let message = m
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let location = m.get("location").and_then(|l| {
        let path = l.get("path")?.as_str()?;
        Some(
            match (
                l.get("line").and_then(Value::as_i64),
                l.get("column").and_then(Value::as_i64),
            ) {
                (Some(line), Some(column)) => format!("{path}:{line}:{column}"),
                (Some(line), None) => format!("{path}:{line}"),
                _ => path.to_string(),
            },
        )
    });
    let severity = match kind {
        DiagKind::Error => "error",
        DiagKind::Warning => "warning",
        DiagKind::Note => "note",
    };
    let raw = match &location {
        Some(location) => format!("{location}: {severity}: {message}"),
        None => format!("{severity}: {message}"),
    };
    if kind == DiagKind::Note && location.is_none() {
        return Parsed {
            event: Event::Other(raw.clone()),
            raw,
        };
    }
    Parsed {
        raw,
        event: Event::Diagnostic {
            kind,
            location,
            message,
        },
    }
}

/// The warnings a successful link printed, once each, at the project.
///
/// Swift Build turns the linker's `ld:` and `clang:` lines into diagnostics
/// of its own but leaves lld's (`ld64.lld: warning: …`) in the task's output,
/// where it puts the project or package and the target in front of each:
/// `/p/App.xcodeproj: App: ld64.lld: warning: …`.
fn linker_warnings(output: &str) -> Vec<Parsed> {
    let mut seen = HashSet::new();
    output
        .lines()
        .filter_map(|line| {
            let (head, message) = line.split_once(": warning: ")?;
            let tool = head.rsplit(": ").next().unwrap_or(head);
            if matches!(tool, "ld" | "clang") || lld_platform_notice(message) {
                return None;
            }
            let project = head
                .split_once(": ")
                .map(|(project, _)| project)
                .filter(|project| is_container(project));
            seen.insert((project, message)).then(|| Parsed {
                raw: match project {
                    Some(project) => format!("{project}: warning: {message}"),
                    None => format!("warning: {message}"),
                },
                event: Event::Diagnostic {
                    kind: DiagKind::Warning,
                    location: project.map(str::to_string),
                    message: message.to_string(),
                },
            })
        })
        .collect()
}

/// Whether `path` is a project or a package manifest, where Swift Build
/// puts what a target's task says rather than in one of its files.
fn is_container(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    path.ends_with(".xcodeproj") || path.ends_with("/Package.swift")
}

/// lld's notice that it "does not support linking for platform iOS", which
/// it repeats for every input of every link for an Apple platform. It's about
/// the Darwin SDK's linker rather than the project, and the products it links
/// run, so it's left out.
fn lld_platform_notice(message: &str) -> bool {
    message.starts_with("This version of lld does not support linking for platform ")
}

/// A failed task that reported no error: an error naming it, and its output
/// on the lines under it.
fn failed_task(description: &str, output: &str) -> Vec<Parsed> {
    let lines: Vec<&str> = output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| {
            l.split_once(": warning: ")
                .is_none_or(|(_, message)| !lld_platform_notice(message))
        })
        .collect();
    let message = if lines.is_empty() {
        format!("{description} failed")
    } else {
        format!("{description} failed:")
    };
    let mut out = vec![Parsed {
        raw: format!("error: {message}"),
        event: Event::Diagnostic {
            kind: DiagKind::Error,
            location: None,
            message,
        },
    }];
    out.extend(lines.into_iter().map(|line| Parsed {
        raw: format!("    {line}"),
        event: Event::Other(line.to_string()),
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(lines: &[&str]) -> Vec<Event> {
        let mut parser = MessageParser::default();
        lines
            .iter()
            .flat_map(|line| parser.push(line))
            .map(|parsed| parsed.event)
            .collect()
    }

    #[test]
    fn tasks_become_task_events_once_per_file() {
        let found = events(&[
            "120",
            r#"{"kind":"taskStarted","id":7,"ruleInfo":"SwiftCompile normal arm64 Compiling\\ A.swift /p/A.swift","executionDescription":"Compiling A.swift"}"#,
            r#"{"kind":"taskStarted","id":9,"ruleInfo":"SwiftCompile normal arm64 /p/A.swift","executionDescription":"Compile A.swift (arm64)"}"#,
            r#"{"kind":"taskStarted","id":10,"ruleInfo":"Ld /dd/App.app/App normal","executionDescription":"Link App (arm64)"}"#,
            r#"{"kind":"taskStarted","id":11,"ruleInfo":"ProcessInfoPlistFile /dd/App.app/Info.plist /p/Info.plist","executionDescription":"Process Info.plist"}"#,
        ]);
        assert_eq!(
            found,
            vec![
                Event::Other("Compiling A.swift".into()),
                Event::Compile {
                    name: "A.swift".into()
                },
                Event::Link {
                    target: "App".into()
                },
                Event::ProcessPlist {
                    name: "Info.plist".into()
                },
            ]
        );
    }

    #[test]
    fn diagnostics_carry_their_location_and_notes_without_one_are_chatter() {
        let found = events(&[
            r#"{"kind":"error","message":"cannot convert","location":{"locationType":"path","path":"/p/A.swift","line":5,"column":31},"locationContext":{"locationType":"task","taskID":9,"targetID":0},"childDiagnostics":[{"kind":"note","message":"declared here","location":{"locationType":"path","path":"/p/B.swift","line":2}}]}"#,
            r#"{"kind":"note","message":"Building targets in dependency order","location":{"locationType":"unknown"}}"#,
        ]);
        assert_eq!(
            found,
            vec![
                Event::Diagnostic {
                    kind: DiagKind::Error,
                    location: Some("/p/A.swift:5:31".into()),
                    message: "cannot convert".into(),
                },
                Event::Diagnostic {
                    kind: DiagKind::Note,
                    location: Some("/p/B.swift:2".into()),
                    message: "declared here".into(),
                },
                Event::Other("note: Building targets in dependency order".into()),
            ]
        );
    }

    #[test]
    fn a_task_failing_without_an_error_reports_its_output() {
        let found = events(&[
            r#"{"kind":"taskStarted","id":3,"ruleInfo":"PhaseScriptExecution Lint /dd/Script.sh","executionDescription":"Run custom shell script 'Lint'"}"#,
            r#"{"kind":"taskOutput","taskID":3,"data":"lint: 2 problems\n\n"}"#,
            r#"{"kind":"taskComplete","id":3,"result":"failed"}"#,
        ]);
        assert_eq!(
            found[1..],
            [
                Event::Diagnostic {
                    kind: DiagKind::Error,
                    location: None,
                    message: "Run custom shell script 'Lint' failed:".into(),
                },
                Event::Other("lint: 2 problems".into()),
            ]
        );
    }

    #[test]
    fn a_failure_already_explained_adds_nothing() {
        let found = events(&[
            r#"{"kind":"taskStarted","id":7,"ruleInfo":"SwiftCompile normal arm64 Compiling\\ A.swift /p/A.swift","executionDescription":"Compiling A.swift"}"#,
            r#"{"kind":"taskStarted","id":9,"ruleInfo":"SwiftCompile normal arm64 /p/A.swift","executionDescription":"Compile A.swift (arm64)"}"#,
            r#"{"kind":"error","message":"bad","location":{"locationType":"path","path":"/p/A.swift","line":1},"locationContext":{"locationType":"task","taskID":9,"targetID":0}}"#,
            r#"{"kind":"taskOutput","taskID":7,"data":"Failed frontend command: swift-frontend ...\n"}"#,
            r#"{"kind":"taskComplete","id":9,"result":"failed"}"#,
            r#"{"kind":"taskComplete","id":7,"result":"failed"}"#,
            r#"{"kind":"buildCompleted","result":"failed"}"#,
            "error: build failed",
        ]);
        let errors = found
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Event::Diagnostic {
                        kind: DiagKind::Error,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(errors, 1, "{found:?}");
        assert_eq!(found.last(), Some(&Event::Result(ResultKind::BuildFailed)));
    }

    #[test]
    fn a_tools_own_error_explains_its_task_without_a_file() {
        let found = events(&[
            r#"{"kind":"taskStarted","id":12,"ruleInfo":"ValidateEmbeddedBinary /dd/App.app/PlugIns/W.appex","executionDescription":"Validate W.appex"}"#,
            r#"{"kind":"taskOutput","taskID":12,"data":"error: Embedded binary's bundle identifier is not prefixed\n"}"#,
            r#"{"kind":"error","message":"Embedded binary's bundle identifier is not prefixed","location":{"locationType":"unknown"},"locationContext":{"locationType":"task","targetID":0,"taskID":12}}"#,
            r#"{"kind":"taskComplete","id":12,"result":"failed"}"#,
        ]);
        assert_eq!(
            found[1..],
            [Event::Diagnostic {
                kind: DiagKind::Error,
                location: None,
                message: "Embedded binary's bundle identifier is not prefixed".into(),
            }]
        );
    }

    #[test]
    fn an_error_on_the_project_still_shows_the_tool_output() {
        let found = events(&[
            r#"{"kind":"taskStarted","id":10,"ruleInfo":"Ld /dd/App.framework/App normal","executionDescription":"Link App (arm64)"}"#,
            r#"{"kind":"taskOutput","taskID":10,"data":"ld64.lld: error: undefined symbol: _foo\n"}"#,
            r#"{"kind":"error","message":"Linker command failed with exit code 1 (use -v to see invocation)","location":{"locationType":"path","path":"/p/App.xcodeproj"},"locationContext":{"locationType":"task","taskID":10,"targetID":0}}"#,
            r#"{"kind":"taskComplete","id":10,"result":"failed","signalled":false}"#,
        ]);
        assert!(
            found.contains(&Event::Other(
                "ld64.lld: error: undefined symbol: _foo".into()
            )),
            "{found:?}"
        );
    }

    #[test]
    fn an_error_on_a_package_still_shows_the_tool_output() {
        let found = events(&[
            r#"{"kind":"taskStarted","id":20,"ruleInfo":"Ld /dd/PackageFrameworks/Kit.framework/Kit normal","executionDescription":"Link Kit (arm64)"}"#,
            r#"{"kind":"taskOutput","taskID":20,"data":"/s/Kit/Package.swift: Kit-product: ld64.lld: error: undefined symbol: _foo\n"}"#,
            r#"{"kind":"error","message":"Linker command failed with exit code 1 (use -v to see invocation)","location":{"locationType":"path","path":"/s/Kit/Package.swift"},"locationContext":{"locationType":"task","taskID":20,"targetID":1}}"#,
            r#"{"kind":"taskComplete","id":20,"result":"failed","signalled":false}"#,
        ]);
        assert!(
            found.contains(&Event::Other(
                "/s/Kit/Package.swift: Kit-product: ld64.lld: error: undefined symbol: _foo".into()
            )),
            "{found:?}"
        );
    }

    #[test]
    fn a_successful_links_lld_warnings_show_once_each() {
        let notice = "This version of lld does not support linking for platform iOS. Proceed at your own risk.";
        let warning = "/p/App.xcodeproj: App: ld64.lld: warning: /dd/libz.tbd: unknown platform";
        let output = format!(
            "/p/App.xcodeproj: App: ld64.lld: warning: {notice}\n{warning}\n{warning}\n\
             /p/App.xcodeproj: App: ld: warning: reported by Swift Build\n"
        );
        let found = events(&[
            r#"{"kind":"taskStarted","id":10,"ruleInfo":"Ld /dd/App.app/App normal","executionDescription":"Link App (arm64)"}"#,
            &json!({"kind": "taskOutput", "taskID": 10, "data": output}).to_string(),
            r#"{"kind":"taskComplete","id":10,"result":"success"}"#,
        ]);
        assert_eq!(
            found,
            vec![
                Event::Link {
                    target: "App".into()
                },
                Event::Diagnostic {
                    kind: DiagKind::Warning,
                    location: Some("/p/App.xcodeproj".into()),
                    message: "/dd/libz.tbd: unknown platform".into(),
                },
            ]
        );
    }

    #[test]
    fn only_a_links_output_is_read_for_warnings() {
        let found = events(&[
            r#"{"kind":"taskStarted","id":7,"ruleInfo":"SwiftCompile normal arm64 /p/A.swift","executionDescription":"Compile A.swift (arm64)"}"#,
            r#"{"kind":"taskOutput","taskID":7,"data":"/p/A.swift:1:5: warning: unused\n"}"#,
            r#"{"kind":"taskComplete","id":7,"result":"success"}"#,
        ]);
        assert_eq!(
            found,
            vec![Event::Compile {
                name: "A.swift".into()
            }]
        );
    }

    #[test]
    fn a_failed_links_output_leaves_out_the_lld_platform_notice() {
        let found = events(&[
            r#"{"kind":"taskStarted","id":10,"ruleInfo":"Ld /dd/App.app/App normal","executionDescription":"Link App (arm64)"}"#,
            r#"{"kind":"taskOutput","taskID":10,"data":"ld64.lld: warning: This version of lld does not support linking for platform iOS. Proceed at your own risk.\nld64.lld: error: undefined symbol: _foo\n"}"#,
            r#"{"kind":"taskComplete","id":10,"result":"failed"}"#,
        ]);
        let other: Vec<&Event> = found
            .iter()
            .filter(|e| matches!(e, Event::Other(_)))
            .collect();
        assert_eq!(
            other,
            vec![&Event::Other(
                "ld64.lld: error: undefined symbol: _foo".into()
            )]
        );
    }

    #[test]
    fn swbuild_errors_before_the_build_are_errors() {
        assert_eq!(
            events(&["error: Could not find target named 'App'"]),
            vec![Event::Diagnostic {
                kind: DiagKind::Error,
                location: None,
                message: "Could not find target named 'App'".into(),
            }]
        );
    }

    #[test]
    fn swiftpm_errors_keep_the_lines_under_them() {
        let stderr = "Fetching https://example.com/a.git\n\
                      warning: 'a': skipping cache due to an error: Failed to clone:\n    \
                      fatal: no cache\n\
                      error: Failed to clone repository https://example.com/a.git:\n    \
                      fatal: repository not found\n\
                      /work/Kit/Package.swift:4:9: error: cannot find 'x' in scope\n";
        assert_eq!(
            swiftpm_errors(stderr),
            [
                "Failed to clone repository https://example.com/a.git:",
                "    fatal: repository not found",
                "/work/Kit/Package.swift:4:9: error: cannot find 'x' in scope",
            ]
        );
    }

    #[test]
    fn only_build_settings_pass_through() {
        let args = |list: &[&str]| list.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        assert!(
            refuse_xcodebuild_flags(&args(&[
                "OTHER_SWIFT_FLAGS=-DX",
                "-derivedDataPath",
                "dd",
                "-xcconfig",
                "a.xcconfig"
            ]))
            .is_ok()
        );
        let err = refuse_xcodebuild_flags(&args(&["-allowProvisioningUpdates"])).unwrap_err();
        assert!(
            err.to_string()
                .contains("'-allowProvisioningUpdates' is an xcodebuild flag")
        );
    }

    #[test]
    fn building_for_testing_adds_the_test_plans_targets() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-swbuild-test-plan");
        let project = dir.join("App.xcodeproj");
        let schemes = project.join("xcshareddata/xcschemes");
        std::fs::create_dir_all(&schemes).unwrap();
        let buildable = |name: &str, container: &str| {
            format!(
                r#"<BuildableReference BlueprintIdentifier="{name}" BuildableName="{name}" BlueprintName="{name}" ReferencedContainer="container:{container}"/>"#
            )
        };
        let scheme = |test_action: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme version="1.7">
   <BuildAction><BuildActionEntries>
      <BuildActionEntry buildForTesting="YES" buildForRunning="YES">{}</BuildActionEntry>
   </BuildActionEntries></BuildAction>
   <TestAction buildConfiguration="Debug">{test_action}</TestAction>
</Scheme>"#,
                buildable("App", "App.xcodeproj")
            )
        };
        let testables = format!(
            r#"<Testables>
                 <TestableReference skipped="NO">{}</TestableReference>
                 <TestableReference skipped="YES">{}</TestableReference>
                 <TestableReference skipped="NO">{}</TestableReference>
               </Testables>"#,
            buildable("AppTests", "App.xcodeproj"),
            buildable("SkippedTests", "App.xcodeproj"),
            buildable("KitTests", "Packages/Kit"),
        );
        std::fs::write(schemes.join("App.xcscheme"), scheme(&testables)).unwrap();
        assert_eq!(
            scheme_targets(&project, "App", BuildFor::Testing).unwrap(),
            ["App", "AppTests"]
        );
        assert_eq!(
            scheme_targets(&project, "App", BuildFor::Running).unwrap(),
            ["App"]
        );

        let plans = r#"<TestPlans><TestPlanReference reference="container:All.xctestplan" default="YES"/></TestPlans>"#;
        std::fs::write(
            schemes.join("App.xcscheme"),
            scheme(&format!("{plans}{testables}")),
        )
        .unwrap();
        std::fs::write(
            dir.join("All.xctestplan"),
            r#"{"testTargets": [
                {"target": {"containerPath": "container:App.xcodeproj", "identifier": "1", "name": "UITests"}},
                {"enabled": false, "target": {"containerPath": "container:App.xcodeproj", "identifier": "2", "name": "SlowTests"}}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            scheme_targets(&project, "App", BuildFor::Testing).unwrap(),
            ["App", "UITests"]
        );
    }

    #[test]
    fn xcode_version_settings_are_swift_builds() {
        let plist = |version: &str| {
            vec![
                (
                    "CFBundleShortVersionString".to_string(),
                    version.to_string(),
                ),
                ("ProductBuildVersion".to_string(), "27A266a".to_string()),
            ]
        };
        assert_eq!(
            xcode_version_settings(&plist("26.4.1")),
            [
                ("XCODE_VERSION_MAJOR", "2600".to_string()),
                ("XCODE_VERSION_MINOR", "2640".to_string()),
                ("XCODE_VERSION_ACTUAL", "2641".to_string()),
                ("XCODE_PRODUCT_BUILD_VERSION", "27A266a".to_string()),
            ]
        );
        assert_eq!(xcode_version_settings(&plist("27.0"))[2].1, "2700");
        assert!(xcode_version_settings(&[]).is_empty());
    }

    #[test]
    fn parameters_put_products_where_xcodebuild_does() {
        let locations = Locations {
            folder: PathBuf::from("/dd/App-x"),
            products: PathBuf::from("/dd/App-x/Build/Products"),
            intermediates: PathBuf::from("/dd/App-x/Build/Intermediates.noindex"),
            derived_data_root: PathBuf::from("/dd"),
        };
        let overrides = BTreeMap::from([("A".to_string(), "1".to_string())]);
        let p = parameters("Debug", "iphoneos", "arm64", &locations, &overrides, None);
        assert_eq!(p["activeRunDestination"]["platform"], "iphoneos");
        assert_eq!(p["activeRunDestination"]["sdkVariant"], "iphoneos");
        assert_eq!(
            p["arenaInfo"]["buildProductsPath"],
            "/dd/App-x/Build/Products"
        );
        assert_eq!(p["overrides"]["commandLine"]["table"]["A"], "1");
        let mac = parameters("Debug", "macosx", "arm64", &locations, &overrides, None);
        assert_eq!(mac["activeRunDestination"]["sdkVariant"], "macos");
    }
}
