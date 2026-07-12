//! `sweetpad source …` — a target's synchronized folders and membership
//! exceptions (CLI_DESIGN §9f).
//!
//! Xcode 16 synchronized root groups make target membership implicit — every
//! file under an attached folder belongs to the target — so this resource is
//! the whole source-management surface: attach/detach folders, and opt
//! individual files out (`exclude`) or back in (`include`) via the target's
//! `PBXFileSystemSynchronizedBuildFileExceptionSet`. All mutation goes through
//! [`sweetpad_lib::sync_pbxproj`] (parse → mutate → serialize, byte-for-byte).
//!
//! Mutations never guess: a workspace needs an unambiguous member project, and
//! `--target` may be omitted only when the project has exactly one target —
//! anything else is a hard error, interactive or not.

use clap::{Args, Subcommand};

use crate::cli::output::Output;
use crate::cli::pbxedit;
use crate::cli::resolve;
use crate::cli::{CliError, CommandResult, ContainerArgs, Context, Render, Rendered};
use sweetpad_lib::pbxproj::Value;
use sweetpad_lib::sync_pbxproj::{self, AddOutcome, ExcludeOutcome, IncludeOutcome, RemoveOutcome};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show each target's synchronized folders and membership exceptions.
    List(ListArgs),
    /// Attach a folder to a target as a synchronized root.
    Add(FolderArgs),
    /// Detach a synchronized folder from a target.
    Remove(FolderArgs),
    /// Opt a file inside a synchronized folder out of the target.
    Exclude(PathArgs),
    /// Drop a file's membership exception (opt it back in).
    Include(PathArgs),
}

/// Flags for `source list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    #[command(flatten)]
    pub container: ContainerArgs,

    /// Show one build target instead of all of them.
    #[arg(long)]
    pub target: Option<String>,
}

/// Flags for `source add`/`remove`.
#[derive(Debug, Args)]
pub struct FolderArgs {
    /// Folder path, relative to the project directory (e.g. `Sources/App`).
    pub dir: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Build target to attach/detach on. Optional only when the project has
    /// exactly one target.
    #[arg(long)]
    pub target: Option<String>,
}

/// Flags for `source exclude`/`include`.
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

/// Locate and parse the pbxproj a `source` action edits, under the
/// never-guess rules shared with `settings set`.
fn open_project(
    ctx: &mut Context,
    container_args: &ContainerArgs,
    target: Option<&String>,
) -> Result<(std::path::PathBuf, Value), CliError> {
    ctx.targeting = container_args.clone().into();
    let container = resolve::container(ctx)?;
    let targets: Vec<String> = target.cloned().into_iter().collect();
    let xcodeproj = pbxedit::mutation_xcodeproj(ctx, &container, &targets)?;
    let root = pbxedit::parse_owned(&xcodeproj)?;
    Ok((xcodeproj, root))
}

/// The target to act on: the `--target` flag, or the project's only target.
/// Multiple targets without a flag is ambiguity — a hard error naming them.
fn settle_target(root: &Value, flag: Option<&String>) -> Result<String, CliError> {
    if let Some(target) = flag {
        return Ok(target.clone());
    }
    let names = sweetpad_lib::settings_pbxproj::target_names(root);
    match names.as_slice() {
        [] => Err(CliError::new("the project declares no targets")),
        [only] => Ok(only.clone()),
        many => Err(CliError::new(format!(
            "the project has {} targets ({}); pass --target to say which one",
            many.len(),
            many.join(", ")
        ))),
    }
}

/// One applied `source` mutation, for the report: a human line plus the
/// structured payload.
struct SourceMutation {
    line: String,
    note: Option<String>,
    json: serde_json::Value,
}

impl Render for SourceMutation {
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
    let (xcodeproj, mut root) = open_project(ctx, &args.container, args.target.as_ref())?;
    let target = settle_target(&root, args.target.as_ref())?;
    let outcome = sync_pbxproj::add_root(&mut root, &target, &args.dir).map_err(CliError::new)?;

    let (line, changed, created_guid) = match &outcome {
        AddOutcome::Created(guid) => (
            format!(
                "attached {} to target {target} as a synchronized folder",
                args.dir
            ),
            true,
            Some(guid.clone()),
        ),
        AddOutcome::AttachedExisting(_) => (
            format!(
                "attached the existing synchronized folder {} to target {target}",
                args.dir
            ),
            true,
            None,
        ),
        AddOutcome::AlreadyAttached(_) => (
            format!(
                "{} is already a synchronized folder of target {target}",
                args.dir
            ),
            false,
            None,
        ),
    };
    if changed {
        pbxedit::write_pbxproj(&xcodeproj, &root)?;
    }

    // A brand-new root may point at a folder that doesn't exist yet — create
    // it so the project opens without a missing (red) reference.
    let mut note = None;
    if created_guid.is_some() {
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

    Ok(Rendered::data(SourceMutation {
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
    let (xcodeproj, mut root) = open_project(ctx, &args.container, args.target.as_ref())?;
    let target = settle_target(&root, args.target.as_ref())?;
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

    Ok(Rendered::data(SourceMutation {
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

fn exclude(ctx: &mut Context, args: &PathArgs) -> CommandResult {
    let (xcodeproj, mut root) = open_project(ctx, &args.container, args.target.as_ref())?;
    let target = settle_target(&root, args.target.as_ref())?;
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

    Ok(Rendered::data(SourceMutation {
        line,
        note: None,
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
    let (xcodeproj, mut root) = open_project(ctx, &args.container, args.target.as_ref())?;
    let target = settle_target(&root, args.target.as_ref())?;
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

    Ok(Rendered::data(SourceMutation {
        line,
        note: None,
        json: serde_json::json!({
            "action": "include",
            "target": target,
            "path": args.path,
            "changed": changed,
        }),
    }))
}

/// The `source list` payload: per target, its synchronized folders and each
/// folder's membership exceptions.
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
    let (_, root) = open_project(ctx, &args.container, args.target.as_ref())?;
    let mut targets = sync_pbxproj::list(&root).map_err(CliError::new)?;
    if let Some(filter) = &args.target {
        targets.retain(|t| &t.target == filter);
        if targets.is_empty() {
            return Err(CliError::new(format!("no target named `{filter}`")));
        }
    }
    Ok(Rendered::data(ListResult { targets }))
}
