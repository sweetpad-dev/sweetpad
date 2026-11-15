//! Scheme execution actions run outside Swift Build's target task graph.

use super::{Invocation, xcodebuild};
use crate::cli::codesign::HiddenSecrets;
use crate::cli::{CliError, ErrorKind, output::Output, process};
use std::path::{Path, PathBuf};
use sweetpad_lib::scheme::Scheme;

#[derive(Clone, Copy)]
enum Phase<'a> {
    BuildPre,
    BuildPost,
    ArchivePre,
    ArchivePost(&'a Path),
}

pub(super) struct Actions {
    file: PathBuf,
    scheme: Scheme,
}

impl Actions {
    pub(super) fn load(
        plan: &xcodebuild::BuildPlan<'_>,
        invocation: &Invocation,
    ) -> Result<Option<Self>, CliError> {
        sweetpad_lib::scheme::locate_with_derived_data(
            &invocation.container,
            plan.scheme,
            Some(&invocation.locations.folder),
        )
        .map(|file| {
            let scheme = sweetpad_lib::scheme::parse_file(&file)
                .map_err(|e| CliError::new(format!("read scheme {}: {e}", file.display())))?;
            Ok(Self { file, scheme })
        })
        .transpose()
    }

    fn run(
        &self,
        stage: Phase<'_>,
        plan: &xcodebuild::BuildPlan<'_>,
        invocation: &Invocation,
        out: &Output,
    ) -> Result<(), CliError> {
        let (scripts, phase) = match stage {
            Phase::BuildPre => (&self.scheme.build_scripts.pre, "BuildAction.PreActions"),
            Phase::BuildPost => (&self.scheme.build_scripts.post, "BuildAction.PostActions"),
            Phase::ArchivePre => (&self.scheme.archive_scripts.pre, "ArchiveAction.PreActions"),
            Phase::ArchivePost(_) => (
                &self.scheme.archive_scripts.post,
                "ArchiveAction.PostActions",
            ),
        };
        for action in scripts {
            // xcodebuild runs only the shell actions and passes over the rest,
            // such as Send Email, which is the IDE's.
            if action.action_type
                != "Xcode.IDEStandardExecutionActionsCore.ExecutionActionType.ShellScriptAction"
            {
                out.note(&format!(
                    "{phase}: skipped '{}', which isn't a shell script, as xcodebuild does",
                    action.title
                ));
                continue;
            }
            let mut env = Vec::new();
            if let Some(reference) = &action.environment_buildable {
                let project = reference.project_path(&self.file).ok_or_else(|| CliError::new(format!(
                    "{phase} action '{}' needs a project target for its build-settings environment", action.title)))?;
                let (project, target) = if project.join("Package.swift").is_file() {
                    register_packages(invocation)?;
                    let target = if reference.blueprint_name.is_empty() {
                        reference.blueprint_identifier.clone()
                    } else {
                        reference.blueprint_name.clone()
                    };
                    (project.join("Package.swift"), target)
                } else {
                    let target = project_target(&project, reference, phase, &action.title)?;
                    (project, target)
                };
                env.extend(
                    xcodebuild::scheme_action_settings(
                        plan,
                        project,
                        &target,
                        &invocation.locations,
                        invocation.package_sysroot.as_deref(),
                    )?
                    .into_iter()
                    .filter(|(key, value)| {
                        !key.is_empty() && !key.contains(['=', '\0']) && !value.contains('\0')
                    })
                    .map(|(key, value)| (key, Some(value))),
                );
            }
            if let Phase::ArchivePost(archive) = stage {
                env.extend([
                    ("ARCHIVE_PATH".into(), Some(archive.display().to_string())),
                    (
                        "ARCHIVE_DSYMS_PATH".into(),
                        Some(archive.join("dSYMs").display().to_string()),
                    ),
                    (
                        "ARCHIVE_PRODUCTS_PATH".into(),
                        Some(archive.join("Products").display().to_string()),
                    ),
                ]);
            }
            out.note(&format!("{phase}: {}", action.title));
            let _secrets =
                HiddenSecrets::hide_environment(&mut env, false).map_err(CliError::new)?;
            let mut transcript = String::new();
            let ok = process::stream_lines_with_env(
                shell(&action.shell),
                &["-c", &action.script],
                Some(&std::env::temp_dir()),
                &env,
                |line| {
                    transcript.push_str(line);
                    transcript.push('\n');
                    out.note(line);
                    if out.is_ndjson() {
                        out.ndjson_event(&serde_json::json!({"event":"scheme_action_output", "phase":phase, "title":action.title, "message":line}));
                    }
                },
            )?;
            if !ok {
                xcodebuild::record_build(plan.container, Some(plan.scheme), false, &[]);
                let log = xcodebuild::project_artifact(plan.container, "-scheme-action.log");
                let recorded = xcodebuild::record_failure_transcript(&log, &transcript);
                return Err(
                    CliError::new(format!("{phase} action '{}' failed", action.title))
                        .kind(ErrorKind::BuildFailure)
                        .tip(
                            recorded
                                .map(|p| format!("read full scheme action log: {}", p.display())),
                        ),
                );
            }
        }
        Ok(())
    }
}

/// The project target `reference` names: by its ID, else by its name.
fn project_target(
    project: &Path,
    reference: &sweetpad_lib::scheme::BuildableRef,
    phase: &str,
    title: &str,
) -> Result<String, CliError> {
    if reference.blueprint_identifier.is_empty() {
        return Ok(reference.blueprint_name.clone());
    }
    sweetpad_lib::project::target_name(project, &reference.blueprint_identifier).ok_or_else(|| {
        CliError::new(format!(
            "{phase} action '{title}' references missing target {} in {}",
            reference.blueprint_identifier,
            project.display()
        ))
    })
}

/// File the local packages of the build's container, as SwiftPM describes
/// them, so a package target's settings resolve as a project target's do.
/// An action can run before the build has read them.
fn register_packages(invocation: &Invocation) -> Result<(), CliError> {
    let packages = sweetpad_core::package_pif::load(
        &invocation.container,
        &invocation.locations.folder.join("SourcePackages"),
        &invocation
            .locations
            .intermediates
            .join("SweetPad/PackageSettings"),
        &sweetpad_core::package_members::Toolchain {
            swift: None,
            developer_dir: None,
        },
    )
    .map_err(|e| CliError::new(format!("couldn't read the local packages: {e}")))?;
    sweetpad_lib::pif::register(&invocation.container, packages);
    Ok(())
}

pub(super) fn build(
    actions: Option<&Actions>,
    post: bool,
    plan: &xcodebuild::BuildPlan<'_>,
    invocation: &Invocation,
    out: &Output,
) -> Result<(), CliError> {
    if let Some(actions) = actions {
        actions.run(
            if post {
                Phase::BuildPost
            } else {
                Phase::BuildPre
            },
            plan,
            invocation,
            out,
        )?;
    }
    Ok(())
}

pub(crate) fn archive(
    post: bool,
    path: &Path,
    plan: &xcodebuild::BuildPlan<'_>,
    out: &Output,
) -> Result<(), CliError> {
    let invocation = Invocation::of(plan)?;
    if let Some(actions) = Actions::load(plan, &invocation)? {
        actions.run(
            if post {
                Phase::ArchivePost(path)
            } else {
                Phase::ArchivePre
            },
            plan,
            &invocation,
            out,
        )?;
    }
    Ok(())
}

/// The shell a scheme action runs in: macOS's `/bin/sh` takes Bash syntax, so
/// on Linux, whose `/bin/sh` may not, an action naming it runs in Bash.
fn shell(named: &str) -> &str {
    if cfg!(target_os = "linux") && named == "/bin/sh" {
        "/bin/bash"
    } else {
        named
    }
}
