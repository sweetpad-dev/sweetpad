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
    /// Test-action overrides, layered over the build defaults for `test` only
    /// (e.g. a `TEST-Debug` configuration while the app builds `UAT-Debug`).
    pub testing: TestingDefaults,
}

/// Test-action config overrides — the `[…​.testing]` sub-table. Mirrors the
/// extension's `sweetpad.testing.*` settings. Unset fields fall back to the
/// build defaults during test resolution.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct TestingDefaults {
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
            layer(&mut merged.scheme, over.scheme.as_ref());
            layer(&mut merged.configuration, over.configuration.as_ref());
            layer(&mut merged.destination, over.destination.as_ref());
            layer(&mut merged.testing.scheme, over.testing.scheme.as_ref());
            layer(
                &mut merged.testing.configuration,
                over.testing.configuration.as_ref(),
            );
            layer(
                &mut merged.testing.destination,
                over.testing.destination.as_ref(),
            );
        }
        merged
    }
}

/// Overlay a per-project override onto a base value, keeping the base when the
/// override is unset.
fn layer(base: &mut Option<String>, over: Option<&String>) {
    if let Some(v) = over {
        *base = Some(v.clone());
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
    crate::paths::home_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_project_layers_overrides_on_defaults() {
        let cfg: Config = toml::from_str(
            r#"
            [defaults]
            configuration = "Debug"
            scheme = "Global"

            [projects."/work/App.xcodeproj"]
            scheme = "App"
            destination = "platform=iOS Simulator,name=iPhone 15"
            "#,
        )
        .unwrap();

        let d = cfg.for_project("/work/App.xcodeproj");
        // Per-project scheme/destination win; configuration falls through.
        assert_eq!(d.scheme.as_deref(), Some("App"));
        assert_eq!(d.configuration.as_deref(), Some("Debug"));
        assert_eq!(
            d.destination.as_deref(),
            Some("platform=iOS Simulator,name=iPhone 15")
        );

        // Unknown project gets only the global defaults.
        let other = cfg.for_project("/other");
        assert_eq!(other.scheme.as_deref(), Some("Global"));
        assert_eq!(other.destination, None);
    }

    #[test]
    fn empty_config_is_default() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.projects.is_empty());
        assert_eq!(cfg.for_project("/x").scheme, None);
    }

    #[test]
    fn testing_section_layers_separately_from_build() {
        // The #219 setup: build on UAT-Debug, test on TEST-Debug.
        let cfg: Config = toml::from_str(
            r#"
            [projects."/work/App.xcodeproj"]
            configuration = "UAT-Debug"
            [projects."/work/App.xcodeproj".testing]
            configuration = "TEST-Debug"
            scheme = "AppTests"
            "#,
        )
        .unwrap();

        let d = cfg.for_project("/work/App.xcodeproj");
        assert_eq!(d.configuration.as_deref(), Some("UAT-Debug"));
        assert_eq!(d.testing.configuration.as_deref(), Some("TEST-Debug"));
        assert_eq!(d.testing.scheme.as_deref(), Some("AppTests"));
        // A testing field left unset stays None (test resolution falls back to build).
        assert_eq!(d.testing.destination, None);
    }
}
