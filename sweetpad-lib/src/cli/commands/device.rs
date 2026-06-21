//! `sweetpad device …` — inspect connected physical devices (via
//! `xcrun devicectl`). Running on a device is `sweetpad app run --device`.

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::{CommandResult, Context, Render, Rendered, devicectl};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List connected physical devices.
    List,
}

pub fn run(_ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::List => list(),
    }
}

/// The device list: human lines (label + udid) with a note when empty, or the
/// `data` of the JSON envelope as `{devices: […]}`.
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
            let conn = if d.connection.is_empty() {
                String::new()
            } else {
                format!("  [{}]", d.connection)
            };
            out.line(&format!("{}{conn}", d.label()));
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
