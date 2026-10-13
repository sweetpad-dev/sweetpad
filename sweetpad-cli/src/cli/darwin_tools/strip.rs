//! `strip`, which an install build runs on each product
//! (`DEPLOYMENT_POSTPROCESSING`) and the copy of an embedded framework runs
//! on its binary: `strip -D [-x | -S] [-T] <file>`, with no style flag for
//! `STRIP_STYLE = all`.
//!
//! The Swift toolchain's `llvm-objcopy` strips Mach-O files when it runs as
//! `llvm-strip`, and takes most of Apple's flags as they are. Three mean
//! something else there: Apple's `-s` and `-R` name files of symbols to keep
//! and to remove (`--keep-symbols`, `--strip-symbols`), and `-u` keeps
//! undefined symbols (`--keep-undefined`).

use std::os::unix::process::CommandExt;
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
    if result.status.success() {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&result.stderr).trim().to_string())
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
            // only quiet Apple's own warnings.
            "-r" | "-no_uuid" | "-no_split_info" | "-no_code_signature_warning" => {}
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
    fn an_unknown_flag_is_an_error() {
        assert!(translate(&args(&["-frobnicate", "App"])).is_err());
    }
}
