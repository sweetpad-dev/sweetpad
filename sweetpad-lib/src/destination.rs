//! Run-destination metadata: the device/platform/OS combo a build is
//! targeted at, and the one table of Apple platforms in every spelling the
//! tools use for them.
//!
//! `xcodebuild -showBuildSettings -destination "platform=…,name=…,OS=…"`
//! emits many settings that depend on the run destination (`ARCHS` reduces
//! to a single active arch, `ONLY_ACTIVE_ARCH` flips to `YES`, the
//! `__IS_NOT_SIMULATOR` internal flag toggles, etc.). The captured oracles
//! encode the destination in the filename suffix; we parse that here so
//! [`crate::project::built_in_settings`] can synthesize destination-aware
//! defaults.
//!
//! [`DestinationSpec`] reads a `-destination` argument field by field, for
//! the callers that need its `id=` or `name=` or want to know what kind of
//! place it names. [`Platform`] maps between the SDK name
//! (`iphonesimulator`), the `-destination` label (`iOS Simulator`) and the
//! OS family simctl and devicectl report (`iOS`, `xrOS`, `visionOS`).

/// One Apple platform: an SDK and how each tool names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    /// Canonical SDK name, as `SUPPORTED_PLATFORMS` and `-sdk` spell it:
    /// `iphonesimulator`.
    pub sdk: &'static str,
    /// The `-destination platform=` label: `iOS Simulator`.
    pub label: &'static str,
    /// The OS family, as devicectl's `platform` and `PLATFORM_DISPLAY_NAME`
    /// spell it: `iOS`, `visionOS`. A simulator runtime spells visionOS
    /// `xrOS`, which [`Platform::matches_os`] also accepts.
    pub os: &'static str,
    /// Whether the platform is a simulator.
    pub simulator: bool,
}

/// Every platform a destination can name, device before simulator within a
/// family.
pub const PLATFORMS: [Platform; 10] = [
    Platform {
        sdk: "macosx",
        label: "macOS",
        os: "macOS",
        simulator: false,
    },
    Platform {
        sdk: "iphoneos",
        label: "iOS",
        os: "iOS",
        simulator: false,
    },
    Platform {
        sdk: "iphonesimulator",
        label: "iOS Simulator",
        os: "iOS",
        simulator: true,
    },
    Platform {
        sdk: "appletvos",
        label: "tvOS",
        os: "tvOS",
        simulator: false,
    },
    Platform {
        sdk: "appletvsimulator",
        label: "tvOS Simulator",
        os: "tvOS",
        simulator: true,
    },
    Platform {
        sdk: "watchos",
        label: "watchOS",
        os: "watchOS",
        simulator: false,
    },
    Platform {
        sdk: "watchsimulator",
        label: "watchOS Simulator",
        os: "watchOS",
        simulator: true,
    },
    Platform {
        sdk: "xros",
        label: "visionOS",
        os: "visionOS",
        simulator: false,
    },
    Platform {
        sdk: "xrsimulator",
        label: "visionOS Simulator",
        os: "visionOS",
        simulator: true,
    },
    Platform {
        sdk: "driverkit",
        label: "DriverKit",
        os: "DriverKit",
        simulator: false,
    },
];

impl Platform {
    /// The platform of a canonical SDK name (`iphonesimulator`), ignoring
    /// case. A versioned name (`iphoneos17.5`) is not one; see
    /// [`crate::project::canonicalize_sdk_base`].
    #[must_use]
    pub fn from_sdk(sdk: &str) -> Option<&'static Platform> {
        PLATFORMS.iter().find(|p| p.sdk.eq_ignore_ascii_case(sdk))
    }

    /// The platform a `-destination platform=` label names. xcodebuild reads
    /// the label without regard to case (`platform=ios simulator` works), and
    /// also takes `OS X` and the `xrOS` spelling visionOS shipped under.
    #[must_use]
    pub fn from_label(label: &str) -> Option<&'static Platform> {
        let label = label.trim();
        let sdk = match label.to_ascii_lowercase().as_str() {
            "os x" | "macosx" => "macosx",
            "xros" => "xros",
            "xros simulator" => "xrsimulator",
            _ => {
                return PLATFORMS
                    .iter()
                    .find(|p| p.label.eq_ignore_ascii_case(label));
            }
        };
        Self::from_sdk(sdk)
    }

    /// The simulator platform for an OS family as simctl's runtime names it
    /// (`iOS`, `watchOS`, `tvOS`, `xrOS`) or devicectl does (`visionOS`).
    #[must_use]
    pub fn simulator_for_os(os: &str) -> Option<&'static Platform> {
        PLATFORMS.iter().find(|p| p.simulator && p.matches_os(os))
    }

    /// The device platform for an OS family, spelled as for
    /// [`Platform::simulator_for_os`].
    #[must_use]
    pub fn device_for_os(os: &str) -> Option<&'static Platform> {
        PLATFORMS.iter().find(|p| !p.simulator && p.matches_os(os))
    }

    /// Whether `os` names this platform's OS family, ignoring case. `xrOS`
    /// and `visionOS` are one family.
    #[must_use]
    pub fn matches_os(&self, os: &str) -> bool {
        let os = os.trim();
        self.os.eq_ignore_ascii_case(os)
            || (self.os == "visionOS" && os.eq_ignore_ascii_case("xrOS"))
    }

    /// Whether this is native macOS.
    #[must_use]
    pub fn is_macos(&self) -> bool {
        self.sdk == "macosx"
    }

    /// The platform tokens an SDK brings to `SUPPORTED_PLATFORMS` when a
    /// target authors only `SDKROOT`: a device SDK brings its simulator
    /// sibling, as Xcode's default does. Accepts the short name (`macosx`), a
    /// versioned one (`iphoneos17.5`), or a full SDK path (`…/MacOSX15.2.sdk`).
    /// Empty for anything that isn't a platform.
    #[must_use]
    pub fn sdk_family_tokens(sdk: &str) -> Vec<&'static str> {
        let name = std::path::Path::new(sdk)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(sdk);
        let Some(platform) = Self::from_sdk(&crate::project::canonicalize_sdk_base(name)) else {
            return Vec::new();
        };
        if platform.simulator || platform.is_macos() || platform.sdk == "driverkit" {
            return vec![platform.sdk];
        }
        let mut tokens = vec![platform.sdk];
        tokens.extend(Self::simulator_for_os(platform.os).map(|p| p.sdk));
        tokens
    }
}

/// A `-destination` argument read field by field: `platform=iOS
/// Simulator,id=<udid>,arch=x86_64`, `generic/platform=iOS`. Keys match
/// without regard to case and values are trimmed. Fields this type has no
/// slot for are ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DestinationSpec {
    /// The `platform=` value as written.
    pub platform_label: Option<String>,
    /// The platform that label names, when it is one of [`PLATFORMS`].
    pub platform: Option<&'static Platform>,
    /// `generic/platform=…`: build for the platform without binding a device.
    pub generic: bool,
    pub id: Option<String>,
    pub name: Option<String>,
    /// The `OS=` value as written (`latest` included).
    pub os: Option<String>,
    pub arch: Option<String>,
    /// `variant=Mac Catalyst` and friends.
    pub variant: Option<String>,
}

impl DestinationSpec {
    /// Read a `-destination` argument. Never fails: a field that isn't there
    /// is `None`, and so is the platform when its label is unknown.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        let mut spec = Self::default();
        for field in s.split(',') {
            let Some((key, value)) = field.split_once('=') else {
                continue;
            };
            let value = value.trim().to_string();
            match key.trim().to_ascii_lowercase().as_str() {
                "platform" => spec.platform_label = Some(value),
                "generic/platform" => {
                    spec.platform_label = Some(value);
                    spec.generic = true;
                }
                "id" => spec.id = Some(value),
                "name" => spec.name = Some(value),
                "os" => spec.os = Some(value),
                "arch" => spec.arch = Some(value),
                "variant" => spec.variant = Some(value),
                _ => {}
            }
        }
        spec.platform = spec
            .platform_label
            .as_deref()
            .and_then(Platform::from_label);
        spec
    }

    /// The SDK the platform builds against (`iphonesimulator`), when the
    /// platform is known.
    #[must_use]
    pub fn sdk(&self) -> Option<&'static str> {
        self.platform.map(|p| p.sdk)
    }

    /// Whether the destination is native macOS (a Catalyst `variant=`
    /// included, since it runs as a Mac app).
    #[must_use]
    pub fn is_macos(&self) -> bool {
        self.platform.is_some_and(Platform::is_macos)
    }

    /// Whether the destination is a simulator, concrete or generic.
    #[must_use]
    pub fn is_simulator(&self) -> bool {
        self.platform.is_some_and(|p| p.simulator)
    }

    /// Whether the destination names physical hardware: a concrete
    /// destination whose platform is neither a simulator nor macOS. A label
    /// this table doesn't know counts as hardware, so a remembered device
    /// on a platform newer than the table is never mistaken for a deleted
    /// simulator.
    #[must_use]
    pub fn is_device(&self) -> bool {
        !self.generic && self.platform_label.is_some() && !self.is_simulator() && !self.is_macos()
    }
}

/// Where a build is targeted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDestination {
    /// Canonical SDK platform name: `macosx`, `iphonesimulator`,
    /// `iphoneos`, `appletvsimulator`, `appletvos`, `watchsimulator`,
    /// `watchos`, `xrsimulator`, `xros`.
    pub platform: String,
    /// OS version reported by the destination (e.g. `26.0.1`). Empty for
    /// macOS, where the captured oracles don't include a version segment.
    pub os_version: String,
    /// Human-readable device label drawn from the oracle filename
    /// (e.g. `iPad-A16`, `Apple-Vision-Pro`). Used for
    /// `RUN_DESTINATION_DEVICE_NAME`-like settings.
    pub device_name: String,
    /// Architecture the destination actually executes — `arm64` on every
    /// captured device, `x86_64` only on older Intel macs (we never see
    /// that in the corpus).
    pub arch: String,
}

impl RunDestination {
    /// True for any `*-simulator` platform.
    #[must_use]
    pub fn is_simulator(&self) -> bool {
        self.platform.ends_with("simulator")
    }

    /// True when the destination is macOS (host, not Catalyst).
    #[must_use]
    pub fn is_macos(&self) -> bool {
        self.platform == "macosx"
    }
}

/// Parse the destination suffix that appears in oracle filenames after
/// the `Config__` prefix. Accepts:
///
/// ```text
/// macOS                                               → macosx
/// iOS-Simulator_OS26.0.1_iPad-A16                     → iphonesimulator + OS + device
/// iOS-Simulator_OS18.1_iPad-10th-generation           → idem
/// tvOS-Simulator_OS26.0_Apple-TV                      → appletvsimulator
/// watchOS-Simulator_OS26.0_Apple-Watch-SE-3-40mm      → watchsimulator
/// visionOS-Simulator_OS26.0_Apple-Vision-Pro          → xrsimulator
/// ```
///
/// Returns `None` for shapes we don't recognise.
#[must_use]
pub fn parse_destination_suffix(s: &str) -> Option<RunDestination> {
    // The macOS case has no OS / device segments.
    if s == "macOS" || s == "macos" {
        return Some(RunDestination {
            platform: "macosx".into(),
            os_version: String::new(),
            device_name: String::new(),
            arch: "arm64".into(),
        });
    }
    // The remaining shapes are `<PlatformLabel>_OS<version>_<device>`, the
    // label hyphenated (`iOS-Simulator`).
    let mut parts = s.splitn(3, '_');
    let platform_label = parts.next()?.replace('-', " ");
    let platform = Platform::from_label(&platform_label)?.sdk.to_string();
    let os_version = parts.next().and_then(|p| p.strip_prefix("OS"))?;
    let device_name = parts.next()?;
    Some(RunDestination {
        platform,
        os_version: os_version.into(),
        device_name: device_name.into(),
        arch: "arm64".into(),
    })
}

/// Parse an `xcodebuild -destination` argument string into a [`RunDestination`].
///
/// Accepts the comma-separated `key=value` form xcodebuild takes on the command
/// line (note the *spaced* platform labels, unlike the hyphenated oracle-filename
/// form [`parse_destination_suffix`] handles):
///
/// ```text
/// platform=iOS Simulator,id=<udid>            // id-only — the common IDE case
/// platform=iOS Simulator,name=iPhone 16,OS=18.5
/// platform=macOS
/// platform=iOS Simulator,arch=x86_64
/// generic/platform=iOS                        // device-less platform build
/// ```
///
/// `platform=` is required and maps to a canonical SDK; every other field is
/// optional. `arch=` defaults to the host arch for macOS / simulator /
/// DriverKit destinations and `arm64` for device platforms. An `OS=` of
/// `latest`/`any` (or absent)
/// is treated as unset — settings resolution doesn't depend on the destination's
/// exact OS. Unknown keys (`id`, `variant`, …) are ignored. Returns `None` when
/// there's no recognized `platform=`.
#[must_use]
pub fn parse_destination_arg(s: &str) -> Option<RunDestination> {
    let spec = DestinationSpec::parse(s);
    let platform = spec.platform?;
    let os_version = spec
        .os
        .filter(|os| !os.eq_ignore_ascii_case("latest") && !os.eq_ignore_ascii_case("any"))
        .unwrap_or_default();
    let arch = spec.arch.filter(|a| !a.is_empty()).unwrap_or_else(|| {
        // macOS, the simulators, and DriverKit execute on the host, so the
        // default active arch is the host's (x86_64 on Intel Macs); device
        // platforms are always arm64.
        if platform.simulator || matches!(platform.sdk, "macosx" | "driverkit") {
            crate::project::host_arch()
        } else {
            "arm64".into()
        }
    });
    Some(RunDestination {
        platform: platform.sdk.to_string(),
        os_version,
        device_name: spec.name.unwrap_or_default(),
        arch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_macos() {
        let d = parse_destination_suffix("macOS").unwrap();
        assert_eq!(d.platform, "macosx");
        assert!(d.os_version.is_empty());
        assert!(d.is_macos());
        assert!(!d.is_simulator());
    }

    #[test]
    fn parses_ios_simulator() {
        let d = parse_destination_suffix("iOS-Simulator_OS26.0.1_iPad-A16").unwrap();
        assert_eq!(d.platform, "iphonesimulator");
        assert_eq!(d.os_version, "26.0.1");
        assert_eq!(d.device_name, "iPad-A16");
        assert_eq!(d.arch, "arm64");
        assert!(d.is_simulator());
    }

    #[test]
    fn parses_visionos_simulator() {
        let d = parse_destination_suffix("visionOS-Simulator_OS26.0_Apple-Vision-Pro").unwrap();
        assert_eq!(d.platform, "xrsimulator");
        assert_eq!(d.os_version, "26.0");
        assert_eq!(d.device_name, "Apple-Vision-Pro");
        assert!(d.is_simulator());
    }

    #[test]
    fn parses_watchos_simulator_with_device_hyphens() {
        let d = parse_destination_suffix("watchOS-Simulator_OS26.0_Apple-Watch-SE-3-40mm").unwrap();
        assert_eq!(d.platform, "watchsimulator");
        assert_eq!(d.device_name, "Apple-Watch-SE-3-40mm");
    }

    #[test]
    fn parses_tvos_simulator() {
        let d = parse_destination_suffix("tvOS-Simulator_OS26.0_Apple-TV").unwrap();
        assert_eq!(d.platform, "appletvsimulator");
    }

    #[test]
    fn rejects_unknown_shapes() {
        assert!(parse_destination_suffix("totally-bogus").is_none());
        assert!(parse_destination_suffix("iOS-Simulator").is_none()); // missing OS
        assert!(parse_destination_suffix("iOS-Simulator_OS26.0").is_none()); // missing device
    }

    #[test]
    fn arg_id_only_is_the_common_case() {
        // An `id=`-only simulator destination (what an IDE typically passes):
        // platform resolves, arch defaults to the host's, OS/name stay empty.
        let d =
            parse_destination_arg("platform=iOS Simulator,id=12345678-1234-1234-1234-123456789012")
                .unwrap();
        assert_eq!(d.platform, "iphonesimulator");
        assert_eq!(d.arch, crate::project::host_arch());
        assert!(d.os_version.is_empty());
        assert!(d.device_name.is_empty());
        assert!(d.is_simulator());
    }

    #[test]
    fn arg_full_simulator() {
        let d = parse_destination_arg("platform=iOS Simulator,name=iPhone 16,OS=18.5,arch=arm64")
            .unwrap();
        assert_eq!(d.platform, "iphonesimulator");
        assert_eq!(d.os_version, "18.5");
        assert_eq!(d.device_name, "iPhone 16");
        assert_eq!(d.arch, "arm64");
    }

    #[test]
    fn arg_macos_and_explicit_arch() {
        assert_eq!(
            parse_destination_arg("platform=macOS").unwrap().platform,
            "macosx"
        );
        let d = parse_destination_arg("platform=iOS Simulator,arch=x86_64").unwrap();
        assert_eq!(d.arch, "x86_64");
    }

    #[test]
    fn arg_os_latest_is_unset() {
        let d = parse_destination_arg("platform=iOS Simulator,OS=latest,id=abc").unwrap();
        assert!(d.os_version.is_empty());
    }

    #[test]
    fn arg_rejects_missing_or_unknown_platform() {
        assert!(parse_destination_arg("id=abc,arch=arm64").is_none());
        assert!(parse_destination_arg("platform=Android").is_none());
    }

    #[test]
    fn arg_default_arch_is_host_for_mac_and_simulators_arm64_for_devices() {
        // A macOS or simulator destination executes on the host, so the
        // default active arch is the host's (x86_64 on Intel Macs); device
        // platforms always run arm64. An explicit `arch=` still wins.
        let host = crate::project::host_arch();
        assert_eq!(parse_destination_arg("platform=macOS").unwrap().arch, host);
        assert_eq!(
            parse_destination_arg("platform=iOS Simulator,id=abc")
                .unwrap()
                .arch,
            host
        );
        assert_eq!(parse_destination_arg("platform=iOS").unwrap().arch, "arm64");
        assert_eq!(
            parse_destination_arg("platform=watchOS").unwrap().arch,
            "arm64"
        );
    }

    #[test]
    fn arg_generic_platform_destinations() {
        // xcodebuild's device-less form, `-destination 'generic/platform=iOS'`
        // — the standard way to build for a platform without picking a device.
        let d = parse_destination_arg("generic/platform=iOS").unwrap();
        assert_eq!(d.platform, "iphoneos");
        assert!(d.device_name.is_empty());
        let d = parse_destination_arg("generic/platform=iOS Simulator").unwrap();
        assert_eq!(d.platform, "iphonesimulator");
    }

    #[test]
    fn arg_and_suffix_agree_on_platform_and_os() {
        // The CLI-arg form and the oracle-filename form should yield the same
        // platform / OS for an equivalent destination (device labels differ
        // in punctuation; the arg form's default arch is host-derived while
        // the suffix form pins the capture machine's arm64, so neither is
        // compared here).
        let arg = parse_destination_arg("platform=iOS Simulator,name=iPad A16,OS=26.5").unwrap();
        let suffix = parse_destination_suffix("iOS-Simulator_OS26.5_iPad-A16").unwrap();
        assert_eq!(arg.platform, suffix.platform);
        assert_eq!(arg.os_version, suffix.os_version);
        assert_eq!(suffix.arch, "arm64");
    }

    #[test]
    fn platform_labels_read_without_regard_to_case() {
        // xcodebuild takes `platform=ios simulator` as readily as the
        // canonical spelling, plus the legacy `OS X` and the `xrOS` names.
        for (label, sdk) in [
            ("iOS Simulator", "iphonesimulator"),
            ("ios simulator", "iphonesimulator"),
            ("macOS", "macosx"),
            ("macos", "macosx"),
            ("OS X", "macosx"),
            ("visionOS Simulator", "xrsimulator"),
            ("xrOS Simulator", "xrsimulator"),
            ("xrOS", "xros"),
            ("DriverKit", "driverkit"),
        ] {
            assert_eq!(
                Platform::from_label(label).map(|p| p.sdk),
                Some(sdk),
                "{label}"
            );
        }
        assert!(Platform::from_label("Android").is_none());
        assert!(Platform::from_label("My Mac").is_none());
    }

    #[test]
    fn simulator_os_names_map_to_one_platform_each() {
        // simctl's runtime spells visionOS `xrOS`; devicectl spells it
        // `visionOS`. Both are the one family.
        for (os, sdk, label) in [
            ("iOS", "iphonesimulator", "iOS Simulator"),
            ("watchOS", "watchsimulator", "watchOS Simulator"),
            ("tvOS", "appletvsimulator", "tvOS Simulator"),
            ("xrOS", "xrsimulator", "visionOS Simulator"),
            ("visionOS", "xrsimulator", "visionOS Simulator"),
        ] {
            let p = Platform::simulator_for_os(os).unwrap();
            assert_eq!((p.sdk, p.label), (sdk, label), "{os}");
        }
        assert!(Platform::simulator_for_os("futureOS").is_none());
        assert!(Platform::simulator_for_os("macOS").is_none());
        assert_eq!(
            Platform::device_for_os("visionOS").unwrap().label,
            "visionOS"
        );
        assert_eq!(Platform::device_for_os("xrOS").unwrap().sdk, "xros");
        assert_eq!(Platform::device_for_os("iOS").unwrap().label, "iOS");
    }

    #[test]
    fn every_platform_round_trips_through_its_sdk_and_label() {
        for p in &PLATFORMS {
            assert_eq!(Platform::from_sdk(p.sdk), Some(p));
            assert_eq!(Platform::from_label(p.label), Some(p));
            assert_eq!(p.simulator, p.sdk.ends_with("simulator"), "{}", p.sdk);
        }
    }

    #[test]
    fn an_sdk_brings_its_simulator_sibling() {
        assert_eq!(Platform::sdk_family_tokens("macosx"), ["macosx"]);
        assert_eq!(
            Platform::sdk_family_tokens("iphoneos17.5"),
            ["iphoneos", "iphonesimulator"]
        );
        assert_eq!(
            Platform::sdk_family_tokens("/SDKs/XROS2.0.sdk"),
            ["xros", "xrsimulator"]
        );
        assert_eq!(
            Platform::sdk_family_tokens("watchsimulator"),
            ["watchsimulator"]
        );
        assert!(Platform::sdk_family_tokens("auto").is_empty());
    }

    #[test]
    fn a_spec_keeps_every_field_it_names() {
        // The shape the extension writes when Rosetta destinations are on:
        // the id stops at its comma.
        let spec = DestinationSpec::parse("platform=iOS Simulator,id=ABCD-1234,arch=x86_64");
        assert_eq!(spec.id.as_deref(), Some("ABCD-1234"));
        assert_eq!(spec.arch.as_deref(), Some("x86_64"));
        assert_eq!(spec.sdk(), Some("iphonesimulator"));
        assert!(spec.is_simulator() && !spec.is_device() && !spec.is_macos());

        let spec = DestinationSpec::parse("platform=macOS,variant=Mac Catalyst");
        assert!(spec.is_macos());
        assert_eq!(spec.variant.as_deref(), Some("Mac Catalyst"));

        let spec = DestinationSpec::parse("platform=iOS Simulator, name=iPhone 17 ,OS=latest");
        assert_eq!(spec.name.as_deref(), Some("iPhone 17"));
        assert_eq!(spec.os.as_deref(), Some("latest"));
        assert!(spec.id.is_none());
    }

    #[test]
    fn devices_are_concrete_non_simulator_non_mac_destinations() {
        assert!(DestinationSpec::parse("platform=iOS,id=00008110-000559182E90401E").is_device());
        assert!(DestinationSpec::parse("platform=visionOS,id=X").is_device());
        // An unknown label keeps the benefit of the doubt.
        assert!(DestinationSpec::parse("platform=futureOS,id=X").is_device());
        assert!(!DestinationSpec::parse("platform=iOS Simulator,id=X").is_device());
        assert!(!DestinationSpec::parse("platform=ios simulator,id=X").is_device());
        assert!(!DestinationSpec::parse("platform=macOS,arch=arm64").is_device());
        assert!(!DestinationSpec::parse("generic/platform=iOS").is_device());
        assert!(!DestinationSpec::parse("id=X").is_device());
    }

    #[test]
    fn a_generic_spec_names_its_platform() {
        let spec = DestinationSpec::parse("generic/platform=visionOS Simulator");
        assert!(spec.generic);
        assert_eq!(spec.sdk(), Some("xrsimulator"));
        assert_eq!(spec.platform_label.as_deref(), Some("visionOS Simulator"));
    }
}
