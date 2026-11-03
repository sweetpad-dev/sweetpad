//! A stand-in for Xcode's `ibtool`, the Interface Builder compiler, for
//! Swift Build on Linux, where there is no Xcode to run. The `sweetpad`
//! binary runs as this when it's started under the name `ibtool`, which
//! Swift Build looks up on the `PATH` the engine gives it.
//!
//! It implements Swift Build's version, compile and link operations for iOS
//! storyboards and XIBs, using Apple's NIBArchive format. The UIKit encoders
//! cover the controls, containers, fonts, colors, connections and layout in
//! the iOS corpus. Unsupported object types and connections report errors.

// What this prints is ibtool's output, which Swift Build parses, so it
// doesn't go through `Output`.
#![allow(clippy::print_stdout)]

mod appkit;
mod localization;
mod nib;
mod output;
mod package;
mod storyboard;
#[cfg(test)]
mod tests;
mod uikit;
mod watch;
mod xib;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// What Swift Build asks for.
#[derive(Debug, Default, PartialEq)]
struct Args {
    version: bool,
    module: String,
    compilation_directory: Option<PathBuf>,
    compile: Option<PathBuf>,
    link: Option<PathBuf>,
    partial_info_plist: Option<PathBuf>,
    inputs: Vec<PathBuf>,
    companion_strings: Vec<(String, PathBuf)>,
    reference_external_strings: bool,
    flatten: Option<bool>,
    strip: Option<PathBuf>,
    output_format: output::Format,
    diagnostic_categories: Vec<String>,
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
    "--strip",
    "--sdk",
];

/// The options that take none and change nothing here.
const FLAGS: &[&str] = &[
    "--errors",
    "--warnings",
    "--notices",
    "--auto-activate-custom-fonts",
    "--reference-external-strings-file",
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
                "--module" => parsed.module = value,
                "--output-format" => parsed.output_format = output::Format::parse(&value)?,
                "--strip" => parsed.strip = path,
                "--flatten" => {
                    parsed.flatten = Some(match value.as_str() {
                        "YES" => true,
                        "NO" => false,
                        _ => return Err("--flatten expects YES or NO".into()),
                    });
                }
                "--companion-strings-file" => {
                    let (locale, path) = value
                        .split_once(':')
                        .ok_or("expected locale:path for --companion-strings-file")?;
                    localization::validate_locale(locale)?;
                    parsed
                        .companion_strings
                        .push((locale.to_string(), PathBuf::from(path)));
                    parsed.reference_external_strings = true;
                }
                _ => {}
            }
        } else if name == "--version" {
            parsed.version = true;
        } else if name == "--reference-external-strings-file" {
            parsed.reference_external_strings = true;
        } else if matches!(name, "--errors" | "--warnings" | "--notices") {
            parsed
                .diagnostic_categories
                .push(name.trim_start_matches("--").to_string());
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
            output::error(output::requested_format(args), None, &message);
            return ExitCode::FAILURE;
        }
    };
    if args.version {
        return if output::version(args.output_format).is_ok() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    if args.inputs.is_empty() {
        output::error(args.output_format, None, "no input document");
        return ExitCode::FAILURE;
    }
    let errors = execute(&args);
    if !errors.is_empty() {
        let _ = output::emit(
            args.output_format,
            &serde_json::json!({"com.apple.ibtool.errors":errors}),
        );
        ExitCode::FAILURE
    } else if output::success(
        args.output_format,
        &args.diagnostic_categories,
        serde_json::Map::new(),
    )
    .is_ok()
    {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn execute(args: &Args) -> Vec<serde_json::Value> {
    let mut errors = Vec::new();
    if let Some(destination) = &args.strip {
        if args.inputs.len() != 1 {
            errors.push(output::error_value(
                Some(destination),
                "--strip requires exactly one input",
            ));
        } else if let Err(message) = package::strip(&args.inputs[0], destination) {
            errors.push(output::error_value(Some(&args.inputs[0]), &message));
        }
        return errors;
    }
    for input in &args.inputs {
        let result = if let Some(destination) = &args.link {
            link(input, destination)
        } else {
            compile(input, args)
        };
        if let Err(message) = result {
            errors.push(output::error_value(Some(input), &message));
        }
    }
    errors
}

fn absolute(path: &Path) -> PathBuf {
    std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
}

fn compile(input: &Path, args: &Args) -> Result<(), String> {
    let extension = input.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !matches!(extension, "storyboard" | "xib" | "nib" | "storyboardc") {
        return Err(format!("ibtool can't compile a .{extension} file"));
    }
    let source = package::source(input)?;
    let document = sweetpad_lib::xcscheme::parse_file(&source).map_err(|e| e.to_string())?;
    let extension = if matches!(extension, "nib" | "storyboardc") {
        if document.child("scenes").is_some() {
            "storyboard"
        } else {
            "xib"
        }
    } else {
        extension
    };
    let options = Options {
        module: &args.module,
        localized: args.reference_external_strings
            || input
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "Base.lproj"),
    };
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Storyboard");
    if args.compile.is_none() && args.compilation_directory.is_none() {
        if extension == "xib" {
            xib::compile(&document, &options)?;
        } else if document.attr("targetRuntime") == Some("watchKit") {
            watch::compile(&document, &options)?;
        } else {
            storyboard::compile(&document, &options)?;
        }
        return Ok(());
    }
    let out = args
        .compile
        .clone()
        .or_else(|| {
            args.compilation_directory.as_ref().map(|dir| {
                dir.join(format!(
                    "{stem}.{}",
                    if extension == "xib" {
                        "nib"
                    } else {
                        "storyboardc"
                    }
                ))
            })
        })
        .ok_or("expected --compile or --compilation-directory")?;
    let write = |path: &Path, bytes: &[u8]| {
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    };
    let design_source = if args.flatten == Some(false) {
        std::fs::read(&source).map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    if extension == "xib" {
        let compiled = xib::compile(&document, &options)?;
        package::write_xib(
            &out,
            &compiled,
            &design_source,
            args.flatten.unwrap_or(true),
        )?;
        localization::copy_xib_companions(&out, &args.companion_strings)?;
    } else {
        let compiled = if document.attr("targetRuntime") == Some("watchKit") {
            watch::compile(&document, &options)?
        } else {
            storyboard::compile(&document, &options)?
        };
        if out.exists() {
            std::fs::remove_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        }
        std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        for (name, bytes) in &compiled.nibs {
            write(&out.join(name), bytes)?;
        }
        if !compiled.info.is_empty() {
            write(&out.join("Info.plist"), &compiled.info)?;
        }
        localization::write_storyboard_companions(&out, &args.companion_strings)?;
        if args.flatten == Some(false) {
            std::fs::write(out.join("designable.storyboard"), &design_source)
                .map_err(|e| e.to_string())?;
        }
    }
    if let Some(path) = &args.partial_info_plist {
        write(
            path,
            &crate::cli::keyed_archive::write(&crate::cli::keyed_archive::Plist::Dict(Vec::new())),
        )?;
    }
    Ok(())
}

#[derive(Default)]
pub(super) struct Options<'a> {
    pub module: &'a str,
    pub localized: bool,
}

/// The runtime name Xcode gives a custom class. A target module overrides
/// the module saved in the document, so renamed targets still load it.
fn custom_class(
    element: &sweetpad_lib::xcscheme::Element,
    options: &Options<'_>,
) -> Option<String> {
    let name = element.attr("customClass")?;
    let module = if element.attr("customModuleProvider") == Some("target") {
        Some(options.module)
    } else {
        element.attr("customModule")
    };
    Some(module.filter(|m| !m.is_empty()).map_or_else(
        || name.to_string(),
        |m| format!("_TtC{}{m}{}{name}", m.len(), name.len()),
    ))
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
    if input
        .canonicalize()
        .ok()
        .is_some_and(|path| target.canonicalize().ok().as_ref() == Some(&path))
    {
        return Ok(());
    }
    if input.is_file() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(input, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        return Ok(());
    }
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    copy_dir(input, &target)?;
    localization::link_companions(input, &target, destination)
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
