//! Shared host paths for SweetPad's machine-managed state (XDG-style).
//!
//! These mirror the layout the VS Code extension writes from
//! `src/cli-server/paths.ts`, and back the CLI's own `config`/`state` files.
//! Keeping them in one always-compiled module (not behind the `cli` feature)
//! lets the BSP server and the `vscode` client share the discovery index that
//! replaced the old in-project `.sweetpad/` directory.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;

/// `$HOME`, if set and non-empty.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

static STATE_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Pin [`state_dir`] to `dir` for the rest of the process, ahead of
/// `$XDG_STATE_HOME` and `$HOME`. The first call wins.
///
/// For a test harness that reaches sweetpad's state in process, such as the
/// package-members cache: setting `XDG_STATE_HOME` there would race the other
/// tests' threads, so without this the test writes into the user's
/// `~/.local/state/sweetpad`. A spawned child takes `XDG_STATE_HOME` instead.
pub fn set_state_dir_override(dir: PathBuf) {
    let _ = STATE_DIR_OVERRIDE.set(dir);
}

/// The directory [`set_state_dir_override`] pinned, else `$XDG_STATE_HOME`,
/// else `$HOME/.local/state`.
#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    if let Some(dir) = STATE_DIR_OVERRIDE.get() {
        return Some(dir.clone());
    }
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg));
    }
    home_dir().map(|h| h.join(".local").join("state"))
}

/// `<state>/sweetpad` — the root of SweetPad's machine-managed state.
#[must_use]
pub fn sweetpad_state_dir() -> Option<PathBuf> {
    state_dir().map(|d| d.join("sweetpad"))
}

/// The project-discovery index the extension maintains: a map of canonical
/// workspace path → running control server. The `vscode` client reads it to
/// find the control socket for the project it's run inside, the way it used to
/// read `.sweetpad/cli.json`.
#[must_use]
pub fn projects_index_file() -> Option<PathBuf> {
    sweetpad_state_dir().map(|d| d.join("projects.json"))
}

/// The discovery-index entry for the nearest registered ancestor of `start`.
///
/// Walks up from `start` (canonicalized first, so the ancestor spellings match
/// the extension's realpath'd keys), returning the first ancestor present in
/// `projects.json`. This is the shared discovery primitive: the `vscode` client
/// reads the entry's control socket, the BSP server its `bspConfig` path.
///
/// One field of the entry is outside that contract: `control.workspacePath` is
/// diagnostic only. It records the folder the server started for, which is not
/// the project the window is on — in a multi-root workspace the two diverge the
/// moment the user selects a project in another folder. It also does not
/// identify the key it was found under, because a window publishes one identical
/// entry under every one of its workspace folders so the walk-up above succeeds
/// from any of them. Ask the running server (`meta.workspacePath`) when the live
/// answer is what's wanted.
#[must_use]
pub fn lookup_index_entry(start: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(projects_index_file()?).ok()?;
    let index: Value = serde_json::from_str(&text).ok()?;
    let projects = index.get("projects")?.as_object()?;
    let mut dir = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    loop {
        if let Some(entry) = dir.to_str().and_then(|key| projects.get(key)) {
            return Some(entry.clone());
        }
        if !dir.pop() {
            return None;
        }
    }
}
