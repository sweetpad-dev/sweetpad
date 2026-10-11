//! A stand-in for Xcode's `actool`, for Swift Build on Linux, where there is
//! no Xcode to run. The `sweetpad` binary runs as this when it's started
//! under the name `actool`, which is how the Swift Build engine hands it to
//! Swift Build (`ASSETCATALOG_EXEC`).
//!
//! It answers what Swift Build asks of actool:
//! - its version;
//! - the catalogs' asset tags, which it leaves empty;
//! - the asset symbols (`GeneratedAssetSymbols.swift` and the rest), written
//!   the way actool writes them, so code that uses `Color(.brand)` compiles;
//! - a compiled catalog: no `Assets.car`, since its format is Apple's own,
//!   but the partial Info.plist and dependency file Swift Build links in,
//!   and a warning naming what the app goes without.
//!
//! A catalog with nothing to compile, like the one Xcode's app template
//! starts with, comes out the same as from actool.

// What this prints is actool's output, which Swift Build parses, so it
// doesn't go through `Output`.
#![allow(clippy::print_stdout)]

mod catalog;
mod symbols;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use catalog::{Catalog, Kind};
use symbols::{Frameworks, Options};

/// The flags that take no value.
const SWITCHES: &[&str] = &[
    "--version",
    "--print-asset-tag-combinations",
    "--notices",
    "--warnings",
    "--errors",
    "--compress-pngs",
    "--include-all-app-icons",
    "--skip-app-store-deployment",
    "--include-sticker-content",
];

/// The command line, as flags and the catalogs they apply to.
#[derive(Debug, Default)]
struct Args {
    inputs: Vec<PathBuf>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    fn parse(command_line: &[String]) -> Args {
        let mut args = Args::default();
        let mut it = command_line.iter().peekable();
        while let Some(arg) = it.next() {
            if !arg.starts_with("--") {
                args.inputs.push(PathBuf::from(arg));
                continue;
            }
            let value = if SWITCHES.contains(&arg.as_str())
                || it.peek().is_none_or(|next| next.starts_with("--"))
            {
                None
            } else {
                it.next().cloned()
            };
            args.flags.push((arg.clone(), value));
        }
        args
    }

    fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|(f, _)| f == flag)
    }

    fn value(&self, flag: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(f, _)| f == flag)
            .and_then(|(_, v)| v.as_deref())
    }

    fn values(&self, flag: &str) -> Vec<&str> {
        self.flags
            .iter()
            .filter(|(f, _)| f == flag)
            .filter_map(|(_, v)| v.as_deref())
            .collect()
    }
}

#[must_use]
pub fn main(command_line: &[String]) -> ExitCode {
    let args = Args::parse(command_line);
    let result = if args.has("--version") {
        print!("{}", version());
        Ok(())
    } else if args.has("--print-asset-tag-combinations") {
        print!(
            "{}",
            plist(
                "<dict>\n\t<key>com.apple.actool.asset-tag-combinations</key>\n\t<array/>\n</dict>"
            )
        );
        Ok(())
    } else if args.has("--generate-swift-asset-symbols")
        || args.has("--generate-objc-asset-symbols")
        || args.has("--generate-asset-symbol-index")
    {
        generate_symbols(&args)
    } else if args.has("--compile") {
        compile(&args)
    } else {
        Err("nothing to do: expected --compile, --version or a --generate option".to_string())
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            println!("/* com.apple.actool.errors */\nerror: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Xcode 27's actool version, so Swift Build plans the same command lines
/// it would for Xcode's.
fn version() -> String {
    plist(
        "<dict>\n\t<key>com.apple.actool.version</key>\n\t<dict>\n\
         \t\t<key>bundle-version</key>\n\t\t<string>25098</string>\n\
         \t\t<key>short-bundle-version</key>\n\t\t<string>27.0</string>\n\t</dict>\n</dict>",
    )
}

fn plist(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n{body}\n</plist>\n"
    )
}

fn catalogs(args: &Args) -> Vec<Catalog> {
    args.inputs.iter().map(|path| Catalog::read(path)).collect()
}

fn generate_symbols(args: &Args) -> Result<(), String> {
    let frameworks = args.values("--generate-asset-symbol-framework-support");
    let options = Options {
        bundle_id: args
            .value("--bundle-identifier")
            .unwrap_or_default()
            .to_string(),
        extensions: args.value("--generate-swift-asset-symbol-extensions") != Some("NO"),
        frameworks: if frameworks.is_empty() {
            Frameworks::default()
        } else {
            Frameworks {
                appkit: frameworks.contains(&"AppKit"),
                uikit: frameworks.contains(&"UIKit"),
                swiftui: frameworks.contains(&"SwiftUI"),
            }
        },
        target_devices: target_devices(args),
    };
    let symbols = symbols::generate(&catalogs(args), &options);
    let mut written = Vec::new();
    for (flag, contents) in [
        ("--generate-swift-asset-symbols", symbols.swift),
        ("--generate-objc-asset-symbols", symbols.objc),
        ("--generate-asset-symbol-index", index_plist(&symbols.index)),
    ] {
        if let Some(path) = args.value(flag) {
            write(Path::new(path), contents.as_bytes())?;
            written.push(path);
        }
    }
    println!("/* com.apple.actool.compilation-results */");
    for path in written {
        println!("{path}");
    }
    println!();
    Ok(())
}

/// The devices the catalog is compiled for, from `--target-device`, or the
/// platform's own when there are none.
fn target_devices(args: &Args) -> Vec<String> {
    let devices = args.values("--target-device");
    if !devices.is_empty() {
        return devices.into_iter().map(str::to_string).collect();
    }
    let platform = args.value("--platform").unwrap_or_default();
    let defaults: &[&str] = match platform {
        "macosx" => &["mac"],
        "appletvos" | "appletvsimulator" => &["tv"],
        "watchos" | "watchsimulator" => &["watch"],
        "xros" | "xrsimulator" => &["vision"],
        _ => &["iphone", "ipad"],
    };
    defaults.iter().map(ToString::to_string).collect()
}

/// A compile: everything Swift Build links into the product except the
/// `Assets.car`, and a warning for each catalog that had something to put
/// in one.
fn compile(args: &Args) -> Result<(), String> {
    let output = args
        .value("--compile")
        .ok_or("--compile needs a directory")?;
    std::fs::create_dir_all(output).map_err(|e| format!("couldn't create {output}: {e}"))?;
    if let Some(path) = args.value("--output-partial-info-plist") {
        write(Path::new(path), plist("<dict/>").as_bytes())?;
    }
    if let Some(path) = args.value("--export-dependency-info") {
        write(Path::new(path), &dependency_info(&args.inputs))?;
    }
    let warnings: Vec<String> = catalogs(args)
        .iter()
        .filter_map(|catalog| {
            let (missing, plural) = missing(catalog)?;
            Some(format!(
                "{}: warning: Linux builds don't compile asset catalogs yet, so {missing} {} missing from the app.",
                catalog.path.display(),
                if plural { "are" } else { "is" },
            ))
        })
        .collect();
    if !warnings.is_empty() {
        println!("/* com.apple.actool.document.warnings */");
        for warning in warnings {
            println!("{warning}");
        }
    }
    println!("/* com.apple.actool.compilation-results */");
    println!();
    Ok(())
}

/// What a catalog would have compiled, in words (`the app icon, 2 colors and
/// 1 image`) and whether that's more than one thing, or nothing when actool
/// would have compiled nothing either.
fn missing(catalog: &Catalog) -> Option<(String, bool)> {
    let compiled = catalog.compiled();
    let count = |kind: Kind| compiled.iter().filter(|a| a.kind == kind).count();
    let counted = |n: usize, one: &str, many: &str| match n {
        0 => None,
        1 => Some((format!("1 {one}"), 1)),
        n => Some((format!("{n} {many}"), n)),
    };
    let parts: Vec<(String, usize)> = [
        (count(Kind::AppIcon) > 0).then(|| ("the app icon".to_string(), 1)),
        counted(count(Kind::Color), "color", "colors"),
        counted(count(Kind::Image), "image", "images"),
        counted(count(Kind::Other), "other asset", "other assets"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let plural = parts.len() > 1 || parts.first().is_some_and(|(_, n)| *n > 1);
    let words: Vec<&str> = parts.iter().map(|(text, _)| text.as_str()).collect();
    let text = match words.as_slice() {
        [] => return None,
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    Some((text, plural))
}

/// ld64's dependency-info format, which Swift Build reads to know what the
/// compile depended on: a version record, then one record per input.
fn dependency_info(inputs: &[PathBuf]) -> Vec<u8> {
    let mut out = vec![0x00];
    out.extend_from_slice(b"sweetpad-actool\0");
    for input in inputs {
        out.push(0x10);
        out.extend_from_slice(input.display().to_string().as_bytes());
        out.push(0);
    }
    out
}

/// The symbol index as the XML property list actool writes.
fn index_plist(index: &serde_json::Value) -> String {
    fn escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    fn render(value: &serde_json::Value, indent: usize, out: &mut String) {
        let pad = "\t".repeat(indent);
        match value {
            serde_json::Value::Object(map) => {
                let _ = writeln!(out, "{pad}<dict>");
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                for key in keys {
                    let _ = writeln!(out, "{pad}\t<key>{}</key>", escape(key));
                    render(&map[key], indent + 1, out);
                }
                let _ = writeln!(out, "{pad}</dict>");
            }
            serde_json::Value::Array(items) if items.is_empty() => {
                let _ = writeln!(out, "{pad}<array/>");
            }
            serde_json::Value::Array(items) => {
                let _ = writeln!(out, "{pad}<array>");
                for item in items {
                    render(item, indent + 1, out);
                }
                let _ = writeln!(out, "{pad}</array>");
            }
            other => {
                let text = other
                    .as_str()
                    .map_or_else(|| other.to_string(), str::to_string);
                let _ = writeln!(out, "{pad}<string>{}</string>", escape(&text));
            }
        }
    }
    let mut body = String::new();
    render(index, 0, &mut body);
    plist(body.trim_end())
}

fn write(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, contents).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests;
