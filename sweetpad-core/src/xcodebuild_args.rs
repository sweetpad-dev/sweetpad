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

/// The `xcodebuild` flags that take the next argument as their value.
pub const VALUE_FLAGS: [&str; 22] = [
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
    "-archivePath",
    "-exportPath",
    "-exportOptionsPlist",
    "-clonedSourcePackagesDirPath",
    "-packageCachePath",
];

/// The argument after the last `flag` in `args`: `xcodebuild` takes one
/// `-xcconfig` and refuses a second, so the last one given is the one that
/// counts.
#[must_use]
pub fn last_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .rev()
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}

#[cfg(test)]
mod tests {
    use super::{last_value, settings};

    #[test]
    fn the_last_value_of_a_flag_counts() {
        let args: Vec<String> = [
            "-xcconfig",
            "a.xcconfig",
            "-quiet",
            "-xcconfig",
            "b.xcconfig",
            "-xcconfig",
        ]
        .iter()
        .map(|a| (*a).to_string())
        .collect();
        assert_eq!(last_value(&args, "-xcconfig"), Some("b.xcconfig"));
        assert_eq!(last_value(&args, "-derivedDataPath"), None);
        assert!(settings(&args).is_empty());
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
