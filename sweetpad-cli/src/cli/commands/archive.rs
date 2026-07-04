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
    #[arg(long, value_name = "PLIST", conflicts_with = "export_method")]
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
    // Archives distribute: Release unless something explicitly says otherwise.
    let configuration = resolved
        .configuration
        .clone()
        .unwrap_or_else(|| "Release".to_string());

    let out_dir = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from("build"));
    let archive_path = out_dir.join(format!("{scheme}.xcarchive"));

    let mut archive_args: Vec<String> = vec![
        "archive".into(),
        "-scheme".into(),
        scheme.clone(),
        "-configuration".into(),
        configuration,
        "-destination".into(),
        "generic/platform=iOS".into(),
        "-archivePath".into(),
        archive_path.display().to_string(),
    ];
    archive_args.extend(xcodebuild::container_args(&resolved.container));
    archive_args.extend(args.passthrough.iter().cloned());

    if args.show_command {
        return Ok(Rendered::data(xcodebuild::CommandPreview {
            program: "xcodebuild",
            args: archive_args,
            cwd: xcodebuild::working_dir(&resolved.container),
        }));
    }

    let _ = std::fs::create_dir_all(&out_dir);
    let cwd = xcodebuild::working_dir(&resolved.container);
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
    let export_dir = out_dir.join("export");
    let plist = if let Some(p) = &args.export_options { p.clone() } else {
        let path = out_dir.join("ExportOptions.plist");
        std::fs::write(&path, export_options_plist(args.export_method)).map_err(|e| {
            CliError::new(format!("failed to write {}: {e}", path.display()))
        })?;
        path
    };
    let export_args: Vec<String> = vec![
        "-exportArchive".into(),
        "-archivePath".into(),
        archive_path.display().to_string(),
        "-exportPath".into(),
        export_dir.display().to_string(),
        "-exportOptionsPlist".into(),
        plist.display().to_string(),
    ];
    run_xcodebuild(ctx, &export_args, cwd.as_deref(), "Exporting")
        .map_err(|e| e.context("exporting the archive"))?;

    Ok(Rendered::data(ArchiveReport {
        archive: archive_path.display().to_string(),
        ipa_dir: Some(export_dir.display().to_string()),
    }))
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
        Err(CliError::new("xcodebuild exited with a non-zero status")
            .kind(ErrorKind::BuildFailure))
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
