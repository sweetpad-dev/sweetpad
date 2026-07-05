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
    /// Set when the on-disk file exists but couldn't be *read* (permissions,
    /// I/O error): the in-memory state is then a guess, and [`save`](State::save)
    /// becomes a no-op — writing would replace every project's remembered
    /// context with near-emptiness.
    #[serde(skip)]
    pub read_only: bool,
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

    /// Named destination aliases (`context alias work-phone <UDID>`), resolved
    /// by `--on NAME`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub destination_aliases: BTreeMap<String, String>,

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
            && self.destination_aliases.is_empty()
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

    /// Load remembered state, preserving what's on disk against both failure
    /// modes:
    ///
    /// - a file that exists but can't be **read** (permissions, I/O error)
    ///   yields defaults flagged [`read_only`](State::read_only) — saves are
    ///   skipped for this run, so the unreadable file survives intact;
    /// - a file that reads but doesn't **parse** is moved aside to a unique
    ///   `state.toml.corrupt[.N]` backup and a warning names both paths.
    ///
    /// Without either guard, the next best-effort [`save`](State::save) would
    /// rewrite the whole file from the near-empty in-memory state — every
    /// project's remembered context lost without a trace.
    #[must_use]
    pub fn load_or_quarantine() -> (Self, Option<String>) {
        let Some(path) = Self::path() else {
            return (Self::default(), None);
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (Self::default(), None);
            }
            Err(e) => {
                let state = Self {
                    read_only: true,
                    ..Self::default()
                };
                return (
                    state,
                    Some(format!(
                        "cannot read the state file {} ({e}); continuing without remembered \
                         context, and skipping state saves so it isn't overwritten",
                        path.display()
                    )),
                );
            }
        };
        match toml::from_str(&text) {
            Ok(state) => (state, None),
            Err(parse_err) => {
                let backup = quarantine_path(&path);
                let warning = match std::fs::rename(&path, &backup) {
                    Ok(()) => format!(
                        "state file is corrupt ({}: {parse_err}); moved it to {} and starting fresh",
                        path.display(),
                        backup.display()
                    ),
                    Err(_) => format!(
                        "state file is corrupt and will be overwritten on the next save: {}: {parse_err}",
                        path.display()
                    ),
                };
                (Self::default(), Some(warning))
            }
        }
    }

    /// Persist state, creating the parent directory as needed. Written to a
    /// temp file and renamed into place, so a crash or a concurrent session
    /// can never leave a torn half-written `state.toml` (concurrent saves are
    /// last-writer-wins with each writer's *complete* file). What lands on disk
    /// is the [`pruned_view`](State::pruned_view), so the file can't grow
    /// forever.
    pub fn save(&self) -> Result<(), String> {
        // An unreadable-on-load file must never be replaced by this run's
        // near-empty view (see `read_only`).
        if self.read_only {
            return Ok(());
        }
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let text = toml::to_string_pretty(&self.pruned_view()).map_err(|e| e.to_string())?;
        // PID alone distinguishes processes; the counter distinguishes saves
        // within this process, so two same-process saves can never interleave
        // writes into one temp file and rename a torn mix into place.
        static SAVE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SAVE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!("toml.tmp.{}.{seq}", std::process::id()));
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The save-time view: entries for deleted projects dropped, recents capped
    /// at [`MAX_RECENTS`] (oldest first out), and usage stats pruned to the
    /// remembered destinations. "Deleted" means the container vanished while
    /// its parent directory remains — a missing parent (an unmounted volume, a
    /// temporarily absent checkout) keeps the entry, so ejecting a disk never
    /// loses a project's context.
    fn pruned_view(&self) -> State {
        let projects = self
            .projects
            .iter()
            .filter(|(key, _)| !project_deleted(std::path::Path::new(key)))
            .map(|(key, entry)| {
                let mut entry = entry.clone();
                if entry.destination_recents.len() > MAX_RECENTS {
                    let excess = entry.destination_recents.len() - MAX_RECENTS;
                    entry.destination_recents.drain(..excess);
                }
                let keep: std::collections::BTreeSet<String> = entry
                    .destination_recents
                    .iter()
                    .map(|d| d.id.clone())
                    .collect();
                entry.destination_usage.retain(|id, _| keep.contains(id));
                (key.clone(), entry)
            })
            .collect();
        State {
            projects,
            read_only: self.read_only,
        }
    }

    /// Mutable access to a project's remembered selections, inserting an empty
    /// entry if absent.
    pub fn project_mut(&mut self, key: &str) -> &mut ProjectState {
        self.projects.entry(key.to_string()).or_default()
    }
}

/// How many destination recents (and their usage entries) a project keeps.
const MAX_RECENTS: usize = 12;

/// A quarantine path that never clobbers an earlier backup: `state.toml.corrupt`,
/// then `.corrupt.1`, `.corrupt.2`, … — the first backup may hold the only
/// surviving copy of the remembered context.
fn quarantine_path(path: &std::path::Path) -> PathBuf {
    let base = path.with_extension("toml.corrupt");
    if !base.exists() {
        return base;
    }
    for n in 1..100 {
        let candidate = path.with_extension(format!("toml.corrupt.{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // 100 backups already exist (a recurring corruption source). Returning
    // `base` here would rename over the oldest — possibly only intact —
    // snapshot; a pid-suffixed name stays collision-free instead.
    path.with_extension(format!("toml.corrupt.pid{}", std::process::id()))
}

/// Whether a state key points at a deleted container: the path is gone but its
/// parent directory still exists.
fn project_deleted(path: &std::path::Path) -> bool {
    !path.exists() && path.parent().is_some_and(std::path::Path::exists)
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
    fn pruned_view_drops_deleted_projects_but_keeps_unmounted_ones() {
        let dir = std::env::temp_dir().join(format!("sweetpad-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = State::default();
        // Parent exists, container doesn't → deleted → pruned.
        let deleted = dir.join("Gone.xcodeproj");
        state.project_mut(&deleted.to_string_lossy()).scheme = Some("X".into());
        // Neither parent nor container exists (unmounted volume) → kept.
        let unmounted = "/Volumes/nonexistent-sweetpad-test/App.xcodeproj";
        state.project_mut(unmounted).scheme = Some("Y".into());
        // Container exists → kept.
        let live = dir.join("Live.xcodeproj");
        std::fs::create_dir_all(&live).unwrap();
        state.project_mut(&live.to_string_lossy()).scheme = Some("Z".into());

        let pruned = state.pruned_view();
        assert!(!pruned.projects.contains_key(&*deleted.to_string_lossy()));
        assert!(pruned.projects.contains_key(unmounted));
        assert!(pruned.projects.contains_key(&*live.to_string_lossy()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pruned_view_caps_recents_and_prunes_usage() {
        let mut state = State::default();
        let p = state.project_mut("/Volumes/nonexistent-sweetpad-test/App.xcodeproj");
        for i in 0..20 {
            let id = format!("UDID-{i}");
            p.destination_recents.push(SelectedDestination {
                id: id.clone(),
                kind: "iOSSimulator".into(),
                name: format!("Sim {i}"),
            });
            p.destination_usage.insert(id, i);
        }
        let pruned = state.pruned_view();
        let entry = pruned
            .projects
            .get("/Volumes/nonexistent-sweetpad-test/App.xcodeproj")
            .unwrap();
        assert_eq!(entry.destination_recents.len(), MAX_RECENTS);
        // Oldest dropped first: the newest ids survive.
        assert_eq!(entry.destination_recents[0].id, "UDID-8");
        // Usage entries follow the surviving recents.
        assert_eq!(entry.destination_usage.len(), MAX_RECENTS);
        assert!(!entry.destination_usage.contains_key("UDID-0"));
        assert!(entry.destination_usage.contains_key("UDID-19"));
    }

    #[test]
    fn read_only_state_never_saves() {
        let mut state = State {
            read_only: true,
            ..State::default()
        };
        state.project_mut("/x").scheme = Some("App".into());
        // No path is touched: read_only short-circuits before Self::path().
        assert!(state.save().is_ok());
    }

    #[test]
    fn quarantine_paths_never_clobber_an_earlier_backup() {
        let dir = std::env::temp_dir().join(format!("sweetpad-quarantine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.toml");
        let first = quarantine_path(&path);
        assert!(first.to_string_lossy().ends_with("state.toml.corrupt"));
        std::fs::write(&first, "old backup").unwrap();
        let second = quarantine_path(&path);
        assert!(second.to_string_lossy().ends_with("state.toml.corrupt.1"));
        assert_ne!(first, second);
        std::fs::remove_dir_all(&dir).unwrap();
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
