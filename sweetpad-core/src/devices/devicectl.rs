//! `xcrun devicectl` output: the device listing, one device's details and
//! lock state, and the process list the app's pids come from. The CLI and
//! the extension each spawn devicectl (it writes to a `--json-output` file,
//! not stdout) and hand the file's text here.
//!
//! The listing comes in two shapes. Through `jsonVersion` 4 a device carried
//! `hardwareProperties` / `deviceProperties` / `connectionProperties`; version 5
//! (Xcode 27) adds a `properties` dictionary that supersedes all three and
//! carries a `_deprecationNotice` saying the old trio will be removed. Both are
//! read, `properties` first. Xcode 27's listing also holds the simulators
//! (`reality: "simulated"`), which are dropped: simctl lists them, and as
//! devices they would get a `platform=iOS` specifier that cannot build for
//! them.

use serde_json::Value;
use sweetpad_lib::destination::Platform;

/// Seconds between the Unix epoch and Core Foundation's 2001-01-01 reference
/// date, which version 5 counts `lastConnectionDate` from.
const CF_EPOCH_OFFSET_SECONDS: f64 = 978_307_200.0;

/// A physical device paired with this Mac.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Device {
    /// CoreDevice's own identifier, the one devicectl's `--device` also
    /// takes.
    pub identifier: String,
    /// The hex UDID xcodebuild's `id=` takes. Falls back to `identifier` for
    /// the entries devicectl lists with an empty hardware section (some USB
    /// iOS 16 and older devices).
    pub udid: String,
    /// Whether `udid` is the hardware UDID rather than the `identifier`
    /// fallback.
    pub has_hardware_udid: bool,
    pub name: String,
    /// The marketing name, else the product type (`iPhone14,5`): the listing
    /// leaves the marketing name out for some wireless devices.
    pub model: String,
    /// `iPhone 13`; empty when devicectl leaves it out.
    pub marketing_name: String,
    /// `iPhone14,5`.
    pub product_type: String,
    /// `iPhone`, `iPad`, `appleWatch`, `appleTV`, `appleVision`,
    /// `realityDevice`; empty when devicectl leaves it out.
    pub device_type: String,
    /// devicectl's platform, `iOS` when it reports none.
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
    /// When the device last connected, in milliseconds since the Unix epoch.
    pub last_connection_ms: Option<f64>,
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

    /// The `-destination platform=` label for this device, e.g. `iOS` or
    /// `visionOS`. A platform the table doesn't know keeps devicectl's name.
    #[must_use]
    pub fn platform_label(&self) -> &str {
        Platform::device_for_os(&self.platform).map_or(self.platform.as_str(), |p| p.label)
    }

    /// The `xcodebuild -destination` specifier targeting this device, e.g.
    /// `platform=iOS,id=<udid>`.
    #[must_use]
    pub fn destination(&self) -> String {
        format!("platform={},id={}", self.platform_label(), self.udid)
    }
}

/// One device record as devicectl reports it, in either JSON shape.
struct Raw<'a>(&'a Value);

impl<'a> Raw<'a> {
    fn at(&self, path: &[&str]) -> &'a Value {
        path.iter().fold(self.0, |v, key| &v[*key])
    }

    fn text(&self, path: &[&str]) -> &'a str {
        self.at(path).as_str().unwrap_or_default()
    }

    /// The version-5 value under `properties`, falling back to the deprecated
    /// one when the listing predates it (or leaves it blank).
    fn pick(&self, current: &[&str], deprecated: &[&str]) -> &'a str {
        let value = self.text(current);
        if value.is_empty() {
            self.text(deprecated)
        } else {
            value
        }
    }

    fn reality(&self) -> &'a str {
        self.pick(
            &["properties", "hardware", "reality"],
            &["hardwareProperties", "reality"],
        )
    }

    fn boot_state(&self) -> &'a str {
        self.pick(
            &["properties", "state", "bootState"],
            &["deviceProperties", "bootState"],
        )
    }

    /// `enabled` / `disabled`, reported only once a connection is made.
    /// Version 5 spells it as a one-case object (`{"enabled": {"mode": 1}}`),
    /// the deprecated field as the bare word.
    fn developer_mode(&self) -> Option<String> {
        let case = |status: &Value| match status {
            Value::String(word) if !word.is_empty() => Some(word.clone()),
            Value::Object(cases) => cases.keys().next().cloned(),
            _ => None,
        };
        case(self.at(&["properties", "state", "developerModeStatus"]))
            .or_else(|| case(self.at(&["deviceProperties", "developerModeStatus"])))
    }

    /// Version 5 counts seconds from Core Foundation's reference date; the
    /// deprecated field is an ISO 8601 string.
    fn last_connection_ms(&self) -> Option<f64> {
        if let Some(seconds) = self
            .at(&["properties", "connection", "lastConnectionDate"])
            .as_f64()
            .filter(|s| s.is_finite())
        {
            return Some((seconds + CF_EPOCH_OFFSET_SECONDS) * 1000.0);
        }
        iso8601_ms(self.text(&["connectionProperties", "lastConnectionDate"]))
    }

    /// The device this entry describes, or `None` for an entry with no id at
    /// all and for the simulators Xcode 27 lists alongside the devices.
    fn to_device(&self) -> Option<Device> {
        let identifier = self.text(&["identifier"]);
        let hardware_udid = self.pick(
            &["properties", "hardware", "udid"],
            &["hardwareProperties", "udid"],
        );
        let udid = if hardware_udid.is_empty() {
            identifier
        } else {
            hardware_udid
        };
        if udid.is_empty() || self.reality() == "simulated" {
            return None;
        }
        let marketing_name = self.pick(
            &["properties", "hardware", "marketingName"],
            &["hardwareProperties", "marketingName"],
        );
        let product_type = self.pick(
            &["properties", "hardware", "productType"],
            &["hardwareProperties", "productType"],
        );
        let platform = self.pick(
            &["properties", "hardware", "platform"],
            &["hardwareProperties", "platform"],
        );
        Some(Device {
            identifier: identifier.to_string(),
            udid: udid.to_string(),
            has_hardware_udid: !hardware_udid.is_empty(),
            name: self
                .pick(
                    &["properties", "state", "name"],
                    &["deviceProperties", "name"],
                )
                .to_string(),
            model: if marketing_name.is_empty() {
                product_type
            } else {
                marketing_name
            }
            .to_string(),
            marketing_name: marketing_name.to_string(),
            product_type: product_type.to_string(),
            device_type: self
                .pick(
                    &["properties", "hardware", "deviceType"],
                    &["hardwareProperties", "deviceType"],
                )
                .to_string(),
            platform: if platform.is_empty() { "iOS" } else { platform }.to_string(),
            os_version: self
                .pick(
                    &["properties", "software", "osVersionNumber", "stringValue"],
                    &["deviceProperties", "osVersionNumber"],
                )
                .to_string(),
            // `properties.connection.state` is version 5's spelling of what
            // `connectionProperties.tunnelState` said; both report
            // `connected` / `disconnected` / `unavailable`.
            connection: self
                .pick(
                    &["properties", "connection", "state"],
                    &["connectionProperties", "tunnelState"],
                )
                .to_string(),
            transport: self
                .pick(
                    &["properties", "connection", "transportType"],
                    &["connectionProperties", "transportType"],
                )
                .to_string(),
            pairing: self
                .pick(
                    &["properties", "connection", "pairingState"],
                    &["connectionProperties", "pairingState"],
                )
                .to_string(),
            last_connection_ms: self.last_connection_ms(),
        })
    }
}

/// Milliseconds since the Unix epoch for an ISO 8601 UTC timestamp as
/// devicectl writes it (`2026-09-26T21:50:00.000Z`, the fraction optional).
/// `None` for anything else.
fn iso8601_ms(text: &str) -> Option<f64> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    let mut date = date.splitn(3, '-').map(str::parse::<i64>);
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut clock = clock.splitn(3, ':').map(str::parse::<i64>);
    let (hour, minute, second) = (
        clock.next()?.ok()?,
        clock.next()?.ok()?,
        clock.next()?.ok()?,
    );
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    let fraction: f64 = if fraction.is_empty() {
        0.0
    } else {
        format!("0.{fraction}").parse().ok()?
    };
    // Days from 1970-01-01 to the date in the proleptic Gregorian calendar
    // (Howard Hinnant's days_from_civil).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    #[allow(clippy::cast_precision_loss)] // a date's seconds fit an f64 exactly
    let seconds = seconds as f64;
    Some((seconds + fraction) * 1000.0)
}

/// Parse `devicectl list devices` JSON into devices sorted by name. Entries
/// without any id are dropped, and so are the simulators Xcode 27 lists here.
pub fn parse_list(raw: &str) -> Result<Vec<Device>, String> {
    let parsed: Value =
        serde_json::from_str(raw).map_err(|e| format!("parsing devicectl output: {e}"))?;
    let devices = parsed["result"]
        .get("devices")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|d| Raw(d).to_device()).collect());
    let Some(mut devices): Option<Vec<Device>> = devices else {
        return if parsed["result"].is_object() {
            Ok(Vec::new())
        } else {
            Err("parsing devicectl output: it has no 'result'".to_string())
        };
    };
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
    /// Present only in the deprecated `deviceProperties`.
    pub ddi_services_available: Option<bool>,
    /// What devicectl warned about while gathering, e.g. `The developer disk
    /// image could not be mounted on this device.` It writes these only to its
    /// human output, read back through `--log-output`.
    pub warnings: Vec<String>,
}

/// The record under `result` of `device info details` (and `lockState`)
/// JSON, or the description devicectl's `error` gives for having none.
fn info_result(raw: &str) -> Result<Value, String> {
    let mut parsed: Value =
        serde_json::from_str(raw).map_err(|e| format!("parsing devicectl output: {e}"))?;
    match parsed.get_mut("result").map(Value::take) {
        Some(result) if !result.is_null() => Ok(result),
        _ => Err(
            parsed["error"]["userInfo"]["NSLocalizedDescription"]["string"]
                .as_str()
                .unwrap_or("devicectl reported no result")
                .to_string(),
        ),
    }
}

/// Parse `device info details` JSON plus the log devicectl wrote alongside
/// it.
pub fn parse_details(raw: &str, log: &str) -> Result<Details, String> {
    let result = info_result(raw)?;
    let record = Raw(&result);
    let device = record
        .to_device()
        .ok_or("devicectl described a device without an id")?;
    let warnings = log
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Error: "))
        .map(str::to_string)
        .collect();
    Ok(Details {
        device,
        boot_state: record.boot_state().to_string(),
        developer_mode: record.developer_mode(),
        ddi_services_available: record
            .at(&["deviceProperties", "ddiServicesAvailable"])
            .as_bool(),
        warnings,
    })
}

/// Whether `device info lockState` JSON says the device is locked
/// (`passcodeRequired`), or `None` when it cannot say.
#[must_use]
pub fn parse_lock_state(raw: &str) -> Option<bool> {
    info_result(raw).ok()?["passcodeRequired"].as_bool()
}

/// A process running out of an app bundle on a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppProcess {
    pub pid: i64,
    /// The executable as devicectl reports it, a `file://` URL.
    pub executable: String,
    /// The bundle's path on the device, decoded:
    /// `/private/var/containers/Bundle/Application/<id>/My App.app`.
    pub app_path: String,
    /// Whether this is the app's own executable, which sits directly in the
    /// bundle, rather than an extension's under `PlugIns/`.
    pub main: bool,
}

/// The processes in `device info processes` JSON whose executable lives
/// inside the `.app` directory named `app_dir_name` (`My App.app`). The
/// directory has to match whole, so `App.app` never claims `MyApp.app`'s
/// processes, and the URL is decoded first, so a bundle name with a space
/// matches.
pub fn parse_app_processes(raw: &str, app_dir_name: &str) -> Result<Vec<AppProcess>, String> {
    let parsed: Value =
        serde_json::from_str(raw).map_err(|e| format!("parsing devicectl output: {e}"))?;
    let needle = format!("/{app_dir_name}/");
    let processes = parsed["result"]["runningProcesses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let pid = p["processIdentifier"].as_i64().filter(|pid| *pid > 0)?;
            let executable = p["executable"].as_str()?;
            let path = crate::bsp::path_from_uri(executable)
                .to_string_lossy()
                .into_owned();
            let at = path.find(&needle)?;
            let inside = &path[at + needle.len()..];
            Some(AppProcess {
                pid,
                executable: executable.to_string(),
                app_path: path[..at + needle.len() - 1].to_string(),
                main: !inside.is_empty() && !inside.contains('/'),
            })
        })
        .collect();
    Ok(processes)
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
        let devices = parse_list(SAMPLE).unwrap();
        assert_eq!(devices.len(), 2);
        // Sorted by name: "Alpha iPad" before "My iPhone".
        assert_eq!(devices[0].name, "Alpha iPad");
        // Empty hardwareProperties → udid falls back to identifier, platform to iOS.
        assert_eq!(devices[0].udid, "ID-2");
        assert!(!devices[0].has_hardware_udid);
        assert_eq!(devices[0].platform, "iOS");
        assert_eq!(devices[0].device_type, "");

        let iphone = &devices[1];
        assert_eq!(iphone.udid, "UDID-1");
        assert!(iphone.has_hardware_udid);
        assert_eq!(iphone.identifier, "ID-1");
        assert_eq!(iphone.model, "iPhone 15 Pro");
        assert_eq!(iphone.connection, "connected");
        assert_eq!(iphone.label(), "My iPhone (iPhone 15 Pro, iOS 17.0)");
        assert_eq!(iphone.destination(), "platform=iOS,id=UDID-1");
    }

    /// `jsonVersion` 5 with the deprecated trio already gone — the shape a
    /// future devicectl is expected to emit. Trimmed from a real listing.
    const SAMPLE_V5: &str = r#"{
      "result": {
        "devices": [
          {
            "identifier": "ID-1",
            "properties": {
              "connection": {"pairingState": "paired", "state": "connected", "transportType": "wired", "lastConnectionDate": 812152200},
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
        let devices = parse_list(SAMPLE_V5).unwrap();
        assert_eq!(devices.len(), 2);

        // Empty hardware section → udid falls back to identifier, platform to iOS.
        assert_eq!(devices[0].name, "Alpha iPad");
        assert_eq!(devices[0].udid, "ID-2");
        assert_eq!(devices[0].platform, "iOS");
        assert_eq!(devices[0].last_connection_ms, None);

        let iphone = &devices[1];
        assert_eq!(iphone.udid, "UDID-1");
        assert_eq!(iphone.model, "iPhone 18 Pro");
        assert_eq!(iphone.device_type, "iPhone");
        assert_eq!(iphone.product_type, "iPhone19,1");
        assert_eq!(iphone.connection, "connected");
        // The version-5 osVersionNumber is an object, not the string itself.
        assert_eq!(iphone.os_version, "27.0");
        assert_eq!(iphone.label(), "My iPhone (iPhone 18 Pro, iOS 27.0)");
        assert_eq!(iphone.transport, "wired");
        assert_eq!(iphone.pairing, "paired");
        // 812152200s after 2001-01-01 is 2026-09-26T21:50:00Z.
        assert_eq!(iphone.last_connection_ms, Some(1_790_459_400_000.0));
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
                "connectionProperties": {"tunnelState": "disconnected", "lastConnectionDate": "2024-01-01T10:00:00Z"},
                "deviceProperties": {"name": "Stale Name", "osVersionNumber": "26.5"},
                "hardwareProperties": {"udid": "UDID-1", "marketingName": "Stale Model", "platform": "iOS"},
                "properties": {
                  "connection": {"state": "connected", "lastConnectionDate": 812152200},
                  "hardware": {"udid": "UDID-1", "marketingName": "iPhone 18 Pro", "platform": "iOS"},
                  "software": {"osVersionNumber": {"stringValue": "27.0"}},
                  "state": {"name": "My iPhone"}
                }
              }
            ]
          }
        }"#;
        let devices = parse_list(raw).unwrap();
        assert_eq!(devices[0].name, "My iPhone");
        assert_eq!(devices[0].model, "iPhone 18 Pro");
        assert_eq!(devices[0].os_version, "27.0");
        assert_eq!(devices[0].connection, "connected");
        assert_eq!(devices[0].last_connection_ms, Some(1_790_459_400_000.0));
    }

    #[test]
    fn the_deprecated_last_connection_date_is_an_iso_string() {
        let raw = r#"{"result":{"devices":[{
          "identifier": "ID-1",
          "connectionProperties": {"lastConnectionDate": "2026-09-26T21:50:00.250Z"},
          "hardwareProperties": {"udid": "UDID-1"}
        }]}}"#;
        let devices = parse_list(raw).unwrap();
        assert_eq!(devices[0].last_connection_ms, Some(1_790_459_400_250.0));
        assert_eq!(iso8601_ms("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(iso8601_ms("2000-03-01T00:00:00Z"), Some(951_868_800_000.0));
        assert_eq!(iso8601_ms("not a date"), None);
        assert_eq!(iso8601_ms("2026-13-01T00:00:00Z"), None);
        assert_eq!(iso8601_ms(""), None);
    }

    #[test]
    fn drops_devices_without_any_id() {
        let raw = r#"{"result":{"devices":[{"identifier":"","hardwareProperties":{}}]}}"#;
        assert!(parse_list(raw).unwrap().is_empty());
        assert!(parse_list(r#"{"result":{}}"#).unwrap().is_empty());
        assert!(parse_list("{}").is_err());
        assert!(parse_list("not json").is_err());
    }

    /// `devicectl list devices --json-output` on Xcode 27 (jsonVersion 5),
    /// with a paired wireless iPhone and the simulators devicectl lists
    /// beside it, trimmed to the fields sweetpad reads.
    const LIST_XCODE_27: &str = include_str!("testdata/devicectl-list-xcode27.json");

    /// The simulators are simctl's to list. As devices they would resolve
    /// `--on device` to one, or make it ambiguous, and give it a
    /// `platform=iOS` specifier that cannot build for it.
    #[test]
    fn the_simulators_in_the_listing_are_not_devices() {
        let devices = parse_list(LIST_XCODE_27).unwrap();
        assert_eq!(devices.len(), 1, "{devices:?}");
        let phone = &devices[0];
        assert_eq!(phone.udid, "00008110-000559182E90401E");
        assert_eq!(phone.identifier, "8648BE6E-199F-55FE-B508-5B42071FEE92");
        assert_eq!(phone.label(), "Iphone 13 (iPhone 13, iOS 27.0)");
        assert_eq!(phone.device_type, "iPhone");
        assert_eq!(phone.connection, "disconnected");
        assert_eq!(phone.transport, "localNetwork");
        assert_eq!(phone.pairing, "paired");
        assert_eq!(
            phone.destination(),
            "platform=iOS,id=00008110-000559182E90401E"
        );
    }

    #[test]
    fn a_device_destination_uses_the_platform_tables_label() {
        let device = |platform: &str| Device {
            udid: "U".to_string(),
            platform: platform.to_string(),
            ..Device::default()
        };
        assert_eq!(device("visionOS").destination(), "platform=visionOS,id=U");
        assert_eq!(device("xrOS").destination(), "platform=visionOS,id=U");
        assert_eq!(device("watchOS").destination(), "platform=watchOS,id=U");
        assert_eq!(device("futureOS").destination(), "platform=futureOS,id=U");
    }

    #[test]
    fn the_link_hint_names_the_transport_and_never_the_idle_state() {
        let device = |transport: &str, pairing: &str| Device {
            udid: "U".to_string(),
            name: "Phone".to_string(),
            platform: "iOS".to_string(),
            connection: "disconnected".to_string(),
            transport: transport.to_string(),
            pairing: pairing.to_string(),
            ..Device::default()
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
        let devices = parse_list(raw).unwrap();
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
        assert_eq!(
            parse_details(raw, "").unwrap_err(),
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

    const PROCESSES: &str = r#"{
      "result": {
        "runningProcesses": [
          {"processIdentifier": 496, "executable": "file:///usr/libexec/backboardd"},
          {"processIdentifier": 1201, "executable": "file:///private/var/containers/Bundle/Application/AAAA/My.app/My"},
          {"processIdentifier": 1202, "executable": "file:///private/var/containers/Bundle/Application/AAAA/My.app/PlugIns/Widget.appex/Widget"},
          {"processIdentifier": 1300, "executable": "file:///private/var/containers/Bundle/Application/BBBB/MyOther.app/MyOther"},
          {"processIdentifier": 1400, "executable": "file:///private/var/containers/Bundle/Application/CCCC/My%20App.app/My%20App"},
          {"processIdentifier": 1500},
          {"processIdentifier": 0, "executable": "file:///x/My.app/My"}
        ]
      }
    }"#;

    #[test]
    fn app_processes_match_the_whole_bundle_directory() {
        let pids = |app: &str| -> Vec<i64> {
            parse_app_processes(PROCESSES, app)
                .unwrap()
                .iter()
                .map(|p| p.pid)
                .collect()
        };
        assert_eq!(pids("My.app"), vec![1201, 1202]);
        assert_eq!(pids("MyOther.app"), vec![1300]);
        // `Other.app` is a suffix of `MyOther.app`, not its directory.
        assert!(pids("Other.app").is_empty());
        assert!(pids("App.app").is_empty());
        assert!(pids("Absent.app").is_empty());
    }

    /// devicectl reports executables as `file://` URLs, which spell a space
    /// `%20`. The bundle name is matched, and its path reported, decoded.
    #[test]
    fn an_app_process_is_found_by_its_decoded_bundle_name() {
        let found = parse_app_processes(PROCESSES, "My App.app").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pid, 1400);
        assert_eq!(
            found[0].app_path,
            "/private/var/containers/Bundle/Application/CCCC/My App.app"
        );
        assert!(found[0].main);
    }

    #[test]
    fn the_main_process_is_the_one_directly_in_the_bundle() {
        let found = parse_app_processes(PROCESSES, "My.app").unwrap();
        let main: Vec<(i64, bool)> = found.iter().map(|p| (p.pid, p.main)).collect();
        assert_eq!(main, vec![(1201, true), (1202, false)]);
        assert_eq!(
            found[0].app_path,
            "/private/var/containers/Bundle/Application/AAAA/My.app"
        );
        assert_eq!(found[1].app_path, found[0].app_path);
    }

    #[test]
    fn find_matches_udid_and_name() {
        let devices = parse_list(SAMPLE).unwrap();
        assert_eq!(find(&devices, "udid-1").unwrap().name, "My iPhone");
        assert_eq!(find(&devices, "Alpha iPad").unwrap().udid, "ID-2");
        assert!(find(&devices, "nope").is_none());
    }

    #[test]
    fn a_process_list_that_is_not_json_is_an_error() {
        assert!(parse_app_processes("nope", "My.app").is_err());
        assert!(parse_app_processes("{}", "My.app").unwrap().is_empty());
    }
}
