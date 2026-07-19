//! `sweetpad pbxproj membership …` — per-file target membership, across both
//! representations (CLI_DESIGN §9g; Xcode's "Target Membership" inspector).
//!
//! `list` reports everything a target builds *with provenance*: the classic
//! `PBXBuildFile` entries (path, build phase, per-file compiler flags/
//! attributes/platform filters) and the synchronized folders with their
//! exceptions. It does not enumerate the disk under folders — the folder plus
//! its exceptions *is* the membership statement; `ls` expands it.
//!
//! The mutation verbs are mechanism-specific and cross-hint: `remove` edits
//! classic build-file entries ([`sweetpad_lib::membership_pbxproj`]) and
//! errors toward `exclude` for files built via a synchronized folder;
//! `exclude`/`include` edit folder exception sets
//! ([`sweetpad_lib::sync_pbxproj`]) and error toward `remove` for files with
//! classic entries. The wrong verb never silently does the other thing.

use clap::{Args, Subcommand};

use crate::cli::output::Output;
use crate::cli::pbxedit;
use crate::cli::{CliError, CommandResult, ContainerArgs, Context, Render, Rendered};
use sweetpad_lib::membership_pbxproj::{self, FileEntry, Removal};
use sweetpad_lib::pbxproj::Value;
use sweetpad_lib::sync_pbxproj::{self, ExcludeOutcome, IncludeOutcome};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show everything a target builds: classic entries and synchronized
    /// folders, with provenance.
    List(ListArgs),
    /// Add classic per-file build entries to a target, in a named phase
    /// (batched).
    Add(AddArgs),
    /// Remove classic per-file build entries from a target (batched).
    Remove(RemoveArgs),
    /// Opt a file inside a synchronized folder out of the target.
    Exclude(PathArgs),
    /// Drop a file's membership exception (opt it back in).
    Include(PathArgs),
}

/// Flags for `pbxproj membership list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    #[command(flatten)]
    pub container: ContainerArgs,

    /// Show one build target instead of all of them.
    #[arg(long)]
    pub target: Option<String>,
}

/// Flags for `pbxproj membership remove`.
#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// File paths, relative to the project directory (batched — one write).
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<String>,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Build target whose membership the files leave. Optional only when the
    /// project has exactly one target.
    #[arg(long)]
    pub target: Option<String>,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
}

/// Flags for `pbxproj membership add`.
#[derive(Debug, Args)]
pub struct AddArgs {
    /// File paths, relative to the project directory (batched — one write).
    /// Each must already have a file reference; create one with 'pbxproj
    /// fileref add'.
    #[arg(value_name = "PATH")]
    pub paths: Vec<String>,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// File reference id to add, instead of naming it by path (repeatable, and
    /// combines with paths). This is what 'pbxproj fileref add' returns, so the
    /// two verbs compose without spelling the file twice — and it stays exact
    /// where a path is shared by two references.
    #[arg(long = "fileref", value_name = "ID")]
    pub filerefs: Vec<String>,

    /// Build target the files join. Optional only when the project has exactly
    /// one target.
    #[arg(long)]
    pub target: Option<String>,

    /// Build phase to add to. Named outright and never derived from the
    /// extension, so a '.ttf' goes wherever you say it goes.
    #[arg(long, value_parser = ["sources", "resources", "headers", "frameworks"])]
    pub phase: String,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
}

/// Flags for `pbxproj membership exclude`/`include`.
#[derive(Debug, Args)]
pub struct PathArgs {
    /// File path, relative to the project directory (e.g.
    /// `App/Resources/Info.plist`).
    pub path: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Build target whose membership the file leaves/rejoins. Optional only
    /// when the project has exactly one target.
    #[arg(long)]
    pub target: Option<String>,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::List(args) => list(ctx, args),
        Action::Add(args) => add(ctx, args),
        Action::Remove(args) => remove(ctx, args),
        Action::Exclude(args) => exclude(ctx, args),
        Action::Include(args) => include(ctx, args),
    }
}

/// Whether `path` has a classic build-file entry for `target`.
fn has_classic_entry(root: &Value, target: &str, path: &str) -> bool {
    membership_pbxproj::classic_members(root, target)
        .map(|entries| entries.iter().any(|e| e.path == path.trim_end_matches('/')))
        .unwrap_or(false)
}

// --- list ---

/// One target's membership, both worlds.
struct TargetMembership {
    target: String,
    explicit: Vec<FileEntry>,
    folders: Vec<sync_pbxproj::RootReport>,
}

/// The `pbxproj membership list` payload.
struct ListResult {
    targets: Vec<TargetMembership>,
}

impl Render for ListResult {
    fn human(&self, out: &Output) {
        for (i, t) in self.targets.iter().enumerate() {
            if i > 0 {
                out.line("");
            }
            out.line(&format!("target {}", t.target));
            if t.explicit.is_empty() && t.folders.is_empty() {
                out.line("  (no membership)");
                continue;
            }
            for entry in &t.explicit {
                let mut details = vec![entry.phase.display()];
                if let Some(flags) = &entry.compiler_flags {
                    details.push(format!("COMPILER_FLAGS: {flags}"));
                }
                if !entry.attributes.is_empty() {
                    details.push(format!("attrs: {}", entry.attributes.join(", ")));
                }
                if !entry.platform_filters.is_empty() {
                    details.push(format!("platforms: {}", entry.platform_filters.join(", ")));
                }
                out.line(&format!("  {}  ({})", entry.path, details.join("; ")));
            }
            for folder in &t.folders {
                out.line(&format!("  folder {}/  (everything under it)", folder.dir));
                for exception in &folder.exceptions {
                    out.line(&format!("    excluded: {exception}"));
                }
            }
        }
    }

    fn json(&self) -> serde_json::Value {
        let targets: Vec<serde_json::Value> = self
            .targets
            .iter()
            .map(|t| {
                let explicit: Vec<serde_json::Value> = t
                    .explicit
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "path": e.path,
                            "phase": e.phase.kind(),
                            "phaseName": match &e.phase {
                                membership_pbxproj::Phase::Copy(name) => {
                                    Some(name.as_str())
                                }
                                _ => None,
                            },
                            "kind": e.kind.as_str(),
                            "compilerFlags": e.compiler_flags,
                            "attributes": e.attributes,
                            "platformFilters": e.platform_filters,
                        })
                    })
                    .collect();
                let folders: Vec<serde_json::Value> = t
                    .folders
                    .iter()
                    .map(|f| {
                        serde_json::json!({
                            "dir": f.dir,
                            "exceptions": f.exceptions,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "target": t.target,
                    "explicit": explicit,
                    "folders": folders,
                })
            })
            .collect();
        serde_json::json!({ "targets": targets })
    }
}

fn list(ctx: &mut Context, args: &ListArgs) -> CommandResult {
    let (_, root) = super::open_project(ctx, &args.container, args.target.as_ref())?;
    let names = match &args.target {
        Some(t) => vec![t.clone()],
        None => sweetpad_lib::settings_pbxproj::target_names(&root),
    };
    let folder_reports = sync_pbxproj::list(&root).map_err(CliError::new)?;
    let mut targets = Vec::new();
    for name in names {
        let explicit = membership_pbxproj::classic_members(&root, &name).map_err(CliError::new)?;
        let folders = folder_reports
            .iter()
            .find(|t| t.target == name)
            .map(|t| t.roots.clone())
            .unwrap_or_default();
        targets.push(TargetMembership {
            target: name,
            explicit,
            folders,
        });
    }
    Ok(Rendered::data(ListResult { targets }))
}

// --- remove ---

/// The `pbxproj membership remove` report.
/// The `add` report: one row per path, naming the phase it joined.
struct AddResult {
    file: String,
    target: String,
    phase: String,
    additions: Vec<membership_pbxproj::Addition>,
}

impl Render for AddResult {
    fn human(&self, out: &Output) {
        out.line(&self.file);
        for a in &self.additions {
            let verb = if a.already_member {
                "already in"
            } else {
                "added to"
            };
            out.line(&format!(
                "  {}: {verb} {} (target {})",
                a.path, a.phase, self.target
            ));
        }
    }

    fn json(&self) -> serde_json::Value {
        let additions: Vec<serde_json::Value> = self
            .additions
            .iter()
            .map(|a| {
                serde_json::json!({
                    "path": a.path,
                    "phase": a.phase,
                    "buildFile": a.build_file,
                    "alreadyMember": a.already_member,
                })
            })
            .collect();
        serde_json::json!({
            "action": "add",
            "file": self.file,
            "target": self.target,
            "phase": self.phase,
            "additions": additions,
        })
    }
}

fn add(ctx: &mut Context, args: &AddArgs) -> CommandResult {
    if args.paths.is_empty() && args.filerefs.is_empty() {
        return Err(CliError::new(
            "name at least one file, by path or by `--fileref <ID>`",
        ));
    }
    let (xcodeproj, mut root) =
        super::open_project_mut(ctx, &args.container, args.target.as_ref(), args.force)?;
    let target = super::settle_target(&root, args.target.as_ref())?;
    let phase = membership_pbxproj::Phase::parse(&args.phase)
        .ok_or_else(|| CliError::new(format!("unknown build phase `{}`", args.phase)))?;

    // Cross-hint, mirroring `remove`: a file a synchronized folder already
    // builds needs no classic entry, and adding one builds it twice. Ids are
    // resolved to their paths first, so naming a file either way gets the same
    // check.
    let mut named = args.paths.clone();
    for id in &args.filerefs {
        named.push(membership_pbxproj::ref_path(&root, id).map_err(CliError::new)?);
    }
    for path in &named {
        if let Some(folder) =
            sync_pbxproj::containing_folder(&root, &target, path).map_err(CliError::new)?
        {
            return Err(CliError::new(format!(
                "{path} sits in the synchronized folder {folder}, which is already the \
                 membership for target {target} — a classic entry would build it twice. \
                 If it is excepted, use `pbxproj membership include {path} --target {target}`"
            )));
        }
    }

    let mut additions = membership_pbxproj::add_membership(&mut root, &target, &args.paths, &phase)
        .map_err(CliError::new)?;
    additions.extend(
        membership_pbxproj::add_membership_by_ids(&mut root, &target, &args.filerefs, &phase)
            .map_err(CliError::new)?,
    );
    if additions.iter().any(|a| !a.already_member) {
        pbxedit::write_pbxproj(&xcodeproj, &root)?;
    }
    Ok(Rendered::data(AddResult {
        file: xcodeproj.join("project.pbxproj").display().to_string(),
        target,
        phase: args.phase.clone(),
        additions,
    }))
}

struct RemoveResult {
    file: String,
    target: String,
    removals: Vec<Removal>,
}

impl Render for RemoveResult {
    fn human(&self, out: &Output) {
        use std::fmt::Write as _;

        out.line(&self.file);
        for r in &self.removals {
            if r.removed_phases.is_empty() {
                out.line(&format!(
                    "  {}: not a member of target {}",
                    r.path, self.target
                ));
                continue;
            }
            let mut line = format!(
                "  {}: removed from {} (target {})",
                r.path,
                r.removed_phases.join(", "),
                self.target
            );
            if r.deleted_reference {
                line.push_str("; no target builds it anymore — reference deleted");
            }
            // The group prune is a side effect of deleting the reference, so
            // say it happened rather than leaving it to a `git diff`.
            if r.pruned_groups > 0 {
                let plural = if r.pruned_groups == 1 { "" } else { "s" };
                let _ = write!(line, "; {} emptied group{plural} pruned", r.pruned_groups);
            }
            out.line(&line);
        }
    }

    fn json(&self) -> serde_json::Value {
        let removals: Vec<serde_json::Value> = self
            .removals
            .iter()
            .map(|r| {
                serde_json::json!({
                    "path": r.path,
                    "removedPhases": r.removed_phases,
                    "deletedReference": r.deleted_reference,
                    "prunedGroups": r.pruned_groups,
                })
            })
            .collect();
        serde_json::json!({
            "action": "remove",
            "file": self.file,
            "target": self.target,
            "removals": removals,
        })
    }
}

fn remove(ctx: &mut Context, args: &RemoveArgs) -> CommandResult {
    let (xcodeproj, mut root) =
        super::open_project_mut(ctx, &args.container, args.target.as_ref(), args.force)?;
    let target = super::settle_target(&root, args.target.as_ref())?;

    // Cross-hint: a file built via a synchronized folder has no classic entry
    // to remove — the folder's exception set is the membership mechanism.
    for path in &args.paths {
        if !has_classic_entry(&root, &target, path)
            && let Some(folder) =
                sync_pbxproj::containing_folder(&root, &target, path).map_err(CliError::new)?
        {
            return Err(CliError::new(format!(
                "{path} is built via the synchronized folder {folder} — use \
                 `pbxproj membership exclude {path} --target {target}`"
            )));
        }
    }

    let removals = membership_pbxproj::remove_membership(&mut root, &target, &args.paths)
        .map_err(CliError::new)?;
    if removals.iter().any(|r| !r.removed_phases.is_empty()) {
        pbxedit::write_pbxproj(&xcodeproj, &root)?;
    }
    Ok(Rendered::data(RemoveResult {
        file: xcodeproj.join("project.pbxproj").display().to_string(),
        target,
        removals,
    }))
}

// --- exclude / include ---

/// One applied exception edit, for the report.
struct ExceptionMutation {
    line: String,
    json: serde_json::Value,
}

impl Render for ExceptionMutation {
    fn human(&self, out: &Output) {
        out.line(&self.line);
    }

    fn json(&self) -> serde_json::Value {
        self.json.clone()
    }
}

fn exclude(ctx: &mut Context, args: &PathArgs) -> CommandResult {
    let (xcodeproj, mut root) =
        super::open_project_mut(ctx, &args.container, args.target.as_ref(), args.force)?;
    let target = super::settle_target(&root, args.target.as_ref())?;

    // Cross-hint: a classic build-file entry isn't silenced by an exception —
    // it has to be removed.
    if has_classic_entry(&root, &target, &args.path) {
        return Err(CliError::new(format!(
            "{} is built via an explicit build-file entry, not a synchronized \
             folder — use `pbxproj membership remove {} --target {target}`",
            args.path, args.path
        )));
    }

    let outcome = sync_pbxproj::exclude(&mut root, &target, &args.path).map_err(CliError::new)?;
    let (line, changed, root_dir, exception) = match outcome {
        ExcludeOutcome::Added {
            root_dir,
            exception,
        } => (
            format!("excluded {exception} from synchronized folder {root_dir} (target {target})"),
            true,
            root_dir,
            exception,
        ),
        ExcludeOutcome::AlreadyExcluded {
            root_dir,
            exception,
        } => (
            format!(
                "{exception} is already excluded from synchronized folder {root_dir} \
                 (target {target})"
            ),
            false,
            root_dir,
            exception,
        ),
    };
    if changed {
        pbxedit::write_pbxproj(&xcodeproj, &root)?;
    }
    Ok(Rendered::data(ExceptionMutation {
        line,
        json: serde_json::json!({
            "action": "exclude",
            "target": target,
            "folder": root_dir,
            "exception": exception,
            "changed": changed,
        }),
    }))
}

fn include(ctx: &mut Context, args: &PathArgs) -> CommandResult {
    let (xcodeproj, mut root) =
        super::open_project_mut(ctx, &args.container, args.target.as_ref(), args.force)?;
    let target = super::settle_target(&root, args.target.as_ref())?;
    let outcome = sync_pbxproj::include(&mut root, &target, &args.path).map_err(CliError::new)?;

    let (line, changed) = match &outcome {
        IncludeOutcome::Removed {
            root_dir,
            exception,
        } => (
            format!(
                "included {exception} back into synchronized folder {root_dir} \
                 (target {target})"
            ),
            true,
        ),
        IncludeOutcome::NotExcluded => (
            format!("{} is not excluded for target {target}", args.path),
            false,
        ),
    };
    if changed {
        pbxedit::write_pbxproj(&xcodeproj, &root)?;
    }
    Ok(Rendered::data(ExceptionMutation {
        line,
        json: serde_json::json!({
            "action": "include",
            "target": target,
            "path": args.path,
            "changed": changed,
        }),
    }))
}
