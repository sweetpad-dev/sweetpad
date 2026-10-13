//! Stand-ins for the small Darwin tools a build runs, for Swift Build on
//! Linux, where there is no Xcode and the toolchain has no Mach-O versions of
//! them. The `sweetpad` binary runs as one of these when it's started under
//! its name, which is how the Swift Build engine puts them on Swift Build's
//! `PATH`:
//! - `copypng`, which copies each loose PNG of an iOS target (see
//!   [`copypng`]);
//! - `lipo`, which makes universal binaries and thins the XCTest runner (see
//!   [`lipo`]);
//! - `strip`, for install builds (see [`strip`]);
//! - `ditto`, which run scripts copy with, like the one XcodeGen gives a
//!   static library with Swift sources (see [`ditto`]).

// What these print is the tools' output, which Swift Build and scripts
// parse, so it doesn't go through `Output`.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod copypng;
mod ditto;
mod lipo;
mod strip;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The tools this module stands in for, by the name they run under.
pub const TOOLS: &[&str] = &["copypng", "ditto", "lipo", "strip"];

/// Run as `tool`, or `None` when this module doesn't stand in for it.
#[must_use]
pub fn main(tool: &str, args: &[String]) -> Option<ExitCode> {
    let result = match tool {
        "copypng" => copypng::main(args),
        "ditto" => ditto::main(args),
        "lipo" => lipo::main(args),
        "strip" => strip::main(args),
        _ => return None,
    };
    Some(match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {tool}: {message}");
            ExitCode::FAILURE
        }
    })
}

/// The first `name` on `PATH`.
fn find_program(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The next argument, as the value of `flag`.
fn value<'a>(
    flag: &str,
    args: &mut impl Iterator<Item = &'a String>,
) -> Result<&'a String, String> {
    args.next().ok_or_else(|| format!("'{flag}' takes a value"))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("can't open {}: {e}", path.display()))
}

/// Write `contents` to `path`, replacing what's there, with its folder.
fn write(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    }
    let _ = std::fs::remove_file(path);
    std::fs::write(path, contents).map_err(|e| format!("can't write {}: {e}", path.display()))
}
