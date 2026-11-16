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
    let tool = llvm_ar().ok_or("no llvm-ar on PATH for '-r'; the Swift toolchain has one")?;
    // `q` appends, so an archive from the last build would keep its members;
    // and unlike `r`, it keeps two objects that share a file name.
    let _ = std::fs::remove_file(&request.output);
    let inputs = request.output.with_extension("o.ar-inputs");
    super::super::write(&inputs, response_file(&request.inputs).as_bytes())?;
    let status = Command::new(&tool)
        .args(["qcsD", "--format=darwin"])
        .arg(&request.output)
        .arg(format!("@{}", inputs.display()))
        .status()
        .map_err(|e| format!("couldn't run {}: {e}", tool.display()))?;
    let _ = std::fs::remove_file(&inputs);
    if !status.success() {
        return Err(format!("{} exited with {status}", tool.display()));
    }
    if let Some(path) = &request.dependency_info {
        super::super::write(path, &dependency_info(&request.inputs, &request.output))?;
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

/// The Swift toolchain's `llvm-ar`, the first on `PATH`.
fn llvm_ar() -> Option<PathBuf> {
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("llvm-ar"))
        .find(|candidate| candidate.is_file())
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
fn dependency_info(inputs: &[PathBuf], output: &Path) -> Vec<u8> {
    let mut data = vec![0x00];
    data.extend_from_slice(b"sweetpad ld64.lld -r\0");
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
        let info = dependency_info(&inputs[1..], Path::new("/b/Kit.o"));
        assert_eq!(info, b"\0sweetpad ld64.lld -r\0\x10/b/c.o\0\x40/b/Kit.o\0");
    }

    #[test]
    fn a_relocatable_link_needs_an_output() {
        assert!(Request::parse(&args(&["-r", "a.o"])).is_err());
    }
}
