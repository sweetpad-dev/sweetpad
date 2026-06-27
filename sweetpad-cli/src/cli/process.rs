//! Small process-runner used by the tool-spawning commands (simulator, build,
//! app). Two modes: [`capture`] for commands whose stdout we parse (e.g.
//! `simctl list --json`), and [`stream`] for long-running commands whose output
//! belongs on the user's terminal live (e.g. `xcodebuild`).

use std::path::Path;
use std::process::{Child, Command, Stdio};

use crate::cli::{CliError, ErrorKind};

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
            "{program} {} exited with a non-zero status",
            args.join(" ")
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

/// Run a command, invoking `on_line` for each line of stdout as it arrives
/// (stderr inherited). Returns whether the process succeeded. Used to feed
/// xcodebuild output through the native log beautifier ([`crate::cli::buildlog`]).
pub fn stream_lines(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    mut on_line: impl FnMut(&str),
) -> Result<bool, CliError> {
    use std::io::{BufRead, BufReader};

    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().map_err(|e| spawn_error(program, &e))?;
    // Safe: stdout was set to piped above.
    let stdout = child.stdout.take().expect("piped stdout");
    for line in BufReader::new(stdout).lines() {
        match line {
            Ok(line) => on_line(&line),
            Err(_) => break, // non-UTF-8 / closed pipe — stop reading, still wait()
        }
    }
    let status = child.wait().map_err(|e| spawn_error(program, &e))?;
    Ok(status.success())
}

/// Spawn a long-running command in the background with stdout **piped** for the
/// caller to read/format on its own thread (stderr inherited, stdin null). Used
/// by the `app run` session to render the simulator log stream while the keypress
/// loop runs; stdin is null so the child never competes for the terminal's keys.
pub fn spawn_piped(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<Child, CliError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.spawn().map_err(|e| spawn_error(program, &e))
}

/// Like [`spawn_piped`], but with **stderr also piped** so the caller can drain and
/// filter it on its own thread instead of letting it reach the terminal raw. Used by
/// the `app run` os_log stream, whose `log` / `simctl spawn … log` child writes
/// boot-time diagnostics to stderr that we'd rather reformat or drop. stdin is null.
pub fn spawn_piped_both(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<Child, CliError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.spawn().map_err(|e| spawn_error(program, &e))
}

/// Spawn a command with stdout **piped** (for the caller to read line-by-line)
/// and placed in its **own process group**, so a supervisor can signal just this
/// process tree — e.g. forward Ctrl-C to an interruptible build without taking
/// down the parent. stdin is null so it never competes for the terminal's keys.
pub fn spawn_piped_group(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<Child, CliError> {
    use std::os::unix::process::CommandExt;

    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .process_group(0);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.spawn().map_err(|e| spawn_error(program, &e))
}

fn spawn_error(program: &str, e: &std::io::Error) -> CliError {
    if e.kind() == std::io::ErrorKind::NotFound {
        CliError::new(format!(
            "`{program}` not found on PATH (Xcode command-line tools are required)"
        ))
        .kind(ErrorKind::ToolMissing)
    } else {
        CliError::new(format!("failed to run `{program}`: {e}"))
    }
}
