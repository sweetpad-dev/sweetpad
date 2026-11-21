//! The parts of Xcode an SDK is made from, as paths inside `Xcode.app`:
//! what `sdk export` packs on the Mac, what an import takes out of an
//! Xcode archive, and what [`super::super::sdk_bundle`] copies from them.

/// The default toolchain, inside `Contents`.
pub(crate) const TOOLCHAIN: &str = "Developer/Toolchains/XcodeDefault.xctoolchain";
/// The toolchain's resource folders under `usr/lib`; `usr/lib/swift-*`
/// (back-deployment runtimes) and `usr/include/swift` come along too.
pub(crate) const TOOLCHAIN_LIBS: [&str; 3] = ["swift", "swift_static", "clang"];
/// What an SDK keeps of each platform's folder.
pub(crate) const PLATFORM_PARTS: [&str; 10] = [
    "Info.plist",
    "version.plist",
    "PlatformBasicInfo.plist",
    "Developer/SDKs",
    "Developer/Library/Xcode/Specifications",
    "Developer/Library/Xcode/Agents/XCTRunner.app",
    "Developer/Library/Frameworks",
    "Developer/Library/PrivateFrameworks",
    "Developer/usr/lib",
    "Library/Application Support",
];
/// Compiler caches of large, machine-specific prebuilt modules, left out
/// wherever they turn up.
pub(crate) const SKIPPED: &str = "prebuilt-modules";

/// Where an archive entry goes, as `Xcode.app/Contents/...`, when an SDK
/// needs it: `name` is the entry's path, under whatever the archive calls
/// the app.
pub(crate) fn selected(name: &str) -> Option<String> {
    let mut parts = name.split('/');
    let app = parts.next()?;
    let is_app = std::path::Path::new(app)
        .extension()
        .is_some_and(|e| e == "app");
    if !is_app || name.split('/').any(|part| part == SKIPPED) {
        return None;
    }
    let relative = name[app.len()..].trim_start_matches('/');
    let keep = || Some(format!("Xcode.app/{relative}"));
    let under = |path: &str, root: &str| {
        path == root
            || path
                .strip_prefix(root)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    if matches!(relative, "Contents/version.plist" | "Contents/Info.plist") {
        return keep();
    }
    if let Some(suffix) = relative.strip_prefix(&format!("Contents/{TOOLCHAIN}/usr/")) {
        if under(suffix, "include/swift")
            || TOOLCHAIN_LIBS
                .iter()
                .any(|lib| under(suffix, &format!("lib/{lib}")))
            || suffix.starts_with("lib/swift-")
        {
            return keep();
        }
        return None;
    }
    let (platform, rest) = relative
        .strip_prefix("Contents/Developer/Platforms/")?
        .split_once('/')?;
    if platform.ends_with(".platform") && PLATFORM_PARTS.iter().any(|root| under(rest, root)) {
        return keep();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::selected;

    #[test]
    fn an_sdk_takes_the_toolchain_resources_and_each_platforms_parts() {
        let toolchain = "Xcode-beta.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr";
        for (name, kept) in [
            ("Xcode-beta.app/Contents/version.plist", true),
            ("Xcode-beta.app/Contents/Info.plist", true),
            ("Xcode-beta.app/Contents/MacOS/Xcode", false),
            (
                &format!("{toolchain}/lib/swift/iphoneos/Swift.swiftmodule"),
                true,
            ),
            (&format!("{toolchain}/lib/swift"), true),
            (&format!("{toolchain}/lib/swiftfoo/x"), false),
            (&format!("{toolchain}/lib/swift-5.5/iphoneos/x.dylib"), true),
            (&format!("{toolchain}/lib/clang/17/include/x.h"), true),
            (&format!("{toolchain}/include/swift/bridging"), true),
            (&format!("{toolchain}/include/swiftly"), false),
            (&format!("{toolchain}/bin/swift-frontend"), false),
            (
                &format!("{toolchain}/lib/swift/iphoneos/prebuilt-modules/27.2/Swift.swiftmodule"),
                false,
            ),
            (
                "Xcode-beta.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk",
                true,
            ),
            (
                "Xcode-beta.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/usr/bin/x",
                false,
            ),
            (
                "Xcode-beta.app/Contents/Developer/Platforms/iPhoneOS.platform/Info.plist",
                true,
            ),
            (
                "Xcode-beta.app/Contents/Developer/Platforms/Info.plist",
                false,
            ),
            ("Payload/Contents/version.plist", false),
        ] {
            assert_eq!(selected(name).is_some(), kept, "{name}");
        }
        assert_eq!(
            selected("Xcode-beta.app/Contents/version.plist").as_deref(),
            Some("Xcode.app/Contents/version.plist")
        );
    }
}
