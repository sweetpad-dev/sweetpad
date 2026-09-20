//! `sweetpad pbxproj group` — the navigator tree on its own: where a file
//! *appears* in Xcode, which says nothing about what builds it (CLI_DESIGN
//! §9g). Membership is [`super::membership`]'s axis.
//!
//! `attach`/`detach` exist because a `PBXGroup`'s `children` is a list of
//! references: the same object can be listed in two groups, and attaching it
//! to one does not take it out of the other. A `project.xcproj` nests its
//! nodes instead, so a node is in exactly one place and the operation is
//! [`Action::Move`] — which works on both formats. The two verbs that have no
//! meaning there say so rather than quietly doing something near enough.

use clap::{Args, Subcommand};

use crate::cli::output::Output;
use crate::cli::pbxedit::Editable;
use crate::cli::{CliError, CommandResult, ContainerArgs, Context, Render, Rendered};
use sweetpad_lib::tree::{AddGroupOutcome, GroupRow, MoveOutcome, RemoveOutcome};
use sweetpad_lib::tree_pbxproj::{self, LinkOutcome};
use sweetpad_lib::tree_xcproj;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show every group: how to name it, its resolved directory, and how many
    /// children it holds.
    List(ListArgs),
    /// Create a group under a parent group.
    Add(AddArgs),
    /// Delete an empty group.
    Remove(RemoveArgs),
    /// Move a node into another group, keeping the file it resolves to.
    Move(MoveArgs),
    /// List an existing object in a group's children (project.pbxproj only).
    Attach(LinkArgs),
    /// Drop an object from a group's children, leaving the object itself
    /// (project.pbxproj only).
    Detach(LinkArgs),
}

/// Flags for `pbxproj group list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    #[command(flatten)]
    pub container: ContainerArgs,
}

/// Flags for `pbxproj group add`.
#[derive(Debug, Args)]
pub struct AddArgs {
    /// The group's name in the navigator.
    pub name: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Group to create it under, named as 'pbxproj group list' prints it.
    /// Defaults to the navigator root.
    #[arg(long)]
    pub parent: Option<String>,

    /// Directory the group contributes to its children's paths. Omit it for a
    /// purely organizational group that adds no directory component.
    #[arg(long)]
    pub path: Option<String>,

    /// What '--path' is anchored to.
    #[arg(long, default_value = "<group>")]
    pub source_tree: String,

    /// Build target to disambiguate which '.xcodeproj' in a workspace to edit.
    /// A group is not per-target.
    #[arg(long)]
    pub target: Option<String>,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
}

/// Flags for `pbxproj group remove`.
#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// The group to delete, named as 'pbxproj group list' prints it: the
    /// object id in a project.pbxproj, the navigator path ('Sources/App') in
    /// a project.xcproj.
    #[arg(value_name = "GROUP")]
    pub address: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Delete even while the group still lists children, leaving them in the
    /// project with nothing showing them. A project.xcproj nests its children
    /// inside the group rather than listing them, so there is nothing left to
    /// orphan and this is refused — empty the group with 'group move' first.
    #[arg(long)]
    pub orphan_children: bool,

    /// Build target to disambiguate which '.xcodeproj' in a workspace to edit.
    #[arg(long)]
    pub target: Option<String>,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
}

/// Flags for `pbxproj group move`.
#[derive(Debug, Args)]
pub struct MoveArgs {
    /// The node to move, named as 'pbxproj fileref list' or 'pbxproj group
    /// list' prints it.
    #[arg(value_name = "NODE")]
    pub address: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Group to move it into. Defaults to the navigator root.
    #[arg(long)]
    pub to: Option<String>,

    /// Build target to disambiguate which '.xcodeproj' in a workspace to edit.
    #[arg(long)]
    pub target: Option<String>,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
}

/// Flags for `pbxproj group attach`/`detach`.
#[derive(Debug, Args)]
pub struct LinkArgs {
    /// The child object's id (a file reference or another group).
    pub id: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// The group whose children list changes, by id or by resolved directory
    /// ('Sources/App'); 'pbxproj group list' shows both.
    #[arg(long)]
    pub group: String,

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
        Action::Move(args) => move_node(ctx, args),
        Action::Attach(args) => link(ctx, args, true),
        Action::Detach(args) => link(ctx, args, false),
    }
}

/// One applied `group` mutation, for the report.
struct GroupMutation {
    line: String,
    json: serde_json::Value,
}

impl Render for GroupMutation {
    fn human(&self, out: &Output) {
        out.line(&self.line);
    }

    fn json(&self) -> serde_json::Value {
        self.json.clone()
    }
}

struct ListResult {
    groups: Vec<GroupRow>,
}

impl Render for ListResult {
    fn human(&self, out: &Output) {
        if self.groups.is_empty() {
            out.line("  (no groups)");
            return;
        }
        for g in &self.groups {
            let dir = display_dir(&g.resolved);
            let title = g.name.as_deref().unwrap_or(dir);
            out.line(&format!(
                "{}  {title}  [{dir}, {} child(ren)]",
                g.address,
                g.children.len()
            ));
        }
    }

    fn json(&self) -> serde_json::Value {
        let groups: Vec<serde_json::Value> = self
            .groups
            .iter()
            .map(|g| {
                serde_json::json!({
                    "address": g.address,
                    "id": g.id,
                    "isa": g.isa,
                    "name": g.name,
                    "path": g.path,
                    "resolved": g.resolved,
                    "sourceTree": g.source_tree,
                    "parent": g.parent,
                    "children": g.children,
                })
            })
            .collect();
        serde_json::json!({ "groups": groups })
    }
}

fn list(ctx: &mut Context, args: &ListArgs) -> CommandResult {
    let (_, document) = super::open_document(ctx, &args.container, None)?;
    let groups = match &document {
        Editable::Pbxproj(root) => tree_pbxproj::list_groups(root),
        Editable::Xcproj(root) => tree_xcproj::list_groups(root),
    }
    .map_err(CliError::new)?;
    Ok(Rendered::data(ListResult { groups }))
}

fn add(ctx: &mut Context, args: &AddArgs) -> CommandResult {
    let targets: Vec<String> = args.target.clone().into_iter().collect();
    let (xcodeproj, mut document) =
        super::open_document_mut(ctx, &args.container, &targets, args.force)?;
    let outcome = match &mut document {
        Editable::Pbxproj(root) => tree_pbxproj::add_group(
            root,
            &args.name,
            args.parent.as_deref(),
            args.path.as_deref(),
            &args.source_tree,
        ),
        Editable::Xcproj(root) => tree_xcproj::add_group(
            root,
            &args.name,
            args.parent.as_deref(),
            args.path.as_deref(),
            &args.source_tree,
        ),
    }
    .map_err(CliError::new)?;

    let under = args.parent.as_deref().unwrap_or("the navigator root");
    let (line, changed, json) = match &outcome {
        AddGroupOutcome::Created { address, resolved } => (
            format!("{address}  {} under {under}", display_dir(resolved)),
            true,
            serde_json::json!({
                "action": "add",
                "address": address,
                "resolved": resolved,
                "parent": args.parent,
                "changed": true,
            }),
        ),
        AddGroupOutcome::AlreadyExists { address, resolved } => (
            format!("{address}  {} (already a group)", display_dir(resolved)),
            false,
            serde_json::json!({
                "action": "add",
                "address": address,
                "resolved": resolved,
                "changed": false,
            }),
        ),
    };
    if changed {
        document.write(&xcodeproj)?;
    }
    Ok(Rendered::data(GroupMutation { line, json }))
}

fn remove(ctx: &mut Context, args: &RemoveArgs) -> CommandResult {
    use std::fmt::Write as _;

    let targets: Vec<String> = args.target.clone().into_iter().collect();
    let (xcodeproj, mut document) =
        super::open_document_mut(ctx, &args.container, &targets, args.force)?;
    let outcome: RemoveOutcome = match &mut document {
        Editable::Pbxproj(root) => {
            tree_pbxproj::remove_group(root, &args.address, args.orphan_children)
        }
        Editable::Xcproj(root) => {
            tree_xcproj::remove_group(root, &args.address, args.orphan_children)
        }
    }
    .map_err(CliError::new)?;
    document.write(&xcodeproj)?;

    let mut line = format!("removed {}", outcome.address);
    if let Some(parent) = &outcome.detached_from {
        let _ = write!(line, "; dropped from group {parent}");
    }
    // Orphans only happen under --orphan-children, and leaving them unsaid
    // would hide objects that nothing now shows.
    if !outcome.orphaned.is_empty() {
        let _ = write!(
            line,
            "; {} child object(s) left unreferenced: {}",
            outcome.orphaned.len(),
            outcome.orphaned.join(", ")
        );
    }
    Ok(Rendered::data(GroupMutation {
        line,
        json: serde_json::json!({
            "action": "remove",
            "address": outcome.address,
            "detachedFrom": outcome.detached_from,
            "orphaned": outcome.orphaned,
            "changed": true,
        }),
    }))
}

fn move_node(ctx: &mut Context, args: &MoveArgs) -> CommandResult {
    let targets: Vec<String> = args.target.clone().into_iter().collect();
    let (xcodeproj, mut document) =
        super::open_document_mut(ctx, &args.container, &targets, args.force)?;
    let outcome = match &mut document {
        Editable::Pbxproj(root) => tree_pbxproj::move_node(root, &args.address, args.to.as_deref()),
        Editable::Xcproj(root) => tree_xcproj::move_node(root, &args.address, args.to.as_deref()),
    }
    .map_err(CliError::new)?;

    let (line, changed, json) = match &outcome {
        MoveOutcome::Moved {
            address,
            from,
            to,
            resolved,
        } => (
            format!("{address} now under {} (still {resolved})", display_dir(to)),
            true,
            serde_json::json!({
                "action": "move",
                "address": address,
                "from": from,
                "to": to,
                "resolved": resolved,
                "changed": true,
            }),
        ),
        MoveOutcome::AlreadyThere { address, group } => (
            format!("{address} is already under {}", display_dir(group)),
            false,
            serde_json::json!({
                "action": "move",
                "address": address,
                "to": group,
                "changed": false,
            }),
        ),
    };
    if changed {
        document.write(&xcodeproj)?;
    }
    Ok(Rendered::data(GroupMutation { line, json }))
}

fn link(ctx: &mut Context, args: &LinkArgs, attach: bool) -> CommandResult {
    let targets: Vec<String> = args.target.clone().into_iter().collect();
    let (xcodeproj, mut document) =
        super::open_document_mut(ctx, &args.container, &targets, args.force)?;
    let root = match &mut document {
        Editable::Pbxproj(root) => root,
        // A node sits in exactly one place here, so there is no "list it here
        // as well" to perform and no "unlist it" that leaves it anywhere.
        Editable::Xcproj(_) => {
            let verb = if attach { "attach" } else { "detach" };
            return Err(CliError::new(format!(
                "`group {verb}` has no meaning in the project.xcproj format: a group holds \
                 its children rather than listing references to them, so a node is in one \
                 place and cannot be in two. Move it with `pbxproj group move {} --to {}`",
                args.id, args.group
            )));
        }
    };
    let outcome = if attach {
        tree_pbxproj::attach(root, &args.id, &args.group)
    } else {
        tree_pbxproj::detach(root, &args.id, &args.group)
    }
    .map_err(CliError::new)?;

    let (line, changed) = match &outcome {
        LinkOutcome::Linked { child, group } => (format!("{group} now lists {child}"), true),
        LinkOutcome::AlreadyLinked { child, group } => {
            (format!("{group} already lists {child}"), false)
        }
        LinkOutcome::Unlinked { child, group } => (
            format!("{group} no longer lists {child} (the object stays)"),
            true,
        ),
        LinkOutcome::NotLinked { child, group } => {
            (format!("{group} does not list {child}"), false)
        }
    };
    if changed {
        document.write(&xcodeproj)?;
    }
    Ok(Rendered::data(GroupMutation {
        line,
        json: serde_json::json!({
            "action": if attach { "attach" } else { "detach" },
            "id": args.id,
            "group": args.group,
            "changed": changed,
        }),
    }))
}

/// The navigator root resolves to the project directory, which prints as an
/// empty string; name it instead of showing nothing.
fn display_dir(resolved: &str) -> &str {
    if resolved.is_empty() {
        "(project root)"
    } else {
        resolved
    }
}
