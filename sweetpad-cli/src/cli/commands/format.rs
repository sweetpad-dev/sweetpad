//! `sweetpad format …` — format/lint Swift sources with swift-format (default)
//! or SwiftLint. Both tools read their own project config (`.swift-format`,
//! `.swiftlint.yml`), so this just locates the tool and the files.

use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

use crate::cli::output::Output;
use crate::cli::{CommandResult, Context, ErrorContext, ErrorKind, Render, Rendered, process};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Tool {
    SwiftFormat,
    Swiftlint,
}

/// The format flags — `--tool`/`--check` are global at the `format` resource
/// so `sweetpad format --check` and `sweetpad format run --check` both parse;
/// the positional paths (which can't be global) are declared at both levels.
#[derive(Debug, clap::Args)]
pub struct FormatArgs {
    /// Files or directories to format; defaults to the project directory.
    pub paths: Vec<PathBuf>,

    /// Which formatter to use (default: `[format] tool` in sweetpad.toml,
    /// else swift-format).
    #[arg(long, value_enum, global = true)]
    pub tool: Option<Tool>,

    /// Check formatting without modifying files (non-zero exit if changes
    /// are needed).
    #[arg(long, global = true)]
    pub check: bool,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Format the given files (the default action: `sweetpad format`).
    Run {
        /// Files or directories to format; defaults to the project directory.
        paths: Vec<PathBuf>,
    },
}

pub fn run(ctx: &mut Context, args: &FormatArgs, action: Option<&Action>) -> CommandResult {
    // Paths may arrive at either level (`format A/` or `format run A/`).
    let paths = match action {
        Some(Action::Run { paths }) if !paths.is_empty() => paths,
        _ => &args.paths,
    };
    let tool = args.tool.unwrap_or_else(|| project_default_tool(ctx));
    format(ctx, paths, tool, args.check)
}

/// The project's `[format] tool` from sweetpad.toml, defaulting to
/// swift-format; a typo'd value is warned about rather than silently ignored.
fn project_default_tool(ctx: &Context) -> Tool {
    let Ok(container) = crate::cli::resolve::container(ctx) else {
        return Tool::SwiftFormat;
    };
    match ctx.project_file(&container).format.tool.as_deref() {
        None | Some("swift-format") => Tool::SwiftFormat,
        Some("swiftlint") => Tool::Swiftlint,
        Some(other) => {
            ctx.out.warn(&format!(
                "sweetpad.toml: unknown [format] tool {other:?} (use swift-format|swiftlint); using swift-format"
            ));
            Tool::SwiftFormat
        }
    }
}

/// The format/lint result. The tool's own output already streamed in human
/// mode; `--json` reports the tool, mode, and whether the check passed.
struct FormatReport {
    tool: String,
    check: bool,
    passed: bool,
}

impl Render for FormatReport {
    fn human(&self, _out: &Output) {}

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "tool": self.tool,
            "mode": if self.check { "check" } else { "format" },
            "passed": self.passed,
        })
    }
}

fn format(ctx: &mut Context, paths: &[PathBuf], tool: Tool, check: bool) -> CommandResult {
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
    ctx.out.note(&format!(
        "{} with {tool:?}",
        if check { "checking" } else { "formatting" }
    ));

    // JSON reserves stdout for the result, so run the tool quietly — its own
    // stdout would otherwise corrupt the envelope. A failed `--check` still
    // reports a result, but exits non-zero so CI catches it.
    if ctx.out.is_json() {
        let passed = process::run(&program, &arg_refs, None, true)?;
        let report = FormatReport {
            tool: format!("{tool:?}"),
            check,
            passed,
        };
        return if passed {
            Ok(Rendered::data(report))
        } else {
            Ok(Rendered::data_with_exit(report, 3))
        };
    }

    process::stream(&program, &arg_refs, None)
        .map_err(|e| e.or_kind(ErrorKind::BuildFailure))
        .context(if check {
            "checking Swift formatting"
        } else {
            "formatting Swift sources"
        })?;
    Ok(Rendered::data(FormatReport {
        tool: format!("{tool:?}"),
        check,
        passed: true,
    }))
}

/// swift-format, preferring the Xcode-bundled copy (`xcrun swift-format`).
fn swift_format_command(check: bool, recursive: bool) -> (String, Vec<String>) {
    let bundled = process::capture("xcrun", &["--find", "swift-format"], None).is_ok();
    let (program, mut args) = if bundled {
        ("xcrun".to_string(), vec!["swift-format".to_string()])
    } else {
        ("swift-format".to_string(), Vec::new())
    };
    args.push(if check {
        "lint".into()
    } else {
        "format".into()
    });
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
