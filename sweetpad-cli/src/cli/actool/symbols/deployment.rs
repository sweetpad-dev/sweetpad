//! Swift asset resource types entered the SDKs in the 2023 OS releases.
//! Earlier deployment targets get bundle-backed types in the generated module.
use super::{Frameworks, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceApi {
    System,
    BackDeployed,
}

const SYSTEM_AVAILABILITY: &str = "@available(iOS 17.0, macOS 14.0, tvOS 17.0, watchOS 10.0, *)";

impl ResourceApi {
    pub fn for_target(platform: Option<&str>, minimum: Option<&str>, mac_catalyst: bool) -> Self {
        let first_system_major = match platform {
            Some("iphoneos" | "iphonesimulator" | "appletvos" | "appletvsimulator") => 17,
            Some("macosx") if mac_catalyst => 17,
            Some("macosx") => 14,
            Some("watchos" | "watchsimulator") => 10,
            // All visionOS versions provide the SDK types. With no target,
            // preserve the standalone generator's SDK-based output.
            _ => return Self::System,
        };
        let major = minimum.and_then(|v| v.split('.').next()?.parse::<u32>().ok());
        if major.is_some_and(|v| v < first_system_major) {
            Self::BackDeployed
        } else {
            Self::System
        }
    }

    pub(super) fn resource_type(self, kind: Kind) -> &'static str {
        match (self, kind) {
            (Self::System, Kind::Color) => "DeveloperToolsSupport.ColorResource",
            (Self::System, _) => "DeveloperToolsSupport.ImageResource",
            (Self::BackDeployed, Kind::Color) => "ColorResource",
            (Self::BackDeployed, _) => "ImageResource",
        }
    }

    pub(super) fn resource_availability(self, kind: Kind) -> &'static str {
        match (self, kind) {
            (Self::System, _) => SYSTEM_AVAILABILITY,
            (Self::BackDeployed, Kind::Color) => "@available(iOS 11.0, macOS 10.13, tvOS 11.0, *)",
            (Self::BackDeployed, _) => "@available(iOS 11.0, macOS 10.7, tvOS 11.0, *)",
        }
    }

    pub(super) fn appkit_availability(self, kind: Kind) -> &'static str {
        match (self, kind) {
            (Self::System, _) => "@available(macOS 14.0, *)\n@available(macCatalyst, unavailable)",
            (Self::BackDeployed, Kind::Color) => {
                "@available(macOS 10.13, *)\n@available(macCatalyst, unavailable)"
            }
            (Self::BackDeployed, _) => {
                "@available(macOS 10.7, *)\n@available(macCatalyst, unavailable)"
            }
        }
    }

    pub(super) fn uikit_availability(self) -> &'static str {
        match self {
            Self::System => "@available(iOS 17.0, tvOS 17.0, *)\n@available(watchOS, unavailable)",
            Self::BackDeployed => {
                "@available(iOS 11.0, tvOS 11.0, *)\n@available(watchOS, unavailable)"
            }
        }
    }

    pub(super) fn swiftui_availability(self) -> &'static str {
        match self {
            Self::System => SYSTEM_AVAILABILITY,
            Self::BackDeployed => "@available(iOS 13.0, macOS 10.15, tvOS 13.0, watchOS 6.0, *)",
        }
    }
}

// Kept as Swift so the exact initializers and their platform guards are readable.
pub(super) fn backwards_support(out: &mut String, frameworks: Frameworks) {
    out.push_str(include_str!("resources.swift").trim_end());
    for (enabled, source) in [
        (frameworks.appkit, include_str!("appkit.swift")),
        (frameworks.uikit, include_str!("uikit.swift")),
        (frameworks.swiftui, include_str!("swiftui.swift")),
    ] {
        if enabled {
            out.push_str("\n\n");
            out.push_str(source.trim_end());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ResourceApi::{self, BackDeployed, System};

    #[test]
    fn uses_the_sdk_types_starting_with_each_platforms_first_supported_release() {
        for (platform, mac_catalyst, last_old, first_new) in [
            ("iphoneos", false, "16.7.10", "17.0"),
            ("iphonesimulator", false, "15.6", "17"),
            ("appletvos", false, "16.6", "17.0"),
            ("appletvsimulator", false, "16.6", "17.0"),
            ("macosx", false, "13.6.1", "14"),
            ("macosx", true, "16.4", "17.0"),
            ("watchos", false, "9.6", "10.0"),
            ("watchsimulator", false, "9.6", "10.0"),
        ] {
            assert_eq!(
                ResourceApi::for_target(Some(platform), Some(last_old), mac_catalyst),
                BackDeployed
            );
            assert_eq!(
                ResourceApi::for_target(Some(platform), Some(first_new), mac_catalyst),
                System
            );
        }
        for platform in ["xros", "xrsimulator"] {
            assert_eq!(
                ResourceApi::for_target(Some(platform), Some("1.0"), false),
                System
            );
        }
        assert_eq!(ResourceApi::for_target(None, None, false), System);
        assert_eq!(
            ResourceApi::for_target(Some("iphoneos"), None, false),
            System
        );
    }
}
