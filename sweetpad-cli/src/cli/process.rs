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

/// The result of a fully-captured [`run_captured`]: whether the command
/// succeeded, the tail of its combined output for error reporting, and the
/// full combined transcript for callers that post-parse it (diagnostics).
pub struct CapturedRun {
    pub success: bool,
    pub tail: String,
    pub combined: String,
}

/// Run a command with **both stdout and stderr captured** — the `--json` path,
/// where raw child noise must not interleave with the structured output. The
/// last lines of the combined output ride back in
/// [`tail`](CapturedRun::tail) so a failure's cause isn't swallowed with the
/// capture.
pub fn run_captured(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<CapturedRun, CliError> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().map_err(|e| spawn_error(program, &e))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(CapturedRun {
        success: output.status.success(),
        tail: tail_lines(&text, 25),
        combined: text,
    })
}

/// The last `n` non-blank lines of `text`, joined — enough context to explain
/// a failure without replaying a whole transcript inside an error message.
fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
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

/// Run a command, invoking `on_line` for each line of output as it arrives.
/// Both stdout and stderr flow through one merged pipe (chronologically, at
/// pipe level), so tool errors that only reach stderr — `xcodebuild: error:
/// Scheme X is not currently configured…` — hit the parser like any other
/// line instead of bypassing diagnostics collection. Returns whether the
/// process succeeded. Used to feed xcodebuild output through the native log
/// beautifier ([`crate::cli::buildlog`]).
pub fn stream_lines(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    mut on_line: impl FnMut(&str),
) -> Result<bool, CliError> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let (reader, out, err) = merged_output_pipe(program)?;
    cmd.stdout(out).stderr(err);
    let mut child = cmd.spawn().map_err(|e| spawn_error(program, &e))?;
    read_lines_lossy(reader, &mut on_line);
    let status = child.wait().map_err(|e| spawn_error(program, &e))?;
    Ok(status.success())
}

/// One pipe whose write end is duplicated for a child's stdout and stderr, so
/// the parent reads both streams merged in arrival order. The read end comes
/// back as a `File`.
fn merged_output_pipe(program: &str) -> Result<(std::fs::File, Stdio, Stdio), CliError> {
    use std::os::fd::FromRawFd;
    let mut fds = [0 as libc::c_int; 2];
    // Safety: pipe(2) into a stack array; on success both fds are valid and
    // owned exclusively by the wrappers constructed below.
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return Err(CliError::new(format!(
                "failed to run `{program}`: {}",
                std::io::Error::last_os_error()
            )));
        }
        let dup = libc::dup(fds[1]);
        if dup < 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fds[0]);
            libc::close(fds[1]);
            return Err(CliError::new(format!("failed to run `{program}`: {e}")));
        }
        Ok((
            std::fs::File::from_raw_fd(fds[0]),
            Stdio::from_raw_fd(fds[1]),
            Stdio::from_raw_fd(dup),
        ))
    }
}

/// Feed `reader` to `on_line` line by line, decoding lossily — one non-UTF-8
/// byte from a build script degrades to U+FFFD instead of ending the stream
/// (which would close the pipe and SIGPIPE a still-writing child, turning a
/// successful build into a reported failure). Interrupted reads retry inside
/// `read_until`; any other read error ends the stream.
pub(crate) fn read_lines_lossy(reader: impl std::io::Read, on_line: &mut impl FnMut(&str)) {
    use std::io::{BufRead, BufReader};
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                while matches!(buf.last(), Some(b'\n' | b'\r')) {
                    buf.pop();
                }
                on_line(&String::from_utf8_lossy(&buf));
            }
        }
    }
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

/// Spawn a command with its output **piped** (stdout+stderr merged into the
/// returned reader — see [`stream_lines`] for why) and placed in its **own
/// process group**, so a supervisor can signal just this process tree — e.g.
/// forward Ctrl-C to an interruptible build without taking down the parent.
/// stdin is null so it never competes for the terminal's keys.
pub fn spawn_piped_group(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<(Child, std::fs::File), CliError> {
    use std::os::unix::process::CommandExt;

    let mut cmd = Command::new(program);
    let (reader, out, err) = merged_output_pipe(program)?;
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let child = cmd.spawn().map_err(|e| spawn_error(program, &e))?;
    Ok((child, reader))
}

/// Spawn a command with **inherited** stdio in its **own process group**: the
/// terminal's Ctrl-C no longer reaches it directly, so the signal handler's
/// forward-only mode ([`crate::cli::signals::set_forward_child`]) delivers
/// exactly one, well-chosen signal instead — `simctl io recordVideo` needs a
/// single SIGINT to finalize its file.
pub fn spawn_group_inherit(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<Child, CliError> {
    use std::os::unix::process::CommandExt;

    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).process_group(0);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.spawn().map_err(|e| spawn_error(program, &e))
}

fn spawn_error(program: &str, e: &std::io::Error) -> CliError {
    if e.kind() == std::io::ErrorKind::NotFound {
        let hint = match program {
            "xcrun" | "xcodebuild" | "xcode-select" | "swift" | "simctl" => {
                " (Xcode command-line tools are required)"
            }
            "brew" => " (install Homebrew from https://brew.sh)",
            _ => "",
        };
        CliError::new(format!("`{program}` not found on PATH{hint}")).kind(ErrorKind::ToolMissing)
    } else {
        CliError::new(format!("failed to run `{program}`: {e}"))
    }
}
