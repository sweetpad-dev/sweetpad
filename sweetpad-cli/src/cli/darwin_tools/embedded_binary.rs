//! `embeddedBinaryValidationUtility <bundle> -info-plist-path <app's
//! Info.plist> [-signing-cert <name>]`, which Swift Build runs on each app
//! extension, App Clip or watch app it embeds in an app.
//!
//! It makes the checks of Xcode's that hold without signing: an error when
//! the embedded bundle's identifier doesn't start with the app's, and a
//! warning when its `CFBundleVersion`, or else its
//! `CFBundleShortVersionString`, differs from the app's or is missing. The messages are
//! Xcode's, on stderr, where Swift Build reads them.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sweetpad_lib::derived_data::plist_strings;

pub(super) fn main(args: &[String]) -> ExitCode {
    match validate(args) {
        Ok(warnings) => {
            for warning in warnings {
                eprintln!("warning: {warning}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The warnings about the embedded bundle, or the error that fails it.
fn validate(args: &[String]) -> Result<Vec<String>, String> {
    let mut embedded = None;
    let mut host_plist = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-info-plist-path" => host_plist = Some(PathBuf::from(super::value(arg, &mut it)?)),
            // Nothing is signed during the build here.
            "-signing-cert" => {
                super::value(arg, &mut it)?;
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option: {flag}")),
            path => embedded = Some(PathBuf::from(path)),
        }
    }
    let embedded = embedded.ok_or("must provide a path to an embedded binary")?;
    let host_plist = host_plist.ok_or("must provide the containing app's -info-plist-path")?;
    let host = plist_strings(&host_plist);
    if host.is_empty() {
        return Err(format!(
            "Couldn't load Info dictionary for containing app ({})",
            host_plist.display()
        ));
    }
    let own = plist_strings(&info_plist(&embedded));
    Ok(check(&own, &host)?.into_iter().collect())
}

/// A bundle's Info.plist: at its root on iOS, in `Contents` on macOS.
fn info_plist(bundle: &Path) -> PathBuf {
    let mac = bundle.join("Contents/Info.plist");
    if mac.is_file() {
        mac
    } else {
        bundle.join("Info.plist")
    }
}

/// Xcode's checks of an embedded bundle's Info.plist keys against its app's.
fn check(own: &[(String, String)], host: &[(String, String)]) -> Result<Option<String>, String> {
    let key = |plist: &[(String, String)], name: &str| {
        plist
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    let own_id = key(own, "CFBundleIdentifier").unwrap_or_default();
    let host_id = key(host, "CFBundleIdentifier").unwrap_or_default();
    if !own_id.starts_with(&host_id) {
        return Err(format!(
            "Embedded binary's bundle identifier is not prefixed with the parent app's bundle \
             identifier.\n\n\t\tEmbedded Binary Bundle Identifier:\t{own_id}\n\t\tParent App \
             Bundle Identifier:\t\t{host_id}\n"
        ));
    }
    let quoted =
        |value: Option<String>| value.map_or_else(|| "(null)".to_string(), |v| format!("'{v}'"));
    for name in ["CFBundleVersion", "CFBundleShortVersionString"] {
        let (mine, theirs) = (key(own, name), key(host, name));
        // Xcode's compares with `isEqual:`, so two missing versions differ.
        if mine.is_none() || mine != theirs {
            return Ok(Some(format!(
                "The {name} of an app extension ({}) must match that of its containing parent app ({}).",
                quoted(mine),
                quoted(theirs)
            )));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plist(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    const APP: &[(&str, &str)] = &[
        ("CFBundleIdentifier", "dev.x.App"),
        ("CFBundleVersion", "1"),
        ("CFBundleShortVersionString", "1.0"),
    ];

    #[test]
    fn a_bundle_id_outside_the_apps_is_an_error() {
        let widget = plist(&[("CFBundleIdentifier", "dev.y.Widget")]);
        let err = check(&widget, &plist(APP)).unwrap_err();
        assert!(err.starts_with("Embedded binary's bundle identifier is not prefixed"));
        assert!(err.contains("Embedded Binary Bundle Identifier:\tdev.y.Widget"));
    }

    #[test]
    fn the_first_version_that_differs_is_a_warning() {
        let widget = plist(&[
            ("CFBundleIdentifier", "dev.x.App.Widget"),
            ("CFBundleShortVersionString", "2.0"),
        ]);
        assert_eq!(
            check(&widget, &plist(APP)).unwrap().as_deref(),
            Some(
                "The CFBundleVersion of an app extension ((null)) must match that of its \
                 containing parent app ('1')."
            )
        );
    }

    #[test]
    fn versions_missing_from_both_still_differ() {
        let app = plist(&[("CFBundleIdentifier", "dev.x.App")]);
        let widget = plist(&[("CFBundleIdentifier", "dev.x.App.Widget")]);
        assert_eq!(
            check(&widget, &app).unwrap().as_deref(),
            Some(
                "The CFBundleVersion of an app extension ((null)) must match that of its \
                 containing parent app ((null))."
            )
        );
    }

    #[test]
    fn a_matching_bundle_passes() {
        let mut widget = plist(APP);
        widget[0].1 = "dev.x.App.Widget".to_string();
        assert_eq!(check(&widget, &plist(APP)).unwrap(), None);
    }
}
