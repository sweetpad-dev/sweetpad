//! `sweetpad destination …` — inspect build destinations.
//!
//! v1 surfaces simulator destinations (from `xcrun simctl`) in the
//! `xcodebuild -destination` specifier form, ready to pass to `build`/`app`.
//! Physical devices and macOS destinations come in a later iteration.

use clap::Subcommand;

use crate::cli::{simctl, CliResult, Context};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List build destinations (simulators) for the resolved scheme.
    List,
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::List => list(ctx),
    }
}

fn list(ctx: &mut Context) -> CliResult {
    let sims = simctl::list()?;

    if ctx.out.is_json() {
        let items: Vec<serde_json::Value> = sims
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "os": s.os,
                    "osVersion": s.os_version,
                    "udid": s.udid,
                    "booted": s.is_booted(),
                    "destination": s.destination(),
                })
            })
            .collect();
        ctx.out.json_value(&serde_json::json!({ "destinations": items }));
        return Ok(());
    }

    if sims.is_empty() {
        ctx.out.note("no destinations available");
        return Ok(());
    }
    for s in &sims {
        let state = if s.is_booted() { " [booted]" } else { "" };
        ctx.out.line(&format!("{}{state}", s.label()));
        ctx.out.line(&format!("    {}", s.destination()));
    }
    Ok(())
}
