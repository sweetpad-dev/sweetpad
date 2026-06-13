//! `sweetpad device …` — inspect connected physical devices (via
//! `xcrun devicectl`). Running on a device is `sweetpad app run --device`.

use clap::Subcommand;

use crate::cli::{CliResult, Context, devicectl};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List connected physical devices.
    List,
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::List => list(ctx),
    }
}

fn list(ctx: &mut Context) -> CliResult {
    let devices = devicectl::list()?;

    if ctx.out.is_json() {
        let items: Vec<serde_json::Value> = devices
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
        ctx.out.json_value(&serde_json::json!({ "devices": items }));
        return Ok(());
    }

    if devices.is_empty() {
        ctx.out.note("no devices connected");
        return Ok(());
    }
    for d in &devices {
        let conn = if d.connection.is_empty() {
            String::new()
        } else {
            format!("  [{}]", d.connection)
        };
        ctx.out.line(&format!("{}{conn}", d.label()));
        ctx.out.line(&format!("    {}", d.udid));
    }
    Ok(())
}
