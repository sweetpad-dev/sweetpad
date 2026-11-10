//! `sweetpad doctor` — diagnose the local toolchain, "flutter doctor" style.
//!
//! Probes the tools the rest of the CLI shells out to (Xcode, `xcodebuild`,
//! Swift, `simctl` runtimes, `devicectl`, the formatters) and reports each as
//! ok / warning / problem with a remediation hint. A missing *required* tool is
//! a hard failure (non-zero exit); optional tools only warn.

use clap::{Args, ValueEnum};
use std::process::{Command, Stdio};

use crate::cli::output::Output;
use crate::cli::swiftpm;
use crate::cli::{CommandResult, Context, Render, Rendered};

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum Target {
    #[default]
    Ios,
    Macos,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Apple platform to diagnose on Linux.
    #[arg(long, value_enum)]
    pub target: Option<Target>,
}

/// One diagnostic line.
struct Check {
    name: &'static str,
    status: Status,
    /// Resolved version/path, or why it failed.
    detail: String,
    /// How to fix it (shown for warnings and problems).
    hint: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warning",
            Status::Fail => "problem",
        }
    }
}

pub fn run(_ctx: &mut Context, args: &DoctorArgs) -> CommandResult {
    let checks = if cfg!(target_os = "linux") {
        gather_linux(args.target.unwrap_or_default())
    } else {
        if args.target.is_some() {
            return Err(crate::cli::CliError::new(
                "doctor --target selects Linux SDK diagnostics; omit --target on macOS",
            )
            .kind(crate::cli::ErrorKind::Usage));
        }
        gather()
    };
    let (ok, warn, fail) = summarize(&checks);
    let report = DoctorReport {
        checks,
        ok,
        warn,
        fail,
    };
    // The report renders in both modes; a problem still forces a non-zero exit
    // (so CI catches a broken toolchain) without a separate error line.
    if fail > 0 {
        Ok(Rendered::data_with_exit(report, 1))
    } else {
        Ok(Rendered::data(report))
    }
}

fn linux_check(
    name: &'static str,
    ok: bool,
    detail: String,
    optional: bool,
    hint: &'static str,
) -> Check {
    Check {
        name,
        status: if ok {
            Status::Ok
        } else if optional {
            Status::Warn
        } else {
            Status::Fail
        },
        detail,
        hint: (!ok).then_some(hint),
    }
}

fn gather_linux(target: Target) -> Vec<Check> {
    let setup = "run sweetpad setup --sdk /path/to/apple-sdk.tar.gz, then source ~/.config/sweetpad/linux-env.sh";
    let mut checks = Vec::new();
    let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    checks.push(linux_check(
        "Linux host",
        cfg!(target_arch = "aarch64")
            && os.lines().any(|s| s == "ID=ubuntu")
            && os.lines().any(|s| s == "VERSION_ID=\"24.04\""),
        format!(
            "{}; {}",
            std::env::consts::ARCH,
            os.lines()
                .find(|s| s.starts_with("PRETTY_NAME="))
                .unwrap_or("unknown distribution")
        ),
        false,
        "use Ubuntu 24.04 ARM64",
    ));
    let swift =
        first_line(probe("swift", &["--version"])).unwrap_or_else(|| "swift missing".into());
    checks.push(linux_check(
        "swift",
        swift.contains(&format!("Swift version {}", super::sdk::toolchain().swift)),
        swift,
        false,
        setup,
    ));
    let swbuild = std::env::var("SWEETPAD_SWBUILD").unwrap_or_else(|_| "swbuild".into());
    let driver = Command::new(&swbuild)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    checks.push(linux_check(
        "Swift Build",
        driver,
        if driver {
            format!("{swbuild} starts successfully")
        } else {
            "swbuild missing or cannot start".into()
        },
        false,
        setup,
    ));
    let service = resolved_program(&swbuild)
        .and_then(|p| p.parent().map(|d| d.join("SWBBuildServiceBundle")));
    checks.push(linux_check(
        "Swift Build service",
        service.as_ref().is_some_and(|p| p.is_file()),
        service.map_or_else(
            || "build service location unavailable".into(),
            |p| p.display().to_string(),
        ),
        false,
        setup,
    ));
    checks.push(tool_check(
        "Darwin libtool",
        first_line(probe("llvm-libtool-darwin", &["--version"])),
        Status::Fail,
        Some("rerun sweetpad setup to install llvm-libtool-darwin"),
    ));
    let macros = crate::cli::swbuild::macro_plugin().unwrap_or_default();
    checks.push(linux_check(
        "Macro plugin",
        macros.is_file()
            && Command::new(&macros)
                .arg("--help")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success()),
        macros.display().to_string(),
        false,
        setup,
    ));
    checks.extend(gather_linux_sdk(target, setup));
    checks.extend(gather_linux_optional(target));
    checks
}

fn resolved_program(program: &str) -> Option<std::path::PathBuf> {
    let direct = std::path::Path::new(program);
    if direct.components().count() > 1 {
        return direct.canonicalize().ok();
    }
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(program))
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok())
}

fn gather_linux_sdk(target: Target, setup: &'static str) -> Vec<Check> {
    use super::sdk::default_sdk;
    let sdk = default_sdk();
    gather_sdk_at(target, setup, &sdk)
}

fn gather_sdk_at(target: Target, setup: &'static str, sdk: &std::path::Path) -> Vec<Check> {
    let mut checks = Vec::new();
    let paths = [
        "swift-sdk.json",
        "toolset-swb.json",
        "version.plist",
        "Developer/Toolchains/XcodeDefault.xctoolchain/usr/include/swift/bridging",
        "Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/clang/include/stddef.h",
        "toolset/bin/ld64.lld",
        "toolset/bin/llvm-lib",
    ];
    let missing: Vec<_> = paths
        .iter()
        .filter(|p| !sdk.join(p).exists())
        .copied()
        .collect();
    checks.push(linux_check(
        "Darwin SDK",
        missing.is_empty(),
        if missing.is_empty() {
            sdk.display().to_string()
        } else {
            format!("{}; missing: {}", sdk.display(), missing.join(", "))
        },
        false,
        setup,
    ));
    checks.push(linux_check("Localized App Intents", sdk.join("appintents-phrases.json").is_file(),
        "Localized shortcut phrase resources".into(), true,
        "export SDK resources on your Mac with sweetpad sdk export; only localized shortcut training needs these resources"));
    let metadata = std::fs::read(sdk.join("swift-sdk.json"))
        .ok()
        .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok());
    let triples: &[&str] = match target {
        Target::Ios => &["arm64-apple-ios", "arm64-apple-ios-simulator"],
        Target::Macos => &["arm64-apple-macosx"],
    };
    for &triple in triples {
        let root = metadata
            .as_ref()
            .and_then(|m| m["targetTriples"][triple]["sdkRootPath"].as_str());
        let exists = root.is_some_and(|p| sdk.join(p).join("SDKSettings.json").is_file());
        checks.push(linux_check(
            triple,
            exists,
            root.unwrap_or("missing target metadata").to_string(),
            false,
            setup,
        ));
    }
    if let Ok(data) = std::fs::read(sdk.join("sweetpad-sdk.json")) {
        let receipt: serde_json::Value = serde_json::from_slice(&data).unwrap_or_default();
        checks.push(linux_check(
            "SDK toolchain compatibility",
            receipt["toolchain"]["swift"].as_str() == Some(super::sdk::toolchain().swift.as_str())
                && receipt["toolchain"]["swiftBuild"].as_str()
                    == Some(super::sdk::toolchain().swift_build.as_str()),
            format!(
                "Xcode {}; Swift {}; Swift Build {}",
                receipt["xcodeVersion"],
                receipt["toolchain"]["swift"],
                receipt["toolchain"]["swiftBuild"]
            ),
            false,
            setup,
        ));
    }
    let linker = sdk.join("toolset/bin/ld64.lld");
    let linker_version = linker.to_str().and_then(|p| probe(p, &["--version"]));
    checks.push(linux_check(
        "Darwin linker",
        linker_version.is_some(),
        first_line(linker_version).unwrap_or_else(|| "cannot start linker".into()),
        false,
        setup,
    ));
    checks
}

fn gather_linux_optional(target: Target) -> Vec<Check> {
    use super::sdk::home;
    let mut checks = Vec::new();
    checks.push(tool_check(
        "IPA export (zip)",
        first_line(probe("zip", &["-v"])),
        Status::Warn,
        Some("install zip to export development IPAs"),
    ));
    let free = probe("df", &["-Pk", &home().to_string_lossy()]).and_then(|s| {
        s.lines()
            .last()?
            .split_whitespace()
            .nth(3)?
            .parse::<u64>()
            .ok()
    });
    checks.push(linux_check(
        "Free disk space",
        free.is_some_and(|n| n >= 10 * 1024 * 1024),
        free.map_or_else(
            || "could not measure free space".into(),
            |n| {
                format!(
                    "{}.{} GiB available",
                    n / 1_048_576,
                    (n % 1_048_576) * 10 / 1_048_576
                )
            },
        ),
        true,
        "free at least 10 GiB for app builds; toolchain setup can require more",
    ));
    if matches!(target, Target::Ios) {
        let usb = resolved_program("usbmuxd");
        checks.push(linux_check(
            "iPhone USB service",
            usb.is_some(),
            usb.map_or_else(|| "usbmuxd unavailable".into(), |p| p.display().to_string()),
            true,
            "install usbmuxd for direct USB connections; Wi-Fi transport does not require it",
        ));
        let identity = std::env::var_os("SWEETPAD_CODE_SIGN_P12").map(std::path::PathBuf::from);
        let configured = identity.as_ref().is_some_and(|p| p.is_file())
            && std::env::var_os("SWEETPAD_CODE_SIGN_P12_PASSWORD").is_some();
        checks.push(linux_check("Development signing", configured, if configured { "identity file and password configured" } else { "identity file or password not configured (unsigned builds remain available)" }.into(), true,
            "set SWEETPAD_CODE_SIGN_P12, SWEETPAD_CODE_SIGN_P12_PASSWORD and matching provisioning profiles for iPhone installation"));
        let profiles = crate::cli::codesign::provisioning::profile_folders()
            .iter()
            .any(|dir| {
                std::fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .any(|entry| {
                        let path = entry.path();
                        path.is_file()
                            && path
                                .extension()
                                .is_some_and(|e| e == "mobileprovision" || e == "provisionprofile")
                    })
            });
        checks.push(linux_check("Provisioning profiles", profiles, if profiles { "profile files found; bundle and entitlement matching happens during signing" } else { "no provisioning profiles found (unsigned builds remain available)" }.into(), true,
            "set SWEETPAD_PROVISIONING_PROFILES to a directory containing matching development profiles"));
        let python = std::env::var("SWEETPAD_DEVICE_PYTHON").unwrap_or_else(|_| "python3".into());
        let adapter = probe(
            &python,
            &[
                "-c",
                "import importlib.metadata; print(importlib.metadata.version('pymobiledevice3'))",
            ],
        );
        checks.push(linux_check("iPhone adapter", adapter.as_deref() == Some("11.50.0"), adapter.unwrap_or_else(|| "pymobiledevice3 unavailable (not required for compilation)".into()), true,
            "install pymobiledevice3==11.50.0 in a virtual environment and set SWEETPAD_DEVICE_PYTHON"));
    }
    checks
}

/// The toolchain report: the checks plus tallies. Renders as glyph lines and a
/// summary note, or `{checks, summary}` in the JSON envelope.
struct DoctorReport {
    checks: Vec<Check>,
    ok: usize,
    warn: usize,
    fail: usize,
}

impl Render for DoctorReport {
    fn human(&self, out: &Output) {
        for c in &self.checks {
            out.line(&format!(
                "{} {}  {}",
                symbol(c.status, out.use_color()),
                c.name,
                c.detail
            ));
            if c.status != Status::Ok
                && let Some(hint) = c.hint
            {
                out.note(&format!("    ↳ {hint}"));
            }
        }
        out.note(&format!(
            "\n{} problem(s), {} warning(s)",
            self.fail, self.warn
        ));
    }

    fn json(&self) -> serde_json::Value {
        let checks: Vec<serde_json::Value> = self
            .checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "status": c.status.as_str(),
                    "detail": c.detail,
                    "hint": c.hint,
                })
            })
            .collect();
        serde_json::json!({
            "checks": checks,
            "summary": { "ok": self.ok, "warnings": self.warn, "problems": self.fail },
        })
    }
}

/// Run every probe and collect the results.
fn gather() -> Vec<Check> {
    let mut checks = Vec::new();

    // Xcode developer directory — everything depends on this.
    checks.push(match probe("xcode-select", &["-p"]) {
        Some(path) => Check {
            name: "Xcode",
            status: Status::Ok,
            detail: path,
            hint: None,
        },
        None => Check {
            name: "Xcode",
            status: Status::Fail,
            detail: "xcode-select -p failed — no developer directory".into(),
            hint: Some("install Xcode, then run 'xcode-select --switch /Applications/Xcode.app'"),
        },
    });

    // xcodebuild — the build/test backend.
    checks.push(tool_check(
        "xcodebuild",
        first_line(probe("xcodebuild", &["-version"])),
        Status::Fail,
        Some("install Xcode and accept its license ('sudo xcodebuild -license')"),
    ));

    // Swift toolchain. Probed with a throwaway TMPDIR, since `swift --version`
    // leaves a temp directory behind.
    checks.push(tool_check(
        "swift",
        first_line(swiftpm::swift_version().filter(|s| !s.trim().is_empty())),
        Status::Fail,
        Some("install the Xcode command-line tools ('xcode-select --install')"),
    ));

    // Simulator runtimes — needed to run on a simulator.
    checks.push(
        match probe("xcrun", &["simctl", "list", "runtimes", "--json"]) {
            Some(json) => {
                let n = parse_runtime_count(&json);
                if n > 0 {
                    Check {
                        name: "Simulator runtimes",
                        status: Status::Ok,
                        detail: format!("{n} available"),
                        hint: None,
                    }
                } else {
                    Check {
                        name: "Simulator runtimes",
                        status: Status::Warn,
                        detail: "none installed".into(),
                        hint: Some("install one in Xcode ▸ Settings ▸ Platforms"),
                    }
                }
            }
            None => Check {
                name: "Simulator runtimes",
                status: Status::Warn,
                detail: "simctl unavailable".into(),
                hint: Some("ensure Xcode (not just the CLT) is selected"),
            },
        },
    );

    // devicectl — only needed for physical devices.
    checks.push(tool_check(
        "devicectl (physical devices)",
        first_line(probe("xcrun", &["devicectl", "--version"])),
        Status::Warn,
        Some("ships with Xcode 15+; only required to run on a real device"),
    ));

    // pymobiledevice3 — the only way to follow a physical device's os_log,
    // since the host `log stream` can't target a device.
    checks.push(tool_check(
        "pymobiledevice3 (device logs)",
        first_line(probe("pymobiledevice3", &["version"])),
        Status::Warn,
        Some("only required to stream logs from a physical device: brew install pymobiledevice3"),
    ));

    // swift-format — the default `sweetpad format` backend.
    checks.push(tool_check(
        "swift-format",
        first_line(probe("xcrun", &["--find", "swift-format"])),
        Status::Warn,
        Some("bundled with recent Xcode; or 'brew install swift-format' — needed for 'sweetpad format'"),
    ));

    // SwiftLint — optional formatter/linter backend.
    checks.push(tool_check(
        "swiftlint",
        first_line(probe("swiftlint", &["version"])),
        Status::Warn,
        Some("optional: 'brew install swiftlint' for 'sweetpad format --tool swiftlint'"),
    ));

    checks
}

/// Build a check from an optional probe result: present → ok, absent → the
/// given severity with the hint.
fn tool_check(
    name: &'static str,
    detail: Option<String>,
    missing: Status,
    hint: Option<&'static str>,
) -> Check {
    match detail {
        Some(detail) => Check {
            name,
            status: Status::Ok,
            detail,
            hint: None,
        },
        None => Check {
            name,
            status: missing,
            detail: "not found".into(),
            hint,
        },
    }
}

/// Run `program args…`, returning trimmed stdout on success, or `None` when the
/// tool is missing or exits non-zero. Both stdio streams are captured so the
/// report stays clean.
fn probe(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Keep only the first line of a probe result (versions are often multi-line).
fn first_line(detail: Option<String>) -> Option<String> {
    detail.map(|s| s.lines().next().unwrap_or_default().trim().to_string())
}

/// Count available runtimes in `simctl list runtimes --json` output.
fn parse_runtime_count(json: &str) -> usize {
    #[derive(serde::Deserialize)]
    struct Runtimes {
        runtimes: Vec<Runtime>,
    }
    #[derive(serde::Deserialize)]
    struct Runtime {
        #[serde(default, rename = "isAvailable")]
        is_available: bool,
    }
    serde_json::from_str::<Runtimes>(json)
        .map(|r| r.runtimes.iter().filter(|rt| rt.is_available).count())
        .unwrap_or(0)
}

/// (ok, warnings, problems) tallied across the checks.
fn summarize(checks: &[Check]) -> (usize, usize, usize) {
    let mut ok = 0;
    let mut warn = 0;
    let mut fail = 0;
    for c in checks {
        match c.status {
            Status::Ok => ok += 1,
            Status::Warn => warn += 1,
            Status::Fail => fail += 1,
        }
    }
    (ok, warn, fail)
}

/// Status glyph, colored when the terminal supports it.
fn symbol(status: Status, color: bool) -> &'static str {
    match (status, color) {
        (Status::Ok, true) => "\x1b[32m✓\x1b[0m",
        (Status::Ok, false) => "[ok]",
        (Status::Warn, true) => "\x1b[33m!\x1b[0m",
        (Status::Warn, false) => "[warn]",
        (Status::Fail, true) => "\x1b[31m✗\x1b[0m",
        (Status::Fail, false) => "[fail]",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sdk_without_localized_phrases_gets_a_warning() {
        let root = crate::cli::testdir::TempDir::new("doctor-legacy-sdk");
        for name in [
            "swift-sdk.json",
            "toolset-swb.json",
            "version.plist",
            "toolset/bin/ld64.lld",
            "toolset/bin/llvm-lib",
            "Developer/Toolchains/XcodeDefault.xctoolchain/usr/include/swift/bridging",
            "Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/clang/include/stddef.h",
        ] {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "{}").unwrap();
        }
        let checks = gather_sdk_at(Target::Ios, "setup hint", &root.join(""));
        assert_eq!(
            checks
                .iter()
                .find(|c| c.name == "Darwin SDK")
                .unwrap()
                .status,
            Status::Ok
        );
        assert_eq!(
            checks
                .iter()
                .find(|c| c.name == "Localized App Intents")
                .unwrap()
                .status,
            Status::Warn
        );
        std::fs::remove_file(root.join(
            "Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/clang/include/stddef.h",
        ))
        .unwrap();
        let missing = gather_sdk_at(Target::Ios, "setup hint", &root.join(""));
        assert_eq!(
            missing
                .iter()
                .find(|c| c.name == "Darwin SDK")
                .unwrap()
                .status,
            Status::Fail
        );
    }

    #[test]
    fn counts_only_available_runtimes() {
        let json = r#"{
            "runtimes": [
                {"name": "iOS 17.0", "isAvailable": true},
                {"name": "iOS 16.0", "isAvailable": false},
                {"name": "watchOS 10.0", "isAvailable": true}
            ]
        }"#;
        assert_eq!(parse_runtime_count(json), 2);
    }

    #[test]
    fn runtime_count_is_zero_on_garbage() {
        assert_eq!(parse_runtime_count("not json"), 0);
        assert_eq!(parse_runtime_count("{}"), 0);
    }

    #[test]
    fn first_line_trims_to_one_line() {
        assert_eq!(
            first_line(Some("Xcode 15.2\nBuild version 15C500".into())),
            Some("Xcode 15.2".to_string())
        );
        assert_eq!(first_line(None), None);
    }

    #[test]
    fn tool_check_present_is_ok_absent_is_severity() {
        let present = tool_check("x", Some("1.0".into()), Status::Fail, None);
        assert_eq!(present.status, Status::Ok);
        let absent = tool_check("x", None, Status::Warn, Some("h"));
        assert_eq!(absent.status, Status::Warn);
        assert_eq!(absent.detail, "not found");
    }

    #[test]
    fn summarize_tallies_by_status() {
        let checks = vec![
            Check {
                name: "a",
                status: Status::Ok,
                detail: String::new(),
                hint: None,
            },
            Check {
                name: "b",
                status: Status::Warn,
                detail: String::new(),
                hint: None,
            },
            Check {
                name: "c",
                status: Status::Fail,
                detail: String::new(),
                hint: None,
            },
            Check {
                name: "d",
                status: Status::Ok,
                detail: String::new(),
                hint: None,
            },
        ];
        assert_eq!(summarize(&checks), (2, 1, 1));
    }

    #[test]
    fn symbol_has_plain_fallback_without_color() {
        assert_eq!(symbol(Status::Ok, false), "[ok]");
        assert_eq!(symbol(Status::Fail, false), "[fail]");
        assert!(symbol(Status::Warn, true).contains('!'));
    }
}
