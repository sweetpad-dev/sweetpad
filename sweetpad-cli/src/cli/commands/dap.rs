//! `sweetpad dap …` — debugging from any editor through the Debug Adapter
//! Protocol. Bare `sweetpad dap` is an adapter in front of Xcode's `lldb-dap`:
//! the editor starts it over stdio, and it builds, installs and launches the
//! app the way `run` does before handing lldb-dap the process to attach to.
//! `dap init` writes an editor's adapter entry, and `dap doctor` checks that
//! lldb-dap resolves. The design is CLI_DESIGN §9u.

use std::path::PathBuf;

use clap::Subcommand;

use crate::cli::{CommandResult, Context};

mod doctor;
mod init;
mod session;
mod stdio;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Write the editor's adapter entry, so its debugger starts 'sweetpad dap'.
    Init {
        /// The editor to configure.
        #[arg(long, value_enum)]
        editor: init::Editor,
        /// Where to write the entry (defaults to the editor's own project
        /// file: '.nvim.lua' for nvim, '.zed/debug.json' for Zed).
        // `--output-file`, not `--output`: the global `-o/--output` owns that flag.
        #[arg(long = "output-file")]
        output_file: Option<PathBuf>,
    },
    /// Check that 'xcrun lldb-dap' resolves and that this Xcode can serve a
    /// debug session.
    Doctor,
}

pub fn run(ctx: &mut Context, action: Option<&Action>) -> CommandResult {
    match action {
        None => session::serve(ctx),
        Some(Action::Init {
            editor,
            output_file,
        }) => init::run(ctx, *editor, output_file.as_deref()),
        Some(Action::Doctor) => doctor::run(ctx),
    }
}

/// The program the adapter starts: `SWEETPAD_LLDB_DAP` when set, so a newer
/// LLVM's lldb-dap or a wrapper script can stand in, else Xcode's through
/// `xcrun`, which follows `DEVELOPER_DIR` and `xcode-select`.
fn lldb_dap_command() -> (String, Vec<String>) {
    match std::env::var("SWEETPAD_LLDB_DAP") {
        Ok(path) if !path.is_empty() => (path, Vec::new()),
        _ => ("xcrun".to_string(), vec!["lldb-dap".to_string()]),
    }
}
