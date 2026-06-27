use std::path::PathBuf;

use sweetpad_lib::resolver::{ResolveContext, flatten_xcconfig, resolve};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn xcconfig_path(name: &str) -> PathBuf {
    fixtures_root().join(format!(
        "_synthetic-xcconfigs/xcode-26.5.0/xcconfigs/{name}.xcconfig"
    ))
}

fn ctx(sdk: &str, arch: &str, configuration: &str) -> ResolveContext {
    ResolveContext {
        sdk: sdk.into(),
        arch: arch.into(),
        configuration: configuration.into(),
        variant: String::new(),
    }
}

#[test]
fn conditional_sdk_macos() {
    let ass = flatten_xcconfig(&xcconfig_path("conditional-sdk")).unwrap();
    let r = resolve(&[&ass], &ctx("macosx", "arm64", "Debug"));
    assert_eq!(r.get("FOO").map(String::as_str), Some("macos"));
}

#[test]
fn conditional_sdk_iphoneos() {
    let ass = flatten_xcconfig(&xcconfig_path("conditional-sdk")).unwrap();
    let r = resolve(&[&ass], &ctx("iphoneos", "arm64", "Debug"));
    assert_eq!(r.get("FOO").map(String::as_str), Some("ios_device"));
}

#[test]
fn conditional_sdk_iphonesimulator() {
    let ass = flatten_xcconfig(&xcconfig_path("conditional-sdk")).unwrap();
    let r = resolve(&[&ass], &ctx("iphonesimulator", "arm64", "Debug"));
    assert_eq!(r.get("FOO").map(String::as_str), Some("ios_sim"));
}

#[test]
fn conditional_sdk_unknown_falls_back_to_base() {
    let ass = flatten_xcconfig(&xcconfig_path("conditional-sdk")).unwrap();
    let r = resolve(&[&ass], &ctx("driverkit", "arm64", "Debug"));
    assert_eq!(r.get("FOO").map(String::as_str), Some("base"));
}

#[test]
fn conditional_arch_arm64() {
    let ass = flatten_xcconfig(&xcconfig_path("conditional-arch")).unwrap();
    let r = resolve(&[&ass], &ctx("macosx", "arm64", "Debug"));
    assert_eq!(r.get("BAR").map(String::as_str), Some("arm64_val"));
}

#[test]
fn conditional_arch_x86_64() {
    let ass = flatten_xcconfig(&xcconfig_path("conditional-arch")).unwrap();
    let r = resolve(&[&ass], &ctx("macosx", "x86_64", "Debug"));
    assert_eq!(r.get("BAR").map(String::as_str), Some("x86_64_val"));
}

#[test]
fn conditional_config_debug() {
    let ass = flatten_xcconfig(&xcconfig_path("conditional-config")).unwrap();
    let r = resolve(&[&ass], &ctx("macosx", "arm64", "Debug"));
    assert_eq!(r.get("BAZ").map(String::as_str), Some("debug_val"));
}

#[test]
fn conditional_config_release() {
    let ass = flatten_xcconfig(&xcconfig_path("conditional-config")).unwrap();
    let r = resolve(&[&ass], &ctx("macosx", "arm64", "Release"));
    assert_eq!(r.get("BAZ").map(String::as_str), Some("release_val"));
}

#[test]
fn multi_line_continuation_value() {
    let ass = flatten_xcconfig(&xcconfig_path("multi-line-continuation")).unwrap();
    let r = resolve(&[&ass], &ctx("macosx", "arm64", "Debug"));
    assert_eq!(
        r.get("QUUX").map(String::as_str),
        Some("first_part second_part third_part")
    );
}

#[test]
fn modifier_syntax_full() {
    let ass = flatten_xcconfig(&xcconfig_path("modifier-syntax")).unwrap();
    let r = resolve(&[&ass], &ctx("macosx", "arm64", "Debug"));
    assert_eq!(r.get("BASE_NAME").map(String::as_str), Some("HelloWorld"));
    assert_eq!(r.get("LOWER_NAME").map(String::as_str), Some("helloworld"));
    assert_eq!(r.get("UPPER_NAME").map(String::as_str), Some("HELLOWORLD"));
    assert_eq!(r.get("DEFAULTED").map(String::as_str), Some("fallback"));
}

#[test]
fn inherited_without_lower_layer_starts_with_space() {
    // With no upstream layer, $(inherited) expands to "", leaving a leading space.
    // This matches xcodebuild's exact output captured in the fixture.
    let ass = flatten_xcconfig(&xcconfig_path("inherited")).unwrap();
    let r = resolve(&[&ass], &ctx("macosx", "arm64", "Debug"));
    assert_eq!(
        r.get("OTHER_LDFLAGS").map(String::as_str),
        Some(" -framework Foundation")
    );
    assert_eq!(
        r.get("OTHER_SWIFT_FLAGS").map(String::as_str),
        Some(" -DMY_FLAG")
    );
}

#[test]
fn include_directive_brings_in_referenced_xcconfig() {
    let ass = flatten_xcconfig(&xcconfig_path("include-directive")).unwrap();
    let r = resolve(&[&ass], &ctx("macosx", "arm64", "Debug"));
    // Included file contributes FOO; this file contributes EXTRA.
    assert_eq!(r.get("FOO").map(String::as_str), Some("macos"));
    assert_eq!(r.get("EXTRA").map(String::as_str), Some("layered"));
}
