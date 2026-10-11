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
//!   but the app icon and images as loose PNGs, the partial Info.plist and
//!   dependency file Swift Build links in, and a warning naming what the
//!   app goes without (see [`compile`]).
//!
//! A catalog with nothing to compile, like the one Xcode's app template
//! starts with, comes out the same as from actool.

// What this prints is actool's output, which Swift Build parses, so it
// doesn't go through `Output`.
#![allow(clippy::print_stdout)]

mod catalog;
mod compile;
mod raster;
mod symbols;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use catalog::Catalog;
use symbols::{Frameworks, Options};

use super::xml_plist;

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
            xml_plist::document(
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
        compile::compile(&args)
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
    xml_plist::document(
        "<dict>\n\t<key>com.apple.actool.version</key>\n\t<dict>\n\
         \t\t<key>bundle-version</key>\n\t\t<string>25098</string>\n\
         \t\t<key>short-bundle-version</key>\n\t\t<string>27.0</string>\n\t</dict>\n</dict>",
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
        (
            "--generate-asset-symbol-index",
            xml_plist::of(&symbols.index),
        ),
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

fn write(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, contents).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests;
