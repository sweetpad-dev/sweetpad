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
            out.line(&format!(
                "{marker}{} · {} ({}){booted}",
                d.kind,
                d.name,
                d.os_label()
            ));
            out.line(&format!("    {}", d.specifier));
        }
    }

    fn json(&self) -> serde_json::Value {
        let items: Vec<serde_json::Value> = self
            .dests
            .iter()
            .map(|d| {
                serde_json::json!({
                    "kind": d.kind,
                    "name": d.name,
                    "os": d.os,
                    "osVersion": d.os_version,
                    "udid": d.udid,
                    "booted": d.booted,
                    "destination": d.specifier,
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
        });
    }

    // Devices are best-effort: no devices (or no devicectl) just means none.
    for d in devicectl::list().unwrap_or_default() {
        let platform = simctl_platform(&d.platform);
        dests.push(Dest {
            kind: "device",
            name: d.name.clone(),
            os: platform.to_string(),
            os_version: d.os_version.clone(),
            booted: None,
            specifier: format!("platform={platform},id={}", d.udid),
            udid: Some(d.udid),
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
