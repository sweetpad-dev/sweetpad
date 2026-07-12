//! `sweetpad pbxproj folder …` — a target's synchronized source folders
//! (CLI_DESIGN §9g; Xcode 16's "folders", `PBXFileSystemSynchronizedRootGroup`).
//!
//! A synchronized folder makes target membership implicit: every file under
//! it belongs to the target. Attaching/detaching folders lives here;
//! per-file opt-outs are `pbxproj membership exclude/include`. All mutation
//! goes through [`sweetpad_lib::sync_pbxproj`] (parse → mutate → serialize,
//! byte-for-byte) and never guesses: a workspace needs an unambiguous member
//! project, and `--target` may be omitted only when the project has exactly
//! one target.

use clap::{Args, Subcommand};

use crate::cli::output::Output;
use crate::cli::pbxedit;
use crate::cli::{CliError, CommandResult, ContainerArgs, Context, Render, Rendered};
use sweetpad_lib::sync_pbxproj::{self, AddOutcome, RemoveOutcome};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show each target's synchronized folders and membership exceptions.
    List(ListArgs),
    /// Attach a folder to a target as a synchronized root.
    Add(FolderArgs),
    /// Detach a synchronized folder from a target.
    Remove(FolderArgs),
}

/// Flags for `pbxproj folder list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    #[command(flatten)]
    pub container: ContainerArgs,

    /// Show one build target instead of all of them.
    #[arg(long)]
    pub target: Option<String>,
}

/// Flags for `pbxproj folder add`/`remove`.
#[derive(Debug, Args)]
pub struct FolderArgs {
    /// Folder path, relative to the project directory (e.g. 'Sources/App').
    pub dir: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Build target to attach/detach on. Optional only when the project has
    /// exactly one target.
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
    }
}

/// One applied `folder` mutation, for the report: a human line plus the
/// structured payload.
struct FolderMutation {
    line: String,
    note: Option<String>,
    json: serde_json::Value,
}

impl Render for FolderMutation {
    fn human(&self, out: &Output) {
        out.line(&self.line);
        if let Some(note) = &self.note {
            out.note(note);
        }
    }

    fn json(&self) -> serde_json::Value {
        self.json.clone()
    }
}

fn add(ctx: &mut Context, args: &FolderArgs) -> CommandResult {
    let (xcodeproj, mut root) =
        super::open_project_mut(ctx, &args.container, args.target.as_ref(), args.force)?;
    let target = super::settle_target(&root, args.target.as_ref())?;
    let outcome = sync_pbxproj::add_root(&mut root, &target, &args.dir).map_err(CliError::new)?;

    let (line, changed, created) = match &outcome {
        AddOutcome::Created(_) => (
            format!(
                "attached {} to target {target} as a synchronized folder",
                args.dir
            ),
            true,
            true,
        ),
        AddOutcome::AttachedExisting(_) => (
            format!(
                "attached the existing synchronized folder {} to target {target}",
                args.dir
            ),
            true,
            false,
        ),
        AddOutcome::AlreadyAttached(_) => (
            format!(
                "{} is already a synchronized folder of target {target}",
                args.dir
            ),
            false,
            false,
        ),
    };
    if changed {
        pbxedit::write_pbxproj(&xcodeproj, &root)?;
    }

    // A brand-new root may point at a folder that doesn't exist yet — create
    // it so the project opens without a missing (red) reference.
    let mut note = None;
    if created {
        let dir = xcodeproj
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(&args.dir);
        if !dir.exists() {
            std::fs::create_dir_all(&dir)
                .map_err(|e| CliError::new(format!("failed to create {}: {e}", dir.display())))?;
            note = Some(format!("created {}", dir.display()));
        }
    }

    Ok(Rendered::data(FolderMutation {
        line,
        note,
        json: serde_json::json!({
            "action": "add",
            "target": target,
            "dir": args.dir,
            "changed": changed,
        }),
    }))
}

fn remove(ctx: &mut Context, args: &FolderArgs) -> CommandResult {
    let (xcodeproj, mut root) =
        super::open_project_mut(ctx, &args.container, args.target.as_ref(), args.force)?;
    let target = super::settle_target(&root, args.target.as_ref())?;
    let outcome =
        sync_pbxproj::remove_root(&mut root, &target, &args.dir).map_err(CliError::new)?;

    let (line, changed, deleted) = match &outcome {
        RemoveOutcome::Detached { deleted_object, .. } => (
            format!("detached {} from target {target}", args.dir),
            true,
            *deleted_object,
        ),
        RemoveOutcome::NotAttached => (
            format!(
                "{} is not a synchronized folder of target {target}",
                args.dir
            ),
            false,
            false,
        ),
    };
    if changed {
        pbxedit::write_pbxproj(&xcodeproj, &root)?;
    }
    let note = deleted.then(|| {
        format!(
            "no other target uses {}; its group was removed from the project \
             (files on disk are untouched)",
            args.dir
        )
    });

    Ok(Rendered::data(FolderMutation {
        line,
        note,
        json: serde_json::json!({
            "action": "remove",
            "target": target,
            "dir": args.dir,
            "changed": changed,
            "deletedGroup": deleted,
        }),
    }))
}

/// The `pbxproj folder list` payload: per target, its synchronized folders
/// and each folder's membership exceptions.
struct ListResult {
    targets: Vec<sync_pbxproj::TargetRoots>,
}

impl Render for ListResult {
    fn human(&self, out: &Output) {
        for (i, t) in self.targets.iter().enumerate() {
            if i > 0 {
                out.line("");
            }
            out.line(&format!("target {}", t.target));
            if t.roots.is_empty() {
                out.line("  (no synchronized folders)");
                continue;
            }
            for root in &t.roots {
                out.line(&format!("  folder {}", root.dir));
                for exception in &root.exceptions {
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
                let folders: Vec<serde_json::Value> = t
                    .roots
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "dir": r.dir,
                            "exceptions": r.exceptions,
                        })
                    })
                    .collect();
                serde_json::json!({ "target": t.target, "folders": folders })
            })
            .collect();
        serde_json::json!({ "targets": targets })
    }
}

fn list(ctx: &mut Context, args: &ListArgs) -> CommandResult {
    let (_, root) = super::open_project(ctx, &args.container, args.target.as_ref())?;
    let mut targets = sync_pbxproj::list(&root).map_err(CliError::new)?;
    if let Some(filter) = &args.target {
        targets.retain(|t| &t.target == filter);
        if targets.is_empty() {
            return Err(CliError::new(format!("no target named `{filter}`")));
        }
    }
    Ok(Rendered::data(ListResult { targets }))
}
