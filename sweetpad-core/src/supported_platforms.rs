//! Which platforms a scheme can build for, the filter behind the CLI's and
//! the VS Code extension's destination pickers.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sweetpad_lib::destination::Platform;
use sweetpad_lib::{project, workspace};

use crate::app_locator::find_scheme;

/// The platform tokens a scheme's targets can build for: the union of their
/// *authored* `SUPPORTED_PLATFORMS`, falling back to the authored `SDKROOT`
/// (a device SDK implies its simulator sibling), read straight from the
/// project's setting layers. Drives the destination pickers' filtering, so a
/// macOS-only app isn't offered a wall of iPhone simulators. Guessing wrong
/// can only ever *widen* the list: resolution failure means no filter, and an
/// explicit destination bypasses the picker entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportedPlatforms(BTreeSet<String>);

impl SupportedPlatforms {
    /// Resolve the platform tokens `scheme` in `container` (a `.xcodeproj`
    /// or `.xcworkspace`) builds for under `configuration`; `None` (no
    /// filtering) for a Swift package, unreadable projects, or when no target
    /// authors either setting. Reads the raw setting layers rather than the
    /// full settings resolver: with no destination settled yet the resolver
    /// would bind a default platform, overriding the very value being
    /// discovered.
    ///
    /// Only the scheme's own targets count. Every target in the container
    /// would union an iOS sibling's tokens into a mac-only scheme, and the
    /// mac-only answer is the one that decides whether a build goes to the
    /// Mac or to a device platform. A scheme with no file to read (an
    /// autocreated one) counts every target in the container.
    #[must_use]
    pub fn resolve(container: &Path, scheme: &str, configuration: &str) -> Option<Self> {
        let projects: Vec<PathBuf> = match container.extension().and_then(|e| e.to_str()) {
            Some("xcodeproj") => vec![container.to_path_buf()],
            Some("xcworkspace") => workspace::open(container).ok()?.project_refs,
            _ => return None,
        };
        let scheme_targets = scheme_build_targets(container, scheme);
        let mut tokens = BTreeSet::new();
        for proj in &projects {
            let Ok(opened) = project::open(proj) else {
                continue;
            };
            for target in &opened.targets {
                if scheme_targets
                    .as_ref()
                    .is_some_and(|names| !names.contains(&target.name))
                {
                    continue;
                }
                let Ok(layers) = project::build_settings_layers(proj, &target.name, configuration)
                else {
                    continue;
                };
                match project::last_unconditional_setting(&layers, "SUPPORTED_PLATFORMS") {
                    Some(platforms) => tokens.extend(
                        platforms
                            .split_whitespace()
                            .filter(|t| !t.contains('$'))
                            .map(str::to_string),
                    ),
                    None => {
                        if let Some(sdk) = project::natural_sdkroot(&layers) {
                            tokens.extend(
                                Platform::sdk_family_tokens(&sdk)
                                    .into_iter()
                                    .map(str::to_string),
                            );
                        }
                    }
                }
            }
        }
        (!tokens.is_empty()).then_some(Self(tokens))
    }

    /// A set of the given tokens.
    #[must_use]
    pub fn from_tokens(tokens: &[&str]) -> Self {
        Self(tokens.iter().map(ToString::to_string).collect())
    }

    /// The tokens, as `SUPPORTED_PLATFORMS` spells them (`iphonesimulator`).
    pub fn tokens(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    /// Whether the scheme can run on this Mac.
    #[must_use]
    pub fn allows_mac(&self) -> bool {
        self.0.contains("macosx")
    }

    /// Whether a simulator of this OS family (`simctl`'s `iOS` / `watchOS` /
    /// `tvOS` / `xrOS`) can run the scheme. Unknown families stay visible:
    /// filtering must never hide something it doesn't understand.
    #[must_use]
    pub fn allows_simulator(&self, os: &str) -> bool {
        Platform::simulator_for_os(os).is_none_or(|p| self.0.contains(p.sdk))
    }

    /// Whether the Mac is the only destination the scheme supports: the case
    /// that can skip listing simulators entirely, and the signal `archive`
    /// uses to pick a generic macOS destination instead of defaulting to iOS.
    #[must_use]
    pub fn is_mac_only(&self) -> bool {
        self.allows_mac() && !self.0.iter().any(|t| t.ends_with("simulator"))
    }
}

/// The target names `scheme` builds, or `None` when there is no scheme file to
/// read: an autocreated scheme Xcode never materialized, or a name that
/// doesn't resolve. `None` means "don't filter", so a missing file falls back
/// to every target in the container rather than to an empty set.
///
/// Entries count regardless of which action they build for. A per-action set
/// (Run vs Test vs Archive) could only ever narrow this further, and a
/// narrower set makes a scheme look *more* platform-specific than it is, the
/// wrong direction to guess in, since over-narrowing would send a build to a
/// platform the scheme can't produce.
fn scheme_build_targets(container: &Path, scheme: &str) -> Option<BTreeSet<String>> {
    let parsed = find_scheme(container, scheme)?;
    let names: BTreeSet<String> = parsed
        .build_entries
        .iter()
        .map(|e| e.buildable.blueprint_name.clone())
        .collect();
    (!names.is_empty()).then_some(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::ScratchDir;

    #[test]
    fn mac_only_targets_are_recognised_for_archive() {
        // `archive` picks a generic macOS destination from this; getting it
        // wrong sends a mac-only project to `generic/platform=iOS`, which
        // fails inside xcodebuild.
        assert!(SupportedPlatforms::from_tokens(&["macosx"]).is_mac_only());
        // Catalyst/multiplatform targets also build for a simulator, so the
        // iOS default stays correct for them.
        assert!(!SupportedPlatforms::from_tokens(&["macosx", "iphonesimulator"]).is_mac_only());
        assert!(!SupportedPlatforms::from_tokens(&["iphoneos", "iphonesimulator"]).is_mac_only());
        // No tokens resolved: no opinion, keep the existing default.
        assert!(!SupportedPlatforms::from_tokens(&[]).is_mac_only());
    }

    #[test]
    fn supported_platforms_map_simulator_families_and_mac() {
        let mac_only = SupportedPlatforms::from_tokens(&["macosx"]);
        assert!(mac_only.allows_mac());
        assert!(!mac_only.allows_simulator("iOS"));

        let ios = SupportedPlatforms::from_tokens(&["iphoneos", "iphonesimulator"]);
        assert!(!ios.allows_mac());
        assert!(ios.allows_simulator("iOS"));
        assert!(!ios.allows_simulator("watchOS"));

        let multi = SupportedPlatforms::from_tokens(&["macosx", "iphonesimulator", "iphoneos"]);
        assert!(multi.allows_mac());
        assert!(multi.allows_simulator("iOS"));

        // An OS family the filter doesn't understand stays visible.
        assert!(mac_only.allows_simulator("futureOS"));
    }

    #[test]
    fn sdk_platform_tokens_bring_the_simulator_sibling() {
        let tokens = Platform::sdk_family_tokens;
        assert_eq!(tokens("macosx"), vec!["macosx"]);
        assert_eq!(tokens("iphoneos17.5"), vec!["iphoneos", "iphonesimulator"]);
        assert_eq!(tokens("xros"), vec!["xros", "xrsimulator"]);
        assert_eq!(
            tokens(
                "/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX15.2.sdk"
            ),
            vec!["macosx"]
        );
        assert!(tokens("somethingelse").is_empty());
    }

    /// simctl names visionOS's runtime `xrOS`, devicectl names it `visionOS`;
    /// both reach the one simulator token through the platform table.
    #[test]
    fn both_visionos_spellings_filter_on_the_xrsimulator_token() {
        let vision = SupportedPlatforms::from_tokens(&["xros", "xrsimulator"]);
        assert!(vision.allows_simulator("xrOS"));
        assert!(vision.allows_simulator("visionOS"));
        assert!(!vision.allows_simulator("iOS"));
    }

    #[test]
    fn a_swift_package_has_no_filter() {
        assert_eq!(
            SupportedPlatforms::resolve(Path::new("/x/Package.swift"), "App", "Debug"),
            None
        );
    }

    /// Write `<name>.xcscheme` with one `BuildActionEntry` per target.
    fn write_scheme(container: &Path, name: &str, targets: &[&str]) {
        use std::fmt::Write as _;
        let dir = container.join("xcshareddata/xcschemes");
        std::fs::create_dir_all(&dir).unwrap();
        let mut entries = String::new();
        for t in targets {
            let _ = write!(
                entries,
                r#"<BuildActionEntry buildForRunning="YES" buildForTesting="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES">
<BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="ID{t}" BuildableName="{t}.app" BlueprintName="{t}" ReferencedContainer="container:App.xcodeproj"/>
</BuildActionEntry>"#
            );
        }
        std::fs::write(
            dir.join(format!("{name}.xcscheme")),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="1600" version="1.7">
<BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES">
<BuildActionEntries>{entries}</BuildActionEntries>
</BuildAction>
</Scheme>"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn scheme_build_targets_reads_only_that_scheme() {
        let dir = ScratchDir::new("sweetpad-scheme-targets").unwrap();
        let proj = dir.join("App.xcodeproj");
        std::fs::create_dir_all(&proj).unwrap();
        write_scheme(&proj, "MacApp", &["MacApp"]);
        write_scheme(&proj, "iOSApp", &["iOSApp", "iOSAppTests"]);

        // Each scheme sees its own targets, not the container's union: the
        // whole point, since the union is what sent a mac-only scheme to
        // `generic/platform=iOS`.
        let mac = scheme_build_targets(&proj, "MacApp").unwrap();
        assert_eq!(
            mac.iter().map(String::as_str).collect::<Vec<_>>(),
            ["MacApp"]
        );
        let ios = scheme_build_targets(&proj, "iOSApp").unwrap();
        assert_eq!(
            ios.iter().map(String::as_str).collect::<Vec<_>>(),
            ["iOSApp", "iOSAppTests"]
        );

        // A scheme with no file on disk (autocreated, or simply absent) must
        // read as "don't filter" rather than as an empty target set, which
        // would resolve no platforms at all.
        assert!(scheme_build_targets(&proj, "Ghost").is_none());
    }
}
