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

use crate::cli::config::ProjectFile;
use crate::cli::resolve::Container;
use crate::cli::{CliError, CliResult, Context};

/// The project generator whose spec regenerates (and overwrites) a
/// `.xcodeproj`, as detected for the [`guard_generated`] check.
pub struct Generator {
    /// Tool name for the message, e.g. `XcodeGen`.
    pub tool: String,
    /// What the user should edit instead, e.g. `project.yml`.
    pub spec: String,
    /// The command that clobbers manual edits, e.g. `xcodegen generate`.
    pub regenerate: String,
}

/// The generator specs recognized beside a `.xcodeproj`, in detection order:
/// the file name, the tool that owns it, and the command that regenerates.
const SPECS: [(&str, &str, &str); 4] = [
    ("project.yml", "XcodeGen", "xcodegen generate"),
    ("project.yaml", "XcodeGen", "xcodegen generate"),
    ("project.json", "XcodeGen", "xcodegen generate"),
    ("Project.swift", "Tuist", "tuist generate"),
];

/// Detect a generator for `xcodeproj`: an explicit `generator = "…"` in the
/// project's `sweetpad.toml` wins, else a generator spec file sitting next to
/// the `.xcodeproj` (`project.yml`/`project.yaml`/`project.json` → XcodeGen,
/// `Project.swift` → Tuist).
pub fn generator_for(project_file: &ProjectFile, xcodeproj: &Path) -> Option<Generator> {
    if let Some(name) = project_file
        .generator
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(match name.to_ascii_lowercase().as_str() {
            "xcodegen" => Generator {
                tool: "XcodeGen".into(),
                spec: "project.yml".into(),
                regenerate: "xcodegen generate".into(),
            },
            "tuist" => Generator {
                tool: "Tuist".into(),
                spec: "Project.swift".into(),
                regenerate: "tuist generate".into(),
            },
            _ => Generator {
                tool: name.to_string(),
                spec: format!("{name}'s spec"),
                regenerate: format!("{name} run"),
            },
        });
    }
    let dir = xcodeproj.parent()?;
    for (spec, tool, regenerate) in SPECS {
        if dir.join(spec).is_file() {
            return Some(Generator {
                tool: tool.into(),
                spec: spec.into(),
                regenerate: regenerate.into(),
            });
        }
    }
    None
}

/// The generator spec beside `xcodeproj`, whichever recognized name is on
/// disk. The [`Generator::spec`] of an explicitly declared generator is the
/// canonical name (`project.yml`), which need not be the one this project
/// actually uses, so staleness resolves the file itself.
fn spec_path(xcodeproj: &Path) -> Option<PathBuf> {
    let dir = xcodeproj.parent()?;
    SPECS
        .iter()
        .map(|(spec, _, _)| dir.join(spec))
        .find(|p| p.is_file())
}

/// Warn text for a generated project whose spec has been edited since it was
/// last generated, or `None` when it is current (or not generated at all).
///
/// A file added to or removed from the spec is invisible to the build until
/// the project is regenerated, and the build then fails with an ordinary
/// `cannot find 'X' in scope` — a compile error that names a symbol when the
/// real cause is a stale project. Comparing `project.pbxproj` rather than the
/// `.xcodeproj` directory is what makes this trustworthy: Xcode writes
/// `xcuserdata` and workspace state inside the bundle constantly, and any of
/// that would otherwise read as "freshly generated".
#[must_use]
pub fn stale_generated(project_file: &ProjectFile, xcodeproj: &Path) -> Option<String> {
    let generator = generator_for(project_file, xcodeproj)?;
    let spec = spec_path(xcodeproj)?;
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let (spec_at, project_at) = (
        modified(&spec)?,
        modified(&xcodeproj.join("project.pbxproj"))?,
    );
    if spec_at <= project_at {
        return None;
    }
    let spec_name = spec.file_name().map_or_else(
        || spec.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let project_name = xcodeproj.file_name().map_or_else(
        || xcodeproj.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    Some(format!(
        "{spec_name} is newer than {project_name} — run '{}' so the build sees your latest \
         file changes",
        generator.regenerate
    ))
}

/// Refuse to mutate a generated project without `--force` (CLI_DESIGN §9g):
/// an edit to the `.xcodeproj` would be silently overwritten by the next
/// regenerate, so the default is a hard error naming the spec to edit
/// instead. `--force` says the ephemeral edit is deliberate.
pub fn guard_generated(project_file: &ProjectFile, xcodeproj: &Path, force: bool) -> CliResult {
    if force {
        return Ok(());
    }
    let Some(generator) = generator_for(project_file, xcodeproj) else {
        return Ok(());
    };
    let name = xcodeproj.file_name().map_or_else(
        || xcodeproj.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    Err(CliError::new(format!(
        "{name} is generated by {} — this edit would be silently overwritten by the next \
         `{}`. Declare the change in {} instead, or pass --force to edit the generated \
         project anyway (deliberate, ephemeral)",
        generator.tool, generator.regenerate, generator.spec
    )))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project(marker: &str, spec: Option<&str>) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sweetpad-genguard-{}-{marker}", std::process::id()));
        let xcodeproj = dir.join("App.xcodeproj");
        std::fs::create_dir_all(&xcodeproj).unwrap();
        if let Some(name) = spec {
            std::fs::write(dir.join(name), "# spec").unwrap();
        }
        xcodeproj
    }

    /// Write `project.pbxproj`, then the spec, in that order — so the spec is
    /// unambiguously the newer of the two even on a coarse-grained filesystem
    /// (mtimes only a syscall apart can otherwise compare equal).
    fn generated_project(marker: &str, spec: &str, spec_is_newer: bool) -> PathBuf {
        let xcodeproj = temp_project(marker, None);
        let dir = xcodeproj.parent().unwrap().to_path_buf();
        let (first, second) = (dir.join(spec), xcodeproj.join("project.pbxproj"));
        let (first, second) = if spec_is_newer {
            (second, first)
        } else {
            (first, second)
        };
        std::fs::write(&first, "one").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&second, "two").unwrap();
        xcodeproj
    }

    #[test]
    fn a_spec_edited_since_the_last_generate_is_reported_stale() {
        let pf = ProjectFile::default();
        let stale = generated_project("stale", "project.yml", true);
        let warning = stale_generated(&pf, &stale).expect("expected a staleness warning");
        assert!(
            warning.contains("project.yml is newer than App.xcodeproj"),
            "{warning}"
        );
        // Names the command that fixes it, in the quoting style a terminal shows.
        assert!(warning.contains("'xcodegen generate'"), "{warning}");
    }

    #[test]
    fn a_freshly_generated_project_is_quiet() {
        let pf = ProjectFile::default();
        let fresh = generated_project("fresh", "project.yml", false);
        assert_eq!(stale_generated(&pf, &fresh), None);
    }

    #[test]
    fn a_hand_written_project_is_never_stale() {
        // No spec beside it and none declared: nothing regenerates it, so an
        // mtime comparison would be meaningless.
        let pf = ProjectFile::default();
        let plain = temp_project("stale-plain", None);
        std::fs::write(plain.join("project.pbxproj"), "hand-written").unwrap();
        assert_eq!(stale_generated(&pf, &plain), None);
    }

    #[test]
    fn a_declared_generator_still_resolves_the_spec_actually_on_disk() {
        // `generator = "xcodegen"` reports the canonical `project.yml`, but this
        // project uses `project.yaml` — comparing the declared name would find
        // no file and silently skip the check.
        let pf = ProjectFile {
            generator: Some("xcodegen".into()),
            ..ProjectFile::default()
        };
        let stale = generated_project("declared", "project.yaml", true);
        let warning = stale_generated(&pf, &stale).expect("expected a staleness warning");
        assert!(warning.contains("project.yaml is newer"), "{warning}");
    }

    #[test]
    fn generator_detection_by_sibling_spec() {
        let pf = ProjectFile::default();
        let xcodegen = temp_project("xcodegen", Some("project.yml"));
        let found = generator_for(&pf, &xcodegen).unwrap();
        assert_eq!(found.tool, "XcodeGen");
        assert_eq!(found.regenerate, "xcodegen generate");

        let tuist = temp_project("tuist", Some("Project.swift"));
        assert_eq!(generator_for(&pf, &tuist).unwrap().tool, "Tuist");

        let plain = temp_project("plain", None);
        assert!(generator_for(&pf, &plain).is_none());
    }

    #[test]
    fn generator_declared_in_config_wins_without_a_spec_file() {
        let mut pf = ProjectFile {
            generator: Some("xcodegen".into()),
            ..ProjectFile::default()
        };
        let plain = temp_project("config", None);
        let found = generator_for(&pf, &plain).unwrap();
        assert_eq!(found.tool, "XcodeGen");
        // A free-form tool still guards, with generic wording.
        pf.generator = Some("my-gen".into());
        assert_eq!(generator_for(&pf, &plain).unwrap().tool, "my-gen");
        // Blank means undeclared.
        pf.generator = Some("  ".into());
        assert!(generator_for(&pf, &plain).is_none());
    }

    #[test]
    fn guard_refuses_without_force_and_yields_with_it() {
        let pf = ProjectFile::default();
        let xcodeproj = temp_project("guard", Some("project.yml"));
        let err = guard_generated(&pf, &xcodeproj, false).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("App.xcodeproj"), "{text}");
        assert!(text.contains("xcodegen generate"), "{text}");
        assert!(text.contains("project.yml"), "{text}");
        assert!(text.contains("--force"), "{text}");
        assert!(guard_generated(&pf, &xcodeproj, true).is_ok());

        let plain = temp_project("guard-plain", None);
        assert!(guard_generated(&pf, &plain, false).is_ok());
    }
}
