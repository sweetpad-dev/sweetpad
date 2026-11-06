//! `ld64.lld`, the linker Swift Build runs, in front of the one in xtool's
//! Darwin SDK. lld can't make a relocatable object, so for `ld -r`, which
//! Swift Build runs to put each package target's objects into one, xtool's
//! makes a static archive instead, named like the object. A link given that
//! archive takes only the members it needs, where the object would have
//! brought in all of it: a dynamic library made from a package target alone
//! comes out with none of the target's code. This hands each such archive
//! to the linker with `-force_load`, which brings in every member, and
//! leaves everything else to xtool's linker.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

pub(super) fn main(args: &[String]) -> ExitCode {
    let Some(linker) = next_linker() else {
        eprintln!("error: ld64.lld: no other ld64.lld on PATH; xtool's Darwin SDK has one");
        return ExitCode::FAILURE;
    };
    let args = if args.iter().any(|a| a == "-r") {
        args.to_vec()
    } else {
        match force_load_prelinked(args) {
            Ok(args) => args,
            Err(message) => {
                eprintln!("error: ld64.lld: {message}");
                return ExitCode::FAILURE;
            }
        }
    };
    match Command::new(&linker).args(&args).status() {
        Ok(status) => ExitCode::from(
            status
                .code()
                .and_then(|c| u8::try_from(c).ok())
                .unwrap_or(1),
        ),
        Err(e) => {
            eprintln!("error: ld64.lld: couldn't run {}: {e}", linker.display());
            ExitCode::FAILURE
        }
    }
}

/// The Darwin SDK's `ld64.lld`: the one the engine links beside this one,
/// else the first other one on `PATH`. A task's `PATH` puts the Swift
/// toolchain's lld first, which refuses to link for Apple's platforms.
fn next_linker() -> Option<PathBuf> {
    let beside = std::env::args_os()
        .next()
        .map(PathBuf::from)
        .and_then(|me| me.parent().map(|dir| dir.join(super::SDK_LINKER)))
        .and_then(|link| link.canonicalize().ok());
    if beside.is_some() {
        return beside;
    }
    let me = std::env::current_exe().ok()?.canonicalize().ok()?;
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("ld64.lld"))
        .filter(|candidate| candidate.is_file())
        .find(|candidate| candidate.canonicalize().ok().as_ref() != Some(&me))
}

/// The options whose value is a file the link writes or already loads.
const TAKES_PATH: &[&str] = &["-o", "-force_load", "-load_hidden"];

/// `args` with each archive xtool made for `ld -r` loaded whole, where it
/// stood or where the file list that names it stood.
fn force_load_prelinked(args: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if TAKES_PATH.contains(&arg.as_str()) {
            out.push(arg.clone());
            out.push(super::value(arg, &mut it)?.clone());
        } else if arg == "-filelist" {
            let (list, archives) = filelist(super::value(arg, &mut it)?)?;
            out.push(arg.clone());
            out.push(list);
            for archive in archives {
                out.push("-force_load".to_string());
                out.push(archive);
            }
        } else if is_prelinked(Path::new(arg)) {
            out.push("-force_load".to_string());
            out.push(arg.clone());
        } else {
            out.push(arg.clone());
        }
    }
    Ok(out)
}

/// The file list `value` names, `<file>` or `<file>,<folder>`, without the
/// archives in it, and those archives. A list with none stays as it is;
/// any other is written again beside it.
fn filelist(value: &str) -> Result<(String, Vec<String>), String> {
    let (list, folder) = match value.rsplit_once(',') {
        Some((list, folder)) if !Path::new(value).is_file() => (list, Some(folder)),
        _ => (value, None),
    };
    let text = std::fs::read_to_string(list)
        .map_err(|e| format!("can't read the file list {list}: {e}"))?;
    let mut kept = String::new();
    let mut archives = Vec::new();
    for line in text.lines() {
        let path = folder.map_or_else(|| PathBuf::from(line), |f| Path::new(f).join(line));
        if is_prelinked(&path) {
            archives.push(path.display().to_string());
        } else {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    if archives.is_empty() {
        return Ok((value.to_string(), archives));
    }
    let rewritten = format!("{list}.objects");
    super::write(Path::new(&rewritten), kept.as_bytes())?;
    let value = match folder {
        Some(folder) => format!("{rewritten},{folder}"),
        None => rewritten,
    };
    Ok((value, archives))
}

/// Whether `path` is an object file name over a static archive.
fn is_prelinked(path: &Path) -> bool {
    use std::io::Read as _;
    if path.extension().is_none_or(|e| e != "o") {
        return false;
    }
    let mut magic = [0; 8];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok_and(|()| &magic == b"!<arch>\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn an_archive_named_as_an_object_is_loaded_whole() {
        let dir = TempDir::new("sweetpad-ld-args");
        let archive = dir.join("Kit.o");
        let object = dir.join("main.o");
        std::fs::write(&archive, b"!<arch>\nmembers").unwrap();
        std::fs::write(&object, b"\xcf\xfa\xed\xfe").unwrap();
        let (archive, object) = (archive.display().to_string(), object.display().to_string());
        assert_eq!(
            force_load_prelinked(&args(&["-dylib", &object, &archive, "-o", &archive])).unwrap(),
            args(&["-dylib", &object, "-force_load", &archive, "-o", &archive])
        );
    }

    #[test]
    fn a_file_list_loses_its_archives_to_force_load() {
        let dir = TempDir::new("sweetpad-ld-filelist");
        std::fs::write(dir.join("Kit.o"), b"!<arch>\n").unwrap();
        std::fs::write(dir.join("main.o"), b"\xcf\xfa\xed\xfe").unwrap();
        let list = dir.join("App.LinkFileList");
        std::fs::write(&list, "main.o\nKit.o\n").unwrap();
        let value = format!("{},{}", list.display(), dir.display());
        let out = force_load_prelinked(&args(&["-filelist", &value])).unwrap();
        let rewritten = format!("{}.objects", list.display());
        assert_eq!(
            out,
            args(&[
                "-filelist",
                &format!("{rewritten},{}", dir.display()),
                "-force_load",
                &dir.join("Kit.o").display().to_string()
            ])
        );
        assert_eq!(std::fs::read_to_string(rewritten).unwrap(), "main.o\n");
    }

    #[test]
    fn a_list_with_no_archives_stays_as_it_is() {
        let dir = TempDir::new("sweetpad-ld-plain");
        std::fs::write(dir.join("main.o"), b"\xcf\xfa\xed\xfe").unwrap();
        let list = dir.join("App.LinkFileList");
        std::fs::write(&list, format!("{}\n", dir.join("main.o").display())).unwrap();
        let list = list.display().to_string();
        assert_eq!(
            force_load_prelinked(&args(&["-filelist", &list])).unwrap(),
            args(&["-filelist", &list])
        );
    }
}
