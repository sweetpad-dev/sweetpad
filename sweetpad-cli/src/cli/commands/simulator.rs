//! `sweetpad simulator …` — manage iOS simulators (via `xcrun simctl`).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Subcommand, ValueEnum};
use serde::Serialize;

use crate::cli::output::Output;
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered, resolve, simctl};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List available simulators.
    List,
    /// Boot a simulator (by name or UDID; prompts when omitted).
    Boot {
        /// Simulator name or UDID to boot.
        target: Option<String>,
        /// Block until the simulator is fully booted (simctl bootstatus).
        #[arg(long)]
        wait: bool,
    },
    /// Create a new simulator.
    Create {
        /// Name for the new simulator.
        name: String,
        /// Device type (e.g. "iPhone 16 Pro", or a simctl devicetype id).
        device_type: String,
        /// Runtime (e.g. "iOS 18.0"); the newest available when omitted.
        #[arg(long)]
        runtime: Option<String>,
    },
    /// Delete a simulator (no undo).
    Delete {
        /// Simulator name or UDID to delete.
        target: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Clone a shut-down simulator under a new name.
    Clone {
        /// Simulator name or UDID to clone.
        target: String,
        /// Name for the clone.
        new_name: String,
    },
    /// Deliver an APNs push payload (a JSON file) to an app.
    Push {
        /// The app's bundle identifier.
        bundle_id: String,
        /// Path to the payload .json (apns format).
        payload: std::path::PathBuf,
        /// Simulator name or UDID (defaults to the booted one).
        target: Option<String>,
    },
    /// Grant or revoke a privacy permission for an app.
    Privacy {
        /// grant or revoke.
        #[arg(value_parser = ["grant", "revoke", "reset"])]
        action: String,
        /// The service: photos, camera, microphone, location, contacts, calendar, …
        service: String,
        /// The app's bundle identifier.
        bundle_id: String,
        /// Simulator name or UDID (defaults to the booted one).
        target: Option<String>,
    },
    /// Override the status bar for clean screenshots (9:41, full bars), or
    /// clear it.
    StatusBar {
        /// Clear the override instead of setting it.
        #[arg(long)]
        clear: bool,
        /// Simulator name or UDID (defaults to the booted one).
        target: Option<String>,
    },
    /// Set the simulated location.
    Location {
        /// Latitude (e.g. 50.4501).
        latitude: f64,
        /// Longitude (e.g. 30.5234).
        longitude: f64,
        /// Simulator name or UDID (defaults to the booted one).
        target: Option<String>,
    },
    /// Add photos/videos to the media library.
    MediaAdd {
        /// Files to add.
        #[arg(required = true)]
        paths: Vec<std::path::PathBuf>,
        /// Simulator name or UDID (defaults to the booted one).
        #[arg(long)]
        target: Option<String>,
    },
    /// Record the screen to an .mp4 (Ctrl-C stops and finalizes).
    Record {
        /// Output file (default: ./sweetpad-shots/<device>-<time>.mp4).
        #[arg(long)]
        output: Option<PathBuf>,
        /// Simulator name or UDID (defaults to the booted one).
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
        /// File to write the screenshot to (default:
        /// ./sweetpad-shots/<device>-<time>.png).
        #[arg(long)]
        output: Option<PathBuf>,
        /// Also copy the screenshot to the clipboard.
        #[arg(long)]
        clipboard: bool,
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
        Action::Boot { target, wait } => boot(ctx, target.as_deref(), *wait),
        Action::Shutdown { target } => shutdown(ctx, target.as_deref()),
        Action::Erase { target } => erase(ctx, target.as_deref()),
        Action::Open => open(),
        Action::Screenshot {
            target,
            output,
            clipboard,
        } => screenshot(ctx, target.as_deref(), output.as_deref(), *clipboard),
        Action::Appearance { mode, target } => appearance(ctx, *mode, target.as_deref()),
        Action::Create {
            name,
            device_type,
            runtime,
        } => create(name, device_type, runtime.as_deref()),
        Action::Delete { target, yes } => delete(ctx, target, *yes),
        Action::Clone { target, new_name } => clone(ctx, target, new_name),
        Action::Push {
            bundle_id,
            payload,
            target,
        } => {
            let sims = simctl::list()?;
            let sim = resolve::select_simulator(ctx, &sims, target.as_deref())?;
            simctl::push(&sim.udid, bundle_id, &payload.display().to_string())?;
            Ok(Rendered::data(report("pushed payload to", sim)))
        }
        Action::Privacy {
            action,
            service,
            bundle_id,
            target,
        } => {
            let sims = simctl::list()?;
            let sim = resolve::select_simulator(ctx, &sims, target.as_deref())?;
            simctl::privacy(&sim.udid, action, service, bundle_id)?;
            Ok(Rendered::data(report("changed privacy on", sim)))
        }
        Action::StatusBar { clear, target } => {
            let sims = simctl::list()?;
            let sim = resolve::select_simulator(ctx, &sims, target.as_deref())?;
            simctl::status_bar(&sim.udid, *clear)?;
            Ok(Rendered::data(report(
                if *clear {
                    "cleared the status bar on"
                } else {
                    "overrode the status bar on"
                },
                sim,
            )))
        }
        Action::Location {
            latitude,
            longitude,
            target,
        } => {
            let sims = simctl::list()?;
            let sim = resolve::select_simulator(ctx, &sims, target.as_deref())?;
            simctl::location(&sim.udid, *latitude, *longitude)?;
            Ok(Rendered::data(report("set the location on", sim)))
        }
        Action::MediaAdd { paths, target } => {
            let sims = simctl::list()?;
            let sim = resolve::select_simulator(ctx, &sims, target.as_deref())?;
            let paths: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
            simctl::media_add(&sim.udid, &paths)?;
            Ok(Rendered::data(report("added media to", sim)))
        }
        Action::Record { output, target } => record(ctx, target.as_deref(), output.as_deref()),
    }
}

fn create(name: &str, device_type: &str, runtime: Option<&str>) -> CommandResult {
    let udid = simctl::create(name, device_type, runtime)?;
    Ok(Rendered::data(SimAction {
        udid,
        name: name.to_string(),
        action: "created",
        note: format!("created {name}"),
    }))
}

fn delete(ctx: &mut Context, target: &str, yes: bool) -> CommandResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, Some(target))?;
    // Deleting is unrecoverable: confirm interactively, or require --yes.
    if !yes {
        if !ctx.out.is_interactive() {
            return Err(CliError::new(
                "refusing to delete a simulator without confirmation; pass --yes",
            ));
        }
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!("Delete {}?", sim.label()))
            .default(false)
            .interact()
            .map_err(|e| {
                CliError::new(format!("confirmation cancelled: {e}")).kind(ErrorKind::UserCancel)
            })?;
        if !confirmed {
            // A declined prompt exits 6 like an Esc'd one (`help exit-codes`).
            return Err(CliError::new("delete declined").kind(ErrorKind::UserCancel));
        }
    }
    simctl::delete(&sim.udid)?;
    Ok(Rendered::data(report("deleted", sim)))
}

fn clone(ctx: &mut Context, target: &str, new_name: &str) -> CommandResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, Some(target))?;
    let udid = simctl::clone(&sim.udid, new_name)?;
    Ok(Rendered::data(SimAction {
        udid,
        name: new_name.to_string(),
        action: "cloned",
        note: format!("cloned {} as {new_name}", sim.label()),
    }))
}

fn record(ctx: &mut Context, target: Option<&str>, output: Option<&std::path::Path>) -> CommandResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, target)?;
    let path = output.map_or_else(
        || {
            let mut p = default_screenshot_path(&sim.name);
            p.set_extension("mp4");
            p
        },
        std::path::Path::to_path_buf,
    );
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(parent);
    }
    ctx.out
        .note(&format!("recording {} — Ctrl-C to stop", sim.label()));
    let stopped = simctl::record(&sim.udid, &path.display().to_string())?;
    if !path.exists() {
        return Err(crate::cli::CliError::new(
            "recordVideo ended without producing a file",
        ));
    }
    if stopped {
        ctx.out.note("recording stopped");
    }
    Ok(Rendered::data(SimScreenshot {
        udid: sim.udid.clone(),
        path: path.display().to_string(),
        label: sim.label(),
    }))
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

fn boot(ctx: &mut Context, target: Option<&str>, wait: bool) -> CommandResult {
    let sims = simctl::list()?;

    let sim = if let Some(t) = target {
        simctl::find(&sims, t).ok_or_else(|| {
            CliError::new(format!("no simulator matching {t:?}")).kind(ErrorKind::TargetResolution)
        })?
    } else {
        // Disambiguate identical labels (name + OS clones) and map back by
        // position, so the picker can never boot the wrong twin.
        let refs: Vec<&simctl::Simulator> = sims.iter().collect();
        let mut labels: Vec<String> = refs.iter().map(|s| s.label()).collect();
        resolve::disambiguate_labels(&mut labels, &refs);
        let chosen = resolve::choose(ctx, "simulator", None, &labels)?;
        labels
            .iter()
            .position(|l| *l == chosen)
            .map(|i| refs[i])
            .ok_or_else(|| CliError::new("simulator not found").kind(ErrorKind::TargetResolution))?
    };

    if sim.is_booted() {
        return Ok(Rendered::data(SimAction::already(
            sim,
            "already booted",
            format!("{} is already booted", sim.label()),
        )));
    }
    simctl::boot(&sim.udid)?;
    if wait {
        ctx.out
            .step("Waiting for boot to finish", || simctl::boot_wait(&sim.udid))?;
    }
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
    clipboard: bool,
) -> CommandResult {
    let sims = simctl::list()?;
    let sim = resolve::select_simulator(ctx, &sims, target)?;

    let path = output.map_or_else(
        || default_screenshot_path(&sim.name),
        std::path::Path::to_path_buf,
    );
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(parent);
    }
    let path_str = path.display().to_string();
    simctl::screenshot(&sim.udid, &path_str)?;

    if clipboard {
        copy_png_to_clipboard(&path)?;
        ctx.out.note("copied to the clipboard");
    }

    Ok(Rendered::data(SimScreenshot {
        udid: sim.udid.clone(),
        path: path_str,
        label: sim.label(),
    }))
}

/// Put a PNG file on the macOS clipboard via osascript (`«class PNGf»`) — the
/// only pasteboard route that needs no extra dependency.
fn copy_png_to_clipboard(path: &std::path::Path) -> Result<(), CliError> {
    // The path rides in as an argv item (`on run argv`) instead of being
    // spliced into the script source, where a `"` or `\` in the path would
    // break compilation — or execute the path's content as AppleScript.
    let script = "on run argv\n\
                  set the clipboard to (read (POSIX file (item 1 of argv)) as «class PNGf»)\n\
                  end run";
    let path = path.to_string_lossy();
    crate::cli::process::capture("osascript", &["-e", script, path.as_ref()], None)
        .map(|_| ())
        .map_err(|e| e.context("copying the screenshot to the clipboard"))
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

/// `./sweetpad-shots/<device>-<epoch-millis>.png` — one predictable folder
/// instead of screenshots scattering through the working directory. Shared
/// with the run session's `s` key. Millisecond resolution so two quick
/// captures (double-tapping `s`) don't overwrite each other.
pub(crate) fn default_screenshot_path(device_name: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let slug: String = device_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    PathBuf::from("sweetpad-shots").join(format!("{slug}-{millis}.png"))
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
    fn default_screenshot_path_is_slugged_and_timestamped() {
        let path = default_screenshot_path("iPhone 16 Pro");
        assert_eq!(path.parent().unwrap(), std::path::Path::new("sweetpad-shots"));
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("iphone-16-pro-"), "name: {name}");
        assert!(name.ends_with(".png"));
    }
}
