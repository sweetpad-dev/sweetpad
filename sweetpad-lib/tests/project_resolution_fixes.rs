//! Regression tests for project-resolution edge cases where we previously
//! diverged from xcodebuild: configuration-name fallback, missing/anchored
//! xcconfig references, the `TestTargetID` host edge, dangling-reference
//! tolerance, and per-user/autocreated scheme discovery.

use std::fs;
use std::path::{Path, PathBuf};

use sweetpad::project::{
    build_settings, is_test_bundle_product_type, is_unit_test_bundle_product_type, open,
    scheme_for_target,
};
use sweetpad::xcconfig::Assignment;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// A fresh scratch `.xcodeproj` under the temp dir, holding `pbxproj` as its
/// `project.pbxproj`. Each test passes a unique `tag` so parallel tests don't
/// collide.
fn scratch_xcodeproj(tag: &str, pbxproj: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("sweetpad-fixes-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let xcodeproj = root.join("App.xcodeproj");
    fs::create_dir_all(&xcodeproj).unwrap();
    fs::write(xcodeproj.join("project.pbxproj"), pbxproj).unwrap();
    xcodeproj
}

fn value_of<'a>(layer: &'a [Assignment], key: &str) -> Option<&'a str> {
    layer
        .iter()
        .rev()
        .find(|a| a.key == key)
        .map(|a| a.value.as_str())
}

/// A two-config project + target where Debug and Release differ on a key and
/// both XCConfigurationLists declare `defaultConfigurationName = Release`.
const TWO_CONFIG_PBXPROJ: &str = "\
// !$*UTF8*$!
{
\tarchiveVersion = 1;
\tobjects = {
\t\tPROJ = { isa = PBXProject; buildConfigurationList = PLIST; mainGroup = MAIN; targets = (APP, TOOL); };
\t\tMAIN = { isa = PBXGroup; sourceTree = \"<group>\"; children = (); };
\t\tPLIST = { isa = XCConfigurationList; buildConfigurations = (PDBG, PREL); defaultConfigurationName = Release; };
\t\tPDBG = { isa = XCBuildConfiguration; name = Debug; buildSettings = { SWIFT_VERSION = 5.0; }; };
\t\tPREL = { isa = XCBuildConfiguration; name = Release; buildSettings = { SWIFT_VERSION = 6.0; }; };
\t\tAPP = { isa = PBXNativeTarget; name = App; buildConfigurationList = TLIST; productType = \"com.apple.product-type.application\"; };
\t\tTLIST = { isa = XCConfigurationList; buildConfigurations = (TDBG, TREL); defaultConfigurationName = Release; };
\t\tTDBG = { isa = XCBuildConfiguration; name = Debug; buildSettings = { PRODUCT_NAME = AppDebug; }; };
\t\tTREL = { isa = XCBuildConfiguration; name = Release; buildSettings = { PRODUCT_NAME = AppRelease; }; };
\t\tTOOL = { isa = PBXNativeTarget; name = Tool; productType = \"com.apple.product-type.tool\"; };
\t};
\trootObject = PROJ;
}
";

/// Fix 1: a configuration name absent from the lists must not error —
/// xcodebuild warns and falls back to each list's own
/// `defaultConfigurationName` (Release here, not Debug).
#[test]
fn unknown_configuration_falls_back_to_default() {
    let proj = scratch_xcodeproj("cfg-fallback", TWO_CONFIG_PBXPROJ);
    let ctx = build_settings(&proj, "App", "Bogus")
        .expect("an unknown configuration must fall back to the list defaults, not error");
    assert_eq!(
        value_of(&ctx.layers[1], "SWIFT_VERSION"),
        Some("6.0"),
        "project layer must come from the default (Release) config, not Debug"
    );
    assert_eq!(
        value_of(&ctx.layers[3], "PRODUCT_NAME"),
        Some("AppRelease"),
        "target layer must come from the default (Release) config, not Debug"
    );
}

/// Fix 1: exact-name matching stays case-sensitive — `debug` is NOT `Debug`,
/// so it takes the same default-config fallback (matching xcodebuild).
#[test]
fn configuration_match_is_case_sensitive() {
    let proj = scratch_xcodeproj("cfg-case", TWO_CONFIG_PBXPROJ);
    let ctx = build_settings(&proj, "App", "debug").unwrap();
    assert_eq!(
        value_of(&ctx.layers[1], "SWIFT_VERSION"),
        Some("6.0"),
        "lowercase 'debug' must not match 'Debug'; it falls back to the default config"
    );
}

/// Fix 1: a target with NO buildConfigurationList contributes empty target
/// layers — xcodebuild resolves it with project-level settings only.
#[test]
fn target_without_configuration_list_resolves_with_project_settings() {
    let proj = scratch_xcodeproj("cfg-no-target-list", TWO_CONFIG_PBXPROJ);
    let ctx = build_settings(&proj, "Tool", "Debug")
        .expect("a target without a buildConfigurationList must still resolve");
    assert_eq!(value_of(&ctx.layers[1], "SWIFT_VERSION"), Some("5.0"));
    assert!(ctx.layers[2].is_empty(), "no target xcconfig layer");
    assert!(ctx.layers[3].is_empty(), "no target inline layer");
}

/// Fix 1: the project-level `defaultConfigurationName` is surfaced on the
/// `Project` struct so BSP / the extension can show it.
#[test]
fn project_exposes_default_configuration() {
    let path = fixtures_root().join("_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj");
    let project = open(&path).unwrap();
    assert_eq!(project.default_configuration.as_deref(), Some("Release"));
}

/// Fix 2: a `baseConfigurationReference` to a file that doesn't exist (the
/// classic CocoaPods-before-`pod install` state) must not be fatal —
/// xcodebuild warns "Unable to open base configuration reference file" and
/// resolves as if no xcconfig were attached.
#[test]
fn missing_base_configuration_reference_is_not_fatal() {
    let scratch_pbxproj = fixtures_root()
        .join("_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj/project.pbxproj");
    let pbxproj = fs::read_to_string(&scratch_pbxproj)
        .unwrap()
        // Attach a dangling xcconfig reference to the target's Debug config…
        .replace(
            "F08EA8A9D109525AA300CBE6 = {",
            "F08EA8A9D109525AA300CBE6 = { baseConfigurationReference = AAA0000000000000000000AA;",
        )
        // …backed by a PBXFileReference whose file was never created.
        .replace(
            "9D45771675EE5736A477EF39 = {",
            "AAA0000000000000000000AA = {isa = PBXFileReference; lastKnownFileType = text.xcconfig; path = Missing.xcconfig; sourceTree = \"<group>\"; };\n        9D45771675EE5736A477EF39 = {",
        );
    let proj = scratch_xcodeproj("missing-xcconfig", &pbxproj);
    let ctx = build_settings(&proj, "Scratch", "Debug")
        .expect("a missing base xcconfig must resolve as if none were attached");
    assert!(
        ctx.layers[2].is_empty(),
        "the dangling xcconfig contributes an empty layer"
    );
    assert_eq!(
        value_of(&ctx.layers[3], "PRODUCT_NAME"),
        Some("Scratch"),
        "the pbxproj-authored settings still resolve"
    );
}

/// Fix 3: an Xcode-16 `baseConfigurationReferenceAnchor` pointing at a
/// `PBXFileSystemSynchronizedRootGroup` nested under pathed groups must
/// accumulate the parent groups' `path` segments — the same walk source-file
/// resolution does — not anchor flat at the project dir.
#[test]
fn anchored_xcconfig_honors_parent_group_chain() {
    let pbxproj = "\
// !$*UTF8*$!
{
\tarchiveVersion = 1;
\tobjects = {
\t\tPROJ = { isa = PBXProject; buildConfigurationList = PLIST; mainGroup = MAIN; targets = (APP); };
\t\tMAIN = { isa = PBXGroup; sourceTree = \"<group>\"; children = (APPGRP); };
\t\tAPPGRP = { isa = PBXGroup; path = App; sourceTree = \"<group>\"; children = (SYNC); };
\t\tSYNC = { isa = PBXFileSystemSynchronizedRootGroup; path = Config; sourceTree = \"<group>\"; };
\t\tPLIST = { isa = XCConfigurationList; buildConfigurations = (PDBG); };
\t\tPDBG = { isa = XCBuildConfiguration; name = Debug; buildSettings = {}; baseConfigurationReferenceAnchor = SYNC; baseConfigurationReferenceRelativePath = Base.xcconfig; };
\t\tAPP = { isa = PBXNativeTarget; name = App; buildConfigurationList = TLIST; };
\t\tTLIST = { isa = XCConfigurationList; buildConfigurations = (TDBG); };
\t\tTDBG = { isa = XCBuildConfiguration; name = Debug; buildSettings = {}; };
\t};
\trootObject = PROJ;
}
";
    let proj = scratch_xcodeproj("anchor-nested", pbxproj);
    // The anchor's folder lives under the `path = App` group:
    // <root>/App/Config/Base.xcconfig, NOT <root>/Config/Base.xcconfig.
    let config_dir = proj.parent().unwrap().join("App/Config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("Base.xcconfig"), "FIX3_MARKER = nested\n").unwrap();

    let ctx = build_settings(&proj, "App", "Debug").unwrap();
    assert_eq!(
        value_of(&ctx.layers[0], "FIX3_MARKER"),
        Some("nested"),
        "the anchored xcconfig must resolve through the parent group's `App` segment"
    );
}

/// Fix 3: an anchor with `sourceTree = SOURCE_ROOT` resolves against the
/// project dir regardless of where the anchor sits in the group tree.
#[test]
fn anchored_xcconfig_honors_source_root_tree() {
    let pbxproj = "\
// !$*UTF8*$!
{
\tarchiveVersion = 1;
\tobjects = {
\t\tPROJ = { isa = PBXProject; buildConfigurationList = PLIST; mainGroup = MAIN; targets = (APP); };
\t\tMAIN = { isa = PBXGroup; sourceTree = \"<group>\"; children = (APPGRP); };
\t\tAPPGRP = { isa = PBXGroup; path = App; sourceTree = \"<group>\"; children = (SYNC); };
\t\tSYNC = { isa = PBXFileSystemSynchronizedRootGroup; path = Rooted; sourceTree = SOURCE_ROOT; };
\t\tPLIST = { isa = XCConfigurationList; buildConfigurations = (PDBG); };
\t\tPDBG = { isa = XCBuildConfiguration; name = Debug; buildSettings = {}; baseConfigurationReferenceAnchor = SYNC; baseConfigurationReferenceRelativePath = Base.xcconfig; };
\t\tAPP = { isa = PBXNativeTarget; name = App; buildConfigurationList = TLIST; };
\t\tTLIST = { isa = XCConfigurationList; buildConfigurations = (TDBG); };
\t\tTDBG = { isa = XCBuildConfiguration; name = Debug; buildSettings = {}; };
\t};
\trootObject = PROJ;
}
";
    let proj = scratch_xcodeproj("anchor-srcroot", pbxproj);
    // SOURCE_ROOT ignores the nested `App` group: <root>/Rooted/Base.xcconfig.
    let config_dir = proj.parent().unwrap().join("Rooted");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("Base.xcconfig"), "FIX3_ROOT = yes\n").unwrap();

    let ctx = build_settings(&proj, "App", "Debug").unwrap();
    assert_eq!(value_of(&ctx.layers[0], "FIX3_ROOT"), Some("yes"));
}

/// Fix 4: the root PBXProject's `TargetAttributes.<test-uuid>.TestTargetID`
/// is the authoritative test-host edge. With two application dependencies and
/// the helper listed first, the host must still resolve to the TestTargetID.
#[test]
fn test_host_prefers_test_target_id_attribute() {
    let pbxproj = "\
// !$*UTF8*$!
{
\tarchiveVersion = 1;
\tobjects = {
\t\tPROJ = { isa = PBXProject; attributes = { TargetAttributes = { TESTS = { TestTargetID = APP2; }; }; }; buildConfigurationList = PLIST; mainGroup = MAIN; targets = (APP1, APP2, TESTS); };
\t\tMAIN = { isa = PBXGroup; sourceTree = \"<group>\"; children = (); };
\t\tPLIST = { isa = XCConfigurationList; buildConfigurations = (PDBG); };
\t\tPDBG = { isa = XCBuildConfiguration; name = Debug; buildSettings = {}; };
\t\tAPP1 = { isa = PBXNativeTarget; name = Helper; buildConfigurationList = PLIST; productType = \"com.apple.product-type.application\"; };
\t\tAPP2 = { isa = PBXNativeTarget; name = Host; buildConfigurationList = PLIST; productType = \"com.apple.product-type.application\"; };
\t\tTESTS = { isa = PBXNativeTarget; name = Tests; buildConfigurationList = PLIST; productType = \"com.apple.product-type.bundle.unit-test\"; dependencies = (DEP1, DEP2); };
\t\tDEP1 = { isa = PBXTargetDependency; target = APP1; };
\t\tDEP2 = { isa = PBXTargetDependency; target = APP2; };
\t};
\trootObject = PROJ;
}
";
    let proj = scratch_xcodeproj("test-target-id", pbxproj);
    let ctx = build_settings(&proj, "Tests", "Debug").unwrap();
    assert_eq!(
        ctx.test_host_target.as_deref(),
        Some("Host"),
        "TestTargetID points at APP2 (Host), overriding the first app dependency"
    );
}

/// Fix 4 fallback: without a TestTargetID attribute, the first application
/// dependency still wins (the pre-existing behavior).
#[test]
fn test_host_falls_back_to_first_app_dependency() {
    let pbxproj = "\
// !$*UTF8*$!
{
\tarchiveVersion = 1;
\tobjects = {
\t\tPROJ = { isa = PBXProject; buildConfigurationList = PLIST; mainGroup = MAIN; targets = (APP1, APP2, TESTS); };
\t\tMAIN = { isa = PBXGroup; sourceTree = \"<group>\"; children = (); };
\t\tPLIST = { isa = XCConfigurationList; buildConfigurations = (PDBG); };
\t\tPDBG = { isa = XCBuildConfiguration; name = Debug; buildSettings = {}; };
\t\tAPP1 = { isa = PBXNativeTarget; name = Helper; buildConfigurationList = PLIST; productType = \"com.apple.product-type.application\"; };
\t\tAPP2 = { isa = PBXNativeTarget; name = Host; buildConfigurationList = PLIST; productType = \"com.apple.product-type.application\"; };
\t\tTESTS = { isa = PBXNativeTarget; name = Tests; buildConfigurationList = PLIST; productType = \"com.apple.product-type.bundle.unit-test\"; dependencies = (DEP1, DEP2); };
\t\tDEP1 = { isa = PBXTargetDependency; target = APP1; };
\t\tDEP2 = { isa = PBXTargetDependency; target = APP2; };
\t};
\trootObject = PROJ;
}
";
    let proj = scratch_xcodeproj("test-host-fallback", pbxproj);
    let ctx = build_settings(&proj, "Tests", "Debug").unwrap();
    assert_eq!(ctx.test_host_target.as_deref(), Some("Helper"));
}

/// Fix 5: a dangling `XCBuildConfiguration` id inside a list is skipped, and a
/// dangling target `buildConfigurationList` is treated as an empty list —
/// neither fails `open()`.
#[test]
fn open_tolerates_dangling_configuration_references() {
    let pbxproj = "\
// !$*UTF8*$!
{
\tarchiveVersion = 1;
\tobjects = {
\t\tPROJ = { isa = PBXProject; buildConfigurationList = PLIST; mainGroup = MAIN; targets = (APP, TOOL); };
\t\tMAIN = { isa = PBXGroup; sourceTree = \"<group>\"; children = (); };
\t\tPLIST = { isa = XCConfigurationList; buildConfigurations = (PDBG, GHOST); };
\t\tPDBG = { isa = XCBuildConfiguration; name = Debug; buildSettings = {}; };
\t\tAPP = { isa = PBXNativeTarget; name = App; buildConfigurationList = PLIST; };
\t\tTOOL = { isa = PBXNativeTarget; name = Tool; buildConfigurationList = MISSINGLIST; };
\t};
\trootObject = PROJ;
}
";
    let proj = scratch_xcodeproj("dangling-refs", pbxproj);
    let project = open(&proj).expect("dangling configuration references must not fail open()");
    assert_eq!(
        project.configurations,
        vec!["Debug"],
        "the dangling GHOST config id is skipped"
    );
    let tool = project.targets.iter().find(|t| t.name == "Tool").unwrap();
    assert!(
        tool.configurations.is_empty(),
        "a dangling buildConfigurationList reads as an empty list"
    );
}

/// Minimal scheme XML whose BuildAction references `blueprint`.
fn scheme_xml(blueprint: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <Scheme LastUpgradeVersion=\"1500\" version=\"1.7\">\n\
         \x20\x20<BuildAction parallelizeBuildables=\"YES\">\n\
         \x20\x20\x20\x20<BuildActionEntries>\n\
         \x20\x20\x20\x20\x20\x20<BuildActionEntry buildForRunning=\"YES\" buildForTesting=\"YES\">\n\
         \x20\x20\x20\x20\x20\x20\x20\x20<BuildableReference BuildableIdentifier=\"primary\" BlueprintIdentifier=\"X\" BuildableName=\"{blueprint}.app\" BlueprintName=\"{blueprint}\" ReferencedContainer=\"container:App.xcodeproj\"/>\n\
         \x20\x20\x20\x20\x20\x20</BuildActionEntry>\n\
         \x20\x20\x20\x20</BuildActionEntries>\n\
         \x20\x20</BuildAction>\n\
         </Scheme>\n"
    )
}

/// The per-user scheme directory `scheme_for_target` must consult, matching
/// the identity `crate::scheme` detects from `$USER`.
fn user_schemes_dir(container: &Path) -> PathBuf {
    let user = std::env::var("USER").unwrap_or_else(|_| "tester".into());
    container.join(format!("xcuserdata/{user}.xcuserdatad/xcschemes"))
}

fn write_scheme(dir: &Path, name: &str, blueprint: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(format!("{name}.xcscheme")), scheme_xml(blueprint)).unwrap();
}

/// Fix 6: a per-user scheme named exactly like the target qualifies — the
/// old implementation only consulted `xcshareddata/xcschemes`.
#[test]
fn scheme_for_target_finds_per_user_scheme_by_name() {
    let proj = scratch_xcodeproj("scheme-user-name", TWO_CONFIG_PBXPROJ);
    write_scheme(&user_schemes_dir(&proj), "App", "App");
    assert_eq!(scheme_for_target(&proj, "App").as_deref(), Some("App"));
}

/// Fix 6: a per-user scheme with a different name still qualifies through its
/// BuildAction's blueprint reference.
#[test]
fn scheme_for_target_finds_per_user_scheme_by_build_action() {
    let proj = scratch_xcodeproj("scheme-user-build", TWO_CONFIG_PBXPROJ);
    write_scheme(&user_schemes_dir(&proj), "Main", "App");
    assert_eq!(scheme_for_target(&proj, "App").as_deref(), Some("Main"));
}

/// Fix 6: shared scheme files are preferred over per-user ones when both
/// reference the target.
#[test]
fn scheme_for_target_prefers_shared_over_user() {
    let proj = scratch_xcodeproj("scheme-shared-first", TWO_CONFIG_PBXPROJ);
    write_scheme(&proj.join("xcshareddata/xcschemes"), "SharedOne", "App");
    write_scheme(&user_schemes_dir(&proj), "AUserOne", "App");
    assert_eq!(
        scheme_for_target(&proj, "App").as_deref(),
        Some("SharedOne"),
        "a shared scheme wins over a per-user scheme even when it sorts later"
    );
}

/// Fix 6: with no scheme file anywhere and autocreation enabled (the default),
/// the target's own name is a valid autocreated scheme for xcodebuild.
#[test]
fn scheme_for_target_autocreates_when_no_scheme_files() {
    let proj = scratch_xcodeproj("scheme-autocreate", TWO_CONFIG_PBXPROJ);
    assert_eq!(scheme_for_target(&proj, "App").as_deref(), Some("App"));
}

/// The plist disabling scheme autocreation, as Xcode writes it (and XcodeGen /
/// Tuist generate it).
const AUTOCREATE_OFF_PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
    <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
    <plist version=\"1.0\">\n<dict>\n\
    \t<key>IDEWorkspaceSharedSettings_AutocreateContextsIfNeeded</key>\n\
    \t<false/>\n</dict>\n</plist>\n";

fn disable_autocreation(xcodeproj: &Path) {
    let dir = xcodeproj.join("project.xcworkspace/xcshareddata");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("WorkspaceSettings.xcsettings"),
        AUTOCREATE_OFF_PLIST,
    )
    .unwrap();
}

/// Fix 6: with autocreation disabled and no scheme files, there is nothing
/// xcodebuild would accept — `None`.
#[test]
fn scheme_for_target_respects_disabled_autocreation() {
    let proj = scratch_xcodeproj("scheme-no-autocreate", TWO_CONFIG_PBXPROJ);
    disable_autocreation(&proj);
    assert_eq!(scheme_for_target(&proj, "App"), None);
}

/// Fix 7: `open()`'s per-target autocreated-scheme fallback is gated on the
/// workspace's autocreation flag — `xcodebuild -list` shows no schemes for an
/// XcodeGen/Tuist project that writes the flag and shares no schemes.
#[test]
fn open_does_not_autocreate_schemes_when_disabled() {
    let proj = scratch_xcodeproj("open-no-autocreate", TWO_CONFIG_PBXPROJ);
    disable_autocreation(&proj);
    let project = open(&proj).unwrap();
    assert!(
        project.schemes.is_empty(),
        "autocreation is disabled and no scheme files exist; got {:?}",
        project.schemes
    );
}

/// Fix 7 control: with the flag absent, autocreation still lists per-target
/// schemes for a project with no scheme files.
#[test]
fn open_autocreates_schemes_by_default() {
    let proj = scratch_xcodeproj("open-autocreate", TWO_CONFIG_PBXPROJ);
    let project = open(&proj).unwrap();
    assert_eq!(project.schemes, vec!["App", "Tool"]);
}

/// Fix 8: UI-testing bundles are XCTest bundles (framework search paths,
/// test-host edge) but are NOT unit-style bundles — xcodebuild builds them
/// into their own `<PRODUCT_NAME>-Runner.app/PlugIns`, never the host app's
/// `PlugIns` (per the corpus captures: tuist `ios_app_with_watchapp2`'s
/// `WatchAppUITests` reports `TARGET_BUILD_SUBPATH =
/// /WatchAppUITests-Runner.app/PlugIns`, `USES_XCTRUNNER = YES`).
#[test]
fn ui_test_bundles_are_not_unit_test_bundles() {
    for unit in [
        "com.apple.product-type.bundle.unit-test",
        "com.apple.product-type.bundle.external-test",
        "com.apple.product-type.bundle.ocunit-test",
    ] {
        assert!(is_unit_test_bundle_product_type(Some(unit)), "{unit}");
        assert!(is_test_bundle_product_type(Some(unit)), "{unit}");
    }
    let ui = "com.apple.product-type.bundle.ui-testing";
    assert!(is_test_bundle_product_type(Some(ui)));
    assert!(
        !is_unit_test_bundle_product_type(Some(ui)),
        "UI-test bundles must not take the host-PlugIns nesting path"
    );
    assert!(!is_unit_test_bundle_product_type(None));
    assert!(!is_unit_test_bundle_product_type(Some(
        "com.apple.product-type.application"
    )));
}
