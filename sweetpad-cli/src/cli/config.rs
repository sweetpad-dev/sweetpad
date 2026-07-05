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
    /// Lint findings from [`load`](Config::load): unknown keys (typos parse
    /// cleanly and are silently ignored otherwise) and `[projects."…"]` tables
    /// whose key can't match a real container. Surfaced as warnings by the
    /// dispatcher; never serialized.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

/// The override knobs, shared by the global `[defaults]` table and each
/// `[projects."…"]` table.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct Defaults {
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
    /// SDK for builds and `settings show` (e.g. `iphonesimulator`). Rarely
    /// needed — the destination usually implies it.
    pub sdk: Option<String>,
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
    /// Default test target: `test run` narrows to `-only-testing:<target>`
    /// when no explicit `--only-testing` selector is given.
    pub target: Option<String>,
}

impl Config {
    /// Standard config path, honoring `XDG_CONFIG_HOME`.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("sweetpad").join("config.toml"))
    }

    /// Load and parse the config file. A missing file is not an error
    /// (returns defaults); a malformed file is. Unknown keys and dead
    /// `[projects."…"]` keys parse cleanly but are collected into
    /// [`warnings`](Config::warnings) so a typo (`schme = "App"`, `[default]`)
    /// isn't silently ignored.
    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Parse config text, linting for unknown keys and dead project keys.
    fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut cfg: Config = toml::from_str(text)?;
        // Lint against the raw document so unknown keys are visible (serde has
        // already dropped them from the typed struct).
        if let Ok(raw) = toml::from_str::<toml::Value>(text) {
            cfg.warnings = lint(&raw);
        }
        cfg.warnings
            .extend(cfg.projects.keys().filter_map(|k| lint_project_key(k)));
        Ok(cfg)
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
            layer(&mut merged.sdk, over.sdk.as_ref());
            layer(&mut merged.testing.scheme, over.testing.scheme.as_ref());
            layer(
                &mut merged.testing.configuration,
                over.testing.configuration.as_ref(),
            );
            layer(
                &mut merged.testing.destination,
                over.testing.destination.as_ref(),
            );
            layer(&mut merged.testing.target, over.testing.target.as_ref());
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

/// The keys a `[defaults]` / `[projects."…"]` table accepts.
const DEFAULTS_KEYS: [&str; 5] = ["scheme", "configuration", "destination", "sdk", "testing"];
/// The keys a `[….testing]` sub-table accepts.
const TESTING_KEYS: [&str; 4] = ["scheme", "configuration", "destination", "target"];

/// Walk the raw document and report every key serde would silently drop.
fn lint(raw: &toml::Value) -> Vec<String> {
    let mut warnings = Vec::new();
    let Some(top) = raw.as_table() else {
        return warnings;
    };
    for (key, value) in top {
        match key.as_str() {
            "defaults" => lint_defaults(value, "[defaults]", &mut warnings),
            "projects" => {
                if let Some(projects) = value.as_table() {
                    for (proj, table) in projects {
                        lint_defaults(table, &format!("[projects.\"{proj}\"]"), &mut warnings);
                    }
                }
            }
            other => warnings.push(format!(
                "config: unknown key `{other}` (did you mean `defaults` or `projects`?)"
            )),
        }
    }
    warnings
}

/// Lint one defaults-shaped table (and its `testing` sub-table) at `at`.
fn lint_defaults(value: &toml::Value, at: &str, warnings: &mut Vec<String>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, sub) in table {
        if key == "testing" {
            if let Some(testing) = sub.as_table() {
                for tkey in testing.keys() {
                    if !TESTING_KEYS.contains(&tkey.as_str()) {
                        warnings.push(format!("config: unknown key `{tkey}` in {at} testing"));
                    }
                }
            }
        } else if !DEFAULTS_KEYS.contains(&key.as_str()) {
            let hint = suggest(key, &DEFAULTS_KEYS)
                .map(|s| format!(" (did you mean `{s}`?)"))
                .unwrap_or_default();
            warnings.push(format!("config: unknown key `{key}` in {at}{hint}"));
        }
    }
}

/// A `[projects."…"]` key that can't match: the key must be the canonicalized
/// **container** path (`/…/App.xcodeproj`), not the directory holding it — a
/// directory key parses cleanly and then silently never applies.
fn lint_project_key(key: &str) -> Option<String> {
    let path = std::path::Path::new(key);
    if is_container_path(path) {
        // Right shape — flag it only if the canonical spelling differs (a
        // symlinked or non-canonical path never matches the resolver's key).
        let canonical = std::fs::canonicalize(path).ok()?;
        (canonical.as_path() != path).then(|| {
            format!(
                "config: [projects.\"{key}\"] won't match — keys are canonicalized paths; use \"{}\"",
                canonical.display()
            )
        })
    } else if path.is_dir() {
        // The doc-example mistake: keyed by the project *directory*.
        let suggestion = crate::cli::resolve::discover(path)
            .map(|c| c.key())
            .map_or_else(String::new, |k| format!("; did you mean \"{k}\"?"));
        Some(format!(
            "config: [projects.\"{key}\"] matches no project — keys are the \
             canonicalized container path (the .xcworkspace/.xcodeproj/Package.swift \
             itself){suggestion}"
        ))
    } else {
        Some(format!(
            "config: [projects.\"{key}\"] matches no project on disk — keys are the \
             canonicalized container path"
        ))
    }
}

/// Whether a path names a container (workspace/project/manifest), existing or not.
fn is_container_path(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("xcworkspace" | "xcodeproj")
    ) || path.file_name().and_then(|f| f.to_str()) == Some("Package.swift")
}

/// The closest known key within an edit distance of 2, for did-you-mean hints.
fn suggest<'a>(unknown: &str, known: &[&'a str]) -> Option<&'a str> {
    known
        .iter()
        .map(|k| (edit_distance(unknown, k), *k))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, k)| k)
}

/// Levenshtein distance, small-string sized (config keys).
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            row.push((prev[j] + cost).min(prev[j + 1] + 1).min(row[j] + 1));
        }
        prev = row;
    }
    prev[b.len()]
}

/// A committed, hand-authored `sweetpad.toml` at the project root — how a team
/// shares defaults (`~/.config/sweetpad/config.toml` stays personal). Read-only
/// like the user config; its precedence slot is between the user config and
/// remembered state. Besides the targeting keys it houses the tool defaults
/// that previously had flags but no config: `[run] hot`/`hot_recompiler` and
/// `[format] tool`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ProjectFile {
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
    pub sdk: Option<String>,
    /// Pin the Xcode used for this project (sets DEVELOPER_DIR).
    pub developer_dir: Option<String>,
    pub testing: TestingDefaults,
    pub run: RunDefaults,
    pub format: FormatDefaults,
}

/// `[run]` — `app run` defaults for this project.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct RunDefaults {
    /// Default `app run` to hot reload (`--no-hot` opts out per run).
    pub hot: Option<bool>,
    /// Default hot-reload recompiler: `resolver` or `buildlog`.
    pub hot_recompiler: Option<String>,
}

/// `[format]` — `format` defaults for this project.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct FormatDefaults {
    /// Default formatter: `swift-format` or `swiftlint`.
    pub tool: Option<String>,
}

impl ProjectFile {
    /// Load the `sweetpad.toml` next to `container_dir` (the container's
    /// parent). Missing file ⇒ defaults. A malformed or typo'd file is
    /// *warned about* rather than fatal — a broken committed file must not
    /// brick every teammate's CLI — and the warnings ride back for the caller
    /// to surface once.
    #[must_use]
    pub fn load_for(container_dir: &std::path::Path) -> (Self, Vec<String>) {
        let path = container_dir.join("sweetpad.toml");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            // Only a *missing* file is silently defaults. A present-but-
            // unreadable committed file (permissions, I/O error on a network
            // mount) must warn like a malformed one — silently dropping the
            // team's pinned defaults would build with the wrong Xcode.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (Self::default(), Vec::new());
            }
            Err(e) => {
                return (
                    Self::default(),
                    vec![format!("{}: {e} (file ignored)", path.display())],
                );
            }
        };
        match toml::from_str::<ProjectFile>(&text) {
            Ok(pf) => {
                let mut warnings = Vec::new();
                if let Ok(raw) = toml::from_str::<toml::Value>(&text) {
                    lint_project_file(&raw, &mut warnings);
                }
                (pf, warnings)
            }
            Err(e) => (
                Self::default(),
                vec![format!("{}: {e} (file ignored)", path.display())],
            ),
        }
    }
}

/// The keys a `sweetpad.toml` accepts at the top level.
const PROJECT_FILE_KEYS: [&str; 8] = [
    "scheme",
    "configuration",
    "destination",
    "sdk",
    "developer_dir",
    "testing",
    "run",
    "format",
];

/// Report every `sweetpad.toml` key serde would silently drop.
fn lint_project_file(raw: &toml::Value, warnings: &mut Vec<String>) {
    let Some(top) = raw.as_table() else { return };
    for (key, value) in top {
        match key.as_str() {
            "testing" => {
                if let Some(t) = value.as_table() {
                    for tkey in t.keys() {
                        if !TESTING_KEYS.contains(&tkey.as_str()) {
                            warnings
                                .push(format!("sweetpad.toml: unknown key `{tkey}` in [testing]"));
                        }
                    }
                }
            }
            "run" => {
                if let Some(t) = value.as_table() {
                    for rkey in t.keys() {
                        if !["hot", "hot_recompiler"].contains(&rkey.as_str()) {
                            warnings.push(format!("sweetpad.toml: unknown key `{rkey}` in [run]"));
                        }
                    }
                }
            }
            "format" => {
                if let Some(t) = value.as_table() {
                    for fkey in t.keys() {
                        if fkey != "tool" {
                            warnings
                                .push(format!("sweetpad.toml: unknown key `{fkey}` in [format]"));
                        }
                    }
                }
            }
            other if !PROJECT_FILE_KEYS.contains(&other) => {
                let hint = suggest(other, &PROJECT_FILE_KEYS)
                    .map(|s| format!(" (did you mean `{s}`?)"))
                    .unwrap_or_default();
                warnings.push(format!("sweetpad.toml: unknown key `{other}`{hint}"));
            }
            _ => {}
        }
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
    sweetpad_core::paths::home_dir()
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
    fn unknown_keys_are_warned_not_ignored() {
        // `[default]` instead of `[defaults]`, and a `schme` typo — both parse
        // cleanly (serde drops them), so the lint is the only signal.
        let cfg = Config::parse("[default]\nscheme = \"App\"\n").unwrap();
        assert!(cfg.warnings.iter().any(|w| w.contains("`default`")));

        let cfg = Config::parse("[defaults]\nschme = \"App\"\n").unwrap();
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("`schme`") && w.contains("did you mean `scheme`?")),
            "warnings: {:?}",
            cfg.warnings
        );

        let cfg = Config::parse("[defaults.testing]\nsdk = \"x\"\n").unwrap();
        assert!(cfg.warnings.iter().any(|w| w.contains("`sdk`")));

        // A clean config produces no warnings.
        let cfg = Config::parse("[defaults]\nscheme = \"App\"\n").unwrap();
        assert!(cfg.warnings.is_empty(), "warnings: {:?}", cfg.warnings);
    }

    #[test]
    fn project_key_that_is_a_directory_is_warned() {
        // The CLI_DESIGN doc-example mistake: keying by the project's directory
        // instead of the container path. It parses, then silently never matches.
        let dir = std::env::temp_dir().join(format!("sweetpad-cfg-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("App.xcodeproj")).unwrap();
        let text = format!("[projects.\"{}\"]\nscheme = \"App\"\n", dir.display());
        let cfg = Config::parse(&text).unwrap();
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("matches no project") && w.contains("App.xcodeproj")),
            "warnings: {:?}",
            cfg.warnings
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn container_shaped_project_keys_are_accepted() {
        // A key with the right shape (even if absent on this machine's disk at
        // lint time it canonicalize-fails → no crash, no false "matches no
        // project on disk" for the container-shaped case).
        let dir = std::env::temp_dir().join(format!("sweetpad-cfg2-{}", std::process::id()));
        let proj = dir.join("App.xcodeproj");
        std::fs::create_dir_all(&proj).unwrap();
        let key = std::fs::canonicalize(&proj).unwrap();
        let text = format!("[projects.\"{}\"]\nscheme = \"App\"\n", key.display());
        let cfg = Config::parse(&text).unwrap();
        assert!(cfg.warnings.is_empty(), "warnings: {:?}", cfg.warnings);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn edit_distance_powers_did_you_mean() {
        assert_eq!(edit_distance("schme", "scheme"), 1);
        assert_eq!(edit_distance("scheme", "scheme"), 0);
        assert!(edit_distance("destination", "scheme") > 2);
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
