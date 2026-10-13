//! A stand-in for Xcode's `momc`, the Core Data model compiler, for Swift
//! Build on Linux, where there is no Xcode to run. The `sweetpad` binary
//! runs as this when it's started under the name `momc`, which Swift Build
//! looks up on the `PATH` the engine gives it.
//!
//! It does the two things Swift Build asks of momc:
//! - compiles a `.xcdatamodel` into a `.mom` file, or a `.xcdatamodeld`
//!   into a `.momd` folder of one for each version and a
//!   `VersionInfo.plist` with each entity's version hash (see
//!   [`compile`]);
//! - writes the classes of the entities whose code generation is Class
//!   Definition or Category/Extension, in Swift or Objective-C, and with
//!   `--dry-run` lists them (see [`codegen`]).
//!
//! A model with fetch request templates, fetched properties, derived or
//! composite attributes, predicates on its indexes, or a minimum or maximum
//! on a date, decimal or binary attribute is an error: those need an
//! `NSPredicate` parser this doesn't have.

// What this prints is momc's output, which Swift Build parses, so it
// doesn't go through `Output`.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod archive;
mod codegen;
mod compile;
mod model;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use compile::Quiet;
use model::Model;

#[derive(Debug)]
struct Args {
    generate: bool,
    dry_run: bool,
    quiet: Quiet,
    source: PathBuf,
    destination: PathBuf,
}

/// The options that take a value, which momc accepts after `=` or as the
/// next argument.
fn takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "--sdkroot" | "--action" | "--module" | "--swift-version"
    ) || (flag.starts_with("--") && flag.ends_with("-deployment-target"))
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args {
        generate: false,
        dry_run: false,
        quiet: Quiet::default(),
        source: PathBuf::new(),
        destination: PathBuf::new(),
    };
    let mut paths = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let Some(flag) = arg.strip_prefix("--").map(|_| arg.as_str()) else {
            paths.push(PathBuf::from(arg));
            continue;
        };
        let (flag, inline) = match flag.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (flag, None),
        };
        if takes_value(flag) {
            let value = match inline {
                Some(value) => value,
                None => it
                    .next()
                    .cloned()
                    .ok_or_else(|| format!("'{flag}' takes a value"))?,
            };
            if flag == "--action" {
                parsed.generate = match value.as_str() {
                    "compile" => false,
                    "generate" => true,
                    other => return Err(format!("unknown action '{other}'")),
                };
            }
            continue;
        }
        match flag {
            "--dry-run" => parsed.dry_run = true,
            "--no-warnings" => parsed.quiet.all = true,
            "--no-inverse-relationship-warnings" => parsed.quiet.inverse = true,
            "--no-delete-rule-warnings" => parsed.quiet.delete_rule = true,
            "--no-max-property-count-warnings"
            | "--suppress-inverse-transient-error"
            | "--print-diagnostic-categories" => {}
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    let [source, destination] = <[PathBuf; 2]>::try_from(paths)
        .map_err(|_| "takes the model and the output path".to_string())?;
    parsed.source = source;
    parsed.destination = destination;
    Ok(parsed)
}

#[must_use]
pub fn main(args: &[String]) -> ExitCode {
    let result = parse_args(args).and_then(|args| {
        if args.generate {
            let header = codegen::Header {
                author: author(),
                date: today(),
            };
            for path in generate(&args, &header)? {
                println!("{}", path.display());
            }
            Ok(true)
        } else {
            let (ok, log) = compile_model(&args)?;
            for line in log {
                eprintln!("{line}");
            }
            Ok(ok)
        }
    });
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("error: momc: {message}");
            ExitCode::FAILURE
        }
    }
}

/// `path` from the root, as Swift Build wants the generated files listed.
fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A model's versions, each its `.xcdatamodel` folder, and which is
/// current: for a `.xcdatamodeld`, the one its `.xccurrentversion` names,
/// else the last; a `.xcdatamodel` is its own only version.
fn versions(source: &Path) -> Result<(Vec<PathBuf>, Option<String>, PathBuf), String> {
    if source.extension().is_none_or(|e| e != "xcdatamodeld") {
        return Ok((vec![source.to_path_buf()], None, source.to_path_buf()));
    }
    let entries =
        std::fs::read_dir(source).map_err(|e| format!("can't read {}: {e}", source.display()))?;
    let mut versions: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "xcdatamodel"))
        .collect();
    versions.sort();
    let named = current_version(source);
    let current = named
        .as_deref()
        .map(|name| source.join(name))
        .filter(|p| versions.contains(p))
        .or_else(|| versions.last().cloned())
        .ok_or_else(|| format!("{} has no model versions", source.display()))?;
    let current_name = named.map(|_| stem(&current));
    Ok((versions, current_name, current))
}

/// The version `.xccurrentversion` names, a property list with
/// `_XCCurrentVersionName`.
fn current_version(source: &Path) -> Option<String> {
    let plist = sweetpad_lib::xcscheme::parse_file(&source.join(".xccurrentversion")).ok()?;
    let dict = plist.child("dict")?;
    let mut children = dict.children.iter();
    while let Some(child) = children.next() {
        if child.name == "key" && child.text == "_XCCurrentVersionName" {
            return children.next().map(|value| value.text.clone());
        }
    }
    None
}

fn load(version: &Path) -> Result<Model, String> {
    let contents = version.join("contents");
    let root = sweetpad_lib::xcscheme::parse_file(&contents)
        .map_err(|e| format!("can't read {}: {e}", contents.display()))?;
    model::parse(&root).map_err(|e| format!("{}: {e}", version.display()))
}

/// `model`'s diagnostics as momc prints them, and whether it has errors.
fn report(version: &Path, model: &Model, quiet: Quiet, log: &mut Vec<String>) -> bool {
    let diagnostics = compile::check(model, quiet);
    let at = absolute(version);
    for d in &diagnostics {
        let kind = if d.error { "error" } else { "warning" };
        log.push(format!(
            "{}:{}: {kind}: {} [{}]",
            at.display(),
            d.location,
            d.message,
            d.category
        ));
    }
    diagnostics.iter().any(|d| d.error)
}

/// Where the compiled model goes: in `destination` when it's a folder,
/// as momc's own name for it, else at `destination` itself.
fn output_path(destination: &Path, source: &Path, extension: &str) -> PathBuf {
    let as_folder = destination.as_os_str().to_string_lossy().ends_with('/')
        || destination.is_dir() && destination.extension().is_none_or(|e| e != extension);
    if as_folder {
        destination.join(format!("{}.{extension}", stem(source)))
    } else {
        destination.to_path_buf()
    }
}

/// Compile the model `args` names, and whether that worked, with what momc
/// prints about it.
fn compile_model(args: &Args) -> Result<(bool, Vec<String>), String> {
    let (paths, current, _) = versions(&args.source)?;
    let mut log = Vec::new();
    let mut models = Vec::new();
    let mut failed = false;
    for path in &paths {
        let model = load(path)?;
        failed |= report(path, &model, args.quiet, &mut log);
        models.push((path, model));
    }
    if failed {
        return Ok((false, log));
    }
    if args.source.extension().is_none_or(|e| e != "xcdatamodeld") {
        let out = output_path(&args.destination, &args.source, "mom");
        write(&out, &compile::archive(&models[0].1))?;
        return Ok((true, log));
    }
    let folder = output_path(&args.destination, &args.source, "momd");
    let mut infos = Vec::new();
    let mut notes = Vec::new();
    for (path, model) in &models {
        let version = stem(path);
        write(
            &folder.join(format!("{version}.mom")),
            &compile::archive(model),
        )?;
        let hashes = compile::version_hashes(model);
        notes.push(format!(
            "{}: note: Model {version} version checksum: {}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            compile::checksum(&hashes)
        ));
        infos.push((version, hashes));
    }
    write(
        &folder.join("VersionInfo.plist"),
        &compile::version_info(&infos, current.as_deref()),
    )?;
    // momc gives each version's checksum ahead of its warnings.
    notes.extend(log);
    Ok((true, notes))
}

/// Generate the classes of the model `args` names, or with `--dry-run`
/// only name them; either way, the files' paths.
fn generate(args: &Args, header: &codegen::Header) -> Result<Vec<PathBuf>, String> {
    let (_, _, current) = versions(&args.source)?;
    let model = load(&current)?;
    let files = codegen::files(&model, &stem(&args.source), header);
    let folder = absolute(&args.destination);
    let mut paths = Vec::new();
    for (name, text) in files {
        let path = folder.join(name);
        if !args.dry_run {
            write(&path, text.as_bytes())?;
        }
        paths.push(path);
    }
    Ok(paths)
}

fn write(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, contents).map_err(|e| format!("can't write {}: {e}", path.display()))
}

/// Who the generated files say made them: the user's name, as momc takes
/// it from their account.
fn author() -> String {
    ["USER", "LOGNAME"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "momc".to_string())
}

/// Today's date, the way the generated files' header gives it.
fn today() -> String {
    // SAFETY: `time` with a null pointer only returns the time, and
    // `localtime_r` fills the caller-owned `tm` from it.
    let tm = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&raw const t, &raw mut tm);
        tm
    };
    format!(
        "{}/{}/{:02}",
        tm.tm_mon + 1,
        tm.tm_mday,
        (tm.tm_year + 1900) % 100
    )
}
