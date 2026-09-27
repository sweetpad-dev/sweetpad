//! The navigator vocabulary both project document formats share: the file
//! nodes, the groups that hold them, and what the editing verbs did.
//!
//! The two formats disagree about how a node is *named*, and that disagreement
//! is the reason this module exists rather than one reader's types serving
//! both. A `project.pbxproj` keeps every node in a flat `objects` dict under a
//! 24-hex id, and a group's `children` is a list of references to those ids, so
//! an id names a node wherever it is listed — and the same node can be listed
//! twice. A `project.xcproj` has no such dict: the navigator is a literal
//! nested array and a node is its own entry. Across 80 converted corpus
//! documents holding 1,903 file nodes, the 192 ids under `files` all sit on a
//! `<PRODUCTS>/…` node, the product a target points at; no ordinary source
//! file has one. Xcode 27.2 also writes one on a node that a reference has to
//! name as `id:<id>` because another node shares its navigator path. Past
//! those, there is nothing to address a file *with* except where it sits.
//!
//! So [`FileRefRow::address`] is the id in one format and the navigator path
//! (`Sources/App/ContentView.swift`) in the other, and every verb takes back
//! what its listing printed. The document's own id, when it has one, rides
//! along in `id` rather than being invented where Xcode writes none, and
//! `id:<id>` names that node too.

/// One file node, as `fileref list` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRefRow {
    /// What names this node to the other verbs: the object id in a
    /// `project.pbxproj`, the navigator path in a `project.xcproj`.
    pub address: String,
    /// The document's own id for the node, where it writes one.
    pub id: Option<String>,
    /// The stored `path`, verbatim.
    pub path: String,
    /// Where `path` is anchored (`<group>`, `SOURCE_ROOT`, `<absolute>`, …).
    pub source_tree: String,
    /// The file type the document records, when it records one.
    pub file_type: Option<String>,
    /// The group holding this node, when one does.
    pub parent: Option<String>,
    /// `path` resolved through `source_tree` and the group chain.
    pub resolved: String,
    /// How many build entries name this file.
    pub build_files: usize,
}

/// One group node, as `group list` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRow {
    pub address: String,
    pub id: Option<String>,
    /// The node's kind, in the spelling its own format uses (`PBXGroup`,
    /// `group`, `variant-group`, …).
    pub isa: String,
    /// `name` when set — a group can be titled independently of its directory.
    pub name: Option<String>,
    pub path: Option<String>,
    pub source_tree: String,
    pub parent: Option<String>,
    pub children: Vec<String>,
    /// The group's directory, resolved up the chain.
    pub resolved: String,
    /// The display names from the navigator root down, joined by `/`
    /// (`Sources/App`): what Xcode shows, and a spelling every group argument
    /// takes. Empty for the navigator root itself, which `/` also names, and
    /// `None` for a group that no group in the navigator lists. A group with
    /// neither a name nor a path adds an empty component, as Xcode spells it
    /// (`/Products` under one at the root).
    pub navigator_path: Option<String>,
    /// Whether this is the navigator root itself: the mainGroup of a
    /// `project.pbxproj`. A `project.xcproj` has no node for its root, so none
    /// of its rows is. A group with no name at the root has an empty
    /// navigator path as well, and this tells the two apart.
    pub is_navigator_root: bool,
}

/// What Xcode shows a node as, in either format: its `name`, else the last
/// component of its `path`, and empty for a node with neither. The navigator
/// path is these joined by `/`, so two siblings that show the same name share
/// one.
pub(crate) fn display_name<'a>(name: Option<&'a str>, path: Option<&'a str>) -> &'a str {
    match (name, path) {
        (Some(name), _) => name,
        (None, Some(path)) => {
            let path = path.trim_end_matches('/');
            path.rsplit_once('/').map_or(path, |(_, last)| last)
        }
        (None, None) => "",
    }
}

/// How a listing shows a navigator path: the path itself, or a label where
/// the path is empty or missing. The navigator root shows `/`, the spelling
/// that always selects it, with a label beside it. `is_root` tells the root
/// from a group at the root with no name, whose path is empty too.
#[must_use]
pub fn navigator_label(path: Option<&str>, is_root: bool) -> &str {
    match path {
        Some("") if is_root => "/ (navigator root)",
        Some("") => "(unnamed)",
        Some(path) => path,
        None => "(not in the navigator)",
    }
}

/// What `add_fileref` did.
#[derive(Debug, PartialEq, Eq)]
pub enum AddRefOutcome {
    /// A new node, with the on-disk path it resolves to.
    Created {
        address: String,
        resolved: String,
        attached_to: Option<String>,
    },
    /// The document already holds this file; nothing was written.
    AlreadyExists { address: String, resolved: String },
}

/// What `add_group` did. `navigator_path` is the group's, as
/// [`GroupRow::navigator_path`] carries it.
#[derive(Debug, PartialEq, Eq)]
pub enum AddGroupOutcome {
    Created {
        address: String,
        resolved: String,
        navigator_path: Option<String>,
    },
    AlreadyExists {
        address: String,
        resolved: String,
        navigator_path: Option<String>,
    },
}

/// What `remove_fileref` or `remove_group` did.
#[derive(Debug, PartialEq, Eq)]
pub struct RemoveOutcome {
    pub address: String,
    /// The group that stopped holding it, when it had one.
    pub detached_from: Option<String>,
    /// Other groups that listed it as well, which only a malformed
    /// `project.pbxproj` has: Xcode 27.2 refuses to open one, and 27.0 keeps
    /// the listing in `detached_from`. The delete takes every listing with it.
    pub also_detached_from: Vec<String>,
    /// Children the removed group still listed. A `project.pbxproj` leaves
    /// them in `objects` as unreferenced nodes; a `project.xcproj` cannot,
    /// which is why it refuses the case instead.
    pub orphaned: Vec<String>,
}

/// The stored path a moved node needs to keep resolving where it does, when
/// its path is relative to its group. Both formats' `move_node` write this,
/// each in its own spelling of the anchor.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MovedPath {
    /// Nothing to write: the node has no path, and the new group's directory
    /// is the one it already resolves to.
    Unchanged,
    /// A path relative to the new group, whose directory holds the node. An
    /// empty one means the node needs no path there, and loses the one it has.
    InGroup(String),
    /// A path from the project directory, where the new group's does not.
    FromProject(String),
}

/// How a node that resolves to `resolved` is spelled inside a group whose
/// directory is `group_dir`. `has_path` says whether it stores a path, and
/// `has_name` whether it stores a name, which then shows in place of one.
///
/// A path from the group is what Xcode writes where one reaches the node. Where
/// none does, it has both spellings available, and the anchor at the project is
/// the one that does not depend on how deep the group sits. A named node that
/// resolves to the group's own directory needs no path at all, which is how an
/// organizational group moved out and back gets its original spelling back.
pub(crate) fn moved_path(
    resolved: &str,
    group_dir: &str,
    has_path: bool,
    has_name: bool,
) -> MovedPath {
    if resolved == group_dir {
        if !has_path {
            return MovedPath::Unchanged;
        }
        if has_name {
            return MovedPath::InGroup(String::new());
        }
    }
    if group_dir.is_empty() {
        return MovedPath::InGroup(resolved.to_string());
    }
    match resolved.strip_prefix(&format!("{group_dir}/")) {
        Some(rest) => MovedPath::InGroup(rest.to_string()),
        None => MovedPath::FromProject(resolved.to_string()),
    }
}

/// The refusal for moving a node with neither a name nor a path somewhere a
/// path would have to be written: Xcode shows that path as the node's name.
pub(crate) fn nameless_move_refusal(address: &str) -> String {
    format!(
        "'{address}' has neither a name nor a path. Moving it would give it a path, which \
         Xcode shows as its name; move its children instead"
    )
}

/// The refusal for deleting a node the rest of the document still names, in
/// either format. `named_as` says what each name is, after "is still": the
/// xcconfig a configuration is based on, the product of a target.
pub(crate) fn still_named_refusal(node: &str, named_as: &[String]) -> String {
    let those = if named_as.len() == 1 {
        "that reference"
    } else {
        "those references"
    };
    format!(
        "{node} is still {}; deleting it would leave {those} naming nothing",
        named_as.join(", and ")
    )
}

/// What `move_node` did.
#[derive(Debug, PartialEq, Eq)]
pub enum MoveOutcome {
    Moved {
        /// The node's address after the move.
        address: String,
        from: Option<String>,
        to: String,
        /// The on-disk path it still resolves to.
        resolved: String,
    },
    /// The node is already in that group; nothing was written.
    AlreadyThere { address: String, group: String },
}
