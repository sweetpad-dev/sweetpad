//! `sweetpad setup`: make a Linux machine build for Apple platforms. Each
//! step checks whether its result is already there and skips when it is,
//! so running setup again is cheap.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::Args;
use serde_json::{Value, json};

use super::sdk::{ImportArgs, Report, default_sdk, home};
use crate::cli::linux_setup::{self, host, swift};
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Rendered};

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)] // independent setup options
#[command(group(
    clap::ArgGroup::new("source")
        .required(true)
        .args(["sdk_archive", "xcode", "no_sdk"]),
))]
pub struct SetupArgs {
    /// Sweetpad SDK export with its checksum, or a private Xcode .xip/.aea archive.
    #[arg(long = "sdk")]
    pub sdk_archive: Option<PathBuf>,
    /// Download this Xcode release from Apple, then import its SDKs.
    #[arg(long)]
    pub xcode: Option<String>,
    /// Set up the host packages and Swift only, without a Darwin SDK.
    #[arg(long)]
    pub no_sdk: bool,
    #[command(flatten)]
    pub session: super::auth::SessionArgs,
    /// Install missing host packages with APT or pacman (uses sudo unless root).
    #[arg(long, conflicts_with = "skip_system_packages")]
    pub install_system_packages: bool,
    /// Continue setup even if some host packages are missing.
    #[arg(long)]
    pub skip_system_packages: bool,
    /// AEA symmetric key or Apple FCS private key file, for offline decryption.
    #[arg(long)]
    pub aea_key_file: Option<PathBuf>,
    /// AEA signing public key file, if the archive does not embed it.
    #[arg(long)]
    pub aea_public_key_file: Option<PathBuf>,
    /// Destination for the extracted SDK.
    #[arg(long)]
    pub sdk_path: Option<PathBuf>,
    /// Replace a different SDK after successful import.
    #[arg(long)]
    pub replace_sdk: bool,
    /// Shell environment file for running Swift by hand (default: ~/.config/sweetpad/linux-env.sh).
    #[arg(long)]
    pub env_file: Option<PathBuf>,
}

/// One setup step's outcome: what it found or did.
enum Outcome {
    Ready(String),
    Done(String),
}

/// Reports each step as it starts and ends: notes on the terminal, events
/// under `-o ndjson`, and a list in the final report.
struct Steps<'a> {
    ctx: &'a Context,
    done: Vec<Value>,
}

impl Steps<'_> {
    fn run(
        &mut self,
        id: &str,
        title: &str,
        step: impl FnOnce(&mut dyn FnMut(&str)) -> Result<Outcome, CliError>,
    ) -> Result<(), CliError> {
        let ctx = self.ctx;
        ctx.out
            .ndjson_event(&json!({"event": "setupStep", "step": id, "status": "started"}));
        let mut note = |line: &str| {
            ctx.out.note(line);
            ctx.out
                .ndjson_event(&json!({"event": "setupProgress", "step": id, "message": line}));
        };
        let outcome = step(&mut note).map_err(|error| {
            ctx.out.ndjson_event(
                &json!({"event": "setupStep", "step": id, "status": "failed", "message": error.to_string()}),
            );
            let diagnostics = vec![json!({"stage": id})];
            error.diagnostics(diagnostics)
        })?;
        let (status, detail) = match outcome {
            Outcome::Ready(detail) => ("ready", detail),
            Outcome::Done(detail) => ("done", detail),
        };
        ctx.out.note(&format!("{title}: {detail}"));
        let record = json!({"step": id, "status": status, "detail": detail});
        ctx.out.ndjson_event(
            &json!({"event": "setupStep", "step": id, "status": status, "detail": detail}),
        );
        self.done.push(record);
        Ok(())
    }
}

#[allow(clippy::too_many_lines)] // the steps, in order
pub fn run(ctx: &mut Context, args: &SetupArgs) -> CommandResult {
    if !cfg!(target_os = "linux") || !cfg!(any(target_arch = "aarch64", target_arch = "x86_64")) {
        return Err(
            CliError::new("sweetpad setup runs on ARM64 or x86_64 Linux").kind(ErrorKind::Usage),
        );
    }
    let pins = super::sdk::toolchain();
    let host =
        host::current(&pins.host_packages).map_err(|e| CliError::new(e).kind(ErrorKind::Usage))?;
    for warning in &host.warnings {
        ctx.out.warn(warning);
    }
    // The Xcode download and the archive check fail before anything is
    // installed.
    let downloaded = args
        .xcode
        .as_ref()
        .map(|version| {
            super::sdk::download(
                ctx,
                &super::sdk::DownloadArgs {
                    xcode: version.clone(),
                    session: super::auth::SessionArgs {
                        session_file: args.session.session_file.clone(),
                    },
                    output_file: None,
                },
            )
        })
        .transpose()?;
    let archive = args
        .sdk_archive
        .as_deref()
        .or_else(|| {
            downloaded
                .as_ref()
                .and_then(|data| data["archive"].as_str())
                .map(Path::new)
        })
        .map(|archive| {
            archive
                .canonicalize()
                .map_err(|e| CliError::new(format!("Couldn't read {}: {e}", archive.display())))
        })
        .transpose()?;
    if let Some(archive) = &archive {
        crate::cli::sdk_import::check(archive, &pins.sdk_toolchain()).map_err(CliError::new)?;
    }
    let destination = std::path::absolute(args.sdk_path.clone().unwrap_or_else(default_sdk))
        .map_err(|e| CliError::new(format!("resolve SDK destination: {e}")))?;
    let interactive = ctx.out.is_interactive();
    let mut steps = Steps {
        ctx,
        done: Vec::new(),
    };

    steps.run("packages", "Host packages", |_| {
        let Some(manager) = host.manager else {
            return Ok(Outcome::Ready(format!(
                "using what is installed on {}",
                host.name
            )));
        };
        let missing = host::missing_packages(&host).map_err(CliError::new)?;
        if missing.is_empty() {
            return Ok(Outcome::Ready(format!(
                "all {} installed",
                host.packages.len()
            )));
        }
        if !args.install_system_packages {
            let detail = host::missing_detail(manager, &missing);
            if args.skip_system_packages {
                return Ok(Outcome::Ready(format!("Skipped. {detail}")));
            }
            return Err(CliError::new(detail).kind(ErrorKind::ToolMissing));
        }
        host::install(&host, &missing, interactive).map_err(CliError::new)?;
        Ok(Outcome::Done(format!(
            "installed {} with {}",
            missing.join(" "),
            manager.name()
        )))
    })?;

    steps.run("swift", &format!("Swift {}", pins.swift), |note| {
        if let Some(found) = swift::find(&pins.swift) {
            swift::probe(&found, &host).map_err(CliError::new)?;
            return Ok(Outcome::Ready(found.bin.display().to_string()));
        }
        let (swift, installed) = swift::install(&host, &pins.swift, note)
            .map_err(|e| CliError::new(e).kind(ErrorKind::ToolMissing))?;
        let path = swift.bin.display().to_string();
        Ok(if installed {
            Outcome::Done(path)
        } else {
            Outcome::Ready(path)
        })
    })?;

    steps.run("archiver", "Darwin archiver", |_| {
        linux_setup::archiver()
            .map(|path| Outcome::Ready(path.display().to_string()))
            .ok_or_else(|| {
                CliError::new("The pinned Swift toolchain is missing llvm-ar")
                    .kind(ErrorKind::ToolMissing)
            })
    })?;

    let mut imported = Value::Null;
    if let Some(archive) = &archive {
        steps.run("sdk", "Darwin SDK", |note| {
            let report = super::sdk::import(
                &ImportArgs {
                    archive: Some(archive.clone()),
                    xcode: None,
                    appintents_phrases: None,
                    build_machine_os_build: None,
                    install_path: Some(destination.clone()),
                    replace: args.replace_sdk,
                    aea_key_file: args.aea_key_file.clone(),
                    aea_public_key_file: args.aea_public_key_file.clone(),
                },
                note,
            )
            .map_err(CliError::new)?;
            let detail = destination.display().to_string();
            let reused = report["reused"] == true;
            imported = report;
            Ok(if reused {
                Outcome::Ready(detail)
            } else {
                Outcome::Done(detail)
            })
        })?;
    }

    let env_file = std::path::absolute(
        args.env_file
            .clone()
            .unwrap_or_else(|| home().join(".config/sweetpad/linux-env.sh")),
    )
    .map_err(|e| CliError::new(format!("resolve environment file: {e}")))?;
    let swift_bin = swift::find(&pins.swift).map(|found| found.bin);
    steps.run("environment", "Environment file", |_| {
        std::fs::create_dir_all(env_file.parent().unwrap_or(Path::new("/")))
            .and_then(|()| {
                std::fs::write(
                    &env_file,
                    environment_contents(
                        archive.as_ref().map(|_| destination.as_path()),
                        swift_bin.as_deref(),
                    ),
                )
            })
            .map_err(|e| CliError::new(format!("Couldn't write {}: {e}", env_file.display())))?;
        Ok(Outcome::Done(env_file.display().to_string()))
    })?;

    let mut sample = Value::Null;
    if archive.is_some() {
        steps.run("sample", "Sample build", |_| {
            sample = validate_sample(&destination)?;
            Ok(Outcome::Done("an iOS app built".into()))
        })?;
    }
    let done = steps.done;
    Ok(Rendered::data(Report(json!({
        "configured": true,
        "host": host.name,
        "steps": done,
        "sdkPath": archive.as_ref().map(|_| &destination),
        "sdk": imported,
        "environmentFile": env_file,
        "sampleBuild": sample,
        "download": downloaded,
        "nextStep": if archive.is_some() {
            "Run sweetpad doctor --target ios, then build your project with sweetpad build"
        } else {
            "Import a Darwin SDK with sweetpad setup --sdk ARCHIVE to build for Apple platforms"
        },
    }))))
}

/// `linux-env.sh`, for running `swift` by hand: sweetpad finds the SDK and
/// Swift on its own. Swift's folder holds no `sweetpad`, so prepending it
/// keeps the caller's choice of CLI.
fn environment_contents(sdk: Option<&Path>, swift_bin: Option<&Path>) -> String {
    let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
    let mut text = String::from("# Generated by sweetpad setup.\n");
    if let Some(sdk) = sdk {
        let _ = writeln!(text, "export SWEETPAD_DARWIN_SDK={}", quote(sdk));
    }
    if let Some(bin) = swift_bin {
        let _ = writeln!(text, "export PATH={}\":$PATH\"", quote(bin));
    }
    text
}

/// Build the bundled sample app for iOS, the check that the toolchain, SDK
/// and Swift Build work together.
fn validate_sample(sdk: &Path) -> Result<Value, CliError> {
    let project = crate::cli::embedded::root()?.join("examples/LinuxApp/LinuxApp.xcodeproj");
    let derived =
        sweetpad_lib::cache::prepare(&sweetpad_lib::cache::root(), Path::new("setup-validation"))
            .map_err(|e| CliError::new(format!("prepare setup build directory: {e}")))?;
    let result = Command::new(std::env::current_exe().map_err(|e| CliError::new(e.to_string()))?)
        .args(["build", "--project"])
        .arg(project)
        .args([
            "--scheme",
            "LinuxApp",
            "--destination",
            "generic/platform=iOS",
            "--json",
            "--non-interactive",
            "--",
            "-derivedDataPath",
        ])
        .arg(derived)
        .args(["ARCHS=arm64", "CODE_SIGNING_ALLOWED=NO"])
        .env("SWEETPAD_DARWIN_SDK", sdk)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| CliError::new(format!("start setup sample build: {e}")))?;
    if !result.status.success() {
        return Err(sample_build_error(&result.stdout, &result.stderr));
    }
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)
        .map_err(|e| CliError::new(format!("read sample build report: {e}")))?;
    if report["data"]["built"] != true {
        return Err(CliError::new(
            "Setup sample did not produce a successful build",
        ));
    }
    Ok(report["data"].clone())
}

fn sample_build_error(stdout: &[u8], stderr: &[u8]) -> CliError {
    // The sample build runs this same CLI and writes its envelope to stdout.
    let report = serde_json::from_slice::<serde_json::Value>(stdout).unwrap_or_else(|_| {
        json!({
            "stdout": String::from_utf8_lossy(&stdout[stdout.len().saturating_sub(16_384)..]),
            "stderr": String::from_utf8_lossy(&stderr[stderr.len().saturating_sub(16_384)..])
        })
    });
    let reason = report["error"]["message"].as_str().unwrap_or(
        "the child build returned no structured error; run sweetpad doctor --target ios",
    );
    let tip = report["error"]["tip"].as_str().map(str::to_owned);
    CliError::new(format!("Setup sample build failed: {reason}"))
        .kind(ErrorKind::BuildFailure)
        .tip(tip)
        .diagnostics(vec![json!({"stage": "sampleBuild", "report": report})])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    #[test]
    fn failed_sample_retains_the_child_error_and_log_hint() {
        let report = json!({"ok": false, "error": {
            "message": "swbuild exited; full log: /tmp/sample-build.log",
            "tip": "read full build log: /tmp/sample-build.log",
            "diagnostics": [{"message": "bwrap: Creating new namespace failed"}]
        }});
        let error = sample_build_error(
            &serde_json::to_vec(&report).unwrap(),
            b"warning: build diagnostic",
        );
        assert!(error.to_string().contains("/tmp/sample-build.log"));
        assert_eq!(error.error_kind(), ErrorKind::BuildFailure);
        assert_eq!(
            error.tip_text(),
            Some("read full build log: /tmp/sample-build.log")
        );
        assert_eq!(error.json()["diagnostics"][0]["report"], report);
        assert!(sample_build_error(b"", b"non-JSON child failure").json()["diagnostics"][0]["report"]["stderr"]
            .as_str().unwrap().contains("non-JSON child failure"));
    }

    #[test]
    fn environment_puts_swift_first_and_quotes_sdk_paths() {
        let text = environment_contents(
            Some(Path::new("/private/SDK's directory")),
            Some(Path::new("/opt/swift/usr/bin")),
        );
        assert!(text.contains("SDK'\\''s directory"));
        assert!(text.contains("export PATH='/opt/swift/usr/bin'\":$PATH\""));
        assert!(!text.contains("$HOME/bin"));
        assert_eq!(
            environment_contents(None, None),
            "# Generated by sweetpad setup.\n"
        );
    }

    #[test]
    fn generated_environment_keeps_the_ci_selected_cli() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("setup-ci-path");
        for name in ["ci/bin/sweetpad", "bin/sweetpad", ".local/bin/sweetpad"] {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env_file = dir.join("environment.sh");
        let sdk = dir.join("SDK's directory");
        std::fs::write(
            &env_file,
            environment_contents(Some(&sdk), Some(&dir.join("swift/usr/bin"))),
        )
        .unwrap();
        let output = Command::new("bash")
            .args([
                "-c",
                "source \"$1\"; command -v sweetpad; printf '%s\\n' \"$SWEETPAD_DARWIN_SDK\"",
                "setup-test",
            ])
            .arg(env_file)
            .env("HOME", dir.join(""))
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", dir.join("ci/bin").display()),
            )
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            vec![
                dir.join("ci/bin/sweetpad").to_str().unwrap(),
                sdk.to_str().unwrap()
            ]
        );
    }
}
