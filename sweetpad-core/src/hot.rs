//! What a hot-reload build and launch need, shared by the CLI's `app run
//! --hot` and the extension's hot reload: the SDKs InjectionNext can inject
//! into, the client dylib and platform directory for each, and the build
//! settings that make the product injectable.

use std::path::Path;

/// The SDK a `-destination` builds for, among the ones hot reload can inject
/// into: the simulators InjectionNext ships a client for, and native macOS.
/// `None` for the rest (devices, watchOS, generic destinations).
#[must_use]
pub fn sdk_for_destination(destination: &str) -> Option<&'static str> {
    let spec = sweetpad_lib::destination::DestinationSpec::parse(destination);
    if spec.generic {
        return None;
    }
    spec.sdk().filter(|sdk| {
        matches!(
            *sdk,
            "iphonesimulator" | "appletvsimulator" | "xrsimulator" | "macosx"
        )
    })
}

/// The InjectionNext client dylib that injects into apps built for `sdk`.
/// `None` for SDKs InjectionNext can't inject into: devices strip
/// `DYLD_INSERT_LIBRARIES`, and watchOS ships no dylib.
#[must_use]
pub fn dylib_name(sdk: &str) -> Option<&'static str> {
    match sdk {
        "iphonesimulator" => Some("libiphonesimulatorInjection.dylib"),
        "appletvsimulator" => Some("libappletvsimulatorInjection.dylib"),
        "xrsimulator" => Some("libxrsimulatorInjection.dylib"),
        "macosx" => Some("libmacosxInjection.dylib"),
        _ => None,
    }
}

/// The `<Platform>.platform` directory an injectable `sdk` lives in, where
/// the InjectionNext.app client finds the XCTest it links.
#[must_use]
pub fn platform_dir(sdk: &str) -> Option<&'static str> {
    dylib_name(sdk).map(|_| sweetpad_lib::project::platform_dir_name_for(sdk))
}

/// The build settings a hot build for `sdk` adds, in order, as `KEY=VALUE`
/// overrides that outrank the project's. Empty when hot reload can't inject
/// into `sdk`.
///
/// Every injectable build links with `-interposable`, so dyld can swap
/// symbols, and emits frontend command lines, so the recompiler can recover
/// each file's compile command from the build log. A macOS app must also be
/// injectable: the hardened runtime makes dyld strip `DYLD_INSERT_LIBRARIES`
/// and library validation reject the recompiled dylibs, and the App Sandbox
/// blocks the client's socket and loading from outside its container. Without
/// these the app launches and injection fails silently. An entitlements file
/// that turns the sandbox on outranks `ENABLE_APP_SANDBOX`, so a macOS build
/// signs with `entitlements` (a copy without the sandbox) when given.
#[must_use]
pub fn build_settings(sdk: &str, entitlements: Option<&Path>) -> Vec<(&'static str, String)> {
    if dylib_name(sdk).is_none() {
        return Vec::new();
    }
    // `$(inherited)` keeps the project's own linker flags.
    let mut settings = vec![
        (
            "OTHER_LDFLAGS",
            "$(inherited) -Xlinker -interposable".to_string(),
        ),
        ("EMIT_FRONTEND_COMMAND_LINES", "YES".to_string()),
    ];
    if sdk == "macosx" {
        settings.push(("ENABLE_HARDENED_RUNTIME", "NO".to_string()));
        settings.push(("ENABLE_APP_SANDBOX", "NO".to_string()));
        if let Some(entitlements) = entitlements {
            settings.push(("CODE_SIGN_ENTITLEMENTS", entitlements.display().to_string()));
        }
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_for_destination_maps_injectable_destinations() {
        assert_eq!(
            sdk_for_destination("platform=iOS Simulator,id=ABC"),
            Some("iphonesimulator")
        );
        assert_eq!(
            sdk_for_destination("platform=visionOS Simulator,name=X"),
            Some("xrsimulator")
        );
        assert_eq!(sdk_for_destination("platform=macOS"), Some("macosx"));
        // Physical device / unknown → unsupported.
        assert_eq!(sdk_for_destination("platform=iOS,id=ABC"), None);
        assert_eq!(sdk_for_destination("generic/platform=iOS"), None);
    }

    #[test]
    fn dylib_and_platform_cover_the_injectable_sdks() {
        for (sdk, dylib, platform) in [
            (
                "iphonesimulator",
                "libiphonesimulatorInjection.dylib",
                "iPhoneSimulator",
            ),
            (
                "appletvsimulator",
                "libappletvsimulatorInjection.dylib",
                "AppleTVSimulator",
            ),
            (
                "xrsimulator",
                "libxrsimulatorInjection.dylib",
                "XRSimulator",
            ),
            ("macosx", "libmacosxInjection.dylib", "MacOSX"),
        ] {
            assert_eq!(dylib_name(sdk), Some(dylib));
            assert_eq!(platform_dir(sdk), Some(platform));
        }
        // Devices, watchOS and unknown SDKs aren't injectable.
        for sdk in [
            "iphoneos",
            "appletvos",
            "xros",
            "watchos",
            "watchsimulator",
            "",
        ] {
            assert_eq!(dylib_name(sdk), None, "{sdk}");
            assert_eq!(platform_dir(sdk), None, "{sdk}");
            assert!(build_settings(sdk, None).is_empty(), "{sdk}");
        }
    }

    #[test]
    fn a_simulator_build_links_interposable_and_emits_command_lines() {
        assert_eq!(
            build_settings("iphonesimulator", Some(Path::new("/ignored.entitlements"))),
            [
                (
                    "OTHER_LDFLAGS",
                    "$(inherited) -Xlinker -interposable".to_string()
                ),
                ("EMIT_FRONTEND_COMMAND_LINES", "YES".to_string()),
            ]
        );
    }

    #[test]
    fn a_macos_build_drops_the_hardened_runtime_and_sandbox() {
        let keys =
            |s: Vec<(&'static str, String)>| s.into_iter().map(|(k, _)| k).collect::<Vec<_>>();
        assert_eq!(
            keys(build_settings("macosx", None)),
            [
                "OTHER_LDFLAGS",
                "EMIT_FRONTEND_COMMAND_LINES",
                "ENABLE_HARDENED_RUNTIME",
                "ENABLE_APP_SANDBOX"
            ]
        );
        let stripped = build_settings("macosx", Some(Path::new("/cache/Debug.entitlements")));
        assert_eq!(
            stripped.last(),
            Some(&(
                "CODE_SIGN_ENTITLEMENTS",
                "/cache/Debug.entitlements".to_string()
            ))
        );
        assert!(
            stripped
                .iter()
                .any(|(k, v)| *k == "ENABLE_HARDENED_RUNTIME" && v == "NO")
        );
    }
}
