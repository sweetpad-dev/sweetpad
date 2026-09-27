//! Reading the arguments an `xcodebuild` command line adds to a build the
//! way `xcodebuild` reads them. The CLI reads them from `sweetpad.toml` and
//! the `--` tail, and the BSP server from the `buildArgs` the extension writes
//! into `bsp.json`.

/// The `KEY=VALUE` build settings in `args`, in order: what
/// `xcodebuild` applies above every project layer. The value after a flag
/// that takes one is skipped, since `-destination platform=macOS` is a
/// specifier, not a setting named `platform`. A flag missing from
/// [`VALUE_FLAGS`] reads as a switch, which costs at most one setting read
/// from its value.
#[must_use]
pub fn settings(args: &[String]) -> Vec<(String, String)> {
    let mut settings = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if VALUE_FLAGS.contains(&arg.as_str()) {
            iter.next();
        } else if let Some((key, value)) = arg.split_once('=')
            && key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            settings.push((key.to_string(), value.to_string()));
        }
    }
    settings
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

/// The value after the last `flag` in `args`, read as `xcodebuild` reads
/// it: a flag that takes a value takes the next argument, dashes and all, so
/// a value spelled like `flag` is not a copy of it. `xcodebuild` takes one
/// `-xcconfig` and refuses a second, so the last one given is the one that
/// counts. A `flag` that ends `args` without a value is skipped: `xcodebuild`
/// refuses that command line, and [`dangling_flag`] names the flag for the
/// caller to report.
#[must_use]
pub fn last_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let mut found = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == flag || VALUE_FLAGS.contains(&arg.as_str()) {
            let value = iter.next();
            if arg == flag
                && let Some(value) = value
            {
                found = Some(value.as_str());
            }
        }
    }
    found
}

/// The flag that ends `args` still waiting for its value, which `xcodebuild`
/// refuses ("option '-xcconfig' requires an argument").
#[must_use]
pub fn dangling_flag(args: &[String]) -> Option<&str> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if VALUE_FLAGS.contains(&arg.as_str()) && iter.next().is_none() {
            return Some(arg);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{dangling_flag, last_value, settings};

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_string()).collect()
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
