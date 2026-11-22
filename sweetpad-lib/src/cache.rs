//! The shared SweetPad cache and its application-owned subdirectories.

use std::io;
use std::path::{Component, Path, PathBuf};

/// The application directory under XDG_CACHE_HOME or ~/.cache.
#[must_use]
pub fn root() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(".cache"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("sweetpad")
}

/// Create and check the application root and each directory below it. Existing
/// permissions on the user's XDG base and its ancestors are left alone.
pub fn prepare(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    let mut path = root.to_path_buf();
    prepare_directory(&path)?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a relative cache subdirectory",
            ));
        };
        path.push(name);
        prepare_directory(&path)?;
    }
    Ok(path)
}

/// Create a cache directory with user-only permissions. Existing directories
/// may be readable by others, but only the owner can write to them.
fn prepare_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let meta = path.symlink_metadata()?;
    // SAFETY: geteuid has no preconditions.
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
        return Err(std::io::Error::other(format!(
            "{} must be a directory you own. Choose a private cache with XDG_CACHE_HOME.",
            path.display()
        )));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(std::io::Error::other(format!(
            "Other users can write to {}. Remove group and other write permissions (chmod go-w) or choose a private cache with XDG_CACHE_HOME.",
            path.display()
        )));
    }
    Ok(())
}

/// Read a regular cache file owned by this user. Refuse links, shared write
/// access and files above the caller's limit.
pub fn read_file(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let info = file.metadata()?;
    // SAFETY: geteuid has no preconditions.
    if !info.is_file()
        || info.uid() != unsafe { libc::geteuid() }
        || info.mode() & 0o022 != 0
        || info.len() > limit
    {
        return Err(io::Error::other(format!(
            "invalid cache file: {}",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("cache file exceeds size limit"));
    }
    Ok(bytes)
}

/// Publish a complete owner-only file with an atomic rename inside a prepared directory.
pub fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing cache directory"))?;
    let stage = crate::scratch::ScratchDir::new_in(parent, ".stage")?;
    let tmp = stage.join("data");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?
        .write_all(bytes)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn refuses_writable_and_symlinked_ancestors_before_creating_children() {
        let tmp = crate::testdir::TempDir::new("sweetpad-cache");
        let root = tmp.join("sweetpad");
        let elsewhere = tmp.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        symlink(&elsewhere, &root).unwrap();
        assert!(prepare(&root, Path::new("swift")).is_err());
        assert!(!elsewhere.join("swift").exists());
        std::fs::remove_file(&root).unwrap();

        let hot = prepare(&root, Path::new("hot-reload")).unwrap();
        std::fs::set_permissions(&hot, std::fs::Permissions::from_mode(0o775)).unwrap();
        assert!(prepare(&root, Path::new("hot-reload/client")).is_err());
        assert!(!hot.join("client").exists());
        std::fs::remove_dir(&hot).unwrap();
        symlink(&elsewhere, &hot).unwrap();
        assert!(prepare(&root, Path::new("hot-reload/client")).is_err());
        assert!(!elsewhere.join("client").exists());
    }
}
