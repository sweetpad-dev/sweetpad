//! `sweetpad` — the SweetPad command-line interface.
//!
//! A pure native binary (no Node runtime needed), distributed via Homebrew
//! (`brew install sweetpad-dev/tap/sweetpad`).
//!
//! Two halves share this one binary:
//!
//!   vscode   one-shot JSON-RPC client for the running SweetPad extension
//!            (the former bundled JS CLI — see [`sweetpad_cli::vscode_cli`]).
//!
//!   <else>   the standalone, headless CLI — "xcodebuild for humans": a
//!            resource-first command tree (`scheme`, `destination`, `project`,
//!            `settings`, `simulator`, `build`, `app`) over the resolver in
//!            this crate. See [`sweetpad_cli::cli`] and `CLI_DESIGN.md`.
//!
//! `vscode` is peeled off here so its bespoke JSON-RPC arg handling stays
//! independent; everything else routes through clap in [`sweetpad_cli::cli::run`].

use std::process::ExitCode;

fn main() -> ExitCode {
    // The Rust runtime ignores SIGPIPE, so a consumer closing the pipe early
    // (`sweetpad … | head`) surfaces as a "failed printing to stdout: Broken
    // pipe" panic (exit 101, backtrace on stderr). Restore the default
    // disposition so the process dies quietly like every other Unix tool.
    // Safety: setting a signal disposition before any threads exist.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("vscode") => ExitCode::from(sweetpad_cli::vscode_cli::run(&args[1..])),
        _ => sweetpad_cli::cli::run(&args),
    }
}
