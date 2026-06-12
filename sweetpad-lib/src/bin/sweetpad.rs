//! `sweetpad` — the SweetPad command-line interface.
//!
//! A pure native binary (no Node runtime needed), distributed inside the VS
//! Code extension (`out/sweetpad`); the `sweetpad.system.installCli` command
//! symlinks it onto the user's PATH. Currently a single command:
//!
//!   vscode    one-shot JSON-RPC client for the running SweetPad extension
//!             (the former bundled JS CLI — see [`sweetpad::vscode_cli`])

use std::process::ExitCode;

const USAGE: &str = "\
sweetpad — SweetPad command-line interface

USAGE:
    sweetpad <command> [args...]

COMMANDS:
    vscode    Control the SweetPad VS Code extension over its JSON-RPC
              control server, e.g. `sweetpad vscode scheme.list`.
              `sweetpad vscode --help` lists the methods.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("vscode") => ExitCode::from(sweetpad::vscode_cli::run(&args[1..])),
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown command {other:?}\n\n{USAGE}");
            ExitCode::from(2)
        }
        None => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
