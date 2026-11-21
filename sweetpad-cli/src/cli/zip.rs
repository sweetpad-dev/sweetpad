//! ZIP files used by IPA export, notarization and SwiftPM binary artifacts.

use std::fs::{self, File};
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

use ::zip::{DateTime, ZipArchive, ZipWriter, write::SimpleFileOptions};
use sweetpad_core::scratch::ScratchDir;

/// Read only the requested member, including archives with ZIP64 offsets.
pub(crate) fn read_member(
    path: &Path,
    wanted: impl Fn(&str) -> bool,
) -> Result<Option<(String, Vec<u8>)>, String> {
    let mut zip = open(path)?;
    let mut member = None;
    for (index, name) in zip.file_names().enumerate() {
        let name = name.map_err(|e| format!("{}: {e}", path.display()))?;
        if wanted(&name) {
            member = Some((index, name.into_owned()));
            break;
        }
    }
    let Some((index, name)) = member else {
        return Ok(None);
    };
    let mut data = Vec::new();
    zip.by_index(index)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut data)
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(Some((name, data)))
}

pub(crate) fn open(path: &Path) -> Result<ZipArchive<File>, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    ZipArchive::new(file).map_err(|e| format!("{} isn't a readable zip file: {e}", path.display()))
}

/// SwiftPM extracts binary artifacts into a fresh directory. Stage the
/// members first, then publish the directory only after validating its links.
pub(crate) fn extract(
    zip: &mut ZipArchive<File>,
    directory: &Path,
    overwrite: bool,
) -> io::Result<()> {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let parent = directory
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let scratch = ScratchDir::new_in(parent, ".sweetpad-unzip")?;
    let staged = scratch.join("artifact");
    fs::create_dir(&staged)?;
    let staged = staged.canonicalize()?;
    let mut links = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry
            .enclosed_name()
            .filter(|p| {
                p.components().all(|c| {
                    matches!(
                        c,
                        std::path::Component::Normal(_) | std::path::Component::CurDir
                    )
                })
            })
            .ok_or_else(|| io::Error::other("invalid ZIP member path"))?;
        let path = staged.join(name);
        if entry.is_dir() {
            fs::create_dir_all(&path)?;
            continue;
        }
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| io::Error::other("missing parent"))?,
        )?;
        if entry.is_symlink() {
            let mut target = String::new();
            entry.take(4097).read_to_string(&mut target)?;
            if target.len() > 4096 || Path::new(&target).is_absolute() {
                return Err(io::Error::other("invalid ZIP symlink"));
            }
            links.push((path, target));
        } else {
            let mut file = File::create_new(&path)?;
            io::copy(&mut entry, &mut file)?;
            if let Some(mode) = entry.unix_mode() {
                file.set_permissions(fs::Permissions::from_mode(mode & 0o777))?;
            }
        }
    }
    // No file writes follow symlinks supplied by the archive.
    for (path, target) in &links {
        symlink(target, path)?;
    }
    for (path, _) in links {
        if !path.canonicalize()?.starts_with(&staged) {
            return Err(io::Error::other("ZIP link leaves artifact"));
        }
    }
    publish(&staged, directory, overwrite)
}

/// Merge Flutter's separately downloaded archives into its cache. Recurse
/// only through real destination directories, never through existing links.
fn publish(source: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    let existing = match fs::symlink_metadata(dest) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return fs::rename(source, dest),
        Err(e) => return Err(e),
    };
    if fs::symlink_metadata(source)?.is_dir() && existing.is_dir() {
        for child in fs::read_dir(source)? {
            let child = child?;
            publish(&child.path(), &dest.join(child.file_name()), overwrite)?;
        }
        return Ok(());
    }
    if !overwrite || existing.is_dir() || fs::symlink_metadata(source)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            dest.display().to_string(),
        ));
    }
    fs::rename(source, dest)
}

/// Write a complete archive beside its destination before publishing it.
pub(crate) fn create(output: &Path, root: &Path, names: &[&Path]) -> io::Result<()> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let scratch = ScratchDir::new_in(parent, ".sweetpad-zip")?;
    let staged = scratch.join("output");
    let mut zip = ZipWriter::new(File::create(&staged)?);
    for name in names {
        if name
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(io::Error::other(format!(
                "expected a relative archive member: {}",
                name.display()
            )));
        }
        let input = root.join(name);
        if fs::symlink_metadata(&input)?.is_dir()
            && parent.canonicalize()?.starts_with(input.canonicalize()?)
        {
            return Err(io::Error::other(
                "cannot write an archive inside its input directory",
            ));
        }
        append(&mut zip, root, name)?;
    }
    zip.finish()?;
    fs::rename(staged, output)
}

fn append(zip: &mut ZipWriter<File>, root: &Path, name: &Path) -> io::Result<()> {
    let path = root.join(name);
    let meta = fs::symlink_metadata(&path)?;
    let name_str = name
        .to_str()
        .ok_or_else(|| io::Error::other("archive path is not UTF-8"))?;
    let mut options = SimpleFileOptions::default()
        .unix_permissions(meta.mode())
        .large_file(meta.len() >= u64::from(u32::MAX));
    if let Some(time) = modified_time(&meta) {
        options = options.last_modified_time(time);
    }
    if meta.is_symlink() {
        let target = fs::read_link(path)?;
        zip.add_symlink(
            name_str,
            target
                .to_str()
                .ok_or_else(|| io::Error::other("link target is not UTF-8"))?,
            options,
        )?;
    } else if meta.is_dir() {
        zip.add_directory(name_str, options)?;
        let mut children = fs::read_dir(path)?
            .map(|entry| entry.map(|e| e.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        children.sort();
        for child in children {
            append(zip, root, &name.join(child))?;
        }
    } else if meta.is_file() {
        zip.start_file(name_str, options)?;
        io::copy(&mut File::open(path)?, zip)?;
    } else {
        return Err(io::Error::other(format!(
            "unsupported file type: {}",
            path.display()
        )));
    }
    Ok(())
}

fn modified_time(meta: &fs::Metadata) -> Option<DateTime> {
    let time = meta.mtime();
    let mut tm = std::mem::MaybeUninit::uninit();
    // SAFETY: localtime_r initializes tm on success, using the supplied timestamp.
    if unsafe { libc::localtime_r(&raw const time, tm.as_mut_ptr()) }.is_null() {
        return None;
    }
    let tm = unsafe { tm.assume_init() };
    DateTime::from_date_and_time(
        (tm.tm_year + 1900).try_into().ok()?,
        (tm.tm_mon + 1).try_into().ok()?,
        tm.tm_mday.try_into().ok()?,
        tm.tm_hour.try_into().ok()?,
        tm.tm_min.try_into().ok()?,
        tm.tm_sec.try_into().ok()?,
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn invalid_bundle_links_leave_existing_output_alone() {
        let dir = crate::cli::testdir::TempDir::new("zip-invalid-link");
        let archive = dir.join("invalid.zip");
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        zip.add_symlink("link", "../../outside", SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
        let dest = dir.join("output");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("keep"), b"keep").unwrap();
        assert!(extract(&mut open(&archive).unwrap(), &dest, true).is_err());
        assert_eq!(fs::read(dest.join("keep")).unwrap(), b"keep");
        assert!(dest.join("link").symlink_metadata().is_err());
    }

    #[test]
    fn app_round_trip_preserves_links_modes_and_empty_directories() {
        let dir = crate::cli::testdir::TempDir::new("zip-app");
        fs::create_dir_all(dir.join("App.app/Empty")).unwrap();
        fs::write(dir.join("App.app/bin"), b"content").unwrap();
        fs::set_permissions(dir.join("App.app/bin"), fs::Permissions::from_mode(0o755)).unwrap();
        symlink("bin", dir.join("App.app/link")).unwrap();
        let archive = dir.join("App.zip");
        create(&archive, &dir, &[Path::new("App.app")]).unwrap();
        assert_eq!(
            read_member(&archive, |name| name.ends_with("/bin"))
                .unwrap()
                .unwrap()
                .1,
            b"content"
        );
        let mut zip = open(&archive).unwrap();
        assert_eq!(
            zip.by_name("App.app/bin").unwrap().unix_mode().unwrap() & 0o777,
            0o755
        );
        assert!(zip.by_name("App.app/link").unwrap().is_symlink());
        assert!(zip.by_name("App.app/Empty/").unwrap().is_dir());
        let dest = dir.join("output");
        extract(&mut zip, &dest, false).unwrap();
        fs::write(dest.join("keep"), b"unrelated").unwrap();
        fs::write(dest.join("App.app/bin"), b"old").unwrap();
        extract(&mut zip, &dest, true).unwrap();
        assert_eq!(fs::read(dest.join("keep")).unwrap(), b"unrelated");
        assert_eq!(
            fs::read_link(dest.join("App.app/link")).unwrap(),
            Path::new("bin")
        );
        assert_eq!(fs::read(dest.join("App.app/link")).unwrap(), b"content");
        assert!(read_member(&archive, |n| n == "absent").unwrap().is_none());
    }
}
