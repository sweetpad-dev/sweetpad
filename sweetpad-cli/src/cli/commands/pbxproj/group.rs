//! `sweetpad pbxproj group` — the `PBXGroup` tree on its own: the navigator
//! structure, which says where a file *appears* in Xcode and nothing about
//! what builds (CLI_DESIGN §9g). Membership is
//! [`super::membership`]'s axis.

use clap::{Args, Subcommand};

use crate::cli::output::Output;
use crate::cli::pbxedit;
use crate::cli::{CliError, CommandResult, ContainerArgs, Context, Render, Rendered};
use sweetpad_lib::tree_pbxproj::{self, AddGroupOutcome, LinkOutcome};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show every group: id, resolved directory, and its children.
    List(ListArgs),
    /// Create a group under a parent group.
    Add(AddArgs),
    /// Delete an empty group by id.
    Remove(RemoveArgs),
    /// List an existing object in a group's children.
    Attach(LinkArgs),
    /// Drop an object from a group's children, leaving the object itself.
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

    /// Parent group id to create it under (from 'pbxproj group list').
    #[arg(long)]
    pub parent: String,

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
    /// The group's object id.
    pub id: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Delete even while the group still lists children, leaving them in the
    /// project with nothing showing them. Emptying it is 'group detach'.
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

/// Flags for `pbxproj group attach`/`detach`.
#[derive(Debug, Args)]
pub struct LinkArgs {
    /// The child object's id (a file reference or another group).
    pub id: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// The group whose children list changes.
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
    groups: Vec<tree_pbxproj::GroupRow>,
}

impl Render for ListResult {
    fn human(&self, out: &Output) {
        if self.groups.is_empty() {
            out.line("  (no groups)");
            return;
        }
        for g in &self.groups {
            let dir = if g.resolved.is_empty() {
                "(project root)"
            } else {
                &g.resolved
            };
            let title = g.name.as_deref().unwrap_or(dir);
            out.line(&format!(
                "{}  {title}  [{dir}, {} child(ren)]",
                g.guid,
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
                    "id": g.guid,
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
    let (_, root) = super::open_project(ctx, &args.container, None)?;
    let groups = tree_pbxproj::list_groups(&root).map_err(CliError::new)?;
    Ok(Rendered::data(ListResult { groups }))
}

fn add(ctx: &mut Context, args: &AddArgs) -> CommandResult {
    let (xcodeproj, mut root) =
        super::open_project_mut(ctx, &args.container, args.target.as_ref(), args.force)?;
    let outcome = tree_pbxproj::add_group(
        &mut root,
        &args.name,
        &args.parent,
        args.path.as_deref(),
        &args.source_tree,
    )
    .map_err(CliError::new)?;

    let (line, changed, json) = match &outcome {
        AddGroupOutcome::Created { guid, resolved } => (
            format!("{guid}  {} under {}", display_dir(resolved), args.parent),
            true,
            serde_json::json!({
                "action": "add",
                "id": guid,
                "resolved": resolved,
                "parent": args.parent,
                "changed": true,
            }),
        ),
        AddGroupOutcome::AlreadyExists { guid, resolved } => (
            format!("{guid}  {} (already a group)", display_dir(resolved)),
            false,
            serde_json::json!({
                "action": "add",
                "id": guid,
                "resolved": resolved,
                "changed": false,
            }),
        ),
    };
    if changed {
        pbxedit::write_pbxproj(&xcodeproj, &root)?;
    }
    Ok(Rendered::data(GroupMutation { line, json }))
}

fn remove(ctx: &mut Context, args: &RemoveArgs) -> CommandResult {
    use std::fmt::Write as _;

    let (xcodeproj, mut root) =
        super::open_project_mut(ctx, &args.container, args.target.as_ref(), args.force)?;
    let outcome = tree_pbxproj::remove_group(&mut root, &args.id, args.orphan_children)
        .map_err(CliError::new)?;
    pbxedit::write_pbxproj(&xcodeproj, &root)?;

    let mut line = format!("removed {}", outcome.guid);
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
            "id": outcome.guid,
            "detachedFrom": outcome.detached_from,
            "orphaned": outcome.orphaned,
            "changed": true,
        }),
    }))
}

fn link(ctx: &mut Context, args: &LinkArgs, attach: bool) -> CommandResult {
    let (xcodeproj, mut root) =
        super::open_project_mut(ctx, &args.container, args.target.as_ref(), args.force)?;
    let outcome = if attach {
        tree_pbxproj::attach(&mut root, &args.id, &args.group)
    } else {
        tree_pbxproj::detach(&mut root, &args.id, &args.group)
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
        pbxedit::write_pbxproj(&xcodeproj, &root)?;
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

/// The mainGroup resolves to the project directory, which prints as an empty
/// string; name it instead of showing nothing.
fn display_dir(resolved: &str) -> &str {
    if resolved.is_empty() {
        "(project root)"
    } else {
        resolved
    }
}
