//! Machine-managed selection state: `~/.local/state/sweetpad/state.toml`.
//!
//! Remembers the user's interactive picks — scheme, configuration, sdk, and
//! destination, plus a separate testing context, recently-used and most-used
//! destinations, and the last launched app — per project so the daily loop
//! doesn't re-prompt. Mirrors the richer context the VS Code extension keeps in
//! its workspace state. Unlike
//! [`crate::cli::config`], this file is freely rewritten by the tool — never
//! hand-author it. Keyed by canonicalized project/workspace path.
//!
//! Honors `XDG_STATE_HOME`, falling back to `~/.local/state`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The whole state file: one [`ProjectState`] per project key.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct State {
    pub projects: BTreeMap<String, ProjectState>,
}

/// Remembered selections for a single project. Scalar fields come first so the
/// nested tables (testing, recents, usage, last-launched) serialize as valid
/// TOML after them; the table fields are skipped when empty so simple entries
/// stay simple.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ProjectState {
    /// The build/run context — what `build` and `app` use.
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub sdk: Option<String>,
    pub destination: Option<String>,

    /// The testing context, kept separate from the build context (mirrors the
    /// extension's `testing.*` keys). `test` reads this, falling back to the
    /// build context where a field is unset.
    #[serde(skip_serializing_if = "TestingState::is_empty")]
    pub testing: TestingState,

    /// Destinations selected before, unique by id, in first-seen order — the
    /// "recent" set. Stored structurally so they survive a device being offline.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub destination_recents: Vec<SelectedDestination>,

    /// How many times each destination (by id) has been selected — drives the
    /// most-used-first picker ordering.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub destination_usage: BTreeMap<String, u32>,

    /// The app launched most recently, for re-launch and inspection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_launched_app: Option<LastLaunchedApp>,
}

impl ProjectState {
    /// Whether nothing is remembered at all — used to drop the entry from the
    /// file once its last field is cleared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scheme.is_none()
            && self.configuration.is_none()
            && self.sdk.is_none()
            && self.destination.is_none()
            && self.testing.is_empty()
            && self.destination_recents.is_empty()
            && self.destination_usage.is_empty()
            && self.last_launched_app.is_none()
    }
}

/// The testing context — the test action's own scheme/configuration/target/
/// destination, independent of the build context.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TestingState {
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub target: Option<String>,
    pub destination: Option<String>,
}

impl TestingState {
    /// Whether nothing is set — used to drop the table from the file entirely.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scheme.is_none()
            && self.configuration.is_none()
            && self.target.is_none()
            && self.destination.is_none()
    }
}

/// A destination remembered structurally (id + kind + display name), so recents
/// and usage stats survive the device being offline. Mirrors the extension's
/// `SelectedDestination`; `id` is the simulator UDID for simulators.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SelectedDestination {
    pub id: String,
    /// Destination kind, e.g. `iOSSimulator` / `watchOSSimulator`.
    pub kind: String,
    pub name: String,
}

/// The most recently launched app, kept for re-launch and for inspection by
/// `context show`. A flat record with a `kind` discriminator (rather than a Rust
/// enum) so it serializes to a single, TOML-clean table.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LastLaunchedApp {
    /// `simulator` | `device` | `macos`.
    pub kind: String,
    pub app_path: String,
    pub bundle_identifier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    /// `CFBundleExecutable` — the process name in os_log (devices).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_name: Option<String>,
    /// Simulator UDID (simulator launches).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulator_udid: Option<String>,
    /// Destination id + kind (device launches).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_type: Option<String>,
}

impl State {
    /// Standard state path, honoring `XDG_STATE_HOME`.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        state_dir().map(|d| d.join("sweetpad").join("state.toml"))
    }

    /// Load remembered state. A missing or unreadable file yields defaults —
    /// state is best-effort and must never block a command.
    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(_) => Ok(Self::default()),
        }
    }

    /// Persist state, creating the parent directory as needed.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Mutable access to a project's remembered selections, inserting an empty
    /// entry if absent.
    pub fn project_mut(&mut self, key: &str) -> &mut ProjectState {
        self.projects.entry(key.to_string()).or_default()
    }
}

/// `$XDG_STATE_HOME` or `$HOME/.local/state`.
fn state_dir() -> Option<PathBuf> {
    sweetpad_core::paths::state_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_via_toml() {
        let mut state = State::default();
        let p = state.project_mut("/work/App.xcodeproj");
        p.scheme = Some("App".into());
        p.destination = Some("platform=iOS Simulator,id=UDID".into());

        let text = toml::to_string_pretty(&state).unwrap();
        let back: State = toml::from_str(&text).unwrap();
        let p = back.projects.get("/work/App.xcodeproj").unwrap();
        assert_eq!(p.scheme.as_deref(), Some("App"));
        assert_eq!(
            p.destination.as_deref(),
            Some("platform=iOS Simulator,id=UDID")
        );
    }

    #[test]
    fn round_trips_the_rich_context() {
        let mut state = State::default();
        let p = state.project_mut("/work/App.xcodeproj");
        p.scheme = Some("App".into());
        p.sdk = Some("iphonesimulator".into());
        p.testing.target = Some("AppTests".into());
        p.testing.destination = Some("platform=iOS Simulator,id=TEST".into());
        p.destination_recents.push(SelectedDestination {
            id: "UDID-1".into(),
            kind: "iOSSimulator".into(),
            name: "iPhone 17".into(),
        });
        p.destination_usage.insert("UDID-1".into(), 3);
        p.last_launched_app = Some(LastLaunchedApp {
            kind: "simulator".into(),
            app_path: "/dd/App.app".into(),
            bundle_identifier: "com.example.App".into(),
            simulator_udid: Some("UDID-1".into()),
            ..Default::default()
        });

        let text = toml::to_string_pretty(&state).unwrap();
        let back: State = toml::from_str(&text).unwrap();
        let p = back.projects.get("/work/App.xcodeproj").unwrap();
        assert_eq!(p.sdk.as_deref(), Some("iphonesimulator"));
        assert_eq!(p.testing.target.as_deref(), Some("AppTests"));
        assert_eq!(p.destination_recents[0].name, "iPhone 17");
        assert_eq!(p.destination_usage.get("UDID-1"), Some(&3));
        assert_eq!(
            p.last_launched_app
                .as_ref()
                .unwrap()
                .simulator_udid
                .as_deref(),
            Some("UDID-1")
        );
    }

    #[test]
    fn empty_rich_fields_are_omitted_from_the_file() {
        let mut state = State::default();
        let p = state.project_mut("/x");
        p.scheme = Some("App".into());
        let text = toml::to_string_pretty(&state).unwrap();
        // A scheme-only entry serializes exactly as before — no empty tables.
        assert!(!text.contains("testing"));
        assert!(!text.contains("destination_recents"));
        assert!(!text.contains("destination_usage"));
        assert!(!text.contains("last_launched_app"));
    }

    #[test]
    fn project_mut_inserts_default() {
        let mut state = State::default();
        assert!(state.projects.is_empty());
        state.project_mut("/x").configuration = Some("Release".into());
        assert_eq!(
            state.projects.get("/x").unwrap().configuration.as_deref(),
            Some("Release")
        );
    }
}
