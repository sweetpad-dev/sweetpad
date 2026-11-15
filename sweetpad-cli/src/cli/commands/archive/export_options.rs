//! The part of ExportOptions.plist sweetpad's own export follows. A key it
//! would ignore while it changes what Xcode writes is refused by name, so an
//! export never differs from Xcode's without saying so.

use std::path::Path;

use super::ExportMethod;
use crate::cli::property_list::{self as plist, Plist};
use crate::cli::{CliError, ErrorKind};

/// What an ExportOptions.plist asks for.
#[derive(Debug, Default)]
pub(crate) struct ExportOptions {
    pub method: Option<ExportMethod>,
    pub team: Option<String>,
    pub manual: bool,
    pub signing_certificate: Option<String>,
    pub installer_certificate: Option<String>,
    /// A profile name or UUID for each bundle identifier.
    pub profiles: Vec<(String, String)>,
    pub icloud_environment: Option<String>,
    /// `destination` is `upload`.
    pub upload: bool,
    /// Keys taken but not acted on, to say so.
    pub notes: Vec<String>,
}

/// The method a `method` value names, with the names before Xcode 15.
fn method(value: &str) -> Option<ExportMethod> {
    Some(match value {
        "app-store-connect" | "app-store" => ExportMethod::AppStoreConnect,
        "release-testing" | "ad-hoc" => ExportMethod::ReleaseTesting,
        "debugging" | "development" => ExportMethod::Debugging,
        "enterprise" => ExportMethod::Enterprise,
        "developer-id" => ExportMethod::DeveloperId,
        "mac-application" => ExportMethod::MacApplication,
        _ => return None,
    })
}

pub(crate) fn read(path: &Path) -> Result<ExportOptions, CliError> {
    let data =
        std::fs::read(path).map_err(|e| usage(format!("can't read {}: {e}", path.display())))?;
    let value = plist::parse(&data).map_err(|e| usage(format!("{}: {e}", path.display())))?;
    parse(&value).map_err(|e| usage(format!("{}: {e}", path.display())))
}

pub(crate) fn parse(value: &Plist) -> Result<ExportOptions, String> {
    let Plist::Dict(entries) = value else {
        return Err("ExportOptions must be a dictionary".to_string());
    };
    let mut options = ExportOptions::default();
    for (key, value) in entries {
        take(&mut options, key, value)?;
    }
    if options.upload && !matches!(options.method, Some(ExportMethod::AppStoreConnect)) {
        return Err(
            "destination 'upload' goes to App Store Connect, which needs method \
                    'app-store-connect'"
                .to_string(),
        );
    }
    Ok(options)
}

/// Follow one key, or refuse it.
#[allow(clippy::too_many_lines)] // one arm per ExportOptions key
fn take(options: &mut ExportOptions, key: &str, value: &Plist) -> Result<(), String> {
    let text = || match value {
        Plist::String(s) => Ok(s.clone()),
        _ => Err(format!("{key} must be a string")),
    };
    let flag = || match value {
        Plist::Bool(b) => Ok(*b),
        _ => Err(format!("{key} must be a boolean")),
    };
    match key {
        "method" => {
            let name = text()?;
            options.method = Some(
                method(&name)
                    .ok_or_else(|| format!("method '{name}' isn't one sweetpad exports"))?,
            );
        }
        "teamID" => options.team = Some(text()?).filter(|t| !t.is_empty()),
        "signingStyle" => {
            options.manual = match text()?.as_str() {
                "manual" => true,
                "automatic" => false,
                other => {
                    return Err(format!(
                        "signingStyle '{other}' is neither manual nor automatic"
                    ));
                }
            };
        }
        "signingCertificate" => {
            options.signing_certificate = Some(text()?).filter(|c| !c.is_empty());
        }
        "installerSigningCertificate" => {
            options.installer_certificate = Some(text()?).filter(|c| !c.is_empty());
        }
        "provisioningProfiles" => {
            let Plist::Dict(map) = value else {
                return Err("provisioningProfiles must map bundle identifiers to profiles".into());
            };
            for (bundle, profile) in map {
                let Plist::String(profile) = profile else {
                    return Err(format!(
                        "provisioningProfiles names no profile for {bundle}"
                    ));
                };
                options.profiles.push((bundle.clone(), profile.clone()));
            }
        }
        "destination" => {
            options.upload = match text()?.as_str() {
                "upload" => true,
                "export" => false,
                other => {
                    return Err(format!(
                        "destination '{other}' is neither export nor upload"
                    ));
                }
            };
        }
        "iCloudContainerEnvironment" => {
            let environment = text()?;
            if !["Development", "Production"].contains(&environment.as_str()) {
                return Err(format!(
                    "iCloudContainerEnvironment '{environment}' is neither Development nor Production"
                ));
            }
            options.icloud_environment = Some(environment);
        }
        "thinning" if text()? != "<none>" => {
            return Err(
                "thinning makes app variants, which this export doesn't; leave it at '<none>'"
                    .to_string(),
            );
        }
        "uploadSymbols" if flag()? => options.notes.push(
            "uploadSymbols: the export holds no symbols; the dSYMs stay in the archive".to_string(),
        ),
        "manageAppVersionAndBuildNumber" if flag()? => options.notes.push(
            "manageAppVersionAndBuildNumber: sweetpad keeps the archived version and build numbers"
                .to_string(),
        ),
        // Bitcode is gone from Xcode, and Apple's toolchain strips Swift
        // symbols from release builds already.
        "uploadSymbols"
        | "manageAppVersionAndBuildNumber"
        | "compileBitcode"
        | "uploadBitcode"
        | "stripSwiftSymbols" => {
            flag()?;
        }
        "testFlightInternalTestingOnly" if flag()? => {
            return Err(
                "testFlightInternalTestingOnly isn't supported: App Store Connect's API \
                        upload has no such option"
                    .to_string(),
            );
        }
        "thinning" | "testFlightInternalTestingOnly" => {}
        other => {
            return Err(format!(
                "{other} isn't supported by sweetpad's export, and ignoring it would change what \
                 you get; remove it, or export with '--engine xcodebuild' on a Mac"
            ));
        }
    }
    Ok(())
}

fn usage(message: String) -> CliError {
    CliError::new(message).kind(ErrorKind::Usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(xml: &str) -> Result<ExportOptions, String> {
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>{xml}</dict></plist>"
        );
        parse(&plist::parse(document.as_bytes()).unwrap())
    }

    #[test]
    fn the_supported_keys_are_followed() {
        let read = options(
            "<key>method</key><string>app-store</string>\
             <key>teamID</key><string>TESTTEAM01</string>\
             <key>signingStyle</key><string>manual</string>\
             <key>signingCertificate</key><string>Apple Distribution</string>\
             <key>provisioningProfiles</key><dict><key>dev.app</key><string>App Store</string></dict>\
             <key>destination</key><string>upload</string>\
             <key>uploadSymbols</key><true/>\
             <key>compileBitcode</key><false/>\
             <key>thinning</key><string>&lt;none&gt;</string>",
        )
        .unwrap();
        assert!(matches!(read.method, Some(ExportMethod::AppStoreConnect)));
        assert_eq!(read.team.as_deref(), Some("TESTTEAM01"));
        assert!(read.manual && read.upload);
        assert_eq!(
            read.signing_certificate.as_deref(),
            Some("Apple Distribution")
        );
        assert_eq!(
            read.profiles,
            [("dev.app".to_string(), "App Store".to_string())]
        );
        assert_eq!(read.notes.len(), 1);
        for (alias, expected) in [("ad-hoc", "release-testing"), ("development", "debugging")] {
            let read = options(&format!("<key>method</key><string>{alias}</string>")).unwrap();
            assert_eq!(read.method.unwrap().plist_value(), expected);
        }
    }

    #[test]
    fn keys_that_would_change_the_output_are_refused_by_name() {
        for (xml, says) in [
            ("<key>manifest</key><dict/>", "manifest isn't supported"),
            (
                "<key>generateAppStoreInformation</key><true/>",
                "generateAppStoreInformation isn't",
            ),
            (
                "<key>thinning</key><string>iPhone15,2</string>",
                "thinning makes app variants",
            ),
            (
                "<key>method</key><string>validation</string>",
                "method 'validation'",
            ),
            (
                "<key>method</key><string>ad-hoc</string><key>destination</key><string>upload</string>",
                "needs method 'app-store-connect'",
            ),
            ("<key>teamID</key><true/>", "teamID must be a string"),
        ] {
            let error = options(xml).err().unwrap();
            assert!(error.contains(says), "{xml}: {error}");
        }
    }
}
