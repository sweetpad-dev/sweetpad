//! `ld64.lld`, the linker Swift Build runs, in front of LLVM's. LLD can't
//! make a relocatable object, so `ld -r`, which Swift Build runs to put each
//! package target's objects into one, makes a static archive instead, named
//! like the object (see [`relocatable`]). A link given that archive takes
//! only the members it needs, where the object would have brought in all of
//! it: a dynamic library made from a package target alone comes out with
//! none of the target's code. So the final link loads each such archive
//! with `-force_load`, which brings in every member. The rest of the link
//! is LLD's, with the load order of the libraries it links matched to
//! Apple's linker (see [`autolink`] and [`reexports`]).

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod autolink;
mod reexports;
mod relocatable;
mod response;

pub(super) fn main(args: &[String]) -> ExitCode {
    match run(args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: ld64.lld: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let expanded = response::expand(args)?;
    if expanded.iter().any(|a| a == "-r") {
        relocatable::archive(&expanded)?;
        return Ok(ExitCode::SUCCESS);
    }
    let linker = next_linker().ok_or("no LLVM linker beside this one; sweetpad links it there")?;
    let transformed =
        force_load_prelinked(&expanded).and_then(autolink::prefer_swiftui_umbrella)?;
    let args = response::forward(args, &expanded, transformed)?;
    // By name, since the linker can be a link to LLVM's multicall binary,
    // which picks its tool from the name it's started under.
    let status = Command::new(&linker)
        .arg0("ld64.lld")
        .args(&args)
        .status()
        .map_err(|e| format!("couldn't run {}: {e}", linker.display()))?;
    Ok(ExitCode::from(
        status
            .code()
            .and_then(|c| u8::try_from(c).ok())
            .unwrap_or(1),
    ))
}

/// LLVM's linker, linked beside this one by the engine. A task's `PATH`
/// puts the Swift toolchain's LLD first, which refuses to link for Apple's
/// platforms, so this one is never looked up there.
fn next_linker() -> Option<PathBuf> {
    std::env::args_os()
        .next()
        .map(PathBuf::from)
        .and_then(|me| me.parent().map(|dir| dir.join(super::SDK_LINKER)))
        .and_then(|link| link.canonicalize().ok())
}

/// The options whose value is a file the link writes or already loads.
const TAKES_PATH: &[&str] = &["-o", "-force_load", "-load_hidden"];

/// `args` with each archive made for `ld -r` loaded whole, where it
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
    let list = FileList::read(value)?;
    let mut kept = String::new();
    let mut archives = Vec::new();
    for line in &list.lines {
        let path = list.path(line);
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
    let rewritten = format!("{}.objects", list.file);
    super::write(Path::new(&rewritten), kept.as_bytes())?;
    let value = match list.folder {
        Some(folder) => format!("{rewritten},{folder}"),
        None => rewritten,
    };
    Ok((value, archives))
}

/// A `-filelist` value, `<file>` or `<file>,<folder>`, and the lines of the
/// file; each names an input, relative to the folder when there is one.
struct FileList<'a> {
    file: &'a str,
    folder: Option<&'a str>,
    lines: Vec<String>,
}

impl<'a> FileList<'a> {
    fn read(value: &'a str) -> Result<Self, String> {
        let (file, folder) = match value.rsplit_once(',') {
            Some((file, folder)) if !Path::new(value).is_file() => (file, Some(folder)),
            _ => (value, None),
        };
        let text = std::fs::read_to_string(file)
            .map_err(|e| format!("can't read the file list {file}: {e}"))?;
        let lines = text
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        Ok(Self {
            file,
            folder,
            lines,
        })
    }

    fn path(&self, line: &str) -> PathBuf {
        self.folder
            .map_or_else(|| PathBuf::from(line), |f| Path::new(f).join(line))
    }

    fn paths(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.lines.iter().map(|line| self.path(line))
    }
}

/// Whether `path` is an object file name over a static archive.
fn is_prelinked(path: &Path) -> bool {
    use std::io::Read as _;
    if path.extension().is_none_or(|e| e != "o") {
        return false;
    }
    let mut magic = [0; 8];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    if file.read_exact(&mut magic).is_err() {
        return false;
    }
    if &magic == b"!<arch>\n" {
        return true;
    }
    if !matches!(&magic[..4], b"\xca\xfe\xba\xbe" | b"\xca\xfe\xba\xbf") {
        return false;
    }
    let mut data = magic.to_vec();
    file.read_to_end(&mut data).is_ok() && super::is_static_archive(path, &data)
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
