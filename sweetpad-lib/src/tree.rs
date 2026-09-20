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
//! file has one. There is nothing to address a file *with* except where it
//! sits.
//!
//! So [`FileRefRow::address`] is the id in one format and the navigator path
//! (`Sources/App/ContentView.swift`) in the other, and every verb takes back
//! what its listing printed. The document's own id, when it has one, rides
//! along in `id` rather than being invented where Xcode writes none.

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

/// What `add_group` did.
#[derive(Debug, PartialEq, Eq)]
pub enum AddGroupOutcome {
    Created { address: String, resolved: String },
    AlreadyExists { address: String, resolved: String },
}

/// What `remove_fileref` or `remove_group` did.
#[derive(Debug, PartialEq, Eq)]
pub struct RemoveOutcome {
    pub address: String,
    /// The group that stopped holding it, when it had one.
    pub detached_from: Option<String>,
    /// Children the removed group still listed. A `project.pbxproj` leaves
    /// them in `objects` as unreferenced nodes; a `project.xcproj` cannot,
    /// which is why it refuses the case instead.
    pub orphaned: Vec<String>,
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
