//! `sweetpad spm …` — operate on SwiftPM `Package.resolved` files.
//!
//! One action: [`Action::Resolve`], the `Package.resolved` counterpart to
//! `pbxproj resolve`. Run it mid-conflict to merge the dependency `pins`
//! semantically (by `identity`) from git's index stages — see
//! [`crate::spm_resolved`] for the engine and [`crate::cli::merge`] for the
//! shared git plumbing. For automatic merges during `git merge`, see
//! `sweetpad merge install`.

use std::path::PathBuf;

use clap::Subcommand;

use crate::cli::merge::{self, Kind};
use crate::cli::{CommandResult, Context};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Semantically merge conflicted `Package.resolved` files.
    Resolve {
        /// Files to resolve. Defaults to every conflicted `Package.resolved`
        /// in the repository.
        paths: Vec<PathBuf>,
        /// Re-merge from HEAD/MERGE_HEAD even when git already auto-merged the
        /// file textually (so there are no conflict stages in the index).
        #[arg(long)]
        force: bool,
    },
}

pub fn run(_ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Resolve { paths, force } => merge::resolve(Kind::Spm, paths, *force),
    }
}
