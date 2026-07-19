//! `sweetpad pbxproj fileref` — the `PBXFileReference` layer on its own: the
//! objects that say a file exists in the project, with no opinion about which
//! group shows it or which target builds it (CLI_DESIGN §9g).

use clap::{Args, Subcommand};

use crate::cli::output::Output;
use crate::cli::pbxedit;
use crate::cli::{CliError, CommandResult, ContainerArgs, Context, Render, Rendered};
use sweetpad_lib::tree_pbxproj::{self, AddRefOutcome};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show every file reference: id, stored path, anchor, and how many build
    /// files point at it.
    List(ListArgs),
    /// Create a file reference. Attaches to a group only when told to, and
    /// never joins a build phase.
    Add(AddArgs),
    /// Delete a file reference by id.
    Remove(RemoveArgs),
}

/// Flags for `pbxproj fileref list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    #[command(flatten)]
    pub container: ContainerArgs,

    /// Show only references whose resolved path starts with this prefix.
    #[arg(long)]
    pub under: Option<String>,
}

/// Flags for `pbxproj fileref add`.
#[derive(Debug, Args)]
pub struct AddArgs {
    /// File paths, interpreted against '--source-tree': group-relative for
    /// '<group>', project-relative for 'SOURCE_ROOT' (batched — one write, and
    /// '--type'/'--source-tree'/'--group' apply to every path).
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<String>,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// 'lastKnownFileType' to record (e.g. 'sourcecode.swift', 'image.png').
    /// Omitted entirely when not given, so Xcode derives it from the
    /// extension — an absent answer rather than a guessed one.
    #[arg(long = "type")]
    pub file_type: Option<String>,

    /// What the path is anchored to: '<group>' (the owning group's directory),
    /// 'SOURCE_ROOT' (the project directory), or '<absolute>'.
    #[arg(long, default_value = "<group>")]
    pub source_tree: String,

    /// Group to list the new references under, by id or by resolved directory
    /// ('Sources/App'). Without it they exist but no group shows them — attach
    /// them later with 'pbxproj group attach'.
    #[arg(long)]
    pub group: Option<String>,

    /// Build target to disambiguate which '.xcodeproj' in a workspace to edit.
    /// A file reference is not per-target.
    #[arg(long)]
    pub target: Option<String>,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
}

/// Flags for `pbxproj fileref remove`.
#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// The reference's object id (from 'pbxproj fileref list').
    pub id: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Delete even while build files still point at the reference, leaving
    /// them dangling. Dropping membership is 'pbxproj membership remove'.
    #[arg(long)]
    pub dangling: bool,

    /// Build target to disambiguate which '.xcodeproj' in a workspace to edit.
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

/// One applied `fileref` mutation, for the report.
struct RefMutation {
    line: String,
    json: serde_json::Value,
}

impl Render for RefMutation {
    fn human(&self, out: &Output) {
        out.line(&self.line);
    }

    fn json(&self) -> serde_json::Value {
        self.json.clone()
    }
}

struct ListResult {
    refs: Vec<tree_pbxproj::FileRefRow>,
}

impl Render for ListResult {
    fn human(&self, out: &Output) {
        if self.refs.is_empty() {
            out.line("  (no file references)");
            return;
        }
        for r in &self.refs {
            let kind = r.file_type.as_deref().unwrap_or("(untyped)");
            out.line(&format!(
                "{}  {}  [{kind}, {}, {} build file(s)]",
                r.guid, r.resolved, r.source_tree, r.build_files
            ));
        }
    }

    fn json(&self) -> serde_json::Value {
        let refs: Vec<serde_json::Value> = self
            .refs
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.guid,
                    "path": r.path,
                    "resolved": r.resolved,
                    "sourceTree": r.source_tree,
                    "fileType": r.file_type,
                    "group": r.parent,
                    "buildFiles": r.build_files,
                })
            })
            .collect();
        serde_json::json!({ "refs": refs })
    }
}

fn list(ctx: &mut Context, args: &ListArgs) -> CommandResult {
    let (_, root) = super::open_project(ctx, &args.container, None)?;
    let mut refs = tree_pbxproj::list_filerefs(&root).map_err(CliError::new)?;
    if let Some(under) = &args.under {
        let prefix = under.trim_matches('/');
        refs.retain(|r| r.resolved.starts_with(prefix));
    }
    Ok(Rendered::data(ListResult { refs }))
}

/// The `fileref add` report: one row per path, in argument order.
struct AddResult {
    rows: Vec<serde_json::Value>,
    lines: Vec<String>,
}

impl Render for AddResult {
    fn human(&self, out: &Output) {
        for line in &self.lines {
            out.line(line);
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({ "action": "add", "refs": self.rows })
    }
}

fn add(ctx: &mut Context, args: &AddArgs) -> CommandResult {
    let (xcodeproj, mut root) =
        super::open_project_mut(ctx, &args.container, args.target.as_ref(), args.force)?;

    // Resolve and apply the whole batch before writing: a bad path refuses the
    // batch rather than half-applying it, the same contract `membership add`
    // keeps.
    let mut rows = Vec::new();
    let mut lines = Vec::new();
    let mut changed = false;
    for path in &args.paths {
        let outcome = tree_pbxproj::add_fileref(
            &mut root,
            path,
            args.file_type.as_deref(),
            &args.source_tree,
            args.group.as_deref(),
        )
        .map_err(CliError::new)?;
        match &outcome {
            AddRefOutcome::Created {
                guid,
                resolved,
                attached_to,
            } => {
                let where_ = match attached_to {
                    Some(group) => format!(" under group {group}"),
                    None => " (unattached — no group lists it)".to_string(),
                };
                lines.push(format!("{guid}  {resolved}{where_}"));
                rows.push(serde_json::json!({
                    "id": guid,
                    "resolved": resolved,
                    "group": attached_to,
                    "changed": true,
                }));
                changed = true;
            }
            AddRefOutcome::AlreadyExists { guid, resolved } => {
                lines.push(format!("{guid}  {resolved} (already a reference)"));
                rows.push(serde_json::json!({
                    "id": guid,
                    "resolved": resolved,
                    "changed": false,
                }));
            }
        }
    }
    if changed {
        pbxedit::write_pbxproj(&xcodeproj, &root)?;
    }
    Ok(Rendered::data(AddResult { rows, lines }))
}

fn remove(ctx: &mut Context, args: &RemoveArgs) -> CommandResult {
    let (xcodeproj, mut root) =
        super::open_project_mut(ctx, &args.container, args.target.as_ref(), args.force)?;
    let outcome =
        tree_pbxproj::remove_fileref(&mut root, &args.id, args.dangling).map_err(CliError::new)?;
    pbxedit::write_pbxproj(&xcodeproj, &root)?;

    // Say what the delete took with it — the group entry has to go or the
    // project would name a missing object, and that is worth stating.
    let detached = match &outcome.detached_from {
        Some(group) => format!("; dropped from group {group}"),
        None => String::new(),
    };
    Ok(Rendered::data(RefMutation {
        line: format!("removed {}{detached}", outcome.guid),
        json: serde_json::json!({
            "action": "remove",
            "id": outcome.guid,
            "detachedFrom": outcome.detached_from,
            "changed": true,
        }),
    }))
}
