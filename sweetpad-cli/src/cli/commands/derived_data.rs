//! `sweetpad derived-data …` — inspect and purge Xcode's DerivedData.
//!
//! DerivedData (`~/Library/Developer/Xcode/DerivedData`) accumulates module
//! caches, indexes, and build products; "delete DerivedData" is the iOS
//! developer's most common reset. `path`/`size` inspect it, `purge` clears it —
//! whole, or scoped to the resolved project's `<Name>-<hash>` folder(s).

use std::path::{Path, PathBuf};

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::resolve::Container;
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered, resolve};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Print this project's DerivedData folder(s) (or the whole store: --all).
    Path {
        /// Operate on the whole DerivedData store, not just this project.
        #[arg(long)]
        all: bool,
    },
    /// Report the on-disk size of DerivedData (this project, or --all).
    Size {
        /// Operate on the whole DerivedData store, not just this project.
        #[arg(long)]
        all: bool,
    },
    /// Delete DerivedData — this project's folder(s) by default, or --all.
    Purge {
        /// Delete the whole DerivedData store, not just this project.
        #[arg(long)]
        all: bool,
        /// Skip the interactive confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Path { all } => path(ctx, *all),
        Action::Size { all } => size(ctx, *all),
        Action::Purge { all, yes } => purge(ctx, *all, *yes),
    }
}

/// The resolved DerivedData folder(s): one path per line in human mode (or a
/// "none found" note when empty), or `{ "root", "paths" }` in the JSON envelope.
struct PathResult {
    root: String,
    paths: Vec<String>,
}

impl Render for PathResult {
    fn human(&self, out: &Output) {
        if self.paths.is_empty() {
            out.note("no matching DerivedData folders found");
            return;
        }
        for p in &self.paths {
            out.line(p);
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "root": self.root,
            "paths": self.paths,
        })
    }
}

/// The on-disk size of the resolved DerivedData folder(s): a "<size> across N
/// folder(s)" line in human mode, or `{ "bytes", "human", "folders" }` in JSON.
struct SizeResult {
    bytes: u64,
    folders: usize,
}

impl Render for SizeResult {
    fn human(&self, out: &Output) {
        out.line(&format!(
            "{} across {} folder(s)",
            format_size(self.bytes),
            self.folders
        ));
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "bytes": self.bytes,
            "human": format_size(self.bytes),
            "folders": self.folders,
        })
    }
}

/// The outcome of a purge: a status note in human mode, or `{ "removed" }` in
/// JSON. `note` is what human mode prints — "purged N folder(s)" on a real
/// purge, or "nothing to purge"/"aborted" on the early paths (which carry an
/// empty `removed`, so `--json` still reports an outcome).
struct PurgeResult {
    removed: Vec<String>,
    note: String,
}

impl Render for PurgeResult {
    fn human(&self, out: &Output) {
        out.note(&self.note);
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({ "removed": self.removed })
    }
}

fn path(ctx: &mut Context, all: bool) -> CommandResult {
    let root = root()?;
    let targets = targets(ctx, &root, all)?;

    let paths: Vec<String> = targets.iter().map(|p| p.display().to_string()).collect();
    Ok(Rendered::data(PathResult {
        root: root.display().to_string(),
        paths,
    }))
}

fn size(ctx: &mut Context, all: bool) -> CommandResult {
    let root = root()?;
    let targets = targets(ctx, &root, all)?;
    let bytes: u64 = targets.iter().map(|p| dir_size(p)).sum();

    Ok(Rendered::data(SizeResult {
        bytes,
        folders: targets.len(),
    }))
}

fn purge(ctx: &mut Context, all: bool, yes: bool) -> CommandResult {
    let root = root()?;
    let targets = targets(ctx, &root, all)?;

    if targets.is_empty() {
        return Ok(Rendered::data(PurgeResult {
            removed: Vec::new(),
            note: "nothing to purge".to_string(),
        }));
    }

    // Confirm before deleting when we can prompt and weren't told `--yes`.
    if !yes && ctx.out.is_interactive() {
        let prompt = if all {
            format!("Delete ALL DerivedData under {}?", root.display())
        } else {
            format!(
                "Delete {} DerivedData folder(s) for this project?",
                targets.len()
            )
        };
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()
            .map_err(|e| {
                CliError::new(format!("confirmation cancelled: {e}")).kind(ErrorKind::UserCancel)
            })?;
        if !confirmed {
            return Ok(Rendered::data(PurgeResult {
                removed: Vec::new(),
                note: "aborted".to_string(),
            }));
        }
    }

    let mut removed = Vec::new();
    for p in &targets {
        std::fs::remove_dir_all(p)
            .map_err(|e| CliError::new(format!("failed to remove {}: {e}", p.display())))?;
        removed.push(p.display().to_string());
    }

    let note = format!("purged {} folder(s)", removed.len());
    Ok(Rendered::data(PurgeResult { removed, note }))
}

/// The DerivedData root, honoring `$HOME`.
fn root() -> Result<PathBuf, CliError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| CliError::new("$HOME is not set; cannot locate DerivedData"))?;
    Ok(derived_data_root(&home))
}

/// `<home>/Library/Developer/Xcode/DerivedData`.
fn derived_data_root(home: &Path) -> PathBuf {
    home.join("Library/Developer/Xcode/DerivedData")
}

/// The paths a command acts on: `--all` is the whole store — just `[root]`
/// (when it exists); the default is each `<Name>-<hash>` folder matching the
/// resolved container's base name.
fn targets(ctx: &Context, root: &Path, all: bool) -> Result<Vec<PathBuf>, CliError> {
    if all {
        return Ok(if root.is_dir() {
            vec![root.to_path_buf()]
        } else {
            Vec::new()
        });
    }

    let container = resolve::container(ctx)?;
    let base = project_base_name(&container).ok_or_else(|| {
        CliError::new("could not determine the project name to scope DerivedData")
    })?;

    let mut matches = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name();
                if matches_project(&name.to_string_lossy(), &base) {
                    matches.push(entry.path());
                }
            }
        }
    }
    matches.sort();
    Ok(matches)
}

/// The container's base name — the stem Xcode prefixes DerivedData folders
/// with (e.g. `MyApp.xcodeproj` → `MyApp`).
fn project_base_name(container: &Container) -> Option<String> {
    container
        .path()
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
}

/// Xcode names DerivedData folders `<Name>-<hash>`. Match the exact name (older
/// layouts) or the `<Name>-` prefix.
fn matches_project(entry: &str, base: &str) -> bool {
    entry == base || entry.starts_with(&format!("{base}-"))
}

/// Recursively sum the size of regular files under `path` (symlinks are not
/// followed). Unreadable entries are skipped rather than failing the command.
fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file()
                && let Ok(meta) = entry.metadata()
            {
                total += meta.len();
            }
        }
    }
    total
}

/// Human-readable byte size (binary units).
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_the_standard_xcode_path() {
        let root = derived_data_root(Path::new("/Users/me"));
        assert_eq!(
            root,
            Path::new("/Users/me/Library/Developer/Xcode/DerivedData")
        );
    }

    #[test]
    fn project_match_is_exact_or_hash_suffixed() {
        assert!(matches_project("MyApp", "MyApp"));
        assert!(matches_project("MyApp-abcdef123", "MyApp"));
        // A different project sharing a prefix must not match.
        assert!(!matches_project("MyAppHelper-abc", "MyApp"));
        assert!(!matches_project("Other-xyz", "MyApp"));
    }

    #[test]
    fn format_size_scales_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KiB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn dir_size_sums_nested_files() {
        let dir = std::env::temp_dir().join(format!(
            "sweetpad-dd-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sub = dir.join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("a.txt"), b"1234").unwrap(); // 4 bytes
        std::fs::write(sub.join("b.txt"), b"567890").unwrap(); // 6 bytes
        assert_eq!(dir_size(&dir), 10);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
