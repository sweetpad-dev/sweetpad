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
    // (The runtime already clobbered any disposition inherited from the
    // parent, so a pre-exec `SIG_IGN` cannot be detected or preserved here.)
    // Safety: setting a signal disposition before any threads exist.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let mut process_args = std::env::args();
    let program = process_args.next().unwrap_or_default();
    let mut args: Vec<String> = process_args.collect();
    // Run as `sweetpad-dap`, the binary is `sweetpad dap`: an editor whose
    // debug adapter setting takes a binary but no arguments (Zed's Swift
    // adapter) starts the adapter through that name.
    let name = std::path::Path::new(&program).file_name();
    if name == Some(std::ffi::OsStr::new("sweetpad-dap")) {
        args.insert(0, "dap".to_string());
    }
    // Run as `actool`, the binary stands in for Xcode's asset catalog compiler,
    // which the Swift Build engine points Swift Build at on Linux.
    if name == Some(std::ffi::OsStr::new("actool")) {
        return sweetpad_cli::cli::actool::main(&args);
    }
    // Run as `xcstringstool`, it stands in for Xcode's String Catalog
    // compiler, which Swift Build looks up on the `PATH` the engine gives it.
    if name == Some(std::ffi::OsStr::new("xcstringstool")) {
        return sweetpad_cli::cli::xcstringstool::main(&args);
    }
    // Run as `momc`, it stands in for Xcode's Core Data model compiler,
    // which Swift Build also looks up on that `PATH`.
    if name == Some(std::ffi::OsStr::new("momc")) {
        return sweetpad_cli::cli::momc::main(&args);
    }
    // Run as `ibtool`, it stands in for Xcode's Interface Builder compiler,
    // which Swift Build also looks up on that `PATH`.
    if name == Some(std::ffi::OsStr::new("ibtool")) {
        return sweetpad_cli::cli::ibtool::main(&args);
    }
    // Run as `lipo`, `strip`, `copypng`, `ditto` or
    // `embeddedBinaryValidationUtility`, it stands in for the Darwin tool of
    // that name, found on the same `PATH`.
    if let Some(code) = name
        .and_then(std::ffi::OsStr::to_str)
        .and_then(|tool| sweetpad_cli::cli::darwin_tools::main(tool, &args))
    {
        return code;
    }
    match args.first().map(String::as_str) {
        Some("vscode") => ExitCode::from(sweetpad_cli::vscode_cli::run(&args[1..])),
        _ => sweetpad_cli::cli::run(&args),
    }
}
