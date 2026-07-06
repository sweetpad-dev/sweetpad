//! `sweetpad archive` — `xcodebuild archive` plus IPA export: the
//! biggest missing chunk of "xcodebuild for humans". Archives the resolved
//! scheme for a generic device destination, then (unless `--no-export`) runs
//! `-exportArchive` with a generated ExportOptions.plist using automatic
//! signing — the fastlane-gym flow without the Ruby.

use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};

use crate::cli::output::Output;
use crate::cli::resolve::{self, Container};
use crate::cli::{
    CliError, CommandResult, Context, ErrorKind, Render, Rendered, buildlog, process, xcodebuild,
};

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
}

impl ExportMethod {
    fn plist_value(self) -> &'static str {
        match self {
            ExportMethod::Debugging => "debugging",
            ExportMethod::AppStoreConnect => "app-store-connect",
            ExportMethod::ReleaseTesting => "release-testing",
            ExportMethod::Enterprise => "enterprise",
        }
    }
}

#[derive(Debug, Args)]
pub struct ArchiveArgs {
    #[command(flatten)]
    pub target: crate::cli::BuildTargetArgs,

    /// Output directory for the .xcarchive and exported .ipa
    /// (default: ./build).
    #[arg(long, value_name = "DIR")]
    pub output: Option<PathBuf>,

    /// How to export the archive.
    #[arg(long, value_enum, default_value_t = ExportMethod::Debugging)]
    pub export_method: ExportMethod,

    /// Use an existing ExportOptions.plist instead of generating one.
    #[arg(
        long,
        value_name = "PLIST",
        conflicts_with = "export_method",
        conflicts_with = "no_export"
    )]
    pub export_options: Option<PathBuf>,

    /// Archive only; skip the IPA export.
    #[arg(long)]
    pub no_export: bool,

    /// Print the exact xcodebuild invocation(s) that would run, then exit.
    #[arg(long)]
    pub show_command: bool,

    /// Extra arguments passed to `xcodebuild archive` verbatim (after `--`),
    /// e.g. `-allowProvisioningUpdates`.
    #[arg(last = true, value_name = "XCODEBUILD_ARGS")]
    pub passthrough: Vec<String>,
}

/// The archive/export outcome.
struct ArchiveReport {
    archive: String,
    ipa_dir: Option<String>,
}

impl Render for ArchiveReport {
    fn human(&self, out: &Output) {
        out.note(&format!("archive: {}", self.archive));
        if let Some(dir) = &self.ipa_dir {
            out.note(&format!("exported: {dir}"));
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "archive": self.archive,
            "exportedTo": self.ipa_dir,
        })
    }
}

pub fn run(ctx: &mut Context, args: &ArchiveArgs) -> CommandResult {
    ctx.targeting = args.target.clone().into();
    let resolved = resolve::resolve(ctx)?;
    if matches!(resolved.container, Container::SwiftPackage(_)) {
        return Err(CliError::new(
            "a Swift package has no app to archive; archive works on Xcode projects/workspaces",
        ));
    }

    let schemes = resolve::schemes(&resolved.container)?;
    if let Some(s) = &resolved.scheme {
        resolve::validate_choice("scheme", s, &schemes)?;
    }
    let scheme = resolve::choose(ctx, "scheme", resolved.scheme.clone(), &schemes)?;
    let configuration = archive_configuration(ctx, &resolved)?;
    let destination = archive_destination(ctx)?;

    // Both xcodebuild steps run with the *container's* parent as cwd, while
    // sweetpad itself creates the output dir and writes the plist from the
    // CLI's cwd — absolutize so the two sides agree on where `build/` is.
    let out_dir = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from("build"));
    let out_dir = std::path::absolute(&out_dir).unwrap_or(out_dir);
    let archive_path = out_dir.join(format!("{scheme}.xcarchive"));

    let mut archive_args: Vec<String> = vec![
        "archive".into(),
        "-scheme".into(),
        scheme.clone(),
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
    archive_args.extend(args.passthrough.iter().cloned());

    let cwd = xcodebuild::working_dir(&resolved.container);
    let export_dir = out_dir.join("export");
    let plist_path = args
        .export_options
        .clone()
        .unwrap_or_else(|| out_dir.join("ExportOptions.plist"));
    let plist_path = std::path::absolute(&plist_path).unwrap_or(plist_path);
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
        let generated_plist = (!args.no_export && args.export_options.is_none())
            .then(|| plist_path.display().to_string());
        return Ok(Rendered::data(ArchivePreview {
            archive: archive_preview,
            export: export_preview,
            generated_plist,
            export_method: args.export_method,
        }));
    }

    let _ = std::fs::create_dir_all(&out_dir);
    run_xcodebuild(ctx, &archive_args, cwd.as_deref(), "Archiving")
        .map_err(|e| e.context("archiving the app"))?;

    if args.no_export {
        return Ok(Rendered::data(ArchiveReport {
            archive: archive_path.display().to_string(),
            ipa_dir: None,
        }));
    }

    // Export: an explicit plist wins; otherwise generate one with automatic
    // signing for the chosen method.
    if args.export_options.is_none() {
        std::fs::write(&plist_path, export_options_plist(args.export_method))
            .map_err(|e| CliError::new(format!("failed to write {}: {e}", plist_path.display())))?;
    }
    run_xcodebuild(ctx, &export_args, cwd.as_deref(), "Exporting")
        .map_err(|e| e.context("exporting the archive"))?;

    Ok(Rendered::data(ArchiveReport {
        archive: archive_path.display().to_string(),
        ipa_dir: Some(export_dir.display().to_string()),
    }))
}

/// The archive's configuration: flag > user config > sweetpad.toml >
/// `Release`. The *remembered* layer is deliberately skipped — a Debug pick
/// remembered from the daily build loop must not silently turn distribution
/// archives into Debug builds.
fn archive_configuration(
    ctx: &mut Context,
    resolved: &resolve::Resolved,
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
/// verbatim; `--on` takes a platform word (`mac`, `ios`, `watchos`, `tvos`,
/// `visionos`) or `device` and maps it to the matching `generic/platform=…`;
/// default iOS. Archives target device platforms, so a simulator reference
/// is rejected rather than resolved.
fn archive_destination(ctx: &Context) -> Result<String, CliError> {
    if let Some(dest) = &ctx.targeting.destination {
        return Ok(dest.clone());
    }
    let Some(on) = &ctx.targeting.on else {
        return Ok("generic/platform=iOS".to_string());
    };
    let platform = match on.to_ascii_lowercase().as_str() {
        "mac" | "macos" => "macOS",
        "ios" | "iphone" | "ipad" | "device" => "iOS",
        "watchos" => "watchOS",
        "tvos" => "tvOS",
        "visionos" | "xros" => "visionOS",
        other => {
            return Err(CliError::new(format!(
                "archive targets a generic device platform; --on {other:?} doesn't name one \
                 (use mac, ios, watchos, tvos, or visionos — or --destination for the raw form)"
            )));
        }
    };
    Ok(format!("generic/platform={platform}"))
}

/// The `--show-command` payload: both xcodebuild invocations, and the
/// ExportOptions.plist that would be generated.
struct ArchivePreview {
    archive: xcodebuild::CommandPreview,
    export: Option<xcodebuild::CommandPreview>,
    generated_plist: Option<String>,
    export_method: ExportMethod,
}

impl Render for ArchivePreview {
    fn human(&self, out: &Output) {
        self.archive.human(out);
        if let Some(export) = &self.export {
            export.human(out);
        }
        if let Some(plist) = &self.generated_plist {
            out.note(&format!(
                "{plist} will be generated (method {}, automatic signing)",
                self.export_method.plist_value()
            ));
        }
    }

    fn json(&self) -> serde_json::Value {
        let mut commands = vec![self.archive.json()];
        if let Some(export) = &self.export {
            commands.push(export.json());
        }
        serde_json::json!({
            "commands": commands,
            "generatedExportOptions": self.generated_plist,
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

    #[test]
    fn export_options_carry_method_and_automatic_signing() {
        let plist = export_options_plist(ExportMethod::AppStoreConnect);
        assert!(plist.contains("<string>app-store-connect</string>"));
        assert!(plist.contains("<string>automatic</string>"));
        assert!(plist.starts_with("<?xml"));
    }
}
