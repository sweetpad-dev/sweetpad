//! Swift intent classes for legacy Siri intent definitions. The source model
//! remains a valid runtime resource; Xcode's private metadata upgrade is not
//! performed, so Siri registration is reported separately from app compilation.
#![allow(clippy::print_stdout, clippy::print_stderr)]
mod swift;
#[cfg(test)]
mod tests;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use sweetpad_lib::xcscheme::Element;

struct Args {
    generate: bool,
    dry_run: bool,
    input: PathBuf,
    output: PathBuf,
    prefix: String,
    visibility: String,
}
fn parse(args: &[String]) -> Result<Args, String> {
    let mut args = args.iter();
    let generate = match args.next().map(String::as_str) {
        Some("generate") => true,
        Some("compile") => false,
        _ => return Err("expected generate or compile".into()),
    };
    let mut parsed = Args {
        generate,
        dry_run: false,
        input: PathBuf::new(),
        output: PathBuf::new(),
        prefix: String::new(),
        visibility: "public".into(),
    };
    while let Some(flag) = args.next() {
        if flag == "-dryRun" {
            parsed.dry_run = true;
            continue;
        }
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "-input" => parsed.input = PathBuf::from(value),
            "-output" => parsed.output = PathBuf::from(value),
            "-classPrefix" => parsed.prefix.clone_from(value),
            "-visibility" => parsed.visibility.clone_from(value),
            "-language" if value == "Swift" => {}
            "-swiftVersion" | "-moduleName" => {}
            _ => return Err(format!("unsupported intent compiler option {flag}={value}")),
        }
    }
    if parsed.input.as_os_str().is_empty() || parsed.output.as_os_str().is_empty() {
        return Err("expected -input and -output".into());
    }
    if !parsed.prefix.is_empty() || parsed.visibility != "public" {
        return Err(
            "intent classes currently require an empty prefix and public visibility".into(),
        );
    }
    Ok(parsed)
}
#[must_use]
pub fn main(args: &[String]) -> ExitCode {
    match parse(args).and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
fn run(args: &Args) -> Result<(), String> {
    let root = sweetpad_lib::xcscheme::parse_file(&args.input).map_err(|e| e.to_string())?;
    let model = root
        .child("dict")
        .ok_or("an intent definition has no dictionary")?;
    let sources = swift::generate(model)?;
    if args.generate {
        for (name, source) in sources {
            let file = args.output.join(name);
            if args.dry_run {
                println!("{}", file.display());
            } else {
                write(&file, source.as_bytes())?;
            }
        }
    } else {
        let name = args
            .input
            .file_name()
            .ok_or("the intent definition has no name")?;
        let bytes = std::fs::read(&args.input).map_err(|e| e.to_string())?;
        write(&args.output.join(name), &bytes)?;
        eprintln!(
            "{}: warning: preserving the intent definition without Xcode's private Siri metadata upgrade; Siri registration requires separate validation",
            args.input.display()
        );
    }
    Ok(())
}
fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}
fn field<'a>(dictionary: &'a Element, key: &str) -> Option<&'a Element> {
    dictionary
        .children
        .chunks_exact(2)
        .find_map(|pair| (pair[0].name == "key" && pair[0].text == key).then_some(&pair[1]))
}
fn text<'a>(dictionary: &'a Element, key: &str) -> Result<&'a str, String> {
    field(dictionary, key)
        .filter(|e| e.name == "string")
        .map(|e| e.text.as_str())
        .ok_or_else(|| format!("missing intent string {key}"))
}
fn list<'a>(dictionary: &'a Element, key: &str) -> impl Iterator<Item = &'a Element> {
    field(dictionary, key)
        .into_iter()
        .flat_map(|e| e.children.iter())
}
fn yes(dictionary: &Element, key: &str) -> bool {
    field(dictionary, key).is_some_and(|v| v.name == "true")
}
fn identifier(value: &str) -> Result<&str, String> {
    if value.is_empty()
        || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || value.starts_with(|c: char| c.is_ascii_digit())
    {
        return Err(format!("unsupported Swift intent identifier {value:?}"));
    }
    Ok(value)
}
fn capitalized(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
