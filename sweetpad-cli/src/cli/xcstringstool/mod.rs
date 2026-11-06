//! A stand-in for Xcode's `xcstringstool`, for Swift Build on Linux, where
//! there is no Xcode to run. The `sweetpad` binary runs as this when it's
//! started under the name `xcstringstool`, which the Swift Build engine puts
//! on Swift Build's `PATH`.
//!
//! It does the two things Swift Build asks of the tool for a String Catalog
//! (`.xcstrings`), writing what Xcode's does:
//! - `compile`: each language's `.strings` and `.stringsdict` tables, and
//!   with `--dry-run` the list of them, which Swift Build plans the build
//!   with (see [`compile`]);
//! - `generate-symbols --language swift`: the `LocalizedStringResource`
//!   symbols for strings added by hand (see [`symbols`]).
//!
//! A string Xcode's tool refuses is an error here too, printed the way
//! Swift Build reads the tool's errors.

// What this prints is xcstringstool's output, which Swift Build parses, so
// it doesn't go through `Output`.
#![allow(clippy::print_stdout, clippy::print_stderr)]

mod compile;
mod format;
mod symbols;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Map, Value};

use super::xml_plist;

/// A String Catalog.
#[derive(Debug)]
struct Catalog {
    source_language: String,
    /// Each string by key.
    strings: Map<String, Value>,
}

impl Catalog {
    fn parse(text: &str) -> Result<Catalog, String> {
        let value: Value = serde_json::from_str(text).map_err(|_| {
            "The data couldn’t be read because it isn’t in the correct format.".to_string()
        })?;
        let field = |key: &str| {
            value
                .get(key)
                .ok_or_else(|| format!("Missing required key '{key}'"))
        };
        field("version")?;
        let source_language = field("sourceLanguage")?
            .as_str()
            .unwrap_or_default()
            .to_string();
        let strings = field("strings")?.as_object().cloned().unwrap_or_default();
        Ok(Catalog {
            source_language,
            strings,
        })
    }
}

/// A string Xcode's tool refuses: the language it's in (or `allLocales`),
/// its key, the variation it's in, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Problem {
    scope: String,
    key: String,
    path: Option<String>,
    message: String,
}

impl Problem {
    /// The problem the way Swift Build reads it: `<catalog>: error: <message>
    /// (<language>: <key>)`.
    fn line(&self, catalog: &Path) -> String {
        let at = self
            .path
            .as_ref()
            .map_or_else(String::new, |path| format!("|==|{path}"));
        format!(
            "{}: error: {} ({}: {}{at})",
            catalog.display(),
            self.message,
            self.scope,
            self.key
        )
    }
}

/// The command line: the subcommand, its options, and the catalog.
#[derive(Debug, Default)]
struct Args {
    subcommand: String,
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    languages: Vec<String>,
    dry_run: bool,
    force_all: bool,
}

impl Args {
    fn parse(command_line: &[String]) -> Result<Args, String> {
        let mut args = Args::default();
        let mut it = command_line.iter();
        args.subcommand = it.next().cloned().unwrap_or_default();
        while let Some(arg) = it.next() {
            let mut value = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| format!("Missing value for '{arg}'"))
            };
            match arg.as_str() {
                "-o" | "--output-directory" => args.output = Some(PathBuf::from(value()?)),
                "-l" | "--language" => args.languages.push(value()?),
                "--dry-run" => args.dry_run = true,
                "--force-build-all-strings" => args.force_all = true,
                "-f" | "--format" => expect(arg, &value()?, "stringsAndStringsdict")?,
                "--serialization-format" => expect(arg, &value()?, "text")?,
                flag if flag.starts_with('-') => return Err(format!("Unknown option '{flag}'")),
                input => args.input = Some(PathBuf::from(input)),
            }
        }
        Ok(args)
    }
}

/// Only the value Swift Build passes, if it passes one, is supported.
fn expect(flag: &str, value: &str, supported: &str) -> Result<(), String> {
    if value == supported {
        Ok(())
    } else {
        Err(format!(
            "'{flag} {value}' isn't supported, only '{supported}'"
        ))
    }
}

#[must_use]
pub fn main(command_line: &[String]) -> ExitCode {
    let result = Args::parse(command_line).and_then(|args| {
        let input = args
            .input
            .clone()
            .ok_or("Missing expected argument '<input-file>'")?;
        let output = args
            .output
            .clone()
            .ok_or("Missing expected argument '--output-directory <output-directory>'")?;
        let text = std::fs::read_to_string(&input)
            .map_err(|e| format!("{}: error: couldn't read the file: {e}", input.display()))?;
        let catalog =
            Catalog::parse(&text).map_err(|e| format!("{}: error: {e}", input.display()))?;
        let table = input
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let problems = match args.subcommand.as_str() {
            "compile" => compile_catalog(&catalog, &args, &table, &output)?,
            "generate-symbols" => generate_symbols(&catalog, &args, &table, &output)?,
            other => return Err(format!("Unknown subcommand '{other}'")),
        };
        let mut lines: Vec<String> = problems.iter().map(|p| p.line(&input)).collect();
        lines.sort();
        for line in &lines {
            eprintln!("{line}");
        }
        Ok(lines.is_empty())
    });
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            if message.contains(": error: ") {
                eprintln!("{message}");
            } else {
                eprintln!("Error: {message}");
            }
            ExitCode::FAILURE
        }
    }
}

fn compile_catalog(
    catalog: &Catalog,
    args: &Args,
    table: &str,
    output: &Path,
) -> Result<Vec<Problem>, String> {
    let options = compile::Options {
        languages: args.languages.clone(),
        force_all: args.force_all,
    };
    let path = |language: &str, extension: &str| {
        output
            .join(format!("{language}.lproj"))
            .join(format!("{table}.{extension}"))
    };
    if args.dry_run {
        for (language, strings, dict) in compile::planned(catalog, &options) {
            if strings {
                println!("{}", path(&language, "strings").display());
            }
            if dict {
                println!("{}", path(&language, "stringsdict").display());
            }
        }
        return Ok(Vec::new());
    }
    let compiled = compile::compile(catalog, &options);
    for (language, tables) in compiled.languages {
        for (extension, entries) in [("strings", tables.strings), ("stringsdict", tables.dict)] {
            if !entries.is_empty() {
                write(
                    &path(&language, extension),
                    &xml_plist::of(&Value::Object(entries)),
                )?;
            }
        }
    }
    Ok(compiled.problems)
}

fn generate_symbols(
    catalog: &Catalog,
    args: &Args,
    table: &str,
    output: &Path,
) -> Result<Vec<Problem>, String> {
    if let Some(language) = args.languages.iter().find(|l| *l != "swift") {
        return Err(format!(
            "'--language {language}' isn't supported, only 'swift'"
        ));
    }
    let (swift, problems) = symbols::generate(catalog, table);
    write(
        &output.join(format!("GeneratedStringSymbols_{table}.swift")),
        &swift,
    )?;
    Ok(problems)
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, contents).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests;
