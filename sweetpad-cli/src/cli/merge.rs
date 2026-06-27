//! Shared plumbing for sweetpad's semantic git-conflict merges.
//!
//! Two file kinds get an object-graph merge instead of git's line-based one:
//! Xcode's `project.pbxproj` ([`sweetpad_lib::pbxproj_merge`]) and SwiftPM's
//! `Package.resolved` ([`sweetpad_lib::spm_resolved`]). Both are driven through the
//! same three entry points, which all funnel into [`merge_text`]:
//!
//! - **`pbxproj resolve` / `spm resolve`** — [`resolve`]: run *after* a conflict,
//!   reconstructing the three clean inputs from git's index stages
//!   (`:1:`/`:2:`/`:3:`) or, with `--force`, from `HEAD`/`MERGE_HEAD`.
//! - **`merge driver`** — [`run_driver`]: the program git itself invokes mid-merge
//!   once installed, reading the three temp files git hands it and writing the
//!   result back over `%A`.
//! - **`merge install`** — [`install`]: wire the driver into `.gitattributes` +
//!   `git config` so `git merge` resolves these files automatically.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::cli::output::Output;
use crate::cli::{CliError, CliResult, CommandResult, Render, Rendered};
use sweetpad_lib::pbxproj_merge::Conflict;

/// A file kind sweetpad knows how to merge semantically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pbxproj,
    Spm,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Pbxproj, Kind::Spm];

    /// Stable token used on the `merge driver <kind>` command line.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Kind::Pbxproj => "pbxproj",
            Kind::Spm => "spm",
        }
    }

    /// The git merge-driver name (`merge.<name>.driver`) and `.gitattributes`
    /// attribute value.
    fn driver_name(self) -> &'static str {
        match self {
            Kind::Pbxproj => "sweetpad-pbxproj",
            Kind::Spm => "sweetpad-spm",
        }
    }

    fn driver_description(self) -> &'static str {
        match self {
            Kind::Pbxproj => "SweetPad semantic Xcode project.pbxproj merge",
            Kind::Spm => "SweetPad semantic Package.resolved merge",
        }
    }

    /// The `.gitattributes` path pattern this kind claims.
    fn attr_pattern(self) -> &'static str {
        match self {
            Kind::Pbxproj => "*.pbxproj",
            Kind::Spm => "Package.resolved",
        }
    }

    /// Whether a repo-relative path is something this kind merges.
    #[must_use]
    pub fn matches(self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        match self {
            Kind::Pbxproj => name.ends_with(".pbxproj"),
            Kind::Spm => name == "Package.resolved",
        }
    }
}

/// A merged document plus any contradictions, before it is written anywhere.
pub struct MergeText {
    pub text: String,
    pub conflicts: Vec<Conflict>,
}

/// Parse the three inputs, merge them, and re-serialize. `base` of `None`
/// covers an add/add with no merge base. `Err` is an unrecoverable parse
/// failure (the file is left for git/the human); a clean parse with real
/// conflicts comes back as a populated `conflicts` vec, not an `Err`.
pub fn merge_text(
    kind: Kind,
    base: Option<&str>,
    ours: &str,
    theirs: &str,
    pathname: &str,
) -> Result<MergeText, String> {
    let base = base.map(str::trim).filter(|s| !s.is_empty());
    match kind {
        Kind::Pbxproj => {
            let parse = |label: &str, t: &str| {
                sweetpad_lib::pbxproj::parse(t)
                    .map_err(|e| format!("failed to parse {label} of {pathname}: {e:?}"))
            };
            let b = base.map(|t| parse("base", t)).transpose()?;
            let o = parse("ours", ours)?;
            let t = parse("theirs", theirs)?;
            let merged = sweetpad_lib::pbxproj_merge::merge(b.as_ref(), &o, &t);
            let text =
                sweetpad_lib::pbxproj_writer::serialize(&merged.value, &project_name_for(pathname));
            Ok(MergeText {
                text,
                conflicts: merged.conflicts,
            })
        }
        Kind::Spm => {
            let parse = |label: &str, t: &str| {
                serde_json::from_str::<serde_json::Value>(t)
                    .map_err(|e| format!("failed to parse {label} of {pathname}: {e}"))
            };
            let b = base.map(|t| parse("base", t)).transpose()?;
            let o = parse("ours", ours)?;
            let t = parse("theirs", theirs)?;
            let merged = sweetpad_lib::spm_resolved::merge(b.as_ref(), &o, &t);
            let text = sweetpad_lib::spm_resolved::serialize(&merged.value);
            Ok(MergeText {
                text,
                conflicts: merged.conflicts,
            })
        }
    }
}

/// Xcode embeds the `.xcodeproj` bundle name in some pbxproj annotations; the
/// writer needs it. Derive it from the path's `*.xcodeproj` parent directory.
fn project_name_for(path: &str) -> String {
    Path::new(path)
        .parent()
        .filter(|p| p.extension().and_then(OsStr::to_str) == Some("xcodeproj"))
        .and_then(|p| p.file_stem())
        .and_then(OsStr::to_str)
        .unwrap_or("Project")
        .to_string()
}

// ---------------------------------------------------------------------------
// `<kind> resolve` — post-conflict, reading git's merge state.
// ---------------------------------------------------------------------------

enum Outcome {
    Resolved,
    Conflicted(Vec<Conflict>),
    Skipped(String),
}

/// Resolve every conflicted file of `kind` (or the explicit `paths`) using
/// git's recorded merge inputs. Writes and stages each clean result; leaves
/// genuinely conflicted files untouched with a report. Errors (non-zero exit)
/// if anything is left unresolved.
pub fn resolve(kind: Kind, paths: &[PathBuf], force: bool) -> CommandResult {
    let repo = PathBuf::from(git(None, &["rev-parse", "--show-toplevel"])?.trim());

    let targets: Vec<String> = if paths.is_empty() {
        git(Some(&repo), &["diff", "--name-only", "--diff-filter=U"])?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && kind.matches(l))
            .map(String::from)
            .collect()
    } else {
        paths.iter().map(|p| to_repo_relative(&repo, p)).collect()
    };

    if targets.is_empty() {
        return Ok(Rendered::data(MergeReport {
            results: Vec::new(),
            empty_note: Some(format!(
                "No conflicted {} files to resolve.",
                kind.attr_pattern()
            )),
        }));
    }

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

    let results: Vec<(String, Outcome)> = targets
        .iter()
        .map(|path| {
            (
                path.clone(),
                resolve_one(&repo, kind, path, merge_head.as_deref()),
            )
        })
        .collect();

    let failed = results
        .iter()
        .filter(|(_, o)| !matches!(o, Outcome::Resolved))
        .count();
    let report = MergeReport {
        results,
        empty_note: None,
    };
    if failed > 0 {
        // Render the per-file report, but exit non-zero on unresolved conflicts.
        Ok(Rendered::data_with_exit(report, 1))
    } else {
        Ok(Rendered::data(report))
    }
}

fn resolve_one(repo: &Path, kind: Kind, path: &str, merge_head: Option<&str>) -> Outcome {
    let (base, ours, theirs) = read_inputs(repo, path, merge_head);
    let (Some(ours), Some(theirs)) = (ours, theirs) else {
        return Outcome::Skipped(
            "could not read both sides from git (file added or deleted on one side?)".into(),
        );
    };

    let merged = match merge_text(kind, base.as_deref(), &ours, &theirs, path) {
        Ok(m) => m,
        Err(reason) => return Outcome::Skipped(reason),
    };
    if !merged.conflicts.is_empty() {
        return Outcome::Conflicted(merged.conflicts);
    }

    let abs = repo.join(path);
    if let Err(e) = std::fs::write(&abs, merged.text) {
        return Outcome::Skipped(format!("failed to write {}: {e}", abs.display()));
    }
    if let Err(e) = git(Some(repo), &["add", "--", path]) {
        return Outcome::Skipped(format!("merged but failed to `git add`: {e}"));
    }
    Outcome::Resolved
}

/// (base, ours, theirs) text. Default: index stages `:1:/:2:/:3:`. `--force`:
/// `merge-base`, `HEAD`, and `MERGE_HEAD` revisions of the path.
fn read_inputs(
    repo: &Path,
    path: &str,
    merge_head: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>) {
    if let Some(merge_head) = merge_head {
        let base = git_opt(repo, &["merge-base", "HEAD", "MERGE_HEAD"])
            .map(|s| s.trim().to_string())
            .and_then(|sha| git_opt(repo, &["show", &format!("{sha}:{path}")]));
        let ours = git_opt(repo, &["show", &format!("HEAD:{path}")]);
        let theirs = git_opt(repo, &["show", &format!("{merge_head}:{path}")]);
        (base, ours, theirs)
    } else {
        let base = git_opt(repo, &["show", &format!(":1:{path}")]);
        let ours = git_opt(repo, &["show", &format!(":2:{path}")]);
        let theirs = git_opt(repo, &["show", &format!(":3:{path}")]);
        (base, ours, theirs)
    }
}

/// The per-file merge outcomes: a status line per file in human mode (or the
/// "no conflicted files" note when there was nothing to do), or `{ "files": […] }`
/// in the JSON envelope.
struct MergeReport {
    results: Vec<(String, Outcome)>,
    empty_note: Option<String>,
}

impl Render for MergeReport {
    fn human(&self, out: &Output) {
        if let Some(note) = &self.empty_note {
            out.line(note);
            return;
        }
        for (path, outcome) in &self.results {
            match outcome {
                Outcome::Resolved => out.line(&format!("  resolved  {path}")),
                Outcome::Skipped(reason) => out.line(&format!("  skipped   {path} — {reason}")),
                Outcome::Conflicted(conflicts) => {
                    out.line(&format!("  CONFLICT  {path}"));
                    for c in conflicts {
                        out.line(&format!(
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

    fn json(&self) -> serde_json::Value {
        let files: Vec<serde_json::Value> = self
            .results
            .iter()
            .map(|(path, outcome)| match outcome {
                Outcome::Resolved => serde_json::json!({ "path": path, "status": "resolved" }),
                Outcome::Conflicted(c) => serde_json::json!({
                    "path": path,
                    "status": "conflicted",
                    "conflicts": c.iter().map(|c| serde_json::json!({
                        "path": c.path, "kind": c.kind.as_str(), "detail": c.detail
                    })).collect::<Vec<_>>(),
                }),
                Outcome::Skipped(reason) => {
                    serde_json::json!({ "path": path, "status": "skipped", "reason": reason })
                }
            })
            .collect();
        serde_json::json!({ "files": files })
    }
}

// ---------------------------------------------------------------------------
// `merge driver` — the program git invokes mid-merge.
// ---------------------------------------------------------------------------

/// Git merge-driver entry point. Reads the base/ours/theirs temp files git
/// passes (`%O`/`%A`/`%B`), writes the merged result back over `ours_path`
/// (git's `%A`, which it then takes as the working-tree content), and returns
/// `Ok` on a clean merge. On a real conflict it leaves `%A` as-is and errors,
/// so git marks the path unmerged (stages intact) — the user can then inspect
/// it with `<kind> resolve`.
pub fn run_driver(
    kind: Kind,
    base_path: &Path,
    ours_path: &Path,
    theirs_path: &Path,
    pathname: Option<&str>,
) -> CliResult {
    let read = |p: &Path| {
        std::fs::read_to_string(p).map_err(|e| CliError::new(format!("read {}: {e}", p.display())))
    };
    let base = read(base_path).ok();
    let ours = read(ours_path)?;
    let theirs = read(theirs_path)?;

    let pathname =
        pathname.map_or_else(|| ours_path.to_string_lossy().into_owned(), str::to_string);

    let merged =
        merge_text(kind, base.as_deref(), &ours, &theirs, &pathname).map_err(CliError::new)?;
    if merged.conflicts.is_empty() {
        std::fs::write(ours_path, merged.text)
            .map_err(|e| CliError::new(format!("write {}: {e}", ours_path.display())))?;
        Ok(())
    } else {
        Err(CliError::new(format!(
            "{} conflict(s) in {pathname} need manual resolution (run `sweetpad {} resolve`)",
            merged.conflicts.len(),
            kind.token(),
        )))
    }
}

// ---------------------------------------------------------------------------
// `merge install` — register the drivers with git.
// ---------------------------------------------------------------------------

/// Configure git to route `.pbxproj` and `Package.resolved` through sweetpad's
/// drivers: a `git config` driver definition (local or `--global`) per kind,
/// plus the matching `.gitattributes` lines.
pub fn install(global: bool) -> CommandResult {
    let repo = PathBuf::from(git(None, &["rev-parse", "--show-toplevel"])?.trim());
    let exe = std::env::current_exe()
        .map_err(|e| CliError::new(format!("cannot locate the sweetpad executable: {e}")))?;
    let exe = exe.to_string_lossy();

    let scope: &[&str] = if global {
        &["config", "--global"]
    } else {
        &["config", "--local"]
    };
    for kind in Kind::ALL {
        let name = kind.driver_name();
        // `%P` (pathname) needs git ≥ 2.x; older git just omits it and we fall
        // back to a default project name.
        let driver = format!("\"{exe}\" merge driver {} %O %A %B %P", kind.token());
        git(
            Some(&repo),
            &[
                scope[0],
                scope[1],
                &format!("merge.{name}.name"),
                kind.driver_description(),
            ],
        )?;
        git(
            Some(&repo),
            &[scope[0], scope[1], &format!("merge.{name}.driver"), &driver],
        )?;
    }

    let lines: Vec<String> = Kind::ALL
        .iter()
        .map(|k| format!("{} merge={}", k.attr_pattern(), k.driver_name()))
        .collect();

    let attr_path = if global {
        global_attributes_file(&repo)?
    } else {
        repo.join(".gitattributes")
    };
    let changed = ensure_lines(&attr_path, &lines)?;

    Ok(Rendered::data(InstallReport {
        global,
        changed,
        attributes_file: attr_path.display().to_string(),
        patterns: lines,
    }))
}

/// The result of `merge install`: the configured-drivers summary lines in human
/// mode, or `{ scope, attributesFile, changed, patterns }` in the JSON envelope.
struct InstallReport {
    global: bool,
    changed: bool,
    attributes_file: String,
    patterns: Vec<String>,
}

impl Render for InstallReport {
    fn human(&self, out: &Output) {
        out.line(&format!(
            "Configured sweetpad merge drivers ({} git config).",
            if self.global { "global" } else { "local" }
        ));
        out.line(&format!(
            "{} {}:",
            if self.changed {
                "Updated"
            } else {
                "Already present in"
            },
            self.attributes_file
        ));
        for line in &self.patterns {
            out.line(&format!("  {line}"));
        }
        if !self.global {
            out.note("commit .gitattributes so collaborators get the same behavior (the driver config is per-clone — they run `sweetpad merge install` once).");
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "scope": if self.global { "global" } else { "local" },
            "attributesFile": self.attributes_file,
            "changed": self.changed,
            "patterns": self.patterns,
        })
    }
}

/// The global gitattributes file (`core.attributesFile`), defaulting to
/// `$XDG_CONFIG_HOME/git/attributes` and setting the config if unset.
fn global_attributes_file(repo: &Path) -> Result<PathBuf, CliError> {
    if let Some(configured) = git_opt(repo, &["config", "--global", "core.attributesFile"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Ok(PathBuf::from(expand_tilde(&configured)));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home_dir().join(".config"));
    let path = base.join("git").join("attributes");
    git(
        Some(repo),
        &[
            "config",
            "--global",
            "core.attributesFile",
            &path.to_string_lossy(),
        ],
    )?;
    Ok(path)
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn expand_tilde(path: &str) -> String {
    path.strip_prefix("~/").map_or_else(
        || path.to_string(),
        |rest| home_dir().join(rest).to_string_lossy().into_owned(),
    )
}

/// Append any of `lines` not already present (verbatim) to `path`, creating it
/// and parent dirs. Returns whether the file changed.
fn ensure_lines(path: &Path, lines: &[String]) -> Result<bool, CliError> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut content = existing.clone();
    let mut changed = false;
    for line in lines {
        if existing.lines().any(|l| l.trim() == line) {
            continue;
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(line);
        content.push('\n');
        changed = true;
    }
    if changed {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CliError::new(format!("create {}: {e}", parent.display())))?;
        }
        std::fs::write(path, content)
            .map_err(|e| CliError::new(format!("write {}: {e}", path.display())))?;
    }
    Ok(changed)
}

// ---------------------------------------------------------------------------
// git helpers
// ---------------------------------------------------------------------------

/// Normalize a user-supplied path to repo-relative (forward slashes).
fn to_repo_relative(repo: &Path, path: &Path) -> String {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |c| c.join(path))
    });
    abs.strip_prefix(repo)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Run git and require success, returning stdout. `cwd` `None` inherits the
/// process directory (for the initial `rev-parse --show-toplevel`).
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

/// Run git, returning stdout on success or `None` on any non-zero exit — for
/// reads that are expected to be absent (a missing `:1:` stage, an unset
/// config key). Stderr suppressed to keep the report clean.
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
