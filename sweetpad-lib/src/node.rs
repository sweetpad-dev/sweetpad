//! N-API bindings: the resolver exposed to the sweetpad VS Code extension as a
//! native node addon (`.node`), so the extension calls into Rust in-process
//! instead of spawning the CLI. Built into the cdylib under `--features node`
//! via `@napi-rs/cli` (`napi build`), which also generates the `.d.ts`. The CLI
//! (`main.rs`) stays the standalone / test entry point.
//!
//! Each function returns a typed object (`#[napi(object)]`) mapped from the
//! library's own structs — no JSON round-tripping.

// N-API entry points must take owned args (the runtime marshals them in); a
// borrowed `&str` isn't an option at the boundary.
#![allow(clippy::needless_pass_by_value)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use napi_derive::napi;

use crate::destination::parse_destination_arg;
use crate::{compiler_args, project, scheme, workspace, xcode};

/// Active Xcode toolchain info. Mirrors `xcrun xcodebuild -version` plus the
/// resolved `DEVELOPER_DIR`.
#[napi(object)]
pub struct XcodeVersion {
    pub developer_dir: String,
    pub short_version: String,
    pub build_version: String,
    pub major_version: u32,
}

/// Resolve the active Xcode (the one `xcode-select` points at).
#[napi]
#[must_use]
pub fn xcode_version() -> XcodeVersion {
    let info = xcode::active_install();
    XcodeVersion {
        developer_dir: info.developer_dir.display().to_string(),
        short_version: info.short_version.clone(),
        build_version: info.build_version.clone(),
        major_version: info.major_version(),
    }
}

/// A single `.xcodeproj`'s targets, configurations, and schemes (shared +
/// per-user, or autocreated per-target when no scheme file exists).
/// Mirrors `xcodebuild -list -project`.
#[napi(object)]
pub struct ProjectInfo {
    pub name: String,
    pub targets: Vec<String>,
    pub configurations: Vec<String>,
    pub schemes: Vec<String>,
}

/// List a `.xcodeproj`'s targets, configurations, and schemes.
#[napi]
pub fn list_project(path: String) -> napi::Result<ProjectInfo> {
    let project = project::open(Path::new(&path)).map_err(to_napi_err)?;
    Ok(ProjectInfo {
        name: project.name,
        targets: project.targets.into_iter().map(|t| t.name).collect(),
        configurations: project.configurations,
        schemes: project.schemes,
    })
}

/// A `.xcworkspace`'s declared `.xcodeproj` paths and merged schemes (the
/// workspace bundle's own plus every member project's, shared + per-user,
/// or autocreated per-target when no scheme file exists anywhere).
/// Mirrors `xcodebuild -list -workspace`, plus the `projects` paths.
#[napi(object)]
pub struct WorkspaceInfo {
    pub name: String,
    /// Absolute paths of every `.xcodeproj` the workspace declares, in order.
    pub projects: Vec<String>,
    pub schemes: Vec<String>,
}

/// List a `.xcworkspace`'s member projects and merged schemes.
#[napi]
pub fn list_workspace(path: String) -> napi::Result<WorkspaceInfo> {
    let ws = workspace::open(Path::new(&path)).map_err(to_napi_err)?;
    let projects = ws
        .project_refs
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let schemes = ws.merged_schemes();
    Ok(WorkspaceInfo {
        name: ws.name,
        projects,
        schemes,
    })
}

fn is_workspace(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("xcworkspace")
}

/// Scheme names for a `.xcodeproj` or `.xcworkspace` — shared and per-user
/// scheme files, falling back to autocreated per-target schemes when none
/// exist. For a workspace, merged across the bundle and its member projects
/// (sorted, like `xcodebuild -list -workspace`).
#[napi]
pub fn schemes(path: String) -> napi::Result<Vec<String>> {
    let p = Path::new(&path);
    if is_workspace(p) {
        Ok(workspace::open(p).map_err(to_napi_err)?.merged_schemes())
    } else {
        Ok(project::open(p).map_err(to_napi_err)?.schemes)
    }
}

/// Target names for a `.xcodeproj` or `.xcworkspace`. For a workspace, the
/// distinct targets across member projects in first-seen order.
#[napi]
pub fn targets(path: String) -> napi::Result<Vec<String>> {
    let p = Path::new(&path);
    if is_workspace(p) {
        Ok(workspace::open(p).map_err(to_napi_err)?.merged_targets())
    } else {
        Ok(project::open(p)
            .map_err(to_napi_err)?
            .targets
            .into_iter()
            .map(|t| t.name)
            .collect())
    }
}

/// Build-configuration names for a `.xcodeproj` or `.xcworkspace`. For a
/// workspace, the distinct configurations across member projects.
#[napi]
pub fn configurations(path: String) -> napi::Result<Vec<String>> {
    let p = Path::new(&path);
    if is_workspace(p) {
        Ok(workspace::open(p)
            .map_err(to_napi_err)?
            .merged_configurations())
    } else {
        Ok(project::open(p).map_err(to_napi_err)?.configurations)
    }
}

/// Options for a `buildSettings` resolution — mirrors
/// `xcodebuild -showBuildSettings`. Either `project` or `workspace` is required;
/// either `scheme` or `target` selects what to resolve.
#[napi(object)]
pub struct BuildSettingsOptions {
    pub project: Option<String>,
    pub workspace: Option<String>,
    pub scheme: Option<String>,
    pub target: Option<String>,
    pub configuration: String,
    /// SDK to bind conditionals to. Defaults to `macosx`. Ignored when
    /// `destination` is set (the destination's platform wins).
    pub sdk: Option<String>,
    /// Arch to bind conditionals to. Defaults to `arm64`. Ignored when
    /// `destination` is set.
    pub arch: Option<String>,
    /// `xcodebuild -destination` string, e.g. `platform=iOS Simulator,id=…`.
    pub destination: Option<String>,
    /// Extra `.xcconfig` overlay (`xcodebuild -xcconfig`).
    pub xcconfig: Option<String>,
    /// A specific `Xcode.app` / `Contents/Developer` to resolve against.
    pub xcode: Option<String>,
    /// `xcodebuild -derivedDataPath` override.
    pub derived_data_path: Option<String>,
    /// Restrict each target's returned settings to these keys. The resolver
    /// still computes the full map (settings reference each other via `$(…)`),
    /// but only these keys cross the boundary — pass the handful you read to
    /// avoid marshalling the full ~1.4k-entry map. Omit for every resolved key.
    pub keys: Option<Vec<String>>,
}

/// One target's resolved build settings (`{ KEY: VALUE }`).
#[napi(object)]
pub struct TargetBuildSettings {
    pub target: String,
    pub settings: HashMap<String, String>,
}

/// Map the N-API options to the library's `BuildSettingsOptions`, parsing the
/// destination string. Shared by `buildSettings` and `compilerArguments`.
fn core_options(
    options: BuildSettingsOptions,
) -> napi::Result<crate::build_settings::BuildSettingsOptions> {
    let destination = match options.destination.as_deref() {
        Some(s) => Some(
            parse_destination_arg(s)
                .ok_or_else(|| napi::Error::from_reason(format!("invalid destination: {s:?}")))?,
        ),
        None => None,
    };
    Ok(crate::build_settings::BuildSettingsOptions {
        project: options.project.map(PathBuf::from),
        workspace: options.workspace.map(PathBuf::from),
        scheme: options.scheme,
        target: options.target,
        configuration: options.configuration,
        sdk: options.sdk.unwrap_or_else(|| "macosx".into()),
        arch: options.arch.unwrap_or_else(|| "arm64".into()),
        destination,
        xcconfig: options.xcconfig.map(PathBuf::from),
        xcode: options.xcode.map(PathBuf::from),
        xcspec_root: None,
        sdksettings_root: None,
        catalog_cache: None,
        derived_data_path: options.derived_data_path.map(PathBuf::from),
        keys: options.keys,
    })
}

/// Resolve build settings for a scheme or target across a project/workspace.
/// Mirrors `xcodebuild -showBuildSettings`.
#[napi]
pub fn build_settings(options: BuildSettingsOptions) -> napi::Result<Vec<TargetBuildSettings>> {
    let opts = core_options(options)?;
    let resolved = crate::build_settings::resolve_build_settings(&opts).map_err(to_napi_err)?;
    Ok(resolved
        .into_iter()
        .map(|t| TargetBuildSettings {
            target: t.target,
            settings: t.settings.into_iter().collect(),
        })
        .collect())
}

/// One generated tool invocation: the tool, its argv, and the input files it
/// compiles (`.swift` for `swiftc`, the C-family sources for `clang`; empty for
/// the linker).
#[napi(object)]
pub struct CompilerToolInvocation {
    pub tool: String,
    pub arguments: Vec<String>,
    pub input_files: Vec<String>,
}

/// One target's generated per-tool compiler/linker argv. A field is absent when
/// the target has no inputs for that tool.
#[napi(object)]
pub struct TargetCompilerArguments {
    pub target: String,
    pub swift: Option<CompilerToolInvocation>,
    pub clang: Option<CompilerToolInvocation>,
    pub link: Option<CompilerToolInvocation>,
}

/// Resolve the per-tool compiler/linker argument vectors (`swiftc` / `clang` /
/// link) for a scheme or target — the command lines `xcodebuild` would invoke,
/// derived from the resolved build settings. Takes the same options as
/// [`build_settings`].
#[napi]
pub fn compiler_arguments(
    options: BuildSettingsOptions,
) -> napi::Result<Vec<TargetCompilerArguments>> {
    let opts = core_options(options)?;
    let resolved = crate::build_settings::resolve_compiler_arguments(&opts).map_err(to_napi_err)?;
    Ok(resolved
        .into_iter()
        .map(|t| TargetCompilerArguments {
            target: t.target,
            swift: t.swift.map(tool_to_napi),
            clang: t.clang.map(tool_to_napi),
            link: t.link.map(tool_to_napi),
        })
        .collect())
}

fn tool_to_napi(t: compiler_args::ToolInvocation) -> CompilerToolInvocation {
    CompilerToolInvocation {
        tool: t.tool,
        arguments: t.arguments,
        input_files: t.input_files,
    }
}

/// Run the Build Server Protocol server (see [`crate::bsp`]) over this process's
/// stdio, blocking until EOF / `build/exit`. `args` are the `bsp` flags, e.g.
/// `["--project", "App.xcodeproj", "--xcode", "/Applications/Xcode.app",
/// "--derived-data-path", "…"]`.
///
/// This lets sourcekit-lsp launch the server through VS Code's bundled Node +
/// the shipped addon (a `buildServer.json` `argv` of `[node, entry.js, …]`),
/// rather than a separate published binary.
#[napi]
pub fn bsp(args: Vec<String>) -> napi::Result<()> {
    crate::bsp::run(&args).map_err(to_napi_err)
}

/// A target referenced by a scheme (a build entry or a testable). Mirrors a
/// `BuildableReference` in the `.xcscheme` XML.
#[napi(object)]
pub struct SchemeBuildable {
    /// Target name — matches a `listProject` / `listWorkspace` target.
    pub blueprint_name: String,
    /// The target's pbxproj UUID.
    pub blueprint_identifier: String,
    /// Produced artifact filename, e.g. `Foo.app`.
    pub buildable_name: String,
    /// `ReferencedContainer`, e.g. `container:Foo.xcodeproj`.
    pub container: String,
}

/// One row of a scheme's Build action, with the five `buildFor*` flags. The
/// app a scheme launches is the first entry whose `forRunning` is true.
// The five flags mirror the scheme XML 1:1 (as in `scheme::BuildEntry`);
// collapsing them would obscure the mapping for no benefit.
#[allow(clippy::struct_excessive_bools)]
#[napi(object)]
pub struct SchemeBuildEntry {
    pub buildable: SchemeBuildable,
    pub for_running: bool,
    pub for_testing: bool,
    pub for_profiling: bool,
    pub for_archiving: bool,
    pub for_analyzing: bool,
}

/// A testable in a scheme's Test action.
#[napi(object)]
pub struct SchemeTestable {
    pub buildable: SchemeBuildable,
    pub skipped: bool,
}

/// A scheme's Test action.
#[napi(object)]
pub struct SchemeTestAction {
    pub configuration: String,
    pub testables: Vec<SchemeTestable>,
    pub code_coverage_enabled: bool,
}

/// A command-line argument under a scheme's `LaunchAction`.
#[napi(object)]
pub struct SchemeCommandLineArgument {
    pub argument: String,
    pub is_enabled: bool,
}

/// An environment variable under a scheme's `LaunchAction`.
#[napi(object)]
pub struct SchemeEnvironmentVariable {
    pub key: String,
    pub value: Option<String>,
    pub is_enabled: bool,
}

/// A parsed `.xcscheme`: its Build entries (with the `buildFor*` flags), Test
/// action, and each action's default build configuration — the scheme-level
/// facts `xcodebuild -showBuildSettings` can't tell you (which target a scheme
/// launches, the config it defaults to, …).
#[napi(object)]
pub struct SchemeInfo {
    pub build_entries: Vec<SchemeBuildEntry>,
    pub build_implicit_dependencies: bool,
    pub parallelize_buildables: bool,
    pub test_action: Option<SchemeTestAction>,
    /// The single target the scheme launches (`LaunchAction`'s runnable) — use
    /// this to pick which target's `buildSettings` to resolve, rather than
    /// guessing from `buildEntries[].forRunning`.
    pub launch_target: Option<SchemeBuildable>,
    pub launch_configuration: Option<String>,
    pub profile_configuration: Option<String>,
    pub archive_configuration: Option<String>,
    pub analyze_configuration: Option<String>,
    pub launch_arguments: Vec<SchemeCommandLineArgument>,
    pub launch_environment_variables: Vec<SchemeEnvironmentVariable>,
    pub launch_language: Option<String>,
    pub launch_region: Option<String>,
}

/// Parse a single `.xcscheme` file into its actions + per-action
/// configurations. Pair with `buildSettings` to resolve only the runnable
/// target instead of every buildable in the scheme.
#[napi]
pub fn parse_scheme(path: String) -> napi::Result<SchemeInfo> {
    let scheme = scheme::parse_file(Path::new(&path)).map_err(to_napi_err)?;
    Ok(scheme_to_napi(scheme))
}

fn buildable_to_napi(b: scheme::BuildableRef) -> SchemeBuildable {
    SchemeBuildable {
        blueprint_name: b.blueprint_name,
        blueprint_identifier: b.blueprint_identifier,
        buildable_name: b.buildable_name,
        container: b.container,
    }
}

fn scheme_to_napi(s: scheme::Scheme) -> SchemeInfo {
    SchemeInfo {
        build_entries: s
            .build_entries
            .into_iter()
            .map(|e| SchemeBuildEntry {
                buildable: buildable_to_napi(e.buildable),
                for_running: e.for_running,
                for_testing: e.for_testing,
                for_profiling: e.for_profiling,
                for_archiving: e.for_archiving,
                for_analyzing: e.for_analyzing,
            })
            .collect(),
        build_implicit_dependencies: s.build_implicit_dependencies,
        parallelize_buildables: s.parallelize_buildables,
        test_action: s.test_action.map(|t| SchemeTestAction {
            configuration: t.configuration,
            testables: t
                .testables
                .into_iter()
                .map(|t| SchemeTestable {
                    buildable: buildable_to_napi(t.buildable),
                    skipped: t.skipped,
                })
                .collect(),
            code_coverage_enabled: t.code_coverage_enabled,
        }),
        launch_target: s.launch_target.map(buildable_to_napi),
        launch_configuration: s.launch_configuration,
        profile_configuration: s.profile_configuration,
        archive_configuration: s.archive_configuration,
        analyze_configuration: s.analyze_configuration,
        launch_arguments: s
            .launch_arguments
            .into_iter()
            .map(|a| SchemeCommandLineArgument {
                argument: a.argument,
                is_enabled: a.is_enabled,
            })
            .collect(),
        launch_environment_variables: s
            .launch_environment_variables
            .into_iter()
            .map(|v| SchemeEnvironmentVariable {
                key: v.key,
                value: v.value,
                is_enabled: v.is_enabled,
            })
            .collect(),
        launch_language: s.launch_language,
        launch_region: s.launch_region,
    }
}

fn to_napi_err(e: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(e.to_string())
}
