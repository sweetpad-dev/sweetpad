//! `sweetpad pbxproj …` — operate on Xcode `project.pbxproj` files.
//!
//! Currently one action: [`Action::Resolve`], a semantic three-way merge for
//! `.pbxproj` files left conflicted by `git merge`/`rebase`/`cherry-pick`.
//!
//! Run it *mid-conflict*: when git can't merge a `pbxproj` textually it marks
//! the path unmerged and keeps the three clean inputs in the index as numbered
//! stages — `:1:` base, `:2:` ours, `:3:` theirs. We read those pristine blobs
//! (never the marker-riddled working copy), parse each into an object graph,
//! merge them with [`crate::pbxproj_merge`], serialize byte-for-byte with
//! [`crate::pbxproj_writer`], write the result, and `git add` it to mark the
//! path resolved. Files with a genuine contradiction are left untouched with a
//! report of exactly which object/field collided.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::Subcommand;

use crate::cli::{CliError, CliResult, Context};
use crate::pbxproj_merge::{self, Conflict};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Semantically merge conflicted `.pbxproj` files using git's merge state.
    Resolve {
        /// Files to resolve. Defaults to every conflicted `.pbxproj` in the
        /// repository.
        paths: Vec<PathBuf>,
        /// Re-merge from HEAD/MERGE_HEAD even when git already auto-merged the
        /// file textually (so there are no conflict stages in the index).
        #[arg(long)]
        force: bool,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::Resolve { paths, force } => resolve(ctx, paths, *force),
    }
}

/// What happened to one file.
enum Outcome {
    Resolved,
    Conflicted(Vec<Conflict>),
    Skipped(String),
}

fn resolve(ctx: &mut Context, paths: &[PathBuf], force: bool) -> CliResult {
    let repo = git(None, &["rev-parse", "--show-toplevel"])?;
    let repo = PathBuf::from(repo.trim());

    let targets = if paths.is_empty() {
        discover_conflicted(&repo)?
    } else {
        paths.iter().map(|p| to_repo_relative(&repo, p)).collect()
    };

    if targets.is_empty() {
        ctx.out.line("No conflicted .pbxproj files to resolve.");
        return Ok(());
    }

    // Resolve `MERGE_HEAD` once for --force (HEAD/MERGE_HEAD recovery path).
    let merge_head = if force {
        Some(
            git_opt(&repo, &["rev-parse", "MERGE_HEAD"])
                .map(|s| s.trim().to_string())
                .ok_or_else(|| {
                    CliError::new(
                        "--force needs an in-progress merge (MERGE_HEAD), but none was found",
                    )
                })?,
        )
    } else {
        None
    };

    let mut results: Vec<(String, Outcome)> = Vec::new();
    for path in &targets {
        results.push((
            path.clone(),
            resolve_one(&repo, path, merge_head.as_deref()),
        ));
    }

    report(ctx, &results);

    let failed = results
        .iter()
        .filter(|(_, o)| !matches!(o, Outcome::Resolved))
        .count();
    if failed > 0 {
        Err(CliError::new(format!(
            "{failed} file(s) left unresolved — see report above"
        )))
    } else {
        Ok(())
    }
}

/// Merge a single conflicted file. Reads the three inputs from git, merges,
/// and on a clean result writes the file and stages it.
fn resolve_one(repo: &Path, path: &str, merge_head: Option<&str>) -> Outcome {
    let (base, ours, theirs) = match read_inputs(repo, path, merge_head) {
        Ok(inputs) => inputs,
        Err(reason) => return Outcome::Skipped(reason),
    };

    let (Some(ours), Some(theirs)) = (ours, theirs) else {
        return Outcome::Skipped(
            "could not read both sides from git (file added or deleted on one side?)".into(),
        );
    };

    let parse = |label: &str, text: &str| {
        crate::pbxproj::parse(text).map_err(|e| format!("failed to parse {label} of {path}: {e:?}"))
    };
    let base_val = match base.as_deref().map(|t| parse("base", t)).transpose() {
        Ok(v) => v,
        Err(reason) => return Outcome::Skipped(reason),
    };
    let ours_val = match parse("ours", &ours) {
        Ok(v) => v,
        Err(reason) => return Outcome::Skipped(reason),
    };
    let theirs_val = match parse("theirs", &theirs) {
        Ok(v) => v,
        Err(reason) => return Outcome::Skipped(reason),
    };

    let merged = pbxproj_merge::merge(base_val.as_ref(), &ours_val, &theirs_val);
    if !merged.is_clean() {
        return Outcome::Conflicted(merged.conflicts);
    }

    let text = crate::pbxproj_writer::serialize(&merged.value, &project_name_for(path));
    let abs = repo.join(path);
    if let Err(e) = std::fs::write(&abs, text) {
        return Outcome::Skipped(format!("failed to write {}: {e}", abs.display()));
    }
    if let Err(e) = git(Some(repo), &["add", "--", path]) {
        return Outcome::Skipped(format!("merged but failed to `git add`: {e}"));
    }
    Outcome::Resolved
}

/// Read (base, ours, theirs) as raw text. Default: index stages `:1:/:2:/:3:`.
/// `--force`: `merge-base`, `HEAD`, and `MERGE_HEAD` revisions of the path.
fn read_inputs(
    repo: &Path,
    path: &str,
    merge_head: Option<&str>,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    if let Some(merge_head) = merge_head {
        let base = git_opt(repo, &["merge-base", "HEAD", "MERGE_HEAD"])
            .map(|s| s.trim().to_string())
            .and_then(|sha| git_opt(repo, &["show", &format!("{sha}:{path}")]));
        let ours = git_opt(repo, &["show", &format!("HEAD:{path}")]);
        let theirs = git_opt(repo, &["show", &format!("{merge_head}:{path}")]);
        Ok((base, ours, theirs))
    } else {
        let base = git_opt(repo, &["show", &format!(":1:{path}")]);
        let ours = git_opt(repo, &["show", &format!(":2:{path}")]);
        let theirs = git_opt(repo, &["show", &format!(":3:{path}")]);
        Ok((base, ours, theirs))
    }
}

/// Conflicted paths (`git diff --diff-filter=U`) ending in `.pbxproj`.
fn discover_conflicted(repo: &Path) -> Result<Vec<String>, CliError> {
    let out = git(Some(repo), &["diff", "--name-only", "--diff-filter=U"])?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.ends_with(".pbxproj"))
        .map(String::from)
        .collect())
}

/// Xcode embeds the `.xcodeproj` bundle name in some annotations; the writer
/// needs it. Derive it from the path's `*.xcodeproj` parent directory.
fn project_name_for(path: &str) -> String {
    Path::new(path)
        .parent()
        .filter(|p| p.extension().and_then(OsStr::to_str) == Some("xcodeproj"))
        .and_then(|p| p.file_stem())
        .and_then(OsStr::to_str)
        .unwrap_or("Project")
        .to_string()
}

/// Normalize a user-supplied path to repo-relative (forward slashes), so git
/// revision lookups and the discovery list speak the same dialect.
fn to_repo_relative(repo: &Path, path: &Path) -> String {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|c| c.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    });
    abs.strip_prefix(repo)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn report(ctx: &Context, results: &[(String, Outcome)]) {
    if ctx.out.is_json() {
        let to_json = |conflicts: &[Conflict]| {
            conflicts
                .iter()
                .map(|c| {
                    serde_json::json!({ "path": c.path, "kind": c.kind.as_str(), "detail": c.detail })
                })
                .collect::<Vec<_>>()
        };
        let files: Vec<serde_json::Value> = results
            .iter()
            .map(|(path, outcome)| match outcome {
                Outcome::Resolved => serde_json::json!({ "path": path, "status": "resolved" }),
                Outcome::Conflicted(c) => {
                    serde_json::json!({ "path": path, "status": "conflicted", "conflicts": to_json(c) })
                }
                Outcome::Skipped(reason) => {
                    serde_json::json!({ "path": path, "status": "skipped", "reason": reason })
                }
            })
            .collect();
        ctx.out.json_value(&serde_json::json!({ "files": files }));
        return;
    }

    for (path, outcome) in results {
        match outcome {
            Outcome::Resolved => ctx.out.line(&format!("  resolved  {path}")),
            Outcome::Skipped(reason) => ctx.out.line(&format!("  skipped   {path} — {reason}")),
            Outcome::Conflicted(conflicts) => {
                ctx.out.line(&format!("  CONFLICT  {path}"));
                for c in conflicts {
                    ctx.out.line(&format!(
                        "      [{}] {} — {}",
                        c.kind.as_str(),
                        c.path,
                        c.detail
                    ));
                }
            }
        }
    }
}

/// Run git and require success, returning stdout. `cwd` of `None` inherits the
/// process directory (used for the initial `rev-parse --show-toplevel`).
fn git(cwd: Option<&Path>, args: &[&str]) -> Result<String, CliError> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd
        .output()
        .map_err(|e| CliError::new(format!("failed to run git: {e}")))?;
    if !out.status.success() {
        return Err(CliError::new(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run git, returning stdout on success or `None` on any non-zero exit — used
/// for stage/revision reads that are *expected* to be absent (e.g. an add/add
/// conflict has no `:1:` base stage). Stderr is suppressed to keep the report
/// clean.
fn git_opt(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}
