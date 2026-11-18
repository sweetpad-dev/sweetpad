//! The sample app `setup` builds, embedded in the binary and written to a
//! content-addressed cache folder on first use.
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
        CliError::new(format!("prepare the embedded sample app: {e}")).kind(ErrorKind::ToolMissing)
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

/// The SHA-256 of the Swift Build patches and lockfile this sweetpad was
/// released with, one after another, as `build-swift-helpers.sh` records it
/// beside the `swbuild` it builds.
pub(crate) fn swift_build_patches() -> String {
    let input = [
        include_bytes!("../../../sweetpad-lib/ci/swift-build-linux.patch").as_slice(),
        include_bytes!("../../../sweetpad-lib/ci/swift-build-generic-debug-bridges.patch"),
        include_bytes!("../../../sweetpad-lib/ci/swift-llbuild-directory-race.patch"),
        include_bytes!("../../../sweetpad-lib/ci/swift-llbuild-spawn-descriptors.patch"),
        include_bytes!("../../../sweetpad-lib/ci/swift-build-Package.resolved"),
    ]
    .concat();
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
    // The cache holds the project setup builds. Only the invoking user
    // may write it, even when XDG_CACHE_HOME points outside the home directory.
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
    let scratch = sweetpad_core::scratch::ScratchDir::new_in(cache, ".stage")?;
    let stage = scratch.join("sample");
    std::fs::DirBuilder::new().mode(0o700).create(&stage)?;
    for (name, bytes) in FILES {
        let path = stage.join(name);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, bytes)?;
    }
    match std::fs::rename(&stage, &destination) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    #[test]
    fn standalone_resources_include_the_sample() {
        let dir = TempDir::new("embedded-public-resources");
        let root = materialize(&dir.join("cache")).unwrap();
        assert!(
            root.join("examples/LinuxApp/LinuxApp.xcodeproj/project.pbxproj")
                .is_file()
        );
        // Swift Build and the macro plugin ship built, beside the binary.
        assert!(!root.join("setup").exists());
        assert_eq!(materialize(&dir.join("cache")).unwrap(), root);
    }

    #[test]
    fn changed_or_symlinked_resources_are_never_reused() {
        let dir = TempDir::new("embedded-corrupt-resources");
        let cache = dir.join("cache");
        let root = materialize(&cache).unwrap();
        let project = root.join("examples/LinuxApp/LinuxApp.xcodeproj/project.pbxproj");
        std::fs::write(&project, "changed").unwrap();
        assert!(materialize(&cache).is_err());
        std::fs::remove_file(&project).unwrap();
        std::os::unix::fs::symlink("other.pbxproj", &project).unwrap();
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
