//! Shared plumbing for commands that mutate a `project.pbxproj` — parse,
//! atomic write, and the strict workspace→member-project mapping the §9f
//! mutation commands (`settings set/unset`, `source …`) share.
//!
//! Unlike the resolution flow (which may pick interactively), mutations never
//! guess: an ambiguous workspace hard-errors — on a TTY or off — naming the
//! flag that disambiguates. `dependency`, which predates that rule, keeps its
//! own interactive picker.

use std::path::{Path, PathBuf};

use sweetpad_lib::pbxproj::Value;

use crate::cli::resolve::Container;
use crate::cli::{CliError, CliResult, Context};

/// Parse a `.xcodeproj`'s `project.pbxproj` into an owned tree for mutation.
pub fn parse_owned(xcodeproj: &Path) -> Result<Value, CliError> {
    let path = xcodeproj.join("project.pbxproj");
    sweetpad_lib::pbxproj::parse_file(&path)
        .map_err(|e| CliError::new(format!("failed to parse {}: {e}", path.display())))
}

/// Write `text` to `path` atomically (same-directory temp + rename), so a
/// crash, signal, or full disk mid-write can't leave a truncated project
/// file behind.
pub fn write_atomic(path: &Path, text: &str) -> CliResult {
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".tmp.{}", std::process::id()));
    let tmp = path.with_file_name(tmp_name);
    if let Err(e) = std::fs::write(&tmp, text) {
        let _ = std::fs::remove_file(&tmp);
        return Err(CliError::new(format!(
            "failed to write {}: {e}",
            path.display()
        )));
    }
    std::fs::rename(&tmp, path)
        .map_err(|e| CliError::new(format!("failed to write {}: {e}", path.display())))
}

/// Serialize a mutated tree back into its `.xcodeproj`, atomically.
pub fn write_pbxproj(xcodeproj: &Path, root: &Value) -> CliResult {
    let name = xcodeproj
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Project");
    let text = sweetpad_lib::pbxproj_writer::serialize(root, name);
    write_atomic(&xcodeproj.join("project.pbxproj"), &text)
}

/// The `.xcodeproj` a mutation edits. A project container is itself; a
/// workspace maps to the one member that owns every named target (or its sole
/// member, or an explicit `--project`) — anything ambiguous is a hard error,
/// interactive or not.
pub fn mutation_xcodeproj(
    ctx: &Context,
    container: &Container,
    targets: &[String],
) -> Result<PathBuf, CliError> {
    let workspace = match container {
        Container::Project(p) => return Ok(p.clone()),
        Container::Workspace(p) => p,
        Container::SwiftPackage(p) => {
            return Err(CliError::new(format!(
                "{} is a Swift package — it has no project.pbxproj to edit",
                p.display()
            )));
        }
    };
    if let Some(project) = &ctx.targeting.project {
        return Ok(project.clone());
    }
    let ws = sweetpad_lib::workspace::open(workspace).map_err(|e| {
        CliError::new(format!(
            "failed to read workspace {}: {e}",
            workspace.display()
        ))
    })?;
    let members = ws.project_refs;
    if members.is_empty() {
        return Err(CliError::new(
            "the workspace references no projects to edit",
        ));
    }
    if members.len() == 1 {
        return Ok(members[0].clone());
    }
    if targets.is_empty() {
        return Err(CliError::new(format!(
            "the workspace has {} member projects ({}); pass --project to say which \
             one to edit",
            members.len(),
            member_list(&members)
        )));
    }
    // Every named target must live in exactly one — and the same — member.
    let mut owner: Option<&PathBuf> = None;
    for target in targets {
        let owners: Vec<&PathBuf> = members
            .iter()
            .filter(|m| member_has_target(m, target))
            .collect();
        match owners.as_slice() {
            [] => {
                return Err(CliError::new(format!(
                    "no member project of the workspace declares a target named \
                     `{target}` (members: {})",
                    member_list(&members)
                )));
            }
            [one] => match owner {
                None => owner = Some(one),
                Some(prev) if prev == *one => {}
                Some(prev) => {
                    return Err(CliError::new(format!(
                        "the named targets live in different member projects ({} and \
                         {}); edit one project at a time",
                        prev.display(),
                        one.display()
                    )));
                }
            },
            many => {
                return Err(CliError::new(format!(
                    "target `{target}` exists in {} member projects ({}); pass \
                     --project to say which one to edit",
                    many.len(),
                    many.iter()
                        .map(|m| m.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
    }
    owner
        .cloned()
        .ok_or_else(|| CliError::new("no target named — pass --project"))
}

fn member_has_target(xcodeproj: &Path, target: &str) -> bool {
    sweetpad_lib::project::parse_pbxproj(xcodeproj)
        .ok()
        .is_some_and(|root| {
            sweetpad_lib::settings_pbxproj::target_names(&root)
                .iter()
                .any(|t| t == target)
        })
}

fn member_list(members: &[PathBuf]) -> String {
    members
        .iter()
        .map(|m| m.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
