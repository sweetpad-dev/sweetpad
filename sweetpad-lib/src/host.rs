//! The account facts Xcode's tools read from the system rather than the
//! environment: the home directory, the login name, and the per-user cache
//! directory.
//!
//! `xcodebuild` ignores `$HOME`, `$USER` and `$TMPDIR` for all three. It puts
//! DerivedData under the account's home from the user database, reports that
//! account in the `HOME` and `USER` settings, reads per-user schemes and
//! workspace settings under its name, and builds `CACHE_ROOT` from
//! `confstr(_CS_DARWIN_USER_CACHE_DIR)`. On Xcode 27, `HOME=/tmp/x
//! USER=other xcodebuild -showBuildSettings` still reports the account's own
//! home, name and DerivedData. The one override it honors is
//! `$CFFIXED_USER_HOME`, Foundation's fixed home, which moves the home and so
//! DerivedData. A process with a redirected `$HOME` (a sandbox, a test
//! harness, `sudo` without `-H`) still builds into the account's DerivedData,
//! so every path sweetpad names for something Xcode writes is found this way.

use std::ffi::{CStr, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The home directory Xcode's tools use: `$CFFIXED_USER_HOME` when it is
/// set, spelled the way `xcodebuild` reports it (`.` and `..` collapsed, a
/// leading `/private` dropped), else the account's home from the user
/// database. `$HOME` answers only when the user database has no entry for
/// the account.
#[must_use]
pub fn home() -> Option<PathBuf> {
    if let Some(fixed) = std::env::var_os("CFFIXED_USER_HOME").filter(|h| !h.is_empty()) {
        let fixed = crate::project::absolutize(Path::new(&fixed));
        return Some(crate::project::without_private_root(&fixed));
    }
    account()
        .map(|account| account.home.clone())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .filter(|home| !home.as_os_str().is_empty())
}

/// The login name of the account the process runs as, from the user
/// database. `$USER` answers only when the user database has no entry.
#[must_use]
pub fn user() -> Option<String> {
    account()
        .map(|account| account.name.clone())
        .or_else(|| std::env::var("USER").ok())
        .filter(|user| !user.is_empty())
}

/// The per-user cache directory, `/var/folders/<x>/<id>/C/` on macOS, that
/// `CACHE_ROOT` is built under: `confstr(_CS_DARWIN_USER_CACHE_DIR)`, which
/// `$TMPDIR` doesn't move. Where the system has no answer, `$TMPDIR` with its
/// final `T` swapped for `C`, which is how macOS lays the two out.
#[must_use]
pub fn darwin_user_cache_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(read_darwin_user_cache_dir)
        .clone()
        .or_else(|| {
            let tmp = std::env::var_os("TMPDIR").filter(|t| !t.is_empty())?;
            let tmp = PathBuf::from(tmp);
            Some(if tmp.file_name() == Some(OsStr::new("T")) {
                tmp.with_file_name("C")
            } else {
                tmp
            })
        })
}

struct Account {
    name: String,
    home: PathBuf,
}

/// The user database's entry for the process's real user id, read once.
fn account() -> Option<&'static Account> {
    static ACCOUNT: OnceLock<Option<Account>> = OnceLock::new();
    ACCOUNT.get_or_init(read_account).as_ref()
}

fn read_account() -> Option<Account> {
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: `passwd` is a plain C struct of pointers and integers, for
    // which all-zero is a valid (if meaningless) value; getpwuid_r fills it.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call, and `buf.len()` is the
    // buffer's real size. On success `found` points at `entry`, whose string
    // fields point into `buf`.
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &raw mut entry,
            buf.as_mut_ptr(),
            buf.len(),
            &raw mut found,
        )
    };
    if status != 0 || found.is_null() || entry.pw_name.is_null() || entry.pw_dir.is_null() {
        return None;
    }
    // SAFETY: both fields are NUL-terminated strings inside `buf`, which
    // outlives these borrows.
    let (name, home) = unsafe { (CStr::from_ptr(entry.pw_name), CStr::from_ptr(entry.pw_dir)) };
    Some(Account {
        name: name.to_str().ok()?.to_string(),
        home: PathBuf::from(OsStr::from_bytes(home.to_bytes())),
    })
}

#[cfg(target_vendor = "apple")]
fn read_darwin_user_cache_dir() -> Option<PathBuf> {
    let mut buf = vec![0 as libc::c_char; 1024];
    // SAFETY: `buf` is valid for `buf.len()` bytes. confstr writes at most
    // that many, NUL-terminated, and returns the length the full value needs
    // (terminator included), or 0 when the name has no value.
    let needed =
        unsafe { libc::confstr(libc::_CS_DARWIN_USER_CACHE_DIR, buf.as_mut_ptr(), buf.len()) };
    if needed == 0 || needed > buf.len() {
        return None;
    }
    // SAFETY: confstr NUL-terminated what it wrote into `buf`.
    let dir = unsafe { CStr::from_ptr(buf.as_ptr()) };
    Some(PathBuf::from(OsStr::from_bytes(dir.to_bytes()))).filter(|d| !d.as_os_str().is_empty())
}

#[cfg(not(target_vendor = "apple"))]
fn read_darwin_user_cache_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The account is the one the process runs as, whatever `$HOME` says:
    /// its home exists and the cache directory sits under the per-user
    /// `/var/folders` tree, not in `$TMPDIR`.
    #[test]
    fn account_facts_come_from_the_system() {
        let home = home().expect("the account has a home");
        assert!(home.is_absolute(), "{}", home.display());
        assert!(!user().unwrap_or_default().is_empty());
        if cfg!(target_vendor = "apple") {
            let cache = darwin_user_cache_dir().expect("confstr answers on macOS");
            assert!(
                cache.ends_with("C"),
                "the per-user cache dir ends in C/: {}",
                cache.display()
            );
        }
    }
}
