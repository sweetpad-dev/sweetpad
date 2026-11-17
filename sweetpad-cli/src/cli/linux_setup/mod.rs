//! What a Linux machine needs to build for Apple platforms, found or put in
//! place: the host packages, the pinned Swift and LLVM's Darwin `libtool`.
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

/// `$XDG_CACHE_HOME/sweetpad`, by default `~/.cache/sweetpad`.
pub(crate) fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".cache")
        })
        .join("sweetpad")
}

/// LLVM's `llvm-libtool-darwin`, which archives static libraries for Apple
/// targets: on `PATH`, else the newest one a distribution's versioned LLVM
/// package installs (`/usr/bin/llvm-libtool-darwin-18`,
/// `/usr/lib/llvm-18/bin/llvm-libtool-darwin`).
pub(crate) fn libtool() -> Option<PathBuf> {
    on_path("llvm-libtool-darwin").or_else(|| {
        let versioned = |dir: &str, matches: &dyn Fn(&str) -> Option<(u32, PathBuf)>| {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|entry| matches(&entry.file_name().to_string_lossy()))
                .filter(|(_, path)| executable(path))
                .max_by_key(|(version, _)| *version)
        };
        let in_bin = versioned("/usr/bin", &|name| {
            let version = name.strip_prefix("llvm-libtool-darwin-")?.parse().ok()?;
            Some((version, Path::new("/usr/bin").join(name)))
        });
        let in_lib = versioned("/usr/lib", &|name| {
            let version = name.strip_prefix("llvm-")?.parse().ok()?;
            Some((
                version,
                Path::new("/usr/lib")
                    .join(name)
                    .join("bin/llvm-libtool-darwin"),
            ))
        });
        [in_bin, in_lib]
            .into_iter()
            .flatten()
            .max_by_key(|(version, _)| *version)
            .map(|(_, path)| path)
    })
}
