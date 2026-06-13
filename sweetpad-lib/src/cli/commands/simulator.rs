//! `sweetpad simulator …` — manage iOS simulators (via `xcrun simctl`).

use clap::Subcommand;

use crate::cli::{CliError, CliResult, Context, resolve, simctl};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List available simulators.
    List,
    /// Boot a simulator (by name or UDID; prompts when omitted).
    Boot {
        /// Simulator name or UDID to boot.
        target: Option<String>,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::List => list(ctx),
        Action::Boot { target } => boot(ctx, target.as_deref()),
    }
}

fn list(ctx: &mut Context) -> CliResult {
    let sims = simctl::list()?;

    if ctx.out.is_json() {
        let items: Vec<serde_json::Value> = sims
            .iter()
            .map(|s| {
                serde_json::json!({
                    "udid": s.udid,
                    "name": s.name,
                    "os": s.os,
                    "osVersion": s.os_version,
                    "state": s.state,
                    "booted": s.is_booted(),
                })
            })
            .collect();
        ctx.out
            .json_value(&serde_json::json!({ "simulators": items }));
        return Ok(());
    }

    if sims.is_empty() {
        ctx.out.note("no simulators available");
        return Ok(());
    }
    for s in &sims {
        let state = if s.is_booted() { " [booted]" } else { "" };
        ctx.out.item(
            &format!("{} {}  {}{state}", s.os, s.label(), s.udid),
            s.is_booted(),
        );
    }
    Ok(())
}

fn boot(ctx: &mut Context, target: Option<&str>) -> CliResult {
    let sims = simctl::list()?;

    let sim = if let Some(t) = target {
        simctl::find(&sims, t)
            .ok_or_else(|| CliError::new(format!("no simulator matching {t:?}")))?
    } else {
        let labels: Vec<String> = sims.iter().map(simctl::Simulator::label).collect();
        let chosen = resolve::choose(ctx, "simulator", None, &labels)?;
        sims.iter()
            .find(|s| s.label() == chosen)
            .ok_or_else(|| CliError::new("simulator not found"))?
    };

    if sim.is_booted() {
        ctx.out.note(&format!("{} is already booted", sim.label()));
        return Ok(());
    }
    simctl::boot(&sim.udid)?;
    ctx.out.note(&format!("booted {}", sim.label()));
    Ok(())
}
