//! `ld -r`, which Swift Build runs to put each package target's objects into
//! one. LLD can't write a relocatable object, so this writes a static archive
//! of them under the object's name, as `llvm-libtool-darwin -static` would,
//! with the Swift toolchain's `llvm-ar`. The link that loads it takes every
//! member (see `force_load_prelinked`). The dependency info Swift Build asks
//! for names the inputs and the archive in ld64's format.

use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) fn archive(args: &[String]) -> Result<(), String> {
    let request = Request::parse(args)?;
    let scratch = scratch(&request.output)?;
    write_archive(&scratch, &request.output, &request.inputs, "qcsD")?;
    if let Some(path) = &request.dependency_info {
        super::super::write(
            path,
            &dependency_info("sweetpad ld64.lld -r", &request.inputs, &request.output),
        )?;
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
struct Request {
    output: PathBuf,
    inputs: Vec<PathBuf>,
    dependency_info: Option<PathBuf>,
}

impl Request {
    /// The output, the dependency info, and the objects from the file lists
    /// and the command line. Every other option is the final link's.
    fn parse(args: &[String]) -> Result<Self, String> {
        let (mut output, mut dependency_info, mut inputs) = (None, None, Vec::new());
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "-o" => output = Some(PathBuf::from(super::super::value(arg, &mut it)?)),
                "-dependency_info" => {
                    dependency_info = Some(PathBuf::from(super::super::value(arg, &mut it)?));
                }
                "-filelist" => {
                    inputs
                        .extend(super::FileList::read(super::super::value(arg, &mut it)?)?.paths());
                }
                // Options whose value could look like an object.
                "-object_path_lto" | "-map" | "-lto_library" => {
                    super::super::value(arg, &mut it)?;
                }
                input
                    if !input.starts_with('-')
                        && Path::new(input).extension().is_some_and(|e| e == "o") =>
                {
                    inputs.push(PathBuf::from(input));
                }
                _ => {}
            }
        }
        let output = output.ok_or("'-r' needs an '-o' output")?;
        Ok(Self {
            output,
            inputs,
            dependency_info,
        })
    }
}

/// Stage beside the output so a failed or repeated build never appends to
/// the old archive and a successful one can replace it atomically.
pub(super) fn scratch(output: &Path) -> Result<sweetpad_core::scratch::ScratchDir, String> {
    let dir = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    sweetpad_core::scratch::ScratchDir::new_in(dir, ".sweetpad-ar").map_err(|e| e.to_string())
}

pub(super) fn write_archive(
    scratch: &Path,
    output: &Path,
    members: &[PathBuf],
    flags: &str,
) -> Result<(), String> {
    let inputs = scratch.join("inputs.rsp");
    super::super::write(&inputs, response_file(members).as_bytes())?;
    let archive = scratch.join("output.a");
    // Swift Build puts the pinned toolchain before the system tools on PATH.
    let status = Command::new("llvm-ar")
        .args([flags, "--format=darwin"])
        .arg(&archive)
        .arg(format!("@{}", inputs.display()))
        .status()
        .map_err(|e| format!("llvm-ar: {e}"))?;
    if !status.success() {
        return Err(format!("llvm-ar exited with {status}"));
    }
    std::fs::rename(archive, output).map_err(|e| format!("{}: {e}", output.display()))
}

/// `paths` as an `llvm-ar` response file, quoted as a GNU command line is.
fn response_file(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        let text = path.display().to_string();
        out.push('"');
        out.push_str(&text.replace('\\', "\\\\").replace('"', "\\\""));
        out.push_str("\"\n");
    }
    out
}

/// ld64's dependency info: a version record, then a record per input and
/// one for the output, each an opcode byte and a NUL-terminated path.
pub(super) fn dependency_info(version: &str, inputs: &[PathBuf], output: &Path) -> Vec<u8> {
    let mut data = vec![0x00];
    data.extend_from_slice(version.as_bytes());
    data.push(0);
    for (opcode, path) in inputs
        .iter()
        .map(|p| (0x10, p.as_path()))
        .chain([(0x40, output)])
    {
        data.push(opcode);
        data.extend_from_slice(path.as_os_str().as_encoded_bytes());
        data.push(0);
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_relocatable_link_takes_its_objects_from_the_file_list() {
        let dir = TempDir::new("sweetpad-ld-r");
        let list = dir.join("Kit.LinkFileList");
        std::fs::write(&list, "a.o\nsub dir/b.o\n").unwrap();
        let value = format!("{},{}", list.display(), dir.display());
        let request = Request::parse(&args(&[
            "-demangle",
            "-arch",
            "arm64",
            "-o",
            "out/Kit.o",
            "-r",
            "-filelist",
            &value,
            "-object_path_lto",
            "out/Kit_lto.o",
            "-dependency_info",
            "out/Kit_dependency_info.dat",
            "-Fout/Products",
            "extra.o",
        ]))
        .unwrap();
        assert_eq!(
            request,
            Request {
                output: PathBuf::from("out/Kit.o"),
                inputs: vec![
                    dir.join("a.o"),
                    dir.join("sub dir/b.o"),
                    PathBuf::from("extra.o")
                ],
                dependency_info: Some(PathBuf::from("out/Kit_dependency_info.dat")),
            }
        );
    }

    #[test]
    fn inputs_are_quoted_for_llvm_ar_and_recorded_as_ld64_does() {
        let inputs = [PathBuf::from("/b/my \"x\".o"), PathBuf::from("/b/c.o")];
        assert_eq!(
            response_file(&inputs),
            "\"/b/my \\\"x\\\".o\"\n\"/b/c.o\"\n"
        );
        let info = dependency_info("sweetpad ld64.lld -r", &inputs[1..], Path::new("/b/Kit.o"));
        assert_eq!(info, b"\0sweetpad ld64.lld -r\0\x10/b/c.o\0\x40/b/Kit.o\0");
    }

    #[test]
    fn a_relocatable_link_needs_an_output() {
        assert!(Request::parse(&args(&["-r", "a.o"])).is_err());
    }
}
