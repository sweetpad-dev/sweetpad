//! Typed model of an `.xcscheme` file.
//!
//! Built on top of [`crate::xcscheme`]'s `Element` parser, this module turns
//! the raw XML tree into structs the planner can drive resolution off:
//! [`Scheme`] with its [`BuildEntry`]s, [`TestAction`], and per-action
//! configuration names.
//!
//! Schemes are the link between "I want to do X with this app" (run, test,
//! profile, archive) and "these are the targets that need resolving."
//! [`crate::build_context::BuildContext::plan_build`] consumes one of these
//! to produce a `Vec<ResolveQuery>`. [`Scheme::launch_settings`] turns the
//! Run action's arguments, environment and app language into what the app is
//! launched with.
//!
//! What's NOT modeled (yet, deliberately): pre/post actions, test plans,
//! custom working directory, debugger / launcher identifiers. Add these
//! incrementally as concrete callers need them — see DOCS.md §3.2 "minimum
//! abstraction."

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::resolver::expand_one;
use crate::xcscheme::{self, Element};

#[derive(Debug, Clone)]
pub struct Scheme {
    /// Buildables in scheme-declared order. The first entry is what Xcode
    /// shows at the top of the scheme editor's Build phase.
    pub build_entries: Vec<BuildEntry>,
    /// Whether xcodebuild walks pbxproj `PBXTargetDependency` edges +
    /// product-name matches when building this scheme. xcodebuild defaults
    /// to YES; we record the value but don't honor it yet (the planner
    /// only emits explicit `build_entries` for now).
    pub build_implicit_dependencies: bool,
    /// Build action's `parallelizeBuildables` attribute.
    pub parallelize_buildables: bool,
    /// Test action — present in every scheme Xcode generates, but
    /// sometimes empty (no testables wired up).
    pub test_action: Option<TestAction>,
    /// `LaunchAction.BuildableProductRunnable` (or `RemoteRunnable` for
    /// watch-app schemes) — the single target the scheme launches (the app).
    /// Distinct from the BuildAction's `for_running` buildables, which also
    /// include frameworks/dependencies built for the run; this is the one
    /// Xcode actually launches. `None` when the scheme has no runnable
    /// (e.g. a library-only scheme).
    pub launch_target: Option<BuildableRef>,
    /// `LaunchAction.buildConfiguration` — the config xcodebuild defaults
    /// to when no explicit `-configuration` is passed.
    pub launch_configuration: Option<String>,
    /// `ProfileAction.buildConfiguration`.
    pub profile_configuration: Option<String>,
    /// `ArchiveAction.buildConfiguration`.
    pub archive_configuration: Option<String>,
    /// `AnalyzeAction.buildConfiguration`.
    pub analyze_configuration: Option<String>,
    /// `LaunchAction.MacroExpansion` — the target whose build settings expand
    /// `$(VAR)` in the launch arguments and environment (the scheme editor's
    /// "Expand Variables Based On"). `None` when absent, and then Xcode uses
    /// [`Self::launch_target`]; [`Self::launch_expansion_target`] applies
    /// that fallback.
    pub launch_macro_expansion: Option<BuildableRef>,
    /// `LaunchAction.CommandLineArguments`, in scheme order, disabled rows
    /// included. [`Self::launch_settings`] turns them into process arguments.
    pub launch_arguments: Vec<CommandLineArgument>,
    /// `LaunchAction.EnvironmentVariables`, in scheme order.
    pub launch_environment_variables: Vec<EnvironmentVariable>,
    /// `LaunchAction`'s `language` attribute (drives `-AppleLanguages`).
    pub launch_language: Option<String>,
    /// `LaunchAction`'s `region` attribute (drives `-AppleLocale`).
    pub launch_region: Option<String>,
    /// `LaunchAction`'s sanitizer toggles. Only the LAUNCH action's flags
    /// reach `xcodebuild`'s scheme build-settings view: the corpus captures
    /// both shapes — Alamofire's `iOS Example` / `watchOS Example WatchKit
    /// App` schemes set `enableThreadSanitizer="YES"` on `LaunchAction` and
    /// every capture reports `ENABLE_THREAD_SANITIZER = YES` (with the
    /// `-tsan` object-dir suffix), while `Alamofire watchOS` sets the same
    /// attribute on `TestAction` and its captures stay NO.
    pub launch_sanitizers: SanitizerEnables,
}

/// Scheme-level sanitizer enablement, parsed from a `LaunchAction`'s
/// `enableAddressSanitizer` / `enableThreadSanitizer` / `enableUBSanitizer`
/// attributes. Feeds `ResolveQuery::scheme_sanitizers`: xcodebuild forces the
/// matching `ENABLE_*_SANITIZER = YES` and suffixes the per-variant object
/// dirs (`-asan` / `-tsan` / `-ubsan`; see `OBJECT_FILE_DIR_<variant>` in
/// [`crate::project::built_in_settings`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SanitizerEnables {
    /// `enableAddressSanitizer="YES"`.
    pub address: bool,
    /// `enableThreadSanitizer="YES"`.
    pub thread: bool,
    /// `enableUBSanitizer="YES"`.
    pub undefined_behavior: bool,
}

impl SanitizerEnables {
    /// Whether any sanitizer is enabled.
    #[must_use]
    pub fn any(self) -> bool {
        self.address || self.thread || self.undefined_behavior
    }
}

/// One row in the scheme editor's Build phase.
// Five action flags ("buildForRunning", "buildForTesting", "buildForProfiling",
// "buildForArchiving", "buildForAnalyzing") mirror the scheme XML 1:1;
// collapsing them into a bitset would obscure the mapping for no benefit.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct BuildEntry {
    pub buildable: BuildableRef,
    pub for_running: bool,
    pub for_testing: bool,
    pub for_profiling: bool,
    pub for_archiving: bool,
    pub for_analyzing: bool,
}

/// Which scheme action a build is for. xcodebuild builds only the
/// `BuildActionEntry`s whose matching `buildFor*` flag is set: plain
/// `build` (and `-showBuildSettings` with no action) uses the Run set,
/// `build-for-testing` / `test` the Test set, and so on. The Alamofire
/// schemes are the corpus example — their test bundles carry
/// `buildForTesting="YES" buildForRunning="NO"` and xcodebuild's
/// `-showBuildSettings` output omits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildFor {
    Running,
    Testing,
    Profiling,
    Archiving,
    Analyzing,
}

impl BuildEntry {
    /// Whether this entry participates in a build for the given action.
    #[must_use]
    pub fn builds_for(&self, action: BuildFor) -> bool {
        match action {
            BuildFor::Running => self.for_running,
            BuildFor::Testing => self.for_testing,
            BuildFor::Profiling => self.for_profiling,
            BuildFor::Archiving => self.for_archiving,
            BuildFor::Analyzing => self.for_analyzing,
        }
    }
}

/// Identity of a target referenced by a scheme. The same shape is used in
/// build entries, testables, launch / profile macro expansions.
#[derive(Debug, Clone)]
pub struct BuildableRef {
    /// Target name, e.g. `Foo`. Matches `project::Target.name`.
    pub blueprint_name: String,
    /// Target's pbxproj UUID (24 hex chars). Useful when blueprint names
    /// collide across containers in a workspace.
    pub blueprint_identifier: String,
    /// Produced artifact's filename, e.g. `Foo.framework`.
    pub buildable_name: String,
    /// `ReferencedContainer="container:Foo.xcodeproj"` — the project that
    /// owns this target. Equals the current project for entries that
    /// resolve in a single-project [`crate::build_context::BuildContext`];
    /// references other containers in workspace schemes.
    pub container: String,
}

/// A `<CommandLineArgument>` under `LaunchAction.CommandLineArguments`.
#[derive(Debug, Clone)]
pub struct CommandLineArgument {
    /// The raw argument string. At launch Xcode expands the build settings
    /// it references, then splits it into words with shell-style quoting
    /// ([`split_launch_argument`]).
    pub argument: String,
    /// `isEnabled="NO"` unchecks the row; an absent attribute is enabled.
    pub is_enabled: bool,
}

/// An `<EnvironmentVariable>` under `LaunchAction.EnvironmentVariables`.
#[derive(Debug, Clone)]
pub struct EnvironmentVariable {
    pub key: String,
    /// `None` when the `value` attribute is absent (distinct from empty `""`);
    /// Xcode writes value-less rows for widget-preview placeholders, and
    /// launches with such a variable set to the empty string.
    pub value: Option<String>,
    /// `isEnabled="NO"` unchecks the row; an absent attribute is enabled.
    pub is_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct TestAction {
    pub configuration: String,
    pub testables: Vec<TestableRef>,
    /// `codeCoverageEnabled="YES"` — gathering coverage for this scheme's
    /// test action forces `CLANG_COVERAGE_MAPPING=YES` on every buildable
    /// xcodebuild resolves for the scheme (validated against the Kingfisher
    /// scheme, which sets it; its 12 build-settings captures all report YES).
    pub code_coverage_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct TestableRef {
    pub buildable: BuildableRef,
    /// `skipped="YES"` — Xcode lets you keep a testable in the scheme but
    /// flag it as not run.
    pub skipped: bool,
}

#[derive(Debug)]
pub enum Error {
    Parse(xcscheme::Error),
    BadScheme(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse(e) => write!(f, "{e}"),
            Error::BadScheme(s) => write!(f, "invalid scheme: {s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<xcscheme::Error> for Error {
    fn from(e: xcscheme::Error) -> Self {
        Error::Parse(e)
    }
}

/// Parse an `.xcscheme` file into a typed [`Scheme`].
pub fn parse_file(path: &Path) -> Result<Scheme, Error> {
    let root = xcscheme::parse_file(path)?;
    from_element(&root)
}

/// The login user whose `xcuserdata` Xcode would consult: the account's
/// name from the user database, as `xcodebuild` reads it, whatever `$USER`
/// says ([`crate::host::user`]). `None` when the process has no usable
/// identity (then we fall back to scanning every user's directory rather
/// than seeing no per-user schemes at all).
fn detected_user() -> Option<String> {
    crate::host::user()
}

/// Test-only: a username whose `xcuserdata` directory is visible through the
/// public scheme-discovery APIs on this host — the account's name when there
/// is one, any fixed name otherwise (no identity → every user dir is
/// scanned). Tests that create per-user scheme files use this so they pass
/// both on developer machines (where scoping applies) and in bare containers.
#[cfg(test)]
pub(crate) fn visible_user() -> String {
    detected_user().unwrap_or_else(|| "tester".into())
}

/// The directories a container (`.xcodeproj` or `.xcworkspace` — both share
/// the same layout) stores scheme files in: `xcshareddata/xcschemes` first,
/// then the per-user `xcuserdata/<user>.xcuserdatad/xcschemes`. Xcode and
/// xcodebuild only consult the *current* user's directory — a committed
/// `xcuserdata/alice.xcuserdatad` scheme is invisible to bob — so we scope to
/// the account's name when the identity is known, and scan every user
/// directory (sorted, for a stable order) only as a best-effort fallback when
/// it isn't.
fn scheme_dirs(container: &Path) -> Vec<PathBuf> {
    scheme_dirs_for_user(container, detected_user().as_deref())
}

fn scheme_dirs_for_user(container: &Path, user: Option<&str>) -> Vec<PathBuf> {
    let mut dirs = vec![container.join("xcshareddata/xcschemes")];
    if let Some(user) = user {
        dirs.push(container.join(format!("xcuserdata/{user}.xcuserdatad/xcschemes")));
    } else if let Ok(entries) = fs::read_dir(container.join("xcuserdata")) {
        let mut user_dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension() == Some(OsStr::new("xcuserdatad")))
            .map(|p| p.join("xcschemes"))
            .collect();
        user_dirs.sort();
        dirs.extend(user_dirs);
    }
    dirs
}

/// Whether Xcode's scheme autocreation is enabled for a container — the
/// `IDEWorkspaceSharedSettings_AutocreateContextsIfNeeded` key in the shared
/// `WorkspaceSettings.xcsettings` (XcodeGen / Tuist commonly write `false` so
/// generated projects don't sprout per-target schemes). Missing file or key
/// means enabled, matching Xcode's default. A `.xcodeproj` keeps the settings
/// inside its embedded `project.xcworkspace`; a `.xcworkspace` holds them
/// directly.
#[must_use]
pub fn autocreation_allowed(container: &Path) -> bool {
    let candidates = [
        container.join("xcshareddata/WorkspaceSettings.xcsettings"),
        container.join("project.xcworkspace/xcshareddata/WorkspaceSettings.xcsettings"),
    ];
    for path in candidates {
        let Ok(root) = xcscheme::parse_file(&path) else {
            continue;
        };
        // XML plist: <plist><dict><key>…</key><false/>…</dict></plist>.
        let Some(dict) = root.child("dict") else {
            continue;
        };
        let mut children = dict.children.iter();
        while let Some(child) = children.next() {
            if child.name == "key"
                && child.text == "IDEWorkspaceSharedSettings_AutocreateContextsIfNeeded"
            {
                return children.next().is_none_or(|v| v.name != "false");
            }
        }
    }
    true
}

/// Sort scheme names the way `xcodebuild -list` prints them:
/// case-insensitively, with the byte order as a tiebreak (NetNewsWire's
/// capture interleaves `NetNewsWire iOS …` between `NetNewsWire` and
/// `NetNewsWire Share Extension`, which only a case-insensitive sort
/// produces).
pub fn sort_like_xcodebuild(names: &mut [String]) {
    names.sort_by(|a, b| {
        a.to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b))
    });
}

/// Scheme names stored in a container: the shared schemes plus every user's
/// personal schemes, deduplicated and sorted alphabetically — the set
/// `xcodebuild -list` reports for the container.
#[must_use]
pub fn container_schemes(container: &Path) -> Vec<String> {
    let mut set = BTreeSet::new();
    for dir in scheme_dirs(container) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension() == Some(OsStr::new("xcscheme"))
                && let Some(name) = p.file_stem().and_then(OsStr::to_str)
            {
                set.insert(name.to_string());
            }
        }
    }
    set.into_iter().collect()
}

/// Locate `<name>.xcscheme` in a container: the shared directory first, then
/// each per-user directory (a shared scheme shadows a same-named user one,
/// matching Xcode). `None` when the scheme has no file — either it doesn't
/// exist or it's an autocreated scheme Xcode never materialized.
#[must_use]
pub fn find_scheme_file(container: &Path, name: &str) -> Option<PathBuf> {
    scheme_dirs(container)
        .into_iter()
        .map(|dir| dir.join(format!("{name}.xcscheme")))
        .find(|p| p.is_file())
}

/// The targets the scheme files stored in a container point at, which decide
/// the targets Xcode autocreates no scheme for.
///
/// Measured on Xcode 27.0 with `xcodebuild -list`: a target that runs (an
/// app, a tool, an extension) loses its autocreated scheme once a scheme with
/// another name runs it in its Launch or Profile action, and keeps it when
/// that scheme only builds it or names it as the test action's macro
/// expansion. A target that doesn't run (a framework, a library, a package's
/// library product) loses it once a scheme builds it. A workspace's scheme
/// files count for its member projects the same way.
#[derive(Debug, Clone, Default)]
pub struct SchemeReferences {
    /// Each scheme's Launch or Profile runnable, as the project its
    /// `ReferencedContainer` names and the target's name.
    runs: Vec<(Option<PathBuf>, String)>,
    /// Each scheme's Build action entries, the same way.
    builds: Vec<(Option<PathBuf>, String)>,
}

impl SchemeReferences {
    /// What the shared and the current user's scheme files in `container` (a
    /// `.xcodeproj`, a `.xcworkspace`, or a package's `.swiftpm/xcode`) point
    /// at. A `container:` reference resolves against the directory holding
    /// `container`; one that names no project (a package's `container:`)
    /// matches by target name alone.
    #[must_use]
    pub fn of(container: &Path) -> Self {
        let base = container.parent().unwrap_or(Path::new(""));
        let project_of = |reference: &BuildableRef| match reference.container.split_once(':') {
            Some(("container", "")) => None,
            Some(("container", rel)) => Some(crate::project::absolutize(&base.join(rel))),
            Some(("absolute", abs)) => Some(crate::project::absolutize(Path::new(abs))),
            _ => None,
        };
        let mut out = Self::default();
        for name in container_schemes(container) {
            let Some(root) =
                find_scheme_file(container, &name).and_then(|p| xcscheme::parse_file(&p).ok())
            else {
                continue;
            };
            for action in ["LaunchAction", "ProfileAction"] {
                if let Some(reference) = root
                    .child(action)
                    .and_then(|a| {
                        a.child("BuildableProductRunnable")
                            .or_else(|| a.child("RemoteRunnable"))
                    })
                    .and_then(|r| r.child("BuildableReference"))
                    .and_then(parse_buildable)
                {
                    out.runs
                        .push((project_of(&reference), reference.blueprint_name.clone()));
                }
            }
            let entries = root
                .child("BuildAction")
                .and_then(|b| b.child("BuildActionEntries"));
            for entry in entries
                .map(|e| e.children_named("BuildActionEntry").collect::<Vec<_>>())
                .unwrap_or_default()
            {
                if let Some(reference) = entry.child("BuildableReference").and_then(parse_buildable)
                {
                    out.builds
                        .push((project_of(&reference), reference.blueprint_name.clone()));
                }
            }
        }
        out
    }

    /// Whether a scheme takes the place of `target`'s autocreated one: a
    /// scheme that runs it, for a target that `runs`, or one that builds it,
    /// for one that doesn't. `project` is the target's `.xcodeproj`; `None`
    /// matches by name alone.
    #[must_use]
    pub fn cover(&self, project: Option<&Path>, target: &str, runs: bool) -> bool {
        let project = project.map(crate::project::absolutize);
        let references = if runs { &self.runs } else { &self.builds };
        references.iter().any(|(p, t)| {
            t == target
                && match (p, &project) {
                    (Some(p), Some(project)) => p == project,
                    _ => true,
                }
        })
    }
}

/// The file behind the scheme `name` that `xcodebuild` lists for `container`,
/// or `None` when it has none (an autocreated scheme Xcode never wrote, or a
/// name the container doesn't know). See [`locate_all`] for where it looks.
#[must_use]
pub fn locate(container: &Path, name: &str) -> Option<PathBuf> {
    scheme_containers(container)
        .flat_map(|c| scheme_dirs(&c))
        .map(|dir| dir.join(format!("{name}.xcscheme")))
        .find(|p| p.is_file())
}

/// Every file named for the scheme `name` among the scheme containers
/// `xcodebuild -list` reads for `container`, the one it uses first. Only the
/// current user's `xcuserdata` counts, as it does for Xcode.
///
/// - A `.xcworkspace` (a project's embedded one included): its own schemes,
///   then each member project's, then each local package's `.swiftpm/xcode`,
///   the workspace's own package members before the ones its projects declare.
/// - A `.xcodeproj`: its own, then the `.swiftpm/xcode` of each local package
///   it declares.
/// - A Swift package, named by its `Package.swift` or its directory: its
///   `.swiftpm/xcode`.
///
/// A package reached only through another package's `.package(path:)` is not
/// looked in: only its manifest names it.
#[must_use]
pub fn locate_all(container: &Path, name: &str) -> Vec<PathBuf> {
    scheme_containers(container)
        .flat_map(|c| scheme_dirs(&c))
        .map(|dir| dir.join(format!("{name}.xcscheme")))
        .filter(|p| p.is_file())
        .collect()
}

/// The directories holding `xcshareddata`/`xcuserdata` scheme folders for
/// `container`, in lookup order. The container's own comes first and costs
/// nothing to name; the members behind it are read only when a lookup gets
/// that far.
fn scheme_containers(container: &Path) -> impl Iterator<Item = PathBuf> {
    let own = if container.file_name() == Some(OsStr::new("Package.swift")) {
        crate::workspace::package_scheme_root(container.parent().unwrap_or(Path::new(".")))
    } else if matches!(
        container.extension().and_then(OsStr::to_str),
        Some("xcworkspace" | "xcodeproj")
    ) {
        container.to_path_buf()
    } else {
        crate::workspace::package_scheme_root(container)
    };
    let container = container.to_path_buf();
    std::iter::once(own).chain(std::iter::once_with(move || members(&container)).flatten())
}

/// The scheme containers behind `container`'s own: a workspace's member
/// projects and local packages, a project's local packages.
fn members(container: &Path) -> Vec<PathBuf> {
    match container.extension().and_then(OsStr::to_str) {
        Some("xcworkspace") => crate::workspace::open(container)
            .map(|ws| {
                let packages = ws
                    .package_refs
                    .iter()
                    .cloned()
                    .chain(ws.project_package_refs())
                    .map(|dir| crate::workspace::package_scheme_root(&dir));
                ws.project_refs.iter().cloned().chain(packages).collect()
            })
            .unwrap_or_default(),
        Some("xcodeproj") => crate::project::open(container)
            .map(|project| {
                project
                    .package_refs
                    .iter()
                    .map(|dir| crate::workspace::package_scheme_root(dir))
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Build a [`Scheme`] from an already-parsed `<Scheme>` element.
pub fn from_element(root: &Element) -> Result<Scheme, Error> {
    if root.name != "Scheme" {
        return Err(Error::BadScheme(format!(
            "expected root element <Scheme>, got <{}>",
            root.name
        )));
    }

    let build_action = root.child("BuildAction");
    let build_entries = build_action
        .and_then(|b| b.child("BuildActionEntries"))
        .map(|entries| {
            entries
                .children_named("BuildActionEntry")
                .filter_map(parse_build_entry)
                .collect()
        })
        .unwrap_or_default();
    let build_implicit_dependencies = build_action
        .and_then(|b| b.attr("buildImplicitDependencies"))
        .is_none_or(parse_yes);
    let parallelize_buildables = build_action
        .and_then(|b| b.attr("parallelizeBuildables"))
        .is_none_or(parse_yes);

    let test_action = root.child("TestAction").map(parse_test_action);
    let launch_action = root.child("LaunchAction");

    Ok(Scheme {
        build_entries,
        build_implicit_dependencies,
        parallelize_buildables,
        test_action,
        // Watch-app schemes launch via `<RemoteRunnable>` (the watch extension
        // runs on the paired device) instead of `<BuildableProductRunnable>`;
        // both wrap the same `BuildableReference` shape.
        launch_target: launch_action
            .and_then(|a| {
                a.child("BuildableProductRunnable")
                    .or_else(|| a.child("RemoteRunnable"))
            })
            .and_then(|r| r.child("BuildableReference"))
            .and_then(parse_buildable),
        launch_configuration: launch_action
            .and_then(|a| a.attr("buildConfiguration"))
            .map(str::to_string),
        launch_macro_expansion: launch_action
            .and_then(|a| a.child("MacroExpansion"))
            .and_then(|m| m.child("BuildableReference"))
            .and_then(parse_buildable),
        launch_arguments: launch_action
            .map(parse_command_line_arguments)
            .unwrap_or_default(),
        launch_environment_variables: launch_action
            .map(parse_environment_variables)
            .unwrap_or_default(),
        launch_language: launch_action
            .and_then(|a| a.attr("language"))
            .map(str::to_string),
        launch_region: launch_action
            .and_then(|a| a.attr("region"))
            .map(str::to_string),
        launch_sanitizers: SanitizerEnables {
            address: launch_action
                .and_then(|a| a.attr("enableAddressSanitizer"))
                .is_some_and(parse_yes),
            thread: launch_action
                .and_then(|a| a.attr("enableThreadSanitizer"))
                .is_some_and(parse_yes),
            undefined_behavior: launch_action
                .and_then(|a| a.attr("enableUBSanitizer"))
                .is_some_and(parse_yes),
        },
        profile_configuration: action_configuration(root, "ProfileAction"),
        archive_configuration: action_configuration(root, "ArchiveAction"),
        analyze_configuration: action_configuration(root, "AnalyzeAction"),
    })
}

fn action_configuration(root: &Element, action: &str) -> Option<String> {
    root.child(action)
        .and_then(|a| a.attr("buildConfiguration"))
        .map(str::to_string)
}

fn parse_build_entry(entry: &Element) -> Option<BuildEntry> {
    let buildable = entry
        .child("BuildableReference")
        .and_then(parse_buildable)?;
    Some(BuildEntry {
        buildable,
        for_running: entry.attr("buildForRunning").is_none_or(parse_yes),
        for_testing: entry.attr("buildForTesting").is_none_or(parse_yes),
        for_profiling: entry.attr("buildForProfiling").is_none_or(parse_yes),
        for_archiving: entry.attr("buildForArchiving").is_none_or(parse_yes),
        for_analyzing: entry.attr("buildForAnalyzing").is_none_or(parse_yes),
    })
}

fn parse_buildable(b: &Element) -> Option<BuildableRef> {
    Some(BuildableRef {
        blueprint_name: b.attr("BlueprintName")?.to_string(),
        blueprint_identifier: b.attr("BlueprintIdentifier").unwrap_or("").to_string(),
        buildable_name: b.attr("BuildableName").unwrap_or("").to_string(),
        container: b.attr("ReferencedContainer").unwrap_or("").to_string(),
    })
}

fn parse_test_action(action: &Element) -> TestAction {
    let configuration = action
        .attr("buildConfiguration")
        .unwrap_or("Debug")
        .to_string();
    let testables = action
        .child("Testables")
        .map(|t| {
            t.children_named("TestableReference")
                .filter_map(parse_testable)
                .collect()
        })
        .unwrap_or_default();
    TestAction {
        configuration,
        testables,
        code_coverage_enabled: action.attr("codeCoverageEnabled").is_some_and(parse_yes),
    }
}

fn parse_testable(t: &Element) -> Option<TestableRef> {
    let buildable = t.child("BuildableReference").and_then(parse_buildable)?;
    Some(TestableRef {
        buildable,
        skipped: t.attr("skipped").is_some_and(parse_yes),
    })
}

fn parse_command_line_arguments(action: &Element) -> Vec<CommandLineArgument> {
    action
        .child("CommandLineArguments")
        .map(|c| {
            c.children_named("CommandLineArgument")
                .filter_map(|a| {
                    Some(CommandLineArgument {
                        argument: a.attr("argument")?.to_string(),
                        is_enabled: a.attr("isEnabled").is_none_or(parse_yes),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_environment_variables(action: &Element) -> Vec<EnvironmentVariable> {
    action
        .child("EnvironmentVariables")
        .map(|c| {
            c.children_named("EnvironmentVariable")
                .filter_map(|v| {
                    Some(EnvironmentVariable {
                        key: v.attr("key")?.to_string(),
                        value: v.attr("value").map(str::to_string),
                        is_enabled: v.attr("isEnabled").is_none_or(parse_yes),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `.xcscheme` attributes are `YES` / `NO` strings.
fn parse_yes(v: &str) -> bool {
    v.eq_ignore_ascii_case("YES")
}

/// What a scheme's Run action launches its app with: the process arguments
/// and environment Xcode passes, in the order it passes them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchSettings {
    /// The enabled argument rows, expanded and split into words, then the
    /// App Language and App Region flags.
    pub args: Vec<String>,
    /// The enabled environment rows with their values expanded, one entry per
    /// key: a later row for a key replaces the value of an earlier one.
    pub env: Vec<(String, String)>,
}

impl Scheme {
    /// The target whose resolved build settings expand `$(VAR)` in the launch
    /// arguments and environment: the Run action's `MacroExpansion`, or else
    /// the target it launches.
    #[must_use]
    pub fn launch_expansion_target(&self) -> Option<&BuildableRef> {
        self.launch_macro_expansion
            .as_ref()
            .or(self.launch_target.as_ref())
    }

    /// Whether an enabled launch argument or environment value refers to a
    /// build setting, so [`Self::launch_settings`] needs the resolved settings
    /// of [`Self::launch_expansion_target`]. A caller can skip resolving them
    /// when this is false.
    #[must_use]
    pub fn launch_references_settings(&self) -> bool {
        self.launch_arguments
            .iter()
            .any(|a| a.is_enabled && a.argument.contains('$'))
            || self
                .launch_environment_variables
                .iter()
                .any(|v| v.is_enabled && v.value.as_deref().is_some_and(|v| v.contains('$')))
    }

    /// The arguments and environment Xcode launches this scheme's app with.
    /// Checked against `xcodebuild test`, which launches with the Run
    /// action's rows when the Test action shares them:
    ///
    /// - disabled rows are left out;
    /// - `$(VAR)`, `${VAR}` and `$VAR` expand against `settings`, the resolved
    ///   build settings of [`Self::launch_expansion_target`]. An undefined
    ///   `$(VAR)` expands to nothing, an undefined bare `$VAR` stays as
    ///   written, and `$$` is a literal `$`;
    /// - each argument row is split into words after expansion
    ///   ([`split_launch_argument`]), so a setting whose value holds a space
    ///   becomes two arguments unless the row quotes it;
    /// - an environment value is used as written once expanded, quotes
    ///   included. A row with no value sets the variable to the empty string.
    ///   Keys are not expanded;
    /// - App Language adds `-AppleLanguages (<language>)` and
    ///   `-AppleTextDirection YES` or `NO`, and App Region adds `-AppleLocale
    ///   <language>_<region>`. With a region and no language, the language is
    ///   the host's, from `host_language`, which is called only then.
    pub fn launch_settings(
        &self,
        settings: &BTreeMap<String, String>,
        host_language: impl FnOnce() -> Option<String>,
    ) -> LaunchSettings {
        let mut args = Vec::new();
        for row in self.launch_arguments.iter().filter(|a| a.is_enabled) {
            args.extend(split_launch_argument(&expand_one(&row.argument, settings)));
        }
        let language = self.launch_language.as_deref().filter(|l| !l.is_empty());
        if let Some(language) = language {
            args.push("-AppleLanguages".into());
            args.push(format!("({language})"));
            args.push("-AppleTextDirection".into());
            args.push(
                if is_right_to_left(language) {
                    "YES"
                } else {
                    "NO"
                }
                .into(),
            );
        }
        if let Some(region) = self.launch_region.as_deref().filter(|r| !r.is_empty())
            && let Some(language) = language.map(str::to_string).or_else(host_language)
        {
            args.push("-AppleLocale".into());
            args.push(format!("{language}_{region}"));
        }

        let mut env: Vec<(String, String)> = Vec::new();
        for row in self
            .launch_environment_variables
            .iter()
            .filter(|v| v.is_enabled && !v.key.is_empty())
        {
            let value = row
                .value
                .as_deref()
                .map(|v| expand_one(v, settings))
                .unwrap_or_default();
            match env.iter_mut().find(|(key, _)| *key == row.key) {
                Some(slot) => slot.1 = value,
                None => env.push((row.key.clone(), value)),
            }
        }
        LaunchSettings { args, env }
    }
}

/// Split one launch-argument row into process arguments the way Xcode does:
/// at unquoted whitespace, with shell-style quoting. Single quotes keep
/// everything up to the next `'` as written. Outside them, a backslash takes
/// the next character literally, inside double quotes too. Quoted text joins
/// the text around it (`a"b c"d` is `ab cd`), `""` is an empty argument, and
/// an unclosed quote runs to the end of the row.
#[must_use]
pub fn split_launch_argument(row: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = row.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                word.extend(chars.by_ref().take_while(|&c| c != '\''));
            }
            '"' => {
                in_word = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => word.extend(chars.next()),
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.extend(chars.next());
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// Whether Foundation lays `language` out right to left, which is the
/// `-AppleTextDirection` Xcode passes with App Language. Mirrors
/// `NSLocale.characterDirection(forLanguage:)` as `xcodebuild` applies it: a
/// right-to-left base language (`he`, `ar-SA`), or a script-qualified tag
/// whose locale data is right to left (`pa-Arab`). A left-to-right script
/// overrides a right-to-left base (`sd-Deva`), and a right-to-left script on
/// a language with no such locale data does not (`az-Arab`).
fn is_right_to_left(language: &str) -> bool {
    const LANGUAGES: &[&str] = &[
        "ar", "ckb", "dv", "fa", "he", "iw", "ks", "lrc", "mzn", "nqo", "ps", "rhg", "sd", "syr",
        "ug", "ur", "yi",
    ];
    const SCRIPTED: &[&str] = &[
        "ff-adlm", "ks-arab", "ms-arab", "pa-arab", "sd-arab", "uz-arab",
    ];
    const SCRIPTS: &[&str] = &["adlm", "arab", "hebr", "nkoo", "rohg", "syrc", "thaa"];
    let tag = language.to_ascii_lowercase().replace('_', "-");
    let mut parts = tag.split('-');
    let base = parts.next().unwrap_or_default();
    let script = parts
        .next()
        .filter(|p| p.len() == 4 && p.bytes().all(|b| b.is_ascii_alphabetic()));
    match script {
        Some(script) if SCRIPTS.contains(&script) => {
            SCRIPTED.contains(&format!("{base}-{script}").as_str()) || LANGUAGES.contains(&base)
        }
        Some(_) => false,
        None => LANGUAGES.contains(&base),
    }
}

/// The host's language, which Xcode pairs with a scheme's App Region when the
/// scheme sets no App Language: the language part of the user's
/// `AppleLocale` default (`en` for `en_UA`). `None` when it can't be read.
#[must_use]
pub fn host_language() -> Option<String> {
    let out = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let locale = String::from_utf8(out.stdout).ok()?;
    let language = locale.trim().split(['_', '@']).next()?.trim();
    (!language.is_empty()).then(|| language.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;
    use std::path::PathBuf;

    fn fixtures_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    #[test]
    fn parses_kingfisher_scheme() {
        let path = fixtures_root()
            .join("kingfisher/xcode-26.5.0/raw/Kingfisher.xcodeproj/xcshareddata/xcschemes/Kingfisher.xcscheme");
        let scheme = parse_file(&path).unwrap();

        assert_eq!(scheme.build_entries.len(), 1);
        let entry = &scheme.build_entries[0];
        assert_eq!(entry.buildable.blueprint_name, "Kingfisher");
        assert_eq!(entry.buildable.buildable_name, "Kingfisher.framework");
        assert_eq!(entry.buildable.container, "container:Kingfisher.xcodeproj");
        assert!(entry.for_running);
        assert!(entry.for_testing);

        assert!(scheme.build_implicit_dependencies);
        assert!(scheme.parallelize_buildables);

        let ta = scheme.test_action.as_ref().unwrap();
        assert_eq!(ta.configuration, "Debug");
        assert_eq!(ta.testables.len(), 1);
        assert_eq!(ta.testables[0].buildable.blueprint_name, "KingfisherTests");
        assert!(!ta.testables[0].skipped);
        assert!(ta.code_coverage_enabled);

        assert_eq!(scheme.launch_configuration.as_deref(), Some("Debug"));
        assert_eq!(scheme.archive_configuration.as_deref(), Some("Release"));
        // Kingfisher is a framework: its LaunchAction carries a `MacroExpansion`,
        // not a `BuildableProductRunnable`, so there's no launchable target.
        assert!(scheme.launch_target.is_none());
    }

    #[test]
    fn parses_share_extension_scheme_with_multiple_entries() {
        let path = fixtures_root().join(
            "ice-cubes/xcode-26.5.0/raw/IceCubesApp.xcodeproj/xcshareddata/xcschemes/IceCubesShareExtension.xcscheme",
        );
        let scheme = parse_file(&path).unwrap();

        let names: Vec<&str> = scheme
            .build_entries
            .iter()
            .map(|e| e.buildable.blueprint_name.as_str())
            .collect();
        assert_eq!(names, vec!["IceCubesShareExtension", "IceCubesApp"]);

        // The scheme builds the extension *first*, but its LaunchAction launches
        // the host app `IceCubesApp` — so `buildEntries[].for_running` (which
        // would pick the extension) is the wrong signal; `launch_target` is
        // authoritative.
        assert_eq!(
            scheme
                .launch_target
                .as_ref()
                .map(|b| b.blueprint_name.as_str()),
            Some("IceCubesApp")
        );
    }

    #[test]
    fn parses_launch_environment_variables() {
        let path = fixtures_root().join(
            "alamofire/xcode-26.5.0/raw/Example/iOS Example.xcodeproj/xcshareddata/xcschemes/iOS Example.xcscheme",
        );
        let scheme = parse_file(&path).unwrap();

        assert_eq!(
            scheme
                .launch_target
                .as_ref()
                .map(|b| b.blueprint_name.as_str()),
            Some("iOS Example")
        );
        // One environment variable, unchecked (`isEnabled="NO"`). The parser
        // keeps disabled rows; the extension filters them at launch.
        assert_eq!(scheme.launch_environment_variables.len(), 1);
        let ev = &scheme.launch_environment_variables[0];
        assert_eq!(ev.key, "OS_ACTIVITY_MODE");
        assert_eq!(ev.value.as_deref(), Some("disable"));
        assert!(!ev.is_enabled);
        // No `<CommandLineArguments>`; `<AdditionalOptions>` is not an
        // environment source and is ignored.
        assert!(scheme.launch_arguments.is_empty());
        assert!(scheme.launch_language.is_none());
        assert!(scheme.launch_region.is_none());
    }

    /// A scheme whose Run action carries `inner` (argument and environment
    /// rows) and the given `LaunchAction` attributes.
    fn launch_scheme(attrs: &str, inner: &str) -> Scheme {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme version="1.7">
   <LaunchAction buildConfiguration="Debug" {attrs}>
      <BuildableProductRunnable>
         <BuildableReference BlueprintIdentifier="A1" BuildableName="App.app"
            BlueprintName="App" ReferencedContainer="container:App.xcodeproj"/>
      </BuildableProductRunnable>
      {inner}
   </LaunchAction>
</Scheme>"#
        );
        from_element(&xcscheme::parse(&xml).unwrap()).unwrap()
    }

    fn arg(argument: &str, enabled: bool) -> String {
        let argument = argument.replace('"', "&quot;");
        let enabled = if enabled { "YES" } else { "NO" };
        format!(r#"<CommandLineArgument argument="{argument}" isEnabled="{enabled}"/>"#)
    }

    fn settings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn no_host() -> Option<String> {
        panic!("the host language is read only for a region without a language")
    }

    /// The rows and results of an `xcodebuild test` run (Xcode 27.0) whose
    /// Test action shares the Run action's arguments and environment.
    #[test]
    fn launch_settings_expand_then_split_like_xcodebuild() {
        let rows = [
            arg("-Plain YES", true),
            arg("-Disabled YES", false),
            arg(r#"-Quoted "a b" 'c d' e\ f"#, true),
            arg(
                "-Expand $(PRODUCT_NAME) ${TARGET_NAME} $(SRCROOT)/x $(NOT_SET)| $PRODUCT_NAME",
                true,
            ),
            arg(
                r#"-Spacey $(SPACEY) $NOT_SET_BARE $(PRODUCT_NAME:lower) $$(PRODUCT_NAME) "$(SPACEY)""#,
                true,
            ),
            arg(
                r#"-Edge a"b c"d "x\"y" 'it''s' "" 'single\q' "dbl\q" tab	sep $(QUOTED) "abc def"#,
                true,
            ),
        ]
        .concat();
        let scheme = launch_scheme(
            "",
            &format!("<CommandLineArguments>{rows}</CommandLineArguments>"),
        );
        let settings = settings(&[
            ("PRODUCT_NAME", "App"),
            ("TARGET_NAME", "App"),
            ("SRCROOT", "/src"),
            ("SPACEY", "p q"),
            ("QUOTED", r#""u v""#),
        ]);
        assert!(scheme.launch_references_settings());
        assert_eq!(
            scheme.launch_settings(&settings, no_host).args,
            [
                "-Plain",
                "YES",
                "-Quoted",
                "a b",
                "c d",
                "e f",
                "-Expand",
                "App",
                "App",
                "/src/x",
                "|",
                "App",
                "-Spacey",
                "p",
                "q",
                "$NOT_SET_BARE",
                "app",
                "$(PRODUCT_NAME)",
                "p q",
                "-Edge",
                "ab cd",
                "x\"y",
                "its",
                "",
                r"single\q",
                "dblq",
                "tab",
                "sep",
                "u v",
                "abc def",
            ]
        );
    }

    #[test]
    fn launch_environment_expands_values_and_keeps_the_last_row_per_key() {
        let scheme = launch_scheme(
            "",
            r#"<EnvironmentVariables>
                <EnvironmentVariable key="PLAIN" value="hello world" isEnabled="YES"/>
                <EnvironmentVariable key="OFF" value="x" isEnabled="NO"/>
                <EnvironmentVariable key="NOVALUE" isEnabled="YES"/>
                <EnvironmentVariable key="QUOTES" value="&quot;q r&quot; 's'" isEnabled="YES"/>
                <EnvironmentVariable key="DUP" value="first" isEnabled="YES"/>
                <EnvironmentVariable key="K_$(PRODUCT_NAME)" value="k" isEnabled="YES"/>
                <EnvironmentVariable key="DUP" value="second" isEnabled="YES"/>
                <EnvironmentVariable key="EXPAND" value="$(PRODUCT_NAME)|$(NOT_SET)|${CONFIGURATION}"/>
            </EnvironmentVariables>"#,
        );
        let settings = settings(&[("PRODUCT_NAME", "App"), ("CONFIGURATION", "Debug")]);
        let env = scheme.launch_settings(&settings, no_host).env;
        let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(
            pairs,
            [
                ("PLAIN", "hello world"),
                ("NOVALUE", ""),
                ("QUOTES", r#""q r" 's'"#),
                ("DUP", "second"),
                ("K_$(PRODUCT_NAME)", "k"),
                ("EXPAND", "App||Debug"),
            ]
        );
    }

    #[test]
    fn launch_settings_skip_settings_when_nothing_refers_to_them() {
        let scheme = launch_scheme(
            "",
            &format!(
                r#"<CommandLineArguments>{}{}</CommandLineArguments>
                <EnvironmentVariables>
                   <EnvironmentVariable key="A" value="plain" isEnabled="YES"/>
                   <EnvironmentVariable key="B" value="$(OFF)" isEnabled="NO"/>
                </EnvironmentVariables>"#,
                arg("-Flag YES", true),
                arg("-Off $(SRCROOT)", false),
            ),
        );
        assert!(!scheme.launch_references_settings());
        let launch = scheme.launch_settings(&BTreeMap::new(), no_host);
        assert_eq!(launch.args, ["-Flag", "YES"]);
        assert_eq!(launch.env, [("A".to_string(), "plain".to_string())]);
    }

    /// App Language and App Region as `xcodebuild` passes them.
    #[test]
    fn launch_language_and_region_flags() {
        let flags = |attrs: &str| {
            launch_scheme(attrs, "")
                .launch_settings(&BTreeMap::new(), || Some("en".into()))
                .args
        };
        assert_eq!(
            flags(r#"language="he" region="IL""#),
            [
                "-AppleLanguages",
                "(he)",
                "-AppleTextDirection",
                "YES",
                "-AppleLocale",
                "he_IL"
            ]
        );
        assert_eq!(
            flags(r#"language="zh-Hans" region="CN""#),
            [
                "-AppleLanguages",
                "(zh-Hans)",
                "-AppleTextDirection",
                "NO",
                "-AppleLocale",
                "zh-Hans_CN"
            ]
        );
        assert_eq!(
            flags(r#"language="ar""#),
            ["-AppleLanguages", "(ar)", "-AppleTextDirection", "YES"]
        );
        // A region alone pairs with the host's language.
        assert_eq!(flags(r#"region="JP""#), ["-AppleLocale", "en_JP"]);
        let unknown =
            launch_scheme(r#"region="JP""#, "").launch_settings(&BTreeMap::new(), || None);
        assert!(unknown.args.is_empty());
        // The arguments rows come first.
        let scheme = launch_scheme(
            r#"language="fr""#,
            &format!(
                "<CommandLineArguments>{}</CommandLineArguments>",
                arg("-X 1", true)
            ),
        );
        assert_eq!(
            scheme.launch_settings(&BTreeMap::new(), no_host).args,
            [
                "-X",
                "1",
                "-AppleLanguages",
                "(fr)",
                "-AppleTextDirection",
                "NO"
            ]
        );
    }

    /// The text direction `xcodebuild` reported for each language.
    #[test]
    fn right_to_left_languages_match_xcodebuild() {
        for rtl in [
            "he", "he-IL", "iw", "ar", "ar-SA", "fa", "ur", "yi", "ckb", "dv", "ps", "ug", "sd",
            "ks", "mzn", "lrc", "syr", "nqo", "rhg", "pa-Arab", "uz-Arab", "ms-Arab", "ff-Adlm",
        ] {
            assert!(is_right_to_left(rtl), "{rtl}");
        }
        for ltr in [
            "fr",
            "en-GB",
            "zh-Hans",
            "ku",
            "arc",
            "az-Arab",
            "sd-Deva",
            "ks-Deva",
            "IDELaunchRTLPseudoLanguage",
        ] {
            assert!(!is_right_to_left(ltr), "{ltr}");
        }
    }

    #[test]
    fn launch_expansion_target_prefers_the_macro_expansion() {
        let scheme = launch_scheme("", "");
        assert_eq!(
            scheme
                .launch_expansion_target()
                .map(|b| b.blueprint_name.as_str()),
            Some("App")
        );
        let scheme = launch_scheme(
            "",
            r#"<MacroExpansion>
                  <BuildableReference BlueprintIdentifier="B2" BuildableName="Other.app"
                     BlueprintName="Other" ReferencedContainer="container:App.xcodeproj"/>
               </MacroExpansion>"#,
        );
        assert_eq!(
            scheme
                .launch_expansion_target()
                .map(|b| b.blueprint_name.as_str()),
            Some("Other")
        );
    }

    #[test]
    fn rejects_non_scheme_root() {
        let element = Element {
            name: "NotAScheme".into(),
            attributes: Vec::new(),
            children: Vec::new(),
            text: String::new(),
        };
        let err = from_element(&element).unwrap_err();
        assert!(format!("{err}").contains("expected root element"));
    }

    /// A scratch container in a directory of its own under the OS temp dir,
    /// which goes when the returned guard drops.
    fn scratch_container(tag: &str) -> (TempDir, PathBuf) {
        let root = TempDir::new(&format!("sweetpad-scheme-{tag}"));
        let dir = root.join("App.xcodeproj");
        std::fs::create_dir_all(&dir).unwrap();
        (root, dir)
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn container_schemes_merges_shared_and_user_schemes() {
        let (_root, dir) = scratch_container("merge");
        let user = visible_user();
        touch(&dir.join("xcshareddata/xcschemes/Shared.xcscheme"));
        touch(&dir.join(format!(
            "xcuserdata/{user}.xcuserdatad/xcschemes/Personal.xcscheme"
        )));
        // Duplicate of the shared scheme in the user dir collapses to one.
        touch(&dir.join(format!(
            "xcuserdata/{user}.xcuserdatad/xcschemes/Shared.xcscheme"
        )));
        assert_eq!(container_schemes(&dir), vec!["Personal", "Shared"]);
    }

    #[test]
    fn container_schemes_empty_without_scheme_files() {
        let (_root, dir) = scratch_container("empty");
        assert!(container_schemes(&dir).is_empty());
    }

    #[test]
    fn scheme_dirs_scope_to_the_known_user() {
        let (_root, dir) = scratch_container("user-scope");
        touch(&dir.join("xcshareddata/xcschemes/Shared.xcscheme"));
        touch(&dir.join("xcuserdata/alice.xcuserdatad/xcschemes/Mine.xcscheme"));
        touch(&dir.join("xcuserdata/bob.xcuserdatad/xcschemes/Foreign.xcscheme"));

        // With a known identity, only that user's directory is consulted —
        // xcodebuild never sees another user's committed schemes.
        let dirs = scheme_dirs_for_user(&dir, Some("alice"));
        assert_eq!(
            dirs,
            vec![
                dir.join("xcshareddata/xcschemes"),
                dir.join("xcuserdata/alice.xcuserdatad/xcschemes"),
            ]
        );
        // Unknown identity: best-effort scan of every user dir.
        let dirs = scheme_dirs_for_user(&dir, None);
        assert_eq!(dirs.len(), 3);
    }

    #[test]
    fn autocreation_allowed_honors_workspace_settings() {
        // Default: no settings file → enabled.
        let (_root, dir) = scratch_container("autocreate-default");
        assert!(autocreation_allowed(&dir));

        // Workspace-style container with the key set to false → disabled.
        let plist = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
            <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
            <plist version=\"1.0\">\n<dict>\n\
            \t<key>IDEWorkspaceSharedSettings_AutocreateContextsIfNeeded</key>\n\
            \t<false/>\n</dict>\n</plist>\n";
        let (_ws_root, ws) = scratch_container("autocreate-off");
        std::fs::create_dir_all(ws.join("xcshareddata")).unwrap();
        std::fs::write(ws.join("xcshareddata/WorkspaceSettings.xcsettings"), plist).unwrap();
        assert!(!autocreation_allowed(&ws));

        // Project-style container (settings inside the embedded workspace),
        // key explicitly true → enabled.
        let (_proj_root, proj) = scratch_container("autocreate-on");
        let inner = proj.join("project.xcworkspace/xcshareddata");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(
            inner.join("WorkspaceSettings.xcsettings"),
            plist.replace("<false/>", "<true/>"),
        )
        .unwrap();
        assert!(autocreation_allowed(&proj));
    }

    #[test]
    fn build_for_flags_gate_entries() {
        let entry = BuildEntry {
            buildable: BuildableRef {
                blueprint_name: "T".into(),
                blueprint_identifier: String::new(),
                buildable_name: "T.xctest".into(),
                container: String::new(),
            },
            for_running: false,
            for_testing: true,
            for_profiling: false,
            for_archiving: false,
            for_analyzing: false,
        };
        assert!(!entry.builds_for(BuildFor::Running));
        assert!(entry.builds_for(BuildFor::Testing));
        assert!(!entry.builds_for(BuildFor::Archiving));
    }

    #[test]
    fn find_scheme_file_prefers_shared_over_user() {
        let (_root, dir) = scratch_container("find");
        let user = visible_user();
        let shared = dir.join("xcshareddata/xcschemes/App.xcscheme");
        touch(&shared);
        touch(&dir.join(format!(
            "xcuserdata/{user}.xcuserdatad/xcschemes/App.xcscheme"
        )));
        let user_only = dir.join(format!(
            "xcuserdata/{user}.xcuserdatad/xcschemes/Mine.xcscheme"
        ));
        touch(&user_only);

        assert_eq!(find_scheme_file(&dir, "App"), Some(shared));
        assert_eq!(find_scheme_file(&dir, "Mine"), Some(user_only));
        assert_eq!(find_scheme_file(&dir, "Nope"), None);
    }

    /// A workspace holding two projects and a local package, each with
    /// scheme files of its own.
    fn scratch_workspace(tag: &str) -> (TempDir, PathBuf) {
        let root = TempDir::new(&format!("sweetpad-scheme-{tag}"));
        let ws = root.join("App.xcworkspace");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(
            ws.join("contents.xcworkspacedata"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Workspace version = "1.0">
   <FileRef location = "group:A.xcodeproj"></FileRef>
   <FileRef location = "group:B.xcodeproj"></FileRef>
   <FileRef location = "group:Pkg"></FileRef>
</Workspace>
"#,
        )
        .unwrap();
        for dir in ["A.xcodeproj", "B.xcodeproj", "Pkg"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        touch(&root.join("Pkg/Package.swift"));
        (root, ws)
    }

    /// `xcodebuild -list -workspace` reads the workspace's schemes, then each
    /// member's, then each local package's `.swiftpm/xcode`; a lookup by name
    /// follows the same order and never leaves the workspace it was given.
    #[test]
    fn locate_searches_the_workspace_then_its_members_then_its_packages() {
        let (root, ws) = scratch_workspace("locate-ws");
        let own = ws.join("xcshareddata/xcschemes/Shared.xcscheme");
        touch(&own);
        touch(&root.join("A.xcodeproj/xcshareddata/xcschemes/Shared.xcscheme"));
        let in_b = root.join("B.xcodeproj/xcshareddata/xcschemes/OnlyInB.xcscheme");
        touch(&in_b);
        let in_package = root.join("Pkg/.swiftpm/xcode/xcshareddata/xcschemes/Custom.xcscheme");
        touch(&in_package);
        // A same-named scheme in a project the workspace doesn't list.
        touch(&root.join("Other.xcodeproj/xcshareddata/xcschemes/Stray.xcscheme"));

        assert_eq!(locate(&ws, "Shared"), Some(own.clone()));
        assert_eq!(
            locate_all(&ws, "Shared"),
            [
                own,
                root.join("A.xcodeproj/xcshareddata/xcschemes/Shared.xcscheme")
            ]
        );
        assert_eq!(locate(&ws, "OnlyInB"), Some(in_b));
        assert_eq!(locate(&ws, "Custom"), Some(in_package));
        assert_eq!(locate(&ws, "Stray"), None);
    }

    /// A package is named by its manifest or its directory, and keeps its
    /// schemes in `.swiftpm/xcode`.
    #[test]
    fn locate_reads_a_packages_swiftpm_container() {
        let root = TempDir::new("sweetpad-scheme-locate-package");
        let scheme = root.join("Pkg/.swiftpm/xcode/xcshareddata/xcschemes/Custom.xcscheme");
        touch(&scheme);
        assert_eq!(
            locate(&root.join("Pkg/Package.swift"), "Custom"),
            Some(scheme.clone())
        );
        assert_eq!(locate(&root.join("Pkg"), "Custom"), Some(scheme));
    }

    /// Xcode never shows one user another user's personal schemes, so a
    /// lookup doesn't either.
    #[test]
    fn locate_skips_another_users_schemes() {
        let Some(user) = detected_user() else {
            eprintln!("skipping: needs a $USER to scope per-user schemes to");
            return;
        };
        let (_root, dir) = scratch_container("locate-user");
        touch(&dir.join("xcuserdata/someone-else.xcuserdatad/xcschemes/Foreign.xcscheme"));
        let mine = dir.join(format!(
            "xcuserdata/{user}.xcuserdatad/xcschemes/Mine.xcscheme"
        ));
        touch(&mine);
        assert_eq!(locate(&dir, "Foreign"), None);
        assert_eq!(locate(&dir, "Mine"), Some(mine));
    }

    /// A scheme building `Kit` and running `App`, both in `App.xcodeproj`.
    const COVERING_SCHEME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme version = "1.7">
   <BuildAction>
      <BuildActionEntries>
         <BuildActionEntry buildForRunning = "YES">
            <BuildableReference BuildableIdentifier = "primary" BlueprintIdentifier = "KIT"
               BuildableName = "Kit.framework" BlueprintName = "Kit" ReferencedContainer = "container:App.xcodeproj">
            </BuildableReference>
         </BuildActionEntry>
      </BuildActionEntries>
   </BuildAction>
   <LaunchAction>
      <BuildableProductRunnable>
         <BuildableReference BuildableIdentifier = "primary" BlueprintIdentifier = "APP"
            BuildableName = "App.app" BlueprintName = "App" ReferencedContainer = "container:App.xcodeproj">
         </BuildableReference>
      </BuildableProductRunnable>
   </LaunchAction>
</Scheme>
"#;

    /// Xcode 27.0 drops a framework's autocreated scheme once a scheme builds
    /// it, and an app's once a scheme runs it, and a reference only counts
    /// for the project it names.
    #[test]
    fn a_scheme_covers_what_it_runs_or_builds_in_the_project_it_names() {
        let (_root, dir) = scratch_container("references");
        let path = dir.join("xcshareddata/xcschemes/Custom.xcscheme");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, COVERING_SCHEME).unwrap();
        let references = SchemeReferences::of(&dir);

        assert!(references.cover(Some(&dir), "Kit", false));
        assert!(references.cover(Some(&dir), "App", true));
        // Building an app doesn't take its scheme's place, running is what does.
        assert!(!references.cover(Some(&dir), "Kit", true));
        assert!(!references.cover(Some(&dir), "Other", false));
        let elsewhere = dir.parent().unwrap().join("Other.xcodeproj");
        assert!(!references.cover(Some(&elsewhere), "App", true));
    }
}
