//! Reading the arguments an `xcodebuild` command line adds to a build the
//! way `xcodebuild` reads them. The CLI reads them from `sweetpad.toml` and
//! the `--` tail, and the BSP server from the `buildArgs` the extension writes
//! into `bsp.json`. Every reader walks them with [`read`], which pairs each
//! flag in [`VALUE_FLAGS`] with its value, so they agree on which argument is
//! a flag and which is a value.

/// One argument of an `xcodebuild` command line as `xcodebuild` reads it: a
/// flag in [`VALUE_FLAGS`] with the argument after it, or any other argument
/// alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arg<'a> {
    /// A flag, a `KEY=VALUE` setting, an action, or anything else.
    pub word: &'a str,
    /// The value of a flag that takes one. `None` for any other argument,
    /// and for a flag that ends the list still waiting for its value.
    pub value: Option<&'a str>,
}

impl<'a> Arg<'a> {
    /// Whether [`Self::word`] is a flag that takes a value.
    #[must_use]
    pub fn takes_value(&self) -> bool {
        takes_value(self.word)
    }

    /// The word and its value, as the command line spells them.
    pub fn words(self) -> impl Iterator<Item = &'a str> {
        std::iter::once(self.word).chain(self.value)
    }
}

/// `args` as `xcodebuild` reads them, one [`Arg`] per flag or other
/// argument. A flag that takes a value takes the next argument, dashes and
/// all: `-xcconfig -quiet` names a file called '-quiet'. A flag missing from
/// [`VALUE_FLAGS`] reads as a switch.
pub fn read(args: &[String]) -> impl Iterator<Item = Arg<'_>> {
    let mut iter = args.iter().map(String::as_str);
    std::iter::from_fn(move || {
        let word = iter.next()?;
        let value = if takes_value(word) { iter.next() } else { None };
        Some(Arg { word, value })
    })
}

/// Whether `flag` takes the next argument as its value.
#[must_use]
pub fn takes_value(flag: &str) -> bool {
    VALUE_FLAGS.contains(&flag)
}

/// Whether `word` is a flag `xcodebuild` takes on a build, test or archive
/// command line: one of the [`VALUE_FLAGS`] or [`SWITCHES`]. A test
/// identifier rides on `-only-testing:` and `-skip-testing:` and doesn't
/// count.
#[must_use]
pub fn is_flag(word: &str) -> bool {
    let name = word.split_once(':').map_or(word, |(name, _)| name);
    takes_value(name) || SWITCHES.contains(&name)
}

/// The `KEY=VALUE` build settings in `args`, in order: what
/// `xcodebuild` applies above every project layer. The value after a flag
/// that takes one is skipped, since `-destination platform=macOS` is a
/// specifier, not a setting named `platform`. A flag missing from
/// [`VALUE_FLAGS`] reads as a switch, which costs at most one setting read
/// from its value.
#[must_use]
pub fn settings(args: &[String]) -> Vec<(String, String)> {
    read(args)
        .filter(|arg| !arg.takes_value())
        .filter_map(|arg| {
            let (key, value) = arg.word.split_once('=')?;
            (key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

/// The `xcodebuild` flags that take the next argument as their value, of the
/// ones that shape a build, test or archive, as Xcode 27 reads them. Each
/// fails at the end of a command line ("option '-jobs' requires an
/// argument"), and takes the next argument whatever it looks like:
/// `-testLanguage -quiet` names a language '-quiet', and a flag that checks
/// its value refuses one spelled like a flag (`-enableCodeCoverage -quiet`).
/// The space form of `-only-testing` and `-skip-testing` is listed;
/// `-only-testing:X` is one argument. The flags of `xcodebuild`'s other modes
/// (`-exportLocalizations`' `-exportLanguage`, …) are left out.
pub const VALUE_FLAGS: [&str; 57] = [
    "-project",
    "-workspace",
    "-target",
    "-scheme",
    "-configuration",
    "-sdk",
    "-arch",
    "-destination",
    "-destination-timeout",
    "-xcconfig",
    "-xctestrun",
    "-testPlan",
    "-toolchain",
    "-jobs",
    "-derivedDataPath",
    "-resultBundlePath",
    "-resultStreamPath",
    "-resultBundleVersion",
    "-archivePath",
    "-exportPath",
    "-exportOptionsPlist",
    "-clonedSourcePackagesDirPath",
    "-packageCachePath",
    "-enableCodeCoverage",
    "-testLanguage",
    "-testRegion",
    "-testProductsPath",
    "-enablePerformanceTestsDiagnostics",
    "-only-testing",
    "-skip-testing",
    "-only-test-configuration",
    "-skip-test-configuration",
    "-collect-test-diagnostics",
    "-test-iterations",
    "-test-timeouts-enabled",
    "-default-test-execution-time-allowance",
    "-maximum-test-execution-time-allowance",
    "-test-repetition-relaunch-enabled",
    "-parallel-testing-enabled",
    "-parallel-testing-worker-count",
    "-maximum-parallel-testing-workers",
    "-maximum-concurrent-test-device-destinations",
    "-maximum-concurrent-test-simulator-destinations",
    "-enableAddressSanitizer",
    "-enableThreadSanitizer",
    "-enableUndefinedBehaviorSanitizer",
    "-enableCodesizeProfile",
    "-codesizeProfileOutputDir",
    "-packageAuthorizationProvider",
    "-defaultPackageRegistryURL",
    "-packageDependencySCMToRegistryTransformation",
    "-packageFingerprintPolicy",
    "-packageSigningEntityPolicy",
    "-scmProvider",
    "-authenticationKeyPath",
    "-authenticationKeyID",
    "-authenticationKeyIssuerID",
];

/// The flags `xcodebuild -help` lists for Xcode 27 that shape a build, test
/// or archive and take no value, beside the [`VALUE_FLAGS`]. The ones that
/// make `xcodebuild` do something else (`-showBuildSettings`, `-list`,
/// `-exportArchive`, `-version`, …) are left out.
pub const SWITCHES: [&str; 18] = [
    "-alltargets",
    "-parallelizeTargets",
    "-quiet",
    "-verbose",
    "-hideShellScriptEnvironment",
    "-showBuildTimingSummary",
    "-skipUnavailableActions",
    "-allowProvisioningUpdates",
    "-allowProvisioningDeviceRegistration",
    "-retry-tests-on-failure",
    "-run-tests-until-failure",
    "-disableAutomaticPackageResolution",
    "-onlyUsePackageVersionsFromResolvedFile",
    "-skipPackageUpdates",
    "-disablePackageRepositoryCache",
    "-skipPackagePluginValidation",
    "-skipMacroValidation",
    "-skipPackageSignatureValidation",
];

/// The value after each `flag` in `args`, in order, read by [`read`]: a
/// value spelled like `flag` is not a copy of it. A build takes every
/// `-destination` it is given. A `flag` that ends `args` without a value gives
/// none.
pub fn values<'a>(args: &'a [String], flag: &str) -> impl Iterator<Item = &'a str> {
    read(args)
        .filter(move |arg| arg.word == flag)
        .filter_map(|arg| arg.value)
}

/// The value after the last `flag` in `args` ([`values`]). `xcodebuild`
/// takes one `-xcconfig` and refuses a second, so the last one given is the
/// one that counts. A `flag` that ends `args` without a value is skipped:
/// `xcodebuild` refuses that command line, and [`dangling_flag`] names the
/// flag for the caller to report.
#[must_use]
pub fn last_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    values(args, flag).last()
}

/// Whether `args` gives `flag` itself, not as the value of a flag that takes
/// one: `-xcconfig -enableCodeCoverage` gives '-xcconfig' and no
/// '-enableCodeCoverage'. A `flag` that ends `args` without its value still
/// counts.
#[must_use]
pub fn has_flag(args: &[String], flag: &str) -> bool {
    read(args).any(|arg| arg.word == flag)
}

/// The flag that ends `args` still waiting for its value, which `xcodebuild`
/// refuses ("option '-xcconfig' requires an argument").
#[must_use]
pub fn dangling_flag(args: &[String]) -> Option<&str> {
    read(args)
        .find(|arg| arg.takes_value() && arg.value.is_none())
        .map(|arg| arg.word)
}

#[cfg(test)]
mod tests {
    use super::{
        Arg, SWITCHES, VALUE_FLAGS, dangling_flag, has_flag, is_flag, last_value, read, settings,
        values,
    };

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_string()).collect()
    }

    #[test]
    fn each_flag_that_takes_a_value_is_read_with_it() {
        let args = s(&[
            "build",
            "-xcconfig",
            "-quiet",
            "-quiet",
            "FOO=1",
            "-destination",
            "platform=macOS",
            "-derivedDataPath",
        ]);
        let arg = |word, value| Arg { word, value };
        assert_eq!(
            read(&args).collect::<Vec<_>>(),
            [
                arg("build", None),
                arg("-xcconfig", Some("-quiet")),
                arg("-quiet", None),
                arg("FOO=1", None),
                arg("-destination", Some("platform=macOS")),
                arg("-derivedDataPath", None),
            ]
        );
        // Written back, the words are the command line as given.
        assert_eq!(read(&args).flat_map(Arg::words).collect::<Vec<_>>(), args);
    }

    #[test]
    fn xcodebuild_takes_its_listed_flags_with_one_dash() {
        let mut seen = std::collections::BTreeSet::new();
        for flag in VALUE_FLAGS.iter().chain(&SWITCHES) {
            assert!(flag.starts_with('-') && !flag.starts_with("--"), "{flag}");
            assert!(seen.insert(flag), "listed twice: {flag}");
            assert!(is_flag(flag), "{flag}");
        }
        assert!(is_flag("-only-testing:AppTests/Slow"));
        for word in [
            "--allowProvisioningUpdates",
            "-bogus",
            "-showBuildSettings",
            "-exportLanguage",
            "-",
            "build",
        ] {
            assert!(!is_flag(word), "{word}");
        }
    }

    #[test]
    fn a_repeated_flag_gives_each_of_its_values() {
        let args = s(&[
            "-destination",
            "id=A",
            "-xcconfig",
            "-destination",
            "-destination",
            "id=B",
        ]);
        assert_eq!(
            values(&args, "-destination").collect::<Vec<_>>(),
            ["id=A", "id=B"]
        );
    }

    #[test]
    fn the_last_value_of_a_flag_counts() {
        let args = s(&[
            "-xcconfig",
            "a.xcconfig",
            "-quiet",
            "-xcconfig",
            "b.xcconfig",
        ]);
        assert_eq!(last_value(&args, "-xcconfig"), Some("b.xcconfig"));
        assert_eq!(last_value(&args, "-derivedDataPath"), None);
        assert_eq!(dangling_flag(&args), None);
        assert!(settings(&args).is_empty());
    }

    #[test]
    fn a_flag_spelled_as_another_flags_value_is_not_a_copy() {
        // xcodebuild reads the argument after a value flag as its value,
        // dashes and all: `-xcconfig -quiet` reads a file named '-quiet'.
        let args = s(&["-xcconfig", "a.xcconfig", "-derivedDataPath", "-xcconfig"]);
        assert_eq!(last_value(&args, "-xcconfig"), Some("a.xcconfig"));
        assert_eq!(last_value(&args, "-derivedDataPath"), Some("-xcconfig"));
        assert_eq!(dangling_flag(&args), None);
        assert_eq!(
            last_value(&s(&["-xcconfig", "-quiet"]), "-xcconfig"),
            Some("-quiet")
        );
    }

    #[test]
    fn a_trailing_flag_without_its_value_is_named_and_not_read() {
        let args = s(&["-xcconfig", "a.xcconfig", "FOO=1", "-xcconfig"]);
        assert_eq!(dangling_flag(&args), Some("-xcconfig"));
        // The copy before it is the last one with a value.
        assert_eq!(last_value(&args, "-xcconfig"), Some("a.xcconfig"));
        assert_eq!(settings(&args), [("FOO".to_string(), "1".to_string())]);

        assert_eq!(
            dangling_flag(&s(&["-derivedDataPath"])),
            Some("-derivedDataPath")
        );
        assert_eq!(
            last_value(&s(&["-derivedDataPath"]), "-derivedDataPath"),
            None
        );
        // A switch ends a command line fine.
        assert_eq!(dangling_flag(&s(&["-xcconfig", "a", "-quiet"])), None);
        assert_eq!(dangling_flag(&[]), None);
    }

    /// The testing flags read their values as the others do, so a
    /// '-enableCodeCoverage YES' is one flag and its value, and one that ends
    /// the list is named.
    #[test]
    fn a_testing_flag_takes_its_value() {
        let args = s(&[
            "-enableCodeCoverage",
            "YES",
            "-only-testing",
            "AppTests/Slow",
            "-test-iterations",
            "3",
            "-xcconfig",
            "-enableCodeCoverage",
            "-only-testing:AppTests/Fast",
            "FOO=1",
        ]);
        assert_eq!(last_value(&args, "-enableCodeCoverage"), Some("YES"));
        assert_eq!(last_value(&args, "-only-testing"), Some("AppTests/Slow"));
        assert_eq!(last_value(&args, "-test-iterations"), Some("3"));
        assert_eq!(last_value(&args, "-xcconfig"), Some("-enableCodeCoverage"));
        assert_eq!(dangling_flag(&args), None);
        assert_eq!(settings(&args), [("FOO".to_string(), "1".to_string())]);

        for flag in ["-enableCodeCoverage", "-only-testing", "-test-iterations"] {
            assert_eq!(dangling_flag(&s(&["-quiet", flag])), Some(flag));
        }
        // The one-word form carries its identifier.
        assert_eq!(dangling_flag(&s(&["-only-testing:AppTests"])), None);
    }

    #[test]
    fn a_flag_is_given_only_where_xcodebuild_reads_a_flag() {
        let args = s(&[
            "-xcconfig",
            "-enableCodeCoverage",
            "-quiet",
            "-test-iterations",
        ]);
        assert!(has_flag(&args, "-xcconfig"));
        assert!(!has_flag(&args, "-enableCodeCoverage"));
        assert!(has_flag(&args, "-quiet"));
        // One still waiting for its value is given.
        assert!(has_flag(&args, "-test-iterations"));
        assert!(!has_flag(&args, "-jobs"));
        assert!(!has_flag(&[], "-quiet"));
    }

    #[test]
    fn a_passthroughs_settings_are_its_assignments_not_its_flag_values() {
        let args: Vec<String> = [
            "-quiet",
            "PRODUCT_BUNDLE_IDENTIFIER=com.x.y",
            "-destination",
            "OS=17.0,platform=iOS Simulator",
            "-derivedDataPath",
            "A=b",
            "SWIFT_ACTIVE_COMPILATION_CONDITIONS=DEBUG STAGING",
            "-only-testing:App/T=1",
            "PRODUCT_NAME=",
        ]
        .iter()
        .map(|a| (*a).to_string())
        .collect();
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert_eq!(
            settings(&args),
            [
                pair("PRODUCT_BUNDLE_IDENTIFIER", "com.x.y"),
                pair("SWIFT_ACTIVE_COMPILATION_CONDITIONS", "DEBUG STAGING"),
                pair("PRODUCT_NAME", ""),
            ]
        );
    }
}
