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
//! whose own `path` is `<PRODUCTS>/App.app`. Two nodes can share a navigator
//! path, as siblings with one display name do, or cousins below two groups
//! with no name. Xcode 27.2 then names the node by `id:` and the `id` it
//! writes on it, and an address takes that spelling too. A navigator path
//! that matches more than one node is an error listing the ids it hit rather
//! than a pick.
//!
//! A group with neither a name nor a path is still a node holding its
//! children. Its display name is empty, so it adds an empty component to the
//! addresses below it. Xcode 27.2 spells them that way when it converts such a
//! project: with one at the root holding `Products`, the product is
//! `/Products/App.app`, and with one inside `Sources` holding `Config`, the
//! xcconfig there is `Sources//Config/Base.xcconfig`.
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
                .map(|c| child_address(Some(&n.address), display_name(c)))
                .collect(),
            parent: n.parent,
            resolved: n.resolved,
            // A node's address is its navigator path, and the document has no
            // node for the navigator root.
            navigator_path: Some(n.address.clone()),
            is_navigator_root: false,
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

    let (indices, base, group_address) = match below_root(root, group)? {
        Some(spec) => {
            let group = find_group(root, spec)?;
            (group.indices, group.resolved, Some(group.address))
        }
        None => (Vec::new(), String::new(), None),
    };
    let address = child_address(group_address.as_deref(), &name);
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

    let (indices, base, parent_address) = match below_root(root, parent)? {
        Some(spec) => {
            let group = find_group(root, spec)?;
            (group.indices, group.resolved, Some(group.address))
        }
        None => (Vec::new(), String::new(), None),
    };
    let display = stored.as_deref().map_or(name, basename);
    let address = child_address(parent_address.as_deref(), display);
    if let Some(existing) = nodes(root)?.into_iter().find(|n| n.address == address) {
        return Ok(AddGroupOutcome::AlreadyExists {
            navigator_path: Some(address.clone()),
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
    Ok(AddGroupOutcome::Created {
        navigator_path: Some(address.clone()),
        address,
        resolved,
    })
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
            "{address} is a group, not a file — delete it with 'pbxproj group remove'"
        ));
    }
    if node.is_folder {
        return Err(format!(
            "{address} is a synchronized folder — detach it with 'pbxproj folder remove'"
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
             'pbxproj membership remove', or pass --dangling to delete the node and its \
             memberships together",
            if members == 1 { "" } else { "s" }
        ));
    }
    let (indices, parent, address) = (node.indices, node.parent, node.address);
    splice_out(root, &indices)?;
    Ok(RemoveOutcome {
        address,
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
            "{address} is a file, not a group — delete it with 'pbxproj fileref remove'"
        ));
    }
    let children = children_of(node.value);
    if !children.is_empty() {
        let hint = if force {
            "--orphan-children cannot apply in the project.xcproj format: these children are \
             nested inside the group rather than listed by it, so deleting it deletes them. \
             Move them out first with 'pbxproj group move'"
        } else {
            "move them out first with 'pbxproj group move'"
        };
        return Err(format!(
            "{address} still holds {} child node(s): {hint}",
            children.len()
        ));
    }
    let (indices, parent, address) = (node.indices, node.parent, node.address);
    splice_out(root, &indices)?;
    Ok(RemoveOutcome {
        address,
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
    let group = below_root(root, group)?;
    let (to_indices, to_base, to_address) = match group {
        Some(spec) => {
            let to = find_group(root, spec)?;
            (to.indices, to.resolved, Some(to.address))
        }
        None => (Vec::new(), String::new(), None),
    };
    if to_indices.starts_with(&node.indices) {
        return Err(format!(
            "{} is inside {address}; a group cannot hold itself",
            to_address.unwrap_or_default()
        ));
    }
    if node.indices.len() == to_indices.len() + 1 && node.indices.starts_with(&to_indices) {
        return Ok(MoveOutcome::AlreadyThere {
            address: node.address,
            group: to_address.unwrap_or_default(),
        });
    }

    let resolved = node.resolved.clone();
    let from = node.parent.clone();
    let name = display_name(node.value).to_string();
    // The move writes a path to keep the node's files, and a node with no
    // name takes its display name from that path.
    if name.is_empty() {
        return Err(format!(
            "'{address}' has neither a name nor a path. Moving it would give it a path, \
             which Xcode shows as its name; move its children instead"
        ));
    }
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
        address: child_address(to_address.as_deref(), &name),
        from,
        to: to_address.unwrap_or_default(),
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
            let address = child_address(parent, display_name(node));
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
///
/// `id:` and a node's `id` names that node, the spelling the document itself
/// uses where a navigator path would name two. Otherwise the address is a
/// navigator path, matched as typed first: below a group with no name it
/// holds an empty component (`/Products`, `Sources//App`) that trimming its
/// slashes would lose. Then it is trimmed, and last it is matched against
/// each address trimmed the same way, so `Products` still finds `/Products`
/// when nothing else is called that.
fn find_node<'a>(root: &'a Value, address: &str) -> Result<Node<'a>, String> {
    let wanted = normalize(address);
    let mut all = nodes(root)?;
    if let Some(index) = position_of_id(&all, address) {
        return Ok(all.swap_remove(index));
    }
    let as_typed = |a: &str| a == address;
    let trimmed = |a: &str| a == wanted;
    let both_trimmed = |a: &str| normalize(a) == wanted;
    let tiers: [&dyn Fn(&str) -> bool; 3] = [&as_typed, &trimmed, &both_trimmed];
    let mut hits: Vec<Node<'a>> = match tiers
        .iter()
        .find(|matches| all.iter().any(|n| matches(&n.address)))
    {
        Some(matches) => all.into_iter().filter(|n| matches(&n.address)).collect(),
        None => Vec::new(),
    };
    match hits.len() {
        1 => Ok(hits.remove(0)),
        0 => Err(missing(root, &wanted)),
        n => {
            let ids: Vec<String> = hits
                .iter()
                .filter_map(|n| node_id(n.value))
                .map(|id| format!("id:{id}"))
                .collect();
            let hint = if ids.is_empty() {
                ", and only Xcode's navigator can tell them apart".to_string()
            } else {
                format!(". Name one by its id: {}", ids.join(", "))
            };
            Err(format!(
                "'{address}' is the navigator path of {n} nodes{hint}"
            ))
        }
    }
}

/// Where the node a reference in the document names resolves to, relative to
/// the project directory, in the spelling [`FileRefRow::resolved`] uses.
///
/// A reference is the node's navigator path, matched exactly, or `id:` and
/// the node's own `id`. Xcode 27.2 writes the second form where the navigator
/// path would name two nodes, as it does below two groups at the root that
/// have no name, and puts the `id` on the node. A configuration's `file` and
/// `anchor`, a target's `product` and the `products-group` all take either
/// form. A reference whose id no node carries is read as a navigator path,
/// since a group's name can itself start with `id:`.
pub(crate) fn resolve_reference(root: &Value, reference: &str) -> Option<String> {
    let all = nodes(root).ok()?;
    position_of_id(&all, reference)
        .or_else(|| all.iter().position(|n| n.address == reference))
        .map(|index| all[index].resolved.clone())
}

/// The node an `id:<id>` spelling names.
fn position_of_id(all: &[Node<'_>], spelling: &str) -> Option<usize> {
    let id = spelling.strip_prefix("id:")?;
    all.iter().position(|n| node_id(n.value) == Some(id))
}

/// The `id` Xcode writes on a node that something names by it.
fn node_id(node: &Value) -> Option<&str> {
    node.get("id").and_then(Value::as_str)
}

/// A group argument, or `None` for the navigator root, which `""` and `/`
/// name here as they name the mainGroup in a `project.pbxproj`.
///
/// A group with no name at the root has an empty address too. There `/` still
/// names the root, and `""` is refused as naming both, the rule a
/// `project.pbxproj` follows.
fn below_root<'a>(root: &Value, spec: Option<&'a str>) -> Result<Option<&'a str>, String> {
    let Some(spec) = spec.filter(|spec| *spec != "/") else {
        return Ok(None);
    };
    if !normalize(spec).is_empty() {
        return Ok(Some(spec));
    }
    if nodes(root)?
        .iter()
        .any(|n| n.is_container && n.address == spec)
    {
        return Err(format!(
            "'{spec}' names both the navigator root and a group with no name at the root; \
             pass '/' for the navigator root"
        ));
    }
    Ok(None)
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
/// saying so is more use than listing every node in the project. A node that
/// carries the id is named by it with `id:` in front.
fn missing(root: &Value, address: &str) -> String {
    if let Some(id) = address.strip_prefix("id:") {
        return format!(
            "no node in the project's navigator tree has the id {id}; 'pbxproj fileref list \
             --json' and 'pbxproj group list --json' print each node's id"
        );
    }
    let all = nodes(root).unwrap_or_default();
    if all.iter().any(|n| node_id(n.value) == Some(address)) {
        return format!(
            "{address} is a node's id; name it as 'id:{address}', since a bare address is a \
             navigator path in the project.xcproj format"
        );
    }
    if address.len() == 24 && address.chars().all(|c| c.is_ascii_hexdigit()) {
        return format!(
            "{address} looks like a pbxproj object id, and this project is in the \
             project.xcproj format, where a node is named by its navigator path (for \
             example 'Sources/App/ContentView.swift') — 'pbxproj fileref list' and \
             'pbxproj group list' print them"
        );
    }
    let near: Vec<String> = all
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
/// to, with the join a `project.pbxproj` node takes
/// ([`crate::project::join_normalized`]). An anchored path is reported in its
/// own spelling, since nothing under the project directory answers it.
fn resolve(base: &str, node: &Value) -> String {
    let Some(path) = stored_path(node) else {
        return base.to_string();
    };
    let (base, path) = match path.strip_prefix("<PROJECT>/") {
        Some(rest) => ("", rest),
        None if path.starts_with('<') => return path.to_string(),
        None => (base, path),
    };
    crate::project::join_normalized(std::path::Path::new(base), path)
        .to_string_lossy()
        .into_owned()
}

/// What Xcode shows the node as: its `name`, else the last component of its
/// `path`, and empty for a node with neither.
pub(crate) fn display_name(node: &Value) -> &str {
    match node.get("name").and_then(Value::as_str) {
        Some(name) => name,
        None => stored_path(node).map_or("", basename),
    }
}

/// The address of a node called `name` under the group at `parent`, or at the
/// navigator root without one. The root adds no component. A group with no
/// name adds an empty one, which is how `/Products` spells a group inside one
/// at the root.
pub(crate) fn child_address(parent: Option<&str>, name: &str) -> String {
    match parent {
        None => name.to_string(),
        Some(parent) => format!("{parent}/{name}"),
    }
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

fn normalize(path: &str) -> String {
    path.trim()
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}
