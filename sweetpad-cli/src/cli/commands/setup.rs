//! Linux bootstrap over the public setup sources embedded in the CLI.
use std::path::PathBuf;
use std::process::{Command, Stdio};

use clap::Args;
use serde_json::json;

use super::sdk::{Report, default_sdk, home};
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Rendered};

#[derive(Debug, Args)]
pub struct SetupArgs {
    /// Sweetpad SDK export with its checksum, or a private Xcode .xip/.aea archive.
    #[arg(
        long = "sdk",
        required_unless_present = "xcode",
        conflicts_with = "xcode"
    )]
    pub sdk_archive: Option<PathBuf>,
    /// Download this Xcode release from Apple, then import its SDKs.
    #[arg(long, required_unless_present = "sdk_archive")]
    pub xcode: Option<String>,
    #[command(flatten)]
    pub session: super::auth::SessionArgs,
    /// Use existing host packages instead of installing them with APT or pacman.
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
    /// Override the embedded public setup sources with this source root.
    #[arg(long)]
    pub setup_dir: Option<PathBuf>,
    /// Generated shell environment file (default: ~/.config/sweetpad/linux-env.sh).
    #[arg(long)]
    pub env_file: Option<PathBuf>,
}

pub fn run(ctx: &mut Context, args: &SetupArgs) -> CommandResult {
    if !cfg!(target_os = "linux") || !cfg!(any(target_arch = "aarch64", target_arch = "x86_64")) {
        return Err(
            CliError::new("sweetpad setup requires ARM64 or x86_64 Linux").kind(ErrorKind::Usage),
        );
    }
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
                .map(std::path::Path::new)
        })
        .ok_or_else(|| {
            CliError::new("Supply --sdk ARCHIVE or --xcode VERSION").kind(ErrorKind::Usage)
        })?
        .canonicalize()
        .map_err(|e| CliError::new(format!("read SDK archive: {e}")))?;
    let root = setup_root(args.setup_dir.as_deref())?;
    let script = root.join("sweetpad-lib/ci/linux-setup.sh");
    if !script.is_file() {
        return Err(
            CliError::new("Setup sources are missing from the selected source root")
                .kind(ErrorKind::ToolMissing),
        );
    }
    let destination = std::path::absolute(args.sdk_path.clone().unwrap_or_else(default_sdk))
        .map_err(|e| CliError::new(format!("resolve SDK destination: {e}")))?;
    let pins = super::sdk::toolchain();
    // Python redirects every bootstrap progress line to stderr, so stdout stays JSON-safe.
    let mut bootstrap = Command::new("python3");
    bootstrap.args(["-c", "import subprocess,sys; sys.exit(subprocess.call(['bash',sys.argv[1]],stdout=sys.stderr))"])
        .arg(script).env("SWEETPAD_SKIP_RUST", "1")
        .env("SWEETPAD_SKIP_SYSTEM_PACKAGES", if args.skip_system_packages { "1" } else { "0" })
        .env("SWIFT_VERSION", &pins.swift).env("SWIFT_BUILD_VERSION", &pins.swift_build)
        .env("XTOOL_VERSION", &pins.xtool)
        .env("XTOOL_SHA256", &pins.xtool_sha256[std::env::consts::ARCH])
        .env("SWEETPAD_NONINTERACTIVE", if ctx.out.is_interactive() { "0" } else { "1" })
        .env("SWEETPAD_SDK_ARCHIVE", archive).env("SWEETPAD_DARWIN_SDK", &destination)
        .env("SWEETPAD_REFRESH_SDK", if args.replace_sdk { "1" } else { "0" })
        .stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
    if let Some(path) = &args.aea_key_file {
        bootstrap.env("SWEETPAD_AEA_KEY_FILE", path);
    }
    if let Some(path) = &args.aea_public_key_file {
        bootstrap.env("SWEETPAD_AEA_PUBLIC_KEY_FILE", path);
    }
    run_bootstrap(&mut bootstrap, ctx.out.is_json() || ctx.out.is_ndjson())?;
    let env_file = std::path::absolute(
        args.env_file
            .clone()
            .unwrap_or_else(|| home().join(".config/sweetpad/linux-env.sh")),
    )
    .map_err(|e| CliError::new(format!("resolve environment file: {e}")))?;
    std::fs::create_dir_all(env_file.parent().unwrap())
        .map_err(|e| CliError::new(e.to_string()))?;
    std::fs::write(&env_file, environment_contents(&destination))
        .map_err(|e| CliError::new(e.to_string()))?;
    let sample = validate_sample(&root, &destination)?;
    Ok(Rendered::data(Report(
        json!({"configured": true, "sdkPath": destination,
        "environmentFile": env_file, "sampleBuild": sample, "setupSourcePath": root,
        "download": downloaded,
        "nextStep": "Source the environment file, then run sweetpad doctor --target ios"}),
    )))
}

fn run_bootstrap(command: &mut Command, machine: bool) -> Result<(), CliError> {
    let missing = |error| {
        CliError::new(format!(
            "start Linux setup: {error}; install python3 before running setup"
        ))
        .kind(ErrorKind::ToolMissing)
    };
    if machine {
        let output = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(missing)?;
        if !output.status.success() {
            return Err(bootstrap_error(&String::from_utf8_lossy(&output.stderr)));
        }
    } else if !command.status().map_err(missing)?.success() {
        return Err(CliError::new(
            "Linux setup failed. Review the diagnostics and rerun the same command.",
        ));
    }
    Ok(())
}

fn bootstrap_error(stderr: &str) -> CliError {
    let tail = String::from_utf8_lossy(&stderr.as_bytes()[stderr.len().saturating_sub(16_384)..])
        .into_owned();
    let reason = tail.lines().rev().find_map(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|value| value["error"].as_str().map(str::to_string))
    });
    CliError::new(reason.map_or_else(
        || "Linux setup failed. Review the bootstrap diagnostics.".into(),
        |reason| format!("Linux setup failed: {reason}"),
    ))
    .diagnostics(vec![json!({"stage":"bootstrap","message":tail})])
}

fn setup_root(explicit: Option<&std::path::Path>) -> Result<PathBuf, CliError> {
    let valid = |p: &std::path::Path| {
        p.join("sweetpad-lib/ci/linux-setup.sh").is_file()
            && p.join("sweetpad-lib/ci/linux-sdk.py").is_file()
            && p.join("sweetpad-lib/ci/linux-toolchain.json").is_file()
            && p.join("sweetpad-cli/macros/Package.swift").is_file()
    };
    if let Some(path) = explicit {
        return valid(path).then(|| path.to_path_buf()).ok_or_else(|| {
            CliError::new("The --setup-dir source root is missing public setup files.")
                .kind(ErrorKind::ToolMissing)
        });
    }
    Ok(crate::cli::embedded::root()?.join("setup"))
}

fn environment_contents(sdk: &std::path::Path) -> String {
    let quoted = sdk.to_string_lossy().replace('\'', "'\\''");
    let swift = &super::sdk::toolchain().swift;
    // Swift's directory contains no CLI. Preserve the caller's selected CLI
    // before adding the helper directory, and select the pinned helpers explicitly.
    format!(
        "# Generated by sweetpad setup.\nexport SWEETPAD_DARWIN_SDK='{quoted}'\nexport SWEETPAD_SWBUILD=\"$HOME/bin/swbuild\"\nexport SWEETPAD_MACROS=\"$HOME/bin/sweetpad-macros\"\nexport PATH=\"$HOME/swift/swift-{swift}-RELEASE/usr/bin:$PATH:$HOME/bin\"\n"
    )
}

fn validate_sample(
    root: &std::path::Path,
    sdk: &std::path::Path,
) -> Result<serde_json::Value, CliError> {
    let source_example = root.join("sweetpad-lib/ci/fixture-linux");
    let packaged_example = root.parent().unwrap_or(root).join("examples/LinuxApp");
    let project = if source_example.is_dir() {
        source_example
    } else {
        packaged_example
    }
    .join("LinuxApp.xcodeproj");
    let derived = home().join(".cache/sweetpad/setup-validation");
    let path = format!(
        "{}/bin:{}/swift/swift-{swift}-RELEASE/usr/bin:{}",
        home().display(),
        home().display(),
        std::env::var("PATH").unwrap_or_default(),
        swift = super::sdk::toolchain().swift
    );
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
        .env("PATH", path)
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
    // CLI error envelopes go to stderr; successful reports go to stdout.
    let report = serde_json::from_slice::<serde_json::Value>(stdout)
        .or_else(|_| serde_json::from_slice(stderr))
        .unwrap_or_else(|_| {
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
        let error = sample_build_error(b"", &serde_json::to_vec(&report).unwrap());
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
    fn machine_setup_errors_retain_the_migration_flag_and_stage() {
        let error = bootstrap_error(
            "setup progress\n{\"error\":\"SDK already exists; use --replace-sdk\"}\n",
        );
        let value = error.json();
        assert!(value["message"].as_str().unwrap().contains("--replace-sdk"));
        assert_eq!(value["diagnostics"][0]["stage"], "bootstrap");
    }

    fn sources(path: &std::path::Path) {
        for file in [
            "sweetpad-lib/ci/linux-setup.sh",
            "sweetpad-lib/ci/linux-sdk.py",
            "sweetpad-lib/ci/linux-toolchain.json",
            "sweetpad-cli/macros/Package.swift",
        ] {
            let p = path.join(file);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "fixture").unwrap();
        }
    }

    #[test]
    fn explicit_public_sources_are_supported() {
        let dir = TempDir::new("setup-source-override");
        sources(&dir);
        assert_eq!(setup_root(Some(&dir)).unwrap(), dir.join(""));
    }

    #[test]
    fn explicit_incomplete_source_never_falls_back() {
        let dir = TempDir::new("setup-source");
        sources(&dir.join("source"));
        assert!(setup_root(Some(&dir.join("missing"))).is_err());
    }

    #[test]
    fn environment_preserves_cli_and_quotes_sdk_paths() {
        let text = environment_contents(std::path::Path::new("/private/SDK's directory"));
        assert!(text.contains("SDK'\\''s directory"));
        assert!(!text.contains(".local/bin"));
        assert!(text.contains("$PATH:$HOME/bin"));
        assert!(text.contains("export SWEETPAD_SWBUILD="));
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
        std::fs::write(&env_file, environment_contents(&sdk)).unwrap();
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
