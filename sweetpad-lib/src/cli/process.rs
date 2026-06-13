//! Small process-runner used by the tool-spawning commands (simulator, build,
//! app). Two modes: [`capture`] for commands whose stdout we parse (e.g.
//! `simctl list --json`), and [`stream`] for long-running commands whose output
//! belongs on the user's terminal live (e.g. `xcodebuild`).

use std::path::Path;
use std::process::{Command, Stdio};

use crate::cli::CliError;

/// Run a command to completion, capturing stdout. Stderr is inherited so the
/// user still sees diagnostics. Errors if the process can't be spawned or exits
/// non-zero.
pub fn capture(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, CliError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().map_err(|e| spawn_error(program, &e))?;
    if !output.status.success() {
        return Err(CliError::new(format!(
            "{program} {} exited with {}",
            args.join(" "),
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Run a command to completion with stdio inherited — output streams straight
/// to the terminal. A non-zero exit is surfaced as an error so callers can stop
/// a pipeline (use [`run`] when a non-zero exit is a meaningful result, e.g.
/// test failures).
pub fn stream(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<(), CliError> {
    if run(program, args, cwd, false)? {
        Ok(())
    } else {
        Err(CliError::new(format!(
            "{program} exited with a non-zero status"
        )))
    }
}

/// Run a command to completion, returning whether it succeeded rather than
/// erroring on a non-zero exit. `quiet` discards stdout (stderr is always
/// inherited) — used when only the exit status / a side-effect matters, e.g.
/// `xcodebuild test` whose pass/fail we read from the result bundle.
pub fn run(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    quiet: bool,
) -> Result<bool, CliError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdout(if quiet {
            Stdio::null()
        } else {
            Stdio::inherit()
        })
        .stderr(Stdio::inherit());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let status = cmd.status().map_err(|e| spawn_error(program, &e))?;
    Ok(status.success())
}

fn spawn_error(program: &str, e: &std::io::Error) -> CliError {
    if e.kind() == std::io::ErrorKind::NotFound {
        CliError::new(format!(
            "`{program}` not found on PATH (Xcode command-line tools are required)"
        ))
    } else {
        CliError::new(format!("failed to run `{program}`: {e}"))
    }
}
