//! `sweetpad-bsp` — the shipped Build Server Protocol server. Starting it *is*
//! the BSP server: there's no subcommand. sourcekit-lsp execs it via
//! `buildServer.json` (`argv: ["…/sweetpad-bsp"]`); it serves BSP over stdio and
//! discovers the project, configuration, and log path over the control socket.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match sweetpad::bsp::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sweetpad-bsp: {e}");
            ExitCode::FAILURE
        }
    }
}
