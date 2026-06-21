//! `sweetpad pbxproj …` — operate on Xcode `project.pbxproj` files.
//!
//! One action: [`Action::Resolve`], a semantic three-way merge for `.pbxproj`
//! files left conflicted by `git merge`/`rebase`/`cherry-pick`. Run it
//! *mid-conflict*: it reconstructs the three clean inputs from git's index
//! stages (`:1:` base, `:2:` ours, `:3:` theirs) — never the marker-riddled
//! working copy — merges the object graphs via [`crate::pbxproj_merge`],
//! serializes byte-for-byte, and `git add`s the result. The shared plumbing
//! lives in [`crate::cli::merge`]; to merge these automatically during
//! `git merge`, see `sweetpad merge install`.

use std::path::PathBuf;

use clap::Subcommand;

use crate::cli::merge::{self, Kind};
use crate::cli::{CommandResult, Context};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Semantically merge conflicted `.pbxproj` files using git's merge state.
    Resolve {
        /// Files to resolve. Defaults to every conflicted `.pbxproj` in the
        /// repository.
        paths: Vec<PathBuf>,
        /// Re-merge from HEAD/MERGE_HEAD even when git already auto-merged the
        /// file textually (so there are no conflict stages in the index).
        #[arg(long)]
        force: bool,
    },
}

pub fn run(_ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Resolve { paths, force } => merge::resolve(Kind::Pbxproj, paths, *force),
    }
}
