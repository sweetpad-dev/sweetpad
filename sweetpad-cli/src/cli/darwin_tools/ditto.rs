//! `ditto`, macOS's copier, which run scripts use; XcodeGen gives each static
//! library with Swift sources one that copies its generated header with it.
//!
//! `ditto <file> <file>` copies a file; `ditto <source>… <folder>` copies
//! each source folder's contents, or each file, into the folder, merging
//! with what's there. Symbolic links stay links, and files keep their
//! permissions. The flags about resource forks, extended attributes, ACLs and
//! quarantine change nothing here; making or opening archives (`-c`, `-x`)
//! is left to `zip` and `unzip`.

use std::path::{Path, PathBuf};

pub(super) fn main(args: &[String]) -> Result<(), String> {
    let mut paths = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-v"
            | "-V"
            | "--rsrc"
            | "--norsrc"
            | "--extattr"
            | "--noextattr"
            | "--acl"
            | "--noacl"
            | "--qtn"
            | "--noqtn"
            | "--preserveHFSCompression"
            | "--nopreserveHFSCompression"
            | "--nocache"
            | "--hfsCompression"
            | "--nohfsCompression"
            | "--persistRootless"
            | "--clone"
            | "--noclone" => {}
            "-c" | "-x" | "-k" | "-z" | "-j" | "--keepParent" | "--sequesterRsrc" => {
                return Err(format!(
                    "'{arg}' makes or opens an archive, which this stand-in doesn't; use zip or unzip"
                ));
            }
            flag if flag.starts_with('-') => return Err(format!("unsupported option: {flag}")),
            path => paths.push(PathBuf::from(path)),
        }
    }
    let Some((destination, sources)) = paths.split_last() else {
        return Err("expected a source and a destination".to_string());
    };
    if sources.is_empty() {
        return Err("expected a source and a destination".to_string());
    }
    if let [source] = sources
        && !source.is_dir()
        && !destination.is_dir()
    {
        return copy(source, destination);
    }
    for source in sources {
        if source.is_dir() {
            merge(source, destination)?;
        } else {
            let name = source
                .file_name()
                .ok_or_else(|| format!("{} has no file name", source.display()))?;
            copy(source, &destination.join(name))?;
        }
    }
    Ok(())
}

/// Copy the contents of the folder `source` into `destination`.
fn merge(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::create_dir_all(destination)
        .map_err(|e| format!("can't create {}: {e}", destination.display()))?;
    let entries =
        std::fs::read_dir(source).map_err(|e| format!("can't read {}: {e}", source.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("can't read {}: {e}", source.display()))?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let kind = entry
            .file_type()
            .map_err(|e| format!("can't read {}: {e}", from.display()))?;
        if kind.is_dir() {
            merge(&from, &to)?;
        } else {
            copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Copy one file or symbolic link to `destination`, replacing what's there.
fn copy(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(source)
        .map_err(|e| format!("can't read {}: {e}", source.display()))?;
    if let Some(dir) = destination.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    }
    if metadata.is_symlink() {
        let target = std::fs::read_link(source)
            .map_err(|e| format!("can't read {}: {e}", source.display()))?;
        let _ = std::fs::remove_file(destination);
        return std::os::unix::fs::symlink(&target, destination)
            .map_err(|e| format!("can't create {}: {e}", destination.display()));
    }
    if std::fs::symlink_metadata(destination).is_ok_and(|m| m.is_symlink()) {
        let _ = std::fs::remove_file(destination);
    }
    std::fs::copy(source, destination).map(|_| ()).map_err(|e| {
        format!(
            "can't copy {} to {}: {e}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    fn args(list: &[&Path]) -> Vec<String> {
        list.iter().map(|p| p.display().to_string()).collect()
    }

    #[test]
    fn a_folder_merges_into_the_destination() {
        let dir = TempDir::new("sweetpad-ditto-merge");
        let source = dir.join("src");
        std::fs::create_dir_all(source.join("sub")).unwrap();
        std::fs::write(source.join("a.txt"), "a").unwrap();
        std::fs::write(source.join("sub/b.txt"), "b").unwrap();
        std::os::unix::fs::symlink("a.txt", source.join("link")).unwrap();
        let destination = dir.join("dst");
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::write(destination.join("kept.txt"), "kept").unwrap();
        main(&args(&[&source, &destination])).unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("sub/b.txt")).unwrap(),
            "b"
        );
        assert_eq!(
            std::fs::read_to_string(destination.join("kept.txt")).unwrap(),
            "kept"
        );
        assert_eq!(
            std::fs::read_link(destination.join("link")).unwrap(),
            Path::new("a.txt")
        );
    }

    #[test]
    fn a_file_copies_to_a_new_path_with_its_folders() {
        let dir = TempDir::new("sweetpad-ditto-file");
        let source = dir.join("Kit-Swift.h");
        std::fs::write(&source, "header").unwrap();
        let destination = dir.join("include/Kit/Kit-Swift.h");
        main(&args(&[&source, &destination])).unwrap();
        assert_eq!(std::fs::read_to_string(destination).unwrap(), "header");
    }

    #[test]
    fn archives_are_refused() {
        assert!(main(&["-c".into(), "-k".into(), "a".into(), "a.zip".into()]).is_err());
    }
}
