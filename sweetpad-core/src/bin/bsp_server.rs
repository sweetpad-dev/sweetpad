//! `bsp-server` — standalone entry point for the Build Server Protocol server
//! (see DOCS.md §8 (BSP server)).
//!
//! Not a user-facing CLI: this exists so the BSP integration tests (and manual
//! debugging) can exec the stdio server, and so `bsp::write_config` has an
//! executable to point `buildServer.json`'s `argv` at. In the extension the
//! same server runs through the N-API addon (`node::bsp`).

use std::process::ExitCode;

const USAGE: &str = "\
bsp-server — Build Server Protocol server for sourcekit-lsp

USAGE:
    bsp-server <command> [options]

COMMANDS:
    bsp      Run the BSP server loop over stdio
    config   Write a buildServer.json so sourcekit-lsp finds the server
             (--project <p> | --workspace <p>) [--xcode <p>]
             [--derived-data-path <p>] [--output <p>]
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("bsp") => sweetpad_core::bsp::run(&args[1..]),
        Some("config") => sweetpad_core::bsp::write_config(&args[1..]),
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(format!("unknown command {other:?}\n\n{USAGE}")),
        None => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bsp-server: {e}");
            ExitCode::FAILURE
        }
    }
}
