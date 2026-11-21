//! Host utilities for build scripts on minimal Linux installations. Their
//! links come last on PATH so projects can use a full system tool if present.
use std::path::{Path, PathBuf};
use std::{fs, io};

mod sync;

#[cfg(target_os = "linux")]
const TOOLS: &[&str] = &["file", "rsync", "unzip", "zip"];

/// A stable, last-resort PATH entry for SwiftPM and build phases. Swift
/// Build includes the environment in its task signature on incremental builds.
#[cfg(target_os = "linux")]
pub(crate) fn fallback_path() -> io::Result<Option<std::ffi::OsString>> {
    use std::hash::{Hash, Hasher};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    let missing: Vec<_> = TOOLS
        .iter()
        .filter(|tool| crate::cli::linux_setup::on_path(tool).is_none())
        .collect();
    if missing.is_empty() {
        return Ok(None);
    }
    let exe = std::env::current_exe()?;
    let mut hash = std::hash::DefaultHasher::new();
    exe.hash(&mut hash);
    let key = format!("{:016x}", hash.finish());
    let cache = std::path::absolute(crate::cli::linux_setup::cache_dir())?;
    let host = cache.join("host-tools");
    let dir = host.join(key);
    for path in [&cache, &host, &dir] {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        let meta = path.symlink_metadata()?;
        // SAFETY: geteuid has no preconditions.
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
            return Err(io::Error::other(format!(
                "build utility cache must be owned by the current user and writable only by that user: {}",
                path.display()
            )));
        }
    }
    for tool in missing {
        let link = dir.join(tool);
        if let Err(e) = std::os::unix::fs::symlink(&exe, &link)
            && (e.kind() != io::ErrorKind::AlreadyExists || fs::read_link(&link)? != exe)
        {
            return Err(e);
        }
    }
    let current = std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into());
    std::env::join_paths(std::env::split_paths(&current).chain([dir]))
        .map(Some)
        .map_err(io::Error::other)
}

pub(super) fn main(name: &str, args: &[String]) -> Option<Result<(), String>> {
    Some(match name {
        "rsync" => sync::run(args),
        "unzip" => unzip(args),
        "zip" => zip(args),
        "file" => file(args),
        _ => return None,
    })
}

fn zip(args: &[String]) -> Result<(), String> {
    let mut paths = Vec::new();
    for arg in args {
        if let Some(flags) = arg.strip_prefix('-') {
            if !flags.chars().all(|f| "qryX".contains(f)) {
                return Err(format!("unsupported zip option {arg}"));
            }
        } else {
            paths.push(Path::new(arg));
        }
    }
    let (output, names) = paths.split_first().ok_or("missing zip output")?;
    if names.is_empty() || output.exists() {
        return Err("expected a new archive and at least one input".into());
    }
    crate::cli::zip::create(output, Path::new("."), names).map_err(|e| e.to_string())
}

fn unzip(args: &[String]) -> Result<(), String> {
    let mut directory = PathBuf::from(".");
    let mut input = None;
    let mut test = false;
    let mut overwrite = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "-d" {
            directory = super::value(arg, &mut args)?.into();
        } else if let Some(flags) = arg.strip_prefix('-') {
            if !flags.chars().all(|f| "qot".contains(f)) {
                return Err(format!("unsupported unzip option {arg}"));
            }
            test |= flags.contains('t');
            overwrite |= flags.contains('o');
        } else if input.replace(Path::new(arg)).is_some() {
            return Err("member selection requires the system unzip".into());
        }
    }
    let mut zip = crate::cli::zip::open(input.ok_or("missing zip archive")?)?;
    if test {
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            io::copy(&mut entry, &mut io::sink()).map_err(|e| e.to_string())?;
        }
        Ok(())
    } else {
        crate::cli::zip::extract(&mut zip, &directory, overwrite).map_err(|e| e.to_string())
    }
}

/// CocoaPods uses file's Mach-O description to decide which binaries to
/// strip and which dSYM bundles to copy.
fn file(args: &[String]) -> Result<(), String> {
    let mut brief = false;
    let mut paths = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-b" | "--brief" => brief = true,
            "--" => {}
            flag if flag.starts_with('-') => return Err(format!("unsupported file option {flag}")),
            _ => paths.push(Path::new(arg)),
        }
    }
    if paths.is_empty() {
        return Err("expected a file".into());
    }
    for path in paths {
        let description = if path.is_dir() {
            "directory".into()
        } else {
            let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            super::lipo::file_description(path, &data)
        };
        if brief {
            println!("{description}");
        } else {
            println!("{}: {description}", path.display());
        }
    }
    Ok(())
}
