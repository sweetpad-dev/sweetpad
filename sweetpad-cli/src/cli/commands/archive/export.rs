//! Exporting an archive on the Swift Build engine, as Xcode's export does:
//! a copy of the archived app signed again for the export method, by
//! sweetpad's own signing, and packed beside the ExportOptions.plist it was
//! made with: an iOS app into an IPA, a macOS app for the Mac App Store
//! into an installer package, and any other macOS app as the app itself.

use std::path::{Path, PathBuf};

use super::export_options::{self, ExportOptions};
use super::{ExportChoice, ExportMethod};
use crate::cli::codesign::installer::Installer;
use crate::cli::codesign::{self, resign};
use crate::cli::pkg;
use crate::cli::property_list::{self as plist, Plist};
use crate::cli::{CliError, Context, ErrorKind};
use sweetpad_core::scratch::ScratchDir;

/// How to export: the flags, or an ExportOptions.plist in their place.
pub(crate) struct Settings {
    pub method: ExportMethod,
    pub options: ExportOptions,
    /// Upload what the export writes: '--upload' or `destination=upload`.
    pub upload: bool,
}

pub(crate) fn settings(choice: &ExportChoice) -> Result<Settings, CliError> {
    let options = match &choice.export_options {
        Some(path) => export_options::read(path)?,
        None => ExportOptions::default(),
    };
    let method = match options.method {
        Some(method) => method,
        None => choice.method()?,
    };
    Ok(Settings {
        method,
        upload: choice.upload || options.upload,
        options,
    })
}

/// What an export wrote.
pub(crate) struct Exported {
    /// The identity the app was signed with, unless it was packed as it was.
    pub identity: Option<String>,
}

/// What the re-signer settled, for the ExportOptions.plist written beside
/// the export.
struct Signed {
    identity: String,
    team: String,
    profiles: Vec<(String, String)>,
    /// The installer identity a Mac App Store package is signed with.
    installer: Option<String>,
}

/// What an export of a macOS app writes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MacOutput {
    /// An installer package for the Mac App Store.
    Package,
    /// The app itself.
    App,
    /// The app, and a Developer ID Installer package of it when there's
    /// an identity to sign one.
    DeveloperId,
}

/// Export the app in `archive` into `export_dir`.
pub(crate) fn export(
    ctx: &Context,
    archive: &Path,
    settings: &Settings,
    export_dir: &Path,
) -> Result<Exported, CliError> {
    let app = archived_app(archive)?;
    let mac = app.join("Contents").is_dir();
    let (method, mac_output) = methods(settings.method, mac)?;
    for note in &settings.options.notes {
        ctx.out.note(note);
    }
    let name = app
        .file_name()
        .ok_or_else(|| CliError::new("the archived app has no name"))?;
    let stem = Path::new(name)
        .file_stem()
        .unwrap_or(name)
        .to_string_lossy()
        .into_owned();
    let file = export_dir.join(match mac_output {
        None => format!("{stem}.ipa"),
        Some(MacOutput::Package) => format!("{stem}.pkg"),
        Some(MacOutput::App | MacOutput::DeveloperId) => format!("{stem}.app"),
    });
    // zip adds to an existing file, which would keep a previous export's files.
    if file.exists() {
        return Err(CliError::new(format!(
            "{} already exists; remove it or export to another '--output-file' directory",
            file.display()
        )));
    }
    let stage = ScratchDir::new("sweetpad-export").map_err(|e| CliError::new(e.to_string()))?;
    let staged = if mac {
        stage.join(name)
    } else {
        stage.join("Payload").join(name)
    };
    copy_tree(&app, &staged)?;

    let mut signed = sign(ctx, settings, method, &staged, mac)?;

    std::fs::create_dir_all(export_dir)
        .map_err(|e| CliError::new(format!("can't create {}: {e}", export_dir.display())))?;
    match mac_output {
        None => {
            let mut contents = vec![Path::new("Payload")];
            if method == Some(resign::Method::AppStoreConnect) {
                contents.extend(apple_content(archive, &stage)?.into_iter().map(Path::new));
            }
            crate::cli::zip::create(&file, &stage, &contents)
                .map_err(|e| CliError::new(format!("cannot write {}: {e}", file.display())))?;
        }
        Some(MacOutput::App) => copy_tree(&staged, &file)?,
        Some(MacOutput::DeveloperId) => {
            copy_tree(&staged, &file)?;
            let team = signed.as_ref().map(|s| s.team.clone());
            match Installer::find(
                true,
                team.as_deref(),
                settings.options.installer_certificate.as_deref(),
            ) {
                Ok(installer) => {
                    let package = export_dir.join(format!("{stem}.pkg"));
                    ctx.out
                        .step("Writing the installer package", || {
                            pkg::product(&staged, Some(&installer), &package)
                        })
                        .map_err(|e| CliError::new(e).context("writing the package"))?;
                    if let Some(signed) = &mut signed {
                        signed.installer = Some(installer.name().to_string());
                    }
                }
                Err(why) => ctx.out.note(&format!("no installer package: {why}")),
            }
        }
        Some(MacOutput::Package) => {
            let team = signed.as_ref().map(|s| s.team.clone());
            let installer = Installer::find(
                false,
                team.as_deref(),
                settings.options.installer_certificate.as_deref(),
            )
            .map_err(|e| CliError::new(e).context("signing the package"))?;
            ctx.out
                .step("Writing the installer package", || {
                    pkg::product(&staged, Some(&installer), &file)
                })
                .map_err(|e| CliError::new(e).context("writing the package"))?;
            if let Some(signed) = &mut signed {
                signed.installer = Some(installer.name().to_string());
            }
        }
    }
    let written = effective_options(settings, method, signed.as_ref());
    let path = export_dir.join("ExportOptions.plist");
    std::fs::write(&path, plist::to_xml(&written))
        .map_err(|e| CliError::new(format!("can't write {}: {e}", path.display())))?;
    Ok(Exported {
        identity: signed.map(|s| s.identity),
    })
}

/// The re-signer's method, none to copy the app as it is, and what a macOS
/// export writes.
fn methods(
    method: ExportMethod,
    mac: bool,
) -> Result<(Option<resign::Method>, Option<MacOutput>), CliError> {
    let usage = |message: String| CliError::new(message).kind(ErrorKind::Usage);
    Ok(match (method, mac) {
        (ExportMethod::Debugging, false) => (Some(resign::Method::Debugging), None),
        (ExportMethod::AppStoreConnect, false) => (Some(resign::Method::AppStoreConnect), None),
        (ExportMethod::ReleaseTesting, false) => (Some(resign::Method::ReleaseTesting), None),
        (ExportMethod::Enterprise, false) => (Some(resign::Method::Enterprise), None),
        (ExportMethod::Debugging, true) => (Some(resign::Method::Debugging), Some(MacOutput::App)),
        (ExportMethod::AppStoreConnect, true) => (
            Some(resign::Method::AppStoreConnect),
            Some(MacOutput::Package),
        ),
        (ExportMethod::MacApplication, true) => (None, Some(MacOutput::App)),
        (ExportMethod::DeveloperId, true) => (
            Some(resign::Method::DeveloperId),
            Some(MacOutput::DeveloperId),
        ),
        (other @ (ExportMethod::DeveloperId | ExportMethod::MacApplication), false) => {
            return Err(usage(format!(
                "'{}' exports a macOS app, and this archive holds an iOS app",
                other.plist_value()
            )));
        }
        (other @ (ExportMethod::ReleaseTesting | ExportMethod::Enterprise), true) => {
            return Err(usage(format!(
                "'{}' exports an iOS app, and this archive holds a macOS app",
                other.plist_value()
            )));
        }
    })
}

/// Sign the staged copy of the app again for `method`, unless it exports
/// as it is.
fn sign(
    ctx: &Context,
    settings: &Settings,
    method: Option<resign::Method>,
    staged: &Path,
    mac: bool,
) -> Result<Option<Signed>, CliError> {
    // A development archive signed here already exports as it is, as long
    // as there are no identities to sign it again with.
    let resign = method
        .filter(|method| *method != resign::Method::Debugging || codesign::identity_configured());
    Ok(match resign {
        Some(method) => {
            let request = resign::Request {
                app: staged.to_path_buf(),
                method,
                team: settings.options.team.clone(),
                manual: settings.options.manual,
                signing_certificate: settings.options.signing_certificate.clone(),
                profiles: settings.options.profiles.clone(),
                icloud_environment: settings.options.icloud_environment.clone(),
            };
            let plan = ctx
                .out
                .step("Signing the app for export", || resign::plan(&request))
                .map_err(|e| CliError::new(e).context("exporting the archive"))?;
            resign::apply(&plan).map_err(|e| CliError::new(e).context("signing the export"))?;
            let profiles = plan
                .steps
                .iter()
                .filter_map(|step| {
                    Some((step.bundle_id()?.to_string(), step.profile()?.to_string()))
                })
                .collect();
            Some(Signed {
                identity: plan.identity,
                team: plan.team,
                profiles,
                installer: None,
            })
        }
        None if method.is_some() => {
            let profile = if mac {
                "Contents/embedded.provisionprofile"
            } else {
                "embedded.mobileprovision"
            };
            let resources = if mac {
                "Contents/_CodeSignature/CodeResources"
            } else {
                "_CodeSignature/CodeResources"
            };
            if !staged.join(profile).is_file() || !staged.join(resources).is_file() {
                return Err(CliError::new(format!(
                    "the archive is unsigned, and there's no identity to sign it with: set {} \
                     and {}, or archive with them set",
                    "SWEETPAD_CODE_SIGN_P12", "SWEETPAD_PROVISIONING_PROFILES"
                )));
            }
            None
        }
        None => None,
    })
}

/// The ExportOptions.plist the export followed, as Xcode writes it beside
/// its export.
fn effective_options(
    settings: &Settings,
    method: Option<resign::Method>,
    signed: Option<&Signed>,
) -> Plist {
    let options = &settings.options;
    let mut entries = vec![(
        "method".to_string(),
        Plist::String(settings.method.plist_value().to_string()),
    )];
    if let Some(team) = signed.map(|s| &s.team).or(options.team.as_ref()) {
        entries.push(("teamID".into(), Plist::String(team.clone())));
    }
    entries.push((
        "signingStyle".into(),
        Plist::String(
            if options.manual {
                "manual"
            } else {
                "automatic"
            }
            .into(),
        ),
    ));
    if let Some(signed) = signed {
        entries.push((
            "signingCertificate".into(),
            Plist::String(signed.identity.clone()),
        ));
        if let Some(installer) = &signed.installer {
            entries.push((
                "installerSigningCertificate".into(),
                Plist::String(installer.clone()),
            ));
        }
        entries.push((
            "provisioningProfiles".into(),
            Plist::Dict(
                signed
                    .profiles
                    .iter()
                    .map(|(bundle, profile)| (bundle.clone(), Plist::String(profile.clone())))
                    .collect(),
            ),
        ));
    }
    if let Some(environment) = &options.icloud_environment {
        entries.push((
            "iCloudContainerEnvironment".into(),
            Plist::String(environment.clone()),
        ));
    }
    entries.push((
        "destination".into(),
        Plist::String(if settings.upload { "upload" } else { "export" }.into()),
    ));
    if method == Some(resign::Method::AppStoreConnect) {
        entries.push(("uploadSymbols".into(), Plist::Bool(false)));
    }
    Plist::Dict(entries)
}

/// The folders beside an archive's products that an App Store IPA carries,
/// as Xcode's export copies them: the Swift runtime libraries the app
/// embeds, as the toolchain has them, and the stub executables of a
/// WatchKit app and of Messages apps and sticker packs. The archive build
/// writes each into the folder its setting names.
pub(crate) const APPLE_CONTENT: [(&str, &str); 4] = [
    ("SWIFT_STDLIB_TOOL_UNSIGNED_DESTINATION_DIR", SWIFT_SUPPORT),
    ("WATCHKIT_2_SUPPORT_FOLDER_PATH", WATCHKIT_SUPPORT),
    (
        "MESSAGES_APPLICATION_SUPPORT_FOLDER_PATH",
        "MessagesApplicationSupport",
    ),
    (
        "MESSAGES_APPLICATION_EXTENSION_SUPPORT_FOLDER_PATH",
        "MessagesApplicationExtensionSupport",
    ),
];

const SWIFT_SUPPORT: &str = "SwiftSupport";
const WATCHKIT_SUPPORT: &str = "WatchKitSupport2";

/// Copy the archive's [`APPLE_CONTENT`] into `stage`, beside the staged
/// `Payload`, and name the folders the IPA gets. Every Swift runtime library
/// the app embeds needs its original in `SwiftSupport`, or App Store Connect
/// rejects the upload. A WatchKit app's stub falls back to the copy inside
/// the watch app, for an archive made without the folder.
fn apple_content(archive: &Path, stage: &Path) -> Result<Vec<&'static str>, CliError> {
    let mut folders = Vec::new();
    for (_, folder) in APPLE_CONTENT {
        let source = archive.join(folder);
        if source.is_dir() {
            copy_tree(&source, &stage.join(folder))?;
            folders.push(folder);
        }
    }
    let payload = stage.join("Payload");
    if !folders.contains(&WATCHKIT_SUPPORT)
        && let Some(stub) = watchkit_stub(&payload)
    {
        copy_tree(&stub, &stage.join(WATCHKIT_SUPPORT).join("WK"))?;
        folders.push(WATCHKIT_SUPPORT);
    }
    let missing: Vec<String> = embedded_swift_libraries(&payload)
        .into_iter()
        .filter(|(platform, name)| {
            !stage
                .join(SWIFT_SUPPORT)
                .join(platform)
                .join(name)
                .is_file()
        })
        .map(|(platform, name)| format!("{platform}/{name}"))
        .collect();
    if !missing.is_empty() {
        return Err(CliError::new(format!(
            "the app embeds Swift runtime libraries the archive has no originals of in \
             {SWIFT_SUPPORT}, which App Store Connect requires: {}; archive the app again",
            missing.join(", ")
        )));
    }
    Ok(folders)
}

/// The `_WatchKitStub/WK` of the first WatchKit app in `payload`.
fn watchkit_stub(payload: &Path) -> Option<PathBuf> {
    let apps = std::fs::read_dir(payload).ok()?.flatten();
    apps.flat_map(|app| {
        std::fs::read_dir(app.path().join("Watch"))
            .into_iter()
            .flatten()
    })
    .flatten()
    .map(|watch| watch.path().join("_WatchKitStub/WK"))
    .find(|stub| stub.is_file())
}

/// The Swift runtime libraries embedded anywhere in `payload`, as the
/// platform of the bundle that holds them and the library's name.
fn embedded_swift_libraries(payload: &Path) -> std::collections::BTreeSet<(String, String)> {
    let mut found = std::collections::BTreeSet::new();
    let mut pending = vec![(payload.to_path_buf(), String::new())];
    while let Some((dir, platform)) = pending.pop() {
        let platform = sweetpad_lib::derived_data::plist_strings(&dir.join("Info.plist"))
            .into_iter()
            .find(|(key, _)| key == "DTPlatformName")
            .map_or(platform, |(_, value)| value);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push((path, platform.clone()));
            } else if name.starts_with("libswift")
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("dylib"))
                && dir.file_name().is_some_and(|parent| parent == "Frameworks")
            {
                found.insert((platform.clone(), name));
            }
        }
    }
    found
}

/// The one app an archive holds.
pub(crate) fn archived_app(archive: &Path) -> Result<PathBuf, CliError> {
    let applications = archive.join("Products/Applications");
    let apps: Vec<PathBuf> = std::fs::read_dir(&applications)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .collect();
    if apps.len() != 1 {
        return Err(CliError::new(format!(
            "archive contains {} applications in {}; expected one",
            apps.len(),
            applications.display()
        )));
    }
    Ok(apps[0].clone())
}

pub(crate) fn copy_tree(source: &Path, destination: &Path) -> Result<(), CliError> {
    let failed = |e: std::io::Error| CliError::new(format!("can't copy {}: {e}", source.display()));
    let metadata = std::fs::symlink_metadata(source).map_err(failed)?;
    if metadata.is_symlink() {
        let target = std::fs::read_link(source).map_err(failed)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, destination).map_err(failed)?;
    } else if metadata.is_dir() {
        std::fs::create_dir_all(destination).map_err(failed)?;
        for entry in std::fs::read_dir(source).map_err(failed)? {
            let entry = entry.map_err(failed)?;
            copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(failed)?;
        }
        std::fs::copy(source, destination).map_err(failed)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// A staged iOS app that embeds the Span runtime and a WatchKit app.
    fn staged_app(stage: &Path) {
        let app = stage.join("Payload/App.app");
        write(
            &app.join("Info.plist"),
            b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>\
              <key>DTPlatformName</key><string>iphoneos</string></dict></plist>",
        );
        write(
            &app.join("Frameworks/libswiftCompatibilitySpan.dylib"),
            b"signed",
        );
        write(&app.join("Watch/Watch.app/_WatchKitStub/WK"), b"stub");
    }

    #[test]
    fn an_app_store_ipa_carries_the_archives_apple_content() {
        let dir = crate::cli::testdir::TempDir::new("export-apple-content");
        let archive = dir.join("App.xcarchive");
        let stage = dir.join("stage");
        write(
            &archive.join("SwiftSupport/iphoneos/libswiftCompatibilitySpan.dylib"),
            b"original",
        );
        write(
            &archive.join("MessagesApplicationExtensionSupport/MessagesApplicationExtensionStub"),
            b"stub",
        );
        staged_app(&stage);
        let folders = apple_content(&archive, &stage).unwrap();
        assert_eq!(
            folders,
            [
                "SwiftSupport",
                "MessagesApplicationExtensionSupport",
                "WatchKitSupport2"
            ]
        );
        // The original, not the copy the app signed.
        assert_eq!(
            std::fs::read(stage.join("SwiftSupport/iphoneos/libswiftCompatibilitySpan.dylib"))
                .unwrap(),
            b"original"
        );
        // The watch app's stub stands in for an archive without the folder.
        assert_eq!(
            std::fs::read(stage.join("WatchKitSupport2/WK")).unwrap(),
            b"stub"
        );
    }

    #[test]
    fn an_embedded_swift_library_without_its_original_is_refused() {
        let dir = crate::cli::testdir::TempDir::new("export-missing-swift-support");
        let archive = dir.join("App.xcarchive");
        std::fs::create_dir_all(&archive).unwrap();
        let stage = dir.join("stage");
        staged_app(&stage);
        let error = apple_content(&archive, &stage).unwrap_err().to_string();
        assert!(
            error.contains("iphoneos/libswiftCompatibilitySpan.dylib"),
            "{error}"
        );
    }
}
