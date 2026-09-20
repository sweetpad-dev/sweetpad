//! The navigator tree of a parsed [`crate::xcproj::Value`] document — the
//! `project.xcproj` counterpart of [`crate::tree_pbxproj`].
//!
//! A pbxproj names a file twice: a `PBXFileReference` says the file exists, a
//! group's `children` entry says where it appears. Here those are one thing.
//! `files` is a literal nested array, a node *is* its own entry, and nothing
//! points at it — so a node lives in exactly one place and deleting it deletes
//! the file from the project. The three axes the pbxproj keeps apart collapse
//! to two: the tree, and the `target-membership` each node carries
//! ([`crate::membership_xcproj`]).
//!
//! **A node is addressed by its navigator path**: the display names from the
//! root, joined by `/` — `Sources/App/ContentView.swift`. A node's display
//! name is its `name` when it has one, else the last component of its `path`,
//! which is what Xcode shows. The document itself addresses nodes this way: a
//! target's `product` names `Products/App.app`, the navigator path of a node
//! whose own `path` is `<PRODUCTS>/App.app`. Two siblings can in principle
//! share a display name, and an address that matches more than one node is an
//! error listing what it hit rather than a pick.
//!
//! The navigator path is not the on-disk path. `<PROJECT>/Sources/Deep.swift`
//! listed at the root appears as `Deep.swift`, and every row carries both.
//!
//! Synchronized folders (`kind: "folder"`) are navigator nodes but appear in
//! neither listing here — the folder *is* the membership statement, and
//! [`crate::sync_xcproj`] owns it. [`move_node`] still moves one, since where
//! a folder appears is this module's question.
//!
//! Everything here is pure (no I/O): callers parse the file, mutate the tree,
//! and serialize/write it.

pub use crate::tree::{
    AddGroupOutcome, AddRefOutcome, FileRefRow, GroupRow, MoveOutcome, RemoveOutcome,
};

use crate::xcproj::{Array, Object, Value};

/// The node kinds that hold `children`. Variant and version groups list them
/// exactly as a plain group does.
const CONTAINER_KINDS: [&str; 3] = ["group", "variant-group", "version-group"];

/// Every file node in the navigator, in document order.
///
/// # Errors
/// Returns a message when `files` is not an array.
pub fn list_filerefs(root: &Value) -> Result<Vec<FileRefRow>, String> {
    Ok(nodes(root)?
        .into_iter()
        .filter(|n| !n.is_container && !n.is_folder)
        .map(|n| FileRefRow {
            path: stored_path(n.value).unwrap_or_default().to_string(),
            source_tree: source_tree(n.value).to_string(),
            file_type: n
                .value
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string),
            id: n
                .value
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string),
            parent: n.parent,
            resolved: n.resolved,
            build_files: n
                .value
                .get("target-membership")
                .and_then(Value::as_array)
                .map_or(0, <[Value]>::len),
            address: n.address,
        })
        .collect())
}

/// Every group node in the navigator, in document order.
///
/// # Errors
/// Returns a message when `files` is not an array.
pub fn list_groups(root: &Value) -> Result<Vec<GroupRow>, String> {
    Ok(nodes(root)?
        .into_iter()
        .filter(|n| n.is_container)
        .map(|n| GroupRow {
            isa: kind(n.value).unwrap_or("group").to_string(),
            name: n
                .value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            path: stored_path(n.value).map(str::to_string),
            source_tree: source_tree(n.value).to_string(),
            id: n
                .value
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string),
            children: children_of(n.value)
                .iter()
                .filter_map(|c| display_name(c).map(|name| join(&n.address, name)))
                .collect(),
            parent: n.parent,
            resolved: n.resolved,
            address: n.address,
        })
        .collect())
}

/// Add a file node for `path`, anchored at `source_tree`, under `group`.
///
/// There is no separate reference to create: the node is the file's existence
/// in this format, so one call does what `fileref add` plus `group attach`
/// does on a pbxproj. Without `group` the node joins the navigator root.
///
/// `file_type` writes `type`; omitting it leaves the key out, so Xcode derives
/// the type from the extension. Nothing here touches `target-membership`.
///
/// # Errors
/// Returns a message when the document is malformed, `path` is empty,
/// `source_tree` has no spelling in this format, or `group` names no group (or
/// more than one).
pub fn add_fileref(
    root: &mut Value,
    path: &str,
    file_type: Option<&str>,
    source_tree: &str,
    group: Option<&str>,
) -> Result<AddRefOutcome, String> {
    let path = normalize(path);
    if path.is_empty() {
        return Err("the file path must not be empty".to_string());
    }
    let anchor = anchor_for(source_tree)?;
    let stored = anchor.spell(&path);
    let name = basename(&stored).to_string();

    let (indices, base, group_address) = match group {
        Some(spec) => {
            let group = find_group(root, spec)?;
            (group.indices, group.resolved, Some(group.address))
        }
        None => (Vec::new(), String::new(), None),
    };
    let address = join(group_address.as_deref().unwrap_or(""), &name);
    if let Some(existing) = nodes(root)?.into_iter().find(|n| n.address == address) {
        return Ok(AddRefOutcome::AlreadyExists {
            address,
            resolved: existing.resolved,
        });
    }

    // Xcode writes a file node on one line, `path` first and `type` after it.
    let mut node = Object::new();
    node.insert("path".to_string(), Value::String(stored.clone()));
    if let Some(file_type) = file_type {
        node.insert("type".to_string(), Value::String(file_type.to_string()));
    }
    node.set_compact(true);

    let resolved = resolve(&base, &Value::Object(node.clone()));
    children_mut(root, &indices)?.push(Value::Object(node));
    Ok(AddRefOutcome::Created {
        address,
        resolved,
        attached_to: group_address,
    })
}

/// Create a group node under `parent`, or the navigator root without one.
///
/// `name` titles it; `path` is the directory it contributes to its children's
/// resolution (omit it for a purely organizational group). Xcode writes only
/// `path` when the two would say the same thing, and only `name` when the
/// group adds no directory, which is what this writes.
///
/// # Errors
/// Returns a message when the document is malformed, `name` is empty,
/// `source_tree` has no spelling in this format, or `parent` names no group
/// (or more than one).
pub fn add_group(
    root: &mut Value,
    name: &str,
    parent: Option<&str>,
    path: Option<&str>,
    source_tree: &str,
) -> Result<AddGroupOutcome, String> {
    if name.trim().is_empty() {
        return Err("the group name must not be empty".to_string());
    }
    let anchor = anchor_for(source_tree)?;
    let stored = path.map(|p| anchor.spell(&normalize(p)));

    let (indices, base, parent_address) = match parent {
        Some(spec) => {
            let group = find_group(root, spec)?;
            (group.indices, group.resolved, Some(group.address))
        }
        None => (Vec::new(), String::new(), None),
    };
    let display = stored.as_deref().map_or(name, basename);
    let address = join(parent_address.as_deref().unwrap_or(""), display);
    if let Some(existing) = nodes(root)?.into_iter().find(|n| n.address == address) {
        return Ok(AddGroupOutcome::AlreadyExists {
            address,
            resolved: existing.resolved,
        });
    }

    let mut node = Object::new();
    node.insert("kind".to_string(), Value::String("group".to_string()));
    // A `name` that only repeats the directory is noise Xcode does not write,
    // and a group with no directory is named rather than pathed.
    if display != name || stored.is_none() {
        node.insert("name".to_string(), Value::String(name.to_string()));
    }
    if let Some(stored) = &stored {
        node.insert("path".to_string(), Value::String(stored.clone()));
    }
    node.insert("children".to_string(), Value::Array(Array::new()));

    let resolved = resolve(&base, &Value::Object(node.clone()));
    children_mut(root, &indices)?.push(Value::Object(node));
    Ok(AddGroupOutcome::Created { address, resolved })
}

/// Delete a file node.
///
/// Refuses while the node still carries memberships unless `force`. A pbxproj
/// would leave dangling build files behind; here the memberships live on the
/// node and go with it, which is a deletion the caller should be asked about
/// rather than one to discover in a diff.
///
/// # Errors
/// Returns a message when the document is malformed, `address` names no node
/// (or more than one), it is a group or a synchronized folder, or it has
/// memberships and `force` is false.
pub fn remove_fileref(
    root: &mut Value,
    address: &str,
    force: bool,
) -> Result<RemoveOutcome, String> {
    let node = find_node(root, address)?;
    if node.is_container {
        return Err(format!(
            "{address} is a group, not a file — delete it with `pbxproj group remove`"
        ));
    }
    if node.is_folder {
        return Err(format!(
            "{address} is a synchronized folder — detach it with `pbxproj folder remove`"
        ));
    }
    let members = node
        .value
        .get("target-membership")
        .and_then(Value::as_array)
        .map_or(0, <[Value]>::len);
    if members > 0 && !force {
        return Err(format!(
            "{address} is still built by {members} target membership{}: drop them with \
             `pbxproj membership remove`, or pass --dangling to delete the node and its \
             memberships together",
            if members == 1 { "" } else { "s" }
        ));
    }
    let (indices, parent) = (node.indices, node.parent);
    splice_out(root, &indices)?;
    Ok(RemoveOutcome {
        address: address.to_string(),
        detached_from: parent,
        orphaned: Vec::new(),
    })
}

/// Delete a group node.
///
/// Refuses while it still holds children. `force` has nothing to offer here:
/// a pbxproj can orphan children because they are separate objects the group
/// merely listed, while these are nested inside it and a delete takes them
/// with it. Moving them out first is [`move_node`].
///
/// # Errors
/// Returns a message when the document is malformed, `address` names no node
/// (or more than one), it is not a group, or it still holds children.
pub fn remove_group(root: &mut Value, address: &str, force: bool) -> Result<RemoveOutcome, String> {
    let node = find_node(root, address)?;
    if !node.is_container {
        return Err(format!(
            "{address} is a file, not a group — delete it with `pbxproj fileref remove`"
        ));
    }
    let children: Vec<String> = children_of(node.value)
        .iter()
        .filter_map(|c| display_name(c).map(|name| join(address, name)))
        .collect();
    if !children.is_empty() {
        let hint = if force {
            "--orphan-children cannot apply in the project.xcproj format: these children are \
             nested inside the group rather than listed by it, so deleting it deletes them. \
             Move them out first with `pbxproj group move`"
        } else {
            "move them out first with `pbxproj group move`"
        };
        return Err(format!(
            "{address} still holds {} child node(s): {hint}",
            children.len()
        ));
    }
    let (indices, parent) = (node.indices, node.parent);
    splice_out(root, &indices)?;
    Ok(RemoveOutcome {
        address: address.to_string(),
        detached_from: parent,
        orphaned: Vec::new(),
    })
}

/// Move a node into `group`, keeping the file it resolves to.
///
/// This is `group attach`/`detach`'s counterpart: a node sits in exactly one
/// place here, so listing it somewhere else is moving it. A `<group>`-relative
/// stored path means a different file under a different group, so it is
/// rewritten — the new group's directory stripped off when it prefixes the
/// resolved path, the `<PROJECT>` anchor when it does not. A descending
/// relative path is what Xcode writes where one reaches the file; where none
/// does it has both spellings available, and the anchor is the one that does
/// not depend on how deep the group sits. An already-anchored path ignores the
/// group chain and is left alone.
///
/// # Errors
/// Returns a message when the document is malformed, either address names no
/// node (or more than one), or the move would put a group inside itself.
pub fn move_node(
    root: &mut Value,
    address: &str,
    group: Option<&str>,
) -> Result<MoveOutcome, String> {
    let node = find_node(root, address)?;
    let (to_indices, to_base, to_address) = match group {
        Some(spec) => {
            let to = find_group(root, spec)?;
            (to.indices, to.resolved, to.address)
        }
        None => (Vec::new(), String::new(), String::new()),
    };
    if to_indices.starts_with(&node.indices) {
        return Err(format!(
            "{to_address} is inside {address}; a group cannot hold itself"
        ));
    }
    if node.indices.len() == to_indices.len() + 1 && node.indices.starts_with(&to_indices) {
        return Ok(MoveOutcome::AlreadyThere {
            address: address.to_string(),
            group: to_address,
        });
    }

    let resolved = node.resolved.clone();
    let from = node.parent.clone();
    let name = display_name(node.value)
        .ok_or_else(|| format!("{address} has neither a name nor a path"))?
        .to_string();
    let relative = source_tree(node.value) == "<group>";
    let indices = node.indices.clone();

    let mut moved = splice_out(root, &indices)?;
    if relative {
        let stored = reanchor(&resolved, &to_base);
        moved
            .as_object_mut()
            .ok_or_else(|| format!("{address} is not an object"))?
            .insert("path".to_string(), Value::String(stored));
    }
    // The target's own indices shift when the node came out of an earlier
    // sibling of the same container, so re-resolve rather than reusing them.
    let to_indices = match group {
        Some(spec) => find_group(root, spec)?.indices,
        None => Vec::new(),
    };
    children_mut(root, &to_indices)?.push(moved);
    Ok(MoveOutcome::Moved {
        address: join(&to_address, &name),
        from,
        to: to_address,
        resolved,
    })
}

/// How to spell `resolved` from inside a group at `group_dir`: relative when
/// the directory contains it, anchored at the project root when it does not.
fn reanchor(resolved: &str, group_dir: &str) -> String {
    if group_dir.is_empty() {
        return resolved.to_string();
    }
    match resolved.strip_prefix(&format!("{group_dir}/")) {
        Some(rest) => rest.to_string(),
        None => format!("<PROJECT>/{resolved}"),
    }
}

/// A node in the navigator, with how to reach it and where it lives.
struct Node<'a> {
    value: &'a Value,
    /// The child index at each level, from the `files` root.
    indices: Vec<usize>,
    /// The navigator path: display names joined by `/`.
    address: String,
    /// The holding group's address, when the node is not at the root.
    parent: Option<String>,
    /// The on-disk path, or the anchored spelling when it is not under the
    /// project directory.
    resolved: String,
    is_container: bool,
    is_folder: bool,
}

/// Every node in the navigator, parents before children.
fn nodes(root: &Value) -> Result<Vec<Node<'_>>, String> {
    fn walk<'a>(
        children: &'a [Value],
        parent: Option<&str>,
        base: &str,
        indices: &mut Vec<usize>,
        depth: usize,
        out: &mut Vec<Node<'a>>,
    ) {
        if depth >= crate::project::MAX_GROUP_DEPTH {
            return;
        }
        for (index, node) in children.iter().enumerate() {
            let Some(name) = display_name(node) else {
                continue;
            };
            let address = join(parent.unwrap_or(""), name);
            let resolved = resolve(base, node);
            let is_container = kind(node).is_some_and(|k| CONTAINER_KINDS.contains(&k));
            indices.push(index);
            out.push(Node {
                value: node,
                indices: indices.clone(),
                address: address.clone(),
                parent: parent.map(str::to_string),
                resolved: resolved.clone(),
                is_container,
                is_folder: kind(node) == Some("folder"),
            });
            if is_container {
                walk(
                    children_of(node),
                    Some(&address),
                    &resolved,
                    indices,
                    depth + 1,
                    out,
                );
            }
            indices.pop();
        }
    }

    let files = match root.get("files") {
        None => return Ok(Vec::new()),
        Some(files) => files
            .as_array()
            .ok_or_else(|| "the document's files is not an array".to_string())?,
    };
    let mut out = Vec::new();
    walk(files, None, "", &mut Vec::new(), 0, &mut out);
    Ok(out)
}

/// The one node at `address`, or a message saying why not.
fn find_node<'a>(root: &'a Value, address: &str) -> Result<Node<'a>, String> {
    let wanted = normalize(address);
    let mut hits: Vec<Node<'a>> = nodes(root)?
        .into_iter()
        .filter(|n| n.address == wanted)
        .collect();
    match hits.len() {
        1 => Ok(hits.remove(0)),
        0 => Err(missing(root, &wanted)),
        n => Err(format!(
            "{wanted} matches {n} navigator nodes; the document holds siblings sharing a \
             display name, which only Xcode's navigator can tell apart"
        )),
    }
}

/// The one group at `address`.
fn find_group<'a>(root: &'a Value, address: &str) -> Result<Node<'a>, String> {
    let node = find_node(root, address)?;
    if node.is_container {
        Ok(node)
    } else if node.is_folder {
        Err(format!(
            "{address} is a synchronized folder: its contents come from the disk, so the \
             document cannot list a node inside it"
        ))
    } else {
        Err(format!("{address} is a file, not a group"))
    }
}

/// Why an address found nothing. An id-shaped one is the pbxproj habit, and
/// saying so is more use than listing every node in the project.
fn missing(root: &Value, address: &str) -> String {
    if address.len() == 24 && address.chars().all(|c| c.is_ascii_hexdigit()) {
        return format!(
            "{address} looks like a pbxproj object id, and this project is in the \
             project.xcproj format, where a node is named by its navigator path (for \
             example `Sources/App/ContentView.swift`) — `pbxproj fileref list` and \
             `pbxproj group list` print them"
        );
    }
    let near: Vec<String> = nodes(root)
        .unwrap_or_default()
        .into_iter()
        .map(|n| n.address)
        .filter(|a| {
            basename(a).eq_ignore_ascii_case(basename(address))
                || a.ends_with(&format!("/{address}"))
        })
        .take(4)
        .collect();
    if near.is_empty() {
        format!("{address} is not in the project's navigator tree")
    } else {
        format!(
            "{address} is not in the project's navigator tree; did you mean {}?",
            near.join(", ")
        )
    }
}

/// The array holding the children of the container at `indices` (the `files`
/// root when empty), creating the key on a group that has none.
fn children_mut<'a>(root: &'a mut Value, indices: &[usize]) -> Result<&'a mut Array, String> {
    let mut array = root
        .get_mut("files")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "the document has no files array".to_string())?;
    for &index in indices {
        let node = array
            .get_mut(index)
            .ok_or_else(|| "the navigator node moved during the edit".to_string())?;
        if node.get("children").is_none() {
            crate::schema_xcproj::insert_node_key(
                node.as_object_mut()
                    .ok_or_else(|| "navigator node is not an object".to_string())?,
                "children",
                Value::Array(Array::new()),
            );
        }
        array = node
            .get_mut("children")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "navigator children is not an array".to_string())?;
    }
    Ok(array)
}

/// Take the node at `indices` out of the tree and hand it back.
fn splice_out(root: &mut Value, indices: &[usize]) -> Result<Value, String> {
    let (&last, parent) = indices
        .split_last()
        .ok_or_else(|| "the navigator root is not a node".to_string())?;
    let children = children_mut(root, parent)?;
    if last >= children.len() {
        return Err("the navigator node moved during the edit".to_string());
    }
    Ok(children.remove(last))
}

/// Where a path is anchored, in the two vocabularies.
enum Anchor {
    /// Relative to the holding group.
    Group,
    Absolute,
    Token(&'static str),
}

impl Anchor {
    fn spell(&self, path: &str) -> String {
        match self {
            Anchor::Group | Anchor::Absolute => path.to_string(),
            Anchor::Token(token) => format!("{token}/{path}"),
        }
    }
}

/// Read a `--source-tree` argument, in either format's spelling.
fn anchor_for(source_tree: &str) -> Result<Anchor, String> {
    match source_tree {
        "<group>" => Ok(Anchor::Group),
        "<absolute>" => Ok(Anchor::Absolute),
        "SOURCE_ROOT" | "<PROJECT>" => Ok(Anchor::Token("<PROJECT>")),
        "BUILT_PRODUCTS_DIR" | "<PRODUCTS>" => Ok(Anchor::Token("<PRODUCTS>")),
        "SDKROOT" | "<SDK>" => Ok(Anchor::Token("<SDK>")),
        "DEVELOPER_DIR" | "<DEVELOPER>" => Ok(Anchor::Token("<DEVELOPER>")),
        other => Err(format!(
            "{other} has no spelling in the project.xcproj format, which anchors a path at \
             <PROJECT>, <PRODUCTS>, <SDK> or <DEVELOPER>, relative to its group (<group>), \
             or absolute (<absolute>)"
        )),
    }
}

/// The anchor a stored path carries, in the `sourceTree` vocabulary both
/// formats' listings print.
fn source_tree(node: &Value) -> &'static str {
    let Some(path) = stored_path(node) else {
        return "<group>";
    };
    match path.split_once('/').map_or(path, |(head, _)| head) {
        "<PROJECT>" => "<PROJECT>",
        "<PRODUCTS>" => "<PRODUCTS>",
        "<SDK>" => "<SDK>",
        "<DEVELOPER>" => "<DEVELOPER>",
        head if head.starts_with('<') => "<anchored>",
        "" => "<absolute>",
        _ => "<group>",
    }
}

/// A node's on-disk path, resolved against the directory its group resolves
/// to. An anchored path is reported in its own spelling, since nothing under
/// the project directory answers it.
fn resolve(base: &str, node: &Value) -> String {
    let Some(path) = stored_path(node) else {
        return base.to_string();
    };
    if let Some(rest) = path.strip_prefix("<PROJECT>/") {
        return normalize(rest);
    }
    if path.starts_with('<') || path.starts_with('/') {
        return path.to_string();
    }
    join(base, &normalize(path))
}

/// What Xcode shows the node as: its `name`, else the last component of its
/// `path`.
fn display_name(node: &Value) -> Option<&str> {
    if let Some(name) = node.get("name").and_then(Value::as_str) {
        return (!name.is_empty()).then_some(name);
    }
    let path = stored_path(node)?;
    let name = basename(path);
    (!name.is_empty()).then_some(name)
}

fn stored_path(node: &Value) -> Option<&str> {
    node.get("path").and_then(Value::as_str)
}

fn kind(node: &Value) -> Option<&str> {
    node.get("kind").and_then(Value::as_str)
}

fn children_of(node: &Value) -> &[Value] {
    node.get("children")
        .and_then(Value::as_array)
        .unwrap_or(&[])
}

fn basename(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit_once('/')
        .map_or(path, |(_, name)| name)
}

fn join(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else {
        format!("{base}/{name}")
    }
}

fn normalize(path: &str) -> String {
    path.trim()
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}
