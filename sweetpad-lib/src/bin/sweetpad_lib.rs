//! `sweetpad-lib` — CLI over the interface-agnostic resolver/generator core.
//!
//! Subcommands:
//!   compiler-args   print the generated swiftc/clang/link argv for a target
//!   bsp             run the Build Server Protocol server (see PLAN_BSP.md)
//!
//! Hand-rolled flag parsing, no external crates — the core's zero-dependency
//! stance extends to its entry point.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use sweetpad::build_settings::{self, BuildSettingsOptions};

const USAGE: &str = "\
sweetpad-lib — Xcode resolver / compiler-argument engine

USAGE:
    sweetpad-lib <command> [options]

COMMANDS:
    compiler-args   Resolve and print a target's swiftc/clang/link argv
    bsp             Run the Build Server Protocol server (for sourcekit-lsp)
    config          Write a buildServer.json so sourcekit-lsp finds the server
                    (--project <p> [--xcode <p>] [--derived-data-path <p>] [--output <p>])

compiler-args options:
    --project <path>          .xcodeproj (or --workspace <path>)
    --target <name>           target to resolve (or --scheme <name>)
    --scheme <name>
    --configuration <name>    default: Debug
    --sdk <name>              default: macosx
    --arch <name>             default: arm64
    --xcode <path>            resolve against a specific Xcode.app
    --derived-data-path <p>   xcodebuild -derivedDataPath override
    --tool <swift|clang|link> print only this tool's argv (raw, one per line)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("compiler-args") => cmd_compiler_args(&args[1..]),
        Some("bsp") => cmd_bsp(&args[1..]),
        Some("config") => cmd_config(&args[1..]),
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown command {other:?}\n\n{USAGE}");
            ExitCode::FAILURE
        }
        None => {
            eprint!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// Parse `--key value` / `--key=value` flags into a map (last wins). Bare
/// positionals are ignored — every input this CLI takes is a named flag.
fn parse_flags(args: &[String]) -> BTreeMap<String, String> {
    let mut flags = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        if let Some(key) = args[i].strip_prefix("--") {
            if let Some((k, v)) = key.split_once('=') {
                flags.insert(k.to_string(), v.to_string());
                i += 1;
            } else if i + 1 < args.len() {
                flags.insert(key.to_string(), args[i + 1].clone());
                i += 2;
            } else {
                flags.insert(key.to_string(), String::new());
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    flags
}

/// Build resolver options from parsed flags, applying the same defaults as the
/// node binding (`Debug` / `macosx` / `arm64`).
fn options_from(flags: &BTreeMap<String, String>) -> BuildSettingsOptions {
    BuildSettingsOptions {
        project: flags.get("project").map(PathBuf::from),
        workspace: flags.get("workspace").map(PathBuf::from),
        scheme: flags.get("scheme").cloned(),
        target: flags.get("target").cloned(),
        configuration: flags
            .get("configuration")
            .cloned()
            .unwrap_or_else(|| "Debug".into()),
        sdk: flags.get("sdk").cloned().unwrap_or_else(|| "macosx".into()),
        arch: flags.get("arch").cloned().unwrap_or_else(|| "arm64".into()),
        destination: None,
        xcconfig: flags.get("xcconfig").map(PathBuf::from),
        xcode: flags.get("xcode").map(PathBuf::from),
        xcspec_root: flags.get("xcspec-root").map(PathBuf::from),
        sdksettings_root: None,
        catalog_cache: None,
        derived_data_path: flags.get("derived-data-path").map(PathBuf::from),
        keys: None,
    }
}

fn cmd_compiler_args(args: &[String]) -> ExitCode {
    let flags = parse_flags(args);
    if !flags.contains_key("project") && !flags.contains_key("workspace") {
        eprintln!("error: --project or --workspace is required\n\n{USAGE}");
        return ExitCode::FAILURE;
    }
    let opts = options_from(&flags);
    let targets = match build_settings::resolve_compiler_arguments(&opts) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // `--tool X` prints only that tool's raw argv (one token per line) so it can
    // be reconstructed into a real invocation; otherwise a labelled dump.
    let only = flags.get("tool").map(String::as_str);
    for t in &targets {
        for (name, inv) in [("swift", &t.swift), ("clang", &t.clang), ("link", &t.link)] {
            let Some(inv) = inv else { continue };
            if let Some(want) = only {
                if want != name {
                    continue;
                }
                for a in &inv.arguments {
                    println!("{a}");
                }
                for f in &inv.input_files {
                    println!("{f}");
                }
            } else {
                println!("=== {} {} ({}) ===", t.target, name, inv.tool);
                for a in &inv.arguments {
                    println!("  {a}");
                }
                if !inv.input_files.is_empty() {
                    println!("  --- input files ({}) ---", inv.input_files.len());
                    for f in &inv.input_files {
                        println!("  {f}");
                    }
                }
            }
        }
    }
    ExitCode::SUCCESS
}

fn cmd_bsp(args: &[String]) -> ExitCode {
    match sweetpad::bsp::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bsp: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_config(args: &[String]) -> ExitCode {
    match sweetpad::bsp::write_config(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("config: {e}");
            ExitCode::FAILURE
        }
    }
}
