//! Scratch directories for the unit tests. The integration tests carry their
//! own copy in `tests/common`, since they cannot see this crate's test code.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh, empty directory under the temp directory, removed with everything
/// in it when this drops: at the end of the test, or while a failed assertion
/// unwinds out of it.
pub struct TempDir(PathBuf);

impl TempDir {
    /// `<temp>/<name>-<pid>-<n>`, where `n` counts the directories this
    /// process has made. One an earlier process with the same pid left behind
    /// is cleared first.
    pub fn new(name: &str) -> Self {
        static MADE: AtomicUsize = AtomicUsize::new(0);
        let n = MADE.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
