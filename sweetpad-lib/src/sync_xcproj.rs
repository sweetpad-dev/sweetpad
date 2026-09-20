//! Synchronized folders in a parsed [`crate::xcproj::Value`] document — the
//! `project.xcproj` counterpart of [`crate::sync_pbxproj`].
//!
//! A folder is a node in the `files` tree with `kind: "folder"`, whose
//! `target-membership` names the targets that build everything under it. Its
//! `membership-exceptions` list carries one entry per target that differs,
//! splitting the pbxproj's single `membershipExceptions` list into named
//! `exclusions` and `inclusions`.
//!
//! Everything here is pure (no I/O): callers parse the file, mutate the tree,
//! and serialize/write it.

pub use crate::membership::{
    AddOutcome, ExcludeOutcome, IncludeOutcome, RemoveOutcome, RootReport, TargetRoots,
};

use crate::xcproj::{Object, Value};

/// Synchronized folders and exceptions per target, in file order — targets
/// with no folders included (empty `roots`), so a report can show "none".
///
/// # Errors
/// Returns a message when the tree is malformed.
pub fn list(root: &Value) -> Result<Vec<TargetRoots>, String> {
    let folders = folders(root);
    Ok(crate::schema_xcproj::target_names(root)
        .into_iter()
        .map(|target| {
            let roots = folders
                .iter()
                .filter(|f| builds_for(f.node, &target))
                .map(|f| RootReport {
                    dir: f.dir.clone(),
                    exceptions: exceptions_of(f.node, &target, "exclusions"),
                })
                .collect();
            TargetRoots { target, roots }
        })
        .collect())
}

/// Attach the folder `dir` (project-dir-relative) to `target`, creating the
/// node when the document has none for that folder. Already attached is a
/// no-op outcome, not an error, so re-run scripts stay green.
///
/// A new node joins the top of the navigator tree as one entry carrying the
/// whole relative path, which is how Xcode shows a folder dragged in from
/// anywhere below the project directory.
///
/// # Errors
/// Returns a message when the tree is malformed or the target is missing.
pub fn add_root(root: &mut Value, target: &str, dir: &str) -> Result<AddOutcome, String> {
    known_target(root, target)?;
    let dir = normalize(dir);
    let found = folders(root)
        .into_iter()
        .find(|f| f.dir == dir)
        .map(|f| (f.indices, builds_for(f.node, target)));
    match found {
        Some((_, true)) => Ok(AddOutcome::AlreadyAttached),
        Some((indices, false)) => {
            let node = node_at_mut(root, &indices).ok_or("no folder node at that path")?;
            add_membership(node, target)?;
            Ok(AddOutcome::AttachedExisting)
        }
        None => {
            let mut node = Object::new();
            node.insert("kind".to_string(), Value::String("folder".to_string()));
            node.insert("path".to_string(), Value::String(dir));
            let mut node = Value::Object(node);
            add_membership(&mut node, target)?;
            files_mut(root)?.push(node);
            Ok(AddOutcome::Created)
        }
    }
}

/// Detach the folder `dir` from `target`, dropping its exceptions with it.
///
/// The node itself stays: in this format a folder is the navigator entry, and
/// one that builds for no target is an ordinary thing to have — Xcode writes
/// exactly that for a folder added for reference only.
///
/// # Errors
/// Returns a message when the tree is malformed or the target is missing.
pub fn remove_root(root: &mut Value, target: &str, dir: &str) -> Result<RemoveOutcome, String> {
    known_target(root, target)?;
    let dir = normalize(dir);
    let Some(indices) = folders(root)
        .into_iter()
        .find(|f| f.dir == dir && builds_for(f.node, target))
        .map(|f| f.indices)
    else {
        return Ok(RemoveOutcome::NotAttached);
    };
    let node = node_at_mut(root, &indices).ok_or("no folder node at that path")?;
    drop_membership(node, target)?;
    drop_exception_set(node, target)?;
    Ok(RemoveOutcome::Detached {
        deleted_object: false,
    })
}

/// Opt `path` out of `target` inside the synchronized folder that holds it.
///
/// # Errors
/// Returns a message when the tree is malformed, the target is missing, or
/// the path lies inside none of the target's folders.
pub fn exclude(root: &mut Value, target: &str, path: &str) -> Result<ExcludeOutcome, String> {
    known_target(root, target)?;
    if let Some(outcome) = ensure_infoplist_exception(root, target, path)? {
        return Ok(outcome);
    }
    let dirs: Vec<String> = folders(root)
        .into_iter()
        .filter(|f| builds_for(f.node, target))
        .map(|f| f.dir)
        .collect();
    Err(if dirs.is_empty() {
        format!("target `{target}` has no synchronized folders")
    } else {
        format!(
            "{} is not inside a synchronized folder of target `{target}` (folders: {})",
            normalize(path),
            dirs.join(", ")
        )
    })
}

/// Drop `target`'s exclusion of `path`. A path that isn't excluded is a no-op
/// outcome, so re-run scripts stay green.
///
/// Only an exclusion is dropped. The format's other kind of exception, an
/// *inclusion*, hands a file to a target the folder does not belong to, so
/// dropping one would take membership away rather than give it back.
///
/// # Errors
/// Returns a message when the tree is malformed or the target is missing.
pub fn include(root: &mut Value, target: &str, path: &str) -> Result<IncludeOutcome, String> {
    known_target(root, target)?;
    let path = normalize(path);
    let Some(found) = containing_folder(root, target, &path) else {
        return Ok(IncludeOutcome::NotExcluded);
    };
    let node = node_at_mut(root, &found.indices).ok_or("no folder node at that path")?;
    if !drop_exclusion(node, target, &found.relative)? {
        return Ok(IncludeOutcome::NotExcluded);
    }
    Ok(IncludeOutcome::Removed {
        root_dir: found.dir,
        exception: found.relative,
    })
}

/// The synchronized folder of `target` that `path` lies inside, as its
/// project-dir-relative directory — `None` when the path lies under none of
/// them. Callers use this to route between exception edits (in-folder) and
/// per-file membership edits (outside).
#[must_use]
pub fn folder_of(root: &Value, target: &str, path: &str) -> Option<String> {
    containing_folder(root, target, &normalize(path)).map(|f| f.dir)
}

/// The `settings set INFOPLIST_FILE=…` hook: except `path` when it lies inside
/// one of `target`'s synchronized folders, and do nothing (`Ok(None)`) when it
/// doesn't — a plist outside every folder needs no exception. Without it an
/// in-folder Info.plist is also copied as a bundle resource, a build failure
/// on flat-bundle platforms (CLI_DESIGN §9f).
///
/// # Errors
/// Returns a message when the tree is malformed.
pub fn ensure_infoplist_exception(
    root: &mut Value,
    target: &str,
    path: &str,
) -> Result<Option<ExcludeOutcome>, String> {
    let path = normalize(path);
    let Some(found) = containing_folder(root, target, &path) else {
        return Ok(None);
    };
    let Some(folder) = node_at_mut(root, &found.indices) else {
        return Err(format!("no folder node at {}", found.dir));
    };
    let added = add_exclusion(folder, target, &found.relative)?;
    Ok(Some(if added {
        ExcludeOutcome::Added {
            root_dir: found.dir,
            exception: found.relative,
        }
    } else {
        ExcludeOutcome::AlreadyExcluded {
            root_dir: found.dir,
            exception: found.relative,
        }
    }))
}

/// The synchronized folder of `target` that `path` lies inside.
struct Found {
    /// How to reach the folder node: an index into `files`, then into each
    /// `children` list below it.
    indices: Vec<usize>,
    /// The folder, project-dir-relative.
    dir: String,
    /// `path` relative to the folder.
    relative: String,
}

fn containing_folder(root: &Value, target: &str, path: &str) -> Option<Found> {
    fn walk(
        nodes: &[Value],
        base: &str,
        target: &str,
        path: &str,
        indices: &mut Vec<usize>,
        depth: usize,
    ) -> Option<Found> {
        if depth >= crate::project::MAX_GROUP_DEPTH {
            return None;
        }
        for (index, node) in nodes.iter().enumerate() {
            let Some(dir) = node_dir(node, base) else {
                continue;
            };
            indices.push(index);
            if let Some(children) = node.get("children").and_then(Value::as_array) {
                if let Some(found) = walk(children, &dir, target, path, indices, depth + 1) {
                    return Some(found);
                }
            } else if node.get("kind").and_then(Value::as_str) == Some("folder")
                && builds_for(node, target)
                && let Some(relative) = path.strip_prefix(&format!("{dir}/"))
            {
                return Some(Found {
                    indices: indices.clone(),
                    dir,
                    relative: relative.to_string(),
                });
            }
            indices.pop();
        }
        None
    }

    walk(
        root.get("files").and_then(Value::as_array)?,
        "",
        target,
        path,
        &mut Vec::new(),
        0,
    )
}

/// A node's directory, project-dir-relative. `None` for a path anchored
/// somewhere the source tree can't reach (`<PRODUCTS>`, `<SDK>`, an absolute
/// path), whose subtree holds nothing a folder edit applies to.
fn node_dir(node: &Value, base: &str) -> Option<String> {
    let Some(path) = node.get("path").and_then(Value::as_str) else {
        return Some(base.to_string());
    };
    let path = if let Some(rest) = path.strip_prefix("<PROJECT>/") {
        return Some(normalize(rest));
    } else if path.starts_with('<') || path.starts_with('/') {
        return None;
    } else {
        normalize(path)
    };
    Some(if base.is_empty() {
        path
    } else {
        format!("{base}/{path}")
    })
}

/// A synchronized folder node, with how to reach it and where it lives.
struct Folder<'a> {
    node: &'a Value,
    indices: Vec<usize>,
    dir: String,
}

/// Every synchronized folder in the document, in file order.
fn folders(root: &Value) -> Vec<Folder<'_>> {
    fn walk<'a>(
        nodes: &'a [Value],
        base: &str,
        indices: &mut Vec<usize>,
        depth: usize,
        out: &mut Vec<Folder<'a>>,
    ) {
        if depth >= crate::project::MAX_GROUP_DEPTH {
            return;
        }
        for (index, node) in nodes.iter().enumerate() {
            let Some(dir) = node_dir(node, base) else {
                continue;
            };
            indices.push(index);
            if let Some(children) = node.get("children").and_then(Value::as_array) {
                walk(children, &dir, indices, depth + 1, out);
            } else if node.get("kind").and_then(Value::as_str) == Some("folder") {
                out.push(Folder {
                    node,
                    indices: indices.clone(),
                    dir,
                });
            }
            indices.pop();
        }
    }

    let mut out = Vec::new();
    walk(
        root.get("files").and_then(Value::as_array).unwrap_or(&[]),
        "",
        &mut Vec::new(),
        0,
        &mut out,
    );
    out
}

/// One kind of exception a folder records for a target, folder-relative.
fn exceptions_of(folder: &Value, target: &str, kind: &str) -> Vec<String> {
    folder
        .get("membership-exceptions")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter(|s| s.get("target").and_then(Value::as_str) == Some(target))
        .filter_map(|s| s.get(kind))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// Refuse a target the document doesn't declare — mutations don't guess.
fn known_target(root: &Value, target: &str) -> Result<(), String> {
    if crate::schema_xcproj::target_names(root)
        .iter()
        .any(|t| t == target)
    {
        Ok(())
    } else {
        Err(format!("no target named {target}"))
    }
}

fn files_mut(root: &mut Value) -> Result<&mut crate::xcproj::Array, String> {
    let root = root
        .as_object_mut()
        .ok_or_else(|| "document root is not an object".to_string())?;
    if root.get("files").is_none() {
        root.insert_sorted("files".to_string(), Value::Array(Vec::new().into()));
    }
    root.get_mut("files")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "files is not an array".to_string())
}

/// Add `target` to a folder's `target-membership`, in sorted position.
fn add_membership(node: &mut Value, target: &str) -> Result<(), String> {
    let node = node
        .as_object_mut()
        .ok_or_else(|| "folder is not an object".to_string())?;
    if node.get("target-membership").is_none() {
        crate::schema_xcproj::insert_node_key(
            node,
            "target-membership",
            Value::Array(Vec::new().into()),
        );
    }
    let members = node
        .get_mut("target-membership")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "target-membership is not an array".to_string())?;
    if members.iter().any(|m| m.as_str() == Some(target)) {
        return Ok(());
    }
    let at = members
        .iter()
        .position(|m| m.as_str().is_some_and(|s| s > target))
        .unwrap_or(members.len());
    members.insert(at, Value::String(target.to_string()));
    Ok(())
}

/// Drop `target` from a folder's `target-membership`, dropping the key when
/// it empties — Xcode omits empty collections.
fn drop_membership(node: &mut Value, target: &str) -> Result<(), String> {
    let node = node
        .as_object_mut()
        .ok_or_else(|| "folder is not an object".to_string())?;
    let Some(members) = node
        .get_mut("target-membership")
        .and_then(Value::as_array_mut)
    else {
        return Ok(());
    };
    members.retain(|m| m.as_str() != Some(target));
    if members.is_empty() {
        node.remove("target-membership");
    }
    Ok(())
}

/// Drop a target's whole exception entry from a folder.
fn drop_exception_set(node: &mut Value, target: &str) -> Result<(), String> {
    let node = node
        .as_object_mut()
        .ok_or_else(|| "folder is not an object".to_string())?;
    let Some(sets) = node
        .get_mut("membership-exceptions")
        .and_then(Value::as_array_mut)
    else {
        return Ok(());
    };
    sets.retain(|s| s.get("target").and_then(Value::as_str) != Some(target));
    if sets.is_empty() {
        node.remove("membership-exceptions");
    }
    Ok(())
}

/// Drop one exclusion, pruning the entry and the list as they empty. Returns
/// whether it was there.
fn drop_exclusion(node: &mut Value, target: &str, relative: &str) -> Result<bool, String> {
    let node = node
        .as_object_mut()
        .ok_or_else(|| "folder is not an object".to_string())?;
    let Some(sets) = node
        .get_mut("membership-exceptions")
        .and_then(Value::as_array_mut)
    else {
        return Ok(false);
    };
    let Some(set) = sets
        .iter_mut()
        .find(|s| s.get("target").and_then(Value::as_str) == Some(target))
        .and_then(Value::as_object_mut)
    else {
        return Ok(false);
    };
    let Some(exclusions) = set.get_mut("exclusions").and_then(Value::as_array_mut) else {
        return Ok(false);
    };
    let before = exclusions.len();
    exclusions.retain(|e| e.as_str() != Some(relative));
    if exclusions.len() == before {
        return Ok(false);
    }
    if exclusions.is_empty() {
        set.remove("exclusions");
    }
    if set.is_empty() || set.iter().all(|(k, _)| k == "target") {
        sets.retain(|s| s.get("target").and_then(Value::as_str) != Some(target));
    }
    if sets.is_empty() {
        node.remove("membership-exceptions");
    }
    Ok(true)
}

fn builds_for(node: &Value, target: &str) -> bool {
    node.get("target-membership")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .any(|m| m == target)
}

/// Add `relative` to the folder's exclusions for `target`, growing the
/// exception entry when the target has none. Returns whether it was new.
fn add_exclusion(folder: &mut Value, target: &str, relative: &str) -> Result<bool, String> {
    let folder = folder
        .as_object_mut()
        .ok_or_else(|| "folder is not an object".to_string())?;
    if folder.get("membership-exceptions").is_none() {
        crate::schema_xcproj::insert_node_key(
            folder,
            "membership-exceptions",
            Value::Array(Vec::new().into()),
        );
    }
    let sets = folder
        .get_mut("membership-exceptions")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "membership-exceptions is not an array".to_string())?;
    if !sets
        .iter()
        .any(|s| s.get("target").and_then(Value::as_str) == Some(target))
    {
        let mut set = Object::new();
        set.insert("target".to_string(), Value::String(target.to_string()));
        sets.push(Value::Object(set));
    }
    let set = sets
        .iter_mut()
        .find(|s| s.get("target").and_then(Value::as_str) == Some(target))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "membership exception is not an object".to_string())?;
    if set.get("exclusions").is_none() {
        crate::schema_xcproj::insert_exception_key(
            set,
            "exclusions",
            Value::Array(Vec::new().into()),
        );
    }
    let exclusions = set
        .get_mut("exclusions")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "exclusions is not an array".to_string())?;
    if exclusions.iter().any(|e| e.as_str() == Some(relative)) {
        return Ok(false);
    }
    let at = exclusions
        .iter()
        .position(|e| e.as_str().is_some_and(|s| s > relative))
        .unwrap_or(exclusions.len());
    exclusions.insert(at, Value::String(relative.to_string()));
    Ok(true)
}

/// Descend to the node an index path names: into `files`, then into each
/// `children` list below it.
fn node_at_mut<'a>(root: &'a mut Value, indices: &[usize]) -> Option<&'a mut Value> {
    let (&first, rest) = indices.split_first()?;
    let mut node = root
        .get_mut("files")
        .and_then(Value::as_array_mut)?
        .get_mut(first)?;
    for &index in rest {
        node = node
            .get_mut("children")
            .and_then(Value::as_array_mut)?
            .get_mut(index)?;
    }
    Some(node)
}

/// A stored path in the shape this module compares: no surrounding space, no
/// leading `./` or `/`, no trailing separator.
fn normalize(path: &str) -> String {
    path.trim()
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}
