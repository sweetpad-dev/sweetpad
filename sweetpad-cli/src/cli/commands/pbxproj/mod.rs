//! `sweetpad pbxproj …` — the plumbing namespace for `project.pbxproj`
//! object-graph surgery (CLI_DESIGN §9g).
//!
//! Inside this namespace you are thinking about pbxproj *objects* — stored
//! build settings, synchronized folders, per-file target membership, merge
//! resolution — not everyday tasks (those stay porcelain: `settings show`,
//! `dependency`, `build`, …). The commands here are built for scripts and
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
use sweetpad_lib::pbxproj::Value;

pub mod folder;
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
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Resolve { paths, force } => merge::resolve(Kind::Pbxproj, paths, *force),
        Action::Settings { action } => settings::run(ctx, action),
        Action::Folder { action } => folder::run(ctx, action),
        Action::Membership { action } => membership::run(ctx, action),
    }
}

/// Locate and parse the pbxproj a namespace action operates on, under the
/// shared never-guess rules ([`pbxedit::mutation_xcodeproj`]).
pub(super) fn open_project(
    ctx: &mut Context,
    container_args: &ContainerArgs,
    target: Option<&String>,
) -> Result<(PathBuf, Value), CliError> {
    ctx.targeting = container_args.clone().into();
    let container = resolve::container(ctx)?;
    let targets: Vec<String> = target.cloned().into_iter().collect();
    let xcodeproj = pbxedit::mutation_xcodeproj(ctx, &container, &targets)?;
    let root = pbxedit::parse_owned(&xcodeproj)?;
    Ok((xcodeproj, root))
}

/// The target to act on: the `--target` flag, or the project's only target.
/// Multiple targets without a flag is ambiguity — a hard error naming them.
pub(super) fn settle_target(root: &Value, flag: Option<&String>) -> Result<String, CliError> {
    if let Some(target) = flag {
        return Ok(target.clone());
    }
    let names = sweetpad_lib::settings_pbxproj::target_names(root);
    match names.as_slice() {
        [] => Err(CliError::new("the project declares no targets")),
        [only] => Ok(only.clone()),
        many => Err(CliError::new(format!(
            "the project has {} targets ({}); pass --target to say which one",
            many.len(),
            many.join(", ")
        ))),
    }
}
