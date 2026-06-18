//! Thin wrapper over `xcrun simctl` — enumerating, finding, and driving iOS
//! simulators. Shared by the `simulator`, `destination`, and `app` commands.
//! Mirrors the device shape the VS Code extension parses from
//! `simctl list --json devices`.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::Deserialize;

use crate::cli::{CliError, ErrorContext, process};

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

    /// Destination kind for the remembered recents/usage records, e.g.
    /// `iOSSimulator`. Pairs the OS with the simulator role.
    #[must_use]
    pub fn kind(&self) -> String {
        format!("{}Simulator", self.os)
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

/// Enumerate every available simulator in picker order — platform first (iOS
/// before the rest), then newest OS version, then device family (iPhone before
/// iPad) and a numeric-aware name sort. Unavailable devices are dropped (they
/// can't be booted/targeted).
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
    sims.sort_by(cmp_for_picker);
    Ok(sims)
}

/// Order simulators for pickers and listings: platform priority, then newest OS
/// version, then device family, then a numeric-aware name sort. Each tier is
/// explicit so the order is intentional rather than a byte-compare side effect
/// (which is what made 17.0 sort before 9.0 and "iPad" before "iPhone").
fn cmp_for_picker(a: &Simulator, b: &Simulator) -> Ordering {
    platform_rank(&a.os)
        .cmp(&platform_rank(&b.os))
        .then_with(|| version_key(&b.os_version).cmp(&version_key(&a.os_version))) // newest first
        .then_with(|| device_rank(&a.name).cmp(&device_rank(&b.name)))
        .then_with(|| natural_cmp(&a.name, &b.name))
}

/// Platform display order: iOS first (the common case), then the other
/// families; anything unrecognized sorts last (so a future platform lands in a
/// defined place rather than wherever its name's bytes happen to fall).
fn platform_rank(os: &str) -> u8 {
    match os {
        "iOS" => 0,
        "tvOS" => 1,
        "watchOS" => 2,
        "xrOS" => 3,
        _ => 4,
    }
}

/// Device-family order within a platform: iPhone before iPad (the common pick),
/// then everything else. Platforms with a single family (Apple TV/Watch/Vision)
/// all land in the last bucket and fall through to the name sort.
fn device_rank(name: &str) -> u8 {
    if name.starts_with("iPhone") {
        0
    } else if name.starts_with("iPad") {
        1
    } else {
        2
    }
}

/// Parse a dotted version ("26.5") into numeric components so it orders
/// numerically: 9.0 before 17.0, where a byte compare puts "17.0" first.
/// Missing or garbled components count as 0.
fn version_key(version: &str) -> Vec<u32> {
    version.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Compare names so embedded numbers order numerically: "iPhone 9" before
/// "iPhone 15", which a plain byte compare reverses. Digit runs compare as
/// numbers; everything else compares byte-wise.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                match take_number(&mut a).cmp(&take_number(&mut b)) {
                    Ordering::Equal => {}
                    ord => return ord,
                }
            }
            (Some(x), Some(y)) => {
                a.next();
                b.next();
                match x.cmp(&y) {
                    Ordering::Equal => {}
                    ord => return ord,
                }
            }
        }
    }
}

/// Consume a leading run of digits as a number (saturating, so a pathologically
/// long run can't overflow).
fn take_number(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> u64 {
    let mut n: u64 = 0;
    while let Some(d) = it.peek().and_then(|c| c.to_digit(10)) {
        n = n.saturating_mul(10).saturating_add(u64::from(d));
        it.next();
    }
    n
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
        .context("installing the app on the simulator")
}

/// Launch an installed app by bundle id; returns simctl's stdout (`bundle: pid`).
pub fn launch(udid: &str, bundle_id: &str) -> Result<String, CliError> {
    process::capture("xcrun", &["simctl", "launch", udid, bundle_id], None)
        .context("launching the app on the simulator")
}

/// Launch with extra environment forwarded to `xcrun simctl`. Used by `--hot` to
/// pass `SIMCTL_CHILD_*` vars (which simctl strips and forwards into the app) so
/// the injection client dylib is `DYLD_INSERT_LIBRARIES`-loaded. Returns stdout.
pub fn launch_with_env(
    udid: &str,
    bundle_id: &str,
    env: &[(String, String)],
) -> Result<String, CliError> {
    let output = std::process::Command::new("xcrun")
        .args(["simctl", "launch", udid, bundle_id])
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .output()
        .map_err(|e| CliError::new(format!("failed to run `xcrun simctl launch`: {e}")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(CliError::new(format!(
            "simctl launch failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

/// Terminate a running app by bundle id. Already-stopped is treated as success
/// (idempotent, mirroring [`boot`]/[`shutdown`]): `simctl` errors with "found
/// nothing to terminate" when the app isn't running, which is not a failure for
/// `app stop` / session teardown.
pub fn terminate(udid: &str, bundle_id: &str) -> Result<(), CliError> {
    let output = std::process::Command::new("xcrun")
        .args(["simctl", "terminate", udid, bundle_id])
        .output()
        .map_err(|e| CliError::new(format!("failed to run `xcrun simctl terminate`: {e}")))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("found nothing to terminate")
        || stderr.contains("Unable to terminate")
        || stderr.contains("No such process")
    {
        return Ok(());
    }
    Err(CliError::new(format!(
        "simctl terminate failed: {}",
        stderr.trim()
    )))
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
        .context("opening the URL on the simulator")
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
    fn sorts_ios_before_watchos_then_by_name() {
        let sims = parse_devices(SAMPLE).unwrap();
        let order: Vec<&str> = sims.iter().map(|s| s.name.as_str()).collect();
        // iOS before watchOS; within iOS, name order.
        assert_eq!(order, vec!["iPhone 14", "iPhone 15", "Apple Watch"]);
    }

    // Several runtimes and families, to exercise every ordering tier at once.
    const MIXED: &str = r#"{
      "devices": {
        "com.apple.CoreSimulator.SimRuntime.iOS-26-5": [
          {"udid":"A","name":"iPhone 15","state":"Shutdown","isAvailable":true},
          {"udid":"B","name":"iPhone 9","state":"Shutdown","isAvailable":true},
          {"udid":"C","name":"iPad Air","state":"Shutdown","isAvailable":true}
        ],
        "com.apple.CoreSimulator.SimRuntime.iOS-17-0": [
          {"udid":"D","name":"iPhone 14","state":"Shutdown","isAvailable":true}
        ],
        "com.apple.CoreSimulator.SimRuntime.tvOS-26-0": [
          {"udid":"E","name":"Apple TV","state":"Shutdown","isAvailable":true}
        ],
        "com.apple.CoreSimulator.SimRuntime.watchOS-11-0": [
          {"udid":"F","name":"Apple Watch","state":"Shutdown","isAvailable":true}
        ],
        "com.apple.CoreSimulator.SimRuntime.xrOS-2-0": [
          {"udid":"G","name":"Apple Vision Pro","state":"Shutdown","isAvailable":true}
        ]
      }
    }"#;

    #[test]
    fn picker_order_is_platform_then_newest_then_family_then_natural() {
        let sims = parse_devices(MIXED).unwrap();
        let order: Vec<&str> = sims.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            order,
            vec![
                // iOS first; newest (26.5) before 17.0; iPhone before iPad;
                // "iPhone 9" before "iPhone 15" (numeric, not byte order).
                "iPhone 9",
                "iPhone 15",
                "iPad Air",
                "iPhone 14",
                // then the remaining platforms in priority order.
                "Apple TV",
                "Apple Watch",
                "Apple Vision Pro",
            ]
        );
    }

    #[test]
    fn natural_cmp_orders_numbers_numerically() {
        assert_eq!(natural_cmp("iPhone 9", "iPhone 15"), Ordering::Less);
        assert_eq!(natural_cmp("iPhone 15", "iPhone 15"), Ordering::Equal);
        assert_eq!(natural_cmp("iPhone 15 Pro", "iPhone 15"), Ordering::Greater);
        // A byte compare would put "iPhone 15" before "iPhone 9"; this must not.
        assert_eq!("iPhone 15".cmp("iPhone 9"), Ordering::Less);
        assert_eq!(natural_cmp("iPhone 15", "iPhone 9"), Ordering::Greater);
    }

    #[test]
    fn version_key_compares_numerically() {
        assert!(version_key("9.0") < version_key("17.0"));
        assert!(version_key("26.5") > version_key("26.4"));
        assert_eq!(version_key("26.5"), vec![26, 5]);
    }

    #[test]
    fn ranks_put_ios_and_iphone_first() {
        assert!(platform_rank("iOS") < platform_rank("tvOS"));
        assert!(platform_rank("watchOS") < platform_rank("unknownOS"));
        assert!(device_rank("iPhone 15") < device_rank("iPad Air"));
        assert!(device_rank("iPad Air") < device_rank("Apple TV"));
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
