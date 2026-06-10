//! Library-level coverage of the build-settings orchestration
//! (`build_settings::resolve_build_settings`) — the behaviours previously
//! exercised end-to-end through the (removed) CLI `build-settings` command.

// Test helpers take owned options and assert on a literal `.sdk` suffix; the
// pedantic lints for those don't apply in tests.
#![allow(
    clippy::needless_pass_by_value,
    clippy::case_sensitive_file_extension_comparisons
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use sweetpad::build_settings::{BuildSettingsOptions, resolve_build_settings};
use sweetpad::destination::parse_destination_arg;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn scratch_proj() -> PathBuf {
    fixtures_root().join("_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj")
}

fn kingfisher_proj() -> PathBuf {
    fixtures_root().join("kingfisher/xcode-26.5.0/raw/Kingfisher.xcodeproj")
}

fn xcconfig_fixture(name: &str) -> PathBuf {
    fixtures_root().join(format!(
        "_synthetic-xcconfigs/xcode-26.5.0/xcconfigs/{name}.xcconfig"
    ))
}

/// Resolve a single target and return its settings map.
fn resolve_one(opts: BuildSettingsOptions) -> BTreeMap<String, String> {
    let mut out = resolve_build_settings(&opts).unwrap();
    assert_eq!(out.len(), 1, "expected exactly one resolved target");
    out.remove(0).settings
}

fn scratch_opts() -> BuildSettingsOptions {
    BuildSettingsOptions {
        project: Some(scratch_proj()),
        target: Some("Scratch".to_string()),
        configuration: "Debug".to_string(),
        sdk: "macosx".to_string(),
        arch: "arm64".to_string(),
        ..Default::default()
    }
}

fn kingfisher_opts() -> BuildSettingsOptions {
    BuildSettingsOptions {
        project: Some(kingfisher_proj()),
        target: Some("Kingfisher".to_string()),
        configuration: "Debug".to_string(),
        sdk: "macosx".to_string(),
        arch: "arm64".to_string(),
        ..Default::default()
    }
}

#[test]
fn scratch_debug_resolves_against_active_xcode() {
    // No xcode/xcspec_root: resolves against the active Xcode's specs + SDKs
    // (matching xcodebuild), so Apple's defaults layer under the project's.
    let s = resolve_one(scratch_opts());
    assert_eq!(
        s.get("ALWAYS_SEARCH_USER_PATHS").map(String::as_str),
        Some("NO")
    );
    assert_eq!(
        s.get("MACOSX_DEPLOYMENT_TARGET").map(String::as_str),
        Some("12.0")
    );
    assert_eq!(s.get("SWIFT_VERSION").map(String::as_str), Some("5.0"));
    assert_eq!(s.get("PRODUCT_NAME").map(String::as_str), Some("Scratch"));
    // `macosx` SDKROOT resolves to the active Xcode's absolute macOS SDK path
    // (e.g. `…/MacOSX15.5.sdk` on Xcode 16.4, `…/MacOSX.sdk` on others).
    let sdkroot = s.get("SDKROOT").expect("SDKROOT present");
    assert!(
        sdkroot.contains("MacOSX") && sdkroot.ends_with(".sdk"),
        "SDKROOT = {sdkroot}"
    );
}

#[test]
fn layers_extra_xcconfig_macos() {
    let opts = BuildSettingsOptions {
        xcconfig: Some(xcconfig_fixture("conditional-sdk")),
        ..scratch_opts()
    };
    let s = resolve_one(opts);
    assert_eq!(s.get("FOO").map(String::as_str), Some("macos"));
    assert_eq!(s.get("SWIFT_VERSION").map(String::as_str), Some("5.0"));
}

#[test]
fn layers_extra_xcconfig_iphoneos() {
    let opts = BuildSettingsOptions {
        sdk: "iphoneos".to_string(),
        xcconfig: Some(xcconfig_fixture("conditional-sdk")),
        ..scratch_opts()
    };
    let s = resolve_one(opts);
    assert_eq!(s.get("FOO").map(String::as_str), Some("ios_device"));
}

#[test]
fn keys_projection_trims_output_to_requested_present_keys() {
    // Requesting a projection returns only the requested keys that resolved:
    // present keys keep their resolved values, an unknown requested key is
    // omitted (not inserted empty), and every non-requested key is dropped.
    let opts = BuildSettingsOptions {
        keys: Some(vec![
            "PRODUCT_NAME".to_string(),
            "MACOSX_DEPLOYMENT_TARGET".to_string(),
            "NOT_A_REAL_SETTING".to_string(),
        ]),
        ..scratch_opts()
    };
    let s = resolve_one(opts);
    assert_eq!(s.get("PRODUCT_NAME").map(String::as_str), Some("Scratch"));
    assert_eq!(
        s.get("MACOSX_DEPLOYMENT_TARGET").map(String::as_str),
        Some("12.0")
    );
    assert!(!s.contains_key("NOT_A_REAL_SETTING"));
    assert!(!s.contains_key("SWIFT_VERSION")); // present without projection
    assert_eq!(s.len(), 2);
}

#[test]
fn unknown_target_errors() {
    let opts = BuildSettingsOptions {
        target: Some("Nonexistent".to_string()),
        ..scratch_opts()
    };
    let err = resolve_build_settings(&opts).unwrap_err();
    assert!(err.contains("no target named"), "err: {err}");
}

#[test]
fn destination_collapses_macos_archs() {
    // The collapsed arch is host-derived; pin it so the assertion holds on
    // non-Apple-Silicon hosts too (and to exercise the override itself).
    sweetpad::project::set_host_override(sweetpad::project::HostOverride {
        arch: Some("arm64".into()),
        ..Default::default()
    });
    // No destination on macOS reports the SDK's full standard arch list; a
    // bound macOS destination collapses ARCHS to the active arch — the
    // headline destination-aware behaviour.
    let no_dest = resolve_one(kingfisher_opts());
    assert_eq!(
        no_dest.get("ARCHS").map(String::as_str),
        Some("arm64 x86_64")
    );

    let opts = BuildSettingsOptions {
        destination: parse_destination_arg("platform=macOS"),
        ..kingfisher_opts()
    };
    let dest = resolve_one(opts);
    assert_eq!(dest.get("ARCHS").map(String::as_str), Some("arm64"));
}

#[test]
fn destination_supplies_platform() {
    // An `id=`-only simulator destination (the common IDE case) supplies the
    // SDK with no explicit `sdk`, and still resolves catalog-backed keys.
    let opts = BuildSettingsOptions {
        destination: parse_destination_arg("platform=iOS Simulator,id=ABC-123"),
        ..kingfisher_opts()
    };
    assert!(opts.destination.is_some(), "destination should parse");
    let s = resolve_one(opts);
    assert_eq!(
        s.get("PLATFORM_NAME").map(String::as_str),
        Some("iphonesimulator")
    );
    assert_eq!(
        s.get("WRAPPER_NAME").map(String::as_str),
        Some("Kingfisher.framework")
    );
}

#[test]
fn invalid_destination_is_rejected_at_parse() {
    // Each caller parses the destination string; an unknown platform yields
    // `None` (the CLI surfaced this as "invalid --destination").
    assert!(parse_destination_arg("platform=Android").is_none());
}

// ----- scheme discovery parity (autocreated / user / workspace-level) -------

/// A unique scratch dir under the OS temp dir containing a copy of the
/// synthetic `Scratch.xcodeproj` (one `Scratch` tool target, no scheme files).
fn scratch_copy(tag: &str) -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("sweetpad-bs-{tag}-{}-{n}", std::process::id()));
    let proj = root.join("Scratch.xcodeproj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::copy(
        scratch_proj().join("project.pbxproj"),
        proj.join("project.pbxproj"),
    )
    .unwrap();
    (root, proj)
}

/// A minimal `.xcscheme` whose BuildAction builds the `Scratch` target.
const SCRATCH_SCHEME_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="1640" version="1.7">
   <BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES">
      <BuildActionEntries>
         <BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES">
            <BuildableReference
               BuildableIdentifier="primary"
               BlueprintIdentifier="14A71A1C6762522AADB33EF1"
               BuildableName="Scratch"
               BlueprintName="Scratch"
               ReferencedContainer="container:Scratch.xcodeproj">
            </BuildableReference>
         </BuildActionEntry>
      </BuildActionEntries>
   </BuildAction>
</Scheme>
"#;

fn write_scheme(dir: &PathBuf, name: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{name}.xcscheme")), SCRATCH_SCHEME_XML).unwrap();
}

#[test]
fn scheme_without_file_resolves_the_same_named_target() {
    // No `.xcscheme` exists anywhere — Xcode's autocreated per-target scheme.
    // xcodebuild resolves it as the same-named target; so do we.
    let (_root, proj) = scratch_copy("autocreated");
    let opts = BuildSettingsOptions {
        project: Some(proj),
        scheme: Some("Scratch".to_string()),
        configuration: "Debug".to_string(),
        sdk: "macosx".to_string(),
        arch: "arm64".to_string(),
        ..Default::default()
    };
    let s = resolve_one(opts);
    assert_eq!(s.get("PRODUCT_NAME").map(String::as_str), Some("Scratch"));
}

#[test]
fn unknown_scheme_with_no_matching_target_errors() {
    let (_root, proj) = scratch_copy("unknown-scheme");
    let opts = BuildSettingsOptions {
        project: Some(proj),
        scheme: Some("Nonexistent".to_string()),
        configuration: "Debug".to_string(),
        sdk: "macosx".to_string(),
        arch: "arm64".to_string(),
        ..Default::default()
    };
    let err = resolve_build_settings(&opts).unwrap_err();
    assert!(err.contains("no target named"), "err: {err}");
}

#[test]
fn user_scheme_in_xcuserdata_resolves() {
    // A per-user (non-shared) scheme under `xcuserdata/<user>.xcuserdatad/`
    // resolves exactly like a shared one (xcodebuild accepts both).
    let (_root, proj) = scratch_copy("user-scheme");
    write_scheme(
        &proj.join("xcuserdata/tester.xcuserdatad/xcschemes"),
        "Custom",
    );
    let opts = BuildSettingsOptions {
        project: Some(proj),
        scheme: Some("Custom".to_string()),
        configuration: "Debug".to_string(),
        sdk: "macosx".to_string(),
        arch: "arm64".to_string(),
        ..Default::default()
    };
    let s = resolve_one(opts);
    assert_eq!(s.get("TARGET_NAME").map(String::as_str), Some("Scratch"));
}

#[test]
fn workspace_level_scheme_resolves() {
    // A scheme stored in the workspace bundle's own `xcshareddata/xcschemes/`
    // (not in any member project) — its buildables dispatch to the member
    // project named by `ReferencedContainer`.
    let (root, _proj) = scratch_copy("ws-scheme");
    let ws = root.join("Test.xcworkspace");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(
        ws.join("contents.xcworkspacedata"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Workspace version=\"1.0\">\n  <FileRef location=\"group:Scratch.xcodeproj\"/>\n</Workspace>\n",
    )
    .unwrap();
    write_scheme(&ws.join("xcshareddata/xcschemes"), "WsScheme");

    let opts = BuildSettingsOptions {
        workspace: Some(ws),
        scheme: Some("WsScheme".to_string()),
        configuration: "Debug".to_string(),
        sdk: "macosx".to_string(),
        arch: "arm64".to_string(),
        ..Default::default()
    };
    let s = resolve_one(opts);
    assert_eq!(s.get("TARGET_NAME").map(String::as_str), Some("Scratch"));
}
