//! Thin wrapper over `xcrun devicectl` — listing and driving physical devices.
//! Shared by the `device` command and the `app … --device` path. `devicectl`
//! writes its listing to a `--json-output` file rather than stdout, so [`list`]
//! routes through a temp file.
//!
//! The listing comes in two shapes. Through `jsonVersion` 4 a device carried
//! `hardwareProperties` / `deviceProperties` / `connectionProperties`; version 5
//! (Xcode 27) adds a `properties` dictionary that supersedes all three and
//! carries a `_deprecationNotice` saying the old trio will be removed. Both are
//! read, `properties` first — see [`RawDevice`]. `device info details` returns
//! the same record for one device, after connecting to it ([`details`]).

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// One device as `devicectl` reports it, in either JSON shape. Every field is
/// defaulted, so a listing that has dropped the deprecated trio still
/// deserializes and answers from `properties` alone.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDevice {
    #[serde(default)]
    properties: Properties,
    #[serde(default)]
    connection_properties: ConnectionProperties,
    #[serde(default)]
    device_properties: DeviceProperties,
    #[serde(default)]
    hardware_properties: HardwareProperties,
    #[serde(default)]
    identifier: String,
}

impl RawDevice {
    fn udid(&self) -> &str {
        pick(
            &self.properties.hardware.udid,
            &self.hardware_properties.udid,
        )
    }

    fn name(&self) -> &str {
        pick(&self.properties.state.name, &self.device_properties.name)
    }

    /// The marketing name, else the product type (`iPhone14,5`): the listing
    /// leaves the marketing name out for some wireless devices.
    fn model(&self) -> &str {
        let marketing = pick(
            &self.properties.hardware.marketing_name,
            &self.hardware_properties.marketing_name,
        );
        pick(
            marketing,
            pick(
                &self.properties.hardware.product_type,
                &self.hardware_properties.product_type,
            ),
        )
    }

    /// `simulated` for the simulators Xcode 27's devicectl lists alongside the
    /// physical devices, `physical` for the rest.
    fn reality(&self) -> &str {
        pick(
            &self.properties.hardware.reality,
            &self.hardware_properties.reality,
        )
    }

    fn platform(&self) -> &str {
        pick(
            &self.properties.hardware.platform,
            &self.hardware_properties.platform,
        )
    }

    fn os_version(&self) -> &str {
        pick(
            &self.properties.software.os_version_number.string_value,
            &self.device_properties.os_version_number,
        )
    }

    /// Reachability. `properties.connection.state` is version 5's spelling of
    /// what `connectionProperties.tunnelState` said, and both report the same
    /// vocabulary (`connected` / `disconnected` / `unavailable`).
    fn connection(&self) -> &str {
        pick(
            &self.properties.connection.state,
            &self.connection_properties.tunnel_state,
        )
    }

    /// `wired` or `localNetwork`.
    fn transport(&self) -> &str {
        pick(
            &self.properties.connection.transport_type,
            &self.connection_properties.transport_type,
        )
    }

    /// `paired` once the device trusts this Mac.
    fn pairing(&self) -> &str {
        pick(
            &self.properties.connection.pairing_state,
            &self.connection_properties.pairing_state,
        )
    }

    fn boot_state(&self) -> &str {
        pick(
            &self.properties.state.boot_state,
            &self.device_properties.boot_state,
        )
    }

    /// `enabled` / `disabled`, reported only once a connection is made.
    /// Version 5 spells it as a one-case object (`{"enabled": {"mode": 1}}`),
    /// the deprecated field as the bare word.
    fn developer_mode(&self) -> Option<String> {
        let case = |status: &serde_json::Value| match status {
            serde_json::Value::String(word) if !word.is_empty() => Some(word.clone()),
            serde_json::Value::Object(cases) => cases.keys().next().cloned(),
            _ => None,
        };
        case(&self.properties.state.developer_mode_status)
            .or_else(|| case(&self.device_properties.developer_mode_status))
    }

    /// Present only in the deprecated `deviceProperties`.
    fn ddi_services_available(&self) -> Option<bool> {
        self.device_properties.ddi_services_available
    }

    /// The device this entry describes, or `None` for an entry with no id at
    /// all and for the simulators Xcode 27 lists alongside the devices.
    /// Devices missing a UDID (devicectl returns an empty hardware section for
    /// some USB iOS ≤16 devices) fall back to their `identifier`.
    fn to_device(&self) -> Option<Device> {
        let udid = pick(self.udid(), &self.identifier);
        if udid.is_empty() || self.reality() == "simulated" {
            return None;
        }
        let platform = if self.platform().is_empty() {
            "iOS"
        } else {
            self.platform()
        };
        Some(Device {
            udid: udid.to_string(),
            name: self.name().to_string(),
            model: self.model().to_string(),
            platform: platform.to_string(),
            os_version: self.os_version().to_string(),
            connection: self.connection().to_string(),
            transport: self.transport().to_string(),
            pairing: self.pairing().to_string(),
        })
    }
}

/// The version-5 value, falling back to the deprecated one when the listing
/// predates it (or leaves it blank).
fn pick<'a>(current: &'a str, deprecated: &'a str) -> &'a str {
    if current.is_empty() {
        deprecated
    } else {
        current
    }
}

/// `jsonVersion` 5's unified dictionary.
#[derive(Debug, Default, Deserialize)]
struct Properties {
    #[serde(default)]
    connection: ConnectionSection,
    #[serde(default)]
    hardware: HardwareSection,
    #[serde(default)]
    software: SoftwareSection,
    #[serde(default)]
    state: StateSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionSection {
    #[serde(default)]
    state: String,
    #[serde(default)]
    transport_type: String,
    #[serde(default)]
    pairing_state: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HardwareSection {
    #[serde(default)]
    udid: String,
    #[serde(default)]
    marketing_name: String,
    #[serde(default)]
    product_type: String,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    reality: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SoftwareSection {
    #[serde(default)]
    os_version_number: OsVersionNumber,
}

/// Where the deprecated `osVersionNumber` was the string itself, version 5
/// wraps it alongside the parsed components.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OsVersionNumber {
    #[serde(default)]
    string_value: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateSection {
    #[serde(default)]
    name: String,
    #[serde(default)]
    boot_state: String,
    #[serde(default)]
    developer_mode_status: serde_json::Value,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionProperties {
    #[serde(default)]
    tunnel_state: String,
    #[serde(default)]
    transport_type: String,
    #[serde(default)]
    pairing_state: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceProperties {
    #[serde(default)]
    name: String,
    #[serde(default)]
    os_version_number: String,
    #[serde(default)]
    boot_state: String,
    #[serde(default)]
    developer_mode_status: serde_json::Value,
    #[serde(default)]
    ddi_services_available: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HardwareProperties {
    #[serde(default)]
    udid: String,
    #[serde(default)]
    marketing_name: String,
    #[serde(default)]
    product_type: String,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    reality: String,
}

/// A physical device paired with this Mac.
#[derive(Debug, Clone)]
pub struct Device {
    pub udid: String,
    pub name: String,
    pub model: String,
    pub platform: String,
    pub os_version: String,
    /// devicectl's connection state. An idle device reads `disconnected`:
    /// xcodebuild and devicectl connect on demand, so it says nothing about
    /// whether the device can be built to.
    pub connection: String,
    /// devicectl's `transportType`: `wired` or `localNetwork`.
    pub transport: String,
    /// devicectl's `pairingState`: `paired` once the device trusts this Mac.
    pub pairing: String,
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

    /// How the device reaches this Mac, in a word or two for a listing: `usb`
    /// or `wifi`, plus `not paired` when it has not trusted this Mac. The
    /// connection state is left out, since an idle device always reads
    /// `disconnected`.
    #[must_use]
    pub fn link_hint(&self) -> Option<String> {
        let mut parts: Vec<&str> = Vec::new();
        match self.transport.as_str() {
            "" => {}
            "wired" => parts.push("usb"),
            "localNetwork" => parts.push("wifi"),
            other => parts.push(other),
        }
        if !self.pairing.is_empty() && self.pairing != "paired" {
            parts.push("not paired");
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}

/// A fresh temp path for one `devicectl` `--json-output` / `--log-output`, so
/// concurrent runs never share a file.
fn temp_file(stem: &str, extension: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "sweetpad-{stem}-{}-{nanos}.{extension}",
        std::process::id()
    ))
}

/// Enumerate the physical devices paired with this Mac.
pub fn list() -> Result<Vec<Device>, CliError> {
    let tmp = temp_file("devices", "json");

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
        return Err(CliError::new("'xcrun devicectl list devices' failed"));
    }

    let raw = std::fs::read_to_string(&tmp)
        .map_err(|e| CliError::new(format!("reading devicectl output: {e}")))?;
    let _ = std::fs::remove_file(&tmp);

    parse_devices(&raw)
}

/// Parse `devicectl list devices` JSON into sorted devices. Split out from
/// [`list`] so it's testable without `devicectl`. Entries without any id are
/// dropped, and so are the simulators Xcode 27 lists here: `simctl` already
/// reports them, and as devices they would get a `platform=iOS` specifier that
/// cannot build for them (see [`RawDevice::to_device`]).
fn parse_devices(raw: &str) -> Result<Vec<Device>, CliError> {
    let parsed: ListOutput = serde_json::from_str(raw)
        .map_err(|e| CliError::new(format!("parsing devicectl output: {e}")))?;

    let mut devices: Vec<Device> = parsed
        .result
        .devices
        .iter()
        .filter_map(RawDevice::to_device)
        .collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(devices)
}

/// What connecting to one device found. The listing reads CoreDevice's cached
/// record, which reports an idle device as `disconnected` and leaves out
/// Developer Mode; `device info details` opens a connection first, so it
/// knows whether the device answers and what its developer services are
/// doing.
#[derive(Debug, Clone)]
pub struct Details {
    /// The device as the probe saw it: `connection` is the state after the
    /// attempt to connect.
    pub device: Device,
    pub boot_state: String,
    /// `enabled` / `disabled`, or `None` when the device did not say.
    pub developer_mode: Option<String>,
    /// Whether the developer disk image's services are up. xcodebuild needs
    /// them to install and run, and mounts the image itself when it can.
    pub ddi_services_available: Option<bool>,
    /// What devicectl warned about while gathering, e.g. `The developer disk
    /// image could not be mounted on this device.` It writes these only to its
    /// human output, read back through `--log-output`.
    pub warnings: Vec<String>,
}

/// How long past its own `--timeout` a devicectl call may run before it is
/// killed.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// The shortest `--timeout` devicectl accepts.
pub const MIN_TIMEOUT_SECS: u64 = 5;

/// Connect to a device and read its details, waiting at most `timeout` for it
/// to answer. A device that cannot be reached still yields details (devicectl
/// returns the best it has); an error means devicectl itself failed or had to
/// be killed.
pub fn details(udid: &str, timeout: Duration) -> Result<Details, CliError> {
    let json = temp_file("details", "json");
    let log = temp_file("details", "log");
    let seconds = timeout.as_secs().max(MIN_TIMEOUT_SECS).to_string();
    let finished = process::run_quiet_within(
        "xcrun",
        &[
            "devicectl",
            "device",
            "info",
            "details",
            "--device",
            udid,
            "--json-output",
            &json.to_string_lossy(),
            "--log-output",
            &log.to_string_lossy(),
            "--timeout",
            &seconds,
        ],
        timeout + KILL_GRACE,
    )?;
    let raw = std::fs::read_to_string(&json).unwrap_or_default();
    let log_text = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&json);
    let _ = std::fs::remove_file(&log);
    if finished.is_none() {
        return Err(CliError::new(format!(
            "devicectl did not finish within {seconds}s"
        )));
    }
    if raw.trim().is_empty() {
        return Err(CliError::new("devicectl exited without a result"));
    }
    parse_details(&raw, &log_text)
}

/// `device info details` (and `lockState`) JSON: the record under `result`,
/// or an `error` whose description says why there is none.
#[derive(Debug, Deserialize)]
struct InfoOutput<T> {
    result: Option<T>,
    #[serde(default)]
    error: serde_json::Value,
}

impl<T> InfoOutput<T> {
    fn into_result(self) -> Result<T, CliError> {
        self.result.ok_or_else(|| {
            let description = self.error["userInfo"]["NSLocalizedDescription"]["string"]
                .as_str()
                .unwrap_or("devicectl reported no result");
            CliError::new(description.to_string())
        })
    }
}

/// Parse `device info details` JSON plus the log devicectl wrote alongside
/// it. Split out from [`details`] so it's testable without a device.
fn parse_details(raw: &str, log: &str) -> Result<Details, CliError> {
    let parsed: InfoOutput<RawDevice> = serde_json::from_str(raw)
        .map_err(|e| CliError::new(format!("parsing devicectl output: {e}")))?;
    let raw_device = parsed.into_result()?;
    let device = raw_device
        .to_device()
        .ok_or_else(|| CliError::new("devicectl described a device without an id"))?;
    let warnings = log
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Error: "))
        .map(str::to_string)
        .collect();
    Ok(Details {
        device,
        boot_state: raw_device.boot_state().to_string(),
        developer_mode: raw_device.developer_mode(),
        ddi_services_available: raw_device.ddi_services_available(),
        warnings,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LockState {
    passcode_required: Option<bool>,
}

/// Whether the device is locked right now (`device info lockState`'s
/// `passcodeRequired`), or `None` when it cannot say. A locked device cannot
/// have its developer services started or its apps launched.
#[must_use]
pub fn locked(udid: &str, timeout: Duration) -> Option<bool> {
    let json = temp_file("lock-state", "json");
    let seconds = timeout.as_secs().max(MIN_TIMEOUT_SECS).to_string();
    let finished = process::run_quiet_within(
        "xcrun",
        &[
            "devicectl",
            "device",
            "info",
            "lockState",
            "--device",
            udid,
            "--json-output",
            &json.to_string_lossy(),
            "--timeout",
            &seconds,
        ],
        timeout + KILL_GRACE,
    )
    .ok()
    .flatten();
    let raw = std::fs::read_to_string(&json).unwrap_or_default();
    let _ = std::fs::remove_file(&json);
    finished?;
    parse_lock_state(&raw)
}

fn parse_lock_state(raw: &str) -> Option<bool> {
    let parsed: InfoOutput<LockState> = serde_json::from_str(raw).ok()?;
    parsed.into_result().ok()?.passcode_required
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
pub fn launch(
    device_id: &str,
    bundle_id: &str,
    args: &[String],
    env: &[(String, String)],
    wait_for_debugger: bool,
) -> Result<String, CliError> {
    let mut cmd_args: Vec<&str> = vec![
        "devicectl",
        "device",
        "process",
        "launch",
        "--terminate-existing",
    ];
    if wait_for_debugger {
        cmd_args.push("--start-stopped");
    }
    cmd_args.extend_from_slice(&["--device", device_id, bundle_id]);
    // Trailing arguments go to the app, exactly as `devicectl … <bundle-id>
    // [<command-line-arguments> ...]` documents.
    cmd_args.extend(args.iter().map(String::as_str));
    // devicectl forwards `DEVICECTL_CHILD_*` from its own environment to the
    // app, the same shape simctl uses for `SIMCTL_CHILD_*`.
    process::capture_env("xcrun", &cmd_args, None, env).context("launching the app on the device")
}

/// Launch with the console attached, streaming the app's stdout/stderr and
/// os_log output to the terminal until it exits (Xcode 16+). This is how device
/// log following works — `devicectl` has no attach-to-running-process console.
pub fn launch_console(
    device_id: &str,
    bundle_id: &str,
    args: &[String],
    env: &[(String, String)],
) -> Result<(), CliError> {
    let mut cmd_args = console_args(device_id, bundle_id);
    cmd_args.extend(args.iter().map(String::as_str));
    process::stream_env("xcrun", &cmd_args, None, env)
}

/// The shared `devicectl … launch --console` prefix; the app's own arguments
/// are appended by the caller.
fn console_args<'a>(device_id: &'a str, bundle_id: &'a str) -> Vec<&'a str> {
    vec![
        "devicectl",
        "device",
        "process",
        "launch",
        "--console",
        "--terminate-existing",
        "--device",
        device_id,
        bundle_id,
    ]
}

/// Like [`launch_console`] but spawned in the background with stdout/stderr piped,
/// handing back the child so the interactive `app run` session can render the device
/// console (the app's own output) while watching for the rebuild key.
pub fn spawn_console(
    device_id: &str,
    bundle_id: &str,
    args: &[String],
    env: &[(String, String)],
) -> Result<std::process::Child, CliError> {
    let mut cmd_args = console_args(device_id, bundle_id);
    cmd_args.extend(args.iter().map(String::as_str));
    process::spawn_piped_both_env("xcrun", &cmd_args, None, env)
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
    let tmp = temp_file("processes", "json");
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
            "'xcrun devicectl device info processes' failed",
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

    /// `jsonVersion` 5 with the deprecated trio already gone — the shape a
    /// future devicectl is expected to emit. Trimmed from a real listing.
    const SAMPLE_V5: &str = r#"{
      "result": {
        "devices": [
          {
            "identifier": "ID-1",
            "properties": {
              "connection": {"pairingState": "paired", "state": "connected", "transportType": "wired"},
              "hardware": {
                "deviceType": "iPhone",
                "marketingName": "iPhone 18 Pro",
                "platform": "iOS",
                "productType": "iPhone19,1",
                "udid": "UDID-1"
              },
              "software": {
                "osVersionNumber": {"components": [27, 0, 0, 0, 0], "stringValue": "27.0"}
              },
              "state": {"bootState": "booted", "name": "My iPhone"}
            }
          },
          {
            "identifier": "ID-2",
            "properties": {"state": {"name": "Alpha iPad"}}
          }
        ]
      }
    }"#;

    #[test]
    fn parses_the_version_5_properties_shape() {
        let devices = parse_devices(SAMPLE_V5).unwrap();
        assert_eq!(devices.len(), 2);

        // Empty hardware section → udid falls back to identifier, platform to iOS.
        assert_eq!(devices[0].name, "Alpha iPad");
        assert_eq!(devices[0].udid, "ID-2");
        assert_eq!(devices[0].platform, "iOS");

        let iphone = &devices[1];
        assert_eq!(iphone.udid, "UDID-1");
        assert_eq!(iphone.model, "iPhone 18 Pro");
        assert_eq!(iphone.connection, "connected");
        // The version-5 osVersionNumber is an object, not the string itself.
        assert_eq!(iphone.os_version, "27.0");
        assert_eq!(iphone.label(), "My iPhone (iPhone 18 Pro, iOS 27.0)");
        assert_eq!(iphone.transport, "wired");
        assert_eq!(iphone.pairing, "paired");
    }

    /// Xcode 27's devicectl populates both shapes at once. They agree in
    /// practice; this pins which one is believed if they ever don't.
    #[test]
    fn properties_outrank_the_deprecated_trio() {
        let raw = r#"{
          "result": {
            "devices": [
              {
                "identifier": "ID-1",
                "connectionProperties": {"tunnelState": "disconnected"},
                "deviceProperties": {"name": "Stale Name", "osVersionNumber": "26.5"},
                "hardwareProperties": {"udid": "UDID-1", "marketingName": "Stale Model", "platform": "iOS"},
                "properties": {
                  "connection": {"state": "connected"},
                  "hardware": {"udid": "UDID-1", "marketingName": "iPhone 18 Pro", "platform": "iOS"},
                  "software": {"osVersionNumber": {"stringValue": "27.0"}},
                  "state": {"name": "My iPhone"}
                }
              }
            ]
          }
        }"#;
        let devices = parse_devices(raw).unwrap();
        assert_eq!(devices[0].name, "My iPhone");
        assert_eq!(devices[0].model, "iPhone 18 Pro");
        assert_eq!(devices[0].os_version, "27.0");
        assert_eq!(devices[0].connection, "connected");
    }

    #[test]
    fn drops_devices_without_any_id() {
        let raw = r#"{"result":{"devices":[{"identifier":"","hardwareProperties":{}}]}}"#;
        assert!(parse_devices(raw).unwrap().is_empty());
    }

    /// Xcode 27's listing with an idle wireless iPhone and one of the
    /// simulators devicectl lists beside it. Trimmed from a real `devicectl
    /// list devices` (jsonVersion 5), deprecated trio included.
    const SAMPLE_XCODE_27: &str = r#"{
      "result": {
        "devices": [
          {
            "connectionProperties": {"pairingState": "paired", "transportType": "localNetwork", "tunnelState": "disconnected"},
            "deviceProperties": {"bootState": "booted", "ddiServicesAvailable": false, "name": "Iphone 13", "osVersionNumber": "27.0"},
            "hardwareProperties": {"deviceType": "iPhone", "platform": "iOS", "productType": "iPhone14,5", "udid": "00008110-000559182E90401E"},
            "identifier": "8648BE6E-199F-55FE-B508-5B42071FEE92",
            "properties": {
              "connection": {"authenticationType": "manualPairing", "pairingState": "paired", "state": "disconnected", "transportType": "localNetwork", "tunnelTransportProtocol": "tcp"},
              "hardware": {"deviceType": "iPhone", "platform": "iOS", "productType": "iPhone14,5", "udid": "00008110-000559182E90401E"},
              "software": {"osVersionNumber": {"components": [27, 0, 0, 0, 0], "stringValue": "27.0"}},
              "state": {"bootState": "booted", "name": "Iphone 13"}
            }
          },
          {
            "connectionProperties": {"pairingState": "paired", "transportType": "sameMachine", "tunnelState": "connected"},
            "deviceProperties": {"name": "iPhone 17", "osVersionNumber": "27.0", "provider": "com.apple.CoreSimulator.SimulatorCoreDevicePlugin"},
            "hardwareProperties": {"marketingName": "iPhone 17", "platform": "iOS", "reality": "simulated", "udid": "F13C004A-0824-4870-B4F2-29AAEE36636E"},
            "identifier": "F13C004A-0824-4870-B4F2-29AAEE36636E",
            "properties": {
              "connection": {"pairingState": "paired", "state": "connected", "transportType": "sameMachine"},
              "hardware": {"marketingName": "iPhone 17", "platform": "iOS", "reality": "simulated", "udid": "F13C004A-0824-4870-B4F2-29AAEE36636E"},
              "state": {"bootState": "booted", "name": "iPhone 17", "visibilityClass": "simulators"}
            },
            "visibilityClass": "simulators"
          }
        ]
      }
    }"#;

    /// The simulator is `simctl`'s to list; as a device it would resolve
    /// `--on device` to it, or make it ambiguous.
    #[test]
    fn the_simulators_in_the_listing_are_not_devices() {
        let devices = parse_devices(SAMPLE_XCODE_27).unwrap();
        assert_eq!(devices.len(), 1, "{devices:?}");
        let phone = &devices[0];
        assert_eq!(phone.udid, "00008110-000559182E90401E");
        // No marketing name in this listing: the product type stands in.
        assert_eq!(phone.label(), "Iphone 13 (iPhone14,5, iOS 27.0)");
        assert_eq!(phone.connection, "disconnected");
        assert_eq!(phone.transport, "localNetwork");
        assert_eq!(phone.pairing, "paired");
    }

    #[test]
    fn the_link_hint_names_the_transport_and_never_the_idle_state() {
        let device = |transport: &str, pairing: &str| Device {
            udid: "U".to_string(),
            name: "Phone".to_string(),
            model: String::new(),
            platform: "iOS".to_string(),
            os_version: String::new(),
            connection: "disconnected".to_string(),
            transport: transport.to_string(),
            pairing: pairing.to_string(),
        };
        assert_eq!(
            device("localNetwork", "paired").link_hint().as_deref(),
            Some("wifi")
        );
        assert_eq!(
            device("wired", "paired").link_hint().as_deref(),
            Some("usb")
        );
        assert_eq!(
            device("wired", "unpaired").link_hint().as_deref(),
            Some("usb, not paired")
        );
        assert_eq!(device("", "").link_hint(), None);

        // A listing older than version 5 carries the same facts in the
        // deprecated trio.
        let raw = r#"{"result":{"devices":[{
          "connectionProperties": {"pairingState": "paired", "transportType": "wired", "tunnelState": "disconnected"},
          "deviceProperties": {"name": "My iPhone"},
          "hardwareProperties": {"udid": "UDID-1", "platform": "iOS"}
        }]}}"#;
        let devices = parse_devices(raw).unwrap();
        assert_eq!(devices[0].transport, "wired");
        assert_eq!(devices[0].pairing, "paired");
        assert_eq!(devices[0].link_hint().as_deref(), Some("usb"));
    }

    /// `devicectl device info details` for a locked iPhone on Wi-Fi, trimmed
    /// from a real run (Xcode 27, jsonVersion 5), with the log it wrote.
    const DETAILS_LOCKED: &str = r#"{
      "info": {"commandType": "devicectl.device.info.details", "jsonVersion": 5, "outcome": "success"},
      "result": {
        "connectionProperties": {"pairingState": "paired", "transportType": "localNetwork", "tunnelState": "connected"},
        "deviceProperties": {"bootState": "booted", "ddiServicesAvailable": false, "developerModeStatus": "enabled", "name": "Iphone 13", "osVersionNumber": "27.0"},
        "hardwareProperties": {"marketingName": "iPhone 13", "platform": "iOS", "productType": "iPhone14,5", "reality": "physical", "udid": "00008110-000559182E90401E"},
        "identifier": "8648BE6E-199F-55FE-B508-5B42071FEE92",
        "properties": {
          "connection": {"pairingState": "paired", "state": "connected", "transportType": "localNetwork"},
          "hardware": {"marketingName": "iPhone 13", "platform": "iOS", "productType": "iPhone14,5", "reality": "physical", "udid": "00008110-000559182E90401E"},
          "software": {"osVersionNumber": {"stringValue": "27.0"}},
          "state": {"bootState": "booted", "developerModeStatus": {"enabled": {"mode": 1}}, "name": "Iphone 13", "preparednessState": 1}
        }
      }
    }"#;

    const DETAILS_LOG: &str = "\
Gathering device information\u{2026}
WARNING: Unable to retrieve complete information for this device. The best available information will be returned.
         Error: The developer disk image could not be mounted on this device.
Current device information:
\u{2022} Identifier: 8648BE6E-199F-55FE-B508-5B42071FEE92
";

    #[test]
    fn details_read_developer_mode_the_disk_image_and_warnings() {
        let details = parse_details(DETAILS_LOCKED, DETAILS_LOG).unwrap();
        assert_eq!(details.device.udid, "00008110-000559182E90401E");
        assert_eq!(details.device.connection, "connected");
        assert_eq!(details.device.transport, "localNetwork");
        assert_eq!(details.boot_state, "booted");
        assert_eq!(details.developer_mode.as_deref(), Some("enabled"));
        assert_eq!(details.ddi_services_available, Some(false));
        assert_eq!(
            details.warnings,
            ["The developer disk image could not be mounted on this device."]
        );

        // Version 5's object spelling alone, as it reads once the deprecated
        // trio is gone; and a status devicectl did not report at all.
        let raw = r#"{"result":{"properties":{
          "hardware": {"udid": "U"},
          "state": {"developerModeStatus": {"disabled": {}}}
        }}}"#;
        let details = parse_details(raw, "").unwrap();
        assert_eq!(details.developer_mode.as_deref(), Some("disabled"));
        assert_eq!(details.ddi_services_available, None);
        let raw = r#"{"result":{"properties":{"hardware": {"udid": "U"}}}}"#;
        assert_eq!(parse_details(raw, "").unwrap().developer_mode, None);
    }

    /// A failed lookup carries devicectl's own description, captured from
    /// `device info details --device 00000000-0000000000000000`.
    #[test]
    fn a_details_failure_carries_devicectls_reason() {
        let raw = r#"{
          "error": {
            "code": 1000,
            "domain": "com.apple.dt.CoreDeviceError",
            "userInfo": {
              "DeviceName": {"string": "00000000-0000000000000000"},
              "NSLocalizedDescription": {"string": "The specified device was not found. (Name: 00000000-0000000000000000)"}
            }
          },
          "info": {"commandType": "devicectl.device.info.details", "jsonVersion": 5, "outcome": "failed"}
        }"#;
        let err = parse_details(raw, "").unwrap_err();
        assert_eq!(
            err.to_string(),
            "The specified device was not found. (Name: 00000000-0000000000000000)"
        );
    }

    /// `device info lockState`, captured from the same locked iPhone.
    #[test]
    fn lock_state_reads_passcode_required() {
        let raw = r#"{"info":{"outcome":"success"},"result":{"deviceIdentifier":"8648BE6E-199F-55FE-B508-5B42071FEE92","passcodeRequired":true,"unlockedSinceBoot":true}}"#;
        assert_eq!(parse_lock_state(raw), Some(true));
        assert_eq!(parse_lock_state(r#"{"error":{}}"#), None);
        assert_eq!(parse_lock_state(""), None);
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
