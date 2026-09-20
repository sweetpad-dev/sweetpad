//! Per-file target membership in a parsed [`crate::xcproj::Value`] document —
//! the `project.xcproj` counterpart of [`crate::membership_pbxproj`].
//!
//! The relation is inverted from the pbxproj's. There a build phase lists
//! `PBXBuildFile` objects that point at file references; here the file's own
//! node carries `target-membership`, whose entries name `<target>/<phase>` as
//! a bare string, or as an object under `build-phase` when the membership has
//! details. Those details map onto the pbxproj's one for one, measured by
//! converting a project that carries each:
//!
//! | `PBXBuildFile` | node |
//! | --- | --- |
//! | `settings.COMPILER_FLAGS` | `arguments` |
//! | `settings.ATTRIBUTES` `Public` / `Private` | `header-role` |
//! | `settings.ATTRIBUTES` `RemoveHeadersOnCopy` | `header-preservation` |
//! | `settings.ATTRIBUTES` `CodeSignOnCopy` | `code-sign-on-copy` |
//! | `platformFilters` | `platforms` |
//!
//! Phases are spelled `compile-sources`, `resources`, `headers`, `frameworks`
//! and `copy/<name>`; a bare target name with no phase is a synchronized
//! folder, which is [`crate::sync_xcproj`]'s business.
//!
//! Everything here is pure (no I/O): callers parse the file, mutate the tree,
//! and serialize/write it.

pub use crate::membership::{Addition, FileEntry, Phase, RefKind, Removal};

use crate::xcproj::Value;

/// A target's per-file membership entries, in navigator order.
///
/// # Errors
/// Returns a message when the target is missing.
pub fn classic_members(root: &Value, target: &str) -> Result<Vec<FileEntry>, String> {
    known_target(root, target)?;
    Ok(nodes(root)
        .into_iter()
        .flat_map(|node| {
            memberships(node.value)
                .filter(|m| m.target == target)
                .map(|m| FileEntry {
                    path: node.path.clone(),
                    phase: m.phase,
                    kind: ref_kind(node.value),
                    compiler_flags: m.compiler_flags,
                    attributes: m.attributes,
                    platform_filters: m.platform_filters,
                })
                .collect::<Vec<_>>()
        })
        .collect())
}

/// Give `target` a membership on each path, in `phase`.
///
/// Every path must already be in the navigator tree. A node is the file's
/// existence in this format, so there is nothing separate to create first and
/// nothing to invent: a path the document does not hold is an error naming it.
///
/// # Errors
/// Returns a message when the target is missing or a path is not in the tree.
pub fn add_membership(
    root: &mut Value,
    target: &str,
    paths: &[String],
    phase: &Phase,
) -> Result<Vec<Addition>, String> {
    known_target(root, target)?;
    let spelling = phase_spelling(target, phase);
    let mut additions = Vec::with_capacity(paths.len());
    for path in paths {
        let path = normalize(path);
        let Some(found) = nodes(root).into_iter().find(|n| n.path == path) else {
            return Err(format!(
                "{path} is not in the project's navigator tree, so there is nothing to \
                 give a membership"
            ));
        };
        let already = memberships(found.value).any(|m| m.target == target && m.phase == *phase);
        if !already {
            let indices = found.indices;
            let node = node_at_mut(root, &indices).ok_or("no node at that path")?;
            insert_membership(node, &spelling)?;
        }
        additions.push(Addition {
            path,
            phase: phase.display(),
            build_file: None,
            already_member: already,
        });
    }
    Ok(additions)
}

/// Take `target`'s membership off each path, in every phase it held.
///
/// The node stays in the navigator: here it *is* the file's place in the
/// project, and Xcode keeps a file listed after the last target stops
/// building it.
///
/// # Errors
/// Returns a message when the target is missing.
pub fn remove_membership(
    root: &mut Value,
    target: &str,
    paths: &[String],
) -> Result<Vec<Removal>, String> {
    known_target(root, target)?;
    let mut removals = Vec::with_capacity(paths.len());
    for path in paths {
        let path = normalize(path);
        let Some(found) = nodes(root).into_iter().find(|n| n.path == path) else {
            removals.push(Removal {
                path,
                removed_phases: Vec::new(),
                deleted_reference: false,
                pruned_groups: 0,
            });
            continue;
        };
        let removed_phases: Vec<String> = memberships(found.value)
            .filter(|m| m.target == target)
            .map(|m| m.phase.display())
            .collect();
        if !removed_phases.is_empty() {
            let indices = found.indices;
            let node = node_at_mut(root, &indices).ok_or("no node at that path")?;
            drop_memberships(node, target)?;
        }
        removals.push(Removal {
            path,
            removed_phases,
            deleted_reference: false,
            pruned_groups: 0,
        });
    }
    Ok(removals)
}

/// Whether `path` carries a per-file membership for `target` — what tells a
/// caller to reach for this module rather than [`crate::sync_xcproj`].
#[must_use]
pub fn has_entry(root: &Value, target: &str, path: &str) -> bool {
    let path = normalize(path);
    nodes(root)
        .into_iter()
        .filter(|n| n.path == path)
        .any(|n| memberships(n.value).any(|m| m.target == target))
}

/// One membership an entry states.
struct Membership {
    target: String,
    phase: Phase,
    compiler_flags: Option<String>,
    attributes: Vec<String>,
    platform_filters: Vec<String>,
}

/// Every per-file membership a node states. An entry naming a target with no
/// phase is a synchronized folder's and is not one of these.
fn memberships(node: &Value) -> impl Iterator<Item = Membership> + '_ {
    node.get("target-membership")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| {
            let spelling = entry
                .as_str()
                .or_else(|| entry.get("build-phase").and_then(Value::as_str))?;
            let (target, phase) = split_spelling(spelling)?;
            Some(Membership {
                target,
                phase,
                compiler_flags: entry
                    .get("arguments")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                attributes: attributes(entry),
                platform_filters: entry
                    .get("platforms")
                    .and_then(Value::as_array)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
            })
        })
}

/// The pbxproj's `settings.ATTRIBUTES` spellings, rebuilt from the keys this
/// format splits them into, so a report reads the same on either.
fn attributes(entry: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match entry.get("header-role").and_then(Value::as_str) {
        Some("public") => out.push("Public".to_string()),
        Some("private") => out.push("Private".to_string()),
        _ => {}
    }
    if entry.get("header-preservation").and_then(Value::as_str) == Some("remove-on-copy") {
        out.push("RemoveHeadersOnCopy".to_string());
    }
    if entry.get("code-sign-on-copy").and_then(Value::as_bool) == Some(true) {
        out.push("CodeSignOnCopy".to_string());
    }
    out
}

/// Split `<target>/<phase>` into its parts. A target's own name can hold a
/// `/`, so the phase is taken from the right.
fn split_spelling(spelling: &str) -> Option<(String, Phase)> {
    for (at, _) in spelling.rmatch_indices('/') {
        let (target, rest) = (&spelling[..at], &spelling[at + 1..]);
        let phase = match rest {
            "compile-sources" => Phase::Sources,
            "resources" => Phase::Resources,
            "headers" => Phase::Headers,
            "frameworks" => Phase::Frameworks,
            _ => match rest.strip_prefix("copy/") {
                Some(name) => Phase::Copy(name.to_string()),
                None => continue,
            },
        };
        return Some((target.to_string(), phase));
    }
    None
}

/// How a membership on `phase` is written.
fn phase_spelling(target: &str, phase: &Phase) -> String {
    let phase = match phase {
        Phase::Sources => "compile-sources".to_string(),
        Phase::Resources => "resources".to_string(),
        Phase::Headers => "headers".to_string(),
        Phase::Frameworks => "frameworks".to_string(),
        Phase::Copy(name) => format!("copy/{name}"),
    };
    format!("{target}/{phase}")
}

fn ref_kind(node: &Value) -> RefKind {
    match node.get("kind").and_then(Value::as_str) {
        Some("variant-group") => RefKind::VariantGroup,
        Some("version-group") => RefKind::VersionGroup,
        Some(_) => RefKind::Other,
        None => RefKind::File,
    }
}

/// A file node, with how to reach it and where it lives.
struct Node<'a> {
    value: &'a Value,
    indices: Vec<usize>,
    path: String,
}

/// Every node in the navigator tree that can hold a membership, in file
/// order. A group is walked into rather than reported; a folder belongs to
/// [`crate::sync_xcproj`].
fn nodes(root: &Value) -> Vec<Node<'_>> {
    fn walk<'a>(
        children: &'a [Value],
        base: &str,
        indices: &mut Vec<usize>,
        depth: usize,
        out: &mut Vec<Node<'a>>,
    ) {
        if depth >= crate::project::MAX_GROUP_DEPTH {
            return;
        }
        for (index, node) in children.iter().enumerate() {
            let Some(path) = node_path(node, base) else {
                continue;
            };
            indices.push(index);
            match node.get("children").and_then(Value::as_array) {
                Some(grandchildren)
                    if node.get("kind").and_then(Value::as_str) == Some("group") =>
                {
                    walk(grandchildren, &path, indices, depth + 1, out);
                }
                _ => {
                    if node.get("kind").and_then(Value::as_str) != Some("folder") {
                        out.push(Node {
                            value: node,
                            indices: indices.clone(),
                            path,
                        });
                    }
                }
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

/// A node's path, project-dir-relative. `None` for one anchored somewhere the
/// source tree can't reach (`<PRODUCTS>`, `<SDK>`, an absolute path).
fn node_path(node: &Value, base: &str) -> Option<String> {
    let Some(path) = node.get("path").and_then(Value::as_str) else {
        return Some(base.to_string());
    };
    if let Some(rest) = path.strip_prefix("<PROJECT>/") {
        return Some(normalize(rest));
    }
    if path.starts_with('<') || path.starts_with('/') {
        return None;
    }
    let path = normalize(path);
    Some(if base.is_empty() {
        path
    } else {
        format!("{base}/{path}")
    })
}

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

/// Add one membership spelling to a node, in sorted position.
fn insert_membership(node: &mut Value, spelling: &str) -> Result<(), String> {
    let node = node
        .as_object_mut()
        .ok_or_else(|| "file node is not an object".to_string())?;
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
    let at = members
        .iter()
        .position(|m| membership_key(m).is_some_and(|k| k > spelling))
        .unwrap_or(members.len());
    members.insert(at, Value::String(spelling.to_string()));
    Ok(())
}

/// Drop every membership a node states for `target`, dropping the key when it
/// empties — Xcode omits empty collections.
fn drop_memberships(node: &mut Value, target: &str) -> Result<(), String> {
    let node = node
        .as_object_mut()
        .ok_or_else(|| "file node is not an object".to_string())?;
    let Some(members) = node
        .get_mut("target-membership")
        .and_then(Value::as_array_mut)
    else {
        return Ok(());
    };
    members.retain(|m| {
        membership_key(m)
            .and_then(split_spelling)
            .is_none_or(|(named, _)| named != target)
    });
    if members.is_empty() {
        node.remove("target-membership");
    }
    Ok(())
}

/// The `<target>/<phase>` an entry states, whichever shape it takes.
fn membership_key(entry: &Value) -> Option<&str> {
    entry
        .as_str()
        .or_else(|| entry.get("build-phase").and_then(Value::as_str))
}

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

fn normalize(path: &str) -> String {
    path.trim()
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}
