//! `sweetpad format …` — format/lint Swift sources with swift-format (default)
//! or SwiftLint. Both tools read their own project config (`.swift-format`,
//! `.swiftlint.yml`), so this just locates the tool and the files.

use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

use crate::cli::{process, CliResult, Context};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Tool {
    SwiftFormat,
    Swiftlint,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Format the given files (or the whole project when none are given).
    Run {
        /// Files or directories to format; defaults to the project directory.
        paths: Vec<PathBuf>,
        /// Which formatter to use.
        #[arg(long, value_enum, default_value_t = Tool::SwiftFormat)]
        tool: Tool,
        /// Check formatting without modifying files (non-zero exit if changes
        /// are needed).
        #[arg(long)]
        check: bool,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::Run { paths, tool, check } => format(ctx, paths, *tool, *check),
    }
}

fn format(ctx: &mut Context, paths: &[PathBuf], tool: Tool, check: bool) -> CliResult {
    // Default to the current directory when no paths are given.
    let default_dir = PathBuf::from(".");
    let targets: Vec<String> = if paths.is_empty() {
        vec![default_dir.display().to_string()]
    } else {
        paths.iter().map(|p| p.display().to_string()).collect()
    };
    let recursive = paths.is_empty() || paths.iter().any(|p| p.is_dir());

    let (program, mut args) = match tool {
        Tool::SwiftFormat => swift_format_command(check, recursive),
        Tool::Swiftlint => swiftlint_command(check),
    };
    args.extend(targets);

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    ctx.out.note(&format!("{} with {tool:?}", if check { "checking" } else { "formatting" }));
    process::stream(&program, &arg_refs, None)
}

/// swift-format, preferring the Xcode-bundled copy (`xcrun swift-format`).
fn swift_format_command(check: bool, recursive: bool) -> (String, Vec<String>) {
    let bundled = process::capture("xcrun", &["--find", "swift-format"], None).is_ok();
    let (program, mut args) = if bundled {
        ("xcrun".to_string(), vec!["swift-format".to_string()])
    } else {
        ("swift-format".to_string(), Vec::new())
    };
    args.push(if check { "lint".into() } else { "format".into() });
    if !check {
        args.push("--in-place".into());
    }
    if recursive {
        args.push("--recursive".into());
    }
    (program, args)
}

/// SwiftLint: `lint` to check, `--fix` to autocorrect.
fn swiftlint_command(check: bool) -> (String, Vec<String>) {
    let args = if check {
        vec!["lint".to_string(), "--quiet".to_string()]
    } else {
        vec!["--fix".to_string(), "--quiet".to_string()]
    };
    ("swiftlint".to_string(), args)
}
