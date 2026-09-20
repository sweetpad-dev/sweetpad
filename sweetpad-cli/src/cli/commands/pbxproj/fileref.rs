//! `sweetpad pbxproj fileref` — the file nodes on their own: what says a file
//! exists in the project, with no opinion about which target builds it
//! (CLI_DESIGN §9g).
//!
//! A `project.pbxproj` splits this from the navigator — a `PBXFileReference`
//! can exist with no group listing it — while a `project.xcproj` has one
//! nested tree in which the node *is* both. The verbs and their flags are the
//! same across the two; what differs is how a node is named, which is the
//! object id in one format and the navigator path in the other.

use clap::{Args, Subcommand};

use crate::cli::output::Output;
use crate::cli::pbxedit::Editable;
use crate::cli::{CliError, CommandResult, ContainerArgs, Context, Render, Rendered};
use sweetpad_lib::tree::{AddRefOutcome, FileRefRow, RemoveOutcome};
use sweetpad_lib::{tree_pbxproj, tree_xcproj};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show every file node: how to name it, the path it resolves to, its
    /// anchor, and how many build entries name it.
    List(ListArgs),
    /// Add a file to the project. Joins no build phase.
    Add(AddArgs),
    /// Delete a file from the project.
    Remove(RemoveArgs),
}

/// Flags for `pbxproj fileref list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    #[command(flatten)]
    pub container: ContainerArgs,

    /// Show only files whose resolved path starts with this prefix.
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

    /// File type to record (e.g. 'sourcecode.swift', 'image.png'). Omitted
    /// entirely when not given, so Xcode derives it from the extension — an
    /// absent answer rather than a guessed one.
    #[arg(long = "type")]
    pub file_type: Option<String>,

    /// What the path is anchored to: '<group>' (the holding group's
    /// directory), 'SOURCE_ROOT' (the project directory), or '<absolute>'.
    #[arg(long, default_value = "<group>")]
    pub source_tree: String,

    /// Group to put the new files under, named as 'pbxproj group list' prints
    /// it. Without it a project.xcproj puts them at the navigator root, while
    /// a project.pbxproj leaves the references with no group showing them —
    /// attach them later with 'pbxproj group attach'.
    #[arg(long)]
    pub group: Option<String>,

    /// Build target to disambiguate which '.xcodeproj' in a workspace to edit.
    /// A file is not per-target.
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
    /// The file to delete, named as 'pbxproj fileref list' prints it: the
    /// object id in a project.pbxproj, the navigator path
    /// ('Sources/App/ContentView.swift') in a project.xcproj.
    #[arg(value_name = "FILE")]
    pub address: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Delete even while a target still builds the file. A project.pbxproj
    /// leaves the build files dangling; a project.xcproj keeps the membership
    /// on the node, so it goes too. Dropping membership on its own is
    /// 'pbxproj membership remove'.
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
    refs: Vec<FileRefRow>,
}

impl Render for ListResult {
    fn human(&self, out: &Output) {
        if self.refs.is_empty() {
            out.line("  (no files)");
            return;
        }
        for r in &self.refs {
            let kind = r.file_type.as_deref().unwrap_or("(untyped)");
            // An address that is already the path says the path; printing it
            // twice would bury the nodes whose navigator position and on-disk
            // location genuinely differ.
            let resolved = if r.resolved == r.address {
                String::new()
            } else {
                format!("  {}", r.resolved)
            };
            out.line(&format!(
                "{}{resolved}  [{kind}, {}, {} build file(s)]",
                r.address, r.source_tree, r.build_files
            ));
        }
    }

    fn json(&self) -> serde_json::Value {
        let refs: Vec<serde_json::Value> = self
            .refs
            .iter()
            .map(|r| {
                serde_json::json!({
                    "address": r.address,
                    "id": r.id,
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
    let (_, document) = super::open_document(ctx, &args.container, None)?;
    let mut refs = match &document {
        Editable::Pbxproj(root) => tree_pbxproj::list_filerefs(root),
        Editable::Xcproj(root) => tree_xcproj::list_filerefs(root),
    }
    .map_err(CliError::new)?;
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
    let targets: Vec<String> = args.target.clone().into_iter().collect();
    let (xcodeproj, mut document) =
        super::open_document_mut(ctx, &args.container, &targets, args.force)?;

    // Resolve and apply the whole batch before writing: a bad path refuses the
    // batch rather than half-applying it, the same contract `membership add`
    // keeps.
    let mut rows = Vec::new();
    let mut lines = Vec::new();
    let mut changed = false;
    for path in &args.paths {
        let outcome = match &mut document {
            Editable::Pbxproj(root) => tree_pbxproj::add_fileref(
                root,
                path,
                args.file_type.as_deref(),
                &args.source_tree,
                args.group.as_deref(),
            ),
            Editable::Xcproj(root) => tree_xcproj::add_fileref(
                root,
                path,
                args.file_type.as_deref(),
                &args.source_tree,
                args.group.as_deref(),
            ),
        }
        .map_err(CliError::new)?;
        match &outcome {
            AddRefOutcome::Created {
                address,
                resolved,
                attached_to,
            } => {
                let where_ = match attached_to {
                    Some(group) => format!(" under group {group}"),
                    None => " (unattached — no group lists it)".to_string(),
                };
                lines.push(format!("{address}  {resolved}{where_}"));
                rows.push(serde_json::json!({
                    "address": address,
                    "resolved": resolved,
                    "group": attached_to,
                    "changed": true,
                }));
                changed = true;
            }
            AddRefOutcome::AlreadyExists { address, resolved } => {
                lines.push(format!("{address}  {resolved} (already in the project)"));
                rows.push(serde_json::json!({
                    "address": address,
                    "resolved": resolved,
                    "changed": false,
                }));
            }
        }
    }
    if changed {
        document.write(&xcodeproj)?;
    }
    Ok(Rendered::data(AddResult { rows, lines }))
}

fn remove(ctx: &mut Context, args: &RemoveArgs) -> CommandResult {
    let targets: Vec<String> = args.target.clone().into_iter().collect();
    let (xcodeproj, mut document) =
        super::open_document_mut(ctx, &args.container, &targets, args.force)?;
    let outcome: RemoveOutcome = match &mut document {
        Editable::Pbxproj(root) => tree_pbxproj::remove_fileref(root, &args.address, args.dangling),
        Editable::Xcproj(root) => tree_xcproj::remove_fileref(root, &args.address, args.dangling),
    }
    .map_err(CliError::new)?;
    document.write(&xcodeproj)?;

    // Say what the delete took with it — the group entry has to go or the
    // project would name a missing object, and that is worth stating.
    let detached = match &outcome.detached_from {
        Some(group) => format!("; dropped from group {group}"),
        None => String::new(),
    };
    Ok(Rendered::data(RefMutation {
        line: format!("removed {}{detached}", outcome.address),
        json: serde_json::json!({
            "action": "remove",
            "address": outcome.address,
            "detachedFrom": outcome.detached_from,
            "changed": true,
        }),
    }))
}
