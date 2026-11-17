//! Export private Apple inputs and import them with the pinned Linux tools.
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde::Deserialize;
use serde_json::Value;

use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered, output::Output};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Download a private Xcode archive from Apple. Reuse or resume cached downloads.
    Download(DownloadArgs),
    /// Export private SDK inputs from your Xcode. Contains no signing keys.
    Export(ExportArgs),
    /// Import a private SDK export or Xcode archive on ARM64 or x86_64 Linux.
    Import(ImportArgs),
    /// Write a Darwin SDK bundle from an unpacked Xcode.app; 'sdk import' runs this.
    #[command(hide = true)]
    WriteBundle(WriteBundleArgs),
}

#[derive(Debug, Args)]
pub struct WriteBundleArgs {
    /// The unpacked Xcode.app.
    #[arg(long)]
    pub xcode: PathBuf,
    /// The bundle folder to create; it must not exist.
    #[arg(long)]
    pub out: PathBuf,
    /// App Intents phrase resources from 'sdk export'.
    #[arg(long)]
    pub appintents_phrases: Option<PathBuf>,
    /// The macOS build the SDK came from.
    #[arg(long)]
    pub build_machine_os_build: Option<String>,
    /// Clang resource headers to use instead of the host Swift's.
    #[arg(long)]
    pub clang_headers: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct DownloadArgs {
    /// Exact release or prerelease version, such as 27.2 or '27.3 beta'.
    #[arg(long)]
    pub xcode: String,
    #[command(flatten)]
    pub session: super::auth::SessionArgs,
    /// Archive destination (default: ~/.cache/sweetpad/downloads/<Apple filename>).
    #[arg(long)]
    pub output_file: Option<PathBuf>,
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
    /// Sweetpad export with its checksum, or a private Xcode .xip/.aea archive.
    #[arg(required_unless_present = "xcode", conflicts_with = "xcode")]
    pub archive: Option<PathBuf>,
    /// An unpacked Xcode.app this host can read, instead of an archive.
    #[arg(long, hide = true)]
    pub xcode: Option<PathBuf>,
    /// With --xcode: the App Intents phrase resources exported from it.
    #[arg(long, hide = true, requires = "xcode")]
    pub appintents_phrases: Option<PathBuf>,
    /// With --xcode: the macOS build to record as the build machine.
    #[arg(long, hide = true, requires = "xcode")]
    pub build_machine_os_build: Option<String>,
    /// SDK destination (default: ~/.swiftpm/swift-sdks/darwin.artifactbundle).
    #[arg(long = "destination")]
    pub install_path: Option<PathBuf>,
    /// Replace an existing SDK only after a successful import.
    #[arg(long)]
    pub replace: bool,
    /// AEA symmetric key or Apple FCS private key file, for offline decryption.
    #[arg(long)]
    pub aea_key_file: Option<PathBuf>,
    /// AEA signing public key file, if the archive does not embed it.
    #[arg(long)]
    pub aea_public_key_file: Option<PathBuf>,
}

pub(super) struct Report(pub Value);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Toolchain {
    pub swift: String,
    pub swift_build: String,
    pub host_packages: crate::cli::linux_setup::host::HostPackages,
}

pub(crate) fn toolchain() -> &'static Toolchain {
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
        out.line(if data["authenticated"] == true {
            "Apple session ready"
        } else if data["loggedOut"] == true {
            "Apple session removed"
        } else if data["downloaded"] == true {
            if data["reused"] == true {
                "Xcode archive already downloaded"
            } else {
                "Xcode download complete"
            }
        } else if data["dryRun"] == true {
            "SDK export plan"
        } else if data["configured"] == true {
            "Linux setup complete"
        } else if data.get("bundlePath").is_some() {
            "SDK bundle written"
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
            ("sessionFile", "Session file"),
            ("appleId", "Apple ID"),
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

pub(super) fn download(ctx: &Context, args: &DownloadArgs) -> Result<Value, CliError> {
    let explicit = args.session.session_file.is_some();
    let path = crate::cli::apple_account::session_path(args.session.session_file.as_deref());
    let mut note = |line: &str| ctx.out.note(line);
    crate::cli::apple_account::download_xcode(
        path,
        explicit,
        &args.xcode,
        args.output_file.as_deref(),
        &mut note,
    )
    .map_err(CliError::new)
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    if let Action::Download(args) = action {
        return Ok(Rendered::data(Report(download(ctx, args)?)));
    }
    if let Action::WriteBundle(args) = action {
        return write_bundle(args).map(|report| Rendered::data(Report(report)));
    }
    let pins = toolchain();
    let report = match action {
        Action::Download(_) | Action::WriteBundle(_) => unreachable!("handled above"),
        Action::Export(args) => crate::cli::sdk_import::export(
            &args.xcode,
            &args.output_file,
            args.dry_run,
            &pins.sdk_toolchain(),
            &mut |message| ctx.out.note(message),
        ),
        Action::Import(args) => import(args, &mut |message| ctx.out.note(message)),
    }
    .map_err(CliError::new)?;
    Ok(Rendered::data(Report(report)))
}

impl Toolchain {
    pub(crate) fn sdk_toolchain(&self) -> crate::cli::sdk_import::Toolchain<'_> {
        crate::cli::sdk_import::Toolchain {
            swift: &self.swift,
            swift_build: &self.swift_build,
        }
    }
}

pub(super) fn import(args: &ImportArgs, progress: &mut dyn FnMut(&str)) -> Result<Value, String> {
    let pins = toolchain();
    let keys = crate::cli::apple_archive::Keys::read(
        args.aea_key_file.as_deref(),
        args.aea_public_key_file.as_deref(),
    )?;
    let source = match (&args.archive, &args.xcode) {
        (_, Some(xcode)) => crate::cli::sdk_import::Source::Xcode(xcode),
        (Some(archive), None) => crate::cli::sdk_import::Source::Archive(archive),
        (None, None) => return Err("Supply an SDK archive or --xcode".into()),
    };
    let clang_headers = crate::cli::sdk_bundle::host_clang_headers(&pins.swift)?;
    let destination = args.install_path.clone().unwrap_or_else(default_sdk);
    crate::cli::sdk_import::import(
        &crate::cli::sdk_import::Import {
            source,
            destination: &destination,
            replace: args.replace,
            keys: &keys,
            phrases: args.appintents_phrases.as_deref(),
            build_machine_os_build: args.build_machine_os_build.as_deref(),
            toolchain: pins.sdk_toolchain(),
            clang_headers: &clang_headers,
        },
        progress,
    )
}

fn write_bundle(args: &WriteBundleArgs) -> Result<Value, CliError> {
    let clang_headers = match &args.clang_headers {
        Some(path) => path.clone(),
        None => crate::cli::sdk_bundle::host_clang_headers(&toolchain().swift)
            .map_err(|e| CliError::new(e).kind(ErrorKind::ToolMissing))?,
    };
    let inputs = crate::cli::sdk_bundle::Inputs {
        xcode: &args.xcode,
        clang_headers: &clang_headers,
        phrases: args.appintents_phrases.as_deref(),
        build_machine_os_build: args.build_machine_os_build.as_deref(),
    };
    crate::cli::sdk_bundle::write(&inputs, &args.out).map_err(CliError::new)
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

    #[test]
    fn download_and_setup_accept_versions_and_ci_sessions() {
        for argv in [
            vec![
                "sweetpad",
                "sdk",
                "download",
                "--xcode",
                "27.3 beta",
                "--session-file",
                "/tmp/session.json",
                "--output-file",
                "/tmp/Xcode.aea",
                "--json",
                "--non-interactive",
            ],
            vec![
                "sweetpad",
                "setup",
                "--xcode",
                "27.3 beta",
                "--session-file",
                "/tmp/session.json",
                "--replace-sdk",
                "--json",
                "--non-interactive",
            ],
        ] {
            assert!(Cli::try_parse_from(&argv).is_ok(), "{argv:?}");
        }
        assert!(Cli::try_parse_from(["sweetpad", "setup"]).is_err());
        assert!(
            Cli::try_parse_from([
                "sweetpad",
                "setup",
                "--sdk",
                "/tmp/sdk.tar.gz",
                "--xcode",
                "27.2"
            ])
            .is_err()
        );
    }
}
