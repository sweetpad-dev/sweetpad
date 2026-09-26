//! `sweetpad destination …` — inspect build destinations.
//!
//! Aggregates the targets `xcodebuild -destination` can address: macOS, every
//! available simulator (`xcrun simctl`), and connected physical devices
//! (`xcrun devicectl`). Each entry carries a ready-to-use `-destination`
//! specifier for `build`/`test`/`app`.

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::{CommandResult, Context, Render, Rendered, devicectl, simctl};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List build destinations: macOS, simulators, and connected devices.
    List,
}

pub fn run(_ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::List => list(),
    }
}

/// A build destination, kind-tagged for display and JSON.
struct Dest {
    kind: &'static str,
    name: String,
    /// Bare platform, e.g. `iOS` / `watchOS` / `macOS`.
    os: String,
    /// OS version (empty for macOS).
    os_version: String,
    booted: Option<bool>,
    udid: Option<String>,
    specifier: String,
    /// The physical device behind a `device` entry, for its connection facts.
    device: Option<devicectl::Device>,
}

impl Dest {
    /// `"iOS 17.0"`, or just `"macOS"` when there's no version.
    fn os_label(&self) -> String {
        if self.os_version.is_empty() {
            self.os.clone()
        } else {
            format!("{} {}", self.os, self.os_version)
        }
    }
}

/// The destination list: human lines (kind · name + specifier), or the `data`
/// of the JSON envelope as `{destinations: […]}`. The `devices` view adds a
/// `selected` mark (the project's remembered destination) on top.
struct DestList {
    dests: Vec<Dest>,
    /// The remembered destination's specifier, when a project resolves.
    selected: Option<String>,
}

impl Render for DestList {
    fn human(&self, out: &Output) {
        for d in &self.dests {
            let booted = if d.booted == Some(true) {
                " [booted]"
            } else {
                ""
            };
            let selected = self.selected.as_deref() == Some(d.specifier.as_str())
                || (self.selected.is_some()
                    && d.udid
                        .as_deref()
                        .is_some_and(|u| self.selected.as_deref().unwrap_or_default().contains(u)));
            let marker = if selected { "* " } else { "" };
            let link = d
                .device
                .as_ref()
                .and_then(devicectl::Device::link_hint)
                .map(|h| format!(" [{h}]"))
                .unwrap_or_default();
            out.line(&format!(
                "{marker}{} · {} ({}){booted}{link}",
                d.kind,
                d.name,
                d.os_label()
            ));
            out.line(&format!("    {}", d.specifier));
        }
    }

    /// A device entry also carries devicectl's `connection`, `transport` and
    /// `pairing`, the same fields `device list` reports; they are null on the
    /// other kinds.
    fn json(&self) -> serde_json::Value {
        let items: Vec<serde_json::Value> = self
            .dests
            .iter()
            .map(|d| {
                let device = d.device.as_ref();
                serde_json::json!({
                    "kind": d.kind,
                    "name": d.name,
                    "os": d.os,
                    "osVersion": d.os_version,
                    "udid": d.udid,
                    "booted": d.booted,
                    "destination": d.specifier,
                    "connection": device.map(|d| &d.connection),
                    "transport": device.map(|d| &d.transport),
                    "pairing": device.map(|d| &d.pairing),
                })
            })
            .collect();
        serde_json::json!({ "destinations": items })
    }
}

fn list() -> CommandResult {
    Ok(Rendered::data(DestList {
        dests: gather()?,
        selected: None,
    }))
}

/// `sweetpad devices` — the same aggregation, ordered most-used-first from the
/// project's usage stats (booted next), with the remembered destination
/// marked. The container is best-effort: outside a project the plain order
/// stands.
pub fn devices(ctx: &mut Context) -> CommandResult {
    let mut dests = gather()?;
    let mut selected = None;
    if let Ok(container) = crate::cli::resolve::container(ctx) {
        let key = container.key();
        if let Some(st) = ctx.state.projects.get(&key) {
            selected.clone_from(&st.destination);
            let usage = st.destination_usage.clone();
            let used = |d: &Dest| {
                d.udid
                    .as_deref()
                    .and_then(|u| usage.get(u).copied())
                    .unwrap_or(0)
            };
            dests.sort_by(|a, b| {
                used(b)
                    .cmp(&used(a))
                    .then_with(|| b.booted.unwrap_or(false).cmp(&a.booted.unwrap_or(false)))
            });
        }
    }
    Ok(Rendered::data(DestList { dests, selected }))
}

/// Everything `xcodebuild -destination` can address, in the resolver's order.
fn gather() -> Result<Vec<Dest>, crate::cli::CliError> {
    let mut dests = vec![Dest {
        kind: "macOS",
        name: "My Mac".to_string(),
        os: "macOS".to_string(),
        os_version: String::new(),
        booted: None,
        udid: None,
        specifier: "platform=macOS".to_string(),
        device: None,
    }];

    // Simulators are the common case; surface failure to enumerate them.
    for s in simctl::list()? {
        dests.push(Dest {
            kind: "simulator",
            name: s.name.clone(),
            os: s.os.clone(),
            os_version: s.os_version.clone(),
            booted: Some(s.is_booted()),
            specifier: s.destination(),
            udid: Some(s.udid),
            device: None,
        });
    }

    // Devices are best-effort: no devices (or no devicectl) just means none.
    for d in devicectl::list().unwrap_or_default() {
        let platform = simctl_platform(&d.platform).to_string();
        dests.push(Dest {
            kind: "device",
            name: d.name.clone(),
            os: platform.clone(),
            os_version: d.os_version.clone(),
            booted: None,
            specifier: format!("platform={platform},id={}", d.udid),
            udid: Some(d.udid.clone()),
            device: Some(d),
        });
    }
    Ok(dests)
}

/// xcodebuild destination platform name for a physical device's platform.
fn simctl_platform(platform: &str) -> &str {
    match platform {
        "" | "iOS" => "iOS",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device entry carries the listing's connection facts under the names
    /// `device list` uses; the other kinds carry nulls in their place.
    #[test]
    fn a_device_entry_carries_its_connection_facts() {
        let phone = devicectl::Device {
            udid: "00008110-000559182E90401E".to_string(),
            name: "Iphone 13".to_string(),
            model: "iPhone14,5".to_string(),
            platform: "iOS".to_string(),
            os_version: "27.0".to_string(),
            connection: "disconnected".to_string(),
            transport: "localNetwork".to_string(),
            pairing: "paired".to_string(),
        };
        let list = DestList {
            dests: vec![
                Dest {
                    kind: "macOS",
                    name: "My Mac".to_string(),
                    os: "macOS".to_string(),
                    os_version: String::new(),
                    booted: None,
                    udid: None,
                    specifier: "platform=macOS".to_string(),
                    device: None,
                },
                Dest {
                    kind: "device",
                    name: phone.name.clone(),
                    os: "iOS".to_string(),
                    os_version: phone.os_version.clone(),
                    booted: None,
                    udid: Some(phone.udid.clone()),
                    specifier: format!("platform=iOS,id={}", phone.udid),
                    device: Some(phone),
                },
            ],
            selected: None,
        };
        let json = list.json();
        let mac = &json["destinations"][0];
        assert!(mac["connection"].is_null() && mac["transport"].is_null());
        let device = &json["destinations"][1];
        assert_eq!(device["connection"], "disconnected");
        assert_eq!(device["transport"], "localNetwork");
        assert_eq!(device["pairing"], "paired");
    }
}
