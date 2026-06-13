//! Thin wrapper over `xcrun simctl` — enumerating, finding, and driving iOS
//! simulators. Shared by the `simulator`, `destination`, and `app` commands.
//! Mirrors the device shape the VS Code extension parses from
//! `simctl list --json devices`.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::cli::{process, CliError};

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
    let parsed: ListOutput =
        serde_json::from_str(&raw).map_err(|e| CliError::new(format!("parsing simctl output: {e}")))?;

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
    Err(CliError::new(format!("simctl boot failed: {}", stderr.trim())))
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

/// `com.apple.CoreSimulator.SimRuntime.iOS-17-0` → (`iOS`, `17.0`).
fn parse_runtime(runtime: &str) -> (String, String) {
    let tail = runtime.rsplit('.').next().unwrap_or(runtime); // iOS-17-0
    match tail.split_once('-') {
        Some((os, version)) => (os.to_string(), version.replace('-', ".")),
        None => (tail.to_string(), String::new()),
    }
}
