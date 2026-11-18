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
//!   static library with Swift sources (see [`ditto`]);
//! - `embeddedBinaryValidationUtility`, which checks each app extension an
//!   app embeds (see [`embedded_binary`]);
//! - `ld64.lld`, in front of LLVM's linker, which makes a static archive
//!   for `ld -r` and loads such archives whole (see [`ld`]);
//! - `dsymutil`, in front of LLVM's (see [`dsymutil`]);
//! - `xcrun`, which run scripts start tools through and ask about an SDK
//!   with (see [`xcrun`]);
//! - `cc`, which preprocesses an Info.plist for an Apple target with Clang
//!   (see [`cc`]).

// What these print is the tools' output, which Swift Build and scripts
// parse, so it doesn't go through `Output`.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod cc;
mod copypng;
mod ditto;
mod dsymutil;
mod embedded_binary;
mod extract_loc_strings;
pub(crate) mod host;
mod ld;
mod lipo;
pub(crate) use lipo::is_static_archive;
mod strip;
mod xcrun;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The tools this module stands in for, by the name they run under.
pub const TOOLS: &[&str] = &[
    "cc",
    "copypng",
    "ditto",
    "embeddedBinaryValidationUtility",
    "ld64.lld",
    "llvm-libtool-darwin",
    "lipo",
    "strip",
    "xcrun",
    "sweetpad-chmod",
    "extractLocStrings",
    "dsymutil",
];

/// The names of the links to LLVM's linker and `dsymutil`, beside the
/// stand-ins for them, which run them (see [`ld`] and [`dsymutil`]).
pub const SDK_LINKER: &str = "sdk-ld64.lld";
pub const SDK_DSYMUTIL: &str = "sdk-dsymutil";

/// Run as `tool`, or `None` when this module doesn't stand in for it.
#[must_use]
pub fn main(tool: &str, args: &[String]) -> Option<ExitCode> {
    let result = match tool {
        "cc" => return Some(cc::main(args)),
        "copypng" => copypng::main(args),
        "ditto" => ditto::main(args),
        "embeddedBinaryValidationUtility" => return Some(embedded_binary::main(args)),
        "ld64.lld" => return Some(ld::main(args)),
        "dsymutil" => return Some(dsymutil::main(args)),
        "llvm-libtool-darwin" => ld::libtool::main(args),
        "lipo" => lipo::main(args),
        "strip" => strip::main(args),
        "extractLocStrings" => extract_loc_strings::main(args),
        "xcrun" => return Some(xcrun::main(args)),
        "sweetpad-chmod" => {
            let args: Vec<&str> = args
                .iter()
                .filter(|a| a.as_str() != "-H")
                .map(|a| if a == "-RH" { "-R" } else { a.as_str() })
                .collect();
            std::process::Command::new("/bin/chmod")
                .args(args)
                .status()
                .map_err(|e| e.to_string())
                .and_then(|s| {
                    s.success()
                        .then_some(())
                        .ok_or_else(|| format!("chmod exited with {s}"))
                })
        }
        _ => host::main(tool, args)?,
    };
    Some(match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {tool}: {message}");
            ExitCode::FAILURE
        }
    })
}

/// Replace this process with `command`, returning only when that fails.
#[cfg(unix)]
fn exec(command: &mut std::process::Command) -> std::io::Error {
    std::os::unix::process::CommandExt::exec(command)
}

#[cfg(not(unix))]
fn exec(command: &mut std::process::Command) -> std::io::Error {
    match command.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => e,
    }
}

/// The first `name` on `PATH`.
pub(super) fn find_program(name: &str) -> Option<PathBuf> {
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
