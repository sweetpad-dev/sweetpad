//! `cc`, which Swift Build runs to preprocess an Info.plist with
//! `INFOPLIST_PREPROCESS`, as Xcode runs its toolchain's: `cc -E -P -x c
//! -Wno-trigraphs Info.plist … -target arm64-apple-ios17.0-simulator
//! -isysroot …`. A Linux host's `cc` may be GCC, which takes no `-target`,
//! so a run for an Apple target goes to Clang. Any other run, like a script
//! building a tool for the host, goes to the next `cc` on `PATH`.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

pub fn main(args: &[String]) -> ExitCode {
    let message = match program(args) {
        Ok(program) => format!(
            "can't run {}: {}",
            program.display(),
            super::exec(Command::new(&program).args(args))
        ),
        Err(message) => message,
    };
    eprintln!("error: cc: {message}");
    ExitCode::FAILURE
}

/// Clang for an Apple target, else the host's own `cc`.
fn program(args: &[String]) -> Result<PathBuf, String> {
    if targets_apple(args) {
        super::find_program("clang")
            .ok_or_else(|| "preprocessing for an Apple target needs 'clang' on PATH".to_string())
    } else {
        host_cc().ok_or_else(|| "there's no other 'cc' on PATH".to_string())
    }
}

/// Whether `args` name an Apple target, with `-target` or `--target=`.
fn targets_apple(args: &[String]) -> bool {
    args.iter().enumerate().any(|(i, arg)| {
        let target = if arg == "-target" {
            args.get(i + 1).map(String::as_str)
        } else {
            arg.strip_prefix("--target=")
        };
        target.is_some_and(|t| t.contains("-apple-"))
    })
}

/// The first `cc` on `PATH` that isn't this stand-in.
fn host_cc() -> Option<PathBuf> {
    let this = std::env::current_exe().ok()?.canonicalize().ok()?;
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("cc"))
        .find(|candidate| {
            candidate.is_file() && candidate.canonicalize().ok().as_ref() != Some(&this)
        })
}

#[cfg(test)]
mod tests {
    use super::targets_apple;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_string()).collect()
    }

    #[test]
    fn an_apple_target_goes_to_clang_and_a_host_build_to_the_hosts_cc() {
        // What Swift Build runs for INFOPLIST_PREPROCESS.
        assert!(targets_apple(&strings(&[
            "-E",
            "-P",
            "-x",
            "c",
            "-Wno-trigraphs",
            "Info.plist",
            "-target",
            "arm64-apple-ios17.0-simulator",
            "-isysroot",
            "/sdk",
        ])));
        assert!(targets_apple(&strings(&[
            "--target=x86_64-apple-macos14",
            "-c",
            "a.c"
        ])));
        assert!(!targets_apple(&strings(&["-O2", "-o", "tool", "tool.c"])));
        assert!(!targets_apple(&strings(&[
            "-target",
            "x86_64-unknown-linux-gnu",
            "a.c"
        ])));
        assert!(!targets_apple(&strings(&["-target"])));
    }
}
