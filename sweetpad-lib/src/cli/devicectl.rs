//! Thin wrapper over `xcrun devicectl` — listing and driving physical devices.
//! Shared by the `device` command and the `app … --device` path. `devicectl`
//! writes its listing to a `--json-output` file rather than stdout, so [`list`]
//! routes through a temp file.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::cli::{CliError, process};

#[derive(Debug, Deserialize)]
struct ListOutput {
    result: ListResult,
}

#[derive(Debug, Deserialize)]
struct ListResult {
    #[serde(default)]
    devices: Vec<RawDevice>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDevice {
    #[serde(default)]
    connection_properties: ConnectionProperties,
    #[serde(default)]
    device_properties: DeviceProperties,
    #[serde(default)]
    hardware_properties: HardwareProperties,
    #[serde(default)]
    identifier: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionProperties {
    #[serde(default)]
    tunnel_state: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceProperties {
    #[serde(default)]
    name: String,
    #[serde(default)]
    os_version_number: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HardwareProperties {
    #[serde(default)]
    udid: String,
    #[serde(default)]
    marketing_name: String,
    #[serde(default)]
    platform: String,
}

/// A connected physical device.
#[derive(Debug, Clone)]
pub struct Device {
    pub udid: String,
    pub name: String,
    pub model: String,
    pub platform: String,
    pub os_version: String,
    pub connection: String,
}

impl Device {
    /// `"My iPhone (iPhone 15 Pro, iOS 17.0)"`.
    #[must_use]
    pub fn label(&self) -> String {
        format!(
            "{} ({}, {} {})",
            self.name, self.model, self.platform, self.os_version
        )
    }
}

/// Enumerate connected physical devices.
pub fn list() -> Result<Vec<Device>, CliError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp: PathBuf = std::env::temp_dir().join(format!(
        "sweetpad-devices-{}-{nanos}.json",
        std::process::id()
    ));

    let ok = process::run(
        "xcrun",
        &[
            "devicectl",
            "list",
            "devices",
            "--json-output",
            &tmp.to_string_lossy(),
            "--timeout",
            "10",
        ],
        None,
        true,
    )?;
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(CliError::new("`xcrun devicectl list devices` failed"));
    }

    let raw = std::fs::read_to_string(&tmp)
        .map_err(|e| CliError::new(format!("reading devicectl output: {e}")))?;
    let _ = std::fs::remove_file(&tmp);

    parse_devices(&raw)
}

/// Parse `devicectl list devices` JSON into sorted devices. Split out from
/// [`list`] so it's testable without `devicectl`. Devices missing a UDID
/// (devicectl returns empty hardwareProperties for some USB iOS ≤16 devices)
/// fall back to their `identifier`, and are dropped only if both are empty.
fn parse_devices(raw: &str) -> Result<Vec<Device>, CliError> {
    let parsed: ListOutput = serde_json::from_str(raw)
        .map_err(|e| CliError::new(format!("parsing devicectl output: {e}")))?;

    let mut devices: Vec<Device> = parsed
        .result
        .devices
        .into_iter()
        .filter_map(|d| {
            let udid = if d.hardware_properties.udid.is_empty() {
                d.identifier
            } else {
                d.hardware_properties.udid
            };
            if udid.is_empty() {
                return None;
            }
            Some(Device {
                udid,
                name: d.device_properties.name,
                model: d.hardware_properties.marketing_name,
                platform: if d.hardware_properties.platform.is_empty() {
                    "iOS".to_string()
                } else {
                    d.hardware_properties.platform
                },
                os_version: d.device_properties.os_version_number,
                connection: d.connection_properties.tunnel_state,
            })
        })
        .collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(devices)
}

/// Find a device by UDID (case-insensitive) or exact name.
#[must_use]
pub fn find<'a>(devices: &'a [Device], query: &str) -> Option<&'a Device> {
    devices
        .iter()
        .find(|d| d.udid.eq_ignore_ascii_case(query))
        .or_else(|| devices.iter().find(|d| d.name == query))
}

/// Install an `.app` bundle onto a device.
pub fn install(device_id: &str, app_path: &str) -> Result<(), CliError> {
    process::stream(
        "xcrun",
        &[
            "devicectl",
            "device",
            "install",
            "app",
            "--device",
            device_id,
            app_path,
        ],
        None,
    )
}

/// Launch an installed app on a device, terminating any existing instance.
pub fn launch(device_id: &str, bundle_id: &str) -> Result<String, CliError> {
    process::capture(
        "xcrun",
        &[
            "devicectl",
            "device",
            "process",
            "launch",
            "--terminate-existing",
            "--device",
            device_id,
            bundle_id,
        ],
        None,
    )
}

/// Launch with the console attached, streaming the app's stdout/stderr and
/// os_log output to the terminal until it exits (Xcode 16+). This is how device
/// log following works — `devicectl` has no attach-to-running-process console.
pub fn launch_console(device_id: &str, bundle_id: &str) -> Result<(), CliError> {
    process::stream(
        "xcrun",
        &[
            "devicectl",
            "device",
            "process",
            "launch",
            "--console",
            "--terminate-existing",
            "--device",
            device_id,
            bundle_id,
        ],
        None,
    )
}

/// Like [`launch_console`] but spawned in the background, handing back the child
/// so the interactive `app run` session can stream the device console while
/// watching for the rebuild key.
pub fn spawn_console(
    device_id: &str,
    bundle_id: &str,
) -> Result<std::process::Child, CliError> {
    process::spawn(
        "xcrun",
        &[
            "devicectl",
            "device",
            "process",
            "launch",
            "--console",
            "--terminate-existing",
            "--device",
            device_id,
            bundle_id,
        ],
        None,
    )
}

/// Terminate a running app on a device.
pub fn terminate(device_id: &str, bundle_id: &str) -> Result<(), CliError> {
    process::stream(
        "xcrun",
        &[
            "devicectl",
            "device",
            "process",
            "terminate",
            "--device",
            device_id,
            bundle_id,
        ],
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "result": {
        "devices": [
          {
            "identifier": "ID-1",
            "connectionProperties": {"tunnelState": "connected"},
            "deviceProperties": {"name": "My iPhone", "osVersionNumber": "17.0"},
            "hardwareProperties": {"udid": "UDID-1", "marketingName": "iPhone 15 Pro", "platform": "iOS"}
          },
          {
            "identifier": "ID-2",
            "connectionProperties": {"tunnelState": "disconnected"},
            "deviceProperties": {"name": "Alpha iPad"},
            "hardwareProperties": {}
          }
        ]
      }
    }"#;

    #[test]
    fn parses_devices_with_fallbacks() {
        let devices = parse_devices(SAMPLE).unwrap();
        assert_eq!(devices.len(), 2);
        // Sorted by name: "Alpha iPad" before "My iPhone".
        assert_eq!(devices[0].name, "Alpha iPad");
        // Empty hardwareProperties → udid falls back to identifier, platform to iOS.
        assert_eq!(devices[0].udid, "ID-2");
        assert_eq!(devices[0].platform, "iOS");

        let iphone = &devices[1];
        assert_eq!(iphone.udid, "UDID-1");
        assert_eq!(iphone.model, "iPhone 15 Pro");
        assert_eq!(iphone.connection, "connected");
        assert_eq!(iphone.label(), "My iPhone (iPhone 15 Pro, iOS 17.0)");
    }

    #[test]
    fn drops_devices_without_any_id() {
        let raw = r#"{"result":{"devices":[{"identifier":"","hardwareProperties":{}}]}}"#;
        assert!(parse_devices(raw).unwrap().is_empty());
    }

    #[test]
    fn find_matches_udid_and_name() {
        let devices = parse_devices(SAMPLE).unwrap();
        assert_eq!(find(&devices, "udid-1").unwrap().name, "My iPhone");
        assert_eq!(find(&devices, "Alpha iPad").unwrap().udid, "ID-2");
        assert!(find(&devices, "nope").is_none());
    }
}
