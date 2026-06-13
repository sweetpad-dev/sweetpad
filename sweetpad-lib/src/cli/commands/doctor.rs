//! `sweetpad doctor` — diagnose the local toolchain, "flutter doctor" style.
//!
//! Probes the tools the rest of the CLI shells out to (Xcode, `xcodebuild`,
//! Swift, `simctl` runtimes, `devicectl`, the formatters) and reports each as
//! ok / warning / problem with a remediation hint. A missing *required* tool is
//! a hard failure (non-zero exit); optional tools only warn.

use std::process::{Command, Stdio};

use crate::cli::{CliError, CliResult, Context};

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

pub fn run(ctx: &mut Context) -> CliResult {
    let checks = gather();

    if ctx.out.is_json() {
        let items: Vec<serde_json::Value> = checks
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
        let (ok, warn, fail) = summarize(&checks);
        ctx.out.json_value(&serde_json::json!({
            "checks": items,
            "summary": { "ok": ok, "warnings": warn, "problems": fail },
        }));
    } else {
        for c in &checks {
            ctx.out.line(&format!(
                "{} {}  {}",
                symbol(c.status, ctx.out.use_color()),
                c.name,
                c.detail
            ));
            if c.status != Status::Ok
                && let Some(hint) = c.hint
            {
                ctx.out.note(&format!("    ↳ {hint}"));
            }
        }
        let (_, warn, fail) = summarize(&checks);
        ctx.out
            .note(&format!("\n{fail} problem(s), {warn} warning(s)"));
    }

    let (_, _, fail) = summarize(&checks);
    if fail > 0 {
        return Err(CliError::new(format!(
            "doctor found {fail} problem(s) — see above"
        )));
    }
    Ok(())
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
            hint: Some("install Xcode, then run `xcode-select --switch /Applications/Xcode.app`"),
        },
    });

    // xcodebuild — the build/test backend.
    checks.push(tool_check(
        "xcodebuild",
        first_line(probe("xcodebuild", &["-version"])),
        Status::Fail,
        Some("install Xcode and accept its license (`sudo xcodebuild -license`)"),
    ));

    // Swift toolchain.
    checks.push(tool_check(
        "swift",
        first_line(probe("swift", &["--version"])),
        Status::Fail,
        Some("install the Xcode command-line tools (`xcode-select --install`)"),
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

    // swift-format — the default `sweetpad format` backend.
    checks.push(tool_check(
        "swift-format",
        first_line(probe("xcrun", &["--find", "swift-format"])),
        Status::Warn,
        Some("bundled with recent Xcode; or `brew install swift-format` — needed for `sweetpad format`"),
    ));

    // SwiftLint — optional formatter/linter backend.
    checks.push(tool_check(
        "swiftlint",
        first_line(probe("swiftlint", &["version"])),
        Status::Warn,
        Some("optional: `brew install swiftlint` for `sweetpad format --tool swiftlint`"),
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
