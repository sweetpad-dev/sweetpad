//! Throwaway directories: a tool run's `TMPDIR` or scratch path, and every
//! crate's test fixtures.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// A fresh, empty directory, removed with everything in it when this drops.
/// It derefs to its path.
pub struct ScratchDir(PathBuf);

impl ScratchDir {
    /// `<temp>/<prefix>-<pid>-<n>` under the temp dir. See
    /// [`ScratchDir::new_in`].
    pub fn new(prefix: &str) -> io::Result<Self> {
        Self::new_in(&std::env::temp_dir(), prefix)
    }

    /// `<parent>/<prefix>-<pid>-<n>`, where `n` counts the directories this
    /// process has made, so concurrent runs get one each. A name that already
    /// exists (left by an earlier process with the same pid) is skipped, never
    /// reused.
    pub fn new_in(parent: &Path, prefix: &str) -> io::Result<Self> {
        static MADE: AtomicU32 = AtomicU32::new(0);
        loop {
            let n = MADE.fetch_add(1, Ordering::Relaxed);
            let dir = parent.join(format!("{prefix}-{}-{n}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
    }
}

impl std::ops::Deref for ScratchDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for ScratchDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<OsStr> for ScratchDir {
    fn as_ref(&self) -> &OsStr {
        self.0.as_os_str()
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_scratch_dir_is_fresh_and_goes_on_drop() {
        let first = ScratchDir::new("sweetpad-scratch-test").unwrap();
        let second = ScratchDir::new("sweetpad-scratch-test").unwrap();
        assert_ne!(*first, *second);
        std::fs::write(first.join("left-behind"), "x").unwrap();
        let path = first.to_path_buf();
        drop(first);
        assert!(!path.exists());
        assert!(second.is_dir());
    }

    #[test]
    fn new_in_makes_it_under_the_parent() {
        let parent = ScratchDir::new("sweetpad-scratch-parent").unwrap();
        let child = ScratchDir::new_in(&parent, "child").unwrap();
        assert_eq!(child.parent(), Some(&*parent));
        assert!(child.is_dir());
    }
}
