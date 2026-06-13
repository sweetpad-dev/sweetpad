//! `sweetpad simulator …` — manage iOS simulators (via `xcrun simctl`).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Subcommand, ValueEnum};

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
    /// Shut down a simulator (defaults to the booted one).
    Shutdown {
        /// Simulator name or UDID to shut down.
        target: Option<String>,
    },
    /// Erase a simulator's contents and settings (it must be shut down).
    Erase {
        /// Simulator name or UDID to erase.
        target: Option<String>,
    },
    /// Open the Simulator.app GUI.
    Open,
    /// Save a PNG screenshot of a booted simulator.
    Screenshot {
        /// Simulator name or UDID to capture (defaults to the booted one).
        target: Option<String>,
        /// File to write the screenshot to (defaults to a timestamped PNG).
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Override a booted simulator's light/dark appearance.
    Appearance {
        /// The appearance to switch to.
        mode: Appearance,
        /// Simulator name or UDID to change (defaults to the booted one).
        target: Option<String>,
    },
}

/// The two UI appearances `simctl ui … appearance` accepts.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    fn as_str(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::List => list(ctx),
        Action::Boot { target } => boot(ctx, target.as_deref()),
        Action::Shutdown { target } => shutdown(ctx, target.as_deref()),
        Action::Erase { target } => erase(ctx, target.as_deref()),
        Action::Open => open(ctx),
        Action::Screenshot { target, output } => {
            screenshot(ctx, target.as_deref(), output.as_deref())
        }
        Action::Appearance { mode, target } => appearance(ctx, *mode, target.as_deref()),
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
    report(ctx, "booted", sim);
    Ok(())
}

fn shutdown(ctx: &mut Context, target: Option<&str>) -> CliResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, target)?;
    if !sim.is_booted() {
        ctx.out
            .note(&format!("{} is already shut down", sim.label()));
        return Ok(());
    }
    simctl::shutdown(&sim.udid)?;
    report(ctx, "shut down", sim);
    Ok(())
}

fn erase(ctx: &mut Context, target: Option<&str>) -> CliResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, target)?;
    if sim.is_booted() {
        return Err(CliError::new(format!(
            "{} is booted; shut it down first (`sweetpad simulator shutdown`)",
            sim.label()
        )));
    }
    simctl::erase(&sim.udid)?;
    report(ctx, "erased", sim);
    Ok(())
}

fn open(ctx: &mut Context) -> CliResult {
    simctl::open_app()?;
    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({ "opened": true }));
    } else {
        ctx.out.note("opened Simulator.app");
    }
    Ok(())
}

fn screenshot(
    ctx: &mut Context,
    target: Option<&str>,
    output: Option<&std::path::Path>,
) -> CliResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, target)?;

    let path = output.map_or_else(default_screenshot_path, std::path::Path::to_path_buf);
    let path_str = path.display().to_string();
    simctl::screenshot(&sim.udid, &path_str)?;

    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({
            "udid": sim.udid,
            "path": path_str,
        }));
    } else {
        ctx.out.note(&format!(
            "saved screenshot of {} to {path_str}",
            sim.label()
        ));
    }
    Ok(())
}

fn appearance(ctx: &mut Context, mode: Appearance, target: Option<&str>) -> CliResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, target)?;
    simctl::set_appearance(&sim.udid, mode.as_str())?;
    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({
            "udid": sim.udid,
            "appearance": mode.as_str(),
        }));
    } else {
        ctx.out.note(&format!(
            "set {} appearance to {}",
            sim.label(),
            mode.as_str()
        ));
    }
    Ok(())
}

/// Report a completed side-effecting action: a small JSON object under `--json`,
/// a human note otherwise.
fn report(ctx: &Context, verb: &str, sim: &simctl::Simulator) {
    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({
            "udid": sim.udid,
            "name": sim.name,
            "action": verb,
        }));
    } else {
        ctx.out.note(&format!("{verb} {}", sim.label()));
    }
}

/// `simulator-screenshot-<epoch-secs>.png` in the working directory.
fn default_screenshot_path() -> PathBuf {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    PathBuf::from(format!("simulator-screenshot-{secs}.png"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_maps_to_simctl_value() {
        assert_eq!(Appearance::Light.as_str(), "light");
        assert_eq!(Appearance::Dark.as_str(), "dark");
    }

    #[test]
    fn default_screenshot_path_is_timestamped_png() {
        let path = default_screenshot_path();
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("simulator-screenshot-"));
        assert!(name.ends_with(".png"));
    }
}
