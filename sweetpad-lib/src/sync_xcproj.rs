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

pub use crate::synchronized::{ExcludeOutcome, IncludeOutcome};

use crate::xcproj::{Object, Value};

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
        folder.insert_sorted(
            "membership-exceptions".to_string(),
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
        set.insert_sorted("target".to_string(), Value::String(target.to_string()));
        sets.push(Value::Object(set));
    }
    let set = sets
        .iter_mut()
        .find(|s| s.get("target").and_then(Value::as_str) == Some(target))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "membership exception is not an object".to_string())?;
    if set.get("exclusions").is_none() {
        set.insert_sorted("exclusions".to_string(), Value::Array(Vec::new().into()));
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
