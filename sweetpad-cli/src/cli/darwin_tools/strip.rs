//! `strip`, which an install build runs on each product
//! (`DEPLOYMENT_POSTPROCESSING`) and the copy of an embedded framework runs
//! on its binary: `strip -D [-x | -S] [-T] <file>`, with no style flag for
//! `STRIP_STYLE = all`.
//!
//! The Swift toolchain's `llvm-objcopy` strips Mach-O files when it runs as
//! `llvm-strip`, and takes most of Apple's flags as they are. Three mean
//! something else there: Apple's `-s` and `-R` name files of symbols to keep
//! and to remove (`--keep-symbols`, `--strip-symbols`), and `-u` keeps
//! undefined symbols (`--keep-undefined`). What it writes then gets the
//! 8-byte aligned string table Apple's strip writes, which dyld requires
//! (see [`crate::cli::codesign::align_string_tables`]).

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

pub(super) fn main(args: &[String]) -> Result<(), String> {
    let translated = translate(args)?;
    let objcopy = super::find_program("llvm-objcopy").ok_or_else(|| {
        "llvm-objcopy isn't on PATH; it comes with the Swift toolchain".to_string()
    })?;
    let result = Command::new(&objcopy)
        .arg0("llvm-strip")
        .args(&translated)
        .output()
        .map_err(|e| format!("couldn't run {}: {e}", objcopy.display()))?;
    print!("{}", String::from_utf8_lossy(&result.stdout));
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).trim().to_string());
    }
    for file in outputs(&translated) {
        let data = super::read(Path::new(file))?;
        if let Some(aligned) = crate::cli::codesign::align_string_tables(&data)? {
            let permissions = std::fs::metadata(file).map(|m| m.permissions());
            super::write(Path::new(file), &aligned)?;
            if let Ok(permissions) = permissions {
                std::fs::set_permissions(file, permissions)
                    .map_err(|e| format!("can't set the mode of {file}: {e}"))?;
            }
        }
    }
    Ok(())
}

/// The files a strip writes: its '-o' file, else each file it strips in
/// place.
fn outputs(translated: &[String]) -> Vec<&str> {
    if let Some(at) = translated.iter().position(|arg| arg == "-o") {
        return translated
            .get(at + 1)
            .map(String::as_str)
            .into_iter()
            .collect();
    }
    translated
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .map(String::as_str)
        .collect()
}

/// Apple's strip flags as `llvm-strip`'s.
fn translate(args: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-S" | "-x" | "-X" | "-T" | "-D" => out.push(arg.clone()),
            "-o" => {
                out.push(arg.clone());
                out.push(super::value(arg, &mut it)?.clone());
            }
            "-s" => out.push(format!("--keep-symbols={}", super::value(arg, &mut it)?)),
            "-R" => out.push(format!("--strip-symbols={}", super::value(arg, &mut it)?)),
            "-u" => out.push("--keep-undefined".to_string()),
            // Symbols the dynamic linker binds stay either way, and the rest
            // only quiet Apple's own warnings. '-no_atom_info', which Swift
            // Build passes when it copies an embedded framework, drops
            // Apple's linker's LC_ATOM_INFO, which LLD doesn't write.
            "-r"
            | "-no_uuid"
            | "-no_split_info"
            | "-no_code_signature_warning"
            | "-no_atom_info" => {}
            flag if flag.starts_with('-') => {
                return Err(format!("unsupported option: {flag}"));
            }
            file => out.push(file.to_string()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn apples_flags_become_llvm_strips() {
        assert_eq!(
            translate(&args(&[
                "-D", "-x", "-T", "-u", "-r", "-s", "keep.txt", "App", "-o", "out"
            ]))
            .unwrap(),
            args(&[
                "-D",
                "-x",
                "-T",
                "--keep-undefined",
                "--keep-symbols=keep.txt",
                "App",
                "-o",
                "out"
            ])
        );
    }

    #[test]
    fn an_embedded_frameworks_copy_strips_as_llvm_strip_does() {
        assert_eq!(
            translate(&args(&[
                "-D",
                "-S",
                "-no_atom_info",
                "In/Alamofire",
                "-o",
                "App.app/Frameworks/Alamofire.framework/Alamofire"
            ]))
            .unwrap(),
            args(&[
                "-D",
                "-S",
                "In/Alamofire",
                "-o",
                "App.app/Frameworks/Alamofire.framework/Alamofire"
            ])
        );
    }

    #[test]
    fn an_unknown_flag_is_an_error() {
        assert!(translate(&args(&["-frobnicate", "App"])).is_err());
    }
}
