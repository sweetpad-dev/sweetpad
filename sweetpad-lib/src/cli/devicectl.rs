//! Thin wrapper over `xcrun devicectl` — listing and driving physical devices.
//! Shared by the `device` command and the `app … --device` path. `devicectl`
//! writes its listing to a `--json-output` file rather than stdout, so [`list`]
//! routes through a temp file.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::cli::{process, CliError};

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
        format!("{} ({}, {} {})", self.name, self.model, self.platform, self.os_version)
    }
}

/// Enumerate connected physical devices.
pub fn list() -> Result<Vec<Device>, CliError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp: PathBuf =
        std::env::temp_dir().join(format!("sweetpad-devices-{}-{nanos}.json", std::process::id()));

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

    let parsed: ListOutput =
        serde_json::from_str(&raw).map_err(|e| CliError::new(format!("parsing devicectl output: {e}")))?;

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
        &["devicectl", "device", "install", "app", "--device", device_id, app_path],
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

/// Terminate a running app on a device.
pub fn terminate(device_id: &str, bundle_id: &str) -> Result<(), CliError> {
    process::stream(
        "xcrun",
        &["devicectl", "device", "process", "terminate", "--device", device_id, bundle_id],
        None,
    )
}
