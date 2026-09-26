//! Scaffolding shared by the CLI's integration tests. The unit tests carry
//! their own [`TempDir`] in `src/cli/testdir.rs`, since the crate's test code
//! is not visible from here.

use std::ffi::OsStr;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh, empty directory under the temp directory, removed with everything
/// in it when this drops: at the end of the test, or while a failed assertion
/// unwinds out of it.
pub struct TempDir(PathBuf);

impl TempDir {
    /// `<temp>/<name>-<pid>-<n>`, where `n` counts the directories this
    /// process has made. The name stays short because a unix socket path
    /// under it has to fit in 104 bytes. One an earlier process with the same
    /// pid left behind is cleared first.
    pub fn new(name: &str) -> Self {
        static MADE: AtomicUsize = AtomicUsize::new(0);
        let n = MADE.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<OsStr> for TempDir {
    fn as_ref(&self) -> &OsStr {
        self.0.as_os_str()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
