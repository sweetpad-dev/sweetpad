//! Export private Apple inputs and import them with the pinned Linux tools.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::{Args, Subcommand};
use serde::Deserialize;
use serde_json::Value;

use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered, output::Output};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Export private SDK inputs from your Xcode. Contains no signing keys.
    Export(ExportArgs),
    /// Verify and import an export on ARM64 or x86_64 Linux. Requires pinned xtool.
    Import(ImportArgs),
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    #[arg(long, default_value = "/Applications/Xcode.app")]
    pub xcode: PathBuf,
    /// New private .tar.gz archive. A .sha256 file is written beside it.
    #[arg(long)]
    pub output_file: PathBuf,
    /// Report input size and required free space without writing an archive.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    /// Export archive. Its .sha256 file must be beside it.
    pub archive: PathBuf,
    /// SDK destination (default: ~/.swiftpm/swift-sdks/darwin.artifactbundle).
    #[arg(long = "destination")]
    pub install_path: Option<PathBuf>,
    /// Replace an existing SDK only after a successful import.
    #[arg(long)]
    pub replace: bool,
}

pub(super) struct Report(pub Value);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Toolchain {
    pub swift: String,
    pub swift_build: String,
    pub xtool: String,
    pub xtool_sha256: std::collections::BTreeMap<String, String>,
}

pub(super) fn toolchain() -> &'static Toolchain {
    static PINS: std::sync::LazyLock<Toolchain> = std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!(
            "../../../../sweetpad-lib/ci/linux-toolchain.json"
        ))
        .expect("the bundled Linux toolchain requirements must be valid")
    });
    &PINS
}

impl Render for Report {
    fn human(&self, out: &Output) {
        let data = &self.0;
        out.line(if data["dryRun"] == true {
            "SDK export plan"
        } else if data["configured"] == true {
            "Linux setup complete"
        } else if data["reused"] == true {
            "SDK already imported"
        } else if data.get("archive").is_some() {
            "SDK export complete"
        } else {
            "SDK import complete"
        });
        for (key, label) in [
            ("archive", "Archive"),
            ("sdkPath", "SDK"),
            ("environmentFile", "Environment file"),
            ("xcodeVersion", "Xcode version"),
            ("xcodeBuild", "Xcode build"),
            ("sha256", "SHA-256"),
        ] {
            if let Some(value) = data[key].as_str() {
                out.line(&format!("{label}: {value}"));
            }
        }
        for (key, label) in [
            ("unpackedBytes", "Input size"),
            ("requiredFreeBytes", "Required free space"),
            ("archiveBytes", "Archive size"),
        ] {
            if let Some(value) = data[key].as_u64() {
                out.line(&format!("{label}: {value} bytes"));
            }
        }
        if data["sampleBuild"]["built"] == true {
            out.line("Sample build passed");
        }
        if let Some(next) = data["nextStep"].as_str() {
            out.note(next);
        }
    }
    fn json(&self) -> Value {
        self.0.clone()
    }
}

pub fn run(_ctx: &mut Context, action: &Action) -> CommandResult {
    let helpers = crate::cli::embedded::root()?.join("setup");
    let mut cmd = Command::new("python3");
    cmd.arg(helpers.join("sweetpad-lib/ci/linux-sdk.py"));
    match action {
        Action::Export(args) => {
            cmd.arg("export")
                .arg("--xcode")
                .arg(&args.xcode)
                .arg("--output")
                .arg(&args.output_file);
            if args.dry_run {
                cmd.arg("--dry-run");
            }
        }
        Action::Import(args) => {
            cmd.arg("import").arg("--archive").arg(&args.archive);
            cmd.arg("--destination")
                .arg(args.install_path.clone().unwrap_or_else(default_sdk));
            if args.replace {
                cmd.arg("--replace");
            }
        }
    }
    let output = cmd
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| {
            CliError::new(format!("SDK preparation requires python3: {e}"))
                .kind(ErrorKind::ToolMissing)
        })?;
    let report: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| CliError::new(format!("SDK helper returned an invalid report: {e}")))?;
    if !output.status.success() {
        return Err(CliError::new(
            report["error"].as_str().unwrap_or("SDK preparation failed"),
        ));
    }
    Ok(Rendered::data(Report(report)))
}

pub(super) fn default_sdk() -> PathBuf {
    std::env::var_os("SWEETPAD_DARWIN_SDK").map_or_else(
        || home().join(".swiftpm/swift-sdks/darwin.artifactbundle"),
        PathBuf::from,
    )
}

pub(super) fn home() -> PathBuf {
    Path::new(&std::env::var_os("HOME").unwrap_or_default()).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Resource};
    use clap::Parser;

    #[test]
    fn archive_path_does_not_shadow_global_output_format() {
        let cli = Cli::try_parse_from([
            "sweetpad",
            "sdk",
            "export",
            "--output-file",
            "/tmp/sdk.tar.gz",
            "-o",
            "json",
            "--non-interactive",
        ])
        .unwrap();
        assert!(cli.global.output.is_some());
        let Some(Resource::Sdk {
            action: Action::Export(args),
        }) = cli.resource
        else {
            panic!("expected SDK export");
        };
        assert_eq!(args.output_file, PathBuf::from("/tmp/sdk.tar.gz"));
    }

    #[test]
    fn setup_and_import_expose_explicit_sdk_replacement() {
        let cli = Cli::try_parse_from([
            "sweetpad",
            "sdk",
            "import",
            "/tmp/sdk.tar.gz",
            "--replace",
            "--json",
        ])
        .unwrap();
        let Some(Resource::Sdk {
            action: Action::Import(args),
        }) = cli.resource
        else {
            panic!("expected SDK import");
        };
        assert!(args.replace);
        assert!(
            Cli::try_parse_from([
                "sweetpad",
                "setup",
                "--sdk",
                "/tmp/sdk.tar.gz",
                "--sdk-path",
                "/tmp/SDK",
                "--replace-sdk",
                "--json"
            ])
            .is_ok()
        );
        assert!(Cli::try_parse_from(["sweetpad", "doctor", "--target", "ios", "--json"]).is_ok());
    }
}
