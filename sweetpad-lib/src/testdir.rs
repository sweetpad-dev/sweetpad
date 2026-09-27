//! The scratch directory every crate's tests make fixtures in: a
//! [`ScratchDir`] that panics instead of returning an error.
//!
//! This one file serves this crate's unit tests, its integration tests
//! (`tests/common`), and the CLI's unit and integration tests, which include
//! it by path since they cannot see this crate's test code.

#![allow(dead_code)] // each includer uses its own share of this

use std::ffi::OsStr;
use std::ops::Deref;
use std::path::Path;

use sweetpad_lib::scratch::ScratchDir;

/// A fresh, empty directory under the temp directory, removed with everything
/// in it when this drops: at the end of the test, or while a failed assertion
/// unwinds out of it.
pub struct TempDir(ScratchDir);

impl TempDir {
    /// `<temp>/<name>-<pid>-<n>`, as [`ScratchDir::new`] names it.
    pub fn new(name: &str) -> Self {
        Self::new_in(&std::env::temp_dir(), name)
    }

    /// [`TempDir::new`] under `parent` instead of the temp directory: `/tmp`
    /// for a unix socket, whose path has to fit in 104 bytes however long
    /// `$TMPDIR` is.
    pub fn new_in(parent: &Path, name: &str) -> Self {
        let dir = ScratchDir::new_in(parent, name)
            .unwrap_or_else(|e| panic!("make a scratch directory in {}: {e}", parent.display()));
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
