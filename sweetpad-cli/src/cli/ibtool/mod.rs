//! A stand-in for Xcode's `ibtool`, the Interface Builder compiler, for
//! Swift Build on Linux, where there is no Xcode to run. The `sweetpad`
//! binary runs as this when it's started under the name `ibtool`, which
//! Swift Build looks up on the `PATH` the engine gives it.
//!
//! It does what Swift Build asks of ibtool for an iOS storyboard:
//! - compiles it into a `.storyboardc` folder of nibs, in Apple's
//!   NIBArchive format (see [`storyboard`] and [`nib`]);
//! - links the compiled storyboards into the product, which copies them.
//!
//! It knows plain view controllers whose views are made of views, labels
//! and image views laid out with Auto Layout, which is what a launch screen
//! is. Anything else in a document is an error naming it, and so is a XIB.

// What this prints is ibtool's output, which Swift Build parses, so it
// doesn't go through `Output`.
#![allow(clippy::print_stdout)]

mod nib;
mod storyboard;
#[cfg(test)]
mod tests;
mod uikit;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// What Swift Build asks for.
#[derive(Debug, Default, PartialEq)]
struct Args {
    compilation_directory: Option<PathBuf>,
    compile: Option<PathBuf>,
    link: Option<PathBuf>,
    partial_info_plist: Option<PathBuf>,
    inputs: Vec<PathBuf>,
}

/// The options that take a value; only some of them change anything here.
const VALUED: &[&str] = &[
    "--module",
    "--target-device",
    "--minimum-deployment-target",
    "--output-format",
    "--output-partial-info-plist",
    "--compilation-directory",
    "--compile",
    "--link",
    "--platform",
    "--companion-strings-file",
    "--flatten",
    "--sdk",
];

/// The options that take none and change nothing here.
const FLAGS: &[&str] = &[
    "--errors",
    "--warnings",
    "--notices",
    "--auto-activate-custom-fonts",
    "--strip",
];

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        if VALUED.contains(&name) {
            let value = match inline {
                Some(value) => value,
                None => rest
                    .next()
                    .cloned()
                    .ok_or_else(|| format!("{name} needs a value"))?,
            };
            let path = Some(PathBuf::from(&value));
            match name {
                "--compilation-directory" => parsed.compilation_directory = path,
                "--compile" => parsed.compile = path,
                "--link" => parsed.link = path,
                "--output-partial-info-plist" => parsed.partial_info_plist = path,
                _ => {}
            }
        } else if FLAGS.contains(&name) {
        } else if name.starts_with("--") {
            return Err(format!("unknown option {name}"));
        } else {
            parsed.inputs.push(PathBuf::from(arg));
        }
    }
    Ok(parsed)
}

#[must_use]
pub fn main(args: &[String]) -> ExitCode {
    let args = match parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            println!("/* com.apple.ibtool.errors */\nerror: {message}");
            return ExitCode::FAILURE;
        }
    };
    let mut failed = false;
    if let Some(destination) = &args.link {
        for input in &args.inputs {
            if let Err(message) = link(input, destination) {
                report(input, &message);
                failed = true;
            }
        }
    } else {
        for input in &args.inputs {
            if let Err(message) = compile(input, &args) {
                report(input, &message);
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn report(input: &Path, message: &str) {
    println!(
        "/* com.apple.ibtool.errors */\n{}: error: {message}",
        absolute(input).display()
    );
}

fn absolute(path: &Path) -> PathBuf {
    std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
}

fn compile(input: &Path, args: &Args) -> Result<(), String> {
    let extension = input.extension().and_then(|e| e.to_str()).unwrap_or("");
    if extension == "xib" {
        return Err("SweetPad's ibtool doesn't compile XIBs yet".to_string());
    }
    if extension != "storyboard" {
        return Err(format!("ibtool can't compile a .{extension} file"));
    }
    let directory = args
        .compilation_directory
        .as_ref()
        .ok_or("a storyboard compiles into a --compilation-directory")?;
    let document = sweetpad_lib::xcscheme::parse_file(input).map_err(|e| e.to_string())?;
    let compiled = storyboard::compile(&document)?;
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Storyboard");
    let out = directory.join(format!("{stem}.storyboardc"));
    let write = |path: &Path, bytes: &[u8]| {
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    };
    if out.exists() {
        std::fs::remove_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    for (name, bytes) in &compiled.nibs {
        write(&out.join(name), bytes)?;
    }
    write(&out.join("Info.plist"), &compiled.info)?;
    if let Some(path) = &args.partial_info_plist {
        write(
            path,
            &crate::cli::keyed_archive::write(&crate::cli::keyed_archive::Plist::Dict(Vec::new())),
        )?;
    }
    Ok(())
}

/// Copies a compiled storyboard into the product, into the `.lproj`
/// folder it was compiled into, if any.
fn link(input: &Path, destination: &Path) -> Result<(), String> {
    let name = input
        .file_name()
        .ok_or_else(|| format!("{} isn't a compiled storyboard", input.display()))?;
    let localized = input
        .parent()
        .and_then(Path::file_name)
        .filter(|parent| Path::new(parent).extension().is_some_and(|e| e == "lproj"));
    let target = match localized {
        Some(lproj) => destination.join(lproj).join(name),
        None => destination.join(name),
    };
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    copy_dir(input, &target)
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            std::fs::copy(&path, &target).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}
