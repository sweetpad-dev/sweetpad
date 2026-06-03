//! Library-level coverage of `.xcconfig` resolution
//! (`resolver::flatten_xcconfig` + `resolver::resolve`) — previously exercised
//! through the (removed) CLI `resolve` command.

use std::collections::BTreeMap;
use std::path::PathBuf;

use sweetpad::resolver::{self, ResolveContext};

fn xcconfig_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "fixtures/_synthetic-xcconfigs/xcode-26.5.0/xcconfigs/{name}.xcconfig"
    ))
}

fn resolve(name: &str, sdk: &str) -> BTreeMap<String, String> {
    let assignments = resolver::flatten_xcconfig(&xcconfig_fixture(name)).unwrap();
    let ctx = ResolveContext {
        sdk: sdk.to_string(),
        arch: "arm64".to_string(),
        configuration: "Debug".to_string(),
        variant: String::new(),
    };
    resolver::resolve(&[&assignments], &ctx)
}

#[test]
fn conditional_sdk_macos() {
    assert_eq!(
        resolve("conditional-sdk", "macosx")
            .get("FOO")
            .map(String::as_str),
        Some("macos")
    );
}

#[test]
fn conditional_sdk_iphoneos() {
    assert_eq!(
        resolve("conditional-sdk", "iphoneos")
            .get("FOO")
            .map(String::as_str),
        Some("ios_device")
    );
}

#[test]
fn include_brings_in_other_xcconfig() {
    let r = resolve("include-directive", "macosx");
    assert_eq!(r.get("FOO").map(String::as_str), Some("macos"));
    assert_eq!(r.get("EXTRA").map(String::as_str), Some("layered"));
}
