//! Scheme execution actions run outside Swift Build's target task graph.

use super::{Invocation, xcodebuild};
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
            if action.action_type
                != "Xcode.IDEStandardExecutionActionsCore.ExecutionActionType.ShellScriptAction"
            {
                return Err(CliError::new(format!(
                    "unsupported {phase} action '{}' of type '{}'",
                    action.title, action.action_type
                ))
                .kind(ErrorKind::Usage));
            }
            let mut env = Vec::new();
            if let Some(reference) = &action.environment_buildable {
                let project = reference.project_path(&self.file).ok_or_else(|| CliError::new(format!(
                    "{phase} action '{}' needs a project target for its build-settings environment", action.title)))?;
                let target = if reference.blueprint_identifier.is_empty() {
                    reference.blueprint_name.clone()
                } else {
                    sweetpad_lib::project::target_name(&project, &reference.blueprint_identifier)
                        .ok_or_else(|| {
                            CliError::new(format!(
                                "{phase} action '{}' references missing target {} in {}",
                                action.title,
                                reference.blueprint_identifier,
                                project.display()
                            ))
                        })?
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
            let shell = if cfg!(target_os = "linux") && action.shell == "/bin/sh" {
                "/bin/bash"
            } else {
                &action.shell
            };
            out.note(&format!("{phase}: {}", action.title));
            let mut transcript = String::new();
            let ok = process::stream_lines_with_env(
                shell,
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
