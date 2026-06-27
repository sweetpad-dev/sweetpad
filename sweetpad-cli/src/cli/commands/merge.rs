//! `sweetpad merge …` — git integration for sweetpad's semantic merges.
//!
//! [`Action::Install`] wires the `.pbxproj` and `Package.resolved` drivers into
//! git (`.gitattributes` + `git config`) so `git merge` resolves them
//! automatically. [`Action::Driver`] is the entry point git invokes per file;
//! it is not meant to be run by hand (git supplies the temp-file paths). Both
//! delegate to [`crate::cli::merge`]; the on-demand counterparts are
//! `pbxproj resolve` and `spm resolve`.

use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

use crate::cli::merge::{self, Kind};
use crate::cli::{CommandResult, Context, Rendered};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DriverKind {
    Pbxproj,
    Spm,
}

impl From<DriverKind> for Kind {
    fn from(k: DriverKind) -> Self {
        match k {
            DriverKind::Pbxproj => Kind::Pbxproj,
            DriverKind::Spm => Kind::Spm,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Register the semantic merge drivers with git (.gitattributes + config).
    Install {
        /// Configure the driver in global git config (and the global
        /// attributes file) instead of this repository.
        #[arg(long)]
        global: bool,
    },
    /// Internal: the merge driver git invokes per conflicted file. Reads the
    /// base/ours/theirs temp files git passes and writes the merged result
    /// over the ours file.
    #[command(hide = true)]
    Driver {
        /// Which file kind this driver handles.
        kind: DriverKind,
        /// Path to git's ancestor (%O) temp file.
        base: PathBuf,
        /// Path to git's current/ours (%A) temp file — also the output.
        ours: PathBuf,
        /// Path to git's other/theirs (%B) temp file.
        theirs: PathBuf,
        /// The real path of the file being merged (git's %P), if available.
        pathname: Option<String>,
    },
}

pub fn run(_ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Install { global } => merge::install(*global),
        Action::Driver {
            kind,
            base,
            ours,
            theirs,
            pathname,
        } => merge::run_driver((*kind).into(), base, ours, theirs, pathname.as_deref())
            .map(|()| Rendered::Streamed),
    }
}
