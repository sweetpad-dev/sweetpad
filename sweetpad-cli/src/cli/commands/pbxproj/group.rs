//! `sweetpad pbxproj group` — the navigator tree on its own: where a file
//! *appears* in Xcode, which says nothing about what builds it (CLI_DESIGN
//! §9g). Membership is [`super::membership`]'s axis.
//!
//! `attach`/`detach` exist because a `PBXGroup`'s `children` is a list of
//! references: an object can exist with no group listing it, and `attach`
//! lists one of those. It refuses one another group lists already, since Xcode
//! 27.2 will not open a project that lists a node in two groups. A
//! `project.xcproj` nests its nodes instead, so a node is in exactly one place
//! and the operation is [`Action::Move`] — which works on both formats. The
//! two verbs that have no meaning there say so rather than quietly doing
//! something near enough.

use clap::{Args, Subcommand};

use crate::cli::output::Output;
use crate::cli::pbxedit::Editable;
use crate::cli::{CliError, CommandResult, ContainerArgs, Context, Render, Rendered};
use sweetpad_lib::tree::{AddGroupOutcome, GroupRow, MoveOutcome, RemoveOutcome, navigator_label};
use sweetpad_lib::tree_pbxproj::{self, LinkOutcome};
use sweetpad_lib::tree_xcproj;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show every group: its address, its navigator path, its resolved
    /// directory, and how many children it holds.
    List(ListArgs),
    /// Create a group under a parent group.
    Add(AddArgs),
    /// Delete an empty group.
    Remove(RemoveArgs),
    /// Move a node into another group, keeping the file it resolves to.
    Move(MoveArgs),
    /// List an object that no group lists yet in a group's children
    /// (project.pbxproj only). Use 'group move' for an object another group
    /// lists.
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
    /// Defaults to the navigator root, which '/' also names.
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

    /// Group to move it into. Defaults to the navigator root, which '/' also
    /// names.
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
    /// The child to list or unlist, a file reference or another group: the
    /// 'address' that 'pbxproj fileref list' or 'pbxproj group list' prints
    /// for it (its object id).
    #[arg(value_name = "NODE")]
    pub address: String,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// The group whose children list changes: its id, its navigator path, or
    /// its resolved directory ('Sources/App'), with '/' for the navigator
    /// root. 'pbxproj group list' prints all three. A navigator path wins
    /// over a directory that reads the same.
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
            out.line(&group_line(g));
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
                    "navigatorPath": g.navigator_path,
                    "sourceTree": g.source_tree,
                    "parent": g.parent,
                    "children": g.children,
                })
            })
            .collect();
        serde_json::json!({ "groups": groups })
    }
}

/// One `group list` row: the address, the navigator path, then the resolved
/// directory and the child count. Each of the three is a spelling a group
/// argument takes. A `project.xcproj` addresses a group by its navigator
/// path, so its rows print that once.
fn group_line(g: &GroupRow) -> String {
    format!(
        "{}  [{}, {} child(ren)]",
        row_head(&g.address, g.navigator_path.as_deref(), g.is_navigator_root),
        display_dir(&g.resolved),
        g.children.len()
    )
}

/// The address and the navigator path that open a group's row, or the path
/// alone where it is the address.
fn row_head(address: &str, navigator_path: Option<&str>, is_root: bool) -> String {
    let navigator = navigator_label(navigator_path, is_root);
    if navigator_path == Some(address) {
        navigator.to_string()
    } else {
        format!("{address}  {navigator}")
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

    // The group as its 'group list' row names it, so the output says which
    // group this is rather than only the directory it resolves to.
    let (line, changed, json) = match &outcome {
        AddGroupOutcome::Created {
            address,
            resolved,
            navigator_path,
        } => (
            format!(
                "{}  [{}]",
                row_head(address, navigator_path.as_deref(), false),
                display_dir(resolved)
            ),
            true,
            serde_json::json!({
                "action": "add",
                "address": address,
                "resolved": resolved,
                "navigatorPath": navigator_path,
                "parent": args.parent,
                "changed": true,
            }),
        ),
        AddGroupOutcome::AlreadyExists {
            address,
            resolved,
            navigator_path,
        } => (
            format!(
                "{}  [{}] (already a group)",
                row_head(address, navigator_path.as_deref(), false),
                display_dir(resolved)
            ),
            false,
            serde_json::json!({
                "action": "add",
                "address": address,
                "resolved": resolved,
                "navigatorPath": navigator_path,
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

    let mut line = format!(
        "removed {}{}",
        outcome.address,
        super::fileref::detached_text(&outcome)
    );
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
            "alsoDetachedFrom": outcome.also_detached_from,
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
            format!(
                "{address} now under {}, still at {}",
                display_group(to),
                display_dir(resolved)
            ),
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
            format!("{address} is already under {}", display_group(group)),
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
                "'group {verb}' has no meaning in the project.xcproj format: a group holds \
                 its children rather than listing references to them, so a node is in one \
                 place and cannot be in two. Move it with 'pbxproj group move {} --to {}'",
                args.address, args.group
            )));
        }
    };
    let outcome = if attach {
        tree_pbxproj::attach(root, &args.address, &args.group)
    } else {
        tree_pbxproj::detach(root, &args.address, &args.group)
    }
    .map_err(CliError::new)?;

    let (line, changed, child, group) = match &outcome {
        LinkOutcome::Linked { child, group } => {
            (format!("{group} now lists {child}"), true, child, group)
        }
        LinkOutcome::AlreadyLinked { child, group } => (
            format!("{group} already lists {child}"),
            false,
            child,
            group,
        ),
        LinkOutcome::Unlinked { child, group } => (
            format!("{group} no longer lists {child} (the object stays)"),
            true,
            child,
            group,
        ),
        LinkOutcome::NotLinked { child, group } => (
            format!("{group} does not list {child}"),
            false,
            child,
            group,
        ),
    };
    if changed {
        document.write(&xcodeproj)?;
    }
    // The child and the group as the rest of the family names them: by
    // address, whatever spelling '--group' was given in.
    let json = serde_json::json!({
        "action": if attach { "attach" } else { "detach" },
        "address": child,
        "group": group,
        "changed": changed,
    });
    Ok(Rendered::data(GroupMutation { line, json }))
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

/// A group's address in a sentence. A project.xcproj has no node for its
/// navigator root, so an outcome gives the root's address as empty.
fn display_group(address: &str) -> &str {
    if address.is_empty() {
        "the navigator root"
    } else {
        address
    }
}
