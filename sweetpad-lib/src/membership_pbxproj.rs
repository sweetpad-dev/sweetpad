//! Reading, adding, and removing classic per-file target membership — the
//! `PBXBuildFile` entries in a target's build phases — in a parsed
//! [`crate::pbxproj::Value`] tree.
//!
//! This is the pre-synchronized-folders representation: every file a target
//! builds has an explicit build-file entry (carrying per-file compiler flags,
//! header attributes, and platform filters) in one of its phases.
//! `sweetpad pbxproj membership list/add/remove` drive this module
//! (CLI_DESIGN §9g); together with [`crate::sync_pbxproj`] they make classic →
//! folder conversion an explicit, caller-owned recipe rather than a converter.
//!
//! Membership is only the build-phase axis. The `PBXFileReference` a build
//! file points at, and the group that shows it, belong to
//! [`crate::tree_pbxproj`] — so [`add_membership`] requires the reference to
//! exist already instead of inventing one with a guessed type and location.
//!
//! Everything here is pure (no I/O): callers parse the file, mutate the tree,
//! and serialize/write it — the same contract as the sibling `*_pbxproj`
//! modules. Removal cleans up after itself: a file reference no build file
//! uses anymore is deleted, and ancestor groups emptied by that deletion are
//! pruned (the orphan contract [`crate::sync_pbxproj::remove_root`] set).

use std::path::Path;

use crate::pbxproj::{Dict, Value};

/// The build phase a classic entry belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Sources,
    Resources,
    Headers,
    Frameworks,
    /// A `PBXCopyFilesBuildPhase`, with its display name (e.g.
    /// `Embed XPC Services`).
    Copy(String),
}

impl Phase {
    /// The stable machine name (`sources`, `resources`, `headers`,
    /// `frameworks`, `copy`).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Phase::Sources => "sources",
            Phase::Resources => "resources",
            Phase::Headers => "headers",
            Phase::Frameworks => "frameworks",
            Phase::Copy(_) => "copy",
        }
    }

    /// The phase a `--phase` flag names. Copy phases are absent on purpose: a
    /// target can carry several and they are told apart by name, so a kind
    /// alone does not address one.
    #[must_use]
    pub fn parse(kind: &str) -> Option<Phase> {
        match kind {
            "sources" => Some(Phase::Sources),
            "resources" => Some(Phase::Resources),
            "headers" => Some(Phase::Headers),
            "frameworks" => Some(Phase::Frameworks),
            _ => None,
        }
    }

    /// Human rendering: the kind, plus the copy phase's name.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Phase::Copy(name) => format!("copy ({name})"),
            other => other.kind().to_string(),
        }
    }

    fn of(isa: &str, phase: &Value) -> Option<Phase> {
        match isa {
            "PBXSourcesBuildPhase" => Some(Phase::Sources),
            "PBXResourcesBuildPhase" => Some(Phase::Resources),
            "PBXHeadersBuildPhase" => Some(Phase::Headers),
            "PBXFrameworksBuildPhase" => Some(Phase::Frameworks),
            "PBXCopyFilesBuildPhase" => {
                let name = phase
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Copy Files");
                Some(Phase::Copy(name.to_string()))
            }
            // Script phases have no file membership to speak of.
            _ => None,
        }
    }
}

/// What kind of node the build file references — files convert to folders;
/// variant/version groups are the constructs a script leaves classic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    File,
    VariantGroup,
    VersionGroup,
    Other,
}

impl RefKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RefKind::File => "file",
            RefKind::VariantGroup => "variantGroup",
            RefKind::VersionGroup => "versionGroup",
            RefKind::Other => "other",
        }
    }

    fn of(isa: &str) -> RefKind {
        match isa {
            "PBXFileReference" => RefKind::File,
            "PBXVariantGroup" => RefKind::VariantGroup,
            "XCVersionGroup" => RefKind::VersionGroup,
            _ => RefKind::Other,
        }
    }
}

/// One classic membership entry: a file (or variant/version group) a target
/// builds, with the per-file details its `PBXBuildFile` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Project-dir-relative resolved path (group-tree walk).
    pub path: String,
    pub phase: Phase,
    pub kind: RefKind,
    /// `settings.COMPILER_FLAGS` — per-file compiler flags.
    pub compiler_flags: Option<String>,
    /// `settings.ATTRIBUTES` — e.g. `Public`/`Private` header visibility,
    /// `RemoveHeadersOnCopy`, `CodeSignOnCopy`.
    pub attributes: Vec<String>,
    /// `platformFilters` (or the older singular `platformFilter`).
    pub platform_filters: Vec<String>,
}

/// The outcome of removing one path's membership from one target. Empty
/// `removed_phases` records the no-op (the path wasn't a member), so re-run
/// scripts stay green.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub path: String,
    /// Display names of the phases entries were removed from.
    pub removed_phases: Vec<String>,
    /// Set when no build file (of any target) references the file anymore,
    /// so the reference itself was deleted from the project.
    pub deleted_reference: bool,
    /// Ancestor groups deleted because the reference removal emptied them.
    pub pruned_groups: usize,
}

/// A target's classic membership entries, in build-phase order.
///
/// # Errors
/// Returns a message when the tree is malformed or the target is missing.
pub fn classic_members(root: &Value, target: &str) -> Result<Vec<FileEntry>, String> {
    let objects = objects(root).ok_or("pbxproj has no objects dict")?;
    let target_guid = find_target_guid(objects, target)?;
    let mut entries = Vec::new();
    for (phase, build_file_guids) in phases_of(objects, &target_guid) {
        for bf_guid in build_file_guids {
            let Some(build_file) = objects.get(&bf_guid) else {
                continue;
            };
            // Package-product links (`productRef`) belong to `dependency`.
            let Some(file_ref) = str_field(build_file, "fileRef") else {
                continue;
            };
            let kind = RefKind::of(objects.get(file_ref).map_or("", isa));
            entries.push(FileEntry {
                path: node_path(objects, file_ref),
                phase: phase.clone(),
                kind,
                compiler_flags: build_file
                    .get("settings")
                    .and_then(|s| s.get("COMPILER_FLAGS"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                attributes: build_file
                    .get("settings")
                    .and_then(|s| s.get("ATTRIBUTES"))
                    .and_then(Value::as_array)
                    .map(str_items)
                    .unwrap_or_default(),
                platform_filters: build_file
                    .get("platformFilters")
                    .and_then(Value::as_array)
                    .map(str_items)
                    .or_else(|| {
                        str_field(build_file, "platformFilter").map(|f| vec![f.to_string()])
                    })
                    .unwrap_or_default(),
            });
        }
    }
    Ok(entries)
}

/// Remove `target`'s classic membership of each path (project-dir-relative):
/// the build-file entries leave the target's phases; a reference no build
/// file uses anymore is deleted (variant/version groups take their child
/// references with them) and emptied ancestor groups are pruned. A path that
/// isn't a member is a recorded no-op.
///
/// # Errors
/// Returns a message when the tree is malformed or the target is missing.
pub fn remove_membership(
    root: &mut Value,
    target: &str,
    paths: &[String],
) -> Result<Vec<Removal>, String> {
    let (main_group, products_group) = group_guards(root);
    let objects = objects_mut(root)?;
    let target_guid = find_target_guid(objects, target)?;
    let mut removals = Vec::new();
    for raw_path in paths {
        let path = normalize(raw_path);
        let ref_guids: Vec<String> = objects
            .iter()
            .filter(|(_, o)| {
                matches!(
                    isa(o),
                    "PBXFileReference" | "PBXVariantGroup" | "XCVersionGroup"
                )
            })
            .filter(|(guid, _)| node_path(objects, guid) == path)
            .map(|(guid, _)| guid.clone())
            .collect();

        let mut removed_phases = Vec::new();
        for (phase, build_file_guids) in phases_of(objects, &target_guid) {
            let mut hit = false;
            for bf_guid in build_file_guids {
                let references_path = objects
                    .get(&bf_guid)
                    .and_then(|bf| str_field(bf, "fileRef"))
                    .is_some_and(|fr| ref_guids.iter().any(|g| g == fr));
                if !references_path {
                    continue;
                }
                remove_guid_from_phase_files(objects, &target_guid, &bf_guid);
                objects.remove(&bf_guid);
                hit = true;
            }
            if hit {
                removed_phases.push(phase.display());
            }
        }

        // Orphan cleanup: a reference nothing builds anymore leaves the tree.
        let mut deleted_reference = false;
        let mut pruned_groups = 0usize;
        for ref_guid in &ref_guids {
            let still_built = objects.iter().any(|(_, o)| {
                isa(o) == "PBXBuildFile" && str_field(o, "fileRef") == Some(ref_guid.as_str())
            });
            if still_built {
                continue;
            }
            let parent = crate::project::parent_group_of(objects, ref_guid);
            delete_node_recursive(objects, ref_guid);
            if let Some(parent) = parent {
                remove_child(objects, &parent, ref_guid);
                pruned_groups += prune_empty_groups(
                    objects,
                    &parent,
                    main_group.as_ref(),
                    products_group.as_ref(),
                );
            }
            deleted_reference = true;
        }

        removals.push(Removal {
            path,
            removed_phases,
            deleted_reference,
            pruned_groups,
        });
    }
    Ok(removals)
}

/// The outcome of adding one path to one target's phase. `already_member`
/// records the no-op, so re-run scripts stay green.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addition {
    pub path: String,
    /// Display name of the phase the entry joined.
    pub phase: String,
    /// The `PBXBuildFile` created, or the existing one on a no-op.
    pub build_file: String,
    pub already_member: bool,
}

/// Give `target` a classic build-file entry for each path, in `phase`.
///
/// Every path must already have a `PBXFileReference`: creating one is
/// [`crate::tree_pbxproj::add_fileref`]'s job, kept separate so the file's
/// type, anchor, and group stay the caller's answers instead of this
/// function's guesses. `phase` is named outright and never derived from the
/// extension — putting a `.ttf` in Sources is a decision the caller gets to
/// make wrongly rather than one this code makes silently.
///
/// Paths resolve exactly as [`remove_membership`]'s do. The whole batch is
/// resolved before anything is written, so a bad path refuses the batch rather
/// than half-applying it.
///
/// # Errors
/// Returns a message when the tree is malformed, the target or phase is
/// missing, or a path has no reference (or more than one).
pub fn add_membership(
    root: &mut Value,
    target: &str,
    paths: &[String],
    phase: &Phase,
) -> Result<Vec<Addition>, String> {
    let objects_ref = objects(root).ok_or("pbxproj has no objects dict")?;
    let target_guid = find_target_guid(objects_ref, target)?;
    let phase_guid = phase_guid_of(objects_ref, &target_guid, phase).ok_or_else(|| {
        format!(
            "target {target} has no {} build phase to add to",
            phase.display()
        )
    })?;

    let mut resolved = Vec::new();
    for raw_path in paths {
        let path = normalize(raw_path);
        let hits: Vec<String> = objects_ref
            .iter()
            .filter(|(_, o)| {
                matches!(
                    isa(o),
                    "PBXFileReference" | "PBXVariantGroup" | "XCVersionGroup"
                )
            })
            .filter(|(guid, _)| node_path(objects_ref, guid) == path)
            .map(|(guid, _)| guid.clone())
            .collect();
        match hits.len() {
            0 => {
                return Err(format!(
                    "no file reference for {path}: create one with \
                     `pbxproj fileref add {path} --group <ID>` first"
                ));
            }
            1 => resolved.push((path, hits[0].clone())),
            n => {
                return Err(format!(
                    "{path} matches {n} file references ({}); pass the one you mean to \
                     `pbxproj membership remove` first",
                    hits.join(", ")
                ));
            }
        }
    }

    let mut additions = Vec::new();
    for (path, ref_guid) in resolved {
        let objects = objects_mut(root)?;
        if let Some(existing) = build_file_in_phase(objects, &phase_guid, &ref_guid) {
            additions.push(Addition {
                path,
                phase: phase.display(),
                build_file: existing,
                already_member: true,
            });
            continue;
        }
        let bf_guid = crate::spm_pbxproj::fresh_guid(
            objects,
            &format!("buildfile#{ref_guid}#{phase_guid}"),
            0,
        );
        let mut build_file = Dict::new();
        build_file.insert("isa".into(), Value::String("PBXBuildFile".into()));
        build_file.insert("fileRef".into(), Value::String(ref_guid.clone()));
        build_file.set_single_line(true);
        objects.insert(bf_guid.clone(), Value::Dict(build_file));

        if let Some(files) = objects
            .get_mut(&phase_guid)
            .and_then(|p| p.get_mut("files"))
            .and_then(Value::as_array_mut)
        {
            files.push(Value::String(bf_guid.clone()));
        } else if let Some(node) = objects.get_mut(&phase_guid).and_then(Value::as_dict_mut) {
            node.insert(
                "files".into(),
                Value::Array(vec![Value::String(bf_guid.clone())]),
            );
        }
        additions.push(Addition {
            path,
            phase: phase.display(),
            build_file: bf_guid,
            already_member: false,
        });
    }
    Ok(additions)
}

/// The GUID of `target`'s phase of this kind, when it has one. A copy phase
/// matches on its name too, since a target can carry several.
fn phase_guid_of(objects: &Dict, target_guid: &str, want: &Phase) -> Option<String> {
    let phase_guids: Vec<String> = objects
        .get(target_guid)
        .and_then(|t| t.get("buildPhases"))
        .and_then(Value::as_array)
        .map(str_items)
        .unwrap_or_default();
    phase_guids.into_iter().find(|guid| {
        objects
            .get(guid)
            .and_then(|obj| Phase::of(isa(obj), obj))
            .is_some_and(|found| &found == want)
    })
}

/// The build file in `phase` that already points at `ref_guid`.
fn build_file_in_phase(objects: &Dict, phase_guid: &str, ref_guid: &str) -> Option<String> {
    let files = objects
        .get(phase_guid)
        .and_then(|p| p.get("files"))
        .and_then(Value::as_array)
        .map(str_items)
        .unwrap_or_default();
    files.into_iter().find(|bf| {
        objects
            .get(bf)
            .and_then(|obj| str_field(obj, "fileRef"))
            .is_some_and(|fr| fr == ref_guid)
    })
}

/// `(phase, build-file GUIDs)` for each membership-bearing phase of a target,
/// in `buildPhases` order.
fn phases_of(objects: &Dict, target_guid: &str) -> Vec<(Phase, Vec<String>)> {
    let Some(phase_guids) = objects
        .get(target_guid)
        .and_then(|t| t.get("buildPhases"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for guid in phase_guids {
        let Some(guid) = guid.as_str() else { continue };
        let Some(phase_obj) = objects.get(guid) else {
            continue;
        };
        let Some(phase) = Phase::of(isa(phase_obj), phase_obj) else {
            continue;
        };
        let files = phase_obj
            .get("files")
            .and_then(Value::as_array)
            .map(str_items)
            .unwrap_or_default();
        out.push((phase, files));
    }
    out
}

/// Drop a build-file GUID from whichever of the target's phase `files`
/// arrays lists it.
fn remove_guid_from_phase_files(objects: &mut Dict, target_guid: &str, bf_guid: &str) {
    let phase_guids: Vec<String> = objects
        .get(target_guid)
        .and_then(|t| t.get("buildPhases"))
        .and_then(Value::as_array)
        .map(str_items)
        .unwrap_or_default();
    for phase_guid in phase_guids {
        if let Some(files) = objects
            .get_mut(&phase_guid)
            .and_then(Value::as_dict_mut)
            .and_then(|p| p.get_mut("files"))
            .and_then(Value::as_array_mut)
        {
            files.retain(|v| v.as_str() != Some(bf_guid));
        }
    }
}

/// Delete a node and (for variant/version groups) its child references —
/// children of those groups exist only for the group.
fn delete_node_recursive(objects: &mut Dict, guid: &str) {
    let children: Vec<String> = objects
        .get(guid)
        .filter(|o| matches!(isa(o), "PBXVariantGroup" | "XCVersionGroup"))
        .and_then(|o| o.get("children"))
        .and_then(Value::as_array)
        .map(str_items)
        .unwrap_or_default();
    for child in children {
        delete_node_recursive(objects, &child);
    }
    objects.remove(guid);
}

fn remove_child(objects: &mut Dict, group: &str, child: &str) {
    if let Some(children) = objects
        .get_mut(group)
        .and_then(Value::as_dict_mut)
        .and_then(|g| g.get_mut("children"))
        .and_then(Value::as_array_mut)
    {
        children.retain(|v| v.as_str() != Some(child));
    }
}

/// Walk up from `group`, deleting each group its child-removal emptied.
/// The main group and Products group survive even when empty — they're
/// structural. Returns how many groups were pruned.
fn prune_empty_groups(
    objects: &mut Dict,
    group: &str,
    main_group: Option<&String>,
    products_group: Option<&String>,
) -> usize {
    let mut pruned = 0;
    let mut current = group.to_string();
    loop {
        if Some(&current) == main_group || Some(&current) == products_group {
            break;
        }
        let empty = objects
            .get(&current)
            .filter(|o| matches!(isa(o), "PBXGroup" | "PBXVariantGroup" | "XCVersionGroup"))
            .is_some_and(|o| {
                o.get("children")
                    .and_then(Value::as_array)
                    .is_none_or(<[Value]>::is_empty)
            });
        if !empty {
            break;
        }
        let parent = crate::project::parent_group_of(objects, &current);
        objects.remove(&current);
        pruned += 1;
        match parent {
            Some(parent) => {
                remove_child(objects, &parent, &current);
                current = parent;
            }
            None => break,
        }
    }
    pruned
}

/// The project-dir-relative path of a group-tree node (its own `path` plus
/// every pathed ancestor), via the shared group walk.
fn node_path(objects: &Dict, guid: &str) -> String {
    crate::project::group_dir(objects, guid, Path::new(""), 0)
        .to_string_lossy()
        .into_owned()
}

fn group_guards(root: &Value) -> (Option<String>, Option<String>) {
    let project = root
        .as_dict()
        .and_then(|d| d.get("rootObject"))
        .and_then(Value::as_str)
        .and_then(|g| objects(root).and_then(|o| o.get(g)));
    let main_group = project
        .and_then(|p| p.get("mainGroup"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let products_group = project
        .and_then(|p| p.get("productRefGroup"))
        .and_then(Value::as_str)
        .map(str::to_string);
    (main_group, products_group)
}

fn str_items(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A classic two-target project: App compiles two sources (one with
    /// per-file flags) under nested groups and copies a resource; Tests
    /// borrows one source. A copy phase with a name embeds an XPC service.
    const FIXTURE: &str = r#"// !$*UTF8*$!
{
	archiveVersion = 1;
	classes = {
	};
	objectVersion = 56;
	objects = {
		BF1 /* Main.swift in Sources */ = {isa = PBXBuildFile; fileRef = FR1 /* Main.swift */; };
		BF2 /* Legacy.swift in Sources */ = {isa = PBXBuildFile; fileRef = FR2 /* Legacy.swift */; settings = {COMPILER_FLAGS = "-w"; }; };
		BF3 /* Logo.png in Resources */ = {isa = PBXBuildFile; fileRef = FR3 /* Logo.png */; };
		BF4 /* Main.swift in Sources */ = {isa = PBXBuildFile; fileRef = FR1 /* Main.swift */; };
		BF5 /* Helper.xpc in Embed XPC Services */ = {isa = PBXBuildFile; fileRef = FR4 /* Helper.xpc */; platformFilters = (macos, ); settings = {ATTRIBUTES = (RemoveHeadersOnCopy, ); }; };
		FR1 /* Main.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = Main.swift; sourceTree = "<group>"; };
		FR2 /* Legacy.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = Legacy.swift; sourceTree = "<group>"; };
		FR3 /* Logo.png */ = {isa = PBXFileReference; lastKnownFileType = image.png; path = Logo.png; sourceTree = "<group>"; };
		FR4 /* Helper.xpc */ = {isa = PBXFileReference; lastKnownFileType = "wrapper.xpc-service"; path = Helper.xpc; sourceTree = "<group>"; };
		MG = {
			isa = PBXGroup;
			children = (
				G1 /* App */,
				FR4 /* Helper.xpc */,
				PG /* Products */,
			);
			sourceTree = "<group>";
		};
		G1 /* App */ = {
			isa = PBXGroup;
			children = (
				FR1 /* Main.swift */,
				G2 /* Legacy */,
				FR3 /* Logo.png */,
			);
			path = App;
			sourceTree = "<group>";
		};
		G2 /* Legacy */ = {
			isa = PBXGroup;
			children = (
				FR2 /* Legacy.swift */,
			);
			path = Legacy;
			sourceTree = "<group>";
		};
		PG /* Products */ = {
			isa = PBXGroup;
			children = (
			);
			name = Products;
			sourceTree = "<group>";
		};
		T1 /* App */ = {
			isa = PBXNativeTarget;
			buildConfigurationList = CLT1;
			buildPhases = (
				SP1 /* Sources */,
				RP1 /* Resources */,
				CP1 /* Embed XPC Services */,
			);
			buildRules = (
			);
			dependencies = (
			);
			name = App;
			productType = "com.apple.product-type.application";
		};
		T2 /* Tests */ = {
			isa = PBXNativeTarget;
			buildConfigurationList = CLT2;
			buildPhases = (
				SP2 /* Sources */,
			);
			buildRules = (
			);
			dependencies = (
			);
			name = Tests;
			productType = "com.apple.product-type.bundle.unit-test";
		};
		SP1 /* Sources */ = {
			isa = PBXSourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (
				BF1 /* Main.swift in Sources */,
				BF2 /* Legacy.swift in Sources */,
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
		RP1 /* Resources */ = {
			isa = PBXResourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (
				BF3 /* Logo.png in Resources */,
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
		CP1 /* Embed XPC Services */ = {
			isa = PBXCopyFilesBuildPhase;
			buildActionMask = 2147483647;
			dstPath = "$(CONTENTS_FOLDER_PATH)/XPCServices";
			dstSubfolderSpec = 16;
			files = (
				BF5 /* Helper.xpc in Embed XPC Services */,
			);
			name = "Embed XPC Services";
			runOnlyForDeploymentPostprocessing = 0;
		};
		SP2 /* Sources */ = {
			isa = PBXSourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (
				BF4 /* Main.swift in Sources */,
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
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
    fn lists_members_with_phases_and_per_file_details() {
        let root = parsed();
        let app = classic_members(&root, "App").unwrap();
        let paths: Vec<(&str, String)> = app
            .iter()
            .map(|e| (e.path.as_str(), e.phase.display()))
            .collect();
        assert_eq!(
            paths,
            vec![
                ("App/Main.swift", "sources".to_string()),
                ("App/Legacy/Legacy.swift", "sources".to_string()),
                ("App/Logo.png", "resources".to_string()),
                ("Helper.xpc", "copy (Embed XPC Services)".to_string()),
            ]
        );
        let legacy = &app[1];
        assert_eq!(legacy.compiler_flags.as_deref(), Some("-w"));
        let xpc = &app[3];
        assert_eq!(xpc.attributes, vec!["RemoveHeadersOnCopy"]);
        assert_eq!(xpc.platform_filters, vec!["macos"]);
        assert_eq!(xpc.kind, RefKind::File);

        let tests = classic_members(&root, "Tests").unwrap();
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].path, "App/Main.swift");
    }

    #[test]
    fn add_puts_the_file_in_the_phase_the_caller_named() {
        let mut root = parsed();
        // A .png into Sources: wrong, and allowed. The extension does not get
        // a vote — that is the whole point of naming the phase.
        let added =
            add_membership(&mut root, "App", &["App/Logo.png".into()], &Phase::Sources).unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].phase, "sources");
        assert!(!added[0].already_member);

        let text = round_trips(&root);
        let bf = &added[0].build_file;
        assert!(text.contains(bf), "the build file exists");
        // It joined the sources phase, and the resources entry it already had
        // is untouched.
        let members = classic_members(&root, "App").unwrap();
        let in_sources = members
            .iter()
            .filter(|e| e.path == "App/Logo.png" && e.phase == Phase::Sources)
            .count();
        let in_resources = members
            .iter()
            .filter(|e| e.path == "App/Logo.png" && e.phase == Phase::Resources)
            .count();
        assert_eq!((in_sources, in_resources), (1, 1));
    }

    #[test]
    fn adding_an_existing_member_is_a_no_op() {
        let mut root = parsed();
        let before = crate::pbxproj_writer::serialize(&root, "Fix");
        let added = add_membership(
            &mut root,
            "App",
            &["App/Main.swift".into()],
            &Phase::Sources,
        )
        .unwrap();
        assert!(added[0].already_member);
        assert_eq!(
            added[0].build_file, "BF1",
            "it reports the entry that exists"
        );
        let after = crate::pbxproj_writer::serialize(&root, "Fix");
        assert_eq!(before, after, "no-ops must not touch the file");
    }

    #[test]
    fn adding_a_path_with_no_reference_points_at_fileref_add() {
        let mut root = parsed();
        let before = crate::pbxproj_writer::serialize(&root, "Fix");
        let err = add_membership(
            &mut root,
            "App",
            &["App/Main.swift".into(), "App/Nope.swift".into()],
            &Phase::Sources,
        )
        .unwrap_err();
        assert!(
            err.contains("no file reference for App/Nope.swift"),
            "{err}"
        );
        assert!(err.contains("fileref add"), "{err}");
        let after = crate::pbxproj_writer::serialize(&root, "Fix");
        assert_eq!(
            before, after,
            "a bad path refuses the whole batch, half-applying nothing"
        );
    }

    #[test]
    fn adding_to_a_phase_the_target_lacks_errors() {
        let mut root = parsed();
        let err = add_membership(
            &mut root,
            "App",
            &["App/Main.swift".into()],
            &Phase::Headers,
        )
        .unwrap_err();
        assert!(err.contains("no headers build phase"), "{err}");
    }

    #[test]
    fn the_generated_build_file_id_is_deterministic() {
        let mut a = parsed();
        let mut b = parsed();
        let ra = add_membership(&mut a, "App", &["App/Logo.png".into()], &Phase::Sources).unwrap();
        let rb = add_membership(&mut b, "App", &["App/Logo.png".into()], &Phase::Sources).unwrap();
        assert_eq!(ra[0].build_file, rb[0].build_file);
        assert_eq!(
            crate::pbxproj_writer::serialize(&a, "Fix"),
            crate::pbxproj_writer::serialize(&b, "Fix"),
            "the same mutation twice produces the same bytes"
        );
    }

    #[test]
    fn unknown_target_errors_with_known_names() {
        let root = parsed();
        let err = classic_members(&root, "Nope").unwrap_err();
        assert!(err.contains("Nope") && err.contains("App, Tests"), "{err}");
    }

    #[test]
    fn removing_a_shared_file_keeps_the_reference_for_the_other_target() {
        let mut root = parsed();
        let removals = remove_membership(&mut root, "Tests", &["App/Main.swift".into()]).unwrap();
        assert_eq!(removals.len(), 1);
        assert_eq!(removals[0].removed_phases, vec!["sources"]);
        assert!(!removals[0].deleted_reference, "App still builds it");

        let text = round_trips(&root);
        assert!(text.contains("FR1"), "the reference survives");
        assert!(!text.contains("BF4"), "the Tests build file is gone");
        assert_eq!(classic_members(&root, "Tests").unwrap().len(), 0);
        assert_eq!(classic_members(&root, "App").unwrap().len(), 4);
    }

    #[test]
    fn removing_the_last_membership_deletes_the_reference_and_prunes_groups() {
        let mut root = parsed();
        let removals =
            remove_membership(&mut root, "App", &["App/Legacy/Legacy.swift".into()]).unwrap();
        assert_eq!(removals[0].removed_phases, vec!["sources"]);
        assert!(removals[0].deleted_reference);
        assert_eq!(
            removals[0].pruned_groups, 1,
            "the emptied Legacy group goes"
        );

        let text = round_trips(&root);
        assert!(!text.contains("FR2"));
        assert!(!text.contains("G2"), "the Legacy group is pruned");
        assert!(text.contains("G1"), "the App group still has children");
    }

    #[test]
    fn batched_removal_dismantles_a_target_in_one_pass() {
        let mut root = parsed();
        // The explicit-conversion recipe: remove everything App builds.
        let paths: Vec<String> = classic_members(&root, "App")
            .unwrap()
            .into_iter()
            .map(|e| e.path)
            .collect();
        let removals = remove_membership(&mut root, "App", &paths).unwrap();
        assert!(removals.iter().all(|r| !r.removed_phases.is_empty()));
        assert_eq!(classic_members(&root, "App").unwrap().len(), 0);

        // Main.swift's reference survives (Tests still builds it); the rest
        // are gone.
        let text = round_trips(&root);
        assert!(text.contains("FR1"));
        assert!(!text.contains("FR3"));
        assert!(!text.contains("FR4"));
        // Phase objects themselves stay (empty) — that's the target's shape.
        assert!(text.contains("PBXCopyFilesBuildPhase"));
    }

    #[test]
    fn non_member_paths_are_recorded_noops() {
        let mut root = parsed();
        let before = crate::pbxproj_writer::serialize(&root, "Fix");
        let removals = remove_membership(
            &mut root,
            "Tests",
            &["App/Logo.png".into(), "Nope.swift".into()],
        )
        .unwrap();
        assert!(removals.iter().all(|r| r.removed_phases.is_empty()));
        assert!(removals.iter().all(|r| !r.deleted_reference));
        let after = crate::pbxproj_writer::serialize(&root, "Fix");
        assert_eq!(before, after, "no-ops must not touch the file");
    }

    #[test]
    fn normalizes_input_paths() {
        let mut root = parsed();
        let removals = remove_membership(&mut root, "App", &["./App/Logo.png".into()]).unwrap();
        assert_eq!(removals[0].removed_phases, vec!["resources"]);
        assert_eq!(removals[0].path, "App/Logo.png");
    }
}
