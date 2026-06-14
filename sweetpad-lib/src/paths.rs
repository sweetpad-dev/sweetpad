//! Shared host paths for SweetPad's machine-managed state (XDG-style).
//!
//! These mirror the layout the VS Code extension writes from
//! `src/cli-server/paths.ts`, and back the CLI's own `config`/`state` files.
//! Keeping them in one always-compiled module (not behind the `cli` feature)
//! lets the BSP server and the `vscode` client share the discovery index that
//! replaced the old in-project `.sweetpad/` directory.

use std::path::PathBuf;

/// `$HOME`, if set and non-empty.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// `$XDG_STATE_HOME`, falling back to `$HOME/.local/state`.
#[must_use]
pub fn state_dir() -> Option<PathBuf> {
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
