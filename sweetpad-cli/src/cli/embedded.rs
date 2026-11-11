//! Content-addressed public resources for standalone CLI installations.
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

use super::{CliError, ErrorKind};

include!(concat!(env!("OUT_DIR"), "/setup-resources.rs"));

pub(crate) fn root() -> Result<PathBuf, CliError> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map_or_else(std::env::temp_dir, |p| p.join(".cache"))
        });
    materialize(&cache.join("sweetpad/distribution")).map_err(|e| {
        CliError::new(format!("prepare embedded setup files: {e}")).kind(ErrorKind::ToolMissing)
    })
}

fn fingerprint() -> String {
    let mut input = Vec::new();
    for (name, bytes) in FILES {
        input.extend_from_slice(&(name.len() as u64).to_le_bytes());
        input.extend_from_slice(name.as_bytes());
        input.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        input.extend_from_slice(bytes);
    }
    let mut digest = String::with_capacity(64);
    for byte in sweetpad_lib::xcode_hash::sha256(&input) {
        write!(digest, "{byte:02x}").unwrap();
    }
    digest
}

fn valid(root: &Path) -> bool {
    FILES.iter().all(|(name, bytes)| {
        let mut path = root.to_path_buf();
        for component in Path::new(name).components() {
            path.push(component);
            if !path
                .symlink_metadata()
                .is_ok_and(|m| !m.file_type().is_symlink())
            {
                return false;
            }
        }
        std::fs::read(path).is_ok_and(|data| data == *bytes)
    })
}

fn materialize(cache: &Path) -> Result<PathBuf, std::io::Error> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(cache)?;
    let metadata = cache.symlink_metadata()?;
    // The cache contains executable bootstrap inputs. Only the invoking user
    // may write them, even when XDG_CACHE_HOME points outside the home directory.
    if !metadata.is_dir() || metadata.file_type().is_symlink()
        // SAFETY: geteuid has no arguments or memory preconditions.
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        return Err(std::io::Error::other(format!(
            "setup cache must be an owned directory without group or world write access: {}",
            cache.display()
        )));
    }
    let destination = cache.join(fingerprint());
    if destination.exists() {
        return verify(&destination);
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stage = cache.join(format!(".stage-{}-{nonce}", std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&stage)?;
    let cleanup = Staged(stage);
    for (name, bytes) in FILES {
        let path = cleanup.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, bytes)?;
    }
    match std::fs::rename(&cleanup.0, &destination) {
        Ok(()) => Ok(destination),
        Err(_) if destination.exists() => verify(&destination),
        Err(error) => Err(error),
    }
}

fn verify(directory: &Path) -> Result<PathBuf, std::io::Error> {
    use std::os::unix::fs::MetadataExt;
    let metadata = directory.symlink_metadata()?;
    if metadata.file_type().is_dir()
        // SAFETY: geteuid has no arguments or memory preconditions.
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o022 == 0
        && valid(directory)
    {
        Ok(directory.to_path_buf())
    } else {
        Err(std::io::Error::other(format!(
            "cached setup files are incomplete or modified; remove {} and retry",
            directory.display()
        )))
    }
}

struct Staged(PathBuf);

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    #[test]
    fn standalone_resources_include_bootstrap_macros_and_sample() {
        let dir = TempDir::new("embedded-public-resources");
        let root = materialize(&dir.join("cache")).unwrap();
        for path in [
            "setup/sweetpad-lib/ci/linux-setup.sh",
            "setup/sweetpad-cli/macros/Package.swift",
            "examples/LinuxApp/LinuxApp.xcodeproj/project.pbxproj",
        ] {
            assert!(root.join(path).is_file(), "{path}");
        }
        assert_eq!(materialize(&dir.join("cache")).unwrap(), root);
    }

    #[test]
    fn changed_or_symlinked_bootstrap_is_never_reused() {
        let dir = TempDir::new("embedded-corrupt-resources");
        let cache = dir.join("cache");
        let root = materialize(&cache).unwrap();
        let script = root.join("setup/sweetpad-lib/ci/linux-setup.sh");
        std::fs::write(&script, "changed").unwrap();
        assert!(materialize(&cache).is_err());
        std::fs::remove_file(&script).unwrap();
        std::os::unix::fs::symlink("linux-sdk.py", &script).unwrap();
        assert!(materialize(&cache).is_err());
    }

    #[test]
    fn concurrent_first_use_has_one_complete_resource_tree() {
        let dir = TempDir::new("embedded-concurrent-resources");
        let jobs: Vec<_> = (0..4)
            .map(|_| {
                let cache = dir.join("cache");
                std::thread::spawn(move || materialize(&cache).unwrap())
            })
            .collect();
        let roots: Vec<_> = jobs.into_iter().map(|j| j.join().unwrap()).collect();
        assert!(roots.iter().all(|root| root == &roots[0] && valid(root)));
        assert_eq!(std::fs::read_dir(dir.join("cache")).unwrap().count(), 1);
    }
}
