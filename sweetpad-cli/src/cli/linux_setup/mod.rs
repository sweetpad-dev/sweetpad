//! What a Linux machine needs to build for Apple platforms, found or put in
//! place: the host packages, the pinned Swift and its Darwin archiver.
//! `setup` runs the steps that are not done yet; `doctor` and the build
//! look the same things up, so neither needs `linux-env.sh` on `PATH`.

pub(crate) mod host;
pub(crate) mod swift;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// `name` on `PATH`, when it is an executable file.
pub(crate) fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|candidate| executable(candidate))
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|info| info.is_file() && info.permissions().mode() & 0o111 != 0)
}

/// The Darwin archiver in the pinned Swift toolchain.
pub(crate) fn archiver() -> Option<PathBuf> {
    swift::pinned()
        .map(|s| s.bin.join("llvm-ar"))
        .filter(|p| executable(p))
}
