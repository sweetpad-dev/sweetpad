//! Reading and mutating Xcode 16 synchronized folders —
//! `PBXFileSystemSynchronizedRootGroup` objects and their
//! `PBXFileSystemSynchronizedBuildFileExceptionSet`s — in a parsed
//! [`crate::pbxproj::Value`] tree.
//!
//! A synchronized root makes target membership implicit: every file under the
//! folder belongs to the target, with per-target `membershipExceptions`
//! opting individual files out. `sweetpad source …` drives this module, and
//! `settings set INFOPLIST_FILE=…` uses [`ensure_infoplist_exception`] so a
//! custom Info.plist inside a root doesn't double as a bundle resource
//! (a hard "Multiple commands produce" failure on flat-bundle platforms).
//!
//! Everything here is pure (no I/O): callers parse the file, mutate the tree,
//! and serialize/write it — the same contract as [`crate::spm_pbxproj`]. New
//! roots are written single-line and exception sets multi-line with sorted
//! `membershipExceptions`, matching Xcode's own output byte-for-byte.

use std::path::Path;

use crate::pbxproj::{Dict, Value};
use crate::settings_pbxproj::insert_sorted;
use crate::spm_pbxproj::fresh_guid;

const ROOT_ISA: &str = "PBXFileSystemSynchronizedRootGroup";
const EXCEPTION_ISA: &str = "PBXFileSystemSynchronizedBuildFileExceptionSet";

/// One synchronized root as seen by one target: where the folder lives and
/// which of its files that target opts out of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootReport {
    pub guid: String,
    /// Project-dir-relative folder path (group-tree walk, like `SRCROOT`).
    pub dir: String,
    /// The target's `membershipExceptions`, root-relative, in file order.
    pub exceptions: Vec<String>,
}

/// A target's synchronized roots, for `source list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRoots {
    pub target: String,
    pub roots: Vec<RootReport>,
}

/// The result of attaching a root: a brand-new object, an existing root (used
/// by another target) newly attached, or nothing to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddOutcome {
    Created(String),
    AttachedExisting(String),
    AlreadyAttached(String),
}

/// The result of detaching a root. `deleted_object` is set when no other
/// target still references it, so the group object itself was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveOutcome {
    Detached { guid: String, deleted_object: bool },
    NotAttached,
}

/// The result of adding a membership exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExcludeOutcome {
    /// `exception` is the root-relative path now excepted under `root_dir`.
    Added {
        root_dir: String,
        exception: String,
    },
    AlreadyExcluded {
        root_dir: String,
        exception: String,
    },
}

/// The result of dropping a membership exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncludeOutcome {
    Removed { root_dir: String, exception: String },
    NotExcluded,
}

/// Synchronized roots and exceptions per target, in file order — targets with
/// no roots included (empty `roots`), so a report can show "none".
///
/// # Errors
/// Returns a message when the tree has no objects dict.
pub fn list(root: &Value) -> Result<Vec<TargetRoots>, String> {
    let objects = objects(root).ok_or("pbxproj has no objects dict")?;
    let mut out = Vec::new();
    for (target_guid, obj) in objects.iter() {
        if !is_target_isa(isa(obj)) {
            continue;
        }
        let Some(target_name) = str_field(obj, "name") else {
            continue;
        };
        let mut roots = Vec::new();
        for root_guid in attached_roots(objects, target_guid) {
            roots.push(RootReport {
                dir: root_dir(objects, &root_guid),
                exceptions: exceptions_of(objects, &root_guid, target_guid),
                guid: root_guid,
            });
        }
        out.push(TargetRoots {
            target: target_name.to_string(),
            roots,
        });
    }
    Ok(out)
}

/// Attach the folder `dir` (project-dir-relative) to `target` as a
/// synchronized root. Reuses an existing root object for the same folder
/// (roots are shared across targets, as Xcode does); already-attached is a
/// no-op outcome, not an error, so re-run scripts stay green.
///
/// # Errors
/// Returns a message when the tree is malformed or the target is missing.
pub fn add_root(root: &mut Value, target: &str, dir: &str) -> Result<AddOutcome, String> {
    let dir = normalize(dir);
    if dir.is_empty() {
        return Err("the folder path must not be empty".to_string());
    }
    let main_group = main_group_guid(root)?;
    let products_group = products_group_guid(root);
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let target_guid = find_target_guid(objects_ref, target)?;

    if let Some(existing) = root_guid_for_dir(objects_ref, &dir) {
        if attached_roots(objects_ref, &target_guid).contains(&existing) {
            return Ok(AddOutcome::AlreadyAttached(existing));
        }
        let objects = objects_mut(root)?;
        attach_to_target(objects, &target_guid, &existing);
        return Ok(AddOutcome::AttachedExisting(existing));
    }

    let objects = objects_mut(root)?;
    let guid = fresh_guid(objects, &format!("syncroot#{dir}"), 0);
    // The minimal shape Xcode 16's fresh templates write (converted projects
    // additionally carry explicitFileTypes/explicitFolders; both parse).
    let mut group = Dict::new();
    group.insert("isa".into(), vstr(ROOT_ISA));
    group.insert("path".into(), vstr(&dir));
    group.insert("sourceTree".into(), vstr("<group>"));
    objects.insert(guid.clone(), Value::Dict(group));

    attach_to_target(objects, &target_guid, &guid);
    insert_child(objects, &main_group, products_group.as_deref(), &guid);
    Ok(AddOutcome::Created(guid))
}

/// Detach the root at `dir` from `target`, dropping the target's exception
/// set for it. The group object itself (and its group-tree entry) goes only
/// when no other target still lists it.
///
/// # Errors
/// Returns a message when the tree is malformed or the target is missing.
pub fn remove_root(root: &mut Value, target: &str, dir: &str) -> Result<RemoveOutcome, String> {
    let dir = normalize(dir);
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let target_guid = find_target_guid(objects_ref, target)?;
    let Some(guid) = attached_roots(objects_ref, &target_guid)
        .into_iter()
        .find(|g| root_dir(objects_ref, g) == dir)
    else {
        return Ok(RemoveOutcome::NotAttached);
    };

    let objects = objects_mut(root)?;
    remove_from_array(objects, &target_guid, "fileSystemSynchronizedGroups", &guid);
    if let Some(set_guid) = exception_set_of(objects, &guid, &target_guid) {
        remove_from_array(objects, &guid, "exceptions", &set_guid);
        drop_key_if_empty_array(objects, &guid, "exceptions");
        objects.remove(&set_guid);
    }

    let still_referenced = objects.iter().any(|(_, o)| {
        is_target_isa(isa(o))
            && o.get("fileSystemSynchronizedGroups")
                .and_then(Value::as_array)
                .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(guid.as_str())))
    });
    if !still_referenced {
        // Drop the object, its group-tree entry, and any leftover exception
        // sets other targets had on it.
        let leftover_sets: Vec<String> = objects
            .get(&guid)
            .and_then(|o| o.get("exceptions"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        for set in leftover_sets {
            objects.remove(&set);
        }
        let group_guids: Vec<String> = objects
            .iter()
            .filter(|(_, o)| matches!(isa(o), "PBXGroup" | "PBXVariantGroup" | "XCVersionGroup"))
            .map(|(g, _)| g.clone())
            .collect();
        for g in group_guids {
            remove_from_array(objects, &g, "children", &guid);
        }
        objects.remove(&guid);
    }
    Ok(RemoveOutcome::Detached {
        guid,
        deleted_object: !still_referenced,
    })
}

/// Add a membership exception for `path` (project-dir-relative) to `target`'s
/// exception set on the synchronized root containing it. Idempotent.
///
/// # Errors
/// Returns a message when the tree is malformed, the target is missing, or
/// `path` lies inside none of the target's synchronized roots.
pub fn exclude(root: &mut Value, target: &str, path: &str) -> Result<ExcludeOutcome, String> {
    if let Some(outcome) = exclude_if_inside(root, target, path)? {
        Ok(outcome)
    } else {
        let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
        let target_guid = find_target_guid(objects_ref, target)?;
        let dirs: Vec<String> = attached_roots(objects_ref, &target_guid)
            .iter()
            .map(|g| root_dir(objects_ref, g))
            .collect();
        Err(if dirs.is_empty() {
            format!("target `{target}` has no synchronized folders")
        } else {
            format!(
                "{} is not inside a synchronized folder of target `{target}` \
                 (folders: {})",
                normalize(path),
                dirs.join(", ")
            )
        })
    }
}

/// Drop `target`'s membership exception for `path`. A path that isn't
/// excepted is a no-op outcome, so re-run scripts stay green.
///
/// # Errors
/// Returns a message when the tree is malformed or the target is missing.
pub fn include(root: &mut Value, target: &str, path: &str) -> Result<IncludeOutcome, String> {
    let path = normalize(path);
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let target_guid = find_target_guid(objects_ref, target)?;
    let Some((root_guid, rel)) = containing_root(objects_ref, &target_guid, &path) else {
        return Ok(IncludeOutcome::NotExcluded);
    };
    let root_dir_name = root_dir(objects_ref, &root_guid);
    let Some(set_guid) = exception_set_of(objects_ref, &root_guid, &target_guid) else {
        return Ok(IncludeOutcome::NotExcluded);
    };

    let objects = objects_mut(root)?;
    if !remove_from_array(objects, &set_guid, "membershipExceptions", &rel) {
        return Ok(IncludeOutcome::NotExcluded);
    }
    let now_empty = objects
        .get(&set_guid)
        .and_then(|s| s.get("membershipExceptions"))
        .and_then(Value::as_array)
        .is_none_or(<[Value]>::is_empty);
    if now_empty {
        objects.remove(&set_guid);
        remove_from_array(objects, &root_guid, "exceptions", &set_guid);
        drop_key_if_empty_array(objects, &root_guid, "exceptions");
    }
    Ok(IncludeOutcome::Removed {
        root_dir: root_dir_name,
        exception: rel,
    })
}

/// The `settings set INFOPLIST_FILE=…` hook: except `path` when it lies
/// inside one of `target`'s synchronized roots, and do nothing (`Ok(None)`)
/// when it doesn't — a plist outside every root needs no exception. Verified
/// live (CLI_DESIGN §9f): without this, an in-root Info.plist is also copied
/// as a bundle resource — a build failure on flat-bundle platforms.
///
/// # Errors
/// Returns a message when the tree is malformed or the target is missing.
pub fn ensure_infoplist_exception(
    root: &mut Value,
    target: &str,
    path: &str,
) -> Result<Option<ExcludeOutcome>, String> {
    exclude_if_inside(root, target, path)
}

/// Shared body of [`exclude`] / [`ensure_infoplist_exception`]: `Ok(None)`
/// when `path` lies inside none of the target's roots.
fn exclude_if_inside(
    root: &mut Value,
    target: &str,
    path: &str,
) -> Result<Option<ExcludeOutcome>, String> {
    let path = normalize(path);
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let target_guid = find_target_guid(objects_ref, target)?;
    let Some((root_guid, rel)) = containing_root(objects_ref, &target_guid, &path) else {
        return Ok(None);
    };
    let root_dir_name = root_dir(objects_ref, &root_guid);
    let existing_set = exception_set_of(objects_ref, &root_guid, &target_guid);

    let objects = objects_mut(root)?;
    let set_guid = if let Some(guid) = existing_set {
        guid
    } else {
        let guid = fresh_guid(objects, &format!("exceptions#{root_guid}#{target_guid}"), 0);
        let mut set = Dict::new();
        set.insert("isa".into(), vstr(EXCEPTION_ISA));
        set.insert("membershipExceptions".into(), Value::Array(Vec::new()));
        set.insert("target".into(), vstr(&target_guid));
        objects.insert(guid.clone(), Value::Dict(set));
        push_sorted_guid(objects, &root_guid, "exceptions", &guid);
        guid
    };
    let exceptions = objects
        .get_mut(&set_guid)
        .and_then(Value::as_dict_mut)
        .and_then(|s| s.get_mut("membershipExceptions"))
        .and_then(Value::as_array_mut)
        .ok_or("exception set has no membershipExceptions array")?;
    if exceptions.iter().any(|v| v.as_str() == Some(rel.as_str())) {
        return Ok(Some(ExcludeOutcome::AlreadyExcluded {
            root_dir: root_dir_name,
            exception: rel,
        }));
    }
    // Xcode keeps membershipExceptions sorted alphabetically.
    let at = exceptions
        .iter()
        .position(|v| v.as_str() > Some(rel.as_str()))
        .unwrap_or(exceptions.len());
    exceptions.insert(at, vstr(&rel));
    Ok(Some(ExcludeOutcome::Added {
        root_dir: root_dir_name,
        exception: rel,
    }))
}

/// The root of `target` containing `path`, with the path made root-relative.
fn containing_root(objects: &Dict, target_guid: &str, path: &str) -> Option<(String, String)> {
    for guid in attached_roots(objects, target_guid) {
        let dir = root_dir(objects, &guid);
        if dir.is_empty() {
            continue;
        }
        if let Some(rel) = path.strip_prefix(&format!("{dir}/")) {
            return Some((guid, rel.to_string()));
        }
    }
    None
}

/// The project-dir-relative directory of a synchronized root (group-tree
/// walk, honoring parent group paths and `sourceTree`).
fn root_dir(objects: &Dict, guid: &str) -> String {
    crate::project::group_dir(objects, guid, Path::new(""), 0)
        .to_string_lossy()
        .into_owned()
}

fn root_guid_for_dir(objects: &Dict, dir: &str) -> Option<String> {
    objects
        .iter()
        .filter(|(_, o)| isa(o) == ROOT_ISA)
        .find(|(g, _)| root_dir(objects, g) == dir)
        .map(|(g, _)| g.clone())
}

fn attached_roots(objects: &Dict, target_guid: &str) -> Vec<String> {
    objects
        .get(target_guid)
        .and_then(|t| t.get("fileSystemSynchronizedGroups"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The GUID of `target`'s exception set on `root_guid`, if any.
fn exception_set_of(objects: &Dict, root_guid: &str, target_guid: &str) -> Option<String> {
    let sets = objects.get(root_guid)?.get("exceptions")?.as_array()?;
    sets.iter()
        .filter_map(Value::as_str)
        .find(|set_guid| {
            objects
                .get(set_guid)
                .is_some_and(|s| str_field(s, "target") == Some(target_guid))
        })
        .map(str::to_string)
}

/// `target`'s membershipExceptions on `root_guid`, in file order.
fn exceptions_of(objects: &Dict, root_guid: &str, target_guid: &str) -> Vec<String> {
    exception_set_of(objects, root_guid, target_guid)
        .and_then(|set_guid| {
            objects
                .get(&set_guid)?
                .get("membershipExceptions")?
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
        })
        .unwrap_or_default()
}

fn attach_to_target(objects: &mut Dict, target_guid: &str, root_guid: &str) {
    let Some(target) = objects.get_mut(target_guid).and_then(Value::as_dict_mut) else {
        return;
    };
    match target
        .get_mut("fileSystemSynchronizedGroups")
        .and_then(Value::as_array_mut)
    {
        Some(arr) => arr.push(vstr(root_guid)),
        None => insert_sorted(
            target,
            "fileSystemSynchronizedGroups",
            Value::Array(vec![vstr(root_guid)]),
        ),
    }
}

/// Add `guid` to `parent`'s children, just before the Products group when it
/// is a child (folders sit above Products in Xcode's tree), else at the end.
fn insert_child(objects: &mut Dict, parent: &str, products_group: Option<&str>, guid: &str) {
    let Some(children) = objects
        .get_mut(parent)
        .and_then(Value::as_dict_mut)
        .and_then(|g| g.get_mut("children"))
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    let at = products_group
        .and_then(|pg| children.iter().position(|v| v.as_str() == Some(pg)))
        .unwrap_or(children.len());
    children.insert(at, vstr(guid));
}

/// Push a GUID into an object's array field, creating the field (at its
/// alphabetical position) when absent.
fn push_sorted_guid(objects: &mut Dict, owner: &str, key: &str, guid: &str) {
    let Some(owner) = objects.get_mut(owner).and_then(Value::as_dict_mut) else {
        return;
    };
    match owner.get_mut(key).and_then(Value::as_array_mut) {
        Some(arr) => arr.push(vstr(guid)),
        None => insert_sorted(owner, key, Value::Array(vec![vstr(guid)])),
    }
}

/// Remove a string from an object's array field. Returns whether it was there.
fn remove_from_array(objects: &mut Dict, owner: &str, key: &str, value: &str) -> bool {
    let Some(arr) = objects
        .get_mut(owner)
        .and_then(Value::as_dict_mut)
        .and_then(|o| o.get_mut(key))
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    let before = arr.len();
    arr.retain(|v| v.as_str() != Some(value));
    arr.len() != before
}

/// Drop an array-valued key once it has emptied (Xcode omits empty
/// `exceptions` arrays rather than writing `()`).
fn drop_key_if_empty_array(objects: &mut Dict, owner: &str, key: &str) {
    let Some(owner) = objects.get_mut(owner).and_then(Value::as_dict_mut) else {
        return;
    };
    if owner
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(<[Value]>::is_empty)
    {
        owner.remove(key);
    }
}

/// Trim `./` prefixes and trailing slashes so path comparisons are textual.
fn normalize(path: &str) -> String {
    let mut p = path;
    while let Some(rest) = p.strip_prefix("./") {
        p = rest;
    }
    p.trim_end_matches('/').to_string()
}

fn find_target_guid(objects: &Dict, name: &str) -> Result<String, String> {
    objects
        .iter()
        .find(|(_, o)| is_target_isa(isa(o)) && str_field(o, "name") == Some(name))
        .map(|(g, _)| g.clone())
        .ok_or_else(|| {
            let known: Vec<&str> = objects
                .iter()
                .filter(|(_, o)| is_target_isa(isa(o)))
                .filter_map(|(_, o)| str_field(o, "name"))
                .collect();
            format!(
                "no target named `{name}` (project has: {})",
                known.join(", ")
            )
        })
}

fn main_group_guid(root: &Value) -> Result<String, String> {
    let objects = objects(root).ok_or("pbxproj has no objects dict")?;
    let project = root
        .as_dict()
        .and_then(|d| d.get("rootObject"))
        .and_then(Value::as_str)
        .and_then(|g| objects.get(g))
        .ok_or("pbxproj has no PBXProject object")?;
    project
        .get("mainGroup")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "PBXProject has no mainGroup".to_string())
}

fn products_group_guid(root: &Value) -> Option<String> {
    let objects = objects(root)?;
    let project = root
        .as_dict()?
        .get("rootObject")
        .and_then(Value::as_str)
        .and_then(|g| objects.get(g))?;
    project
        .get("productRefGroup")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn objects(root: &Value) -> Option<&Dict> {
    root.as_dict()?.get("objects")?.as_dict()
}

fn objects_mut(root: &mut Value) -> Result<&mut Dict, String> {
    root.as_dict_mut()
        .and_then(|d| d.get_mut("objects"))
        .and_then(Value::as_dict_mut)
        .ok_or_else(|| "pbxproj has no objects dict".to_string())
}

fn isa(obj: &Value) -> &str {
    obj.get("isa").and_then(Value::as_str).unwrap_or("")
}

fn str_field<'a>(obj: &'a Value, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

fn is_target_isa(isa: &str) -> bool {
    matches!(
        isa,
        "PBXNativeTarget" | "PBXAggregateTarget" | "PBXLegacyTarget"
    )
}

fn vstr(value: &str) -> Value {
    Value::String(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"// !$*UTF8*$!
{
	archiveVersion = 1;
	classes = {
	};
	objectVersion = 77;
	objects = {
		P1 /* Project object */ = {
			isa = PBXProject;
			buildConfigurationList = CLP;
			mainGroup = MG;
			productRefGroup = PG /* Products */;
			targets = (
				T1,
				T2,
			);
		};
		MG = {
			isa = PBXGroup;
			children = (
				SR1 /* App */,
				PG /* Products */,
			);
			sourceTree = "<group>";
		};
		PG /* Products */ = {
			isa = PBXGroup;
			children = (
			);
			name = Products;
			sourceTree = "<group>";
		};
		SR1 /* App */ = {isa = PBXFileSystemSynchronizedRootGroup; explicitFileTypes = {}; explicitFolders = (); path = App; sourceTree = "<group>"; };
		T1 /* App */ = {
			isa = PBXNativeTarget;
			buildConfigurationList = CLT1;
			fileSystemSynchronizedGroups = (
				SR1 /* App */,
			);
			name = App;
			productType = "com.apple.product-type.application";
		};
		T2 /* Widget */ = {
			isa = PBXNativeTarget;
			buildConfigurationList = CLT2;
			name = Widget;
			productType = "com.apple.product-type.app-extension";
		};
		CLP = {
			isa = XCConfigurationList;
			buildConfigurations = (
			);
		};
		CLT1 = {
			isa = XCConfigurationList;
			buildConfigurations = (
			);
		};
		CLT2 = {
			isa = XCConfigurationList;
			buildConfigurations = (
			);
		};
	};
	rootObject = P1 /* Project object */;
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
    fn lists_roots_and_exceptions_per_target() {
        let mut root = parsed();
        exclude(&mut root, "App", "App/Resources/Info.plist").unwrap();
        let report = list(&root).unwrap();
        let app = report.iter().find(|t| t.target == "App").unwrap();
        assert_eq!(app.roots.len(), 1);
        assert_eq!(app.roots[0].dir, "App");
        assert_eq!(app.roots[0].exceptions, vec!["Resources/Info.plist"]);
        let widget = report.iter().find(|t| t.target == "Widget").unwrap();
        assert!(widget.roots.is_empty());
    }

    #[test]
    fn exclude_creates_a_sorted_exception_set_and_is_idempotent() {
        let mut root = parsed();
        let outcome = exclude(&mut root, "App", "App/Resources/Info.plist").unwrap();
        assert_eq!(
            outcome,
            ExcludeOutcome::Added {
                root_dir: "App".into(),
                exception: "Resources/Info.plist".into()
            }
        );
        // A second file sorts before the first alphabetically.
        exclude(&mut root, "App", "App/Extra.plist").unwrap();
        let report = list(&root).unwrap();
        let app = report.iter().find(|t| t.target == "App").unwrap();
        assert_eq!(
            app.roots[0].exceptions,
            vec!["Extra.plist", "Resources/Info.plist"]
        );
        // Idempotent.
        let again = exclude(&mut root, "App", "App/Extra.plist").unwrap();
        assert_eq!(
            again,
            ExcludeOutcome::AlreadyExcluded {
                root_dir: "App".into(),
                exception: "Extra.plist".into()
            }
        );
        let text = round_trips(&root);
        assert!(text.contains("PBXFileSystemSynchronizedBuildFileExceptionSet"));
        assert!(text.contains("membershipExceptions"));
    }

    #[test]
    fn exclude_outside_every_root_errors_with_the_folders() {
        let mut root = parsed();
        let err = exclude(&mut root, "App", "Elsewhere/Info.plist").unwrap_err();
        assert!(
            err.contains("Elsewhere/Info.plist") && err.contains("App"),
            "{err}"
        );
        let err = exclude(&mut root, "Widget", "App/Info.plist").unwrap_err();
        assert!(err.contains("no synchronized folders"), "{err}");
    }

    #[test]
    fn include_removes_the_exception_and_empty_set() {
        let mut root = parsed();
        exclude(&mut root, "App", "App/Resources/Info.plist").unwrap();
        let outcome = include(&mut root, "App", "App/Resources/Info.plist").unwrap();
        assert_eq!(
            outcome,
            IncludeOutcome::Removed {
                root_dir: "App".into(),
                exception: "Resources/Info.plist".into()
            }
        );
        // Set object and the root's `exceptions` key are both gone.
        let text = round_trips(&root);
        assert!(!text.contains("ExceptionSet"));
        assert!(!text.contains("exceptions ="));
        // Removing again is a no-op.
        assert_eq!(
            include(&mut root, "App", "App/Resources/Info.plist").unwrap(),
            IncludeOutcome::NotExcluded
        );
    }

    #[test]
    fn ensure_infoplist_exception_skips_paths_outside_roots() {
        let mut root = parsed();
        let outcome = ensure_infoplist_exception(&mut root, "App", "Config/Info.plist").unwrap();
        assert_eq!(outcome, None);
        let outcome = ensure_infoplist_exception(&mut root, "App", "App/Info.plist").unwrap();
        assert_eq!(
            outcome,
            Some(ExcludeOutcome::Added {
                root_dir: "App".into(),
                exception: "Info.plist".into()
            })
        );
    }

    #[test]
    fn add_root_creates_attaches_and_reuses() {
        let mut root = parsed();
        let outcome = add_root(&mut root, "Widget", "WidgetSources").unwrap();
        let AddOutcome::Created(guid) = outcome else {
            panic!("expected Created, got {outcome:?}");
        };
        // In the tree before Products, attached to the target, single-line.
        let text = round_trips(&root);
        assert!(text.contains("WidgetSources"));
        let report = list(&root).unwrap();
        let widget = report.iter().find(|t| t.target == "Widget").unwrap();
        assert_eq!(widget.roots[0].dir, "WidgetSources");

        // The same folder attaches to another target by reusing the object.
        assert_eq!(
            add_root(&mut root, "App", "WidgetSources").unwrap(),
            AddOutcome::AttachedExisting(guid.clone())
        );
        // Attaching twice is a no-op.
        assert_eq!(
            add_root(&mut root, "App", "WidgetSources").unwrap(),
            AddOutcome::AlreadyAttached(guid)
        );
    }

    #[test]
    fn new_root_lands_before_products_in_the_tree() {
        let mut root = parsed();
        add_root(&mut root, "Widget", "WidgetSources").unwrap();
        let objects = objects(&root).unwrap();
        let children: Vec<&str> = objects
            .get("MG")
            .and_then(|g| g.get("children"))
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(children.last().copied(), Some("PG"), "Products stays last");
        assert_eq!(children.len(), 3);
    }

    #[test]
    fn remove_root_detaches_and_deletes_when_orphaned() {
        let mut root = parsed();
        add_root(&mut root, "Widget", "App").unwrap();
        exclude(&mut root, "Widget", "App/Widget.swift").unwrap();

        // Detaching from one target keeps the shared object alive.
        let outcome = remove_root(&mut root, "Widget", "App").unwrap();
        assert_eq!(
            outcome,
            RemoveOutcome::Detached {
                guid: "SR1".into(),
                deleted_object: false
            }
        );
        let text = round_trips(&root);
        assert!(!text.contains("ExceptionSet"), "widget's set is gone");
        assert!(text.contains("PBXFileSystemSynchronizedRootGroup"));

        // Detaching from the last target removes the object and tree entry.
        let outcome = remove_root(&mut root, "App", "App").unwrap();
        assert_eq!(
            outcome,
            RemoveOutcome::Detached {
                guid: "SR1".into(),
                deleted_object: true
            }
        );
        let text = round_trips(&root);
        assert!(!text.contains("PBXFileSystemSynchronizedRootGroup"));
        // Not attached → no-op.
        assert_eq!(
            remove_root(&mut root, "App", "App").unwrap(),
            RemoveOutcome::NotAttached
        );
    }

    #[test]
    fn normalizes_dot_prefixes_and_trailing_slashes() {
        let mut root = parsed();
        assert_eq!(
            add_root(&mut root, "App", "./App/").unwrap(),
            AddOutcome::AlreadyAttached("SR1".into())
        );
        let outcome = exclude(&mut root, "App", "./App/Info.plist").unwrap();
        assert_eq!(
            outcome,
            ExcludeOutcome::Added {
                root_dir: "App".into(),
                exception: "Info.plist".into()
            }
        );
    }
}
