//! Portable .xcarchive creation and development IPA packaging. Swift Build
//! performs an install action; export preserves the resulting signing identity.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{ArchiveArgs, ArchivePreview, ArchiveReport, ExportMethod};
use crate::cli::resolve::Resolved;
use crate::cli::{
    CliError, CommandResult, Context, ErrorKind, Rendered, process, swbuild, xcodebuild, xml_plist,
};
use sweetpad_core::scratch::ScratchDir;

pub(super) fn run(
    ctx: &Context,
    args: &ArchiveArgs,
    resolved: &Resolved,
    scheme: &str,
    configuration: &str,
    destination: &str,
    out_dir: &Path,
) -> CommandResult {
    if args.export_options.is_some() {
        return Err(CliError::new("Swift Build export uses the identity and profiles selected for the build; custom ExportOptions.plist files require xcodebuild").kind(ErrorKind::Usage));
    }
    if !args.no_export && !matches!(args.export_method, ExportMethod::Debugging) {
        return Err(CliError::new("Swift Build currently exports development IPAs with '--export-method debugging'; other export methods require xcodebuild").kind(ErrorKind::Usage));
    }
    let parsed = sweetpad_lib::destination::parse_destination_arg(destination)
        .ok_or_else(|| CliError::new("archive needs a generic device platform"))?;
    if parsed.platform.contains("simulator") || (!args.no_export && parsed.platform != "iphoneos") {
        return Err(CliError::new(
            "IPA export needs an iOS device destination; use '--no-export' to archive a macOS app",
        )
        .kind(ErrorKind::Usage));
    }
    let archive = out_dir.join(format!("{scheme}.xcarchive"));
    let mut settings = ctx.xcodebuild_args(xcodebuild::Action::Archive, &args.passthrough)?;
    settings.extend([
        format!("DSTROOT={}", archive.join("Products").display()),
        "DEPLOYMENT_LOCATION=YES".to_string(),
        "DEPLOYMENT_POSTPROCESSING=YES".to_string(),
    ]);
    let plan = xcodebuild::BuildPlan {
        action: xcodebuild::BuildAction::Archive,
        container: &resolved.container,
        scheme,
        configuration,
        destination: Some(destination),
        sdk: resolved.sdk.as_deref(),
        clean: false,
        hot: false,
        hot_entitlements: None,
        result_bundle: None,
        passthrough: &settings,
    };
    if args.show_command {
        return Ok(Rendered::data(ArchivePreview {
            archive: swbuild::command(&plan)?,
            export: (!args.no_export).then(|| xcodebuild::CommandPreview {
                program: "zip",
                args: vec![
                    "-yr".into(),
                    out_dir
                        .join("export")
                        .join(format!("{scheme}.ipa"))
                        .display()
                        .to_string(),
                    "Payload".into(),
                ],
                cwd: None,
            }),
            generated_plist: None,
            export_method: args.export_method,
        }));
    }
    // Never combine a new install with an archive left by a previous build.
    if archive.exists() {
        return Err(CliError::new(format!(
            "{} already exists; choose another '--output-file' directory",
            archive.display()
        )));
    }
    swbuild::run(&plan, &ctx.out).map_err(|e| e.or_kind(ErrorKind::BuildFailure))?;
    let app = archived_app(&archive)?;
    finish_archive(&archive, &app, scheme, &plan)?;
    let ipa_dir = if args.no_export {
        None
    } else {
        let export = out_dir.join("export");
        export_ipa(&app, &export.join(format!("{scheme}.ipa")))?;
        Some(export.display().to_string())
    };
    Ok(Rendered::data(ArchiveReport {
        archive: archive.display().to_string(),
        ipa_dir,
    }))
}

fn archived_app(archive: &Path) -> Result<PathBuf, CliError> {
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

fn finish_archive(
    archive: &Path,
    app: &Path,
    scheme: &str,
    plan: &xcodebuild::BuildPlan<'_>,
) -> Result<(), CliError> {
    let values = sweetpad_lib::derived_data::plist_strings(&app.join("Info.plist"));
    let value = |key: &str| {
        values
            .iter()
            .find(|(k, _)| k == key)
            .map_or("", |(_, v)| v.as_str())
    };
    let name = app
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| CliError::new("archive app has no name"))?;
    let date = process::capture("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"], None)?;
    let info = archive_info(
        name,
        scheme,
        value("CFBundleIdentifier"),
        value("CFBundleShortVersionString"),
        value("CFBundleVersion"),
        date.trim(),
    );
    std::fs::write(archive.join("Info.plist"), info).map_err(|e| CliError::new(e.to_string()))?;
    // The build's dSYM stays in the configuration products folder even when
    // its app is installed into DSTROOT.
    let settings = xcodebuild::command_line_settings(plan.passthrough, plan.container);
    let home = std::env::var("HOME").unwrap_or_default();
    let locations = sweetpad_lib::derived_data::resolve(
        &sweetpad_lib::derived_data::ContainerKey::of(plan.container.path()),
        &home,
        settings.derived_data_path.as_deref(),
        true,
    );
    let suffix =
        sweetpad_lib::destination::parse_destination_arg(plan.destination.unwrap_or_default())
            .map_or(String::new(), |d| {
                if d.platform == "macosx" {
                    String::new()
                } else {
                    format!("-{}", d.platform)
                }
            });
    let products = locations
        .products
        .join(format!("{}{suffix}", plan.configuration));
    copy_symbols(&products, app, &archive.join("dSYMs"))?;
    Ok(())
}

fn copy_symbols(products: &Path, app: &Path, destination: &Path) -> Result<(), CliError> {
    let mut bundles = BTreeSet::new();
    let mut pending = vec![app.to_path_buf()];
    while let Some(path) = pending.pop() {
        if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && path.extension().is_some_and(|e| {
                ["app", "appex", "framework", "xpc", "bundle"]
                    .iter()
                    .any(|s| e == *s)
            })
        {
            bundles.insert(format!("{name}.dSYM"));
        }
        pending.extend(subdirectories(&path)?);
    }
    let mut pending = vec![products.to_path_buf()];
    while let Some(path) = pending.pop() {
        for child in subdirectories(&path)? {
            if child.extension().is_some_and(|e| e == "dSYM") {
                if child
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| bundles.contains(n))
                {
                    copy_tree(&child, &destination.join(child.file_name().unwrap()))?;
                }
            } else if !child.extension().is_some_and(|e| {
                ["app", "appex", "framework", "bundle"]
                    .iter()
                    .any(|s| e == *s)
            }) {
                pending.push(child);
            }
        }
    }
    Ok(())
}

fn subdirectories(path: &Path) -> Result<Vec<PathBuf>, CliError> {
    if !path.is_dir() {
        return Ok(Vec::new());
    }
    std::fs::read_dir(path)
        .map_err(|e| CliError::new(e.to_string()))?
        .filter_map(|e| match e {
            Ok(entry) => match entry.file_type() {
                Ok(kind) if kind.is_dir() => Some(Ok(entry.path())),
                Ok(_) => None,
                Err(e) => Some(Err(CliError::new(e.to_string()))),
            },
            Err(e) => Some(Err(CliError::new(e.to_string()))),
        })
        .collect()
}

fn archive_info(
    app: &str,
    scheme: &str,
    bundle: &str,
    version: &str,
    build: &str,
    date: &str,
) -> String {
    let e = xml_plist::escape;
    xml_plist::document(&format!(
        "<dict>\n<key>ApplicationProperties</key><dict>\n<key>ApplicationPath</key><string>Applications/{}</string>\n<key>CFBundleIdentifier</key><string>{}</string>\n<key>CFBundleShortVersionString</key><string>{}</string>\n<key>CFBundleVersion</key><string>{}</string>\n</dict>\n<key>ArchiveVersion</key><integer>2</integer>\n<key>CreationDate</key><date>{}</date>\n<key>Name</key><string>{}</string>\n<key>SchemeName</key><string>{}</string>\n</dict>",
        e(app),
        e(bundle),
        e(version),
        e(build),
        e(date),
        e(scheme),
        e(scheme)
    ))
}

fn export_ipa(app: &Path, output: &Path) -> Result<(), CliError> {
    if !app.join("embedded.mobileprovision").is_file()
        || !app.join("_CodeSignature/CodeResources").is_file()
    {
        return Err(CliError::new(
            "the archive is unsigned; set SWEETPAD_CODE_SIGN_P12 and SWEETPAD_PROVISIONING_PROFILES before archiving, or use '--no-export'",
        ));
    }
    let stage = ScratchDir::new("sweetpad-ipa").map_err(|e| CliError::new(e.to_string()))?;
    copy_tree(
        app,
        &stage.join("Payload").join(
            app.file_name()
                .ok_or_else(|| CliError::new("app has no name"))?,
        ),
    )?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CliError::new(e.to_string()))?;
    }
    // zip updates an existing archive, which could retain stale bundle files.
    if output.exists() {
        return Err(CliError::new(format!(
            "{} already exists",
            output.display()
        )));
    }
    let output = sweetpad_lib::project::absolutize(output);
    process::capture(
        "zip",
        &["-qry", &output.display().to_string(), "Payload"],
        Some(&stage),
    )
    .map(|_| ())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), CliError> {
    let metadata = std::fs::symlink_metadata(source).map_err(|e| CliError::new(e.to_string()))?;
    if metadata.is_symlink() {
        let target = std::fs::read_link(source).map_err(|e| CliError::new(e.to_string()))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, destination)
            .map_err(|e| CliError::new(e.to_string()))?;
    } else if metadata.is_dir() {
        std::fs::create_dir_all(destination).map_err(|e| CliError::new(e.to_string()))?;
        for entry in std::fs::read_dir(source).map_err(|e| CliError::new(e.to_string()))? {
            let entry = entry.map_err(|e| CliError::new(e.to_string()))?;
            copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CliError::new(e.to_string()))?;
        }
        std::fs::copy(source, destination).map_err(|e| CliError::new(e.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_collects_symbols_for_embedded_products() {
        let dir = crate::cli::testdir::TempDir::new("archive-embedded-symbols");
        let app = dir.join("Products/App.app");
        std::fs::create_dir_all(app.join("PlugIns/Widget.appex")).unwrap();
        std::fs::create_dir_all(app.join("Frameworks/Library.framework")).unwrap();
        let products = dir.join("build");
        for name in ["App.app", "Widget.appex", "Library.framework", "Other.app"] {
            let symbol = products
                .join("PackageFrameworks")
                .join(format!("{name}.dSYM"));
            std::fs::create_dir_all(&symbol).unwrap();
            std::fs::write(symbol.join("fixture"), name).unwrap();
        }
        let symbols = dir.join("archive/dSYMs");
        copy_symbols(&products, &app, &symbols).unwrap();
        for name in ["App.app", "Widget.appex", "Library.framework"] {
            assert!(symbols.join(format!("{name}.dSYM/fixture")).is_file());
        }
        assert!(!symbols.join("Other.app.dSYM").exists());
    }
    #[test]
    fn unsigned_archives_cannot_be_exported_as_installable_ipas() {
        let dir = crate::cli::testdir::TempDir::new("sweetpad-unsigned-ipa");
        assert!(
            export_ipa(&dir.join("App.app"), &dir.join("App.ipa"))
                .unwrap_err()
                .to_string()
                .contains("unsigned")
        );
    }
    #[test]
    fn archive_metadata_preserves_the_application_path_and_typed_values() {
        let info = archive_info(
            "A&B.app",
            "A&B",
            "dev.test",
            "1.0",
            "2",
            "2026-11-06T00:00:00Z",
        );
        let root = sweetpad_lib::xcscheme::parse(&info).unwrap();
        assert!(info.contains("Applications/A&amp;B.app"));
        assert_eq!(
            root.child("dict")
                .unwrap()
                .children_named("integer")
                .next()
                .unwrap()
                .text,
            "2"
        );
        assert!(info.contains("<date>2026-11-06T00:00:00Z</date>"));
    }
}
