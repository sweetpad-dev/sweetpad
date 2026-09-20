//! The classic project tree: `PBXFileReference` objects and the `PBXGroup`
//! nodes that list them, as explicit primitives.
//!
//! Xcode's pre-synchronized-folders representation names a file twice — a
//! *reference* (the file exists in the project) and a *group entry* (where it
//! appears in the navigator). Neither says anything about what **builds**;
//! that is [`crate::membership_pbxproj`]'s `PBXBuildFile` layer. The three
//! stay separate axes here so a script can move one without the others
//! shifting underneath it: [`add_fileref`] never picks a build phase, and
//! [`remove_fileref`] never prunes a group.
//!
//! The one linkage that is not optional is referential integrity — a group's
//! `children` must not name an object that no longer exists — so deleting a
//! node also drops it from its parent's list. Every outcome reports that,
//! rather than leaving it to a `git diff`.
//!
//! A reference's `path` is interpreted against its `sourceTree`: `<group>`
//! (the default) resolves it under the owning group's directory, `SOURCE_ROOT`
//! against the project directory, `<absolute>` as-is. Callers get the resolved
//! on-disk path back in the outcome so a wrong pairing is visible immediately
//! instead of at the next build.
//!
//! Everything here is pure (no I/O): callers parse the file, mutate the tree,
//! and serialize/write it — the same contract as the sibling `*_pbxproj`
//! modules.

use std::path::{Path, PathBuf};

use crate::pbxproj::{Dict, Value};
use crate::spm_pbxproj::fresh_guid;

const REF_ISA: &str = "PBXFileReference";
const GROUP_ISA: &str = "PBXGroup";

/// The group-like objects a child can hang from. Variant and version groups
/// list children exactly as a plain group does.
const GROUP_ISAS: [&str; 3] = ["PBXGroup", "PBXVariantGroup", "XCVersionGroup"];

pub use crate::tree::{
    AddGroupOutcome, AddRefOutcome, FileRefRow, GroupRow, MoveOutcome, RemoveOutcome,
};

/// What [`attach`] or [`detach`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum LinkOutcome {
    Linked { child: String, group: String },
    AlreadyLinked { child: String, group: String },
    Unlinked { child: String, group: String },
    NotLinked { child: String, group: String },
}

/// Every `PBXFileReference` in the project.
///
/// # Errors
/// Returns a message when the tree has no `objects` dict.
pub fn list_filerefs(root: &Value) -> Result<Vec<FileRefRow>, String> {
    let objects = objects(root).ok_or("pbxproj has no objects dict")?;
    let project_dir = Path::new("");
    let mut rows: Vec<FileRefRow> = objects
        .iter()
        .filter(|(_, o)| isa(o) == REF_ISA)
        .map(|(guid, o)| {
            let source_tree = str_field(o, "sourceTree").unwrap_or("<group>").to_string();
            FileRefRow {
                address: guid.clone(),
                id: Some(guid.clone()),
                path: str_field(o, "path").unwrap_or_default().to_string(),
                file_type: str_field(o, "lastKnownFileType")
                    .or_else(|| str_field(o, "explicitFileType"))
                    .map(str::to_string),
                parent: crate::project::parent_group_of(objects, guid),
                resolved: display(&resolve_node(objects, guid, project_dir)),
                source_tree,
                build_files: build_file_count(objects, guid),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.resolved.cmp(&b.resolved).then(a.address.cmp(&b.address)));
    Ok(rows)
}

/// Every group node in the project.
///
/// # Errors
/// Returns a message when the tree has no `objects` dict.
pub fn list_groups(root: &Value) -> Result<Vec<GroupRow>, String> {
    let objects = objects(root).ok_or("pbxproj has no objects dict")?;
    let project_dir = Path::new("");
    let mut rows: Vec<GroupRow> = objects
        .iter()
        .filter(|(_, o)| GROUP_ISAS.contains(&isa(o)))
        .map(|(guid, o)| GroupRow {
            address: guid.clone(),
            id: Some(guid.clone()),
            isa: isa(o).to_string(),
            name: str_field(o, "name").map(str::to_string),
            path: str_field(o, "path").map(str::to_string),
            source_tree: str_field(o, "sourceTree").unwrap_or("<group>").to_string(),
            parent: crate::project::parent_group_of(objects, guid),
            children: children_of(objects, guid),
            resolved: display(&crate::project::group_dir(objects, guid, project_dir, 0)),
        })
        .collect();
    rows.sort_by(|a, b| a.resolved.cmp(&b.resolved).then(a.address.cmp(&b.address)));
    Ok(rows)
}

/// Create a `PBXFileReference` for `path`, anchored at `source_tree`.
///
/// `file_type` writes `lastKnownFileType`; omitting it leaves the key out, so
/// Xcode derives the type from the extension — an absent answer, not a guessed
/// one. `group` names either an object id or the group's resolved directory,
/// and attaches the new reference to that group's `children`; without it the
/// reference exists but no group lists it (legal, and invisible in Xcode's
/// navigator until something attaches it).
///
/// Nothing here touches a build phase: a reference is not membership. Pair it
/// with [`crate::membership_pbxproj::add_membership`] to make a target build it.
///
/// # Errors
/// Returns a message when the tree is malformed, `path` is empty, or `group`
/// is not an existing group node.
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
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let group = group
        .map(|spec| resolve_group(objects_ref, spec))
        .transpose()?;
    let group = group.as_deref();
    if let Some(existing) = objects_ref.iter().find_map(|(guid, o)| {
        (isa(o) == REF_ISA
            && str_field(o, "path") == Some(path.as_str())
            && str_field(o, "sourceTree").unwrap_or("<group>") == source_tree)
            .then(|| guid.clone())
    }) {
        let resolved = display(&resolve_node(objects_ref, &existing, Path::new("")));
        return Ok(AddRefOutcome::AlreadyExists {
            address: existing,
            resolved,
        });
    }

    let objects = objects_mut(root)?;
    let guid = fresh_guid(objects, &format!("fileref#{source_tree}#{path}"), 0);
    // Xcode writes file references on one line; matching that keeps the diff
    // against an Xcode-touched project readable.
    let mut node = Dict::new();
    node.insert("isa".into(), vstr(REF_ISA));
    if let Some(file_type) = file_type {
        node.insert("lastKnownFileType".into(), vstr(file_type));
    }
    node.insert("path".into(), vstr(&path));
    node.insert("sourceTree".into(), vstr(source_tree));
    node.set_single_line(true);
    objects.insert(guid.clone(), Value::Dict(node));

    if let Some(group) = group {
        push_child(objects, group, &guid);
    }
    let resolved = display(&resolve_node(objects, &guid, Path::new("")));
    Ok(AddRefOutcome::Created {
        address: guid,
        resolved,
        attached_to: group.map(str::to_string),
    })
}

/// Create a `PBXGroup` under `parent`.
///
/// `name` titles the group; `path` is the directory it contributes to its
/// children's resolution (omit it for a purely organizational group that adds
/// no directory component). `parent` names either an object id or the group's
/// resolved directory.
///
/// # Errors
/// Returns a message when the tree is malformed or `parent` names no group, or
/// more than one.
pub fn add_group(
    root: &mut Value,
    name: &str,
    parent: Option<&str>,
    path: Option<&str>,
    source_tree: &str,
) -> Result<AddGroupOutcome, String> {
    if name.is_empty() {
        return Err("the group name must not be empty".to_string());
    }
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let parent = &settle_group(objects_ref, parent)?;
    if let Some(existing) = children_of(objects_ref, parent).into_iter().find(|child| {
        objects_ref.get(child).is_some_and(|o| {
            GROUP_ISAS.contains(&isa(o))
                && (str_field(o, "name") == Some(name) || str_field(o, "path") == Some(name))
        })
    }) {
        let resolved = display(&crate::project::group_dir(
            objects_ref,
            &existing,
            Path::new(""),
            0,
        ));
        return Ok(AddGroupOutcome::AlreadyExists {
            address: existing,
            resolved,
        });
    }

    let objects = objects_mut(root)?;
    let guid = fresh_guid(objects, &format!("group#{parent}#{name}"), 0);
    let mut node = Dict::new();
    node.insert("isa".into(), vstr(GROUP_ISA));
    node.insert("children".into(), Value::Array(Vec::new()));
    // Xcode omits `name` when it would just repeat `path`.
    if path != Some(name) {
        node.insert("name".into(), vstr(name));
    }
    if let Some(path) = path {
        node.insert("path".into(), vstr(path));
    }
    node.insert("sourceTree".into(), vstr(source_tree));
    objects.insert(guid.clone(), Value::Dict(node));
    push_child(objects, parent, &guid);

    let resolved = display(&crate::project::group_dir(objects, &guid, Path::new(""), 0));
    Ok(AddGroupOutcome::Created {
        address: guid,
        resolved,
    })
}

/// Delete a `PBXFileReference`.
///
/// Refuses while any `PBXBuildFile` still points at it unless `force` — a
/// dangling `fileRef` is a corrupt project, and dropping the membership is
/// [`crate::membership_pbxproj::remove_membership`]'s job, not a side effect of
/// this one. No group is pruned: an emptied group stays.
///
/// # Errors
/// Returns a message when the tree is malformed, `guid` is not a file
/// reference, or build files still reference it and `force` is false.
pub fn remove_fileref(root: &mut Value, guid: &str, force: bool) -> Result<RemoveOutcome, String> {
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let node = objects_ref
        .get(guid)
        .ok_or_else(|| format!("no object with id {guid}"))?;
    if isa(node) != REF_ISA {
        return Err(format!("{guid} is a {}, not a {REF_ISA}", isa(node)));
    }
    let used = build_file_count(objects_ref, guid);
    if used > 0 && !force {
        return Err(format!(
            "{guid} is still built by {used} build-file entr{}: drop the membership with \
             `pbxproj membership remove`, or pass --dangling to delete it anyway",
            if used == 1 { "y" } else { "ies" }
        ));
    }
    let parent = crate::project::parent_group_of(objects_ref, guid);

    let objects = objects_mut(root)?;
    if let Some(parent) = &parent {
        remove_child(objects, parent, guid);
    }
    objects.remove(guid);
    Ok(RemoveOutcome {
        address: guid.to_string(),
        detached_from: parent,
        orphaned: Vec::new(),
    })
}

/// Delete a group node.
///
/// Refuses while it still lists children unless `force` — emptying it is
/// [`detach`]'s job. Under `force` the children stay in `objects` as
/// unreferenced nodes and are reported in `orphaned`.
///
/// # Errors
/// Returns a message when the tree is malformed, `guid` is not a group, or it
/// has children and `force` is false.
pub fn remove_group(root: &mut Value, guid: &str, force: bool) -> Result<RemoveOutcome, String> {
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let node = objects_ref
        .get(guid)
        .ok_or_else(|| format!("no object with id {guid}"))?;
    if !GROUP_ISAS.contains(&isa(node)) {
        return Err(format!("{guid} is a {}, not a group", isa(node)));
    }
    let children = children_of(objects_ref, guid);
    if !children.is_empty() && !force {
        return Err(format!(
            "{guid} still lists {} child object(s): detach them with `pbxproj group detach`, \
             or pass --orphan-children to delete it anyway",
            children.len()
        ));
    }
    let parent = crate::project::parent_group_of(objects_ref, guid);

    let objects = objects_mut(root)?;
    if let Some(parent) = &parent {
        remove_child(objects, parent, guid);
    }
    objects.remove(guid);
    Ok(RemoveOutcome {
        address: guid.to_string(),
        detached_from: parent,
        orphaned: children,
    })
}

/// List `child` in `group`'s `children`. `group` names either an object id or
/// the group's resolved directory.
///
/// # Errors
/// Returns a message when the tree is malformed, `group` names no group (or
/// more than one), or `child` does not exist.
pub fn attach(root: &mut Value, child: &str, group: &str) -> Result<LinkOutcome, String> {
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let group = &resolve_group(objects_ref, group)?;
    if !objects_ref.contains_key(child) {
        return Err(format!("no object with id {child}"));
    }
    if children_of(objects_ref, group).iter().any(|c| c == child) {
        return Ok(LinkOutcome::AlreadyLinked {
            child: child.to_string(),
            group: group.clone(),
        });
    }
    let objects = objects_mut(root)?;
    push_child(objects, group, child);
    Ok(LinkOutcome::Linked {
        child: child.to_string(),
        group: group.clone(),
    })
}

/// Drop `child` from `group`'s `children`, leaving the object itself in place.
/// `group` names either an object id or the group's resolved directory.
///
/// # Errors
/// Returns a message when the tree is malformed or `group` names no group, or
/// more than one.
pub fn detach(root: &mut Value, child: &str, group: &str) -> Result<LinkOutcome, String> {
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let group = &resolve_group(objects_ref, group)?;
    if !children_of(objects_ref, group).iter().any(|c| c == child) {
        return Ok(LinkOutcome::NotLinked {
            child: child.to_string(),
            group: group.clone(),
        });
    }
    let objects = objects_mut(root)?;
    remove_child(objects, group, child);
    Ok(LinkOutcome::Unlinked {
        child: child.to_string(),
        group: group.clone(),
    })
}

/// Move a node from whichever group lists it into `group`, keeping the file it
/// resolves to.
///
/// This is the one verb whose meaning carries over to a `project.xcproj`, where
/// a node sits in exactly one place and [`attach`]/[`detach`] have no
/// counterpart. The node's stored `path` is rewritten when it has to be: a
/// `<group>`-anchored path means something different under a different group,
/// so the new group's directory is stripped off the resolved path when it
/// prefixes it, and the reference is re-anchored to `SOURCE_ROOT` when it does
/// not. Any other `sourceTree` already ignores the group chain and is left
/// alone. The outcome carries the resolved path so the preservation is
/// checkable rather than promised.
///
/// # Errors
/// Returns a message when the tree is malformed, `child` does not exist, or
/// `group` names no group (or more than one).
pub fn move_node(
    root: &mut Value,
    child: &str,
    group: Option<&str>,
) -> Result<MoveOutcome, String> {
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let group = settle_group(objects_ref, group)?;
    let node = objects_ref
        .get(child)
        .ok_or_else(|| format!("no object with id {child}"))?;
    if child == group {
        return Err(format!("{child} cannot hold itself"));
    }
    if GROUP_ISAS.contains(&isa(node)) && is_ancestor(objects_ref, child, &group) {
        return Err(format!(
            "{group} is inside {child}; moving a group into its own descendant would \
             detach the whole subtree"
        ));
    }
    let from = crate::project::parent_group_of(objects_ref, child);
    if from.as_deref() == Some(group.as_str()) {
        return Ok(MoveOutcome::AlreadyThere {
            address: child.to_string(),
            group,
        });
    }
    let resolved = if GROUP_ISAS.contains(&isa(node)) {
        display(&crate::project::group_dir(
            objects_ref,
            child,
            Path::new(""),
            0,
        ))
    } else {
        display(&resolve_node(objects_ref, child, Path::new("")))
    };
    let anchored = str_field(node, "sourceTree").unwrap_or("<group>") == "<group>";
    let group_dir = display(&crate::project::group_dir(
        objects_ref,
        &group,
        Path::new(""),
        0,
    ));

    let objects = objects_mut(root)?;
    if let Some(from) = &from {
        remove_child(objects, from, child);
    }
    push_child(objects, &group, child);
    if anchored {
        let (path, source_tree) = reanchor(&resolved, &group_dir);
        if let Some(node) = objects.get_mut(child).and_then(Value::as_dict_mut) {
            if path.is_empty() {
                node.remove("path");
            } else {
                node.insert("path".into(), vstr(&path));
            }
            node.insert("sourceTree".into(), vstr(source_tree));
        }
    }
    Ok(MoveOutcome::Moved {
        address: child.to_string(),
        from,
        to: group,
        resolved,
    })
}

/// How to spell `resolved` from inside a group at `group_dir`: relative when
/// the directory contains it, anchored at the project root when it does not.
fn reanchor(resolved: &str, group_dir: &str) -> (String, &'static str) {
    if group_dir.is_empty() {
        return (resolved.to_string(), "<group>");
    }
    match resolved.strip_prefix(&format!("{group_dir}/")) {
        Some(rest) => (rest.to_string(), "<group>"),
        None => (resolved.to_string(), "SOURCE_ROOT"),
    }
}

/// Whether `descendant` is reachable from `ancestor` through `children`.
fn is_ancestor(objects: &Dict, ancestor: &str, descendant: &str) -> bool {
    fn walk(objects: &Dict, at: &str, wanted: &str, depth: usize) -> bool {
        if depth >= crate::project::MAX_GROUP_DEPTH {
            return false;
        }
        children_of(objects, at)
            .iter()
            .any(|child| child == wanted || walk(objects, child, wanted, depth + 1))
    }
    walk(objects, ancestor, descendant, 0)
}

/// The reference whose resolved path is `path`, for callers that work in file
/// paths rather than ids. `None` when nothing matches; `Err` when more than one
/// does (ambiguity is the caller's to resolve, with an id).
///
/// # Errors
/// Returns a message when the tree is malformed or the path is ambiguous.
pub fn fileref_for_path(root: &Value, path: &str) -> Result<Option<String>, String> {
    let objects = objects(root).ok_or("pbxproj has no objects dict")?;
    let wanted = normalize(path);
    let hits: Vec<String> = objects
        .iter()
        .filter(|(_, o)| isa(o) == REF_ISA)
        .filter(|(guid, _)| display(&resolve_node(objects, guid, Path::new(""))) == wanted)
        .map(|(guid, _)| guid.clone())
        .collect();
    match hits.len() {
        0 => Ok(None),
        1 => Ok(Some(hits[0].clone())),
        _ => Err(format!(
            "{wanted} matches {} file references ({}); pass the id you mean",
            hits.len(),
            hits.join(", ")
        )),
    }
}

/// Settle a group argument: an object id, the group's navigator path
/// (`Sources/App`, the display names from the mainGroup down), or its resolved
/// directory.
///
/// An id is unambiguous by construction, so it wins outright. A path is matched
/// against both the navigator path and the resolved directory, which is what
/// lets one spelling address a group in either document format —
/// [`crate::tree_xcproj`] has only the navigator path, and an organizational
/// group (a `name` with no `path`) has a navigator path but resolves to its
/// parent's directory. Naming no group, or two, is an error rather than a pick.
fn settle_group(objects: &Dict, spec: Option<&str>) -> Result<String, String> {
    match spec {
        Some(spec) => resolve_group(objects, spec),
        None => main_group(objects).ok_or_else(|| "the project has no mainGroup".to_string()),
    }
}

/// The navigator root: the group `PBXProject` points at.
fn main_group(objects: &Dict) -> Option<String> {
    objects
        .iter()
        .find(|(_, o)| isa(o) == "PBXProject")
        .and_then(|(_, o)| str_field(o, "mainGroup"))
        .map(str::to_string)
}

fn resolve_group(objects: &Dict, spec: &str) -> Result<String, String> {
    if let Some(node) = objects.get(spec) {
        return if GROUP_ISAS.contains(&isa(node)) {
            Ok(spec.to_string())
        } else {
            Err(format!("{spec} is a {}, not a group", isa(node)))
        };
    }
    let wanted = normalize(spec);
    let mut hits: Vec<String> = navigator_paths(objects)
        .into_iter()
        .filter(|(_, path)| *path == wanted)
        .map(|(guid, _)| guid)
        .collect();
    for (guid, _) in objects
        .iter()
        .filter(|(_, o)| GROUP_ISAS.contains(&isa(o)))
        .filter(|(guid, _)| {
            display(&crate::project::group_dir(objects, guid, Path::new(""), 0)) == wanted
        })
    {
        if !hits.contains(guid) {
            hits.push(guid.clone());
        }
    }
    match hits.len() {
        1 => Ok(hits.into_iter().next().unwrap_or_default()),
        0 => Err(format!(
            "no group with id, navigator path, or directory {spec}; `pbxproj group list` \
             shows all three"
        )),
        n => Err(format!(
            "{wanted} names {n} groups ({}); pass the id you mean",
            hits.join(", ")
        )),
    }
}

/// Every group's navigator path — the display names from the mainGroup down,
/// joined by `/`, which is how a `project.xcproj` addresses its nodes and the
/// only spelling that tells two organizational groups apart.
fn navigator_paths(objects: &Dict) -> Vec<(String, String)> {
    fn walk(objects: &Dict, guid: &str, base: &str, depth: usize, out: &mut Vec<(String, String)>) {
        if depth >= crate::project::MAX_GROUP_DEPTH {
            return;
        }
        for child in children_of(objects, guid) {
            let Some(node) = objects.get(&child) else {
                continue;
            };
            if !GROUP_ISAS.contains(&isa(node)) {
                continue;
            }
            let Some(name) = display_name(node) else {
                continue;
            };
            let path = if base.is_empty() {
                name.to_string()
            } else {
                format!("{base}/{name}")
            };
            walk(objects, &child, &path, depth + 1, out);
            out.push((child.clone(), path));
        }
    }
    let mut out = Vec::new();
    if let Some(root) = main_group(objects) {
        walk(objects, &root, "", 0, &mut out);
    }
    out
}

/// What Xcode shows a node as: its `name`, else the last component of its
/// `path`.
fn display_name(node: &Value) -> Option<&str> {
    str_field(node, "name")
        .or_else(|| {
            str_field(node, "path").map(|p| {
                p.trim_end_matches('/')
                    .rsplit_once('/')
                    .map_or(p, |(_, name)| name)
            })
        })
        .filter(|name| !name.is_empty())
}

fn children_of(objects: &Dict, guid: &str) -> Vec<String> {
    objects
        .get(guid)
        .and_then(|o| o.get("children"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn push_child(objects: &mut Dict, group: &str, child: &str) {
    if let Some(children) = objects
        .get_mut(group)
        .and_then(|o| o.get_mut("children"))
        .and_then(Value::as_array_mut)
    {
        children.push(vstr(child));
        return;
    }
    // A group with no `children` key yet: give it one.
    if let Some(node) = objects.get_mut(group).and_then(Value::as_dict_mut) {
        node.insert("children".into(), Value::Array(vec![vstr(child)]));
    }
}

fn remove_child(objects: &mut Dict, group: &str, child: &str) {
    if let Some(children) = objects
        .get_mut(group)
        .and_then(|o| o.get_mut("children"))
        .and_then(Value::as_array_mut)
    {
        children.retain(|c| c.as_str() != Some(child));
    }
}

fn build_file_count(objects: &Dict, ref_guid: &str) -> usize {
    objects
        .iter()
        .filter(|(_, o)| isa(o) == "PBXBuildFile" && str_field(o, "fileRef") == Some(ref_guid))
        .count()
}

/// A node's on-disk path: its own `path` anchored by `sourceTree`, with
/// `<group>` resolving up the parent chain.
fn resolve_node(objects: &Dict, guid: &str, project_dir: &Path) -> PathBuf {
    let Some(node) = objects.get(guid) else {
        return project_dir.to_path_buf();
    };
    let path = str_field(node, "path").unwrap_or_default();
    match str_field(node, "sourceTree").unwrap_or("<group>") {
        "<absolute>" => PathBuf::from(path),
        "<group>" => match crate::project::parent_group_of(objects, guid) {
            Some(parent) => crate::project::group_dir(objects, &parent, project_dir, 0).join(path),
            None => project_dir.join(path),
        },
        _ => project_dir.join(path),
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().trim_start_matches('/').to_string()
}

fn normalize(path: &str) -> String {
    path.trim_start_matches("./").trim_matches('/').to_string()
}

fn vstr(s: &str) -> Value {
    Value::String(s.to_string())
}

fn objects(root: &Value) -> Option<&Dict> {
    root.get("objects")?.as_dict()
}

fn objects_mut(root: &mut Value) -> Result<&mut Dict, String> {
    root.get_mut("objects")
        .and_then(Value::as_dict_mut)
        .ok_or_else(|| "pbxproj has no objects dict".to_string())
}

fn isa(obj: &Value) -> &str {
    obj.get("isa").and_then(Value::as_str).unwrap_or_default()
}

fn str_field<'a>(obj: &'a Value, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A classic project: the App group holds one source and a nested Legacy
    /// group; one build file makes App compile the source.
    const FIXTURE: &str = r#"// !$*UTF8*$!
{
	archiveVersion = 1;
	objectVersion = 56;
	objects = {
		BF1 /* Main.swift in Sources */ = {isa = PBXBuildFile; fileRef = FR1 /* Main.swift */; };
		FR1 /* Main.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = Main.swift; sourceTree = "<group>"; };
		G1 /* App */ = {
			isa = PBXGroup;
			children = (
				FR1 /* Main.swift */,
				G2 /* Legacy */,
			);
			path = App;
			sourceTree = "<group>";
		};
		G2 /* Legacy */ = {
			isa = PBXGroup;
			children = (
			);
			path = Legacy;
			sourceTree = "<group>";
		};
		MG = {
			isa = PBXGroup;
			children = (
				G1 /* App */,
			);
			sourceTree = "<group>";
		};
		SP1 = {
			isa = PBXSourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (
				BF1 /* Main.swift in Sources */,
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
		T1 /* App */ = {
			isa = PBXNativeTarget;
			buildConfigurationList = CLT1;
			buildPhases = (
				SP1,
			);
			name = App;
			productType = "com.apple.product-type.application";
		};
		CLT1 = {isa = XCConfigurationList; buildConfigurations = (); };
		P1 = {
			isa = PBXProject;
			mainGroup = MG;
			targets = (
				T1 /* App */,
			);
		};
	};
	rootObject = P1;
}
"#;

    fn parsed() -> Value {
        crate::pbxproj::parse(FIXTURE).expect("fixture parses")
    }

    fn round_trips(root: &Value) -> String {
        let text = crate::pbxproj_writer::serialize(root, "Fix");
        let reparsed = crate::pbxproj::parse(&text).expect("mutated pbxproj parses");
        let again = crate::pbxproj_writer::serialize(&reparsed, "Fix");
        assert_eq!(text, again, "serialize → parse → serialize is stable");
        text
    }

    #[test]
    fn adding_a_reference_attaches_it_and_touches_no_build_phase() {
        let mut root = parsed();
        let outcome = add_fileref(
            &mut root,
            "Extra.swift",
            Some("sourcecode.swift"),
            "<group>",
            Some("G1"),
        )
        .unwrap();
        let AddRefOutcome::Created {
            address: guid,
            resolved,
            attached_to,
        } = outcome
        else {
            panic!("expected a fresh reference");
        };
        assert_eq!(resolved, "App/Extra.swift", "resolves under the App group");
        assert_eq!(attached_to.as_deref(), Some("G1"));

        let text = round_trips(&root);
        assert!(text.contains(&guid));
        // A reference is not membership: the sources phase is untouched.
        // (Counting the `isa`, since the writer also emits Begin/End section
        // markers naming the same type.)
        assert_eq!(
            text.matches("isa = PBXBuildFile").count(),
            1,
            "no build file was invented"
        );
    }

    #[test]
    fn a_reference_without_a_group_is_created_unattached() {
        let mut root = parsed();
        let outcome = add_fileref(&mut root, "Loose.swift", None, "SOURCE_ROOT", None).unwrap();
        let AddRefOutcome::Created {
            address: guid,
            resolved,
            attached_to,
        } = outcome
        else {
            panic!("expected a fresh reference");
        };
        assert_eq!(
            resolved, "Loose.swift",
            "SOURCE_ROOT ignores the group chain"
        );
        assert!(attached_to.is_none());
        assert!(
            crate::project::parent_group_of(objects(&root).unwrap(), &guid).is_none(),
            "no group lists it"
        );
        // No file type given means no key written — an absent answer, not a guess.
        let text = round_trips(&root);
        let line = text.lines().find(|l| l.contains(&guid)).unwrap();
        assert!(!line.contains("lastKnownFileType"), "{line}");
    }

    #[test]
    fn adding_the_same_path_twice_is_a_no_op() {
        let mut root = parsed();
        let before = crate::pbxproj_writer::serialize(&root, "Fix");
        let outcome = add_fileref(
            &mut root,
            "Main.swift",
            Some("sourcecode.swift"),
            "<group>",
            Some("G1"),
        )
        .unwrap();
        assert_eq!(
            outcome,
            AddRefOutcome::AlreadyExists {
                address: "FR1".into(),
                resolved: "App/Main.swift".into()
            }
        );
        let after = crate::pbxproj_writer::serialize(&root, "Fix");
        assert_eq!(before, after, "no-ops must not touch the file");
    }

    #[test]
    fn removing_a_built_reference_refuses_without_force() {
        let mut root = parsed();
        let err = remove_fileref(&mut root, "FR1", false).unwrap_err();
        assert!(err.contains("still built by 1 build-file entry"), "{err}");
        assert!(err.contains("membership remove"), "{err}");
        // The refusal really refused.
        assert!(objects(&root).unwrap().contains_key("FR1"));
    }

    #[test]
    fn removing_a_reference_detaches_it_but_prunes_no_group() {
        let mut root = parsed();
        // Drop the build file first, the way the two-step contract intends.
        objects_mut(&mut root).unwrap().remove("BF1");
        let outcome = remove_fileref(&mut root, "FR1", false).unwrap();
        assert_eq!(outcome.detached_from.as_deref(), Some("G1"));
        assert!(outcome.orphaned.is_empty());

        let text = round_trips(&root);
        assert!(
            !text.contains("FR1"),
            "the reference and its child entry go"
        );
        assert!(
            text.contains("G1 /* App */"),
            "the emptied group stays — pruning is not this verb's job"
        );
    }

    #[test]
    fn removing_a_group_with_children_refuses_without_force() {
        let mut root = parsed();
        let err = remove_group(&mut root, "G1", false).unwrap_err();
        assert!(err.contains("still lists 2 child object(s)"), "{err}");

        let outcome = remove_group(&mut root, "G1", true).unwrap();
        assert_eq!(outcome.detached_from.as_deref(), Some("MG"));
        assert_eq!(outcome.orphaned, vec!["FR1", "G2"], "orphans are reported");
        let text = round_trips(&root);
        assert!(
            text.contains("FR1"),
            "orphans stay in objects, unreferenced"
        );
    }

    #[test]
    fn attach_and_detach_move_only_the_child_entry() {
        let mut root = parsed();
        assert_eq!(
            detach(&mut root, "FR1", "G1").unwrap(),
            LinkOutcome::Unlinked {
                child: "FR1".into(),
                group: "G1".into()
            }
        );
        assert!(
            objects(&root).unwrap().contains_key("FR1"),
            "detach leaves the object alone"
        );
        assert_eq!(
            detach(&mut root, "FR1", "G1").unwrap(),
            LinkOutcome::NotLinked {
                child: "FR1".into(),
                group: "G1".into()
            }
        );
        assert_eq!(
            attach(&mut root, "FR1", "G2").unwrap(),
            LinkOutcome::Linked {
                child: "FR1".into(),
                group: "G2".into()
            }
        );
        assert_eq!(
            attach(&mut root, "FR1", "G2").unwrap(),
            LinkOutcome::AlreadyLinked {
                child: "FR1".into(),
                group: "G2".into()
            }
        );
        let text = round_trips(&root);
        assert!(text.contains("FR1"));
    }

    #[test]
    fn a_new_group_resolves_under_its_parent() {
        let mut root = parsed();
        let outcome = add_group(&mut root, "Views", Some("G1"), Some("Views"), "<group>").unwrap();
        let AddGroupOutcome::Created {
            address: guid,
            resolved,
        } = outcome
        else {
            panic!("expected a fresh group");
        };
        assert_eq!(resolved, "App/Views");
        // A path equal to the name means Xcode omits `name`.
        let text = round_trips(&root);
        let block = text.split(&guid).nth(1).unwrap();
        assert!(!block[..120].contains("name = Views"), "{block}");
    }

    #[test]
    fn the_wrong_id_kind_errors_instead_of_guessing() {
        let mut root = parsed();
        let err = remove_group(&mut root, "FR1", false).unwrap_err();
        assert!(err.contains("not a group"), "{err}");
        let err = remove_fileref(&mut root, "G1", false).unwrap_err();
        assert!(err.contains("not a PBXFileReference"), "{err}");
        let err = attach(&mut root, "FR1", "SP1").unwrap_err();
        assert!(err.contains("not a group"), "{err}");
        let err = attach(&mut root, "NOPE", "G1").unwrap_err();
        assert!(err.contains("no object with id NOPE"), "{err}");
    }

    #[test]
    fn a_path_resolves_to_its_reference() {
        let root = parsed();
        assert_eq!(
            fileref_for_path(&root, "App/Main.swift")
                .unwrap()
                .as_deref(),
            Some("FR1")
        );
        assert!(fileref_for_path(&root, "App/Nope.swift").unwrap().is_none());
    }

    #[test]
    fn a_group_can_be_named_by_its_directory_instead_of_its_id() {
        let mut root = parsed();
        let outcome = add_fileref(&mut root, "Extra.swift", None, "<group>", Some("App")).unwrap();
        let AddRefOutcome::Created {
            resolved,
            attached_to,
            ..
        } = outcome
        else {
            panic!("expected a fresh reference");
        };
        assert_eq!(resolved, "App/Extra.swift");
        assert_eq!(
            attached_to.as_deref(),
            Some("G1"),
            "the directory settles to the id, and the outcome reports the id"
        );
    }

    #[test]
    fn a_nested_group_directory_resolves_and_an_unknown_one_is_refused() {
        let mut root = parsed();
        let outcome =
            add_fileref(&mut root, "Old.swift", None, "<group>", Some("App/Legacy")).unwrap();
        let AddRefOutcome::Created { attached_to, .. } = outcome else {
            panic!("expected a fresh reference");
        };
        assert_eq!(attached_to.as_deref(), Some("G2"));

        let err = add_fileref(&mut root, "X.swift", None, "<group>", Some("App/Nope")).unwrap_err();
        assert!(
            err.contains("no group with id, navigator path, or directory App/Nope"),
            "{err}"
        );
    }

    /// The navigator path is what a `project.xcproj` addresses nodes by, so it
    /// has to name a group here too — and it is the only spelling that tells
    /// apart two groups sharing a directory.
    #[test]
    fn a_group_answers_to_its_navigator_path() {
        let mut root = parsed();
        add_group(&mut root, "Other", Some("MG"), Some("App"), "<group>").unwrap();

        let outcome = add_fileref(&mut root, "X.swift", None, "<group>", Some("Other")).unwrap();
        let AddRefOutcome::Created {
            attached_to,
            resolved,
            ..
        } = outcome
        else {
            panic!("expected a fresh reference");
        };
        assert!(attached_to.is_some());
        assert_eq!(resolved, "App/X.swift", "it still resolves under App");
    }

    #[test]
    fn a_directory_naming_two_groups_is_refused_rather_than_picked() {
        let mut root = parsed();
        // A second group whose directory is also `App` — legal, and exactly the
        // case where only the caller knows which one it meant.
        add_group(&mut root, "Other", Some("MG"), Some("App"), "<group>").unwrap();

        let err = add_fileref(&mut root, "X.swift", None, "<group>", Some("App")).unwrap_err();
        assert!(err.contains("App names 2 groups"), "{err}");
        assert!(err.contains("pass the id you mean"), "{err}");

        // The id still names one of them outright.
        assert!(add_fileref(&mut root, "X.swift", None, "<group>", Some("G1")).is_ok());
    }

    #[test]
    fn attach_and_detach_take_a_directory_too() {
        let mut root = parsed();
        let outcome = attach(&mut root, "FR1", "App/Legacy").unwrap();
        assert_eq!(
            outcome,
            LinkOutcome::Linked {
                child: "FR1".to_string(),
                group: "G2".to_string(),
            }
        );
        let outcome = detach(&mut root, "FR1", "App/Legacy").unwrap();
        assert_eq!(
            outcome,
            LinkOutcome::Unlinked {
                child: "FR1".to_string(),
                group: "G2".to_string(),
            }
        );
    }

    /// `move` is what `attach`/`detach` become where a node sits in exactly one
    /// place, so it has to keep working on the format that has both.
    #[test]
    fn moving_a_reference_rewrites_its_path_to_keep_the_same_file() {
        let mut root = parsed();
        // Out of App into the mainGroup, where a `<group>`-relative path is
        // read from the project directory: `App/Main.swift` still reaches it.
        let outcome = move_node(&mut root, "FR1", Some("MG")).unwrap();
        assert_eq!(
            outcome,
            MoveOutcome::Moved {
                address: "FR1".into(),
                from: Some("G1".into()),
                to: "MG".into(),
                resolved: "App/Main.swift".into(),
            }
        );
        let text = round_trips(&root);
        assert!(text.contains("path = App/Main.swift"), "{text}");
        assert_eq!(
            list_filerefs(&root).unwrap()[0].resolved,
            "App/Main.swift",
            "the file it names did not move"
        );

        // Back under App, whose directory contains it: the prefix comes off.
        move_node(&mut root, "FR1", Some("App")).unwrap();
        let text = round_trips(&root);
        assert!(text.contains("path = Main.swift"), "{text}");
        assert!(text.contains("sourceTree = \"<group>\""), "{text}");

        assert_eq!(
            move_node(&mut root, "FR1", Some("App")).unwrap(),
            MoveOutcome::AlreadyThere {
                address: "FR1".into(),
                group: "G1".into(),
            }
        );
    }

    /// A group whose directory cannot reach the file leaves no relative
    /// spelling, so the reference is anchored at the project root instead.
    #[test]
    fn a_move_that_relative_spelling_cannot_follow_anchors_the_path() {
        let mut root = parsed();
        move_node(&mut root, "FR1", Some("G2")).unwrap();
        let text = round_trips(&root);
        assert!(text.contains("path = App/Main.swift"), "{text}");
        assert!(text.contains("sourceTree = SOURCE_ROOT"), "{text}");
        assert_eq!(list_filerefs(&root).unwrap()[0].resolved, "App/Main.swift");
    }

    #[test]
    fn a_group_cannot_be_moved_into_its_own_descendant() {
        let mut root = parsed();
        let err = move_node(&mut root, "G1", Some("G2")).unwrap_err();
        assert!(err.contains("is inside G1"), "{err}");
    }

    /// Without a group the navigator root is meant, which is the `mainGroup`
    /// here and the top of `files` in a `project.xcproj` — one spelling for
    /// both formats.
    #[test]
    fn no_group_named_means_the_navigator_root() {
        let mut root = parsed();
        move_node(&mut root, "FR1", None).unwrap();
        assert_eq!(
            crate::project::parent_group_of(objects(&root).unwrap(), "FR1").as_deref(),
            Some("MG")
        );
        let AddGroupOutcome::Created { address, .. } =
            add_group(&mut root, "Shared", None, Some("Shared"), "<group>").unwrap()
        else {
            panic!("expected a new group");
        };
        assert_eq!(
            crate::project::parent_group_of(objects(&root).unwrap(), &address).as_deref(),
            Some("MG")
        );
    }
}
