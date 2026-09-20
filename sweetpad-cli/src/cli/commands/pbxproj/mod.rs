//! `sweetpad pbxproj …` — the plumbing namespace for project-document
//! surgery (CLI_DESIGN §9g).
//!
//! Inside this namespace you are thinking about what the project file stores —
//! build settings, synchronized folders, per-file target membership, the
//! navigator tree, merge resolution — not everyday tasks (those stay
//! porcelain: `settings show`, `dependency`, `build`, …). Everything but
//! `resolve` and `group attach`/`detach` reads and writes either document
//! format, `project.pbxproj` or `project.xcproj`. The commands here are built for scripts and
//! agents: explicit, idempotent, never guessing — an ambiguous workspace,
//! target, or configuration is a hard error naming the flag that
//! disambiguates, on a TTY or off. All mutation rides the shared
//! parse → mutate → serialize pipeline ([`crate::cli::pbxedit`]).

use std::path::PathBuf;

use clap::Subcommand;

use crate::cli::merge::{self, Kind};
use crate::cli::pbxedit;
use crate::cli::resolve;
use crate::cli::{CliError, CommandResult, ContainerArgs, Context};

pub mod fileref;
pub mod folder;
pub mod group;
pub mod membership;
pub mod settings;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Semantically merge conflicted '.pbxproj' files using git's merge state.
    Resolve {
        /// Files to resolve. Defaults to every conflicted '.pbxproj' in the
        /// repository.
        paths: Vec<PathBuf>,
        /// Re-merge from HEAD/MERGE_HEAD even when git already auto-merged the
        /// file textually (so there are no conflict stages in the index).
        #[arg(long)]
        force: bool,
    },
    /// The stored settings layer: show, set, and unset raw
    /// 'buildSettings' entries.
    Settings {
        #[command(subcommand)]
        action: settings::Action,
    },
    /// A target's synchronized source folders.
    Folder {
        #[command(subcommand)]
        action: folder::Action,
    },
    /// Per-file target membership, across both representations.
    Membership {
        #[command(subcommand)]
        action: membership::Action,
    },
    /// The files the project holds, independent of which group shows them
    /// or which target builds them.
    Fileref {
        #[command(subcommand)]
        action: fileref::Action,
    },
    /// The navigator group tree: where a file appears in Xcode, which is a
    /// separate question from what builds it.
    Group {
        #[command(subcommand)]
        action: group::Action,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Resolve { paths, force } => merge::resolve(Kind::Pbxproj, paths, *force),
        Action::Settings { action } => settings::run(ctx, action),
        Action::Folder { action } => folder::run(ctx, action),
        Action::Membership { action } => membership::run(ctx, action),
        Action::Fileref { action } => fileref::run(ctx, action),
        Action::Group { action } => group::run(ctx, action),
    }
}

/// Locate and parse the project document a namespace action operates on,
/// under the shared never-guess rules ([`pbxedit::mutation_xcodeproj`]).
pub(super) fn open_document(
    ctx: &mut Context,
    container_args: &ContainerArgs,
    target: Option<&String>,
) -> Result<(PathBuf, pbxedit::Editable), CliError> {
    ctx.targeting = container_args.clone().into();
    let container = resolve::container(ctx)?;
    let targets: Vec<String> = target.cloned().into_iter().collect();
    let xcodeproj = pbxedit::mutation_xcodeproj(ctx, &container, &targets)?;
    let document = pbxedit::Editable::parse(&xcodeproj)?;
    Ok((xcodeproj, document))
}

/// [`open_document`] for the *mutation* verbs: additionally refuses to edit a
/// generated project without `--force` ([`pbxedit::guard_generated`]), before
/// any work or disk side effect happens.
pub(super) fn open_document_mut(
    ctx: &mut Context,
    container_args: &ContainerArgs,
    targets: &[String],
    force: bool,
) -> Result<(PathBuf, pbxedit::Editable), CliError> {
    ctx.targeting = container_args.clone().into();
    let container = resolve::container(ctx)?;
    let xcodeproj = pbxedit::mutation_xcodeproj(ctx, &container, targets)?;
    pbxedit::guard_generated(ctx.project_file(&container), &xcodeproj, force)?;
    let document = pbxedit::Editable::parse(&xcodeproj)?;
    Ok((xcodeproj, document))
}

/// The target to act on: the `--target` flag, or the project's only target.
/// Multiple targets without a flag is ambiguity — a hard error naming them.
pub(super) fn settle_target(
    document: &pbxedit::Editable,
    flag: Option<&String>,
) -> Result<String, CliError> {
    if let Some(target) = flag {
        return Ok(target.clone());
    }
    match document.target_names().as_slice() {
        [] => Err(CliError::new("the project declares no targets")),
        [only] => Ok(only.clone()),
        many => Err(CliError::new(format!(
            "the project has {} targets ({}); pass --target to say which one",
            many.len(),
            many.join(", ")
        ))),
    }
}
