//! Thin wrapper over `xcrun simctl` — enumerating, finding, and driving iOS
//! simulators. Shared by the `simulator`, `destination`, and `app` commands.
//! Mirrors the device shape the VS Code extension parses from
//! `simctl list --json devices`.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::cli::{CliError, process};

/// `simctl list --json devices` output: runtime identifier → its devices.
#[derive(Debug, Deserialize)]
struct ListOutput {
    devices: BTreeMap<String, Vec<RawDevice>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDevice {
    udid: String,
    name: String,
    state: String,
    #[serde(default)]
    is_available: bool,
}

/// A simulator, with its runtime parsed into a friendly OS + version.
#[derive(Debug, Clone)]
pub struct Simulator {
    pub udid: String,
    pub name: String,
    /// `Booted` / `Shutdown` (as reported by simctl).
    pub state: String,
    pub available: bool,
    /// e.g. `iOS`, `watchOS`, `tvOS`, `xrOS`.
    pub os: String,
    /// e.g. `17.0`.
    pub os_version: String,
}

impl Simulator {
    #[must_use]
    pub fn is_booted(&self) -> bool {
        self.state.eq_ignore_ascii_case("Booted")
    }

    /// `"iPhone 15 (17.0)"`.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{} ({})", self.name, self.os_version)
    }

    /// The `xcodebuild -destination` specifier targeting this simulator,
    /// e.g. `platform=iOS Simulator,id=<udid>`.
    #[must_use]
    pub fn destination(&self) -> String {
        format!("platform={},id={}", platform(&self.os), self.udid)
    }
}

/// Map a simulator OS to its xcodebuild destination platform name.
#[must_use]
pub fn platform(os: &str) -> &'static str {
    match os {
        "watchOS" => "watchOS Simulator",
        "tvOS" => "tvOS Simulator",
        "xrOS" => "visionOS Simulator",
        _ => "iOS Simulator",
    }
}

/// Enumerate every available simulator, sorted by OS then name. Unavailable
/// devices are dropped (they can't be booted/targeted).
pub fn list() -> Result<Vec<Simulator>, CliError> {
    let raw = process::capture("xcrun", &["simctl", "list", "--json", "devices"], None)?;
    parse_devices(&raw)
}

/// Parse `simctl list --json devices` output into sorted, available
/// simulators. Split out from [`list`] so it's testable without `simctl`.
fn parse_devices(raw: &str) -> Result<Vec<Simulator>, CliError> {
    let parsed: ListOutput = serde_json::from_str(raw)
        .map_err(|e| CliError::new(format!("parsing simctl output: {e}")))?;

    let mut sims = Vec::new();
    for (runtime, devices) in parsed.devices {
        let (os, os_version) = parse_runtime(&runtime);
        for d in devices {
            if !d.is_available {
                continue;
            }
            sims.push(Simulator {
                udid: d.udid,
                name: d.name,
                state: d.state,
                available: d.is_available,
                os: os.clone(),
                os_version: os_version.clone(),
            });
        }
    }
    sims.sort_by(|a, b| {
        a.os.cmp(&b.os)
            .then_with(|| a.os_version.cmp(&b.os_version))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(sims)
}

/// Find a simulator by UDID (case-insensitive) or exact name. When several
/// share a name, the booted one wins, else the first.
#[must_use]
pub fn find<'a>(sims: &'a [Simulator], query: &str) -> Option<&'a Simulator> {
    if let Some(s) = sims.iter().find(|s| s.udid.eq_ignore_ascii_case(query)) {
        return Some(s);
    }
    let mut by_name: Vec<&Simulator> = sims.iter().filter(|s| s.name == query).collect();
    by_name.sort_by_key(|s| !s.is_booted());
    by_name.first().copied()
}

/// Boot a simulator. Already-booted is treated as success so the run/install
/// pipeline is idempotent.
pub fn boot(udid: &str) -> Result<(), CliError> {
    let output = std::process::Command::new("xcrun")
        .args(["simctl", "boot", udid])
        .output()
        .map_err(|e| CliError::new(format!("failed to run `xcrun simctl boot`: {e}")))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("current state: Booted") {
        return Ok(());
    }
    Err(CliError::new(format!(
        "simctl boot failed: {}",
        stderr.trim()
    )))
}

/// Install an `.app` bundle onto a booted simulator.
pub fn install(udid: &str, app_path: &str) -> Result<(), CliError> {
    process::stream("xcrun", &["simctl", "install", udid, app_path], None)
}

/// Launch an installed app by bundle id; returns simctl's stdout (`bundle: pid`).
pub fn launch(udid: &str, bundle_id: &str) -> Result<String, CliError> {
    process::capture("xcrun", &["simctl", "launch", udid, bundle_id], None)
}

/// Terminate a running app by bundle id.
pub fn terminate(udid: &str, bundle_id: &str) -> Result<(), CliError> {
    process::stream("xcrun", &["simctl", "terminate", udid, bundle_id], None)
}

/// Shut down a simulator. Already-shutdown is treated as success so the command
/// is idempotent (mirrors [`boot`]).
pub fn shutdown(udid: &str) -> Result<(), CliError> {
    let output = std::process::Command::new("xcrun")
        .args(["simctl", "shutdown", udid])
        .output()
        .map_err(|e| CliError::new(format!("failed to run `xcrun simctl shutdown`: {e}")))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("current state: Shutdown") || stderr.contains("Unable to shutdown") {
        return Ok(());
    }
    Err(CliError::new(format!(
        "simctl shutdown failed: {}",
        stderr.trim()
    )))
}

/// Erase a simulator's contents and settings (the device must be shut down).
pub fn erase(udid: &str) -> Result<(), CliError> {
    process::stream("xcrun", &["simctl", "erase", udid], None)
}

/// Open a URL on a booted simulator (`simctl openurl`) — drives deep links and
/// universal links into the app.
pub fn open_url(udid: &str, url: &str) -> Result<(), CliError> {
    process::stream("xcrun", &["simctl", "openurl", udid, url], None)
}

/// Capture a PNG screenshot of a booted simulator to `path`.
pub fn screenshot(udid: &str, path: &str) -> Result<(), CliError> {
    process::stream("xcrun", &["simctl", "io", udid, "screenshot", path], None)
}

/// Override a booted simulator's UI appearance (`light` / `dark`).
pub fn set_appearance(udid: &str, appearance: &str) -> Result<(), CliError> {
    process::stream(
        "xcrun",
        &["simctl", "ui", udid, "appearance", appearance],
        None,
    )
}

/// Open the Simulator.app GUI (no specific device required).
pub fn open_app() -> Result<(), CliError> {
    process::stream("open", &["-a", "Simulator"], None)
}

/// `com.apple.CoreSimulator.SimRuntime.iOS-17-0` → (`iOS`, `17.0`).
fn parse_runtime(runtime: &str) -> (String, String) {
    let tail = runtime.rsplit('.').next().unwrap_or(runtime); // iOS-17-0
    match tail.split_once('-') {
        Some((os, version)) => (os.to_string(), version.replace('-', ".")),
        None => (tail.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "devices": {
        "com.apple.CoreSimulator.SimRuntime.iOS-17-0": [
          {"udid":"AAAA","name":"iPhone 15","state":"Booted","isAvailable":true},
          {"udid":"BBBB","name":"iPhone 14","state":"Shutdown","isAvailable":true},
          {"udid":"DEAD","name":"Old","state":"Shutdown","isAvailable":false}
        ],
        "com.apple.CoreSimulator.SimRuntime.watchOS-10-0": [
          {"udid":"CCCC","name":"Apple Watch","state":"Shutdown","isAvailable":true}
        ]
      }
    }"#;

    #[test]
    fn parses_and_filters_unavailable() {
        let sims = parse_devices(SAMPLE).unwrap();
        // The unavailable "Old" device is dropped.
        assert_eq!(sims.len(), 3);
        assert!(sims.iter().all(|s| s.udid != "DEAD"));
    }

    #[test]
    fn sorts_by_os_then_version_then_name() {
        let sims = parse_devices(SAMPLE).unwrap();
        let order: Vec<&str> = sims.iter().map(|s| s.name.as_str()).collect();
        // iOS before watchOS; within iOS, name order.
        assert_eq!(order, vec!["iPhone 14", "iPhone 15", "Apple Watch"]);
    }

    #[test]
    fn parses_runtime_into_os_and_version() {
        assert_eq!(
            parse_runtime("com.apple.CoreSimulator.SimRuntime.iOS-17-0"),
            ("iOS".to_string(), "17.0".to_string())
        );
        assert_eq!(
            parse_runtime("com.apple.CoreSimulator.SimRuntime.watchOS-10-2"),
            ("watchOS".to_string(), "10.2".to_string())
        );
    }

    #[test]
    fn destination_specifier_maps_platform() {
        let sims = parse_devices(SAMPLE).unwrap();
        let watch = sims.iter().find(|s| s.os == "watchOS").unwrap();
        assert_eq!(
            watch.destination(),
            format!("platform=watchOS Simulator,id={}", watch.udid)
        );
        let iphone = sims.iter().find(|s| s.name == "iPhone 15").unwrap();
        assert_eq!(iphone.destination(), "platform=iOS Simulator,id=AAAA");
    }

    #[test]
    fn find_matches_udid_case_insensitively() {
        let sims = parse_devices(SAMPLE).unwrap();
        assert_eq!(find(&sims, "aaaa").unwrap().name, "iPhone 15");
    }

    #[test]
    fn find_by_name_prefers_booted() {
        let sims = vec![
            Simulator {
                udid: "1".into(),
                name: "Dup".into(),
                state: "Shutdown".into(),
                available: true,
                os: "iOS".into(),
                os_version: "17.0".into(),
            },
            Simulator {
                udid: "2".into(),
                name: "Dup".into(),
                state: "Booted".into(),
                available: true,
                os: "iOS".into(),
                os_version: "17.0".into(),
            },
        ];
        assert_eq!(find(&sims, "Dup").unwrap().udid, "2");
    }

    #[test]
    fn label_includes_version() {
        let s = Simulator {
            udid: "x".into(),
            name: "iPhone 15".into(),
            state: "Booted".into(),
            available: true,
            os: "iOS".into(),
            os_version: "17.0".into(),
        };
        assert_eq!(s.label(), "iPhone 15 (17.0)");
        assert!(s.is_booted());
    }
}
