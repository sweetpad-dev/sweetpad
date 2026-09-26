//! `sweetpad device …` — physical devices (via `xcrun devicectl`): list the
//! paired ones, or connect to one and say whether it can be built to. Running
//! on a device is `sweetpad run --on device`.

use std::time::Duration;

use clap::Subcommand;

use crate::cli::buildlog::outcome_word;
use crate::cli::output::Output;
use crate::cli::{
    CliError, CommandResult, Context, ErrorKind, Render, Rendered, devicectl, simctl,
};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List the physical devices paired with this Mac.
    List,
    /// Check that a physical device is ready to build to and run on.
    ///
    /// Connects to the device and reports its pairing, connection, Developer
    /// Mode, developer disk image, lock and boot state, then one verdict:
    /// ready, or the first thing to fix. Exits 1 when the device is not ready.
    Info {
        /// Device name or UDID, 'device', or a 'context alias' name. Defaults
        /// to the only paired device.
        device: Option<String>,
        /// Seconds to wait for the device to answer before reporting it
        /// unreachable (at least 5, devicectl's minimum).
        #[arg(long, value_name = "SECONDS", default_value_t = 10, value_parser = parse_timeout)]
        timeout: u64,
    },
}

fn parse_timeout(value: &str) -> Result<u64, String> {
    let seconds: u64 = value
        .parse()
        .map_err(|_| format!("'{value}' is not a number of seconds"))?;
    if seconds < devicectl::MIN_TIMEOUT_SECS {
        return Err(format!(
            "must be at least {} seconds, devicectl's minimum",
            devicectl::MIN_TIMEOUT_SECS
        ));
    }
    Ok(seconds)
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::List => list(),
        Action::Info { device, timeout } => info(ctx, device.as_deref(), *timeout),
    }
}

/// The device list: human lines (label, link hint, udid) with a note when
/// empty, or the `data` of the JSON envelope as `{devices: […]}`.
struct DeviceList {
    devices: Vec<devicectl::Device>,
}

impl Render for DeviceList {
    fn human(&self, out: &Output) {
        if self.devices.is_empty() {
            out.note("no devices connected");
            return;
        }
        for d in &self.devices {
            let hint = d
                .link_hint()
                .map(|h| format!("  [{h}]"))
                .unwrap_or_default();
            out.line(&format!("{}{hint}", d.label()));
            out.line(&format!("    {}", d.udid));
        }
    }

    fn json(&self) -> serde_json::Value {
        let items: Vec<serde_json::Value> = self
            .devices
            .iter()
            .map(|d| {
                serde_json::json!({
                    "udid": d.udid,
                    "name": d.name,
                    "model": d.model,
                    "platform": d.platform,
                    "osVersion": d.os_version,
                    "connection": d.connection,
                    "transport": d.transport,
                    "pairing": d.pairing,
                })
            })
            .collect();
        serde_json::json!({ "devices": items })
    }
}

fn list() -> CommandResult {
    let devices = devicectl::list()?;
    Ok(Rendered::data(DeviceList { devices }))
}

/// The lock-state query runs only once the device has connected, so it
/// answers in well under devicectl's shortest timeout.
const LOCK_STATE_WAIT: Duration = Duration::from_secs(devicectl::MIN_TIMEOUT_SECS);

/// `device info`: resolve the device from the listing, then connect to it.
/// The listing alone cannot answer, since an idle device reads `disconnected`
/// there whether or not it works.
fn info(ctx: &Context, reference: Option<&str>, timeout: u64) -> CommandResult {
    let devices = devicectl::list()?;
    let listed = resolve_device(ctx, &devices, reference)?;
    let wait = Duration::from_secs(timeout);
    let probed = ctx.out.step(&format!("Connecting to {}", listed.name), || {
        devicectl::details(&listed.udid, wait)
    });
    let report = match probed {
        Ok(details) => {
            let locked = (details.device.connection == "connected")
                .then(|| devicectl::locked(&listed.udid, LOCK_STATE_WAIT))
                .flatten();
            Readiness {
                device: details.device,
                boot_state: details.boot_state,
                developer_mode: details.developer_mode,
                ddi_services_available: details.ddi_services_available,
                locked,
                warnings: details.warnings,
                failure: None,
                timeout,
            }
        }
        Err(e) => Readiness {
            device: listed.clone(),
            boot_state: String::new(),
            developer_mode: None,
            ddi_services_available: None,
            locked: None,
            warnings: Vec::new(),
            failure: Some(e.to_string()),
            timeout,
        },
    };
    let exit = u8::from(report.verdict().is_some());
    Ok(Rendered::data_with_exit(report, exit))
}

/// Settle the device a reference names, the way `--on` would but over
/// physical devices only: a `context alias` of the project in the current
/// directory first, then the lookup in [`choose`].
fn resolve_device<'a>(
    ctx: &Context,
    devices: &'a [devicectl::Device],
    reference: Option<&str>,
) -> Result<&'a devicectl::Device, CliError> {
    let alias = reference.and_then(|r| {
        let key = crate::cli::resolve::container(ctx).ok()?.key();
        ctx.state
            .projects
            .get(&key)?
            .destination_aliases
            .get(r)
            .cloned()
    });
    let reference = alias.as_deref().or(reference);
    choose(devices, reference).map_err(|miss| {
        let message = miss.message(devices, reference, |r| {
            simctl::list().is_ok_and(|sims| {
                sims.iter()
                    .any(|s| s.udid.eq_ignore_ascii_case(r) || s.name.eq_ignore_ascii_case(r))
            })
        });
        CliError::new(message).kind(ErrorKind::TargetResolution)
    })
}

/// Why a reference settled on no device.
#[derive(Debug, PartialEq, Eq)]
enum Miss {
    /// Nothing is paired at all.
    NonePaired,
    /// No reference, or `device`, with more than one paired.
    Unnamed,
    /// The reference matches no paired device.
    NoMatch,
    /// The reference is part of more than one device's name.
    Ambiguous(Vec<String>),
}

impl Miss {
    /// The error text. `is_simulator` is asked only for a reference that
    /// matched nothing, since it has to list the simulators.
    fn message(
        &self,
        devices: &[devicectl::Device],
        reference: Option<&str>,
        is_simulator: impl Fn(&str) -> bool,
    ) -> String {
        let labels = |devices: &[&devicectl::Device]| {
            devices
                .iter()
                .map(|d| format!("{} ({})", d.name, d.udid))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let names = devices
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let reference = reference.unwrap_or_default();
        match self {
            Self::NonePaired => "no physical device is paired with this Mac; connect one by USB, \
                                 unlock it, and tap Trust"
                .to_string(),
            Self::Unnamed => format!(
                "{} devices are paired with this Mac; name one: {}",
                devices.len(),
                labels(&devices.iter().collect::<Vec<_>>())
            ),
            Self::NoMatch if is_simulator(reference) => format!(
                "'{reference}' is a simulator, and 'device info' checks physical devices \
                 (paired: {names})"
            ),
            Self::NoMatch => format!("no paired device matches '{reference}' (paired: {names})"),
            Self::Ambiguous(udids) => {
                let matched: Vec<&devicectl::Device> =
                    devices.iter().filter(|d| udids.contains(&d.udid)).collect();
                format!(
                    "'{reference}' matches {} devices; name one: {}",
                    matched.len(),
                    labels(&matched)
                )
            }
        }
    }
}

/// The paired device a reference names: the only one when there is no
/// reference (or it is `device`), else a UDID, an exact name, or a part of
/// one name, each case-insensitive.
fn choose<'a>(
    devices: &'a [devicectl::Device],
    reference: Option<&str>,
) -> Result<&'a devicectl::Device, Miss> {
    if devices.is_empty() {
        return Err(Miss::NonePaired);
    }
    let Some(reference) = reference.filter(|r| !r.eq_ignore_ascii_case("device")) else {
        return match devices {
            [only] => Ok(only),
            _ => Err(Miss::Unnamed),
        };
    };
    if let Some(d) = devices
        .iter()
        .find(|d| d.udid.eq_ignore_ascii_case(reference))
        .or_else(|| {
            devices
                .iter()
                .find(|d| d.name.eq_ignore_ascii_case(reference))
        })
    {
        return Ok(d);
    }
    let lower = reference.to_ascii_lowercase();
    let partial: Vec<&devicectl::Device> = devices
        .iter()
        .filter(|d| d.name.to_ascii_lowercase().contains(&lower))
        .collect();
    match partial.as_slice() {
        [] => Err(Miss::NoMatch),
        [one] => Ok(one),
        many => Err(Miss::Ambiguous(
            many.iter().map(|d| d.udid.clone()).collect(),
        )),
    }
}

/// What `device info` found: the probe's facts and the verdict drawn from
/// them.
struct Readiness {
    device: devicectl::Device,
    boot_state: String,
    developer_mode: Option<String>,
    ddi_services_available: Option<bool>,
    locked: Option<bool>,
    warnings: Vec<String>,
    /// Why devicectl returned no details at all.
    failure: Option<String>,
    timeout: u64,
}

impl Readiness {
    /// `None` when the device is ready, otherwise the first thing in the way
    /// and how to clear it, in the order they have to be cleared: a device
    /// that is not paired cannot connect, and one that has not connected
    /// cannot report Developer Mode or its lock state.
    fn verdict(&self) -> Option<String> {
        let name = &self.device.name;
        let reach = "wake and unlock it, and keep it on the same Wi-Fi network as this \
                     Mac or connect it by USB";
        if !self.device.pairing.is_empty() && self.device.pairing != "paired" {
            return Some(format!(
                "{name} is not paired with this Mac; connect it by USB, unlock it, and tap Trust"
            ));
        }
        if let Some(failure) = &self.failure {
            return Some(format!("{name} did not answer ({failure}); {reach}"));
        }
        if self.device.connection != "connected" {
            return Some(format!(
                "{name} did not connect within {}s; {reach}",
                self.timeout
            ));
        }
        if let Some(mode) = &self.developer_mode
            && mode != "enabled"
        {
            return Some(
                "Developer Mode is off; turn it on in Settings > Privacy & Security > Developer \
                 Mode, then restart the device"
                    .to_string(),
            );
        }
        if !self.boot_state.is_empty() && self.boot_state != "booted" {
            return Some(format!(
                "{name} is not booted ({}); start it up",
                self.boot_state
            ));
        }
        if self.locked == Some(true) {
            return Some(if self.ddi_services_available == Some(true) {
                format!("{name} is locked; unlock it so apps can be launched on it")
            } else {
                format!("{name} is locked; unlock it so Xcode can start its development services")
            });
        }
        if self.ddi_services_available == Some(false) {
            let why = self
                .warnings
                .first()
                .map(|w| format!(" ({})", w.trim_end_matches('.')))
                .unwrap_or_default();
            return Some(format!(
                "{name}'s development services are not running{why}; open Xcode's Devices and \
                 Simulators window to see why"
            ));
        }
        None
    }
}

/// A fact the device may not have reported: its value, or `unknown`.
fn known(value: Option<&str>) -> &str {
    value.filter(|v| !v.is_empty()).unwrap_or("unknown")
}

impl Render for Readiness {
    fn human(&self, out: &Output) {
        let d = &self.device;
        out.line(&d.label());
        out.line(&format!("    {}", d.udid));
        let connection = match d.link_hint() {
            Some(hint) if !d.connection.is_empty() => format!("{} ({hint})", d.connection),
            _ => known(Some(&d.connection)).to_string(),
        };
        let disk = match self.ddi_services_available {
            Some(true) => "mounted",
            Some(false) => "not mounted",
            None => "unknown",
        };
        let lock = match self.locked {
            Some(true) => "locked",
            Some(false) => "unlocked",
            None => "unknown",
        };
        let rows = [
            ("pairing", known(Some(&d.pairing))),
            ("connection", connection.as_str()),
            ("developer mode", known(self.developer_mode.as_deref())),
            ("developer disk", disk),
            ("lock", lock),
            ("boot", known(Some(&self.boot_state))),
        ];
        for (name, value) in rows {
            out.line(&format!("  {name:<15} {value}"));
        }
        for warning in &self.warnings {
            out.line(&format!("  {:<15} {warning}", "devicectl"));
        }
        let color = out.use_color();
        match self.verdict() {
            None => out.line(&format!(
                "{} to build and run",
                outcome_word(true, "ready", color)
            )),
            Some(reason) => out.line(&format!(
                "{}: {reason}",
                outcome_word(false, "not ready", color)
            )),
        }
    }

    fn json(&self) -> serde_json::Value {
        let d = &self.device;
        let reason = self.verdict();
        let text = |s: &str| (!s.is_empty()).then(|| s.to_string());
        serde_json::json!({
            "udid": d.udid,
            "name": d.name,
            "model": d.model,
            "platform": d.platform,
            "osVersion": d.os_version,
            "destination": format!("platform={},id={}", d.platform, d.udid),
            "pairing": text(&d.pairing),
            "connection": text(&d.connection),
            "transport": text(&d.transport),
            "developerMode": self.developer_mode,
            "ddiServicesAvailable": self.ddi_services_available,
            "locked": self.locked,
            "bootState": text(&self.boot_state),
            "warnings": self.warnings,
            "error": self.failure,
            "ready": reason.is_none(),
            "reason": reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, udid: &str) -> devicectl::Device {
        devicectl::Device {
            udid: udid.to_string(),
            name: name.to_string(),
            model: "iPhone14,5".to_string(),
            platform: "iOS".to_string(),
            os_version: "27.0".to_string(),
            connection: "connected".to_string(),
            transport: "localNetwork".to_string(),
            pairing: "paired".to_string(),
        }
    }

    #[test]
    fn a_reference_settles_like_on_does() {
        let phone = device("Iphone 13", "00008110-000559182E90401E");
        let pad = device("Work iPad", "00008027-000A1B2C3D4E5F60");
        let both = [phone.clone(), pad.clone()];

        assert_eq!(choose(&[], None).unwrap_err(), Miss::NonePaired);
        assert_eq!(choose(&both, None).unwrap_err(), Miss::Unnamed);
        assert_eq!(choose(&both, Some("device")).unwrap_err(), Miss::Unnamed);
        let only = [phone.clone()];
        assert_eq!(choose(&only, None).unwrap().udid, phone.udid);
        assert_eq!(choose(&only, Some("device")).unwrap().udid, phone.udid);

        assert_eq!(
            choose(&both, Some("00008110-000559182e90401e"))
                .unwrap()
                .name,
            "Iphone 13"
        );
        assert_eq!(choose(&both, Some("iphone 13")).unwrap().udid, phone.udid);
        assert_eq!(choose(&both, Some("ipad")).unwrap().udid, pad.udid);
        assert_eq!(choose(&both, Some("Pixel")).unwrap_err(), Miss::NoMatch);
        assert!(matches!(
            choose(&both, Some("i")).unwrap_err(),
            Miss::Ambiguous(udids) if udids.len() == 2
        ));
    }

    #[test]
    fn a_miss_names_what_is_paired() {
        let both = [
            device("Iphone 13", "00008110-000559182E90401E"),
            device("Work iPad", "00008027-000A1B2C3D4E5F60"),
        ];
        let no_sims = |_: &str| false;
        assert_eq!(
            Miss::NoMatch.message(&both, Some("Pixel"), no_sims),
            "no paired device matches 'Pixel' (paired: Iphone 13, Work iPad)"
        );
        assert_eq!(
            Miss::NoMatch.message(&both, Some("iPhone 17"), |_| true),
            "'iPhone 17' is a simulator, and 'device info' checks physical devices \
             (paired: Iphone 13, Work iPad)"
        );
        assert_eq!(
            Miss::Unnamed.message(&both, None, no_sims),
            "2 devices are paired with this Mac; name one: Iphone 13 \
             (00008110-000559182E90401E), Work iPad (00008027-000A1B2C3D4E5F60)"
        );
        // Terminals print backticks literally.
        assert!(!Miss::NonePaired.message(&[], None, no_sims).contains('`'));
    }

    fn ready_phone() -> Readiness {
        Readiness {
            device: device("Iphone 13", "00008110-000559182E90401E"),
            boot_state: "booted".to_string(),
            developer_mode: Some("enabled".to_string()),
            ddi_services_available: Some(true),
            locked: Some(false),
            warnings: Vec::new(),
            failure: None,
            timeout: 10,
        }
    }

    #[test]
    fn the_verdict_names_the_first_thing_to_fix() {
        assert_eq!(ready_phone().verdict(), None);

        // The iPhone 13 this was written against: connected over Wi-Fi,
        // Developer Mode on, locked, so the disk image could not be mounted.
        let mut locked = ready_phone();
        locked.locked = Some(true);
        locked.ddi_services_available = Some(false);
        locked.warnings =
            vec!["The developer disk image could not be mounted on this device.".into()];
        assert_eq!(
            locked.verdict().unwrap(),
            "Iphone 13 is locked; unlock it so Xcode can start its development services"
        );

        let mut dev_mode = locked;
        dev_mode.developer_mode = Some("disabled".to_string());
        assert!(
            dev_mode
                .verdict()
                .unwrap()
                .starts_with("Developer Mode is off; turn it on in Settings > Privacy & Security")
        );

        let mut idle = dev_mode;
        idle.device.connection = "disconnected".to_string();
        assert!(
            idle.verdict()
                .unwrap()
                .starts_with("Iphone 13 did not connect within 10s;")
        );

        let mut unpaired = idle;
        unpaired.device.pairing = "unpaired".to_string();
        assert!(
            unpaired
                .verdict()
                .unwrap()
                .starts_with("Iphone 13 is not paired with this Mac;")
        );

        let mut failed = ready_phone();
        failed.failure = Some("devicectl did not finish within 10s".to_string());
        assert!(
            failed
                .verdict()
                .unwrap()
                .starts_with("Iphone 13 did not answer (devicectl did not finish within 10s);")
        );

        let mut unmounted = ready_phone();
        unmounted.ddi_services_available = Some(false);
        unmounted.warnings =
            vec!["The developer disk image could not be mounted on this device.".into()];
        assert_eq!(
            unmounted.verdict().unwrap(),
            "Iphone 13's development services are not running (The developer disk image could \
             not be mounted on this device); open Xcode's Devices and Simulators window to see why"
        );
    }

    #[test]
    fn the_json_carries_the_facts_and_the_verdict() {
        let mut phone = ready_phone();
        phone.locked = Some(true);
        let json = phone.json();
        assert_eq!(json["ready"], false);
        assert_eq!(
            json["reason"],
            "Iphone 13 is locked; unlock it so apps can be launched on it"
        );
        assert_eq!(json["developerMode"], "enabled");
        assert_eq!(json["transport"], "localNetwork");
        assert_eq!(
            json["destination"],
            "platform=iOS,id=00008110-000559182E90401E"
        );

        let json = ready_phone().json();
        assert_eq!(json["ready"], true);
        assert!(json["reason"].is_null());
        assert!(json["error"].is_null());
    }
}
