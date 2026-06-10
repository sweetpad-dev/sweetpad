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
//! to produce a `Vec<ResolveQuery>`.
//!
//! What's NOT modeled (yet, deliberately): pre/post actions, test plans,
//! custom working directory, debugger / launcher identifiers. Add these
//! incrementally as concrete callers need them — see CLAUDE.md "minimum
//! abstraction."

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

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
    /// `LaunchAction.CommandLineArguments`, in scheme order. The caller
    /// filters by `is_enabled` and applies Xcode's whitespace-splitting.
    pub launch_arguments: Vec<CommandLineArgument>,
    /// `LaunchAction.EnvironmentVariables`, in scheme order.
    pub launch_environment_variables: Vec<EnvironmentVariable>,
    /// `LaunchAction`'s `language` attribute (drives `-AppleLanguages`).
    pub launch_language: Option<String>,
    /// `LaunchAction`'s `region` attribute (drives `-AppleLocale`).
    pub launch_region: Option<String>,
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
    /// The raw argument string. Xcode splits it on whitespace at launch.
    pub argument: String,
    /// `isEnabled="NO"` unchecks the row; an absent attribute is enabled.
    pub is_enabled: bool,
}

/// An `<EnvironmentVariable>` under `LaunchAction.EnvironmentVariables`.
#[derive(Debug, Clone)]
pub struct EnvironmentVariable {
    pub key: String,
    /// `None` when the `value` attribute is absent (distinct from empty `""`);
    /// Xcode writes value-less rows for widget-preview placeholders.
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

/// The login user whose `xcuserdata` Xcode would consult, or `None` when the
/// process has no usable identity (then we fall back to scanning every user's
/// directory rather than seeing no per-user schemes at all).
fn detected_user() -> Option<String> {
    std::env::var("USER").ok().filter(|u| !u.is_empty())
}

/// Test-only: a username whose `xcuserdata` directory is visible through the
/// public scheme-discovery APIs on this host — the detected `$USER` when set,
/// any fixed name otherwise (no identity → every user dir is scanned). Tests
/// that create per-user scheme files use this so they pass both on developer
/// machines (where `$USER` is set and scoping applies) and in bare containers.
#[cfg(test)]
pub(crate) fn visible_user() -> String {
    detected_user().unwrap_or_else(|| "tester".into())
}

/// The directories a container (`.xcodeproj` or `.xcworkspace` — both share
/// the same layout) stores scheme files in: `xcshareddata/xcschemes` first,
/// then the per-user `xcuserdata/<user>.xcuserdatad/xcschemes`. Xcode and
/// xcodebuild only consult the *current* user's directory — a committed
/// `xcuserdata/alice.xcuserdatad` scheme is invisible to bob — so we scope to
/// `$USER` when the identity is known, and scan every user directory (sorted,
/// for a stable order) only as a best-effort fallback when it isn't.
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

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn rejects_non_scheme_root() {
        let element = Element {
            name: "NotAScheme".into(),
            ..Default::default()
        };
        let err = from_element(&element).unwrap_err();
        assert!(format!("{err}").contains("expected root element"));
    }

    /// A unique scratch container dir under the OS temp dir.
    fn scratch_container(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "sweetpad-scheme-{tag}-{}-{n}.xcodeproj",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }

    #[test]
    fn container_schemes_merges_shared_and_user_schemes() {
        let dir = scratch_container("merge");
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
        let dir = scratch_container("empty");
        assert!(container_schemes(&dir).is_empty());
    }

    #[test]
    fn scheme_dirs_scope_to_the_known_user() {
        let dir = scratch_container("user-scope");
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
        let dir = scratch_container("autocreate-default");
        assert!(autocreation_allowed(&dir));

        // Workspace-style container with the key set to false → disabled.
        let plist = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
            <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
            <plist version=\"1.0\">\n<dict>\n\
            \t<key>IDEWorkspaceSharedSettings_AutocreateContextsIfNeeded</key>\n\
            \t<false/>\n</dict>\n</plist>\n";
        let ws = scratch_container("autocreate-off");
        std::fs::create_dir_all(ws.join("xcshareddata")).unwrap();
        std::fs::write(ws.join("xcshareddata/WorkspaceSettings.xcsettings"), plist).unwrap();
        assert!(!autocreation_allowed(&ws));

        // Project-style container (settings inside the embedded workspace),
        // key explicitly true → enabled.
        let proj = scratch_container("autocreate-on");
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
        let dir = scratch_container("find");
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
}
