//! Finding the Xcode workspaces, projects and Swift packages in a directory
//! tree: the one walk behind the CLI's auto-discovery and the extension's
//! project picker, so both skip the same trees.
//!
//! A vendored tree holds projects that are never the one meant:
//! `Pods/Pods.xcodeproj`, a checkout under `.build/checkouts` or
//! `SourcePackages/checkouts`, a package under `node_modules`. The walk never
//! enters those, nor a dotted directory, nor a bundle.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// Build output and vendored dependency trees, which hold projects that are
/// never the one meant — `Pods/Pods.xcodeproj` above all. `SourcePackages` is
/// where `-clonedSourcePackagesDirPath` commonly puts package checkouts.
pub const VENDORED_DIRS: [&str; 7] = [
    "node_modules",
    "Pods",
    "Carthage",
    "vendor",
    "DerivedData",
    "build",
    "SourcePackages",
];

/// The kind of a container, in the order Xcode prefers one over another when
/// a directory holds several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Workspace,
    Project,
    Package,
}

/// The containers in one directory, each kind sorted by name so a pick never
/// depends on `read_dir` order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub workspaces: Vec<PathBuf>,
    pub projects: Vec<PathBuf>,
    /// The directory's `Package.swift`.
    pub package: Option<PathBuf>,
}

impl Found {
    /// Whether the directory holds no container at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty() && self.projects.is_empty() && self.package.is_none()
    }

    /// Every container in the directory: workspaces, then projects, then the
    /// package, the order Xcode prefers them in.
    pub fn containers(&self) -> impl Iterator<Item = (Kind, &Path)> {
        self.workspaces
            .iter()
            .map(|p| (Kind::Workspace, p.as_path()))
            .chain(self.projects.iter().map(|p| (Kind::Project, p.as_path())))
            .chain(self.package.iter().map(|p| (Kind::Package, p.as_path())))
    }
}

/// Every container directly inside `dir`. An unreadable directory holds none.
#[must_use]
pub fn in_dir(dir: &Path) -> Found {
    let mut found = Found::default();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match path.extension().and_then(OsStr::to_str) {
            Some("xcworkspace") => found.workspaces.push(path),
            Some("xcodeproj") => found.projects.push(path),
            _ => {
                if path.file_name() == Some(OsStr::new("Package.swift")) {
                    found.package = Some(path);
                }
            }
        }
    }
    found.workspaces.sort();
    found.projects.sort();
    found
}

/// Directories the walk never enters: [vendored trees](VENDORED_DIRS), dotted
/// directories (`.build`, `.git`, `.swiftpm`), and bundles, which are
/// directories on macOS and so would otherwise be walked like ordinary ones.
#[must_use]
pub fn skip_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return true;
    };
    if name.starts_with('.') {
        return true;
    }
    VENDORED_DIRS.contains(&name)
        || matches!(
            path.extension().and_then(OsStr::to_str),
            Some("xcodeproj" | "xcworkspace" | "app" | "framework" | "bundle" | "playground")
        )
}

/// A directory the walk found containers in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// How many levels below the walk's root the directory is: 0 for the root.
    pub depth: usize,
    pub found: Found,
}

/// Every directory from `root` down to `max_depth` levels below it that holds
/// a container, breadth-first: shallower directories first, and the
/// directories at one depth in path order.
///
/// The walk enters every directory [`skip_dir`] allows, one that holds a
/// container included, so a package nested in an app's repository is found
/// beside the app. It never follows a symlink:
/// [`std::fs::DirEntry::file_type`] doesn't follow one, so a link to a
/// directory never reports `is_dir`, and the walk can't cycle or escape the
/// tree it was pointed at.
#[must_use]
pub fn below(root: &Path, max_depth: usize) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut level = vec![root.to_path_buf()];
    for depth in 0..=max_depth {
        let mut next = Vec::new();
        for dir in &level {
            let found = in_dir(dir);
            if !found.is_empty() {
                hits.push(Hit { depth, found });
            }
            if depth == max_depth {
                continue;
            }
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            let mut dirs: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .filter(|p| !skip_dir(p))
                .collect();
            // Sorted so a same-depth tie resolves the same way on every
            // machine, `read_dir` order being arbitrary.
            dirs.sort();
            next.extend(dirs);
        }
        level = next;
    }
    hits
}

/// Every container from `root` down to `max_depth` levels below it, in the
/// order [`below`] finds their directories and, within a directory,
/// workspaces first. The first is the one to open when nothing says which:
/// the nearest, and the kind Xcode prefers.
#[must_use]
pub fn containers(root: &Path, max_depth: usize) -> Vec<(Kind, usize, PathBuf)> {
    below(root, max_depth)
        .into_iter()
        .flat_map(|hit| {
            let depth = hit.depth;
            hit.found
                .containers()
                .map(|(kind, path)| (kind, depth, path.to_path_buf()))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;

    fn touch_dir(root: &Path, rel: &str) {
        fs::create_dir_all(root.join(rel)).unwrap();
    }

    fn touch_file(root: &Path, rel: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }

    fn relative(root: &Path, found: &[(Kind, usize, PathBuf)]) -> Vec<String> {
        found
            .iter()
            .map(|(_, _, p)| p.strip_prefix(root).unwrap().display().to_string())
            .collect()
    }

    #[test]
    fn in_dir_sorts_each_kind_and_finds_the_manifest() {
        let root = TempDir::new("sweetpad-discover-in-dir");
        touch_dir(&root, "B.xcworkspace");
        touch_dir(&root, "A.xcworkspace");
        touch_dir(&root, "App.xcodeproj");
        touch_file(&root, "Package.swift");
        let found = in_dir(&root);
        assert_eq!(
            found.workspaces,
            [root.join("A.xcworkspace"), root.join("B.xcworkspace")]
        );
        assert_eq!(found.projects, [root.join("App.xcodeproj")]);
        assert_eq!(found.package, Some(root.join("Package.swift")));
    }

    /// `Pods/Pods.xcodeproj`, `node_modules/…`, a checkout under
    /// `.build/checkouts` or `SourcePackages/checkouts`, a package's
    /// `.swiftpm/xcode/package.xcworkspace`, and a project's embedded
    /// workspace are never offered.
    #[test]
    fn the_walk_skips_vendored_trees_dotted_directories_and_bundles() {
        let root = TempDir::new("sweetpad-discover-skip");
        touch_dir(&root, "App.xcworkspace");
        touch_dir(&root, "App.xcodeproj/project.xcworkspace");
        touch_dir(&root, "Pods/Pods.xcodeproj");
        touch_file(&root, "node_modules/react-native/Package.swift");
        touch_file(&root, ".build/checkouts/Alamofire/Package.swift");
        touch_file(&root, "SourcePackages/checkouts/Alamofire/Package.swift");
        touch_dir(&root, "Carthage/Checkouts/Lib/Lib.xcodeproj");
        touch_dir(&root, "build/Stale.xcodeproj");
        touch_dir(&root, "vendor/Lib.xcodeproj");
        touch_file(&root, "Packages/Core/Package.swift");
        touch_dir(&root, "Packages/Core/.swiftpm/xcode/package.xcworkspace");
        assert_eq!(
            relative(&root, &containers(&root, 4)),
            [
                "App.xcworkspace",
                "App.xcodeproj",
                "Packages/Core/Package.swift"
            ]
        );
    }

    #[test]
    fn nearer_directories_come_first_and_ties_go_in_path_order() {
        let root = TempDir::new("sweetpad-discover-order");
        touch_dir(&root, "z/Z.xcodeproj");
        touch_dir(&root, "a/deep/Deep.xcodeproj");
        touch_dir(&root, "b/B.xcodeproj");
        touch_file(&root, "b/Package.swift");
        touch_dir(&root, "b/B.xcworkspace");
        let found = containers(&root, 2);
        assert_eq!(
            relative(&root, &found),
            [
                "b/B.xcworkspace",
                "b/B.xcodeproj",
                "b/Package.swift",
                "z/Z.xcodeproj",
                "a/deep/Deep.xcodeproj"
            ]
        );
        assert_eq!(found[0].0, Kind::Workspace);
        assert_eq!(found[4].1, 2);
    }

    #[test]
    fn the_walk_stops_at_its_depth() {
        let root = TempDir::new("sweetpad-discover-depth");
        touch_dir(&root, "a/b/c/Deep.xcodeproj");
        assert!(containers(&root, 2).is_empty());
        assert_eq!(
            relative(&root, &containers(&root, 3)),
            ["a/b/c/Deep.xcodeproj"]
        );
        touch_dir(&root, "Root.xcodeproj");
        assert_eq!(containers(&root, 0)[0].1, 0);
    }

    #[cfg(unix)]
    #[test]
    fn the_walk_never_follows_a_symlink() {
        let root = TempDir::new("sweetpad-discover-link");
        let outside = TempDir::new("sweetpad-discover-link-target");
        touch_dir(&outside, "Elsewhere.xcodeproj");
        std::os::unix::fs::symlink(&*outside, root.join("link")).unwrap();
        assert!(containers(&root, 2).is_empty());
    }
}
