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

use crate::tree::{MovedPath, moved_path, nameless_move_refusal};
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
    if let Some(existing) = existing_at(root, &address, false)? {
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
    // The group shows as `name` whichever of the two keys carries it.
    let address = child_address(parent_address.as_deref(), name);
    if let Some(existing) = existing_at(root, &address, true)? {
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
    if stored
        .as_deref()
        .is_none_or(|stored| basename(stored) != name)
    {
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
    if let Some(refusal) = named_by_refusal(root, &address, &indices)? {
        return Err(refusal);
    }
    splice_out(root, &indices)?;
    Ok(RemoveOutcome {
        address,
        detached_from: parent,
        also_detached_from: Vec::new(),
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
    if let Some(refusal) = named_by_refusal(root, &address, &indices)? {
        return Err(refusal);
    }
    splice_out(root, &indices)?;
    Ok(RemoveOutcome {
        address,
        detached_from: parent,
        also_detached_from: Vec::new(),
        orphaned: Vec::new(),
    })
}

/// Move a node into `group`, keeping the file it resolves to.
///
/// This is `group attach`/`detach`'s counterpart: a node sits in exactly one
/// place here, so listing it somewhere else is moving it. A `<group>`-relative
/// stored path means a different file under a different group, so it is
/// rewritten the way [`crate::tree::moved_path`] spells it, with the
/// `<PROJECT>` anchor where the new group's directory does not hold it. An
/// already-anchored path ignores the group chain and is left alone. A node with
/// neither a name nor a path moves only where no path has to be written, since
/// Xcode would show that path as its name.
///
/// The document names nodes by navigator path: a configuration's xcconfig, a
/// target's product, the products group. A move changes the path of every
/// node it takes, so each such reference into the moved subtree is rewritten
/// to follow it, and Xcode still opens the project. A move onto a path
/// another node already has is refused, as an add there is.
///
/// # Errors
/// Returns a message when the document is malformed, either address names no
/// node (or more than one), the move would put a group inside itself, it
/// would have to write a path on a node with no name, or another node already
/// has the navigator path the node would get.
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
    let rewrite = if source_tree(node.value) == "<group>" {
        let has_name = node.value.get("name").is_some();
        match moved_path(
            &resolved,
            &to_base,
            stored_path(node.value).is_some(),
            has_name,
        ) {
            MovedPath::Unchanged => None,
            MovedPath::InGroup(path) => Some(path),
            MovedPath::FromProject(path) => Some(format!("<PROJECT>/{path}")),
        }
    } else {
        None
    };
    // A node with no name takes its display name from the path a move writes.
    if rewrite.is_some() && name.is_empty() {
        return Err(nameless_move_refusal(address));
    }
    let new_address = child_address(to_address.as_deref(), &name);
    if let Some(refusal) = taken(
        root,
        &new_address,
        kind_noun(&node),
        "move it under another group",
    )? {
        return Err(refusal);
    }
    let indices = node.indices.clone();
    let follow = following(root, &indices, &node.address, &new_address)?;

    let before = root.clone();
    let mut moved = splice_out(root, &indices)?;
    if let Some(stored) = rewrite {
        let moved = moved
            .as_object_mut()
            .ok_or_else(|| format!("{address} is not an object"))?;
        if stored.is_empty() {
            moved.remove("path");
        } else {
            crate::schema_xcproj::insert_node_key(moved, "path", Value::String(stored));
        }
    }
    // The target's own indices shift when the node came out of an earlier
    // sibling of the same container, so re-resolve rather than reusing them.
    let to_indices = match group {
        Some(spec) => find_group(root, spec)?.indices,
        None => Vec::new(),
    };
    children_mut(root, &to_indices)?.push(moved);
    if let Some(what) = rewrite_followed(root, &follow)? {
        *root = before;
        return Err(format!(
            "moving {address} would leave {what} naming more than one node; move it under \
             another group"
        ));
    }
    Ok(MoveOutcome::Moved {
        address: new_address,
        from,
        to: to_address.unwrap_or_default(),
        resolved,
    })
}

/// A reference a move takes along: where it sits, the navigator path it
/// gets, and how many nodes shared the path it had.
struct Follow {
    reference: Reference,
    spelling: String,
    sharing: usize,
}

/// The references naming the subtree at `indices` by navigator path, each
/// with the path it gets when the subtree's root moves from `old` to `new`.
/// One written as `id:` needs nothing, so it is left out.
fn following(root: &Value, indices: &[usize], old: &str, new: &str) -> Result<Vec<Follow>, String> {
    let all = nodes(root)?;
    Ok(references_into(root, indices)?
        .into_iter()
        .filter(|(r, named)| r.spelling == *named)
        .map(|(reference, named)| Follow {
            spelling: format!("{new}{}", &named[old.len()..]),
            sharing: all.iter().filter(|n| n.address == named).count(),
            reference,
        })
        .collect())
}

/// Write each followed reference's new path, and say which one, if any,
/// names a different number of nodes than it did. A group's name can hold a `/`, so a
/// rewritten path could spell some other node's too, and the caller undoes
/// the edit then rather than writing it.
fn rewrite_followed(root: &mut Value, follow: &[Follow]) -> Result<Option<String>, String> {
    for f in follow {
        set_reference(root, &f.reference, &f.spelling)?;
    }
    let all = nodes(root)?;
    Ok(follow
        .iter()
        .find(|f| all.iter().filter(|n| n.address == f.spelling).count() != f.sharing)
        .map(|f| f.reference.what.clone()))
}

/// A node's kind, for a message.
fn kind_noun(node: &Node<'_>) -> &'static str {
    if node.is_container {
        "group"
    } else if node.is_folder {
        "synchronized folder"
    } else {
        "file"
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

/// The node an add would find already at `address`: one of the kind being
/// added (a group when `group`, else a file), or a refusal when only a node of
/// another kind is there. Xcode lists such a pair side by side, but they would
/// share one navigator path, and no argument could then tell the two apart.
fn existing_at<'a>(
    root: &'a Value,
    address: &str,
    group: bool,
) -> Result<Option<Node<'a>>, String> {
    let mut there: Vec<Node<'a>> = nodes(root)?
        .into_iter()
        .filter(|n| n.address == address)
        .collect();
    if let Some(index) = there
        .iter()
        .position(|n| n.is_container == group && !n.is_folder)
    {
        return Ok(Some(there.swap_remove(index)));
    }
    let Some(other) = there.first() else {
        return Ok(None);
    };
    let (new, remedy) = if group {
        ("group", "pick another name")
    } else {
        ("file", "add it under another group")
    };
    Err(taken_refusal(address, other, new, remedy))
}

/// The refusal for a write that would put a node of kind `new` at an
/// `address` another node already has, or `None` when none does. Xcode reads
/// a reference one component at a time, so a reference through either node
/// would then name both, and it refuses the project ("Invalid reference").
fn taken(root: &Value, address: &str, new: &str, remedy: &str) -> Result<Option<String>, String> {
    Ok(nodes(root)?
        .iter()
        .find(|n| n.address == address)
        .map(|other| taken_refusal(address, other, new, remedy)))
}

/// The refusal for adding a synchronized folder at the navigator root where
/// another node already has its `address` and a reference runs through it,
/// or `None`. Xcode reads a reference one component at a time, so the
/// reference would then name two nodes, and Xcode 27.0 and 27.2 refuse the
/// project ("Invalid reference"). Two nodes sharing a path no reference
/// runs through are a project Xcode opens.
pub(crate) fn shared_under_reference(
    root: &Value,
    address: &str,
) -> Result<Option<String>, String> {
    let all = nodes(root)?;
    let Some(other) = all.iter().find(|n| n.address == address) else {
        return Ok(None);
    };
    let below = format!("{address}/");
    let Some(reference) = references(root)
        .into_iter()
        .find(|r| r.spelling == address || r.spelling.starts_with(&below))
    else {
        return Ok(None);
    };
    Ok(Some(format!(
        "'{address}' is already the navigator path of a {}, and the document names {} through \
         it. A synchronized folder beside it would make that reference name two nodes, which \
         Xcode refuses; move the {0} into another group first with 'pbxproj group move'",
        kind_noun(other),
        reference.what
    )))
}

fn taken_refusal(address: &str, other: &Node<'_>, new: &str, remedy: &str) -> String {
    format!(
        "'{address}' is already the navigator path of a {}. A {new} beside it would share \
         that path, and no argument could then tell the two apart; {remedy}",
        kind_noun(other)
    )
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
    position_of_reference(&all, reference).map(|index| all[index].resolved.clone())
}

/// The node a reference names, as [`resolve_reference`] reads it.
fn position_of_reference(all: &[Node<'_>], reference: &str) -> Option<usize> {
    position_of_id(all, reference).or_else(|| all.iter().position(|n| n.address == reference))
}

/// The `products-group` Xcode reads when the document writes none.
const DEFAULT_PRODUCTS_GROUP: &str = "Products";

/// One place in the document that names a navigator node: a configuration's
/// xcconfig `file` or the `anchor` of one, a target's `product`, or the
/// `products-group`.
///
/// Every one of them has to keep naming a node. Xcode 27.0 and 27.2 refuse to
/// open a document whose reference names nothing ("Invalid reference"), so a
/// move rewrites the ones it would break and a delete refuses to break one.
#[derive(Debug, Clone)]
struct Reference {
    /// The keys and indices from the document down to the string.
    at: Vec<Step>,
    /// The reference as written. A document with no `products-group` has
    /// the one Xcode reads in its place.
    spelling: String,
    /// What the reference is, for a message, after "is still".
    what: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Key(&'static str),
    Index(usize),
}

/// Every reference the document makes to a navigator node.
fn references(root: &Value) -> Vec<Reference> {
    fn configurations(
        owner: &Value,
        list: &'static str,
        base: &[Step],
        scope: &str,
        out: &mut Vec<Reference>,
    ) {
        let entries = owner.get(list).and_then(Value::as_array).unwrap_or(&[]);
        for (index, entry) in entries.iter().enumerate() {
            let Some(file) = entry.get("file") else {
                continue;
            };
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let what =
                format!("the xcconfig that the '{name}' configuration of {scope} is based on");
            let mut at = base.to_vec();
            at.extend([Step::Key(list), Step::Index(index), Step::Key("file")]);
            if let Some(spelling) = file.as_str() {
                out.push(Reference {
                    at,
                    spelling: spelling.to_string(),
                    what,
                });
            } else if let Some(anchor) = file.get("anchor").and_then(Value::as_str) {
                at.push(Step::Key("anchor"));
                out.push(Reference {
                    at,
                    spelling: anchor.to_string(),
                    what: format!("the folder holding {what}"),
                });
            }
        }
    }

    let mut out = Vec::new();
    configurations(root, "configurations", &[], "the project", &mut out);
    let targets = root.get("targets").and_then(Value::as_array).unwrap_or(&[]);
    for (index, target) in targets.iter().enumerate() {
        let name = target
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let base = [Step::Key("targets"), Step::Index(index)];
        let scope = format!("target '{name}'");
        configurations(
            target,
            "specialized-configurations",
            &base,
            &scope,
            &mut out,
        );
        if let Some(product) = target.get("product").and_then(Value::as_str) {
            let mut at = base.to_vec();
            at.push(Step::Key("product"));
            out.push(Reference {
                at,
                spelling: product.to_string(),
                what: format!("the product of target '{name}'"),
            });
        }
    }
    out.push(Reference {
        at: vec![Step::Key("products-group")],
        spelling: root
            .get("products-group")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PRODUCTS_GROUP)
            .to_string(),
        what: "the project's products group".to_string(),
    });
    out
}

/// Write `spelling` where `reference` sits. A `products-group` that comes to
/// name the default is left out, as Xcode leaves it out.
fn set_reference(root: &mut Value, reference: &Reference, spelling: &str) -> Result<(), String> {
    if reference.at == [Step::Key("products-group")] {
        let document = root
            .as_object_mut()
            .ok_or("the document is not an object")?;
        if spelling == DEFAULT_PRODUCTS_GROUP {
            document.remove("products-group");
        } else {
            crate::schema_xcproj::insert_document_key(
                document,
                "products-group",
                Value::String(spelling.to_string()),
            );
        }
        return Ok(());
    }
    let mut at = &mut *root;
    for step in &reference.at {
        at = match step {
            Step::Key(key) => at.get_mut(key),
            Step::Index(index) => at.as_array_mut().and_then(|items| items.get_mut(*index)),
        }
        .ok_or("a reference moved during the edit")?;
    }
    *at = Value::String(spelling.to_string());
    Ok(())
}

/// The references naming the node at `indices` or anything inside it, each
/// with the navigator path of the node it names.
fn references_into(root: &Value, indices: &[usize]) -> Result<Vec<(Reference, String)>, String> {
    let all = nodes(root)?;
    Ok(references(root)
        .into_iter()
        .filter_map(|r| {
            let named = &all[position_of_reference(&all, &r.spelling)?];
            named
                .indices
                .starts_with(indices)
                .then(|| (r, named.address.clone()))
        })
        .collect())
}

/// The refusal for deleting the node at `indices` while a reference names
/// it, or `None` when none does.
fn named_by_refusal(
    root: &Value,
    address: &str,
    indices: &[usize],
) -> Result<Option<String>, String> {
    let named: Vec<String> = references_into(root, indices)?
        .into_iter()
        .map(|(r, _)| r.what)
        .collect();
    Ok((!named.is_empty()).then(|| crate::tree::still_named_refusal(address, &named)))
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

/// What Xcode shows the node as ([`crate::tree::display_name`]).
pub(crate) fn display_name(node: &Value) -> &str {
    crate::tree::display_name(node.get("name").and_then(Value::as_str), stored_path(node))
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
