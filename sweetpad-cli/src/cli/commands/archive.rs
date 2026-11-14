//! `sweetpad archive` — `xcodebuild archive` plus IPA export: the
//! biggest missing chunk of "xcodebuild for humans". Archives the resolved
//! scheme for a generic device destination, then (unless `--no-export`) runs
//! `-exportArchive` with a generated ExportOptions.plist using automatic
//! signing — the fastlane-gym flow without the Ruby.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueEnum};

use crate::cli::appstore::key::{ApiKey, KeyArgs};
use crate::cli::commands::upload::{self, FollowArgs};
use crate::cli::output::Output;
use crate::cli::resolve::{self, Container};
use crate::cli::{
    CliError, CommandResult, Context, ErrorKind, Render, Rendered, buildlog, process, xcodebuild,
};

mod portable;

/// The export method — ExportOptions.plist `method` values (Xcode 15+ names).
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ExportMethod {
    /// Development/debugging distribution (ad-hoc install on registered devices).
    Debugging,
    /// App Store Connect upload.
    AppStoreConnect,
    /// TestFlight-external/ad-hoc style release testing.
    ReleaseTesting,
    /// Enterprise (in-house) distribution.
    Enterprise,
    /// macOS Developer ID distribution (notarizable, outside the App Store).
    DeveloperId,
    /// macOS application distribution (a signed '.app', no installer).
    MacApplication,
}

impl ExportMethod {
    pub(crate) fn plist_value(self) -> &'static str {
        match self {
            ExportMethod::Debugging => "debugging",
            ExportMethod::AppStoreConnect => "app-store-connect",
            ExportMethod::ReleaseTesting => "release-testing",
            ExportMethod::Enterprise => "enterprise",
            ExportMethod::DeveloperId => "developer-id",
            ExportMethod::MacApplication => "mac-application",
        }
    }
}

#[derive(Debug, Args)]
pub struct ArchiveArgs {
    #[command(flatten)]
    pub target: crate::cli::BuildTargetArgs,

    /// Build with xcodebuild or Swift Build (the default on Linux).
    #[arg(long, value_enum)]
    pub engine: Option<crate::cli::swbuild::Engine>,

    /// Output directory for the .xcarchive and exported .ipa
    /// (default: ./build).
    // `--output-file`, not `--output`: the global `-o/--output` selects the
    // output *mode* (json/ndjson) on every command, so it owns that flag.
    #[arg(long = "output-file", value_name = "DIR")]
    pub output_file: Option<PathBuf>,

    #[command(flatten)]
    pub export: ExportChoice,

    /// Archive only; skip the IPA export.
    #[arg(long, conflicts_with_all = ["export_options", "upload"])]
    pub no_export: bool,

    /// Print the exact xcodebuild invocation(s) that would run, then exit.
    #[arg(long)]
    pub show_command: bool,

    /// Extra arguments passed to 'xcodebuild archive' verbatim (after '--'),
    /// e.g. '-allowProvisioningUpdates'.
    #[arg(last = true, value_name = "XCODEBUILD_ARGS")]
    pub passthrough: Vec<String>,
}

/// How to export, and where the export goes next. Shared by 'archive' and
/// 'archive export'.
#[derive(Debug, Clone, Args)]
pub struct ExportChoice {
    /// How to export the archive (default 'debugging', or
    /// 'app-store-connect' with '--upload').
    #[arg(long, value_enum)]
    pub export_method: Option<ExportMethod>,

    /// Use an existing ExportOptions.plist instead of generating one.
    #[arg(long, value_name = "PLIST", conflicts_with = "export_method")]
    pub export_options: Option<PathBuf>,

    /// Upload the exported .ipa or .pkg to App Store Connect, as 'sweetpad
    /// upload' does. Implies '--export-method app-store-connect'.
    #[arg(long)]
    pub upload: bool,

    /// The app's App Store Connect ID, for '--upload'. Looked up from the
    /// bundle identifier when left out.
    #[arg(long, value_name = "ID", requires = "upload")]
    pub app_id: Option<String>,

    #[command(flatten)]
    pub key: KeyArgs,

    #[command(flatten)]
    pub follow: FollowArgs,
}

impl ExportChoice {
    /// The method to export with. `--upload` means App Store Connect, and any
    /// other method beside it is refused by the command line alone.
    pub(crate) fn method(&self) -> Result<ExportMethod, CliError> {
        match (self.export_method, self.upload) {
            (Some(ExportMethod::AppStoreConnect) | None, true) => Ok(ExportMethod::AppStoreConnect),
            (Some(other), true) => Err(CliError::new(format!(
                "'--upload' sends a build to App Store Connect, which takes only \
                 '--export-method app-store-connect'; '{}' builds can't be uploaded",
                other.plist_value()
            ))
            .kind(ErrorKind::Usage)),
            (method, false) => Ok(method.unwrap_or(ExportMethod::Debugging)),
        }
    }

    /// Refusals decided by the command line: a method that can't upload, and
    /// an upload without an API key.
    fn check(&self) -> Result<(), CliError> {
        self.method()?;
        if self.upload {
            if let Some(plist) = &self.export_options {
                let values = sweetpad_lib::derived_data::plist_strings(plist);
                let method = values.iter().find(|(k, _)| k == "method").map(|(_, v)| v.as_str());
                if !matches!(method, Some("app-store-connect" | "app-store")) {
                    return Err(CliError::new(format!(
                        "'--upload' needs an App Store Connect export, but {} sets method {}",
                        plist.display(),
                        method.map_or_else(|| "nothing".to_string(), |m| format!("'{m}'"))
                    ))
                    .kind(ErrorKind::Usage));
                }
            }
            ApiKey::resolve(&self.key)?;
        }
        Ok(())
    }
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Export an existing .xcarchive again, with another method or
    /// ExportOptions.plist, and optionally upload the result.
    Export(ExportArgs),
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// The .xcarchive to export.
    pub archive: PathBuf,

    /// Export with xcodebuild or with sweetpad's own signing (the default on
    /// Linux).
    #[arg(long, value_enum)]
    pub engine: Option<crate::cli::swbuild::Engine>,

    /// Output directory for the export (default: ./build).
    #[arg(long = "output-file", value_name = "DIR")]
    pub output_file: Option<PathBuf>,

    #[command(flatten)]
    pub export: ExportChoice,

    /// Print the exact invocation that would run, then exit.
    #[arg(long)]
    pub show_command: bool,
}

/// The archive/export outcome.
struct ArchiveReport {
    archive: String,
    ipa_dir: Option<String>,
    upload: Option<upload::Report>,
}

impl ArchiveReport {
    /// Rendered, exiting 1 when Apple rejected the upload.
    fn rendered(self) -> Rendered {
        if self.upload.as_ref().is_some_and(|u| u.upload.failed()) {
            Rendered::data_with_exit(self, 1)
        } else {
            Rendered::data(self)
        }
    }
}

impl Render for ArchiveReport {
    fn human(&self, out: &Output) {
        out.note(&format!("archive: {}", self.archive));
        if let Some(dir) = &self.ipa_dir {
            out.note(&format!("exported: {dir}"));
        }
        if let Some(upload) = &self.upload {
            upload.human(out);
        }
    }

    fn json(&self) -> serde_json::Value {
        let mut data = serde_json::json!({
            "archive": self.archive,
            "exportedTo": self.ipa_dir,
        });
        if let Some(upload) = &self.upload {
            data["upload"] = upload.json();
        }
        data
    }
}

/// Upload the one .ipa or .pkg an export wrote to `export_dir`.
fn upload_export(ctx: &Context, choice: &ExportChoice, export_dir: &Path) -> Result<upload::Report, CliError> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(export_dir)
        .map_err(|e| CliError::new(format!("can't read {}: {e}", export_dir.display())))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("ipa") || e.eq_ignore_ascii_case("pkg"))
        })
        .collect();
    found.sort();
    let [file] = found.as_slice() else {
        return Err(CliError::new(format!(
            "the export in {} holds {} .ipa/.pkg files; upload one with 'sweetpad upload FILE'",
            export_dir.display(),
            found.len()
        )));
    };
    upload::upload_file(ctx, file, choice.app_id.as_deref(), &choice.key, &choice.follow)
        .map_err(|e| e.context("uploading the export"))
}

pub fn run(ctx: &mut Context, args: &ArchiveArgs, action: Option<&Action>) -> CommandResult {
    if let Some(Action::Export(export)) = action {
        return run_export(ctx, export);
    }
    args.export.check()?;
    ctx.targeting = args.target.clone().into();
    resolve::reject_on_destination_conflict(ctx)?;
    if let Some(on) = &ctx.targeting.on {
        on_platform(on)?;
    }
    let passthrough = ctx.xcodebuild_args(xcodebuild::Action::Archive, &args.passthrough)?;
    let mut resolved = resolve::resolve(ctx)?;
    if matches!(resolved.container, Container::SwiftPackage(_)) {
        return Err(CliError::new(
            "a Swift package has no app to archive; archive works on Xcode projects/workspaces",
        ));
    }

    let schemes = resolve::schemes_for_context(ctx, &resolved.container)?;
    let scheme = resolve::settle_scheme(ctx, &mut resolved, &schemes, !args.show_command)?;
    let configuration = archive_configuration(ctx, &resolved, &scheme)?;
    let destination = archive_destination(ctx, &resolved, &scheme, &configuration)?;

    run_resolved(
        ctx,
        args,
        &resolved,
        &scheme,
        configuration,
        destination,
        &passthrough,
    )
}

fn run_resolved(
    ctx: &Context,
    args: &ArchiveArgs,
    resolved: &resolve::Resolved,
    scheme: &str,
    configuration: String,
    destination: String,
    passthrough: &[String],
) -> CommandResult {
    // A relative output dir must mean the same directory for the CLI's own
    // writes (create_dir_all, the generated plist) and for xcodebuild, which
    // runs from the container's parent — absolutize once against the CLI cwd,
    // collapsing `.` and `..` so the paths the report prints read cleanly.
    let out_dir = sweetpad_lib::project::absolutize(
        args.output_file.as_deref().unwrap_or(Path::new("build")),
    );
    let archive_path = out_dir.join(format!("{scheme}.xcarchive"));

    if args
        .engine
        .unwrap_or_else(crate::cli::swbuild::Engine::for_host)
        == crate::cli::swbuild::Engine::SwiftBuild
    {
        return portable::run(
            ctx,
            args,
            resolved,
            scheme,
            &configuration,
            &destination,
            &out_dir,
        );
    }

    let mut archive_args: Vec<String> = vec![
        "archive".into(),
        "-scheme".into(),
        scheme.to_string(),
        "-configuration".into(),
        configuration,
        "-destination".into(),
        destination,
        "-archivePath".into(),
        archive_path.display().to_string(),
    ];
    if let Some(sdk) = &resolved.sdk {
        archive_args.push("-sdk".into());
        archive_args.push(sdk.clone());
    }
    archive_args.extend(xcodebuild::container_args(&resolved.container));
    archive_args.extend(passthrough.iter().cloned());

    let cwd = xcodebuild::working_dir(&resolved.container);
    let export_dir = out_dir.join("export");
    let plist_path = args.export.export_options.as_deref().map_or_else(
        || out_dir.join("ExportOptions.plist"),
        sweetpad_lib::project::absolutize,
    );
    let export_args: Vec<String> = vec![
        "-exportArchive".into(),
        "-archivePath".into(),
        archive_path.display().to_string(),
        "-exportPath".into(),
        export_dir.display().to_string(),
        "-exportOptionsPlist".into(),
        plist_path.display().to_string(),
    ];

    // The dry run previews *both* steps (the flag says "invocation(s)"), plus
    // the plist that would be generated.
    if args.show_command {
        let archive_preview = xcodebuild::CommandPreview {
            program: "xcodebuild",
            args: archive_args,
            cwd: cwd.clone(),
        };
        let export_preview = (!args.no_export).then(|| xcodebuild::CommandPreview {
            program: "xcodebuild",
            args: export_args,
            cwd: cwd.clone(),
        });
        let generated_plist = (!args.no_export && args.export.export_options.is_none())
            .then(|| plist_path.display().to_string());
        return Ok(Rendered::data(ArchivePreview {
            archive: Some(archive_preview),
            export: export_preview,
            generated_plist,
            export_method: args.export.method()?,
            upload: args.export.upload,
        }));
    }

    let _ = std::fs::create_dir_all(&out_dir);
    run_xcodebuild(ctx, &archive_args, cwd.as_deref(), "Archiving")
        .map_err(|e| e.context("archiving the app"))?;

    if args.no_export {
        return Ok(Rendered::data(ArchiveReport {
            archive: archive_path.display().to_string(),
            ipa_dir: None,
            upload: None,
        }));
    }

    // Export: an explicit plist wins; otherwise generate one with automatic
    // signing for the chosen method.
    if args.export.export_options.is_none() {
        std::fs::write(&plist_path, export_options_plist(args.export.method()?))
            .map_err(|e| CliError::new(format!("failed to write {}: {e}", plist_path.display())))?;
    }
    run_xcodebuild(ctx, &export_args, cwd.as_deref(), "Exporting")
        .map_err(|e| e.context("exporting the archive"))?;
    let upload = if args.export.upload {
        Some(upload_export(ctx, &args.export, &export_dir)?)
    } else {
        None
    };
    Ok(ArchiveReport {
        archive: archive_path.display().to_string(),
        ipa_dir: Some(export_dir.display().to_string()),
        upload,
    }
    .rendered())
}

/// `archive export`: export an existing archive, with xcodebuild or (on the
/// Swift Build engine) sweetpad's own signing.
fn run_export(ctx: &Context, args: &ExportArgs) -> CommandResult {
    args.export.check()?;
    let archive = sweetpad_lib::project::absolutize(&args.archive);
    if !archive.join("Info.plist").is_file() {
        return Err(CliError::new(format!(
            "{} isn't an .xcarchive (it has no Info.plist)",
            archive.display()
        ))
        .kind(ErrorKind::Usage));
    }
    let out_dir = sweetpad_lib::project::absolutize(
        args.output_file.as_deref().unwrap_or(Path::new("build")),
    );
    let export_dir = out_dir.join("export");
    if args
        .engine
        .unwrap_or_else(crate::cli::swbuild::Engine::for_host)
        == crate::cli::swbuild::Engine::SwiftBuild
    {
        return portable::export_existing(ctx, args, &archive, &export_dir);
    }
    let plist_path = args.export.export_options.as_deref().map_or_else(
        || out_dir.join("ExportOptions.plist"),
        sweetpad_lib::project::absolutize,
    );
    let export_args: Vec<String> = vec![
        "-exportArchive".into(),
        "-archivePath".into(),
        archive.display().to_string(),
        "-exportPath".into(),
        export_dir.display().to_string(),
        "-exportOptionsPlist".into(),
        plist_path.display().to_string(),
    ];
    if args.show_command {
        return Ok(Rendered::data(ArchivePreview {
            archive: None,
            export: Some(xcodebuild::CommandPreview {
                program: "xcodebuild",
                args: export_args,
                cwd: None,
            }),
            generated_plist: args
                .export
                .export_options
                .is_none()
                .then(|| plist_path.display().to_string()),
            export_method: args.export.method()?,
            upload: args.export.upload,
        }));
    }
    let _ = std::fs::create_dir_all(&out_dir);
    if args.export.export_options.is_none() {
        std::fs::write(&plist_path, export_options_plist(args.export.method()?))
            .map_err(|e| CliError::new(format!("failed to write {}: {e}", plist_path.display())))?;
    }
    run_xcodebuild(ctx, &export_args, None, "Exporting")
        .map_err(|e| e.context("exporting the archive"))?;
    let upload = if args.export.upload {
        Some(upload_export(ctx, &args.export, &export_dir)?)
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

/// The archive's configuration: flag > user config > sweetpad.toml > the
/// scheme's `ArchiveAction` (what Xcode's Product > Archive builds) >
/// `Release`. The *remembered* layer is deliberately skipped — a Debug pick
/// remembered from the daily build loop must not silently turn distribution
/// archives into Debug builds.
fn archive_configuration(
    ctx: &mut Context,
    resolved: &resolve::Resolved,
    scheme: &str,
) -> Result<String, CliError> {
    let key = resolved.container.key();
    let cfg = ctx.config.for_project(&key);
    let pf = ctx.project_file(&resolved.container).configuration.clone();
    let configuration = ctx
        .targeting
        .configuration
        .clone()
        .or(cfg.configuration)
        .or(pf)
        .or_else(|| {
            resolve::scheme_configuration(
                ctx,
                &resolved.container,
                scheme,
                resolve::SchemeAction::Archive,
            )
        })
        .unwrap_or_else(|| "Release".to_string());
    let candidates = resolve::configurations(&resolved.container).unwrap_or_default();
    if configuration == "Release" && !candidates.is_empty() && !candidates.contains(&configuration)
    {
        // A project without a Release configuration drops to the picker
        // rather than dying inside xcodebuild.
        return resolve::choose(ctx, "configuration", None, &candidates);
    }
    resolve::validate_choice("configuration", &configuration, &candidates)?;
    Ok(configuration)
}

/// The archive's `-destination`: an explicit `--destination` passes through
/// verbatim; `--on` names a platform ([`on_platform`]) and maps to the
/// matching `generic/platform=…`; default iOS.
fn archive_destination(
    ctx: &Context,
    resolved: &resolve::Resolved,
    scheme: &str,
    configuration: &str,
) -> Result<String, CliError> {
    if let Some(dest) = &ctx.targeting.destination {
        return Ok(dest.clone());
    }
    let Some(on) = &ctx.targeting.on else {
        // Without an explicit target, follow what the scheme actually builds
        // for — a mac-only project archived as `generic/platform=iOS` just
        // fails inside xcodebuild. Mirrors the build path's auto-targeting.
        let mac_only =
            resolve::SupportedPlatforms::resolve(resolved.container.path(), scheme, configuration)
                .is_some_and(|p| p.is_mac_only());
        return Ok(if mac_only {
            "generic/platform=macOS".to_string()
        } else {
            "generic/platform=iOS".to_string()
        });
    };
    Ok(format!("generic/platform={}", on_platform(on)?))
}

/// The platform an archive's `--on` names: a platform word (`mac`, `ios`,
/// `watchos`, `tvos`, `visionos`) or `device`. Archives target device
/// platforms, so a simulator reference is refused rather than resolved. The
/// word alone decides the refusal, so it is a usage error, and [`run`]
/// checks it before looking for the project.
fn on_platform(on: &str) -> Result<&'static str, CliError> {
    let other = on.to_ascii_lowercase();
    let family = if other == "device" {
        Some("iOS")
    } else {
        resolve::platform_word(&other)
    };
    family
        .and_then(sweetpad_lib::destination::Platform::device_for_os)
        .map(|p| p.label)
        .ok_or_else(|| {
            CliError::new(format!(
                "archive targets a generic device platform; --on {other:?} doesn't name one \
                 (use mac, ios, watchos, tvos, or visionos — or --destination for the raw form)"
            ))
            .kind(ErrorKind::Usage)
        })
}

/// The `--show-command` payload: the xcodebuild invocations, the
/// ExportOptions.plist that would be generated, and whether an upload follows.
struct ArchivePreview {
    archive: Option<xcodebuild::CommandPreview>,
    export: Option<xcodebuild::CommandPreview>,
    generated_plist: Option<String>,
    export_method: ExportMethod,
    upload: bool,
}

impl Render for ArchivePreview {
    fn human(&self, out: &Output) {
        if let Some(archive) = &self.archive {
            archive.human(out);
        }
        if let Some(export) = &self.export {
            export.human(out);
        }
        if let Some(plist) = &self.generated_plist {
            out.note(&format!(
                "{plist} will be generated (method {}, automatic signing)",
                self.export_method.plist_value()
            ));
        }
        if self.upload {
            out.note("then the exported file is uploaded to App Store Connect");
        }
    }

    fn json(&self) -> serde_json::Value {
        let commands: Vec<serde_json::Value> = self
            .archive
            .iter()
            .chain(self.export.iter())
            .map(xcodebuild::CommandPreview::json)
            .collect();
        serde_json::json!({
            "commands": commands,
            "generatedExportOptions": self.generated_plist,
            "upload": self.upload,
        })
    }
}

/// Run one xcodebuild step with the standard output modes (beautified /
/// captured-for-json / raw under -v).
fn run_xcodebuild(
    ctx: &Context,
    args: &[String],
    cwd: Option<&Path>,
    label: &str,
) -> Result<(), CliError> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let ok = if ctx.out.is_json() || ctx.out.is_ndjson() {
        let run = process::run_captured("xcodebuild", &refs, cwd)?;
        if !run.success {
            return Err(CliError::new(format!(
                "xcodebuild exited with a non-zero status:\n{}",
                run.tail
            ))
            .kind(ErrorKind::BuildFailure));
        }
        true
    } else if ctx.out.is_verbose() {
        process::run("xcodebuild", &refs, cwd, false)?
    } else {
        buildlog::run("xcodebuild", &refs, cwd, &ctx.out, label)?
    };
    if ok {
        Ok(())
    } else {
        Err(CliError::new("xcodebuild exited with a non-zero status").kind(ErrorKind::BuildFailure))
    }
}

/// A minimal ExportOptions.plist: the chosen method plus automatic signing —
/// Xcode fills in team/profiles from the project's settings.
fn export_options_plist(method: ExportMethod) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>method</key>
	<string>{}</string>
	<key>signingStyle</key>
	<string>automatic</string>
</dict>
</plist>
"#,
        method.plist_value()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A word that names no device platform is refused by the word alone,
    /// so it exits 2 like the refusals clap makes itself.
    #[test]
    fn an_on_that_names_no_platform_is_a_usage_error() {
        assert_eq!(on_platform("Mac").unwrap(), "macOS");
        assert_eq!(on_platform("device").unwrap(), "iOS");
        let err = on_platform("toaster").expect_err("toaster named a platform");
        assert_eq!(err.error_kind(), ErrorKind::Usage);
        assert!(err.to_string().contains("--on \"toaster\""), "{err}");
    }

    #[test]
    fn export_options_carry_method_and_automatic_signing() {
        let plist = export_options_plist(ExportMethod::AppStoreConnect);
        assert!(plist.contains("<string>app-store-connect</string>"));
        assert!(plist.contains("<string>automatic</string>"));
        assert!(plist.starts_with("<?xml"));
    }
}
