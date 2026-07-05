//! Thin wrapper over `xcrun devicectl` — listing and driving physical devices.
//! Shared by the `device` command and the `app … --device` path. `devicectl`
//! writes its listing to a `--json-output` file rather than stdout, so [`list`]
//! routes through a temp file.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::cli::{CliError, ErrorContext, process};

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

/// Install an `.app` bundle onto a device. Captures stdout (stderr stays
/// visible): `devicectl` prints install progress there, which is noise under
/// the caller's step line — and in `--json` mode it would interleave with the
/// envelope on the same stream and break the consumer's parse.
pub fn install(device_id: &str, app_path: &str) -> Result<(), CliError> {
    process::capture(
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
    .map(|_| ())
    .context("installing the app on the device")
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
    .context("launching the app on the device")
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

/// Like [`launch_console`] but spawned in the background with stdout/stderr piped,
/// handing back the child so the interactive `app run` session can render the device
/// console (the app's own output) while watching for the rebuild key.
pub fn spawn_console(device_id: &str, bundle_id: &str) -> Result<std::process::Child, CliError> {
    process::spawn_piped_both(
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

/// Uninstall an app from a device. Stdout is captured for the same reason as
/// [`install`] — keep `devicectl`'s progress chatter off the stream the
/// `--json` envelope goes to.
pub fn uninstall(device_id: &str, bundle_id: &str) -> Result<(), CliError> {
    process::capture(
        "xcrun",
        &[
            "devicectl",
            "device",
            "uninstall",
            "app",
            "--device",
            device_id,
            bundle_id,
        ],
        None,
    )
    .map(|_| ())
    .context("uninstalling the app from the device")
}

/// Terminate a running app on a device. `devicectl` has no
/// terminate-by-bundle-id — processes are addressed by pid — so the app's
/// pid(s) are looked up in the device's process list by the `.app` directory
/// name in the executable path, then signalled. Nothing running is success,
/// mirroring `simctl terminate`'s idempotence.
pub fn terminate(device_id: &str, app_dir_name: &str) -> Result<(), CliError> {
    if app_dir_name.is_empty() {
        return Err(CliError::new(
            "cannot terminate: the app bundle's name is unknown",
        ));
    }
    for pid in app_pids(device_id, app_dir_name)? {
        // Best-effort per pid: the process may have exited between the list
        // and the signal, which devicectl reports as a failure.
        let _ = process::run(
            "xcrun",
            &[
                "devicectl",
                "device",
                "process",
                "signal",
                "--signal",
                "15",
                "--pid",
                &pid.to_string(),
                "--device",
                device_id,
            ],
            None,
            true,
        );
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ProcessesOutput {
    result: ProcessesResult,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessesResult {
    #[serde(default)]
    running_processes: Vec<RawProcess>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProcess {
    #[serde(default)]
    process_identifier: i64,
    #[serde(default)]
    executable: String,
}

/// Pids of running processes whose executable lives inside the named `.app`
/// directory. `devicectl device info processes` routes through a
/// `--json-output` temp file like [`list`].
fn app_pids(device_id: &str, app_dir_name: &str) -> Result<Vec<i64>, CliError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp: PathBuf = std::env::temp_dir().join(format!(
        "sweetpad-processes-{}-{nanos}.json",
        std::process::id()
    ));
    let ok = process::run(
        "xcrun",
        &[
            "devicectl",
            "device",
            "info",
            "processes",
            "--device",
            device_id,
            "--json-output",
            &tmp.to_string_lossy(),
        ],
        None,
        true,
    )?;
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(CliError::new(
            "`xcrun devicectl device info processes` failed",
        ));
    }
    let raw = std::fs::read_to_string(&tmp)
        .map_err(|e| CliError::new(format!("reading devicectl output: {e}")))?;
    let _ = std::fs::remove_file(&tmp);
    parse_app_pids(&raw, app_dir_name)
}

/// Parse the process list, keeping pids whose executable path contains the
/// `.app` directory (executables are reported as `file://` URLs, e.g.
/// `file:///private/var/.../My.app/My`).
fn parse_app_pids(raw: &str, app_dir_name: &str) -> Result<Vec<i64>, CliError> {
    let parsed: ProcessesOutput = serde_json::from_str(raw)
        .map_err(|e| CliError::new(format!("parsing devicectl output: {e}")))?;
    let needle = format!("/{app_dir_name}/");
    Ok(parsed
        .result
        .running_processes
        .into_iter()
        .filter(|p| p.process_identifier > 0 && p.executable.contains(&needle))
        .map(|p| p.process_identifier)
        .collect())
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
    fn app_pids_match_the_bundle_directory() {
        let raw = r#"{
          "result": {
            "runningProcesses": [
              {"processIdentifier": 496, "executable": "file:///usr/libexec/backboardd"},
              {"processIdentifier": 1201, "executable": "file:///private/var/containers/Bundle/Application/AAAA/My.app/My"},
              {"processIdentifier": 1300, "executable": "file:///private/var/containers/Bundle/Application/BBBB/MyOther.app/MyOther"},
              {"processIdentifier": 0, "executable": "file:///x/My.app/My"}
            ]
          }
        }"#;
        assert_eq!(parse_app_pids(raw, "My.app").unwrap(), vec![1201]);
        assert_eq!(parse_app_pids(raw, "MyOther.app").unwrap(), vec![1300]);
        assert!(parse_app_pids(raw, "Absent.app").unwrap().is_empty());
    }

    #[test]
    fn find_matches_udid_and_name() {
        let devices = parse_devices(SAMPLE).unwrap();
        assert_eq!(find(&devices, "udid-1").unwrap().name, "My iPhone");
        assert_eq!(find(&devices, "Alpha iPad").unwrap().udid, "ID-2");
        assert!(find(&devices, "nope").is_none());
    }
}
