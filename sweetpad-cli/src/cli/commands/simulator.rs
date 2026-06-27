//! `sweetpad simulator …` — manage iOS simulators (via `xcrun simctl`).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Subcommand, ValueEnum};
use serde::Serialize;

use crate::cli::output::Output;
use crate::cli::{CliError, CommandResult, Context, Render, Rendered, resolve, simctl};

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

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::List => list(),
        Action::Boot { target } => boot(ctx, target.as_deref()),
        Action::Shutdown { target } => shutdown(ctx, target.as_deref()),
        Action::Erase { target } => erase(ctx, target.as_deref()),
        Action::Open => open(),
        Action::Screenshot { target, output } => {
            screenshot(ctx, target.as_deref(), output.as_deref())
        }
        Action::Appearance { mode, target } => appearance(ctx, *mode, target.as_deref()),
    }
}

/// The simulator list: a marked human list, or the `data` of the JSON envelope.
#[derive(Serialize)]
struct SimList {
    simulators: Vec<SimEntry>,
}

#[derive(Serialize)]
struct SimEntry {
    udid: String,
    name: String,
    os: String,
    #[serde(rename = "osVersion")]
    os_version: String,
    state: String,
    booted: bool,
    /// Display label (name + OS version); carried for `human`, not serialized.
    #[serde(skip)]
    label: String,
}

impl Render for SimList {
    fn human(&self, out: &Output) {
        if self.simulators.is_empty() {
            out.note("no simulators available");
            return;
        }
        for s in &self.simulators {
            let marker = if s.booted { " [booted]" } else { "" };
            out.item(
                &format!("{} {}  {}{marker}", s.os, s.label, s.udid),
                s.booted,
            );
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

fn list() -> CommandResult {
    let sims = simctl::list()?;
    let simulators = sims
        .iter()
        .map(|s| SimEntry {
            udid: s.udid.clone(),
            name: s.name.clone(),
            os: s.os.clone(),
            os_version: s.os_version.clone(),
            state: s.state.clone(),
            booted: s.is_booted(),
            label: s.label(),
        })
        .collect();
    Ok(Rendered::data(SimList { simulators }))
}

fn boot(ctx: &mut Context, target: Option<&str>) -> CommandResult {
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
        return Ok(Rendered::data(SimAction::already(
            sim,
            "already booted",
            format!("{} is already booted", sim.label()),
        )));
    }
    simctl::boot(&sim.udid)?;
    Ok(Rendered::data(report("booted", sim)))
}

fn shutdown(ctx: &mut Context, target: Option<&str>) -> CommandResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, target)?;
    if !sim.is_booted() {
        return Ok(Rendered::data(SimAction::already(
            sim,
            "already shut down",
            format!("{} is already shut down", sim.label()),
        )));
    }
    simctl::shutdown(&sim.udid)?;
    Ok(Rendered::data(report("shut down", sim)))
}

fn erase(ctx: &mut Context, target: Option<&str>) -> CommandResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, target)?;
    if sim.is_booted() {
        return Err(CliError::new(format!(
            "{} is booted; shut it down first (`sweetpad simulator shutdown`)",
            sim.label()
        )));
    }
    simctl::erase(&sim.udid)?;
    Ok(Rendered::data(report("erased", sim)))
}

fn open() -> CommandResult {
    simctl::open_app()?;
    Ok(Rendered::data(SimOpen))
}

fn screenshot(
    ctx: &mut Context,
    target: Option<&str>,
    output: Option<&std::path::Path>,
) -> CommandResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, target)?;

    let path = output.map_or_else(default_screenshot_path, std::path::Path::to_path_buf);
    let path_str = path.display().to_string();
    simctl::screenshot(&sim.udid, &path_str)?;

    Ok(Rendered::data(SimScreenshot {
        udid: sim.udid.clone(),
        path: path_str,
        label: sim.label(),
    }))
}

fn appearance(ctx: &mut Context, mode: Appearance, target: Option<&str>) -> CommandResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, target)?;
    simctl::set_appearance(&sim.udid, mode.as_str())?;
    Ok(Rendered::data(SimAppearance {
        udid: sim.udid.clone(),
        appearance: mode.as_str(),
        label: sim.label(),
    }))
}

/// A completed side-effecting action on a simulator (`boot`/`shutdown`/`erase`,
/// or an early no-op). Renders as a small JSON object (`{udid, name, action}`),
/// or a human note otherwise.
struct SimAction {
    udid: String,
    name: String,
    action: &'static str,
    /// The human note to print; carried so the no-op paths keep their wording.
    note: String,
}

impl SimAction {
    /// An early no-op (already booted / already shut down): the same JSON shape
    /// with a status `action`, and the original note for human mode.
    fn already(sim: &simctl::Simulator, action: &'static str, note: String) -> Self {
        Self {
            udid: sim.udid.clone(),
            name: sim.name.clone(),
            action,
            note,
        }
    }
}

impl Render for SimAction {
    fn human(&self, out: &Output) {
        out.note(&self.note);
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "udid": self.udid,
            "name": self.name,
            "action": self.action,
        })
    }
}

/// Build a `SimAction` for a completed action: `{udid, name, action: verb}` JSON,
/// `"<verb> <label>"` note.
fn report(verb: &'static str, sim: &simctl::Simulator) -> SimAction {
    SimAction {
        udid: sim.udid.clone(),
        name: sim.name.clone(),
        action: verb,
        note: format!("{verb} {}", sim.label()),
    }
}

/// `simulator open`: opened the GUI. `{opened: true}` / "opened Simulator.app".
struct SimOpen;

impl Render for SimOpen {
    fn human(&self, out: &Output) {
        out.note("opened Simulator.app");
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({ "opened": true })
    }
}

/// `simulator screenshot`: `{udid, path}` / a saved note.
struct SimScreenshot {
    udid: String,
    path: String,
    /// Display label for the human note; not serialized.
    label: String,
}

impl Render for SimScreenshot {
    fn human(&self, out: &Output) {
        out.note(&format!(
            "saved screenshot of {} to {}",
            self.label, self.path
        ));
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "udid": self.udid,
            "path": self.path,
        })
    }
}

/// `simulator appearance`: `{udid, appearance}` / a set note.
struct SimAppearance {
    udid: String,
    appearance: &'static str,
    /// Display label for the human note; not serialized.
    label: String,
}

impl Render for SimAppearance {
    fn human(&self, out: &Output) {
        out.note(&format!(
            "set {} appearance to {}",
            self.label, self.appearance
        ));
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "udid": self.udid,
            "appearance": self.appearance,
        })
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
