//! Hand-authored configuration: `~/.config/sweetpad/config.toml`.
//!
//! Global settings plus optional per-project overrides keyed by canonicalized
//! project path. **The tool only ever reads this file** — it never rewrites it,
//! so user comments and formatting are preserved. Machine-written remembered
//! selections live separately in [`crate::cli::state`].
//!
//! Honors `XDG_CONFIG_HOME`, falling back to `~/.config`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

/// Parsed `config.toml`. Missing file ⇒ [`Config::default`] (all empty).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Global defaults applied to every project unless overridden.
    pub defaults: Defaults,
    /// Per-project overrides, keyed by absolute project/workspace path.
    pub projects: BTreeMap<String, Defaults>,
}

/// The override knobs, shared by the global `[defaults]` table and each
/// `[projects."…"]` table.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct Defaults {
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
}

impl Config {
    /// Standard config path, honoring `XDG_CONFIG_HOME`.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("sweetpad").join("config.toml"))
    }

    /// Load and parse the config file. A missing file is not an error
    /// (returns defaults); a malformed file is.
    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Effective defaults for `project_key`: per-project overrides layered on
    /// top of the global defaults.
    #[must_use]
    pub fn for_project(&self, project_key: &str) -> Defaults {
        let mut merged = self.defaults.clone();
        if let Some(over) = self.projects.get(project_key) {
            if over.scheme.is_some() {
                merged.scheme.clone_from(&over.scheme);
            }
            if over.configuration.is_some() {
                merged.configuration.clone_from(&over.configuration);
            }
            if over.destination.is_some() {
                merged.destination.clone_from(&over.destination);
            }
        }
        merged
    }
}

/// `$XDG_CONFIG_HOME` or `$HOME/.config`.
fn config_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg));
    }
    home_dir().map(|h| h.join(".config"))
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}
