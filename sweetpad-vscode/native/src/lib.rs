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

use sweetpad_core::app_locator::CommandLineSettings;
use sweetpad_core::xcodebuild_args;
use sweetpad_lib::destination::{RunDestination, parse_destination_arg};
use sweetpad_lib::{compiler_args, project, scheme, workspace, xcode};

/// Active Xcode toolchain info. Mirrors `xcrun xcodebuild -version` plus the
/// resolved `DEVELOPER_DIR`.
#[napi(object)]
pub struct XcodeVersion {
    pub developer_dir: String,
    pub short_version: String,
    pub build_version: String,
    pub major_version: u32,
}

/// Resolve the Xcode toolchain info. `developer_dir` pins a specific install
/// (the extension passes its login shell's `DEVELOPER_DIR`, which this
/// process's own environment doesn't see); omitted, the active Xcode (env
/// `DEVELOPER_DIR`, else `xcode-select -p`) is detected.
#[napi]
#[must_use]
pub fn xcode_version(developer_dir: Option<String>) -> XcodeVersion {
    let info = match developer_dir.as_deref() {
        Some(dir) if !dir.is_empty() => xcode::install_at(Path::new(dir)),
        _ => xcode::active_install(),
    };
    XcodeVersion {
        developer_dir: info.developer_dir.display().to_string(),
        short_version: info.short_version.clone(),
        build_version: info.build_version.clone(),
        major_version: info.major_version(),
    }
}

/// Forget the session-memoized Xcode state (the `xcode-select -p` result and
/// per-install `version.plist` reads) so the next call re-detects the active
/// Xcode. The addon is long-lived inside the extension host; call this after
/// the user switches Xcode (`xcode-select -s`, a changed `DEVELOPER_DIR`) or
/// refreshes the shell environment.
#[napi]
pub fn flush_xcode_cache() {
    xcode::flush_caches();
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
    /// Absolute paths of every local SwiftPM package the workspace declares.
    /// Their products are NOT in `schemes`: naming those needs a manifest
    /// evaluation, which only the async [`schemes`] does.
    pub packages: Vec<String>,
    pub schemes: Vec<String>,
}

/// List a `.xcworkspace`'s member projects, member packages, and merged
/// schemes.
#[napi]
pub fn list_workspace(path: String) -> napi::Result<WorkspaceInfo> {
    let ws = workspace::open(Path::new(&path)).map_err(to_napi_err)?;
    let projects = ws
        .project_refs
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let packages = ws
        .package_refs
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let schemes = ws.merged_schemes();
    Ok(WorkspaceInfo {
        name: ws.name,
        projects,
        packages,
        schemes,
    })
}

fn is_workspace(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("xcworkspace")
}

/// Which toolchain evaluates the manifests a names read needs. The extension
/// host sees neither the login shell's `DEVELOPER_DIR` nor
/// `sweetpad.build.swiftCommand`, so the extension passes both.
#[napi(object)]
pub struct ManifestToolchain {
    /// The `swift` to run, when not the one on `PATH`.
    pub swift: Option<String>,
    /// The `DEVELOPER_DIR` to run it under.
    pub developer_dir: Option<String>,
}

fn toolchain_of(options: Option<ManifestToolchain>) -> sweetpad_core::package_members::Toolchain {
    let non_empty = |value: Option<String>| value.filter(|v| !v.is_empty()).map(PathBuf::from);
    options.map_or_else(Default::default, |options| {
        sweetpad_core::package_members::Toolchain {
            swift: non_empty(options.swift),
            developer_dir: non_empty(options.developer_dir),
        }
    })
}

/// Which names to read. Both include what the container's local SwiftPM
/// packages declare, and naming those means running `swift package
/// dump-package`, which takes seconds on a cold manifest cache — so the calls
/// are promises, computed on a worker thread rather than on the event loop the
/// whole editor shares.
enum Names {
    Schemes,
    Targets,
}

/// Read `path`'s names, evaluating the manifests of every local package the
/// container reaches. A `Package.swift` reads the package opened on its own:
/// what `xcodebuild -list` prints in its directory. Runs off the main thread
/// (see [`NamesTask`]).
fn read_names(
    path: &str,
    which: &Names,
    toolchain: &sweetpad_core::package_members::Toolchain,
) -> napi::Result<Vec<String>> {
    let p = Path::new(path);
    if p.file_name().and_then(|n| n.to_str()) == Some("Package.swift") {
        let dir = p.parent().unwrap_or(Path::new("."));
        let package =
            sweetpad_core::package_members::standalone(dir, toolchain, std::process::Stdio::null())
                .map_err(to_napi_err)?;
        return Ok(match which {
            Names::Schemes => package.schemes,
            Names::Targets => package.targets,
        });
    }
    if !is_workspace(p) {
        let project = project::open(p).map_err(to_napi_err)?;
        let members = sweetpad_core::package_members::resolve_project(&project, toolchain);
        return Ok(match which {
            Names::Schemes => project
                .schemes_with_packages(&sweetpad_core::package_members::scheme_pairs(&members)),
            Names::Targets => project
                .targets_with_packages(&sweetpad_core::package_members::target_pairs(&members)),
        });
    }
    let ws = workspace::open(p).map_err(to_napi_err)?;
    let members = sweetpad_core::package_members::resolve_workspace(&ws, toolchain);
    Ok(match which {
        Names::Schemes => {
            ws.merged_schemes_with_packages(&sweetpad_core::package_members::scheme_pairs(&members))
        }
        Names::Targets => {
            ws.merged_targets_with_packages(&sweetpad_core::package_members::target_pairs(&members))
        }
    })
}

pub struct NamesTask {
    path: String,
    which: Names,
    toolchain: sweetpad_core::package_members::Toolchain,
}

impl napi::Task for NamesTask {
    type Output = Vec<String>;
    type JsValue = Vec<String>;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        read_names(&self.path, &self.which, &self.toolchain)
    }

    fn resolve(&mut self, _env: napi::Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output)
    }
}

/// Scheme names for a `.xcodeproj`, `.xcworkspace` or `Package.swift` —
/// shared and per-user scheme files, plus autocreated per-target schemes, plus
/// the products of every local SwiftPM package the container reaches. For a
/// workspace, merged across the bundle and every member project, sorted like
/// `xcodebuild -list -workspace`. For a package, its `.swiftpm/xcode` scheme
/// files and the schemes its manifest synthesizes, sorted like `xcodebuild
/// -list` in its directory. `toolchain` evaluates the manifests.
#[napi(ts_return_type = "Promise<Array<string>>")]
#[must_use]
pub fn schemes(
    path: String,
    toolchain: Option<ManifestToolchain>,
) -> napi::bindgen_prelude::AsyncTask<NamesTask> {
    napi::bindgen_prelude::AsyncTask::new(NamesTask {
        path,
        which: Names::Schemes,
        toolchain: toolchain_of(toolchain),
    })
}

/// Target names for a `.xcodeproj`, `.xcworkspace` or `Package.swift`. For a
/// workspace, the distinct targets across member projects in first-seen
/// order; then, for either, each local package's targets — test targets
/// included, since a target list drives `-only-testing:`. For a package, every
/// target its manifest declares.
#[napi(ts_return_type = "Promise<Array<string>>")]
#[must_use]
pub fn targets(
    path: String,
    toolchain: Option<ManifestToolchain>,
) -> napi::bindgen_prelude::AsyncTask<NamesTask> {
    napi::bindgen_prelude::AsyncTask::new(NamesTask {
        path,
        which: Names::Targets,
        toolchain: toolchain_of(toolchain),
    })
}

/// A container the discovery walk found.
#[napi(object)]
pub struct DiscoveredContainer {
    /// The `.xcworkspace`, `.xcodeproj` or `Package.swift`.
    pub path: String,
    /// `"workspace"`, `"project"` or `"package"`.
    pub kind: String,
    /// How many directories below the root it sits: 0 for one in the root.
    pub depth: u32,
}

pub struct DiscoverTask {
    root: String,
    max_depth: u32,
}

impl napi::Task for DiscoverTask {
    type Output = Vec<DiscoveredContainer>;
    type JsValue = Vec<DiscoveredContainer>;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        use sweetpad_lib::discover::{self, Kind};
        let max_depth = usize::try_from(self.max_depth).unwrap_or(usize::MAX);
        Ok(discover::containers(Path::new(&self.root), max_depth)
            .into_iter()
            .map(|(kind, depth, path)| DiscoveredContainer {
                path: path.display().to_string(),
                kind: match kind {
                    Kind::Workspace => "workspace",
                    Kind::Project => "project",
                    Kind::Package => "package",
                }
                .to_string(),
                depth: u32::try_from(depth).unwrap_or(u32::MAX),
            })
            .collect())
    }

    fn resolve(&mut self, _env: napi::Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output)
    }
}

/// Every container from `root` down to `maxDepth` directories below it — the
/// walk the CLI's auto-discovery takes. Vendored trees (`Pods`,
/// `node_modules`, `Carthage`, `vendor`, `DerivedData`, `build`,
/// `SourcePackages`), dotted directories (`.build`, `.swiftpm`) and bundles
/// are never entered, nor is a symlink. Nearer directories come first, and a
/// directory's workspaces before its projects before its package, so the
/// first is the one to open when nothing says which. A promise, since a big
/// tree takes a while to read: the walk runs on a worker thread.
#[napi(ts_return_type = "Promise<Array<DiscoveredContainer>>")]
#[must_use]
pub fn discover_containers(
    root: String,
    max_depth: u32,
) -> napi::bindgen_prelude::AsyncTask<DiscoverTask> {
    napi::bindgen_prelude::AsyncTask::new(DiscoverTask { root, max_depth })
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
    /// SDK to bind conditionals to. Omitted, each target resolves under the
    /// SDK its own `SDKROOT` names, as a plain `xcodebuild
    /// -showBuildSettings` does. Ignored when `destination` is set (the
    /// destination's platform wins).
    pub sdk: Option<String>,
    /// Arch to bind conditionals to. Omitted, the resolver's default, as for
    /// the CLI. Ignored when `destination` is set.
    pub arch: Option<String>,
    /// `xcodebuild -destination` string, e.g. `platform=iOS Simulator,id=…`.
    pub destination: Option<String>,
    /// Extra `.xcconfig` overlay (`xcodebuild -xcconfig`).
    pub xcconfig: Option<String>,
    /// A specific `Xcode.app` / `Contents/Developer` to resolve against.
    pub xcode: Option<String>,
    /// `xcodebuild -derivedDataPath` override.
    pub derived_data_path: Option<String>,
    /// The arguments the extension's builds add to their own
    /// (`sweetpad.build.args`), read as `xcodebuild` reads them: their
    /// `KEY=VALUE` settings sit above every project layer, and a `-xcconfig`,
    /// `-derivedDataPath`, `-scheme` or `-configuration` among them replaces
    /// the one these options name, as it does on the build's command line.
    pub build_args: Option<Vec<String>>,
    /// The directory the build runs `xcodebuild` in, which a relative path in
    /// `buildArgs` is read against.
    pub working_directory: Option<String>,
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

/// Parse an `xcodebuild -destination` string, refusing one without a platform.
fn destination(spec: Option<&str>) -> napi::Result<Option<RunDestination>> {
    spec.map(|s| {
        parse_destination_arg(s)
            .ok_or_else(|| napi::Error::from_reason(format!("invalid destination: {s:?}")))
    })
    .transpose()
}

/// Map the N-API options to the library's `BuildSettingsOptions`, parsing the
/// destination string and reading the build's own arguments. Shared by
/// `buildSettings`, `locateApp` and `compilerArguments`.
fn core_options(
    options: BuildSettingsOptions,
) -> napi::Result<sweetpad_core::build_settings::BuildSettingsOptions> {
    let destination = destination(options.destination.as_deref())?;
    let args = options.build_args.unwrap_or_default();
    let command_line =
        CommandLineSettings::of(&args, options.working_directory.as_deref().map(Path::new));
    // The extension's builds let a flag in `sweetpad.build.args` replace the
    // one they pass themselves, so the build ran with the last one given.
    let last = |flag: &str| xcodebuild_args::last_value(&args, flag).map(String::from);
    Ok(sweetpad_core::build_settings::BuildSettingsOptions {
        project: options.project.map(PathBuf::from),
        workspace: options.workspace.map(PathBuf::from),
        scheme: options
            .scheme
            .map(|picked| last("-scheme").unwrap_or(picked)),
        target: options.target,
        configuration: last("-configuration").unwrap_or(options.configuration),
        sdk: options.sdk.unwrap_or_default(),
        arch: options.arch.unwrap_or_default(),
        destination,
        xcconfig: command_line
            .xcconfig
            .or_else(|| options.xcconfig.map(PathBuf::from)),
        xcode: options.xcode.map(PathBuf::from),
        xcspec_root: None,
        sdksettings_root: None,
        catalog_cache: None,
        derived_data_path: command_line
            .derived_data_path
            .or_else(|| options.derived_data_path.map(PathBuf::from)),
        overrides: command_line.overrides,
        // The extension locates the built app from these paths, so they must
        // follow the host's Xcode Derived Data configuration.
        read_xcode_locations: true,
        keys: options.keys,
    })
}

/// Resolve build settings for a scheme or target across a project/workspace.
/// Mirrors `xcodebuild -showBuildSettings`.
#[napi]
pub fn build_settings(options: BuildSettingsOptions) -> napi::Result<Vec<TargetBuildSettings>> {
    let opts = core_options(options)?;
    let resolved =
        sweetpad_core::build_settings::resolve_build_settings(&opts).map_err(to_napi_err)?;
    Ok(resolved
        .into_iter()
        .map(|t| TargetBuildSettings {
            target: t.target,
            settings: t.settings.into_iter().collect(),
        })
        .collect())
}

/// The app a build produced: the target that builds it, the `.app` path, its
/// bundle id, the executable inside it, and the target's resolved settings.
#[napi(object)]
pub struct LocatedApp {
    pub target: String,
    pub path: String,
    pub bundle_id: String,
    /// `TARGET_BUILD_DIR/EXECUTABLE_PATH`, the binary to run for a macOS app.
    pub executable: String,
    pub settings: HashMap<String, String>,
}

fn located_to_napi(located: sweetpad_core::app_locator::Located) -> LocatedApp {
    LocatedApp {
        target: located.settings.target,
        path: located.app.path.display().to_string(),
        bundle_id: located.app.bundle_id,
        executable: located.app.executable.display().to_string(),
        settings: located.settings.settings.into_iter().collect(),
    }
}

/// Find the app a build with these options writes, through the in-process
/// resolver: the target the scheme's Run action launches, or the one that
/// runs on the destination, with the product path, bundle id and executable
/// the build's own arguments give it. The CLI's `app` verbs locate the app
/// the same way. `keys` trims the located target's returned settings.
#[napi]
pub fn locate_app(options: BuildSettingsOptions) -> napi::Result<LocatedApp> {
    let opts = core_options(options)?;
    sweetpad_core::app_locator::locate(opts)
        .map(located_to_napi)
        .map_err(to_napi_err)
}

/// Options for [`pick_app`].
#[napi(object)]
pub struct PickAppOptions {
    /// Each target's settings, as `xcodebuild -showBuildSettings` reported
    /// them for the build.
    pub targets: Vec<TargetBuildSettings>,
    /// The `.xcodeproj` or `.xcworkspace` and the scheme the settings are
    /// for: the scheme's Run action names the target Xcode launches.
    pub container: String,
    pub scheme: String,
    /// The build's `xcodebuild -destination` string.
    pub destination: Option<String>,
    /// The build's SDK, which narrows the pick when there is no destination.
    pub sdk: Option<String>,
    /// Restrict the picked target's returned settings to these keys.
    pub keys: Option<Vec<String>>,
}

/// Pick the app among build settings resolved outside the addon (through a
/// customized `xcodebuild`), by the rules [`locate_app`] uses.
#[napi]
pub fn pick_app(options: PickAppOptions) -> napi::Result<LocatedApp> {
    let targets = options
        .targets
        .into_iter()
        .map(|t| sweetpad_core::build_settings::TargetSettings {
            target: t.target,
            settings: t.settings.into_iter().collect(),
        })
        .collect();
    let destination = destination(options.destination.as_deref())?;
    let platform = sweetpad_core::app_locator::platform(
        destination.as_ref(),
        options.sdk.as_deref().unwrap_or_default(),
    );
    let launch =
        sweetpad_core::app_locator::launch_target(Path::new(&options.container), &options.scheme);
    sweetpad_core::app_locator::pick(
        targets,
        platform.as_deref(),
        launch.as_deref(),
        options.keys.as_deref(),
    )
    .map(located_to_napi)
    .map_err(to_napi_err)
}

/// The platforms (`SUPPORTED_PLATFORMS` tokens, `iphonesimulator`) the
/// scheme's targets build for under `configuration`, read from the targets'
/// authored settings the way the CLI's destination picker reads them. `null`
/// when that can't be told (a Swift package, an unreadable project, targets
/// that author neither `SUPPORTED_PLATFORMS` nor `SDKROOT`): offer every
/// destination then.
#[napi]
#[must_use]
pub fn supported_platforms(
    container: String,
    scheme: String,
    configuration: String,
) -> Option<Vec<String>> {
    sweetpad_core::supported_platforms::SupportedPlatforms::resolve(
        Path::new(&container),
        &scheme,
        &configuration,
    )
    .map(|p| p.tokens().map(str::to_string).collect())
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
    let resolved =
        sweetpad_core::build_settings::resolve_compiler_arguments(&opts).map_err(to_napi_err)?;
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
    /// Whether an enabled launch argument or environment value refers to a
    /// build setting, so `schemeLaunchSettings` needs resolved settings to
    /// expand it.
    pub launch_references_settings: bool,
}

/// Parse a single `.xcscheme` file into its actions + per-action
/// configurations. Pair with `buildSettings` to resolve only the runnable
/// target instead of every buildable in the scheme.
#[napi]
pub fn parse_scheme(path: String) -> napi::Result<SchemeInfo> {
    let scheme = scheme::parse_file(Path::new(&path)).map_err(to_napi_err)?;
    Ok(scheme_to_napi(scheme))
}

/// The `.xcscheme` file behind the scheme `name` of a `.xcworkspace`,
/// `.xcodeproj` or `Package.swift`: the one `xcodebuild` reads, from the
/// container, its member projects and its local packages, and only the
/// current user's `xcuserdata`. `null` for a scheme with no file (autocreated)
/// or a name the container doesn't list.
#[napi]
#[must_use]
pub fn locate_scheme(container: String, name: String) -> Option<String> {
    scheme::locate(Path::new(&container), &name).map(|p| p.display().to_string())
}

/// Every file named for the scheme `name` in the places [`locate_scheme`]
/// looks, the one it returns first.
#[napi]
#[must_use]
pub fn scheme_files(container: String, name: String) -> Vec<String> {
    scheme::locate_all(Path::new(&container), &name)
        .into_iter()
        .map(|p| p.display().to_string())
        .collect()
}

/// What a scheme's Run action launches its app with.
#[napi(object)]
pub struct SchemeLaunchSettings {
    /// Process arguments, in launch order.
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// The arguments and environment Xcode launches the app of the scheme at
/// `path` with: enabled rows only, `$(VAR)` expanded, arguments split with
/// shell-style quoting, then the App Language and App Region flags.
/// `buildSettings` are the scheme's resolved targets. Those of its expansion
/// target (the Run action's `MacroExpansion`, else the target it launches,
/// else the first) expand `$(VAR)`. An empty list is enough when
/// `parseScheme(path).launchReferencesSettings` is false.
#[napi]
pub fn scheme_launch_settings(
    path: String,
    build_settings: Vec<TargetBuildSettings>,
) -> napi::Result<SchemeLaunchSettings> {
    let scheme = scheme::parse_file(Path::new(&path)).map_err(to_napi_err)?;
    let named = scheme
        .launch_expansion_target()
        .and_then(|t| build_settings.iter().find(|s| s.target == t.blueprint_name));
    let settings: std::collections::BTreeMap<String, String> = named
        .or(build_settings.first())
        .map(|t| t.settings.clone().into_iter().collect())
        .unwrap_or_default();
    let launch = scheme.launch_settings(&settings, scheme::host_language);
    Ok(SchemeLaunchSettings {
        args: launch.args,
        env: launch.env.into_iter().collect(),
    })
}

/// One `KEY=VALUE` build setting override.
#[napi(object)]
pub struct BuildSettingOverride {
    pub name: String,
    pub value: String,
}

/// The build settings a hot-reload build for `sdk` adds, in order: an
/// interposable link and frontend command lines, and on macOS no hardened
/// runtime or App Sandbox, without which injection fails silently. Empty when
/// hot reload can't inject into `sdk` (devices, watchOS).
#[napi]
#[must_use]
pub fn hot_reload_build_settings(sdk: String) -> Vec<BuildSettingOverride> {
    sweetpad_core::hot::build_settings(&sdk, None)
        .into_iter()
        .map(|(name, value)| BuildSettingOverride {
            name: name.to_string(),
            value,
        })
        .collect()
}

/// The InjectionNext client dylib that injects into apps built for `sdk`, or
/// null when hot reload can't inject into it.
#[napi]
#[must_use]
pub fn hot_reload_dylib_name(sdk: String) -> Option<String> {
    sweetpad_core::hot::dylib_name(&sdk).map(str::to_string)
}

/// The `<Platform>.platform` directory of an injectable `sdk`, where the
/// InjectionNext.app client finds the XCTest it links; null otherwise.
#[napi]
#[must_use]
pub fn hot_reload_platform_dir(sdk: String) -> Option<String> {
    sweetpad_core::hot::platform_dir(&sdk).map(str::to_string)
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
    let launch_references_settings = s.launch_references_settings();
    SchemeInfo {
        launch_references_settings,
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

/// An `xcodebuild -destination` argument read field by field, e.g.
/// `platform=iOS Simulator,id=<udid>,arch=x86_64`. Each field is `undefined`
/// when the argument leaves it out.
#[napi(object)]
pub struct DestinationFields {
    /// The `platform=` (or `generic/platform=`) value as written.
    pub platform: Option<String>,
    /// The SDK that platform builds against (`iphonesimulator`), when the
    /// platform is a known one.
    pub sdk: Option<String>,
    /// `generic/platform=…`: a device-less build.
    pub generic: bool,
    pub id: Option<String>,
    pub name: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub variant: Option<String>,
}

/// Read a `-destination` argument the way the CLI does. Never fails: an
/// unknown platform just has no `sdk`.
#[napi]
#[must_use]
pub fn parse_destination(destination: String) -> DestinationFields {
    let spec = sweetpad_lib::destination::DestinationSpec::parse(&destination);
    DestinationFields {
        sdk: spec.sdk().map(str::to_string),
        platform: spec.platform_label,
        generic: spec.generic,
        id: spec.id,
        name: spec.name,
        os: spec.os,
        arch: spec.arch,
        variant: spec.variant,
    }
}

/// A simulator from `simctl list --json devices`.
#[napi(object)]
pub struct SimctlSimulator {
    pub udid: String,
    pub name: String,
    /// `Booted` / `Shutdown` / `Booting` …, as simctl reports it.
    pub state: String,
    /// The runtime's OS: `iOS`, `watchOS`, `tvOS`, `xrOS`.
    pub os: String,
    /// `27.0`.
    pub os_version: String,
    /// `com.apple.CoreSimulator.SimRuntime.iOS-27-0`.
    pub runtime: String,
    /// `com.apple.CoreSimulator.SimDeviceType.iPhone-17`; empty when simctl
    /// leaves it out.
    pub device_type_identifier: String,
    /// The `-destination` specifier for it: `platform=iOS Simulator,id=<udid>`.
    pub destination: String,
}

/// Parse `simctl list --json devices` output into the available simulators,
/// in the CLI's picker order. Unavailable simulators are left out.
#[napi]
pub fn parse_simulators(json: String) -> napi::Result<Vec<SimctlSimulator>> {
    let sims = sweetpad_core::devices::simctl::parse_list(&json).map_err(to_napi_err)?;
    Ok(sims
        .into_iter()
        .map(|s| SimctlSimulator {
            destination: s.destination(),
            udid: s.udid,
            name: s.name,
            state: s.state,
            os: s.os,
            os_version: s.os_version,
            runtime: s.runtime,
            device_type_identifier: s.device_type,
        })
        .collect())
}

/// A physical device from `devicectl list devices` JSON, in either of the
/// shapes devicectl writes. Each optional field is `undefined` when devicectl
/// leaves it out.
#[napi(object)]
pub struct DevicectlDevice {
    /// CoreDevice's identifier, which devicectl's `--device` takes.
    pub identifier: String,
    /// The hex hardware UDID (`00008110-000559182E90401E`) xcodebuild's `id=`
    /// takes. `undefined` for the entries devicectl lists with an empty
    /// hardware section (some USB devices on iOS 16 and older).
    pub udid: Option<String>,
    pub name: Option<String>,
    /// `iPhone 13`.
    pub marketing_name: Option<String>,
    /// `iPhone14,5`.
    pub product_type: Option<String>,
    /// `iPhone`, `iPad`, `appleWatch`, `appleTV`, `appleVision`,
    /// `realityDevice`.
    pub device_type: Option<String>,
    /// devicectl's platform, `iOS` when it reports none.
    pub platform: String,
    /// `27.0`.
    pub os_version: Option<String>,
    /// `connected` / `disconnected` / `unavailable`.
    pub connection: Option<String>,
    /// `wired` / `localNetwork`.
    pub transport: Option<String>,
    /// `paired` once the device trusts this Mac.
    pub pairing: Option<String>,
    /// When the device last connected, in milliseconds since the Unix epoch.
    pub last_connection_ms: Option<f64>,
}

/// Parse `devicectl list devices --json-output` JSON into the physical
/// devices, sorted by name. The simulators Xcode 27's devicectl lists beside
/// them are left out: `parseSimulators` reads those from simctl.
#[napi]
pub fn parse_devicectl_devices(json: String) -> napi::Result<Vec<DevicectlDevice>> {
    let present = |s: String| (!s.is_empty()).then_some(s);
    let devices = sweetpad_core::devices::devicectl::parse_list(&json).map_err(to_napi_err)?;
    Ok(devices
        .into_iter()
        .map(|d| DevicectlDevice {
            udid: d.has_hardware_udid.then_some(d.udid),
            identifier: d.identifier,
            name: present(d.name),
            marketing_name: present(d.marketing_name),
            product_type: present(d.product_type),
            device_type: present(d.device_type),
            platform: d.platform,
            os_version: present(d.os_version),
            connection: present(d.connection),
            transport: present(d.transport),
            pairing: present(d.pairing),
            last_connection_ms: d.last_connection_ms,
        })
        .collect())
}

/// A process running out of an app bundle on a device.
#[napi(object)]
pub struct DevicectlAppProcess {
    pub pid: i64,
    /// The executable as devicectl reports it, a `file://` URL.
    pub executable: String,
    /// The bundle's path on the device, decoded:
    /// `/private/var/containers/Bundle/Application/<id>/My App.app`.
    pub app_path: String,
    /// Whether this is the app's own executable rather than an extension's.
    pub main: bool,
}

/// The processes in `devicectl device info processes --json-output` JSON that
/// run out of the `.app` directory named `appDirName` (`My App.app`). The
/// directory has to match whole, so `App.app` never matches `MyApp.app`.
#[napi]
pub fn devicectl_app_processes(
    json: String,
    app_dir_name: String,
) -> napi::Result<Vec<DevicectlAppProcess>> {
    let processes = sweetpad_core::devices::devicectl::parse_app_processes(&json, &app_dir_name)
        .map_err(to_napi_err)?;
    Ok(processes
        .into_iter()
        .map(|p| DevicectlAppProcess {
            pid: p.pid,
            executable: p.executable,
            app_path: p.app_path,
            main: p.main,
        })
        .collect())
}

fn to_napi_err(e: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(e.to_string())
}
