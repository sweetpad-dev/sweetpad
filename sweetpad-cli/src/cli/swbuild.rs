//! Building with Swift Build (`swbuild`), the open-source engine under Xcode's
//! build system, in place of `xcodebuild`.
//!
//! Swift Build takes no project files. It reads the project interchange
//! format (PIF) Xcode translates a workspace into, so a build writes the
//! workspace's PIF ([`sweetpad_lib::workspace_pif::pif`]), with the Swift packages it
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
use std::fmt::Write as _;
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

mod host_macros;
pub(crate) mod scheme_actions;
mod test_manifest;

pub(crate) use test_manifest::localize as localize_manifest;

/// The engine a build runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Engine {
    Xcodebuild,
    #[value(name = "swift-build")]
    SwiftBuild,
}

impl Engine {
    /// Commands without an --engine flag still honor the shared environment selection.
    pub fn selected() -> Result<Self, CliError> {
        match std::env::var("SWEETPAD_ENGINE") {
            Ok(value) => <Self as clap::ValueEnum>::from_str(&value, false).map_err(|error| {
                CliError::new(format!("SWEETPAD_ENGINE: {error}")).kind(ErrorKind::Usage)
            }),
            Err(std::env::VarError::NotPresent) => Ok(Self::for_host()),
            Err(error) => {
                Err(CliError::new(format!("SWEETPAD_ENGINE: {error}")).kind(ErrorKind::Usage))
            }
        }
    }
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

/// Remove this container's generated products without loading its SDK.
pub fn clean(container: &Container, passthrough: &[String]) -> Result<(), CliError> {
    let settings = xcodebuild::command_line_settings(passthrough, container);
    let locations = derived_data::resolve(
        &ContainerKey::of(container.path()),
        &std::env::var("HOME").unwrap_or_default(),
        settings.derived_data_path.as_deref(),
        true,
    );
    for path in [&locations.products, &locations.intermediates] {
        match std::fs::remove_dir_all(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(CliError::new(format!("clean {}: {error}", path.display()))),
        }
    }
    Ok(())
}

/// The test manifest for the products this Swift Build invocation just built.
/// Kept separate from build statistics so a successful compilation still
/// reports success when a test option needs a user-supplied manifest.
pub fn xctestrun(plan: &BuildPlan<'_>) -> Result<PathBuf, String> {
    let invocation = Invocation::of(plan).map_err(|e| e.to_string())?;
    test_manifest::write(plan, &invocation)
}

/// Build `plan` with Swift Build, rendering its progress the way an
/// `xcodebuild` build renders in every output mode, and recording the
/// diagnostics `build diagnostics` reads back.
pub fn run(plan: &BuildPlan<'_>, out: &Output) -> Result<Option<buildlog::BuildStats>, CliError> {
    let invocation = Invocation::of(plan)?;
    let folder = &invocation.locations.folder;
    let _lock = BuildLock::acquire(folder, || {
        out.note(&format!(
            "waiting for another build in {} to finish",
            folder.display()
        ));
    })?;
    if plan.clean {
        clean(plan.container, plan.passthrough)?;
    }
    let actions = scheme_actions::Actions::load(plan, &invocation)?;
    scheme_actions::build(actions.as_ref(), false, plan, &invocation, out)?;
    invocation.write_inputs(plan, out)?;
    let start = Instant::now();
    let (program, arguments) = darwin_shell_command(&invocation.program, &invocation.args)?;
    let args: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let mut parser = MessageParser::default();
    let mut diagnostics = Vec::new();
    let mut transcript = String::new();
    let (verbose, quiet) = output_levels(plan.passthrough);
    let mut progress = (!out.is_json() && !out.is_ndjson())
        .then(|| BuildProgress::start(out, "Building").with_levels(verbose, quiet));
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
    let mut env = invocation.env.clone();
    let secrets =
        super::codesign::HiddenSecrets::hide_environment(&mut env, true).map_err(CliError::new)?;
    let status =
        process::stream_build_lines(&program, &args, invocation.cwd.as_deref(), &env, |line| {
            transcript.push_str(line);
            transcript.push('\n');
            parser.push(line).iter().for_each(&mut take);
        })?;
    if let Some(signal) = crate::cli::signals::interrupted_build() {
        process::stop_leftovers();
        drop(secrets);
        crate::cli::signals::finish(signal);
    }
    let ok = status.success();
    if !ok
        && let Some(progress) = &mut progress
        && let Some(banner) = progress.close_failed(ResultKind::BuildFailed)
    {
        out.line(&banner);
    }
    xcodebuild::record_build(plan.container, Some(plan.scheme), ok, &diagnostics);
    if ok {
        scheme_actions::build(actions.as_ref(), true, plan, &invocation, out)?;
        return Ok(Some(buildlog::BuildStats::tally(
            &diagnostics,
            start.elapsed(),
        )));
    }
    let log = xcodebuild::project_artifact(plan.container, "-build.log");
    let recorded = xcodebuild::record_failure_transcript(&log, &transcript);
    let detail = recorded
        .as_ref()
        .map_or_else(String::new, |p| format!("; full log: {}", p.display()));
    let killer = killed_by(status, program == "bwrap");
    if killer.is_some() {
        // Its tasks outlive it, with nothing left to finish or stop them.
        process::stop_leftovers();
    }
    let ended = killer.map_or_else(
        || "swbuild exited with a non-zero status".to_string(),
        |signal| format!("swbuild was killed by {}", signal_name(signal)),
    );
    let tip = case_mismatches(&diagnostics)
        .into_iter()
        .chain(failure_tip(killer, recorded.as_deref()))
        .reduce(|a, b| format!("{a}; {b}"));
    Err(xcodebuild::tool_failure(
        &ended,
        &invocation.args,
        diagnostics,
        parser.blocker(),
        progress.is_some(),
        !Output::streams_share_a_file(),
        &detail,
    )
    .context("building the project")
    .tip(tip))
}

/// What Swift Build couldn't find that's on disk under another case, as
/// `'…/Core.swift' is '…/core.swift' on disk`. A Mac's file system ignores
/// case, so a project can name a file one way and hold it another until it
/// builds on Linux.
fn case_mismatches(diagnostics: &[Value]) -> Option<String> {
    let mut found: Vec<String> = Vec::new();
    for message in diagnostics.iter().filter_map(|d| d["message"].as_str()) {
        let Some(path) = message
            .strip_prefix("Build input file cannot be found: '")
            .and_then(|rest| rest.split_once('\''))
            .map(|(path, _)| Path::new(path))
        else {
            continue;
        };
        if let Some(real) = other_case(path) {
            let hint = format!("'{}' is '{}' on disk", path.display(), real.display());
            if !found.contains(&hint) {
                found.push(hint);
            }
        }
    }
    (!found.is_empty()).then(|| {
        format!(
            "{}, and Linux file names are case-sensitive",
            found.join(", ")
        )
    })
}

/// The file at `path` with each name in it matched without regard to case,
/// when that finds one and `path` itself isn't there.
fn other_case(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return None;
    }
    let mut real = PathBuf::new();
    for component in path.components() {
        let name = component.as_os_str();
        let next = real.join(name);
        if next.exists() || matches!(component, std::path::Component::RootDir) {
            real = next;
            continue;
        }
        let wanted = name.to_str()?.to_lowercase();
        let entry = std::fs::read_dir(&real).ok()?.flatten().find(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.to_lowercase() == wanted)
        })?;
        real.push(entry.file_name());
    }
    Some(real)
}

/// Where a failed build points next: the log it left in `recorded`, after
/// the out-of-memory killer when SIGKILL ended it on Linux.
fn failure_tip(killer: Option<i32>, recorded: Option<&Path>) -> Option<String> {
    let read_log = recorded.map(|p| format!("read full build log: {}", p.display()));
    if killer == Some(libc::SIGKILL) && cfg!(target_os = "linux") {
        Some(format!(
            "the kernel's out-of-memory killer sends SIGKILL, and 'journalctl -k' says if it did{}",
            read_log.map_or_else(String::new, |tip| format!("; {tip}"))
        ))
    } else {
        read_log
    }
}

/// The signal that killed `swbuild`, if one did. `bwrap`, which runs it
/// where `/bin/sh` isn't Bash, exits with 128 plus that signal's number.
fn killed_by(status: std::process::ExitStatus, wrapped: bool) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt as _;
    status.signal().or_else(|| {
        status
            .code()
            .filter(|&code| wrapped && code > 128 && code < 128 + 32)
            .map(|code| code - 128)
    })
}

/// A signal's name, like `SIGKILL`, or its number.
fn signal_name(signal: i32) -> String {
    [
        (libc::SIGHUP, "SIGHUP"),
        (libc::SIGINT, "SIGINT"),
        (libc::SIGQUIT, "SIGQUIT"),
        (libc::SIGILL, "SIGILL"),
        (libc::SIGTRAP, "SIGTRAP"),
        (libc::SIGABRT, "SIGABRT"),
        (libc::SIGBUS, "SIGBUS"),
        (libc::SIGFPE, "SIGFPE"),
        (libc::SIGKILL, "SIGKILL"),
        (libc::SIGSEGV, "SIGSEGV"),
        (libc::SIGPIPE, "SIGPIPE"),
        (libc::SIGTERM, "SIGTERM"),
    ]
    .iter()
    .find(|(number, _)| *number == signal)
    .map_or_else(
        || format!("signal {signal}"),
        |(_, name)| (*name).to_string(),
    )
}

/// One build's hold on its DerivedData folder. Swift Build keeps a build
/// database there that a second build at the same time can't open, and a
/// build rewrites its inputs there before it runs, so a second build waits
/// for the first instead of failing halfway or reading the first one's
/// request. The hold ends when this value drops.
struct BuildLock {
    _held: Option<std::fs::File>,
}

impl BuildLock {
    /// Waits for any other build in `folder`, calling `waiting` first when
    /// there is one. A filesystem without locks builds without one.
    fn acquire(folder: &Path, waiting: impl FnOnce()) -> Result<Self, CliError> {
        let path = folder.join("sweetpad-build.lock");
        let file = std::fs::create_dir_all(folder).and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&path)
        });
        let Ok(file) = file else {
            return Ok(Self { _held: None });
        };
        match file.try_lock() {
            Ok(()) => Ok(Self { _held: Some(file) }),
            Err(std::fs::TryLockError::WouldBlock) => {
                waiting();
                file.lock()
                    .map_err(|e| CliError::new(format!("couldn't lock {}: {e}", path.display())))?;
                Ok(Self { _held: Some(file) })
            }
            Err(std::fs::TryLockError::Error(_)) => Ok(Self { _held: None }),
        }
    }
}

/// macOS /bin/sh accepts Bash syntax. Keep that contract for scripts that
/// invoke it explicitly, without changing the Linux machine's system shell.
pub(crate) fn darwin_shell_command(
    program: &Path,
    arguments: &[String],
) -> Result<(String, Vec<String>), CliError> {
    let program = program.to_string_lossy().into_owned();
    if !cfg!(target_os = "linux") {
        return Ok((program, arguments.to_vec()));
    }
    let shell = std::fs::canonicalize("/bin/sh").map_err(|e| CliError::new(e.to_string()))?;
    let bash = std::fs::canonicalize("/bin/bash").map_err(|e| CliError::new(e.to_string()))?;
    if shell == bash {
        return Ok((program, arguments.to_vec()));
    }
    if super::darwin_tools::find_program("bwrap").is_none() {
        return Err(
            CliError::new("building Xcode scripts on Linux needs 'bubblewrap' and Bash")
                .kind(ErrorKind::ToolMissing)
                .tip(Some(
                    "install Bash and bubblewrap with your distribution's package manager"
                        .to_string(),
                )),
        );
    }
    let mut args = vec![
        "--die-with-parent".into(),
        "--bind".into(),
        "/".into(),
        "/".into(),
        "--dev-bind".into(),
        "/dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
        "--ro-bind".into(),
        bash.display().to_string(),
        shell.display().to_string(),
        "--".into(),
        program,
    ];
    args.extend(arguments.iter().cloned());
    Ok(("bwrap".into(), args))
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
    /// The plugin for Apple's macros that the Darwin SDK has none for (see
    /// [`macro_plugin`]).
    macros: Option<PathBuf>,
    /// Destination headers used by pkg-config while resolving package PIFs.
    package_sysroot: Option<PathBuf>,
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
///   [`super::darwin_tools`]);
/// - `macros`, the plugin that expands the macros of
///   [`sweetpad_lib::xcode::DARWIN_MACRO_MODULES`].
fn toolset(tools: &Path, macros: Option<&Path>) -> String {
    let mut options = vec![
        "-Xfrontend".to_string(),
        "-enable-cross-import-overlays".to_string(),
    ];
    if let Some(plugin) = macros {
        options.push("-load-plugin-executable".to_string());
        options.push(format!(
            "{}#{}",
            plugin.display(),
            sweetpad_lib::xcode::DARWIN_MACRO_MODULES.join(",")
        ));
    }
    let toolset = serde_json::json!({
        "schemaVersion": "1.0",
        "swiftCompiler": { "extraCLIOptions": options },
        "linker": { "path": tools.join("ld64.lld") },
    });
    format!("{toolset:#}\n")
}

/// `sweetpad-macros`: see [`sweetpad_lib::xcode::darwin_macro_plugin`].
pub(crate) fn macro_plugin() -> Option<PathBuf> {
    sweetpad_lib::xcode::darwin_macro_plugin()
}

/// `swbuild build` with the parameters file, the PIF and the build request,
/// which names the scheme's targets by GUID. A `--target` would name them
/// again, and `swbuild` looks those names up before the request replaces
/// them, failing on a name two targets share, like a package's product and
/// module.
fn swbuild_args(plan: &BuildPlan<'_>, parameters_file: &Path, pif: &Path) -> Vec<String> {
    vec![
        "build".to_string(),
        "--configuration".to_string(),
        plan.configuration.to_string(),
        "--buildParametersFile".to_string(),
        parameters_file.display().to_string(),
        "--buildRequestFile".to_string(),
        pif.with_file_name("build-request.json")
            .display()
            .to_string(),
        pif.display().to_string(),
    ]
}

impl Invocation {
    #[allow(clippy::too_many_lines)] // one linear assembly: guards, locations, overrides, env
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
                "hot reload needs an xcodebuild build; select SWEETPAD_ENGINE=xcodebuild",
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
        let tools = host
            .as_ref()
            .map(|h| tools_dir(&inputs, &h.bundle))
            .transpose()?;
        let toolset = host.as_ref().map(|_| inputs.join("toolset.json"));
        let mut overrides: BTreeMap<String, String> = XCODE_DEFAULTS
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        if let (Some(host), Some(toolset)) = (&host, &toolset) {
            overrides.extend(host.embedded_platform_overrides(&sdk, &destination.arch, toolset));
            overrides.extend(host.overrides(&sdk, &destination.arch, toolset));
        }
        let every_arch = plan.action == BuildAction::Archive || builds_every_arch(plan.destination);
        if host.is_some() && every_arch {
            standard_architectures(&mut overrides);
        }
        let archive_signs =
            host.is_some() && plan.action == BuildAction::Archive && DarwinSdk::is_signed(&sdk);
        if archive_signs {
            sign_archive(&mut overrides);
        }
        if let Some(tools) = &tools {
            let tool = |name: &str| tools.join(name).display().to_string();
            overrides.insert("CHMOD".to_string(), tool("sweetpad-chmod"));
            overrides.insert("ASSETCATALOG_EXEC".to_string(), tool("actool"));
        }
        let mut additions = BTreeMap::new();
        if host.is_some() {
            scope_darwin_settings(&mut overrides, &sdk);
            additions = take_additions(&mut overrides);
        }
        overrides.extend(settings.overrides.iter().cloned());
        if host.is_some() {
            scope_darwin_settings(&mut overrides, &sdk);
        }
        let mut parameters = parameters(
            plan.configuration,
            &sdk,
            &destination.arch,
            &locations,
            (&overrides, &additions),
            settings.xcconfig.as_deref(),
        );

        if plan.action == BuildAction::Archive {
            parameters["action"] = json!("install");
        }
        if every_arch {
            parameters["activeRunDestination"]["disableOnlyActiveArch"] = json!(true);
        }

        Ok(Self {
            program: swbuild_program()?,
            args: swbuild_args(plan, &parameters_file, &pif),
            cwd: xcodebuild::working_dir(plan.container),
            linker: host.as_ref().map(|h| h.bundle.join("toolset/bin/ld64.lld")),
            macros: host.as_ref().and_then(|_| macro_plugin()),
            package_sysroot: host.as_ref().map(|h| h.sdk_root(&sdk)).transpose()?,
            env: host
                .zip(tools.as_deref())
                .map(|(h, tools)| {
                    h.env(
                        tools,
                        DarwinSdk::signs(&sdk) || archive_signs,
                        archive_signs,
                        run_device(plan.destination, &sdk),
                    )
                })
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
    fn write_inputs(&self, plan: &BuildPlan<'_>, out: &Output) -> Result<(), CliError> {
        let translate = |e: sweetpad_lib::project::Error| {
            CliError::new(format!(
                "couldn't translate {} for Swift Build: {e}",
                self.container.display()
            ))
        };
        let mut pif = sweetpad_lib::workspace_pif::pif(&self.container).map_err(translate)?;
        for member in
            sweetpad_lib::workspace_pif::missing_members(&self.container).map_err(translate)?
        {
            out.warn(&format!(
                "{} isn't on disk, so this build leaves out the targets it would have; Xcode shows it as missing",
                member.display()
            ));
        }
        let packages = sweetpad_lib::workspace_pif::packages(&self.container).map_err(translate)?;
        if !packages.is_empty() {
            let dumped = self.package_pif(&packages, out)?;
            sweetpad_lib::workspace_pif::add_packages(&mut pif, dumped, &self.packages);
            // Dependency packages omit their test products. Treat the package
            // owning an explicit scheme as a root, as Xcode's integration does.
            let targets = scheme_targets(
                &self.container,
                plan.scheme,
                build_for(plan.action),
                Some(&self.locations.folder),
                test_plan_name(plan.passthrough),
            )?;
            let names = target_names(&targets);
            let missing: Vec<&String> = targets
                .iter()
                .filter(|target| scheme_pif_target(&pif, target).is_none())
                .map(|target| &target.name)
                .collect();
            let mut roots = Vec::new();
            if !missing.is_empty() {
                roots.extend(scheme_package(
                    &self.container,
                    plan.scheme,
                    &self.locations.folder,
                ));
                for package in &packages {
                    if let sweetpad_lib::workspace_pif::Package::Local(root) = package
                        && !roots.contains(root)
                        && let Ok(manifest) = super::swiftpm::manifest_at(root)
                        && manifest
                            .targets
                            .iter()
                            .any(|t| missing.iter().any(|name| ***name == t.name))
                    {
                        roots.push(root.clone());
                    }
                }
            }
            for root in roots {
                let dumped = self.dump_package_pif(&root, out)?;
                sweetpad_lib::workspace_pif::add_package_root(&mut pif, &dumped, &root);
            }
            warn_project_plugins(&pif, &names, out);
        }
        let host_tools = if self.tools.is_some() {
            host_macros::prepare(
                &mut pif,
                &self.locations.intermediates.join("SweetPad/HostTools"),
                plan.configuration,
                &target_names(&scheme_targets(
                    &self.container,
                    plan.scheme,
                    build_for(plan.action),
                    Some(&self.locations.folder),
                    test_plan_name(plan.passthrough),
                )?),
            )?
        } else {
            std::collections::HashSet::default()
        };
        let write = |path: &Path, value: &Value| -> Result<(), CliError> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| {
                    CliError::new(format!("couldn't create {}: {e}", dir.display()))
                })?;
            }
            let text = serde_json::to_string(value).map_err(|e| CliError::new(e.to_string()))?;
            write_whole(path, text.as_bytes())
        };
        let mut request = build_request(plan, &self.container, &pif, &self.parameters)?;
        host_macros::exclude_prebuilt(&mut request, &host_tools);
        write(&self.pif.with_file_name("build-request.json"), &request)?;
        write(&self.pif, &Value::Array(pif))?;
        write(&self.parameters_file, &self.parameters)?;
        if let Some(tools) = &self.tools {
            link_tools(tools, self.linker.as_deref())?;
        }
        if let (Some(path), Some(tools)) = (&self.toolset, &self.tools) {
            write_changed(path, toolset(tools, self.macros.as_deref()).as_bytes())?;
        }
        Ok(())
    }

    /// The PIF SwiftPM emits for the workspace's packages. Their root
    /// manifest goes beside the PIF with the workspace's `Package.resolved`,
    /// so the versions it pins are the ones built, and the checkouts go in
    /// `SourcePackages`, where Xcode keeps them.
    fn package_pif(
        &self,
        packages: &[sweetpad_lib::workspace_pif::Package],
        out: &Output,
    ) -> Result<Vec<Value>, CliError> {
        write_changed(
            &self.packages.join("Package.swift"),
            sweetpad_lib::workspace_pif::manifest(packages).as_bytes(),
        )?;
        if let Ok(resolved) = std::fs::read(resolved_file(&self.container)) {
            write_changed(&self.packages.join("Package.resolved"), &resolved)?;
        }
        self.dump_package_pif(&self.packages, out)
    }

    fn dump_package_pif(&self, root: &Path, out: &Output) -> Result<Vec<Value>, CliError> {
        // A package promoted to a root has a different dependency graph.
        // Reusing the app's scratch directory removes the root package from
        // its workspace-state.json and makes its remote schemes disappear.
        let scratch = if root == self.packages {
            self.locations.folder.join("SourcePackages")
        } else {
            self.locations
                .intermediates
                .join("SweetPad/PackageRoots")
                .join(sweetpad_lib::derived_data::container_hash(root))
        };
        // Xcode builds a dynamic library product as a framework in
        // `PackageFrameworks`, which an app embeds; SwiftPM's own builds
        // make a dylib unless asked otherwise.
        let stdout = swift_package(
            root,
            &scratch,
            &["--experimental-build-dylibs-as-frameworks", "dump-pif"],
            self.package_sysroot.is_some(),
            self.package_sysroot.as_deref(),
            out,
        )?;
        let mut dumped: Vec<Value> = serde_json::from_slice(&stdout).map_err(|e| {
            CliError::new(format!(
                "couldn't read the PIF SwiftPM printed for the package dependencies: {e}"
            ))
        })?;
        sweetpad_lib::workspace_pif::retarget_plugin_tools(
            &mut dumped,
            &scratch.join("out/Products"),
        );
        Ok(dumped)
    }
}

/// Run `swift package <command>` on the package at `root`, with SwiftPM's
/// scratch folder at `scratch`, behind a spinner, and return what it
/// printed. `apple` says the packages build for an Apple platform, whose
/// SDK is at `sysroot` when they are about to build.
fn swift_package(
    root: &Path,
    scratch: &Path,
    command: &[&str],
    apple: bool,
    sysroot: Option<&Path>,
    out: &Output,
) -> Result<Vec<u8>, CliError> {
    let spinner = Spinner::start_timed(
        "Resolving packages",
        out.is_interactive() && !out.is_quiet(),
        out.use_color_stderr(),
    );
    // A git prompt for credentials would wait unseen behind the spinner.
    let mut swift = Command::new("swift");
    swift
        .arg("package")
        .arg("--package-path")
        .arg(root)
        .arg("--scratch-path")
        .arg(scratch)
        .args(command)
        .env("GIT_TERMINAL_PROMPT", "0");
    if apple {
        // pkg-config runs on the host, but its headers belong to the
        // destination SDK. Keep host macro builds outside this environment.
        if let Some(sysroot) = sysroot {
            swift.env("PKG_CONFIG_SYSROOT_DIR", sysroot);
        }
        // Package manifests execute on the host. Dependencies that need
        // destination-specific resources can opt in without changing
        // their Linux host-tool builds.
        swift.env("SWEETPAD_APPLE_TARGET", "1");
    }
    let output = swift
        .stdin(Stdio::null())
        .output()
        .map_err(|e| process::spawn_error("swift", &e));
    drop(spinner);
    let output = output?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let errors = swiftpm_errors(&stderr);
        let reason = if errors.is_empty() {
            format!(
                "'swift package {}' exited with {}",
                command.last().unwrap_or(&""),
                output.status
            )
        } else {
            errors.join("\n")
        };
        return Err(CliError::new(format!(
            "couldn't resolve the package dependencies: {reason}"
        )));
    }
    Ok(output.stdout)
}

/// Resolve the Swift packages `container`'s projects use where there is no
/// xcodebuild, as a build does: SwiftPM resolves the root manifest naming
/// them, pinned by the workspace's `Package.resolved`, with the checkouts in
/// DerivedData's `SourcePackages`, or in `clone_dir`. Pins that changed go
/// back to the workspace's file; one whose pins stay is left as it is.
pub fn resolve_packages(
    container: &Path,
    clone_dir: Option<&Path>,
    out: &Output,
) -> Result<(), CliError> {
    let packages = sweetpad_lib::workspace_pif::packages(container).map_err(|e| {
        CliError::new(format!(
            "couldn't read the packages {} uses: {e}",
            container.display()
        ))
    })?;
    if packages.is_empty() {
        return Ok(());
    }
    let locations = derived_data::resolve(
        &ContainerKey::of(container),
        &std::env::var("HOME").unwrap_or_default(),
        None,
        true,
    );
    let (root, scratch, _lock) = if let Some(dir) = clone_dir {
        (dir.join("SweetPadPackages"), dir.to_path_buf(), None)
    } else {
        let lock = BuildLock::acquire(&locations.folder, || {
            out.note(&format!(
                "waiting for a build in {} to finish",
                locations.folder.display()
            ));
        })?;
        (
            locations.intermediates.join("SweetPad/SweetPadPackages"),
            locations.folder.join("SourcePackages"),
            Some(lock),
        )
    };
    write_changed(
        &root.join("Package.swift"),
        sweetpad_lib::workspace_pif::manifest(&packages).as_bytes(),
    )?;
    let pins = resolved_file(container);
    let pinned = std::fs::read(&pins).ok();
    match &pinned {
        Some(text) => write_changed(&root.join("Package.resolved"), text)?,
        None => match std::fs::remove_file(root.join("Package.resolved")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(CliError::new(format!(
                    "couldn't reset the package pins: {e}"
                )));
            }
        },
    }
    let apple = DarwinSdk::for_host()?.is_some();
    swift_package(&root, &scratch, &["resolve"], apple, None, out)?;
    let Ok(resolved) = std::fs::read(root.join("Package.resolved")) else {
        return Ok(());
    };
    if pinned
        .as_deref()
        .is_some_and(|old| same_pins(old, &resolved))
    {
        return Ok(());
    }
    if let Some(dir) = pins.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| CliError::new(format!("couldn't create {}: {e}", dir.display())))?;
    }
    write_whole(&pins, &carry_origin(pinned.as_deref(), &resolved))
}

/// SwiftPM's pins, `resolved`, in the format of the workspace's file,
/// `old`. Xcode writes format 3 with an `originHash` it works out from the
/// project's package requirements, which SwiftPM can't reproduce; the root
/// manifest's tools version makes SwiftPM write format 2. The hash carries
/// over: it still holds after an update, and Xcode replaces it after a
/// requirement changes.
fn carry_origin(old: Option<&[u8]>, resolved: &[u8]) -> Vec<u8> {
    let hash = old
        .and_then(|old| serde_json::from_slice::<Value>(old).ok())
        .filter(|old| old["version"] == 3)
        .and_then(|old| old["originHash"].as_str().map(str::to_string));
    let (Some(hash), Ok(text)) = (hash, std::str::from_utf8(resolved)) else {
        return resolved.to_vec();
    };
    let pins = text
        .strip_prefix("{\n")
        .and_then(|rest| rest.strip_suffix("\n  \"version\" : 2\n}\n"));
    let Some(pins) = pins else {
        return resolved.to_vec();
    };
    format!("{{\n  \"originHash\" : \"{hash}\",\n{pins}\n  \"version\" : 3\n}}\n").into_bytes()
}

/// Whether two `Package.resolved` files pin the same versions, whatever
/// their `originHash`, which Xcode and SwiftPM work out from different
/// inputs.
fn same_pins(a: &[u8], b: &[u8]) -> bool {
    let pins = |text: &[u8]| {
        let value: Value = serde_json::from_slice(text).ok()?;
        let pins = value
            .get("pins")
            .or_else(|| value.pointer("/object/pins"))?;
        Some(pins.clone())
    };
    pins(a).is_some_and(|a| Some(a) == pins(b))
}

/// Swift Build runs a package target's build-tool plugins, which SwiftPM
/// evaluates, but not a project target's, which Xcode evaluates itself.
fn warn_project_plugins(pif: &[Value], names: &[String], out: &Output) {
    for (target, plugin) in sweetpad_lib::workspace_pif::project_target_plugins(pif, names) {
        out.warn(&format!(
            "'{target}' runs the build-tool plugin '{plugin}', which Swift Build runs only \
             for package targets; this build skips the plugin's commands and the files they \
             generate"
        ));
    }
}

/// Take the settings the engine adds to rather than sets, `$(inherited)
/// <addition>`, such as tvOS's `-fuse-ld=lld`, out of `overrides`. They go
/// in the build's top settings level, above the user's command-line
/// settings and `-xcconfig` file (which Swift Build places above command-line
/// settings), so their `$(inherited)` holds whatever the user set:
/// `OTHER_LDFLAGS=-ObjC` builds with `-ObjC -fuse-ld=lld`.
fn take_additions(overrides: &mut BTreeMap<String, String>) -> BTreeMap<String, String> {
    let names: Vec<String> = overrides
        .iter()
        .filter(|(_, value)| value.starts_with("$(inherited) "))
        .map(|(name, _)| name.clone())
        .collect();
    names
        .into_iter()
        .filter_map(|name| overrides.remove_entry(&name))
        .collect()
}

/// A package's host macro uses the Linux SDK. It must not inherit the app's
/// Apple architecture, cross linker, or Swift compatibility libraries.
fn scope_darwin_settings(overrides: &mut BTreeMap<String, String>, sdk: &str) {
    for name in [
        "ARCHS",
        "ONLY_ACTIVE_ARCH",
        "SWIFT_SDK_TOOLSETS",
        "SWIFT_RESOURCE_DIR",
        "CLANG_RESOURCE_DIR",
        "OTHER_LDFLAGS",
        "LIBRARY_SEARCH_PATHS",
        "SYSTEM_HEADER_SEARCH_PATHS",
        "LIBTOOL",
        "DSYMUTIL",
        "CODESIGN",
        "DONT_RUN_SWIFT_STDLIB_TOOL",
        "SWIFT_STDLIB_TOOL_TOOLCHAINS",
        "SWIFT_STDLIB_TOOL_STRIP_BITCODE",
        "STRIP_BITCODE_FROM_COPIED_FILES",
        "MODULE_VERIFIER_KIND",
        "GENERATE_INTERMEDIATE_TEXT_BASED_STUBS",
    ] {
        if let Some(value) = overrides.remove(name) {
            overrides.insert(format!("{name}[sdk={sdk}*]"), value);
        }
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
    write_whole(path, contents)
}

/// Write `contents` to `path` by renaming a finished file over it, so a
/// reader sees the old file or the new one and never one cut short.
fn write_whole(path: &Path, contents: &[u8]) -> Result<(), CliError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| CliError::new(format!("couldn't create {}: {e}", dir.display())))?;
    }
    let staging = sibling(path, "tmp");
    std::fs::write(&staging, contents)
        .and_then(|()| std::fs::rename(&staging, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&staging);
            CliError::new(format!("couldn't write {}: {e}", path.display()))
        })
}

/// A name beside `path` no other process picks: `.<name>.<pid>-<time>.<kind>`.
fn sibling(path: &Path, kind: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    path.with_file_name(format!(".{name}.{}-{nanos}.{kind}", std::process::id()))
}

/// The folder of stand-in links for this binary and the Darwin SDK in
/// `bundle`, named by what the stand-ins' outputs depend on besides their
/// inputs: `SweetPad/Tools/<fingerprint>`. Swift Build runs a stand-in by
/// its path, so a new sweetpad or a new SDK gives each task that runs one a
/// new command line, and Swift Build runs it again. Links of one fixed name
/// would leave the outputs of the previous sweetpad in place.
fn tools_dir(inputs: &Path, bundle: &Path) -> Result<PathBuf, CliError> {
    let exe = std::env::current_exe()
        .map_err(|e| CliError::new(format!("couldn't find the sweetpad binary: {e}")))?;
    Ok(inputs.join("Tools").join(tools_fingerprint(&exe, bundle)))
}

/// The files of a Darwin SDK bundle that change when it's imported from
/// another Xcode.
const SDK_IDENTITY: [&str; 3] = ["info.json", "darwin-sdk-version.txt", "version.plist"];

/// A short hash of this binary (its version, path, size and time) and of
/// the Darwin SDK in `bundle` (its path and [`SDK_IDENTITY`]).
fn tools_fingerprint(exe: &Path, bundle: &Path) -> String {
    let mut identity = format!(
        "{}\n{}\n{}\n",
        env!("SWEETPAD_VERSION"),
        exe.display(),
        bundle.display()
    )
    .into_bytes();
    if let Ok(meta) = std::fs::metadata(exe) {
        identity.extend(meta.len().to_le_bytes());
        if let Ok(time) = meta.modified()
            && let Ok(since) = time.duration_since(std::time::UNIX_EPOCH)
        {
            identity.extend(since.as_nanos().to_le_bytes());
        }
    }
    for name in SDK_IDENTITY {
        identity.extend(std::fs::read(bundle.join(name)).unwrap_or_default());
        identity.push(0);
    }
    sweetpad_lib::xcode_hash::sha256(&identity)[..8]
        .iter()
        .fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

/// How long a folder of stand-in links for another sweetpad or SDK stays,
/// in case a build with that sweetpad is still running.
const STALE_TOOLS: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Link this binary into `dir` under the names of the Xcode tools it stands
/// in for: `actool`, the asset catalog compiler (see [`super::actool`]),
/// `ibtool`, the Interface Builder compiler (see [`super::ibtool`]), `momc`,
/// the Core Data model compiler (see [`super::momc`]), `xcstringstool`, the
/// String Catalog compiler (see [`super::xcstringstool`]), and the small
/// Darwin tools like `lipo` (see [`super::darwin_tools`]), with the Darwin
/// SDK's `linker` beside them for the linker stand-in to run.
///
/// The links are made in a folder of their own that is then renamed to
/// `dir`, so a build running at the same time never finds one missing. A
/// `dir` that already holds them all stays as it is. Folders beside it,
/// for another sweetpad or SDK, go once they're a day old.
fn link_tools(dir: &Path, linker: Option<&Path>) -> Result<(), CliError> {
    let fail = |e: std::io::Error| {
        CliError::new(format!(
            "couldn't link the stand-ins for Xcode's tools into {}: {e}",
            dir.display()
        ))
    };
    let exe = std::env::current_exe().map_err(fail)?;
    let links = stand_in_links(&exe, linker);
    let intact = |dir: &Path| {
        links
            .iter()
            .all(|(name, target)| std::fs::read_link(dir.join(name)).is_ok_and(|t| t == *target))
    };
    if !intact(dir) {
        let staging = sibling(dir, "tmp");
        std::fs::create_dir_all(&staging).map_err(fail)?;
        for (name, target) in &links {
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, staging.join(name)).map_err(fail)?;
            #[cfg(not(unix))]
            std::fs::copy(target, staging.join(name)).map_err(fail)?;
        }
        if std::fs::rename(&staging, dir).is_err() {
            // Another build made `dir` first, or an earlier one left it
            // broken, which is moved aside for the new links.
            if intact(dir) {
                let _ = std::fs::remove_dir_all(&staging);
            } else {
                let broken = sibling(dir, "old");
                let _ = std::fs::rename(dir, &broken);
                std::fs::rename(&staging, dir).map_err(fail)?;
                let _ = std::fs::remove_dir_all(&broken);
            }
        }
    }
    remove_stale_tools(dir);
    Ok(())
}

/// Remove the folders beside `current` that are [`STALE_TOOLS`] old, and
/// `SweetPad/bin`, where sweetpad 0.1.25 and earlier linked the stand-ins.
fn remove_stale_tools(current: &Path) {
    let Some(parent) = current.parent() else {
        return;
    };
    let legacy = parent.parent().map(|inputs| inputs.join("bin"));
    let now = std::time::SystemTime::now();
    let siblings = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path());
    for path in siblings.chain(legacy) {
        let old = std::fs::symlink_metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|time| now.duration_since(time).ok())
            .is_some_and(|age| age > STALE_TOOLS);
        if path != current && old {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// The stand-ins' names and what each links to: this binary, `exe`, for
/// Xcode's tools, and the Darwin SDK's `linker` and its `dsymutil`.
fn stand_in_links(exe: &Path, linker: Option<&Path>) -> Vec<(&'static str, PathBuf)> {
    let dsymutil = linker
        .and_then(Path::parent)
        .map(|p| p.join("dsymutil"))
        .filter(|p| p.is_file());
    [
        "actool",
        "ibtool",
        "momc",
        "mapc",
        "xcstringstool",
        "intentbuilderc",
        "appintentsmetadataprocessor",
        "appintentsnltrainingprocessor",
        "appshortcutstringsprocessor",
        "codesign",
        "derq",
        super::codesign::provisioning::TOOL,
    ]
    .iter()
    .chain(super::darwin_tools::TOOLS)
    .map(|tool| (*tool, exe.to_path_buf()))
    .chain(linker.map(|l| (super::darwin_tools::SDK_LINKER, l.to_path_buf())))
    .chain(dsymutil.map(|p| (super::darwin_tools::SDK_DSYMUTIL, p)))
    .collect()
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
/// (`KEY=VALUE`), `-derivedDataPath`, `-xcconfig` and `-testPlan`. Other flags are
/// xcodebuild's own.
fn refuse_xcodebuild_flags(passthrough: &[String]) -> Result<(), CliError> {
    let mut args = passthrough.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-testPlan" => {
                if args
                    .next()
                    .is_none_or(|value| value.is_empty() || value.starts_with('-'))
                {
                    return Err(
                        CliError::new("'-testPlan' requires a plan name").kind(ErrorKind::Usage)
                    );
                }
            }
            "-derivedDataPath" | "-xcconfig" => {
                args.next();
            }
            // Output levels, which the build's progress takes (see `output_levels`).
            "-verbose" | "-quiet" => {}
            flag if flag.starts_with('-') => {
                return Err(CliError::new(format!(
                    "'{flag}' is an xcodebuild flag from passthrough arguments or sweetpad.toml's [xcodebuild].args; this build runs on Swift Build, which \
                     takes only build settings ('KEY=VALUE'), '-derivedDataPath', '-xcconfig', '-testPlan', \
                     '-verbose' and '-quiet' after '--'"
                ))
                .kind(ErrorKind::Usage));
            }
            _ => {}
        }
    }
    Ok(())
}

/// xcodebuild's `-verbose` and `-quiet` among the passthrough arguments, as
/// the levels of `-v` and `-q` they stand for here: the build's progress
/// shows what `-v` shows, or only what `-q` leaves.
fn output_levels(passthrough: &[String]) -> (bool, bool) {
    let has = |flag: &str| passthrough.iter().any(|a| a == flag);
    (has("-verbose"), has("-quiet"))
}

fn test_plan_name(passthrough: &[String]) -> Option<&str> {
    passthrough
        .windows(2)
        .rev()
        .find(|a| a[0] == "-testPlan")
        .map(|a| a[1].as_str())
}

/// A target a scheme builds: its name, and where the scheme says, the
/// project and pbxproj ID that tell it from a target of the same name in
/// another project of the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SchemeTarget {
    name: String,
    project: Option<(PathBuf, String)>,
}

impl SchemeTarget {
    fn named(name: &str) -> Self {
        Self {
            name: name.to_string(),
            project: None,
        }
    }

    /// Whether `other` is this target again: the same name, from the same
    /// project where both say which.
    fn same_as(&self, other: &Self) -> bool {
        self.name == other.name
            && match (&self.project, &other.project) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
}

/// The names of `targets`, for what reads a build's targets by name.
fn target_names(targets: &[SchemeTarget]) -> Vec<String> {
    targets.iter().map(|t| t.name.clone()).collect()
}

/// What a build action builds a scheme for.
fn build_for(action: BuildAction) -> BuildFor {
    match action {
        BuildAction::Build => BuildFor::Running,
        BuildAction::BuildForTesting => BuildFor::Testing,
        BuildAction::Archive => BuildFor::Archiving,
    }
}

/// The PIF target `target` is: the one its project and ID pick out, else the
/// one [`pif_target`] finds by its name.
fn scheme_pif_target<'a>(pif: &'a [Value], target: &SchemeTarget) -> Option<&'a Value> {
    target
        .project
        .as_ref()
        .and_then(|(project, id)| {
            sweetpad_lib::workspace_pif::project_target_guid(pif, project, id)
        })
        .and_then(|guid| {
            pif.iter()
                .filter(|object| object["type"] == "target")
                .map(|object| &object["contents"])
                .find(|t| t["guid"] == guid.as_str())
        })
        .or_else(|| pif_target(pif, &target.name))
}

/// The targets a scheme builds for `build_for`, by name, the way Swift Build
/// looks them up. A scheme with no file stands for the target of its name.
/// Building for testing adds what the test action runs, as `xcodebuild
/// build-for-testing` does: the default test plan's enabled targets, or
/// without plans the testables that aren't skipped. Package entries refer
/// to SwiftPM's products just as project entries refer to project targets.
fn scheme_targets(
    container: &Path,
    scheme: &str,
    build_for: BuildFor,
    derived_data: Option<&Path>,
    requested_plan: Option<&str>,
) -> Result<Vec<SchemeTarget>, CliError> {
    let explicit_plan = requested_plan.is_some() && build_for == BuildFor::Testing;
    let missing_plan = || {
        CliError::new(format!(
            "scheme '{scheme}' has no test plan named '{}'",
            requested_plan.unwrap_or_default()
        ))
    };
    let Some(file) =
        sweetpad_lib::scheme::locate_with_derived_data(container, scheme, derived_data)
    else {
        if explicit_plan {
            return Err(missing_plan());
        }
        return Ok(vec![SchemeTarget::named(scheme)]);
    };
    let parsed = match sweetpad_lib::scheme::parse_file(&file) {
        Ok(parsed) => parsed,
        Err(error) if explicit_plan => return Err(CliError::new(error.to_string())),
        Err(_) => return Ok(vec![SchemeTarget::named(scheme)]),
    };
    if explicit_plan && parsed.test_action.is_none() {
        return Err(missing_plan());
    }
    let mut targets: Vec<SchemeTarget> = Vec::new();
    let mut add = |name: &str, project: Option<PathBuf>, identifier: &str| {
        let target = SchemeTarget {
            name: name.to_string(),
            project: project
                .filter(|_| !identifier.is_empty())
                .map(|p| (p, identifier.to_string())),
        };
        if !targets.iter().any(|t| t.same_as(&target)) {
            targets.push(target);
        }
    };
    let reference = |buildable: &sweetpad_lib::scheme::BuildableRef| {
        (
            buildable.blueprint_name.clone(),
            buildable.project_path(&file),
            buildable.blueprint_identifier.clone(),
        )
    };
    for entry in parsed
        .build_entries
        .iter()
        .filter(|e| e.builds_for(build_for))
    {
        let (name, project, identifier) = reference(&entry.buildable);
        add(&name, project, &identifier);
    }
    if build_for == BuildFor::Testing
        && let Some(test) = &parsed.test_action
    {
        let plan = sweetpad_lib::scheme::select_test_plan(&test.test_plans, requested_plan)
            .map_err(CliError::new)?;
        if let Some(plan) = plan {
            // `container:` paths are relative to the folder the scheme's
            // project or workspace is in.
            let package = file
                .ancestors()
                .find(|p| p.file_name().is_some_and(|n| n == ".swiftpm"))
                .and_then(Path::parent);
            let base = package
                .or_else(|| {
                    file.ancestors()
                        .find(|a| {
                            a.extension()
                                .is_some_and(|e| e == "xcodeproj" || e == "xcworkspace")
                        })
                        .and_then(Path::parent)
                })
                .unwrap_or(Path::new("."));
            let path = base.join(&plan.path);
            let plan_targets = sweetpad_lib::scheme::test_plan_targets(&path).ok_or_else(|| {
                CliError::new(format!(
                    "couldn't read test plan {} of scheme '{scheme}'",
                    path.display()
                ))
            })?;
            for target in plan_targets.iter().filter(|t| t.enabled) {
                let project = Path::new(&target.container)
                    .extension()
                    .is_some_and(|e| e == "xcodeproj")
                    .then(|| sweetpad_lib::project::absolutize(&base.join(&target.container)));
                add(&target.name, project, &target.identifier);
            }
        } else {
            for testable in test.testables.iter().filter(|t| !t.skipped) {
                let (name, project, identifier) = reference(&testable.buildable);
                add(&name, project, &identifier);
            }
        }
    }
    if targets.is_empty() {
        return Err(CliError::new(format!(
            "scheme '{scheme}' builds no targets for this action"
        ))
        .kind(ErrorKind::TargetResolution));
    }
    Ok(targets)
}

/// The package whose `.swiftpm/xcode` directory owns an explicit scheme.
fn scheme_package(container: &Path, scheme: &str, derived_data: &Path) -> Option<PathBuf> {
    sweetpad_lib::scheme::locate_with_derived_data(container, scheme, Some(derived_data))?
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == ".swiftpm"))?
        .parent()
        .map(Path::to_path_buf)
}

/// The console build command disables implicit dependencies by default.
/// Supply a request so workspace-linked products follow the scheme's policy,
/// as they do in Xcode, instead of silently building only explicit targets.
fn build_request(
    plan: &BuildPlan<'_>,
    container: &Path,
    pif: &[Value],
    parameters: &Value,
) -> Result<Value, CliError> {
    let build_for = build_for(plan.action);
    let derived_data = parameters["arenaInfo"]["derivedDataPath"]
        .as_str()
        .map(Path::new);
    let mut targets = Vec::new();
    for scheme_target in scheme_targets(
        container,
        plan.scheme,
        build_for,
        derived_data,
        test_plan_name(plan.passthrough),
    )? {
        let target = scheme_pif_target(pif, &scheme_target).ok_or_else(|| {
            CliError::new(format!(
                "scheme target '{}' isn't in the workspace PIF",
                scheme_target.name
            ))
        })?;
        targets.push(json!({"guid":target["guid"]}));
    }
    let scheme =
        sweetpad_lib::scheme::locate_with_derived_data(container, plan.scheme, derived_data)
            .and_then(|path| sweetpad_lib::scheme::parse_file(&path).ok());
    Ok(json!({
        "parameters":parameters,
        "configuredTargets":targets,
        "useParallelTargets":scheme.as_ref().is_none_or(|s|s.parallelize_buildables),
        "useImplicitDependencies":scheme.as_ref().is_none_or(|s|s.build_implicit_dependencies),
        "dependencyScope":"workspace", "useDryRun":false,
        "continueBuildingAfterErrors":false, "hideShellScriptEnvironment":true,
        "showNonLoggedProgress":true, "enableIndexBuildArena":false,
        "useLegacyBuildLocations":false,
        "buildCommand":{"command":"build","style":"buildOnly","skipDependencies":false},
        "schemeCommand":if plan.action == BuildAction::Archive { "archive" } else { "launch" }, "containerPath":container,
    }))
}

/// The PIF target a scheme builds as `name`: the project's target of that
/// name, else a local package's product. SwiftPM names a product's target
/// `<name>-product` and a module's `<name>`, and the scheme Xcode makes for
/// a product builds the product, which may share no name with a module, as
/// one made only of binary targets doesn't. A package's module of the name
/// comes last.
fn pif_target<'a>(pif: &'a [Value], name: &str) -> Option<&'a Value> {
    let targets = || {
        pif.iter()
            .filter(|object| object["type"] == "target")
            .map(|object| &object["contents"])
    };
    let from_package = |target: &Value| {
        target["guid"]
            .as_str()
            .is_some_and(|g| g.starts_with("PACKAGE-"))
    };
    let product = format!("{name}-product");
    targets()
        .find(|t| t["name"] == name && !from_package(t))
        .or_else(|| targets().find(|t| t["name"] == product && from_package(t)))
        .or_else(|| targets().find(|t| t["name"] == name))
}

/// Swift Build's build parameters: the configuration, the run destination,
/// where the build writes (the folders `xcodebuild` uses, so the product
/// locator finds the app), and the settings that override the project's.
fn parameters(
    configuration: &str,
    sdk: &str,
    arch: &str,
    locations: &Locations,
    (overrides, additions): (&BTreeMap<String, String>, &BTreeMap<String, String>),
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
    if !additions.is_empty() {
        overrides_json["environmentConfig"] = json!({ "table": additions });
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

/// Where the Darwin Swift SDK bundle is: `SWEETPAD_DARWIN_SDK`, else the one
/// `xtool sdk install` writes.
pub(super) fn darwin_sdk_bundle() -> PathBuf {
    sweetpad_lib::xcode::darwin_sdk_bundle()
}

/// A Darwin Swift SDK bundle: the Apple SDKs, Xcode's runtime libraries and a
/// linker that writes Mach-O, which stand in for Xcode where there is none.
struct DarwinSdk {
    bundle: PathBuf,
}

impl DarwinSdk {
    fn sdk_root(&self, sdk: &str) -> Result<PathBuf, CliError> {
        let explicit = Path::new(sdk);
        if explicit.is_dir() {
            return Ok(explicit.to_path_buf());
        }
        let platforms = self.bundle.join("Developer/Platforms");
        for platform in std::fs::read_dir(platforms).into_iter().flatten().flatten() {
            let sdks = platform.path().join("Developer/SDKs");
            for entry in std::fs::read_dir(sdks).into_iter().flatten().flatten() {
                if entry
                    .path()
                    .file_stem()
                    .is_some_and(|name| name.eq_ignore_ascii_case(sdk))
                {
                    return Ok(entry.path());
                }
            }
        }
        Err(CliError::new(format!(
            "no SDK '{sdk}' in {}",
            self.bundle.display()
        )))
    }

    /// The bundle a build here needs: none on macOS, where Swift Build finds
    /// Xcode itself, and elsewhere the one at [`darwin_sdk_bundle`].
    fn for_host() -> Result<Option<Self>, CliError> {
        if cfg!(target_os = "macos") {
            return Ok(None);
        }
        let bundle = darwin_sdk_bundle();
        if !bundle.join("toolset-swb.json").is_file() {
            return Err(CliError::new(format!(
                "no Darwin Swift SDK at {}, which a build for Apple platforms needs without Xcode",
                bundle.display()
            ))
            .kind(ErrorKind::ToolMissing)
            .tip(Some(
                "export SDK files on your Mac with 'sweetpad sdk export', then run \
                 'sweetpad setup --sdk ARCHIVE' on Ubuntu, or point SWEETPAD_DARWIN_SDK at an installed SDK"
                    .to_string(),
            )));
        }
        Ok(Some(Self { bundle }))
    }

    /// The settings a build needs without Xcode's own tools.
    ///
    /// - `CODE_SIGNING_ALLOWED=NO`, unless the build is signed (see
    ///   [`Self::signs`]). A signed build sets `CODESIGN=codesign`, the
    ///   stand-in, over the bundle's `true`, which leaves signing to xtool.
    /// - `SWIFT_SDK_TOOLSETS`: the bundle's linker, which links for Apple
    ///   platforms where the toolchain's own refuses to, and `toolset`, with
    ///   what a build here needs on top (see [`toolset`]).
    /// - `LM_SKIP_METADATA_EXTRACTION`: enable the portable App Intents
    ///   registration and language-model processors.
    /// - `SWIFT_STDLIB_TOOL_STRIP_BITCODE` and
    ///   `STRIP_BITCODE_FROM_COPIED_FILES`: current Apple builds have no
    ///   embedded bitcode. Keep portable runtime and framework copying
    ///   without invoking the macOS-only bitcode tool.
    /// - `LIBRARY_SEARCH_PATHS`: the Swift compatibility libraries an older
    ///   deployment target links (`swiftCompatibility56`) are Xcode's, in the
    ///   bundle, and the toolchain here has none.
    /// - `GENERATE_INTERMEDIATE_TEXT_BASED_STUBS`: a framework's stub comes
    ///   from `tapi`, which is Xcode's; it only sharpens incremental relinks.
    /// - The Xcode version (see [`xcode_version_settings`]) and, for a
    ///   platform without its `version.plist`, its build as the SDK's, which
    ///   go into the app's Info.plist as `DTXcode`, `DTXcodeBuild` and
    ///   `DTPlatformBuild`, and the macOS build the SDK came from (see
    ///   [`build_machine_settings`]).
    fn overrides(&self, sdk: &str, arch: &str, toolset: &Path) -> Vec<(String, String)> {
        let swift_libraries = self
            .bundle
            .join("Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/$(PLATFORM_NAME)");
        let mut versions = xcode_version_settings(&sweetpad_lib::derived_data::plist_strings(
            &self.bundle.join("version.plist"),
        ));
        versions.extend(build_machine_settings(
            &sweetpad_lib::derived_data::plist_strings(&self.bundle.join("build-machine.plist")),
        ));
        if !self.platform_has_version(sdk) {
            versions.push((
                "PLATFORM_PRODUCT_BUILD_VERSION",
                "$(SDK_PRODUCT_BUILD_VERSION)".to_string(),
            ));
        }
        let mut settings: Vec<(String, String)> = [
            (
                "LIBRARY_SEARCH_PATHS",
                format!("$(inherited) {}", swift_libraries.display()),
            ),
            (
                "SYSTEM_HEADER_SEARCH_PATHS",
                format!(
                    "$(inherited) {}",
                    self.bundle
                        .join("Developer/Toolchains/XcodeDefault.xctoolchain/usr/include")
                        .display()
                ),
            ),
            ("ARCHS", arch.to_string()),
            ("LIBTOOL", "llvm-libtool-darwin".to_string()),
            ("DSYMUTIL", "dsymutil".to_string()),
            ("ONLY_ACTIVE_ARCH", "YES".to_string()),
            (
                "SWIFT_SDK_TOOLSETS",
                format!(
                    "{} {}",
                    self.bundle.join("toolset-swb.json").display(),
                    toolset.display()
                ),
            ),
            ("LM_SKIP_METADATA_EXTRACTION", "NO".to_string()),
            ("GENERATE_INTERMEDIATE_TEXT_BASED_STUBS", "NO".to_string()),
            ("SWIFT_STDLIB_TOOL_STRIP_BITCODE", "NO".to_string()),
            ("STRIP_BITCODE_FROM_COPIED_FILES", "NO".to_string()),
            (
                "SWIFT_STDLIB_TOOL_TOOLCHAINS",
                self.bundle
                    .join("Developer/Toolchains/XcodeDefault.xctoolchain")
                    .display()
                    .to_string(),
            ),
            ("MODULE_VERIFIER_KIND", "builtin".to_string()),
            // Apple sandbox profiles require sandbox-exec, which Linux cannot run.
            ("DISABLE_TASK_SANDBOXING", "YES".to_string()),
        ]
        .into_iter()
        .chain(versions)
        .chain(std::iter::once(if Self::signs(sdk) {
            ("CODESIGN", "codesign".to_string())
        } else {
            ("CODE_SIGNING_ALLOWED", "NO".to_string())
        }))
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        if [
            "appletvos",
            "appletvsimulator",
            "watchos",
            "watchsimulator",
            "xros",
            "xrsimulator",
        ]
        .contains(&sdk.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.'))
        {
            let resources = self
                .bundle
                .join("Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift");
            settings.extend([
                ("SWIFT_RESOURCE_DIR".into(), resources.display().to_string()),
                (
                    "CLANG_RESOURCE_DIR".into(),
                    resources.join("clang").display().to_string(),
                ),
                ("OTHER_LDFLAGS".into(), "$(inherited) -fuse-ld=lld".into()),
            ]);
        }
        settings
    }

    /// Companion apps can build for another Apple SDK in the same request.
    /// Scope their cross tools too, while leaving Linux host macros untouched.
    fn embedded_platform_overrides(
        &self,
        sdk: &str,
        arch: &str,
        toolset: &Path,
    ) -> BTreeMap<String, String> {
        let primary = sdk.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
        let mut result = BTreeMap::new();
        for platform in [
            "iphoneos",
            "iphonesimulator",
            "macosx",
            "appletvos",
            "appletvsimulator",
            "watchos",
            "watchsimulator",
            "xros",
            "xrsimulator",
        ] {
            if platform == primary {
                continue;
            }
            let mut settings = self
                .overrides(platform, arch, toolset)
                .into_iter()
                .collect();
            scope_darwin_settings(&mut settings, platform);
            result.extend(
                settings
                    .into_iter()
                    .filter(|(key, _)| key.contains("[sdk=")),
            );
        }
        result
    }

    /// Whether a build for `sdk` is signed, by the `codesign` and
    /// provisioning stand-ins as Xcode would (see [`super::codesign`]): one
    /// for a device or a Mac, with an identity in `SWEETPAD_CODE_SIGN_P12` to
    /// sign with, and one for a simulator, which Xcode signs ad hoc with its
    /// entitlements in the executable, where App Groups and keychain groups
    /// come from.
    fn signs(sdk: &str) -> bool {
        sdk.contains("simulator")
            || (Self::is_signed(sdk) && super::codesign::identity_configured())
    }

    /// Whether `sdk` is a device's or a Mac's, whose builds a signature
    /// lets run and whose archives an export signs again.
    fn is_signed(sdk: &str) -> bool {
        let platform = sdk.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
        SIGNED_PLATFORMS.contains(&platform)
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
    /// then the bundle's. A signed build has Swift Build ask the
    /// provisioning stand-in for what Xcode would provision, which signs an
    /// archive's target ad hoc on `fallback` when the identities can't, and
    /// checks profiles against `device`, the device the build runs on.
    fn env(
        &self,
        tools: &Path,
        signs: bool,
        fallback: bool,
        device: Option<String>,
    ) -> Vec<(String, Option<String>)> {
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
            (
                "SWEETPAD_PROVISIONING".to_string(),
                signs.then(|| {
                    tools
                        .join(super::codesign::provisioning::TOOL)
                        .display()
                        .to_string()
                }),
            ),
            (
                super::codesign::provisioning::FALLBACK_VARIABLE.to_string(),
                fallback.then(|| "ad-hoc".to_string()),
            ),
            (
                super::codesign::provisioning::DEVICE_VARIABLE.to_string(),
                device,
            ),
        ]
    }
}

/// The device a build for an iOS-family device runs on, as its destination
/// names it, whose identifier a development or ad hoc profile must list.
fn run_device(destination: Option<&str>, sdk: &str) -> Option<String> {
    let spec = sweetpad_lib::destination::DestinationSpec::parse(destination?);
    let device_sdk = ["iphoneos", "appletvos", "watchos", "xros"]
        .iter()
        .any(|platform| sdk.starts_with(platform));
    spec.id.filter(|_| device_sdk && !spec.generic)
}

/// Whether a build is for every architecture the project names, as
/// xcodebuild builds a generic destination: a simulator or Mac app gets
/// arm64 and x86_64, a watch app arm64_32 beside arm64. `arch=` keeps a
/// generic destination to that one architecture; xcodebuild refuses `arch=`
/// on a generic destination, so nothing it builds is at stake.
fn builds_every_arch(destination: Option<&str>) -> bool {
    destination
        .map(sweetpad_lib::destination::DestinationSpec::parse)
        .is_some_and(|spec| spec.generic && spec.arch.is_none())
}

/// A build for every architecture drops the host's one-architecture
/// overrides, so `ARCHS_STANDARD` applies unless the project says otherwise.
/// Archives build this way, and so does a generic destination without
/// `arch=`; other builds stay on the destination's one architecture.
fn standard_architectures(overrides: &mut BTreeMap<String, String>) {
    overrides.retain(|key, _| {
        let name = key.split('[').next().unwrap_or(key);
        name != "ARCHS" && name != "ONLY_ACTIVE_ARCH"
    });
    overrides.insert("ONLY_ACTIVE_ARCH".to_string(), "NO".to_string());
}

/// The platforms whose builds are signed: devices and the Mac.
const SIGNED_PLATFORMS: [&str; 5] = ["iphoneos", "appletvos", "watchos", "xros", "macosx"];

/// A device or Mac archive is always signed: ad hoc, with its
/// entitlements, when the identities can't sign it, for its export to sign
/// again. The stand-in signs the products of every platform the archive
/// embeds, like a watch app in an iOS archive. A device SDK refuses an ad
/// hoc signature unless allowed.
fn sign_archive(overrides: &mut BTreeMap<String, String>) {
    overrides.remove("CODE_SIGNING_ALLOWED");
    overrides.insert("CODESIGN".to_string(), "codesign".to_string());
    for platform in SIGNED_PLATFORMS {
        overrides.insert(format!("CODESIGN[sdk={platform}*]"), "codesign".to_string());
    }
    overrides.insert("AD_HOC_CODE_SIGNING_ALLOWED".to_string(), "YES".to_string());
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

/// `MAC_OS_X_PRODUCT_BUILD_VERSION` from the SDK's `build-machine.plist`:
/// the macOS build it came from, which Swift Build writes into an app's
/// Info.plist as `BuildMachineOSBuild` and leaves out when it's empty. An
/// SDK imported before the file existed has none.
fn build_machine_settings(plist: &[(String, String)]) -> Vec<(&'static str, String)> {
    plist
        .iter()
        .find(|(k, v)| k == "BuildMachineOSBuild" && !v.is_empty())
        .map(|(_, build)| ("MAC_OS_X_PRODUCT_BUILD_VERSION", build.clone()))
        .into_iter()
        .collect()
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
///
/// What xcodebuild's raw log shows and the parsed view drops comes through as
/// [`Event::Other`], which only `-v` renders: each task's command line, and
/// the output of a task that reported nothing of its own. A task whose tool
/// can't be started blocks the build rather than breaking it
/// ([`blocker`](Self::blocker)).
#[derive(Debug, Default)]
pub struct MessageParser {
    /// What each started task is, by task ID.
    tasks: HashMap<i64, Task>,
    /// Each task's output so far, by task ID.
    output: HashMap<i64, String>,
    /// Tasks that reported an error of their own, which explains their
    /// failure.
    diagnosed: HashSet<i64>,
    /// Tasks that reported a diagnostic of any kind, whose output says the
    /// same.
    reported: HashSet<i64>,
    /// Tasks whose tool couldn't be started.
    unspawned: HashSet<i64>,
    /// The way past the first tool that couldn't be started.
    blocker: Option<String>,
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

    /// How past a build blocked by a tool that couldn't be started, or `None`.
    #[must_use]
    pub fn blocker(&self) -> Option<String> {
        self.blocker.clone()
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
                let mut parsed = vec![Parsed {
                    raw: description,
                    event,
                }];
                parsed.extend(command_line(text("commandLineDisplayString")));
                parsed
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
                    let mut parsed = if task.is_some_and(|t| t.link) {
                        linker_warnings(&output)
                    } else {
                        Vec::new()
                    };
                    if parsed.is_empty() && !id.is_some_and(|id| self.reported.contains(&id)) {
                        parsed.extend(task_output(&output));
                    }
                    return parsed;
                }
                if id.is_some_and(|id| self.diagnosed.contains(&id))
                    || (self.errors && task.is_some_and(|t| t.swift))
                {
                    return Vec::new();
                }
                let description = task.map_or("A build task", |t| t.description.as_str());
                failed_task(description, &output)
            }
            kind @ ("error" | "warning" | "note" | "remark") => self.diagnostic_message(kind, m),
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

    /// The events a diagnostic message (`error`, `warning`, `note`,
    /// `remark`) makes, noting which task it explains.
    fn diagnostic_message(&mut self, kind: &str, m: &Value) -> Vec<Parsed> {
        let task = m.pointer("/locationContext/taskID").and_then(Value::as_i64);
        if let Some(id) = task {
            self.reported.insert(id);
        }
        let message = m.get("message").and_then(Value::as_str).unwrap_or_default();
        if kind == "error"
            && let Some(tool) = unspawned_tool(message)
        {
            if let Some(id) = task {
                self.unspawned.insert(id);
            }
            let description = task
                .and_then(|id| self.tasks.get(&id))
                .map(|t| t.description.as_str());
            self.blocker
                .get_or_insert_with(|| spawn_blocker(tool, description));
        }
        // A tool that never ran wrote no serialized diagnostics, which
        // Swift Build then reports it couldn't read.
        if message.starts_with("Could not read serialized diagnostics file")
            && task.is_some_and(|id| self.unspawned.contains(&id))
        {
            return Vec::new();
        }
        if kind == "error" {
            self.errors = true;
            // `Linker command failed …` points at the project or the
            // package and says nothing of why; any other error a task
            // reports, in a file or not, explains its failure.
            let at_project = m
                .pointer("/location/path")
                .and_then(Value::as_str)
                .is_some_and(is_container);
            if !at_project && let Some(id) = task {
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

/// A task's command line as `-v` shows it, under the task, as xcodebuild's
/// log does. A task that runs in Swift Build's own process shows only its
/// `cd` line, which says nothing, so it's left out.
fn command_line(display: &str) -> Option<Parsed> {
    let display = display.trim_end();
    let mut lines = display.lines().filter(|l| !l.trim().is_empty());
    lines.next()?;
    lines.next()?;
    Some(Parsed {
        raw: display.to_string(),
        event: Event::Other(display.to_string()),
    })
}

/// What a task that succeeded printed, for `-v`, without lld's notice about
/// the platform.
fn task_output(output: &str) -> Option<Parsed> {
    let lines: Vec<&str> = output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| {
            l.split_once(": warning: ")
                .is_none_or(|(_, message)| !lld_platform_notice(message))
        })
        .collect();
    (!lines.is_empty()).then(|| {
        let text = lines.join("\n");
        Parsed {
            raw: text.clone(),
            event: Event::Other(text),
        }
    })
}

/// The tool in Swift Build's `unable to spawn process 'metal' (No such file
/// or directory)`: one that isn't on this machine. A tool that is there but
/// won't start isn't this.
fn unspawned_tool(message: &str) -> Option<&str> {
    let rest = message.strip_prefix("unable to spawn process '")?;
    let (tool, reason) = rest.split_once("' (")?;
    reason
        .starts_with("No such file or directory")
        .then_some(tool)
}

/// What a build tool with no Linux version is, by the name Swift Build starts
/// it under.
fn macos_only_tool(tool: &str) -> Option<&'static str> {
    let name = tool.rsplit('/').next().unwrap_or(tool);
    Some(match name {
        "metal" | "metallib" | "metal-ar" => "Apple's Metal shader compiler",
        "osacompile" => "the AppleScript compiler",
        "Rez" | "ResMerger" => "the Carbon resource compiler",
        "mig" => "the Mach interface generator",
        "iig" => "the DriverKit interface generator",
        "dtrace" => "DTrace",
        "realitytool" => "the RealityKit content compiler",
        "coremlc" => "the Core ML model compiler",
        "scntool" | "copySceneKitAssets" | "compileSceneKitShaders" => "SceneKit's asset compiler",
        "TextureAtlas" => "SpriteKit's texture atlas compiler",
        _ => return None,
    })
}

/// The way past a build whose `task` needs `tool`, which isn't here.
fn spawn_blocker(tool: &str, task: Option<&str>) -> String {
    let task = task.map_or_else(|| "a build task".to_string(), |t| format!("'{t}'"));
    match macos_only_tool(tool) {
        Some(what) if !cfg!(target_os = "macos") => format!(
            "{task} needs '{tool}', {what}, which has no Linux version; build this target on a Mac"
        ),
        _ => format!(
            "{task} needs '{tool}', which Swift Build couldn't find; install it on the search path"
        ),
    }
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
    if lines.iter().any(|line| line.trim().starts_with("error: ")) {
        return lines
            .into_iter()
            .map(|line| Parsed {
                raw: line.to_string(),
                event: match line.trim().strip_prefix("error: ") {
                    Some(message) => Event::Diagnostic {
                        kind: DiagKind::Error,
                        location: None,
                        message: message.to_string(),
                    },
                    None => Event::Other(line.to_string()),
                },
            })
            .collect();
    }
    let message = if lines.is_empty() {
        format!("{description} failed")
    } else {
        format!("{description} failed:\n{}", lines.join("\n"))
    };
    vec![Parsed {
        raw: format!("error: {message}"),
        event: Event::Diagnostic {
            kind: DiagKind::Error,
            location: None,
            message,
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Mac's file system ignores case, so a project can name `Core.swift`
    /// and hold `core.swift`; Linux then can't find the file. The Mac finds
    /// it, so the test runs where names are case-sensitive.
    #[test]
    #[cfg(not(target_os = "macos"))]
    fn a_missing_input_found_under_another_case_is_named() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-swbuild-case");
        std::fs::create_dir_all(dir.join("Sources/Kit")).unwrap();
        std::fs::write(dir.join("Sources/Kit/core.swift"), "").unwrap();
        let wanted = dir.join("sources/Kit/Core.swift");
        assert_eq!(
            other_case(&wanted),
            Some(dir.join("Sources/Kit/core.swift"))
        );
        assert_eq!(other_case(&dir.join("Sources/Kit/core.swift")), None);
        assert_eq!(other_case(&dir.join("Sources/Kit/Other.swift")), None);
        let diagnostics = [json!({"message": format!(
            "Build input file cannot be found: '{}'. Did you forget to declare this file as an output?",
            wanted.display()
        )})];
        let hint = case_mismatches(&diagnostics).unwrap();
        assert!(
            hint.ends_with("core.swift' on disk, and Linux file names are case-sensitive"),
            "{hint}"
        );
    }

    #[test]
    fn failed_shell_output_is_in_json_diagnostics() {
        let parsed = failed_task("Run script", "/bin/sh: set: Illegal option -o pipefail\n");
        assert!(
            matches!(&parsed[0].event, Event::Diagnostic { kind: DiagKind::Error, message, .. }
            if message.contains("Illegal option -o pipefail"))
        );
    }

    #[test]
    fn failed_tools_preserve_their_own_unlocated_errors_in_json_diagnostics() {
        let parsed = failed_task(
            "Extract app intents metadata",
            "error: appintentsmetadataprocessor: unsupported parameter option\n",
        );
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0].event,
            Event::Diagnostic {
                kind: DiagKind::Error,
                location: None,
                message: "appintentsmetadataprocessor: unsupported parameter option".into()
            }
        );
    }

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
            [Event::Diagnostic {
                kind: DiagKind::Error,
                location: None,
                message: "Run custom shell script 'Lint' failed:\nlint: 2 problems".into(),
            },]
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
            found.iter().any(|e| matches!(e, Event::Diagnostic { message, .. } if message.contains("ld64.lld: error: undefined symbol: _foo"))),
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
            found.iter().any(|e| matches!(e, Event::Diagnostic { message, .. } if message.contains("/s/Kit/Package.swift: Kit-product: ld64.lld: error: undefined symbol: _foo"))),
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
        // The output is what `-v` shows, not a diagnostic.
        assert_eq!(
            found,
            vec![
                Event::Compile {
                    name: "A.swift".into()
                },
                Event::Other("/p/A.swift:1:5: warning: unused".into()),
            ]
        );
    }

    #[test]
    fn verbose_shows_command_lines_and_the_output_no_diagnostic_carried() {
        let found = events(&[
            r#"{"kind":"taskStarted","id":3,"ruleInfo":"PhaseScriptExecution Gen /dd/Script.sh","executionDescription":"Run custom shell script 'Gen'","commandLineDisplayString":"    cd /p\n    /bin/sh -c /dd/Script.sh\n"}"#,
            r#"{"kind":"taskStarted","id":4,"ruleInfo":"SwiftDriverJobDiscovery normal arm64","executionDescription":"Compile","commandLineDisplayString":"    cd /p\n    \n"}"#,
            r#"{"kind":"taskOutput","taskID":3,"data":"generated 2 files\n"}"#,
            r#"{"kind":"taskComplete","id":3,"result":"success"}"#,
            r#"{"kind":"taskStarted","id":5,"ruleInfo":"SwiftCompile normal arm64 /p/A.swift","executionDescription":"Compile A.swift (arm64)"}"#,
            r#"{"kind":"warning","message":"unused","location":{"locationType":"path","path":"/p/A.swift","line":1,"column":5},"locationContext":{"locationType":"task","taskID":5}}"#,
            r#"{"kind":"taskOutput","taskID":5,"data":"/p/A.swift:1:5: warning: unused\n"}"#,
            r#"{"kind":"taskComplete","id":5,"result":"success"}"#,
        ]);
        let others: Vec<&str> = found
            .iter()
            .filter_map(|e| match e {
                Event::Other(raw) => Some(raw.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            others,
            [
                "Run custom shell script 'Gen'",
                "    cd /p\n    /bin/sh -c /dd/Script.sh",
                "Compile",
                "generated 2 files",
            ]
        );
    }

    /// Measured on Linux with a `.metal` file: Swift Build reports the spawn
    /// failure against the task, then that it couldn't read the diagnostics
    /// the compiler never wrote.
    #[test]
    fn a_tool_with_no_linux_version_blocks_the_build() {
        let mut parser = MessageParser::default();
        let found: Vec<Event> = [
            r#"{"kind":"taskStarted","id":6,"ruleInfo":"CompileMetalFile /p/Shaders.metal","executionDescription":"Compile Shaders.metal"}"#,
            r#"{"kind":"error","message":"unable to spawn process 'metal' (No such file or directory)","location":{"locationType":"unknown"},"locationContext":{"locationType":"task","targetID":0,"taskID":6}}"#,
            r#"{"kind":"warning","message":"Could not read serialized diagnostics file: Error Domain=NSCocoaErrorDomain Code=260","location":{"locationType":"path","path":"/dd/Shaders.dia"},"locationContext":{"locationType":"task","targetID":0,"taskID":6}}"#,
            r#"{"kind":"taskComplete","id":6,"result":"failed"}"#,
        ]
        .iter()
        .flat_map(|line| parser.push(line))
        .map(|parsed| parsed.event)
        .collect();
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(matches!(
            &found[1],
            Event::Diagnostic {
                kind: DiagKind::Error,
                ..
            }
        ));
        let blocker = parser.blocker().unwrap();
        assert!(
            blocker.starts_with("'Compile Shaders.metal' needs 'metal'"),
            "{blocker}"
        );
        assert_eq!(
            unspawned_tool("unable to spawn process 'x' (Permission denied)"),
            None
        );
    }

    #[test]
    fn a_failed_links_output_leaves_out_the_lld_platform_notice() {
        let found = events(&[
            r#"{"kind":"taskStarted","id":10,"ruleInfo":"Ld /dd/App.app/App normal","executionDescription":"Link App (arm64)"}"#,
            r#"{"kind":"taskOutput","taskID":10,"data":"ld64.lld: warning: This version of lld does not support linking for platform iOS. Proceed at your own risk.\nld64.lld: error: undefined symbol: _foo\n"}"#,
            r#"{"kind":"taskComplete","id":10,"result":"failed"}"#,
        ]);
        assert!(found.iter().any(|e| matches!(e, Event::Diagnostic { message, .. } if message.contains("undefined symbol: _foo") && !message.contains("Proceed at your own risk"))));
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
        assert!(refuse_xcodebuild_flags(&args(&["-quiet", "-verbose"])).is_ok());
        assert_eq!(output_levels(&args(&["-quiet", "A=B"])), (false, true));
        let err = refuse_xcodebuild_flags(&args(&["-allowProvisioningUpdates"])).unwrap_err();
        assert!(
            err.to_string()
                .contains("'-allowProvisioningUpdates' is an xcodebuild flag")
        );
    }

    #[test]
    fn build_requests_follow_the_schemes_dependency_policy() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-build-request");
        let project = dir.join("Demo.xcodeproj");
        let schemes = project.join("xcshareddata/xcschemes");
        std::fs::create_dir_all(&schemes).unwrap();
        let pif = vec![json!({"type":"target","contents":{"guid":"DEMO-GUID","name":"Demo"}})];
        let container = Container::Project(project.clone());
        let plan = BuildPlan {
            action: BuildAction::Build,
            container: &container,
            scheme: "Demo",
            configuration: "Debug",
            destination: Some("generic/platform=iOS Simulator"),
            sdk: None,
            clean: false,
            hot: false,
            hot_entitlements: None,
            result_bundle: None,
            passthrough: &[],
        };
        for implicit in [true, false] {
            let value = if implicit { "YES" } else { "NO" };
            std::fs::write(schemes.join("Demo.xcscheme"),format!(r#"<Scheme><BuildAction buildImplicitDependencies="{value}" parallelizeBuildables="NO"><BuildActionEntries><BuildActionEntry buildForRunning="YES"><BuildableReference BlueprintIdentifier="1" BlueprintName="Demo" ReferencedContainer="container:Demo.xcodeproj"/></BuildActionEntry></BuildActionEntries></BuildAction></Scheme>"#)).unwrap();
            let request =
                build_request(&plan, &project, &pif, &json!({"configurationName":"Debug"}))
                    .unwrap();
            assert_eq!(request["useImplicitDependencies"], implicit);
            assert_eq!(request["useParallelTargets"], false);
            assert_eq!(request["configuredTargets"], json!([{"guid":"DEMO-GUID"}]));
            assert_eq!(request["parameters"]["configurationName"], "Debug");
        }
    }

    #[test]
    fn profiles_are_checked_against_the_device_a_build_runs_on() {
        let udid = "00008110-001A2C3E0A12801E";
        let on = |destination: &str, sdk: &str| run_device(Some(destination), sdk);
        assert_eq!(
            on(&format!("platform=iOS,id={udid}"), "iphoneos"),
            Some(udid.to_string())
        );
        assert_eq!(
            on(&format!("platform=watchOS,id={udid}"), "watchos26.0"),
            Some(udid.to_string())
        );
        assert_eq!(
            on(
                &format!("platform=iOS Simulator,id={udid}"),
                "iphonesimulator"
            ),
            None
        );
        assert_eq!(on("generic/platform=iOS", "iphoneos"), None);
        assert_eq!(on(&format!("platform=macOS,id={udid}"), "macosx"), None);
        assert_eq!(run_device(None, "iphoneos"), None);
    }

    #[test]
    fn a_generic_destination_builds_every_architecture_unless_it_names_one() {
        assert!(builds_every_arch(Some("generic/platform=iOS Simulator")));
        assert!(builds_every_arch(Some("generic/platform=macOS")));
        assert!(!builds_every_arch(Some(
            "generic/platform=iOS Simulator,arch=arm64"
        )));
        assert!(!builds_every_arch(Some(
            "platform=iOS Simulator,name=iPhone 17"
        )));
        assert!(!builds_every_arch(None));
    }

    #[test]
    fn additional_apple_platforms_use_destination_resources_and_lld() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-platform-sdk");
        let sdk = DarwinSdk {
            bundle: dir.to_path_buf(),
        };
        for platform in ["appletvsimulator", "watchsimulator", "xrsimulator"] {
            let mut settings: BTreeMap<_, _> = sdk
                .overrides(platform, "arm64", &dir.join("toolset.json"))
                .into_iter()
                .collect();
            assert!(
                settings["SWIFT_RESOURCE_DIR"].ends_with("XcodeDefault.xctoolchain/usr/lib/swift")
            );
            assert!(
                settings["CLANG_RESOURCE_DIR"]
                    .ends_with("XcodeDefault.xctoolchain/usr/lib/swift/clang")
            );
            assert_eq!(settings["OTHER_LDFLAGS"], "$(inherited) -fuse-ld=lld");
            scope_darwin_settings(&mut settings, platform);
            assert!(!settings.contains_key("SWIFT_RESOURCE_DIR"));
            assert!(settings.contains_key(&format!("SWIFT_RESOURCE_DIR[sdk={platform}*]")));
        }
        for platform in ["iphoneos", "iphonesimulator27.2", "macosx"] {
            assert!(
                !sdk.overrides(platform, "arm64", &dir.join("toolset.json"))
                    .iter()
                    .any(|(k, _)| k == "SWIFT_RESOURCE_DIR")
            );
        }
    }

    #[test]
    fn the_settings_the_engine_adds_to_go_above_the_users() {
        let mut overrides: BTreeMap<String, String> = [
            (
                "OTHER_LDFLAGS[sdk=appletvsimulator*]",
                "$(inherited) -fuse-ld=lld",
            ),
            (
                "LIBRARY_SEARCH_PATHS[sdk=iphoneos*]",
                "$(inherited) /sdk/swift",
            ),
            ("ARCHS[sdk=appletvsimulator*]", "arm64"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let additions = take_additions(&mut overrides);
        assert_eq!(
            additions.keys().collect::<Vec<_>>(),
            [
                "LIBRARY_SEARCH_PATHS[sdk=iphoneos*]",
                "OTHER_LDFLAGS[sdk=appletvsimulator*]"
            ]
        );
        assert_eq!(
            overrides.keys().collect::<Vec<_>>(),
            ["ARCHS[sdk=appletvsimulator*]"]
        );
    }

    #[test]
    fn pins_written_back_keep_the_workspaces_format_and_origin() {
        let xcode = br#"{
  "originHash" : "7977",
  "pins" : [
    {
      "identity" : "swift-collections",
      "state" : {
        "version" : "1.1.0"
      }
    }
  ],
  "version" : 3
}
"#;
        let swiftpm = br#"{
  "pins" : [
    {
      "identity" : "swift-collections",
      "state" : {
        "version" : "1.7.6"
      }
    }
  ],
  "version" : 2
}
"#;
        assert!(!same_pins(xcode, swiftpm));
        let written = carry_origin(Some(xcode), swiftpm);
        assert_eq!(
            String::from_utf8(written.clone()).unwrap(),
            String::from_utf8(xcode.to_vec())
                .unwrap()
                .replace("1.1.0", "1.7.6")
        );
        assert!(same_pins(&written, swiftpm));
        assert_eq!(carry_origin(None, swiftpm), swiftpm);
    }

    #[test]
    fn a_simulator_build_is_signed_ad_hoc_by_the_stand_in() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-simulator-signing");
        let sdk = DarwinSdk {
            bundle: dir.to_path_buf(),
        };
        for platform in [
            "iphonesimulator27.2",
            "appletvsimulator",
            "watchsimulator",
            "xrsimulator",
        ] {
            assert!(DarwinSdk::signs(platform), "{platform}");
            let settings: BTreeMap<_, _> = sdk
                .overrides(platform, "arm64", &dir.join("toolset.json"))
                .into_iter()
                .collect();
            assert_eq!(settings["CODESIGN"], "codesign", "{platform}");
            assert!(!settings.contains_key("CODE_SIGNING_ALLOWED"), "{platform}");
        }
    }

    #[test]
    fn an_ios_request_configures_its_watch_companion_without_configuring_host_macros() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-companion-sdk");
        let sdk = DarwinSdk {
            bundle: dir.to_path_buf(),
        };
        let settings = sdk.embedded_platform_overrides(
            "iphonesimulator27.2",
            "arm64",
            &dir.join("toolset.json"),
        );
        assert!(settings.keys().all(|key| key.contains("[sdk=")));
        assert!(!settings.contains_key("ARCHS[sdk=iphonesimulator*]"));
        assert_eq!(settings["ARCHS[sdk=watchsimulator*]"], "arm64");
        assert_eq!(
            settings["OTHER_LDFLAGS[sdk=watchsimulator*]"],
            "$(inherited) -fuse-ld=lld"
        );
        assert!(settings["SWIFT_RESOURCE_DIR[sdk=watchsimulator*]"].ends_with("usr/lib/swift"));
        assert!(settings.contains_key("SWIFT_SDK_TOOLSETS[sdk=watchsimulator*]"));
    }

    #[test]
    fn a_scheme_names_a_project_target_then_a_package_product() {
        let target =
            |guid: &str, name: &str| json!({"type":"target","contents":{"guid":guid,"name":name}});
        let pif = vec![
            target("PACKAGE-TARGET:Shared", "Shared"),
            target("PACKAGE-PRODUCT:shared_Shared.Shared", "Shared-product"),
            target("PACKAGE-PRODUCT:vendor_Foo.Foo", "Foo-product"),
            target("PACKAGE-TARGET:Lonely", "Lonely"),
            target("APP-GUID", "Shared"),
        ];
        let guid = |name: &str| pif_target(&pif, name).map(|t| t["guid"].clone());
        assert_eq!(guid("Shared"), Some(json!("APP-GUID")));
        assert_eq!(guid("Foo"), Some(json!("PACKAGE-PRODUCT:vendor_Foo.Foo")));
        assert_eq!(guid("Lonely"), Some(json!("PACKAGE-TARGET:Lonely")));
        assert_eq!(guid("Missing"), None);
        let packages_only = &pif[..4];
        assert_eq!(
            pif_target(packages_only, "Shared").map(|t| t["guid"].clone()),
            Some(json!("PACKAGE-PRODUCT:shared_Shared.Shared"))
        );
    }

    #[test]
    fn explicit_test_plans_require_an_explicit_scheme_test_action() {
        let dir = crate::cli::testdir::TempDir::new("missing-test-plan");
        let project = dir.join("Empty.xcodeproj");
        std::fs::create_dir_all(&project).unwrap();
        assert!(scheme_targets(&project, "Empty", BuildFor::Testing, None, Some("Smoke")).is_err());
        let schemes = project.join("xcshareddata/xcschemes");
        std::fs::create_dir_all(&schemes).unwrap();
        std::fs::write(
            schemes.join("Empty.xcscheme"),
            "<Scheme><BuildAction/></Scheme>",
        )
        .unwrap();
        assert!(scheme_targets(&project, "Empty", BuildFor::Testing, None, Some("Smoke")).is_err());
        assert!(refuse_xcodebuild_flags(&["-testPlan".into()]).is_err());
        assert!(refuse_xcodebuild_flags(&["-testPlan".into(), "Smoke".into()]).is_ok());
    }

    /// The fixture's workspace has a `Core` framework in each of its two
    /// projects, and its `BCore` scheme builds B's.
    #[test]
    fn a_scheme_target_keeps_the_project_and_id_its_entry_names() {
        let dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../sweetpad-lib/ci/fixture-name-collision");
        let targets = scheme_targets(
            &dir.join("W.xcworkspace"),
            "BCore",
            BuildFor::Running,
            None,
            None,
        )
        .unwrap();
        assert_eq!(targets.len(), 1);
        let (project, id) = targets[0].project.clone().unwrap();
        assert!(project.ends_with("B/B.xcodeproj"), "{}", project.display());
        assert_eq!(id, "13D97D987A7E5AF1ED577B2E");
        let pif = sweetpad_lib::workspace_pif::pif(&dir.join("W.xcworkspace")).unwrap();
        let picked = scheme_pif_target(&pif, &targets[0]).unwrap();
        let by_name = pif_target(&pif, "Core").unwrap();
        assert_ne!(picked["guid"], by_name["guid"]);
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
            target_names(&scheme_targets(&project, "App", BuildFor::Testing, None, None).unwrap()),
            ["App", "AppTests", "KitTests"]
        );
        assert_eq!(
            target_names(&scheme_targets(&project, "App", BuildFor::Running, None, None).unwrap()),
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
            target_names(&scheme_targets(&project, "App", BuildFor::Testing, None, None).unwrap()),
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
    fn the_build_machine_comes_from_the_sdk() {
        let plist = vec![("BuildMachineOSBuild".to_string(), "26B131".to_string())];
        assert_eq!(
            build_machine_settings(&plist),
            [("MAC_OS_X_PRODUCT_BUILD_VERSION", "26B131".to_string())]
        );
        assert!(build_machine_settings(&[]).is_empty());
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
        let additions = BTreeMap::from([(
            "OTHER_LDFLAGS[sdk=appletvos*]".to_string(),
            "$(inherited) -fuse-ld=lld".to_string(),
        )]);
        let p = parameters(
            "Debug",
            "iphoneos",
            "arm64",
            &locations,
            (&overrides, &additions),
            None,
        );
        assert_eq!(p["activeRunDestination"]["platform"], "iphoneos");
        assert_eq!(p["activeRunDestination"]["sdkVariant"], "iphoneos");
        assert_eq!(
            p["arenaInfo"]["buildProductsPath"],
            "/dd/App-x/Build/Products"
        );
        assert_eq!(p["overrides"]["commandLine"]["table"]["A"], "1");
        assert_eq!(
            p["overrides"]["environmentConfig"]["table"]["OTHER_LDFLAGS[sdk=appletvos*]"],
            "$(inherited) -fuse-ld=lld"
        );
        let none = BTreeMap::new();
        let mac = parameters(
            "Debug",
            "macosx",
            "arm64",
            &locations,
            (&overrides, &none),
            None,
        );
        assert!(mac["overrides"].get("environmentConfig").is_none());
        assert_eq!(mac["activeRunDestination"]["sdkVariant"], "macos");
    }

    #[test]
    fn a_second_build_in_the_same_derived_data_waits_for_the_first() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-build-lock");
        let first = BuildLock::acquire(&dir, || panic!("nothing holds the folder yet")).unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let folder = dir.to_path_buf();
        let second = std::thread::spawn(move || {
            let mut waited = false;
            let lock = BuildLock::acquire(&folder, || waited = true).unwrap();
            sent.send(waited).unwrap();
            drop(lock);
        });
        assert!(
            received
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "the second build ran while the first held the folder"
        );
        drop(first);
        let waited = received
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert!(waited, "the second build said nothing about waiting");
        second.join().unwrap();
    }

    #[test]
    fn the_stand_ins_get_a_new_folder_when_sweetpad_or_its_sdk_changes() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-tools-fingerprint");
        let exe = dir.join("sweetpad");
        let bundle = dir.join("darwin.artifactbundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(&exe, "one build").unwrap();
        std::fs::write(bundle.join("info.json"), r#"{"xcodeVersion":"27.1"}"#).unwrap();
        let first = tools_fingerprint(&exe, &bundle);
        assert_eq!(first, tools_fingerprint(&exe, &bundle));
        std::fs::write(&exe, "a newer build").unwrap();
        let rebuilt = tools_fingerprint(&exe, &bundle);
        assert_ne!(first, rebuilt, "a new binary at the same path");
        std::fs::write(bundle.join("info.json"), r#"{"xcodeVersion":"27.2"}"#).unwrap();
        assert_ne!(
            rebuilt,
            tools_fingerprint(&exe, &bundle),
            "an SDK from another Xcode"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_stand_in_links_are_whole_and_old_folders_go() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-link-tools");
        let exe = std::env::current_exe().unwrap();
        let old = dir.join("Tools/older");
        let recent = dir.join("Tools/recent");
        let legacy = dir.join("bin");
        for folder in [&old, &recent, &legacy] {
            std::fs::create_dir_all(folder).unwrap();
        }
        let two_days_ago = std::time::SystemTime::now() - 2 * STALE_TOOLS;
        for folder in [&old, &legacy] {
            std::fs::File::open(folder)
                .unwrap()
                .set_modified(two_days_ago)
                .unwrap();
        }
        let tools = dir.join("Tools/current");
        link_tools(&tools, None).unwrap();
        assert_eq!(std::fs::read_link(tools.join("actool")).unwrap(), exe);
        std::fs::remove_file(tools.join("ibtool")).unwrap();
        link_tools(&tools, None).unwrap();
        assert_eq!(std::fs::read_link(tools.join("ibtool")).unwrap(), exe);
        let mut left: Vec<_> = std::fs::read_dir(dir.join("Tools"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["current", "recent"]);
        assert!(!legacy.exists());
    }

    #[test]
    fn a_killed_swbuild_names_its_signal_bare_or_through_bwrap() {
        use std::os::unix::process::ExitStatusExt as _;
        let killed = std::process::ExitStatus::from_raw(libc::SIGKILL);
        assert_eq!(killed_by(killed, false), Some(libc::SIGKILL));
        let through_bwrap = std::process::ExitStatus::from_raw((128 + libc::SIGSEGV) << 8);
        assert_eq!(killed_by(through_bwrap, true), Some(libc::SIGSEGV));
        assert_eq!(killed_by(through_bwrap, false), None);
        let failed = std::process::ExitStatus::from_raw(1 << 8);
        assert_eq!(killed_by(failed, true), None);
        assert_eq!(signal_name(libc::SIGKILL), "SIGKILL");
        assert_eq!(signal_name(64), "signal 64");
    }
}
