//! Machine-managed selection state: `~/.local/state/sweetpad/state.toml`.
//!
//! Remembers the user's interactive picks (last scheme, configuration,
//! destination) per project so the daily loop doesn't re-prompt. Unlike
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

/// Remembered selections for a single project.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ProjectState {
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
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
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg));
    }
    super::config::home_dir().map(|h| h.join(".local").join("state"))
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
        assert_eq!(p.destination.as_deref(), Some("platform=iOS Simulator,id=UDID"));
    }

    #[test]
    fn project_mut_inserts_default() {
        let mut state = State::default();
        assert!(state.projects.is_empty());
        state.project_mut("/x").configuration = Some("Release".into());
        assert_eq!(state.projects.get("/x").unwrap().configuration.as_deref(), Some("Release"));
    }
}
