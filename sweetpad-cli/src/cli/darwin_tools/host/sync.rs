//! Local rsync operations emitted by CocoaPods for frameworks and resources.
use std::collections::HashSet;
use std::ffi::{CString, OsStr};
use std::fs::{self, FileTimes};
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // independent rsync options
struct Options {
    delete: bool,
    update: bool,
    follow: bool,
    flat: bool,
    flutter_modes: bool,
    list: Option<PathBuf>,
    excludes: Vec<String>,
    protects: Vec<String>,
}

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let mut options = Options::default();
    let mut paths = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--delete" => options.delete = true,
            "--copy-links" => options.follow = true,
            "--no-relative" => options.flat = true,
            "--chmod=Du=rwx,Dgo=rx,Fu=rw,Fgo=r" => options.flutter_modes = true,
            "-8" | "--links" | "--archive" | "--recursive" | "--verbose" => {}
            "--filter" => {
                let rule = super::super::value(arg, &mut it)?;
                if let Some(pattern) = rule.strip_prefix("- ") {
                    options.excludes.push(pattern.into());
                } else if let Some(pattern) = rule.strip_prefix("P ") {
                    options.protects.push(pattern.into());
                } else {
                    return Err(format!("unsupported rsync filter {rule}"));
                }
            }
            "--exclude" => options
                .excludes
                .push(super::super::value(arg, &mut it)?.clone()),
            "--files-from" => options.list = Some(super::super::value(arg, &mut it)?.into()),
            _ if arg.starts_with("--files-from=") => options.list = Some(arg[13..].into()),
            _ if arg.starts_with("--exclude=") => options.excludes.push(arg[10..].into()),
            _ if arg.starts_with('-') && arg[1..].chars().all(|c| "avrluL".contains(c)) => {
                options.update |= arg.contains('u');
                options.follow |= arg.contains('L');
            }
            _ if arg.starts_with('-') || arg.contains(':') => {
                return Err(format!("unsupported local rsync argument {arg}"));
            }
            _ => paths.push(arg.as_str()),
        }
    }
    let (dest, sources) = paths.split_last().ok_or("missing rsync destination")?;
    if sources.is_empty() {
        return Err("missing rsync source".into());
    }
    let dest = Path::new(dest);
    let mut files: Vec<_> = sources
        .iter()
        .map(|s| (PathBuf::from(s), s.ends_with('/')))
        .collect();
    if let Some(list) = &options.list {
        if files.len() != 1 || !options.flat || options.delete {
            return Err(
                "rsync resource lists require one source, --no-relative and no --delete".into(),
            );
        }
        let base = &files[0].0;
        let lines = fs::read_to_string(list).map_err(|e| format!("{}: {e}", list.display()))?;
        files = lines
            .lines()
            .filter(|s| !s.is_empty())
            .map(|s| {
                let name = Path::new(s.trim_start_matches('/'));
                if name
                    .components()
                    .any(|c| c == std::path::Component::ParentDir)
                {
                    return Err("invalid parent path in resource list");
                }
                Ok((base.join(name), false))
            })
            .collect::<Result<_, _>>()?;
    }
    let container = dest.is_dir()
        || files.len() != 1
        || options.list.is_some()
        || dest.as_os_str().as_bytes().ends_with(b"/");
    if container {
        fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    }
    for (source, contents) in files {
        let target = if container && !contents {
            dest.join(source.file_name().ok_or("missing source basename")?)
        } else {
            dest.to_owned()
        };
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let resolved = parent
            .canonicalize()
            .map_err(|e| format!("{}: {e}", parent.display()))?
            .join(target.file_name().unwrap_or_default());
        if source
            .canonicalize()
            .is_ok_and(|src| resolved.starts_with(&src) || src.starts_with(&resolved))
        {
            return Err("source and destination overlap".into());
        }
        options
            .copy(&source, &target, Path::new(""), &mut HashSet::new())
            .map_err(|e| format!("{} -> {}: {e}", source.display(), target.display()))?;
    }
    Ok(())
}

fn matches(patterns: &[String], relative: &Path, dir: bool) -> bool {
    patterns.iter().any(|pattern| {
        if pattern.ends_with('/') && !dir {
            return false;
        }
        let pattern = pattern.trim_end_matches('/');
        let subject = if pattern.contains('/') {
            relative.as_os_str()
        } else {
            relative.file_name().unwrap_or(OsStr::new(""))
        };
        let (Ok(pattern), Ok(subject)) = (CString::new(pattern), CString::new(subject.as_bytes()))
        else {
            return false;
        };
        // SAFETY: both arguments are NUL-terminated for this call.
        unsafe { libc::fnmatch(pattern.as_ptr(), subject.as_ptr(), 0) == 0 }
    })
}

impl Options {
    fn copy(
        &self,
        src: &Path,
        dst: &Path,
        rel: &Path,
        parents: &mut HashSet<(u64, u64)>,
    ) -> io::Result<()> {
        let meta = if self.follow {
            fs::metadata(src)
        } else {
            fs::symlink_metadata(src)
        }?;
        if matches(&self.excludes, rel, meta.is_dir()) {
            return Ok(());
        }
        let previous = match fs::symlink_metadata(dst) {
            Ok(meta) => Some(meta),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if previous
            .as_ref()
            .is_some_and(|m| (m.dev(), m.ino()) == (meta.dev(), meta.ino()))
        {
            return Err(io::Error::other("source equals destination"));
        }
        if meta.is_dir() {
            if !parents.insert((meta.dev(), meta.ino())) {
                return Err(io::Error::other("directory symlink cycle"));
            }
            if previous.is_some_and(|m| !m.is_dir()) {
                fs::remove_file(dst)?;
            }
            fs::create_dir_all(dst)?;
            let mut names = HashSet::new();
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                let name = entry.file_name();
                self.copy(&entry.path(), &dst.join(&name), &rel.join(&name), parents)?;
                names.insert(name);
            }
            if self.delete {
                for entry in fs::read_dir(dst)? {
                    let entry = entry?;
                    if !names.contains(&entry.file_name()) {
                        self.prune(&entry.path(), &rel.join(entry.file_name()))?;
                    }
                }
            }
            parents.remove(&(meta.dev(), meta.ino()));
        } else {
            if self.update
                && previous
                    .as_ref()
                    .is_some_and(|m| m.is_file() && m.modified().ok() > meta.modified().ok())
            {
                return Ok(());
            }
            let scratch = sweetpad_core::scratch::ScratchDir::new_in(
                dst.parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
                ".sweetpad-rsync",
            )?;
            let staged = scratch.join("file");
            if meta.is_symlink() {
                std::os::unix::fs::symlink(fs::read_link(src)?, &staged)?;
            } else if meta.is_file() {
                fs::copy(src, &staged)?;
            } else {
                return Err(io::Error::other("unsupported file type"));
            }
            if previous.is_some_and(|m| m.is_dir()) {
                fs::remove_dir(dst)?;
            }
            fs::rename(staged, dst)?;
        }
        if !meta.is_symlink() {
            let handle = fs::File::open(dst)?;
            handle.set_times(FileTimes::new().set_modified(meta.modified()?))?;
            let permissions = if self.flutter_modes {
                use std::os::unix::fs::PermissionsExt;
                fs::Permissions::from_mode(if meta.is_dir() { 0o755 } else { 0o644 })
            } else {
                meta.permissions()
            };
            handle.set_permissions(permissions)?;
        }
        Ok(())
    }

    /// Preserve excluded and protected entries even inside a stale directory.
    fn prune(&self, path: &Path, rel: &Path) -> io::Result<()> {
        let dir = fs::symlink_metadata(path)?.is_dir();
        if matches(&self.excludes, rel, dir) || matches(&self.protects, rel, dir) {
            return Ok(());
        }
        if dir {
            for child in fs::read_dir(path)? {
                let child = child?;
                self.prune(&child.path(), &rel.join(child.file_name()))?;
            }
            if fs::read_dir(path)?.next().is_none() {
                fs::remove_dir(path)?;
            }
        } else {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;
    use std::os::unix::fs::symlink;

    #[test]
    fn pods_framework_copy_filters_deletes_and_preserves_links() {
        let dir = TempDir::new("pods-rsync");
        let src = dir.join("Kit.framework");
        let dest = dir.join("Frameworks");
        fs::create_dir_all(src.join("Headers")).unwrap();
        fs::create_dir_all(dest.join("Kit.framework")).unwrap();
        fs::write(src.join("Code"), b"binary").unwrap();
        symlink("Code", src.join("Kit")).unwrap();
        fs::write(dest.join("Kit.framework/stale"), b"x").unwrap();
        fs::write(dest.join("Kit.framework/.temp.123456"), b"x").unwrap();
        run(&[
            "--delete",
            "-av",
            "--filter",
            "P .*.??????",
            "--links",
            "--filter",
            "- Headers",
            src.to_str().unwrap(),
            dest.to_str().unwrap(),
        ]
        .map(str::to_owned))
        .unwrap();
        let out = dest.join("Kit.framework");
        assert_eq!(fs::read_link(out.join("Kit")).unwrap(), Path::new("Code"));
        assert!(out.join(".temp.123456").exists());
        assert!(!out.join("stale").exists());
        assert!(!out.join("Headers").exists());
    }

    #[test]
    fn resource_file_lists_flatten_absolute_paths_and_follow_links() {
        let dir = TempDir::new("pods-resource-list");
        let src = dir.join("source");
        fs::write(&src, b"resource").unwrap();
        let link = dir.join("linked");
        symlink(&src, &link).unwrap();
        let list = dir.join("files.txt");
        fs::write(&list, format!("{}\n", link.display())).unwrap();
        let dest = dir.join("Resources");
        run(&[
            "-avr".into(),
            "--copy-links".into(),
            "--no-relative".into(),
            format!("--files-from={}", list.display()),
            "/".into(),
            dest.to_str().unwrap().into(),
        ])
        .unwrap();
        assert_eq!(fs::read(dest.join("linked")).unwrap(), b"resource");
        assert!(!dest.join("linked").is_symlink());
    }
}
