//! Portable .xcarchive creation on the Swift Build engine, which performs
//! an install action into the archive; `export` signs and packs the app.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::export::{self, archived_app, copy_tree};
use super::{ArchiveArgs, ArchivePreview, ArchiveReport, ExportArgs};
use crate::cli::commands::upload;
use crate::cli::resolve::Resolved;
use crate::cli::{
    CliError, CommandResult, Context, ErrorKind, Rendered, process, swbuild, xcodebuild, xml_plist,
};

pub(super) fn run(
    ctx: &Context,
    args: &ArchiveArgs,
    resolved: &Resolved,
    scheme: &str,
    configuration: &str,
    destination: &str,
    out_dir: &Path,
) -> CommandResult {
    let exporting = if args.no_export {
        None
    } else {
        Some(export::settings(&args.export)?)
    };
    let parsed = sweetpad_lib::destination::parse_destination_arg(destination)
        .ok_or_else(|| CliError::new("archive needs a generic device platform"))?;
    if parsed.platform.contains("simulator") {
        return Err(
            CliError::new("archive needs a device destination, not a simulator")
                .kind(ErrorKind::Usage),
        );
    }
    if exporting.is_some() && parsed.platform == "macosx" {
        return Err(CliError::new(
            "exporting a macOS archive isn't supported by sweetpad's own signing yet; use \
             '--no-export' to archive a macOS app",
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
            archive: Some(swbuild::command(&plan)?),
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
            export_method: exporting
                .as_ref()
                .map_or(args.export.method()?, |s| s.method),
            upload: exporting.as_ref().is_some_and(|s| s.upload),
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
    swbuild::scheme_actions::archive(false, &archive, &plan, &ctx.out)?;
    finish_archive(&archive, &app, scheme, &plan)?;
    swbuild::scheme_actions::archive(true, &archive, &plan, &ctx.out)?;
    let Some(exporting) = exporting else {
        return Ok(ArchiveReport {
            archive: archive.display().to_string(),
            ipa_dir: None,
            upload: None,
        }
        .rendered());
    };
    export_and_upload(
        ctx,
        &args.export,
        &archive,
        &exporting,
        &out_dir.join("export"),
    )
}

/// Export `archive` into `export_dir`, then upload it when asked.
fn export_and_upload(
    ctx: &Context,
    choice: &super::ExportChoice,
    archive: &Path,
    settings: &export::Settings,
    export_dir: &Path,
) -> CommandResult {
    let exported = export::export(ctx, archive, settings, export_dir)?;
    if let Some(identity) = &exported.identity {
        ctx.out.note(&format!("signed with {identity}"));
    }
    let upload = if settings.upload {
        let report = upload::upload_file(
            ctx,
            &exported.file,
            choice.app_id.as_deref(),
            &choice.key,
            &choice.follow,
        )
        .map_err(|e| e.context("uploading the export"))?;
        Some(report)
    } else {
        None
    };
    Ok(ArchiveReport {
        archive: archive.display().to_string(),
        ipa_dir: Some(export_dir.display().to_string()),
        upload,
    }
    .rendered())
}

/// `archive export` on the Swift Build engine.
pub(super) fn export_existing(
    ctx: &Context,
    args: &ExportArgs,
    archive: &Path,
    export_dir: &Path,
) -> CommandResult {
    let settings = export::settings(&args.export)?;
    if args.show_command {
        return Ok(Rendered::data(ArchivePreview {
            archive: None,
            export: Some(xcodebuild::CommandPreview {
                program: "zip",
                args: vec![
                    "-qryX".into(),
                    export_dir.join("<App>.ipa").display().to_string(),
                    "Payload".into(),
                ],
                cwd: None,
            }),
            generated_plist: None,
            export_method: settings.method,
            upload: settings.upload,
        }));
    }
    export_and_upload(ctx, &args.export, archive, &settings, export_dir)
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
