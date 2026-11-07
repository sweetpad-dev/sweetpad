//! LLVM builds the DWARF. This adapter supplies the bundle metadata that
//! LLVM reads through Core Foundation only on Darwin, and omits duplicate
//! SDK interfaces while retaining interfaces owned by the application.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use sweetpad_lib::{derived_data::plist_strings, xcscheme};

pub(super) fn main(args: &[String]) -> ExitCode {
    let result = run(args);
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: dsymutil: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<u8, String> {
    let tool = backend().ok_or("no LLVM dsymutil beside the SDK linker or on PATH")?;
    let status = Command::new(tool)
        .args(args)
        .status()
        .map_err(|e| e.to_string())?;
    let code = status
        .code()
        .and_then(|c| u8::try_from(c).ok())
        .unwrap_or(1);
    if code == 0
        && let Some((input, output)) = bundle_output(args)?
    {
        repair_metadata(&input, &output)?;
        let roots: Vec<_> = std::env::var_os("XCODE_EXTRA_PLATFORM_FOLDERS")
            .iter()
            .flat_map(std::env::split_paths)
            .filter_map(|p| {
                p.parent()
                    .filter(|p| p.ends_with("Developer"))
                    .map(Path::to_path_buf)
            })
            .collect();
        omit_sdk_interfaces(&output, &roots)?;
    }
    Ok(code)
}

fn backend() -> Option<PathBuf> {
    let beside = std::env::args_os()
        .next()
        .map(PathBuf::from)
        .and_then(|p| p.parent().map(|p| p.join(super::SDK_DSYMUTIL)))
        .and_then(|p| p.canonicalize().ok());
    if beside.is_some() {
        return beside;
    }
    let me = std::env::current_exe().ok()?.canonicalize().ok()?;
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|p| p.join("dsymutil"))
        .find(|p| p.is_file() && p.canonicalize().ok().as_ref() != Some(&me))
}

/// Swift Build emits one bundle per invocation. Inspection, flat output,
/// and multi-input command lines keep LLVM's behavior without modification.
fn bundle_output(args: &[String]) -> Result<Option<(PathBuf, PathBuf)>, String> {
    let mut inputs = Vec::new();
    let mut output = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-f" | "--flat" | "-S" | "--no-output" | "--dump-debug-map" | "-s" | "--symtab"
            | "--help" | "-h" | "--version" | "-v" | "--update" | "-u" => return Ok(None),
            "-o" | "--out" => output = Some(PathBuf::from(super::value(arg, &mut it)?)),
            "--arch"
            | "--accelerator"
            | "--linker"
            | "--toolchain"
            | "-j"
            | "--num-threads"
            | "--oso-prepend-path"
            | "--object-prefix-map"
            | "--remarks-prepend-path"
            | "--remarks-output-format"
            | "--reproducer"
            | "--use-reproducer"
            | "--build-variant-suffix"
            | "--verify-dwarf"
            | "--embed-resource"
            | "-D"
            | "-cas"
            | "-cas-plugin-path"
            | "-cas-plugin-option" => {
                super::value(arg, &mut it)?;
            }
            "--" => {
                inputs.extend(it.map(PathBuf::from));
                break;
            }
            other if other.starts_with("--out=") => output = Some(PathBuf::from(&other[6..])),
            other if other.starts_with('-') => {}
            path => inputs.push(PathBuf::from(path)),
        }
    }
    if inputs.len() != 1 {
        return Ok(None);
    }
    let input = inputs.remove(0);
    let output = output.unwrap_or_else(|| PathBuf::from(format!("{}.dSYM", input.display())));
    Ok(output
        .join("Contents/Info.plist")
        .is_file()
        .then_some((input, output)))
}

fn repair_metadata(input: &Path, output: &Path) -> Result<(), String> {
    let own = input.ancestors().skip(1).find_map(|parent| {
        [
            parent.join("Info.plist"),
            parent.join("Contents/Info.plist"),
            parent.join("Resources/Info.plist"),
        ]
        .into_iter()
        .map(|p| plist_strings(&p))
        .find(|entries| {
            entries
                .iter()
                .any(|(k, v)| k == "CFBundleIdentifier" && !v.is_empty())
        })
    });
    let Some(own) = own else {
        return Ok(());
    };
    let path = output.join("Contents/Info.plist");
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut root = xcscheme::parse(&text).map_err(|e| e.to_string())?;
    let dict = root
        .children
        .iter_mut()
        .find(|c| c.name == "dict")
        .ok_or("dSYM plist has no dictionary")?;
    for pair in dict.children.chunks_exact_mut(2) {
        let key = &pair[0].text;
        if [
            "CFBundleIdentifier",
            "CFBundleVersion",
            "CFBundleShortVersionString",
        ]
        .contains(&key.as_str())
            && let Some((_, value)) = own.iter().find(|(k, _)| k == key)
        {
            pair[1].text = if key == "CFBundleIdentifier" {
                format!("com.apple.xcode.dsym.{value}")
            } else {
                value.clone()
            };
        }
    }
    super::write(&path, xcscheme::serialize(&root).as_bytes())
}

/// The DWARF records each interface's original path. Remove a duplicate only
/// when that path is inside the imported SDK and its bytes match exactly.
/// The DWARF file itself and interfaces from project dependencies stay intact.
fn omit_sdk_interfaces(bundle: &Path, roots: &[PathBuf]) -> Result<(), String> {
    let swift = bundle.join("Contents/Resources/Swift");
    if !swift.is_dir() || roots.is_empty() {
        return Ok(());
    }
    let roots: Vec<_> = roots.iter().filter_map(|p| p.canonicalize().ok()).collect();
    let dwarf = bundle.join("Contents/Resources/DWARF");
    for entry in std::fs::read_dir(dwarf).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !path.is_file() {
            continue;
        }
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        for value in BufReader::new(file).split(0) {
            let value = value.map_err(|e| e.to_string())?;
            let Ok(text) = std::str::from_utf8(&value) else {
                continue;
            };
            if !text.ends_with(".swiftinterface") {
                continue;
            }
            let source = Path::new(text);
            let Ok(real) = source.canonicalize() else {
                continue;
            };
            if !roots.iter().any(|root| real.starts_with(root)) {
                continue;
            }
            let Some(module) = source
                .parent()
                .filter(|p| p.extension().is_some_and(|e| e == "swiftmodule"))
                .and_then(Path::file_stem)
            else {
                continue;
            };
            let bytes = std::fs::read(&real).map_err(|e| e.to_string())?;
            for arch in std::fs::read_dir(&swift).map_err(|e| e.to_string())? {
                let candidate = arch
                    .map_err(|e| e.to_string())?
                    .path()
                    .join(format!("{}.swiftinterface", module.to_string_lossy()));
                if candidate.is_file()
                    && std::fs::read(&candidate).map_err(|e| e.to_string())? == bytes
                {
                    std::fs::remove_file(candidate).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    #[test]
    fn metadata_uses_the_nearest_bundle_and_preserves_other_keys() {
        let dir = TempDir::new("sweetpad-dsym-metadata");
        let app = dir.join("App.app");
        let test = app.join("PlugIns/Tests.xctest");
        let output = dir.join("Tests.xctest.dSYM");
        for (path, id, version) in [(&app, "app", "1"), (&test, "app.tests", "12")] {
            super::super::write(&path.join("Info.plist"),format!("<plist><dict><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundleVersion</key><string>{version}</string></dict></plist>").as_bytes()).unwrap();
        }
        super::super::write(&output.join("Contents/Info.plist"), b"<plist><dict><key>CFBundleIdentifier</key><string>wrong</string><key>CFBundleVersion</key><string>0</string><key>Toolchain</key><string>retain</string></dict></plist>").unwrap();
        repair_metadata(&test.join("Tests"), &output).unwrap();
        let values = plist_strings(&output.join("Contents/Info.plist"));
        assert!(values.contains(&(
            "CFBundleIdentifier".into(),
            "com.apple.xcode.dsym.app.tests".into()
        )));
        assert!(values.contains(&("CFBundleVersion".into(), "12".into())));
        assert!(values.contains(&("Toolchain".into(), "retain".into())));
    }

    #[test]
    fn sdk_duplicates_are_omitted_without_losing_project_interfaces_or_dwarf() {
        let dir = TempDir::new("sweetpad-dsym-interfaces");
        let sdk = dir.join("SDK/Developer");
        let framework = sdk.join("usr/lib/XCTest.swiftmodule/arm64.swiftinterface");
        let custom = dir.join("Project/Custom.swiftmodule/arm64.swiftinterface");
        let changed = sdk.join("usr/lib/Changed.swiftmodule/arm64.swiftinterface");
        let output = dir.join("Probe.dSYM");
        for p in [&framework, &custom, &changed] {
            super::super::write(p, b"interface").unwrap();
        }
        let swift = output.join("Contents/Resources/Swift/arm64");
        for name in ["XCTest", "Custom"] {
            super::super::write(&swift.join(format!("{name}.swiftinterface")), b"interface")
                .unwrap();
        }
        super::super::write(
            &swift.join("Changed.swiftinterface"),
            b"different interface",
        )
        .unwrap();
        let data = format!(
            "{}\0{}\0{}\0",
            framework.display(),
            custom.display(),
            changed.display()
        );
        let dwarf = output.join("Contents/Resources/DWARF/Probe");
        super::super::write(&dwarf, data.as_bytes()).unwrap();
        omit_sdk_interfaces(&output, &[sdk]).unwrap();
        assert!(!swift.join("XCTest.swiftinterface").exists());
        assert!(swift.join("Custom.swiftinterface").exists());
        assert!(swift.join("Changed.swiftinterface").exists());
        assert_eq!(std::fs::read(dwarf).unwrap(), data.as_bytes());
    }
}
