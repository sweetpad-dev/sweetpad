//! Throwaway directories for tool runs that leave files in `$TMPDIR`.
//!
//! The Swift driver makes a `TemporaryDirectory.*` in `$TMPDIR` on every run,
//! `swift --version` included, and never removes it; SwiftPM adds a lock file
//! there named for the scratch path it builds in. A child whose `TMPDIR` is a
//! [`ScratchDir`] leaves all of that in a directory that goes when the run is
//! done.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// A fresh, empty directory under the temp dir, removed with everything in it
/// when this drops. It derefs to its path.
pub struct ScratchDir(PathBuf);

impl ScratchDir {
    /// `<temp>/<prefix>-<pid>-<n>`, where `n` counts the directories this
    /// process has made, so concurrent runs get one each. A name that already
    /// exists (left by an earlier process with the same pid) is skipped, never
    /// reused.
    pub fn new(prefix: &str) -> io::Result<Self> {
        static MADE: AtomicU32 = AtomicU32::new(0);
        loop {
            let n = MADE.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
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
}
