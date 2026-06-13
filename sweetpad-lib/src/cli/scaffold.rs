//! Pure project scaffolding for `sweetpad project new`.
//!
//! Turns a validated [`ProjectSpec`] into the set of files that make up a
//! minimal, buildable SwiftUI app (iOS or macOS): the `project.pbxproj`
//! (assembled as a
//! [`crate::pbxproj`] object graph and serialized by [`crate::pbxproj_writer`]),
//! the shared `.xcscheme` (built as a [`crate::xcscheme::Element`]), the inner
//! `.xcworkspace`, a `.gitignore`, and the two Swift sources.
//!
//! No I/O happens here — the command layer ([`crate::cli::commands::project`])
//! writes the returned files. Keeping generation pure makes it unit-testable
//! without a Mac (CLI_DESIGN.md §10): the tests round-trip the generated
//! pbxproj back through the parser and re-serialize it to prove it is
//! self-consistent and well-formed.

use std::path::PathBuf;

use crate::pbxproj::{Dict, Value};
use crate::xcscheme::Element;

/// Which Apple platform the generated app targets. Derives `clap::ValueEnum`
/// so `--platform ios|macos` parses and tab-completes; the whole module already
/// lives behind the `cli` feature, so the clap coupling costs nothing extra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Platform {
    Ios,
    Macos,
}

impl Platform {
    /// Human label for prompts and reports.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Platform::Ios => "iOS",
            Platform::Macos => "macOS",
        }
    }

    /// The deployment target proposed when the user doesn't pick one.
    #[must_use]
    pub fn default_deployment_target(self) -> &'static str {
        match self {
            Platform::Ios => "17.0",
            Platform::Macos => "14.0",
        }
    }

    /// `SDKROOT` for the platform.
    fn sdkroot(self) -> &'static str {
        match self {
            Platform::Ios => "iphoneos",
            Platform::Macos => "macosx",
        }
    }

    /// The build setting that carries the deployment target.
    fn deployment_target_key(self) -> &'static str {
        match self {
            Platform::Ios => "IPHONEOS_DEPLOYMENT_TARGET",
            Platform::Macos => "MACOSX_DEPLOYMENT_TARGET",
        }
    }

    /// Where a built app bundle looks for embedded frameworks — `Frameworks/`
    /// sits beside the executable on iOS, one level up (`Contents/Frameworks`)
    /// on macOS.
    fn runpath(self) -> &'static str {
        match self {
            Platform::Ios => "@executable_path/Frameworks",
            Platform::Macos => "@executable_path/../Frameworks",
        }
    }
}

/// Validated inputs for a new project.
#[derive(Debug, Clone)]
pub struct ProjectSpec {
    pub name: String,
    pub bundle_id: String,
    pub deployment_target: String,
    pub platform: Platform,
}

impl ProjectSpec {
    /// Build a spec, validating each field. The error messages are
    /// user-facing — the command surfaces them and (interactively) re-prompts.
    pub fn new(
        name: &str,
        bundle_id: &str,
        deployment_target: &str,
        platform: Platform,
    ) -> Result<Self, String> {
        validate_name(name)?;
        validate_bundle_id(bundle_id)?;
        validate_deployment_target(deployment_target)?;
        Ok(Self {
            name: name.to_string(),
            bundle_id: bundle_id.to_string(),
            deployment_target: deployment_target.to_string(),
            platform,
        })
    }
}

/// A generated file, with a path relative to the project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaffoldFile {
    pub path: PathBuf,
    pub contents: String,
}

/// The project name doubles as a Swift type name (`<Name>App`), a target name,
/// and a product name, so it must be a plain identifier — letters, digits, and
/// underscores, starting with a letter or underscore. This sidesteps a whole
/// class of quoting/module-name bugs that spaces or punctuation would invite.
pub fn validate_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err("project name must not be empty".to_string());
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(format!(
            "project name {name:?} must start with a letter or underscore"
        ));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!(
            "project name {name:?} may only contain letters, digits, and underscores \
             (no spaces or punctuation)"
        ));
    }
    Ok(())
}

/// Reverse-DNS-ish: non-empty, and limited to the characters Apple allows in a
/// bundle identifier (alphanumerics, dots, hyphens).
pub fn validate_bundle_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("bundle identifier must not be empty".to_string());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(format!(
            "bundle identifier {id:?} may only contain letters, digits, dots, and hyphens"
        ));
    }
    Ok(())
}

/// A version-shaped string like `17.0` or `16`.
pub fn validate_deployment_target(target: &str) -> Result<(), String> {
    let shaped = target.chars().next().is_some_and(|c| c.is_ascii_digit())
        && target.chars().all(|c| c.is_ascii_digit() || c == '.');
    if shaped {
        Ok(())
    } else {
        Err(format!(
            "deployment target {target:?} must look like a version, e.g. \"17.0\""
        ))
    }
}

/// Generate every file for the project, each with a project-root-relative path.
#[must_use]
pub fn scaffold(spec: &ProjectSpec) -> Vec<ScaffoldFile> {
    let name = &spec.name;
    let xcodeproj = PathBuf::from(format!("{name}.xcodeproj"));

    let pbxproj = crate::pbxproj_writer::serialize(&project_graph(spec), name);
    let scheme = crate::xcscheme::serialize(&scheme_element(spec));

    vec![
        ScaffoldFile {
            path: xcodeproj.join("project.pbxproj"),
            contents: pbxproj,
        },
        ScaffoldFile {
            path: xcodeproj
                .join("project.xcworkspace")
                .join("contents.xcworkspacedata"),
            contents: WORKSPACE_DATA.to_string(),
        },
        ScaffoldFile {
            path: xcodeproj
                .join("xcshareddata")
                .join("xcschemes")
                .join(format!("{name}.xcscheme")),
            contents: scheme,
        },
        ScaffoldFile {
            path: PathBuf::from(name).join(format!("{name}App.swift")),
            contents: app_swift(name),
        },
        ScaffoldFile {
            path: PathBuf::from(name).join("ContentView.swift"),
            contents: CONTENT_VIEW_SWIFT.to_string(),
        },
        ScaffoldFile {
            path: PathBuf::from(".gitignore"),
            contents: GITIGNORE.to_string(),
        },
    ]
}

// --- pbxproj object graph -------------------------------------------------
//
// Deterministic 24-char-hex GUIDs. Fresh projects don't need the randomness
// Xcode uses, and stable ids make the generated pbxproj snapshot-testable.

const PBXPROJECT: &str = "000000000000000000000001";
const MAIN_GROUP: &str = "000000000000000000000002";
const APP_GROUP: &str = "000000000000000000000003";
const PRODUCTS_GROUP: &str = "000000000000000000000004";
const APP_TARGET: &str = "000000000000000000000005";
const APP_PRODUCT: &str = "000000000000000000000006";
const APP_SWIFT_REF: &str = "000000000000000000000007";
const CONTENT_VIEW_REF: &str = "000000000000000000000008";
const APP_SWIFT_BUILD: &str = "000000000000000000000009";
const CONTENT_VIEW_BUILD: &str = "00000000000000000000000A";
const SOURCES_PHASE: &str = "00000000000000000000000B";
const FRAMEWORKS_PHASE: &str = "00000000000000000000000C";
const RESOURCES_PHASE: &str = "00000000000000000000000D";
const PROJ_CONFIG_LIST: &str = "00000000000000000000000E";
const TARGET_CONFIG_LIST: &str = "00000000000000000000000F";
const PROJ_DEBUG: &str = "000000000000000000000010";
const PROJ_RELEASE: &str = "000000000000000000000011";
const TARGET_DEBUG: &str = "000000000000000000000012";
const TARGET_RELEASE: &str = "000000000000000000000013";

fn vstr(value: impl Into<String>) -> Value {
    Value::String(value.into())
}

fn vdict<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Dict(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn varr<const N: usize>(items: [Value; N]) -> Value {
    Value::Array(items.into())
}

/// A reference to another object by GUID.
fn gid(guid: &str) -> Value {
    Value::String(guid.to_string())
}

#[allow(clippy::too_many_lines)] // one cohesive object-graph assembler, object by object
fn project_graph(spec: &ProjectSpec) -> Value {
    let name = spec.name.clone();
    let mut objects = Dict::new();

    // PBXBuildFile — one per compiled source.
    objects.insert(
        APP_SWIFT_BUILD.to_string(),
        vdict([
            ("isa", vstr("PBXBuildFile")),
            ("fileRef", gid(APP_SWIFT_REF)),
        ]),
    );
    objects.insert(
        CONTENT_VIEW_BUILD.to_string(),
        vdict([
            ("isa", vstr("PBXBuildFile")),
            ("fileRef", gid(CONTENT_VIEW_REF)),
        ]),
    );

    // PBXFileReference — the product and the two sources.
    objects.insert(
        APP_PRODUCT.to_string(),
        vdict([
            ("isa", vstr("PBXFileReference")),
            ("explicitFileType", vstr("wrapper.application")),
            ("includeInIndex", vstr("0")),
            ("path", vstr(format!("{name}.app"))),
            ("sourceTree", vstr("BUILT_PRODUCTS_DIR")),
        ]),
    );
    objects.insert(
        APP_SWIFT_REF.to_string(),
        vdict([
            ("isa", vstr("PBXFileReference")),
            ("lastKnownFileType", vstr("sourcecode.swift")),
            ("path", vstr(format!("{name}App.swift"))),
            ("sourceTree", vstr("<group>")),
        ]),
    );
    objects.insert(
        CONTENT_VIEW_REF.to_string(),
        vdict([
            ("isa", vstr("PBXFileReference")),
            ("lastKnownFileType", vstr("sourcecode.swift")),
            ("path", vstr("ContentView.swift")),
            ("sourceTree", vstr("<group>")),
        ]),
    );

    // PBXFrameworksBuildPhase — empty, but every app target carries one.
    objects.insert(
        FRAMEWORKS_PHASE.to_string(),
        vdict([
            ("isa", vstr("PBXFrameworksBuildPhase")),
            ("buildActionMask", vstr("2147483647")),
            ("files", varr([])),
            ("runOnlyForDeploymentPostprocessing", vstr("0")),
        ]),
    );

    // PBXGroup — main → (sources group, products group).
    objects.insert(
        MAIN_GROUP.to_string(),
        vdict([
            ("isa", vstr("PBXGroup")),
            ("children", varr([gid(APP_GROUP), gid(PRODUCTS_GROUP)])),
            ("sourceTree", vstr("<group>")),
        ]),
    );
    objects.insert(
        APP_GROUP.to_string(),
        vdict([
            ("isa", vstr("PBXGroup")),
            (
                "children",
                varr([gid(APP_SWIFT_REF), gid(CONTENT_VIEW_REF)]),
            ),
            ("path", vstr(name.clone())),
            ("sourceTree", vstr("<group>")),
        ]),
    );
    objects.insert(
        PRODUCTS_GROUP.to_string(),
        vdict([
            ("isa", vstr("PBXGroup")),
            ("children", varr([gid(APP_PRODUCT)])),
            ("name", vstr("Products")),
            ("sourceTree", vstr("<group>")),
        ]),
    );

    // PBXNativeTarget — the app.
    objects.insert(
        APP_TARGET.to_string(),
        vdict([
            ("isa", vstr("PBXNativeTarget")),
            ("buildConfigurationList", gid(TARGET_CONFIG_LIST)),
            (
                "buildPhases",
                varr([
                    gid(SOURCES_PHASE),
                    gid(FRAMEWORKS_PHASE),
                    gid(RESOURCES_PHASE),
                ]),
            ),
            ("buildRules", varr([])),
            ("dependencies", varr([])),
            ("name", vstr(name.clone())),
            ("productName", vstr(name.clone())),
            ("productReference", gid(APP_PRODUCT)),
            ("productType", vstr("com.apple.product-type.application")),
        ]),
    );

    // PBXProject — the root object.
    objects.insert(
        PBXPROJECT.to_string(),
        vdict([
            ("isa", vstr("PBXProject")),
            (
                "attributes",
                vdict([
                    ("BuildIndependentTargetsInParallel", vstr("YES")),
                    ("LastUpgradeCheck", vstr("1500")),
                    (
                        "TargetAttributes",
                        vdict([(APP_TARGET, vdict([("CreatedOnToolsVersion", vstr("15.0"))]))]),
                    ),
                ]),
            ),
            ("buildConfigurationList", gid(PROJ_CONFIG_LIST)),
            ("compatibilityVersion", vstr("Xcode 14.0")),
            ("developmentRegion", vstr("en")),
            ("hasScannedForEncodings", vstr("0")),
            ("knownRegions", varr([vstr("en"), vstr("Base")])),
            ("mainGroup", gid(MAIN_GROUP)),
            ("productRefGroup", gid(PRODUCTS_GROUP)),
            ("projectDirPath", vstr("")),
            ("projectRoot", vstr("")),
            ("targets", varr([gid(APP_TARGET)])),
        ]),
    );

    // PBXResourcesBuildPhase — empty (no asset catalog in v1).
    objects.insert(
        RESOURCES_PHASE.to_string(),
        vdict([
            ("isa", vstr("PBXResourcesBuildPhase")),
            ("buildActionMask", vstr("2147483647")),
            ("files", varr([])),
            ("runOnlyForDeploymentPostprocessing", vstr("0")),
        ]),
    );

    // PBXSourcesBuildPhase — the two Swift files.
    objects.insert(
        SOURCES_PHASE.to_string(),
        vdict([
            ("isa", vstr("PBXSourcesBuildPhase")),
            ("buildActionMask", vstr("2147483647")),
            (
                "files",
                varr([gid(APP_SWIFT_BUILD), gid(CONTENT_VIEW_BUILD)]),
            ),
            ("runOnlyForDeploymentPostprocessing", vstr("0")),
        ]),
    );

    // XCBuildConfiguration — project- and target-level Debug/Release.
    objects.insert(
        PROJ_DEBUG.to_string(),
        vdict([
            ("isa", vstr("XCBuildConfiguration")),
            ("buildSettings", project_settings(spec, true)),
            ("name", vstr("Debug")),
        ]),
    );
    objects.insert(
        PROJ_RELEASE.to_string(),
        vdict([
            ("isa", vstr("XCBuildConfiguration")),
            ("buildSettings", project_settings(spec, false)),
            ("name", vstr("Release")),
        ]),
    );
    objects.insert(
        TARGET_DEBUG.to_string(),
        vdict([
            ("isa", vstr("XCBuildConfiguration")),
            ("buildSettings", target_settings(spec)),
            ("name", vstr("Debug")),
        ]),
    );
    objects.insert(
        TARGET_RELEASE.to_string(),
        vdict([
            ("isa", vstr("XCBuildConfiguration")),
            ("buildSettings", target_settings(spec)),
            ("name", vstr("Release")),
        ]),
    );

    // XCConfigurationList — project and target.
    objects.insert(
        PROJ_CONFIG_LIST.to_string(),
        vdict([
            ("isa", vstr("XCConfigurationList")),
            (
                "buildConfigurations",
                varr([gid(PROJ_DEBUG), gid(PROJ_RELEASE)]),
            ),
            ("defaultConfigurationIsVisible", vstr("0")),
            ("defaultConfigurationName", vstr("Release")),
        ]),
    );
    objects.insert(
        TARGET_CONFIG_LIST.to_string(),
        vdict([
            ("isa", vstr("XCConfigurationList")),
            (
                "buildConfigurations",
                varr([gid(TARGET_DEBUG), gid(TARGET_RELEASE)]),
            ),
            ("defaultConfigurationIsVisible", vstr("0")),
            ("defaultConfigurationName", vstr("Release")),
        ]),
    );

    Value::Dict(
        [
            ("archiveVersion".to_string(), vstr("1")),
            ("classes".to_string(), Value::Dict(Dict::new())),
            ("objectVersion".to_string(), vstr("56")),
            ("objects".to_string(), Value::Dict(objects)),
            ("rootObject".to_string(), gid(PBXPROJECT)),
        ]
        .into_iter()
        .collect(),
    )
}

/// Project-level build settings — the Xcode iOS-app template defaults, trimmed
/// to what actually matters, split into the Debug/Release halves.
fn project_settings(spec: &ProjectSpec, debug: bool) -> Value {
    let mut s = Dict::new();
    let mut put = |key: &str, value: &str| s.insert(key.to_string(), vstr(value));

    put("ALWAYS_SEARCH_USER_PATHS", "NO");
    put("CLANG_ANALYZER_NONNULL", "YES");
    put("CLANG_ENABLE_MODULES", "YES");
    put("CLANG_ENABLE_OBJC_ARC", "YES");
    put("CLANG_ENABLE_OBJC_WEAK", "YES");
    put("COPY_PHASE_STRIP", "NO");
    put("ENABLE_STRICT_OBJC_MSGSEND", "YES");
    put("GCC_C_LANGUAGE_STANDARD", "gnu17");
    put("GCC_NO_COMMON_BLOCKS", "YES");
    put(
        spec.platform.deployment_target_key(),
        &spec.deployment_target,
    );
    put("MTL_FAST_MATH", "YES");
    put("SDKROOT", spec.platform.sdkroot());
    put("SWIFT_VERSION", "5.0");

    if debug {
        put("DEBUG_INFORMATION_FORMAT", "dwarf");
        put("ENABLE_TESTABILITY", "YES");
        put("GCC_DYNAMIC_NO_PIC", "NO");
        put("GCC_OPTIMIZATION_LEVEL", "0");
        put("MTL_ENABLE_DEBUG_INFO", "INCLUDE_SOURCE");
        put("ONLY_ACTIVE_ARCH", "YES");
        put("SWIFT_ACTIVE_COMPILATION_CONDITIONS", "DEBUG");
        put("SWIFT_OPTIMIZATION_LEVEL", "-Onone");
        s.insert(
            "GCC_PREPROCESSOR_DEFINITIONS".to_string(),
            varr([vstr("$(inherited)"), vstr("DEBUG=1")]),
        );
    } else {
        put("DEBUG_INFORMATION_FORMAT", "dwarf-with-dsym");
        put("ENABLE_NS_ASSERTIONS", "NO");
        put("MTL_ENABLE_DEBUG_INFO", "NO");
        put("SWIFT_COMPILATION_MODE", "wholemodule");
        put("SWIFT_OPTIMIZATION_LEVEL", "-O");
    }

    Value::Dict(s)
}

/// Target-level build settings — identical across configurations for a v1 app.
fn target_settings(spec: &ProjectSpec) -> Value {
    let mut s = Dict::new();
    let mut put = |key: &str, value: &str| s.insert(key.to_string(), vstr(value));

    put("ASSETCATALOG_COMPILER_GENERATE_ASSET_SYMBOLS", "NO");
    put("CODE_SIGN_STYLE", "Automatic");
    put("CURRENT_PROJECT_VERSION", "1");
    put("ENABLE_PREVIEWS", "YES");
    put("GENERATE_INFOPLIST_FILE", "YES");
    put("MARKETING_VERSION", "1.0");
    put("PRODUCT_BUNDLE_IDENTIFIER", &spec.bundle_id);
    put("PRODUCT_NAME", "$(TARGET_NAME)");
    put("SWIFT_EMIT_LOC_STRINGS", "YES");

    match spec.platform {
        Platform::Ios => {
            put("INFOPLIST_KEY_UIApplicationSceneManifest_Generation", "YES");
            put("INFOPLIST_KEY_UILaunchScreen_Generation", "YES");
            put(
                "INFOPLIST_KEY_UISupportedInterfaceOrientations_iPad",
                "UIInterfaceOrientationPortrait UIInterfaceOrientationPortraitUpsideDown \
                 UIInterfaceOrientationLandscapeLeft UIInterfaceOrientationLandscapeRight",
            );
            put(
                "INFOPLIST_KEY_UISupportedInterfaceOrientations_iPhone",
                "UIInterfaceOrientationPortrait UIInterfaceOrientationLandscapeLeft \
                 UIInterfaceOrientationLandscapeRight",
            );
            put("TARGETED_DEVICE_FAMILY", "1,2");
        }
        Platform::Macos => {
            // macOS apps generate a different Info.plist and have no device
            // family / launch-screen / orientation keys.
            put("INFOPLIST_KEY_NSHumanReadableCopyright", "");
        }
    }

    s.insert(
        "LD_RUNPATH_SEARCH_PATHS".to_string(),
        varr([vstr("$(inherited)"), vstr(spec.platform.runpath())]),
    );

    Value::Dict(s)
}

// --- shared .xcscheme -----------------------------------------------------

fn elem<const A: usize, const C: usize>(
    name: &str,
    attributes: [(&str, &str); A],
    children: [Element; C],
) -> Element {
    Element {
        name: name.to_string(),
        attributes: attributes
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        children: children.into(),
        text: String::new(),
    }
}

/// A `BuildableReference` pointing at the app target — referenced from the
/// build, launch, and profile actions.
fn buildable_reference(spec: &ProjectSpec) -> Element {
    let name = &spec.name;
    elem(
        "BuildableReference",
        [
            ("BuildableIdentifier", "primary"),
            ("BlueprintIdentifier", APP_TARGET),
            ("BuildableName", &format!("{name}.app")),
            ("BlueprintName", name),
            (
                "ReferencedContainer",
                &format!("container:{name}.xcodeproj"),
            ),
        ],
        [],
    )
}

fn product_runnable(spec: &ProjectSpec) -> Element {
    elem(
        "BuildableProductRunnable",
        [("runnableDebuggingMode", "0")],
        [buildable_reference(spec)],
    )
}

fn scheme_element(spec: &ProjectSpec) -> Element {
    let lldb = "Xcode.DebuggerFoundation.Debugger.LLDB";
    let lldb_launcher = "Xcode.DebuggerFoundation.Launcher.LLDB";

    let build_action = elem(
        "BuildAction",
        [
            ("parallelizeBuildables", "YES"),
            ("buildImplicitDependencies", "YES"),
        ],
        [elem(
            "BuildActionEntries",
            [],
            [elem(
                "BuildActionEntry",
                [
                    ("buildForTesting", "YES"),
                    ("buildForRunning", "YES"),
                    ("buildForProfiling", "YES"),
                    ("buildForArchiving", "YES"),
                    ("buildForAnalyzing", "YES"),
                ],
                [buildable_reference(spec)],
            )],
        )],
    );

    let test_action = elem(
        "TestAction",
        [
            ("buildConfiguration", "Debug"),
            ("selectedDebuggerIdentifier", lldb),
            ("selectedLauncherIdentifier", lldb_launcher),
            ("shouldUseLaunchSchemeArgsEnv", "YES"),
        ],
        [elem("Testables", [], [])],
    );

    let launch_action = elem(
        "LaunchAction",
        [
            ("buildConfiguration", "Debug"),
            ("selectedDebuggerIdentifier", lldb),
            ("selectedLauncherIdentifier", lldb_launcher),
            ("launchStyle", "0"),
            ("useCustomWorkingDirectory", "NO"),
            ("ignoresPersistentStateOnLaunch", "NO"),
            ("debugDocumentVersioning", "YES"),
            ("debugServiceExtension", "internal"),
            ("allowLocationSimulation", "YES"),
        ],
        [product_runnable(spec)],
    );

    let profile_action = elem(
        "ProfileAction",
        [
            ("buildConfiguration", "Release"),
            ("shouldUseLaunchSchemeArgsEnv", "YES"),
            ("savedToolIdentifier", ""),
            ("useCustomWorkingDirectory", "NO"),
            ("debugDocumentVersioning", "YES"),
        ],
        [product_runnable(spec)],
    );

    let analyze_action = elem("AnalyzeAction", [("buildConfiguration", "Debug")], []);
    let archive_action = elem(
        "ArchiveAction",
        [
            ("buildConfiguration", "Release"),
            ("revealArchiveInOrganizer", "YES"),
        ],
        [],
    );

    elem(
        "Scheme",
        [("LastUpgradeVersion", "1500"), ("version", "1.7")],
        [
            build_action,
            test_action,
            launch_action,
            profile_action,
            analyze_action,
            archive_action,
        ],
    )
}

// --- source & static files ------------------------------------------------

fn app_swift(name: &str) -> String {
    format!(
        "import SwiftUI\n\
         \n\
         @main\n\
         struct {name}App: App {{\n\
         \x20   var body: some Scene {{\n\
         \x20       WindowGroup {{\n\
         \x20           ContentView()\n\
         \x20       }}\n\
         \x20   }}\n\
         }}\n"
    )
}

const CONTENT_VIEW_SWIFT: &str = "import SwiftUI\n\
\n\
struct ContentView: View {\n\
\x20   var body: some View {\n\
\x20       VStack {\n\
\x20           Image(systemName: \"globe\")\n\
\x20               .imageScale(.large)\n\
\x20               .foregroundStyle(.tint)\n\
\x20           Text(\"Hello, world!\")\n\
\x20       }\n\
\x20       .padding()\n\
\x20   }\n\
}\n\
\n\
#Preview {\n\
\x20   ContentView()\n\
}\n";

const WORKSPACE_DATA: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Workspace\n\
\x20  version = \"1.0\">\n\
\x20  <FileRef\n\
\x20     location = \"self:\">\n\
\x20  </FileRef>\n\
</Workspace>\n";

const GITIGNORE: &str = "# Xcode\n\
build/\n\
DerivedData/\n\
*.xcuserstate\n\
xcuserdata/\n\
\n\
# macOS\n\
.DS_Store\n\
\n\
# Swift Package Manager\n\
.build/\n\
.swiftpm/\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ProjectSpec {
        ProjectSpec::new("MyApp", "com.example.MyApp", "17.0", Platform::Ios).unwrap()
    }

    #[test]
    fn validates_names() {
        assert!(validate_name("MyApp").is_ok());
        assert!(validate_name("_Internal2").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("2Cool").is_err());
        assert!(validate_name("My App").is_err());
        assert!(validate_name("My-App").is_err());
    }

    #[test]
    fn validates_bundle_ids_and_targets() {
        assert!(validate_bundle_id("com.example.app").is_ok());
        assert!(validate_bundle_id("com.example app").is_err());
        assert!(validate_deployment_target("17.0").is_ok());
        assert!(validate_deployment_target("16").is_ok());
        assert!(validate_deployment_target("latest").is_err());
    }

    #[test]
    fn emits_the_expected_files() {
        let files = scaffold(&spec());
        let paths: Vec<String> = files
            .iter()
            .map(|f| f.path.to_string_lossy().into_owned())
            .collect();
        assert!(paths.contains(&"MyApp.xcodeproj/project.pbxproj".to_string()));
        assert!(
            paths.contains(&"MyApp.xcodeproj/xcshareddata/xcschemes/MyApp.xcscheme".to_string())
        );
        assert!(paths.contains(&"MyApp/MyAppApp.swift".to_string()));
        assert!(paths.contains(&"MyApp/ContentView.swift".to_string()));
    }

    fn pbxproj(files: &[ScaffoldFile]) -> &str {
        &files
            .iter()
            .find(|f| f.path.ends_with("project.pbxproj"))
            .expect("pbxproj present")
            .contents
    }

    #[test]
    fn pbxproj_parses_and_round_trips() {
        let files = scaffold(&spec());
        let raw = pbxproj(&files);

        // The generated pbxproj must parse with the crate's own parser …
        let value = crate::pbxproj::parse(raw).expect("generated pbxproj parses");

        // … and re-serialize to exactly the same bytes (self-consistent).
        let again = crate::pbxproj_writer::serialize(&value, "MyApp");
        assert_eq!(raw, again, "round-trip through parse/serialize is stable");
    }

    #[test]
    fn project_resolves_target_and_configurations() {
        let files = scaffold(&spec());
        let value = crate::pbxproj::parse(pbxproj(&files)).unwrap();
        let project =
            crate::project::open_from_value(&value, std::path::Path::new("MyApp.xcodeproj"))
                .expect("resolves into a Project");

        assert_eq!(project.targets.len(), 1);
        assert_eq!(project.targets[0].name, "MyApp");
        assert_eq!(project.configurations, vec!["Debug", "Release"]);
    }

    #[test]
    fn scheme_is_well_formed_xml() {
        let xml = crate::xcscheme::serialize(&scheme_element(&spec()));
        // Parse it back to confirm the generated scheme is valid.
        let root = crate::xcscheme::parse(&xml).expect("scheme parses");
        assert_eq!(root.name, "Scheme");
        let refs = root.descendants_named("BuildableReference");
        assert!(!refs.is_empty());
        assert_eq!(refs[0].attr("BuildableName"), Some("MyApp.app"));
    }

    #[test]
    fn bundle_id_lands_in_target_settings() {
        let files =
            scaffold(&ProjectSpec::new("Demo", "io.demo.app", "16.4", Platform::Ios).unwrap());
        let raw = pbxproj(&files);
        assert!(raw.contains("PRODUCT_BUNDLE_IDENTIFIER = io.demo.app;"));
        assert!(raw.contains("IPHONEOS_DEPLOYMENT_TARGET = 16.4;"));
    }

    #[test]
    fn macos_uses_macos_sdk_and_omits_ios_keys() {
        let spec =
            ProjectSpec::new("MacApp", "com.example.MacApp", "14.0", Platform::Macos).unwrap();
        let files = scaffold(&spec);
        let raw = pbxproj(&files);

        // Parses and resolves like any other project …
        let value = crate::pbxproj::parse(raw).expect("macOS pbxproj parses");
        let project =
            crate::project::open_from_value(&value, std::path::Path::new("MacApp.xcodeproj"))
                .expect("resolves");
        assert_eq!(project.targets[0].name, "MacApp");

        // … but carries the macOS SDK/deployment key and none of the iOS-only ones.
        assert!(raw.contains("SDKROOT = macosx;"));
        assert!(raw.contains("MACOSX_DEPLOYMENT_TARGET = 14.0;"));
        assert!(!raw.contains("IPHONEOS_DEPLOYMENT_TARGET"));
        assert!(!raw.contains("TARGETED_DEVICE_FAMILY"));
        assert!(!raw.contains("INFOPLIST_KEY_UILaunchScreen_Generation"));
    }
}
