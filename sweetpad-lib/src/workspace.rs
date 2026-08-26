//! Typed model of an `.xcworkspace`.
//!
//! Reads `contents.xcworkspacedata` (XML, same parser as `.xcscheme`) and
//! the workspace-level schemes (shared `xcshareddata/xcschemes/` plus
//! per-user `xcuserdata/<user>.xcuserdatad/xcschemes/`).
//! Returns absolute paths to every referenced `.xcodeproj` and every local
//! SwiftPM package.
//!
//! What `contents.xcworkspacedata` looks like:
//!
//! ```xml
//! <Workspace version="1.0">
//!   <FileRef location="group:Foo.xcodeproj"/>
//!   <FileRef location="group:Sub/Bar.xcodeproj"/>
//!   <FileRef location="group:MyLib"/>
//!   <Group location="container:..." name="Subgroup">
//!     <FileRef location="group:Baz.xcodeproj"/>
//!   </Group>
//! </Workspace>
//! ```
//!
//! Every member is a `<FileRef>`; only the path tells them apart. A local
//! package (`MyLib` above) points at the directory holding its `Package.swift`
//! and carries no extension at all.
//!
//! Location prefixes we handle: `container:` (workspace dir), `group:`
//! (parent-group dir, falling back to workspace dir), `absolute:` (absolute
//! path), `self:` (workspace dir), `developer:` (`DEVELOPER_DIR`).

use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::project;
use crate::xcode;
use crate::xcscheme::{self, Element};

#[derive(Debug, Clone)]
pub struct Workspace {
    /// `.xcworkspace` basename without extension (e.g. `Kingfisher`).
    pub name: String,
    /// Absolute path to the `.xcworkspace` directory.
    pub path: PathBuf,
    /// Absolute paths to every `.xcodeproj` referenced by the workspace,
    /// in declaration order.
    pub project_refs: Vec<PathBuf>,
    /// Absolute paths to every local SwiftPM package referenced by the
    /// workspace itself (a `FileRef` at a directory holding a
    /// `Package.swift`), in declaration order. A package's schemes are its
    /// manifest's products, and a manifest is Swift source that only the
    /// toolchain can evaluate — so this crate reports the membership and
    /// leaves naming to a caller that can run `swift package dump-package`.
    ///
    /// A member listed here is the workspace's own package, which is why it
    /// also gets a scheme per *test* target; the packages a member project
    /// declares ([`Workspace::project_package_refs`]) contribute products
    /// only.
    pub package_refs: Vec<PathBuf>,
    /// The workspace bundle's own scheme names (shared plus per-user files),
    /// sorted alphabetically. Member-project schemes are merged in by
    /// [`Workspace::merged_schemes`].
    pub schemes: Vec<String>,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Parse(xcscheme::Error),
    BadWorkspace(String),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<xcscheme::Error> for Error {
    fn from(e: xcscheme::Error) -> Self {
        Error::Parse(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Parse(e) => write!(f, "{e}"),
            Error::BadWorkspace(s) => write!(f, "invalid workspace: {s}"),
        }
    }
}

impl std::error::Error for Error {}

/// Open a `.xcworkspace` directory and extract referenced projects + schemes.
pub fn open(workspace_path: &Path) -> Result<Workspace, Error> {
    let contents = workspace_path.join("contents.xcworkspacedata");
    let root = xcscheme::parse_file(&contents)?;
    if root.name != "Workspace" {
        return Err(Error::BadWorkspace(format!(
            "expected root <Workspace>, got <{}>",
            root.name
        )));
    }

    // `group:` / `container:` references are anchored at the directory
    // *containing* the `.xcworkspace`, not the workspace bundle itself:
    // `Foo.xcworkspace/contents.xcworkspacedata` says `group:Foo.xcodeproj`
    // and the project lives next to (not inside) the workspace.
    let base = workspace_path
        .parent()
        .map_or_else(|| workspace_path.to_path_buf(), Path::to_path_buf);
    let mut project_refs = Vec::new();
    let mut package_refs = Vec::new();
    collect_member_refs(&root, &base, &base, &mut project_refs, &mut package_refs);
    // A workspace can declare the same `.xcodeproj` more than once (e.g. via a
    // group alias); Xcode lists it once. Drop duplicates, keep first-seen order.
    let mut seen = std::collections::HashSet::new();
    project_refs.retain(|p| seen.insert(p.clone()));
    let mut seen = std::collections::HashSet::new();
    package_refs.retain(|p| seen.insert(p.clone()));

    let schemes = crate::scheme::container_schemes(workspace_path);

    let name = workspace_path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_string();

    Ok(Workspace {
        name,
        path: workspace_path.to_path_buf(),
        project_refs,
        package_refs,
        schemes,
    })
}

impl Workspace {
    /// Schemes that `xcodebuild -list -workspace` would surface: the
    /// workspace's own schemes UNION every member project's schemes — scheme
    /// files plus each project's autocreated per-target schemes (see
    /// [`project::open`]) — deduplicated and sorted the way `xcodebuild`
    /// sorts (case-insensitively). When the shared workspace settings
    /// disable scheme autocreation (XcodeGen / Tuist write the flag), only
    /// scheme files are merged. Failures (missing project, unreadable
    /// directory) are skipped silently.
    #[must_use]
    pub fn merged_schemes(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> = self.schemes.iter().cloned().collect();
        let autocreate = crate::scheme::autocreation_allowed(&self.path);
        for project_path in &self.project_refs {
            if autocreate && let Ok(proj) = project::open(project_path) {
                set.extend(proj.schemes);
                continue;
            }
            set.extend(crate::scheme::container_schemes(project_path));
        }
        for package_path in &self.package_refs {
            set.extend(crate::scheme::container_schemes(&package_scheme_root(
                package_path,
            )));
        }
        let mut out: Vec<String> = set.into_iter().collect();
        crate::scheme::sort_like_xcodebuild(&mut out);
        out
    }

    /// [`Workspace::merged_schemes`] plus the schemes a caller resolved from
    /// the workspace's local package graph, each entry paired with the package
    /// directory it came from.
    ///
    /// The split exists because product names need `swift package
    /// dump-package` — see [`Workspace::package_refs`]. A caller with no
    /// resolved names gets exactly `merged_schemes`.
    ///
    /// The graph reaches past this workspace's own `FileRef` members — a
    /// member project's packages and everything those pull in through
    /// `.package(path:)` are in it too — and only a caller that evaluated the
    /// manifests knows how far it went, so the names are merged as given
    /// rather than filtered against [`Workspace::package_refs`].
    #[must_use]
    pub fn merged_schemes_with_packages(
        &self,
        package_schemes: &[(PathBuf, Vec<String>)],
    ) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> =
            self.merged_schemes().into_iter().collect();
        for (_, names) in package_schemes {
            set.extend(names.iter().cloned());
        }
        let mut out: Vec<String> = set.into_iter().collect();
        crate::scheme::sort_like_xcodebuild(&mut out);
        out
    }

    /// Distinct target names across every member project, in first-seen order
    /// (project declaration order, then each project's pbxproj target order).
    /// A workspace has no `xcodebuild -list` target output; this is what the
    /// extension needs to populate target pickers.
    #[must_use]
    pub fn merged_targets(&self) -> Vec<String> {
        self.merged_from_projects(|proj| proj.targets.into_iter().map(|t| t.name))
    }

    /// [`Workspace::merged_targets`] plus the targets a caller read from the
    /// workspace's local package graph — packages appended after the projects,
    /// in the order the caller resolved them.
    ///
    /// Target names need `swift package dump-package`; see
    /// [`Workspace::package_refs`]. A caller with nothing resolved gets
    /// exactly `merged_targets`. The names are merged as given, for the reason
    /// [`Workspace::merged_schemes_with_packages`] gives.
    #[must_use]
    pub fn merged_targets_with_packages(
        &self,
        package_targets: &[(PathBuf, Vec<String>)],
    ) -> Vec<String> {
        let mut out = self.merged_targets();
        let mut seen: std::collections::HashSet<String> = out.iter().cloned().collect();
        for (_, names) in package_targets {
            for name in names {
                if seen.insert(name.clone()) {
                    out.push(name.clone());
                }
            }
        }
        out
    }

    /// The local SwiftPM packages the member *projects* declare
    /// (`XCLocalSwiftPackageReference`), in project order then declaration
    /// order, with duplicates and this workspace's own `FileRef` members
    /// dropped.
    ///
    /// `xcodebuild -list -workspace` gives these packages' products schemes
    /// just like a `FileRef` member's, which is why they have to be resolved
    /// too — a package dragged into an app project rather than into the
    /// workspace is the common way to end up here. Projects that fail to open
    /// are skipped.
    #[must_use]
    pub fn project_package_refs(&self) -> Vec<PathBuf> {
        let mut seen: std::collections::HashSet<PathBuf> =
            self.package_refs.iter().cloned().collect();
        let mut out = Vec::new();
        for project_path in &self.project_refs {
            let Ok(proj) = project::open(project_path) else {
                continue;
            };
            for dir in proj.package_refs {
                if seen.insert(dir.clone()) {
                    out.push(dir);
                }
            }
        }
        out
    }

    /// Distinct build-configuration names across every member project, in
    /// first-seen order.
    ///
    /// A local package contributes nothing here: SwiftPM has no Xcode
    /// configurations, and `xcodebuild` accepts any `-configuration` for a
    /// package scheme rather than matching it against a list. A workspace made
    /// only of packages therefore has no project to name one, so it falls back
    /// to the Debug/Release pair SwiftPM maps onto — leaving the picker empty
    /// would offer no way to build at all.
    #[must_use]
    pub fn merged_configurations(&self) -> Vec<String> {
        let from_projects = self.merged_from_projects(|proj| proj.configurations);
        if from_projects.is_empty() && !self.package_refs.is_empty() {
            return vec!["Debug".to_string(), "Release".to_string()];
        }
        from_projects
    }

    /// Collect a per-project string list across every member project,
    /// deduplicated in first-seen order. Projects that fail to open are skipped.
    fn merged_from_projects<I>(&self, pick: impl Fn(project::Project) -> I) -> Vec<String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for project_path in &self.project_refs {
            let Ok(proj) = project::open(project_path) else {
                continue;
            };
            for name in pick(proj) {
                if seen.insert(name.clone()) {
                    out.push(name);
                }
            }
        }
        out
    }

    /// Locate the `.xcodeproj` member that owns a scheme by name. Returns
    /// the first project with a `<name>.xcscheme` file (shared or per-user).
    /// Otherwise — under the same autocreation gate as
    /// [`Workspace::merged_schemes`] — falls back to the first project whose
    /// scheme list ([`project::open`]'s files-plus-autocreated set) includes
    /// the name: scheme files elsewhere do NOT suppress a member's
    /// autocreated per-target schemes, so every name `merged_schemes`
    /// surfaces must dispatch. Used by callers (the CLI) that need to route
    /// a scheme-driven build to the right project.
    #[must_use]
    pub fn project_for_scheme(&self, scheme_name: &str) -> Option<&Path> {
        if let Some(p) = self
            .project_refs
            .iter()
            .find(|p| crate::scheme::find_scheme_file(p, scheme_name).is_some())
        {
            return Some(p.as_path());
        }
        if !crate::scheme::autocreation_allowed(&self.path) {
            return None;
        }
        self.project_refs
            .iter()
            .find(|p| {
                project::open(p)
                    .map(|proj| proj.schemes.iter().any(|s| s == scheme_name))
                    .unwrap_or(false)
            })
            .map(PathBuf::as_path)
    }
}

/// The scheme container inside a SwiftPM package. Xcode keeps a package's
/// schemes under `.swiftpm/xcode` rather than in the package directory
/// itself, so [`crate::scheme::container_schemes`] has to be pointed at that
/// subdirectory to find them.
#[must_use]
pub fn package_scheme_root(package_dir: &Path) -> PathBuf {
    package_dir.join(".swiftpm/xcode")
}

fn collect_member_refs(
    element: &Element,
    group_base: &Path,
    container_base: &Path,
    projects: &mut Vec<PathBuf>,
    packages: &mut Vec<PathBuf>,
) {
    for child in &element.children {
        match child.name.as_str() {
            "FileRef" => {
                let Some(location) = child.attr("location") else {
                    continue;
                };
                let Some(path) = resolve_location(location, group_base, container_base) else {
                    continue;
                };
                if path.extension().and_then(OsStr::to_str) == Some("xcodeproj") {
                    projects.push(path);
                } else if path.join("Package.swift").is_file() {
                    // A local SwiftPM package joins a workspace as a `FileRef`
                    // at its directory, with no extension to recognize it by —
                    // the manifest inside is what makes it a member.
                    packages.push(path);
                }
            }
            "Group" => {
                // A Group's `location` (when present) re-anchors its
                // children's `group:` references. Without one, children
                // resolve against the same base as their parent. The
                // container anchor never moves — `container:` is always
                // relative to the workspace's own directory.
                let child_base = child
                    .attr("location")
                    .and_then(|loc| resolve_location(loc, group_base, container_base))
                    .unwrap_or_else(|| group_base.to_path_buf());
                collect_member_refs(child, &child_base, container_base, projects, packages);
            }
            _ => {}
        }
    }
}

/// Resolve a `location="<prefix>:<rest>"` to an absolute path. `group:` is
/// relative to the enclosing group's resolved location (`group_base`);
/// `container:` / `self:` are relative to the directory containing the
/// workspace (`container_base`), regardless of group nesting.
fn resolve_location(location: &str, group_base: &Path, container_base: &Path) -> Option<PathBuf> {
    let (prefix, rest) = location.split_once(':')?;
    match prefix {
        "group" => Some(group_base.join(rest)),
        "container" | "self" => Some(container_base.join(rest)),
        // `absolute:` usually carries an absolute path, but Xcode also permits a
        // relative one (e.g. `absolute:../Foo.xcodeproj`); anchor those at the
        // workspace dir, matching CocoaPods' `File.expand_path`.
        "absolute" => {
            let p = PathBuf::from(rest);
            Some(if p.is_absolute() {
                p
            } else {
                container_base.join(rest)
            })
        }
        "developer" => Some(xcode::detect_developer_dir().join(rest)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixtures_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    #[test]
    fn opens_kingfisher_workspace() {
        let ws_path = fixtures_root().join("kingfisher/xcode-26.5.0/raw/Kingfisher.xcworkspace");
        let ws = open(&ws_path).unwrap();
        assert_eq!(ws.name, "Kingfisher");
        let names: Vec<&str> = ws
            .project_refs
            .iter()
            .map(|p| p.file_name().and_then(OsStr::to_str).unwrap_or(""))
            .collect();
        assert_eq!(
            names,
            vec!["Kingfisher.xcodeproj", "Kingfisher-Demo.xcodeproj"]
        );
        // Both referenced projects should resolve to existing directories
        // on disk.
        for p in &ws.project_refs {
            assert!(p.exists(), "expected {} to exist", p.display());
        }
    }

    #[test]
    fn opens_alamofire_workspace_with_three_projects() {
        let ws_path = fixtures_root().join("alamofire/xcode-26.5.0/raw/Alamofire.xcworkspace");
        let ws = open(&ws_path).unwrap();
        let names: Vec<&str> = ws
            .project_refs
            .iter()
            .map(|p| p.file_name().and_then(OsStr::to_str).unwrap_or(""))
            .collect();
        assert_eq!(
            names,
            vec![
                "Alamofire.xcodeproj",
                "iOS Example.xcodeproj",
                "watchOS Example.xcodeproj",
            ]
        );
    }

    #[test]
    fn resolves_location_prefixes() {
        // `group:` anchors at the enclosing group's dir; `container:`,
        // `self:`, and relative `absolute:` anchor at the directory
        // containing the .xcworkspace bundle.
        let group = PathBuf::from("/tmp/parent/Sub");
        let container = PathBuf::from("/tmp/parent");
        assert_eq!(
            resolve_location("container:Foo.xcodeproj", &group, &container),
            Some(PathBuf::from("/tmp/parent/Foo.xcodeproj")),
        );
        assert_eq!(
            resolve_location("group:Sub/Bar.xcodeproj", &group, &container),
            Some(PathBuf::from("/tmp/parent/Sub/Sub/Bar.xcodeproj")),
        );
        assert_eq!(
            resolve_location("absolute:/abs/path/Baz.xcodeproj", &group, &container),
            Some(PathBuf::from("/abs/path/Baz.xcodeproj")),
        );
        // A relative `absolute:` rest anchors at the workspace dir.
        assert_eq!(
            resolve_location("absolute:../Rel.xcodeproj", &group, &container),
            Some(PathBuf::from("/tmp/parent/../Rel.xcodeproj")),
        );
        assert_eq!(
            resolve_location("self:nested", &group, &container),
            Some(PathBuf::from("/tmp/parent/nested")),
        );
        // `developer:` resolves under the active DEVELOPER_DIR.
        assert!(
            resolve_location("developer:Tools/foo", &group, &container)
                .is_some_and(|p| p.ends_with("Tools/foo"))
        );
        assert!(resolve_location("bogus:nope", &group, &container).is_none());
    }

    #[test]
    fn dedups_and_resolves_nested_group_refs() {
        let ws_path = fixtures_root().join("_synthetic-workspace/Dup.xcworkspace");
        let ws = open(&ws_path).unwrap();
        let base = ws_path.parent().unwrap();
        // The duplicate `App.xcodeproj` collapses to one; the nested group
        // re-anchors its child under `Sub/`.
        assert_eq!(
            ws.project_refs,
            vec![
                base.join("App.xcodeproj"),
                base.join("Sub/Nested.xcodeproj")
            ],
        );
    }

    #[test]
    fn container_refs_anchor_at_workspace_dir_even_inside_groups() {
        // `container:` is always relative to the directory containing the
        // workspace; only `group:` re-anchors with the enclosing Group.
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("sweetpad-ws-container-{}-{n}", std::process::id()));
        let ws = root.join("Test.xcworkspace");
        fs::create_dir_all(&ws).unwrap();
        fs::write(
            ws.join("contents.xcworkspacedata"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Workspace version="1.0">
  <Group location="group:Sub" name="Sub">
    <FileRef location="container:App.xcodeproj"/>
    <FileRef location="group:Nested.xcodeproj"/>
  </Group>
</Workspace>
"#,
        )
        .unwrap();
        let ws = open(&ws).unwrap();
        assert_eq!(
            ws.project_refs,
            vec![
                root.join("App.xcodeproj"),
                root.join("Sub/Nested.xcodeproj")
            ],
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// A scratch workspace under the OS temp dir containing one copy of the
    /// synthetic `Scratch.xcodeproj` (a single `Scratch` target, no scheme
    /// files), referenced via `group:`.
    fn scratch_workspace(tag: &str) -> (PathBuf, PathBuf) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("sweetpad-ws-{tag}-{}-{n}", std::process::id()));
        let proj = root.join("Scratch.xcodeproj");
        fs::create_dir_all(&proj).unwrap();
        fs::copy(
            fixtures_root().join(
                "_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj/project.pbxproj",
            ),
            proj.join("project.pbxproj"),
        )
        .unwrap();
        let ws = root.join("Test.xcworkspace");
        fs::create_dir_all(&ws).unwrap();
        fs::write(
            ws.join("contents.xcworkspacedata"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Workspace version=\"1.0\">\n  <FileRef location=\"group:Scratch.xcodeproj\"/>\n</Workspace>\n",
        )
        .unwrap();
        (ws, proj)
    }

    /// Add a local SwiftPM package next to `ws_path`'s project and reference
    /// it the way Xcode does: a `FileRef` at the package *directory*, with no
    /// extension marking it as anything.
    fn add_package(ws_path: &Path, name: &str) -> PathBuf {
        let root = ws_path.parent().unwrap();
        let pkg = root.join(name);
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("Package.swift"), b"// swift-tools-version: 5.9\n").unwrap();
        fs::write(
            ws_path.join("contents.xcworkspacedata"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Workspace version=\"1.0\">\n  <FileRef location=\"group:Scratch.xcodeproj\"/>\n  <FileRef location=\"group:{name}\"/>\n</Workspace>\n"
            ),
        )
        .unwrap();
        pkg
    }

    #[test]
    fn a_local_package_member_is_kept_apart_from_the_projects() {
        let (ws_path, _proj) = scratch_workspace("pkg-refs");
        let pkg = add_package(&ws_path, "MyLib");
        let ws = open(&ws_path).unwrap();
        assert_eq!(
            ws.project_refs,
            vec![ws_path.parent().unwrap().join("Scratch.xcodeproj")]
        );
        assert_eq!(ws.package_refs, vec![pkg]);
        let _ = fs::remove_dir_all(ws_path.parent().unwrap());
    }

    #[test]
    fn a_directory_without_a_manifest_is_not_a_package_member() {
        let (ws_path, _proj) = scratch_workspace("pkg-nomanifest");
        let pkg = add_package(&ws_path, "NotAPackage");
        fs::remove_file(pkg.join("Package.swift")).unwrap();
        let ws = open(&ws_path).unwrap();
        // A `FileRef` at a plain folder (a group of loose files) is neither a
        // project nor a package, and must not be reported as either.
        assert!(ws.package_refs.is_empty());
        let _ = fs::remove_dir_all(ws_path.parent().unwrap());
    }

    #[test]
    fn a_packages_own_scheme_files_live_under_dot_swiftpm() {
        let (ws_path, _proj) = scratch_workspace("pkg-schemefiles");
        let pkg = add_package(&ws_path, "MyLib");
        let shared = package_scheme_root(&pkg).join("xcshareddata/xcschemes");
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("CustomLib.xcscheme"), b"").unwrap();

        let ws = open(&ws_path).unwrap();
        // Read straight from the package with no manifest evaluation — a
        // customized scheme is a file like any other.
        assert_eq!(ws.merged_schemes(), vec!["CustomLib", "Scratch"]);
        let _ = fs::remove_dir_all(ws_path.parent().unwrap());
    }

    #[test]
    fn resolved_package_schemes_merge_in_from_across_the_graph() {
        let (ws_path, _proj) = scratch_workspace("pkg-merge");
        let pkg = add_package(&ws_path, "MyLib");
        let ws = open(&ws_path).unwrap();

        let resolved = vec![
            (pkg, vec!["LibA".to_string(), "LibB".to_string()]),
            // A package reached through a member's `.package(path:)` is no
            // `FileRef` of this workspace, and its products are schemes all
            // the same.
            (
                PathBuf::from("/elsewhere/Nested"),
                vec!["NestedLib".to_string()],
            ),
        ];
        assert_eq!(
            ws.merged_schemes_with_packages(&resolved),
            vec!["LibA", "LibB", "NestedLib", "Scratch"]
        );
        // With nothing resolved it degrades to exactly `merged_schemes`.
        assert_eq!(ws.merged_schemes_with_packages(&[]), ws.merged_schemes());
        let _ = fs::remove_dir_all(ws_path.parent().unwrap());
    }

    #[test]
    fn resolved_package_targets_append_after_the_projects() {
        let (ws_path, _proj) = scratch_workspace("pkg-targets");
        let pkg = add_package(&ws_path, "MyLib");
        let ws = open(&ws_path).unwrap();

        let resolved = vec![
            (
                pkg,
                vec![
                    "LibA".to_string(),
                    // A test target belongs in a target list even though it is
                    // never a scheme.
                    "LibATests".to_string(),
                ],
            ),
            (
                PathBuf::from("/elsewhere/Nested"),
                vec!["NestedLib".to_string()],
            ),
        ];
        assert_eq!(
            ws.merged_targets_with_packages(&resolved),
            vec!["Scratch", "LibA", "LibATests", "NestedLib"]
        );
        assert_eq!(ws.merged_targets_with_packages(&[]), ws.merged_targets());
        let _ = fs::remove_dir_all(ws_path.parent().unwrap());
    }

    /// A workspace holding the `_synthetic-spm` fixture project, which
    /// declares `XCLocalSwiftPackageReference "Dep"`. `absolute:` keeps the
    /// scratch workspace from having to copy the fixture.
    fn workspace_over_the_spm_fixture(tag: &str, also_reference_the_package: bool) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let spm = fixtures_root().join("_synthetic-spm/project");
        let root =
            std::env::temp_dir().join(format!("sweetpad-ws-{tag}-{}-{n}", std::process::id()));
        let ws_path = root.join("Test.xcworkspace");
        fs::create_dir_all(&ws_path).unwrap();
        let package_ref = if also_reference_the_package {
            format!(
                "  <FileRef location=\"absolute:{}\"/>\n",
                spm.join("Dep").display()
            )
        } else {
            String::new()
        };
        fs::write(
            ws_path.join("contents.xcworkspacedata"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Workspace version=\"1.0\">\n  <FileRef location=\"absolute:{}\"/>\n{package_ref}</Workspace>\n",
                spm.join("SpmApp.xcodeproj").display()
            ),
        )
        .unwrap();
        ws_path
    }

    #[test]
    fn a_member_projects_local_packages_are_reported_apart_from_the_workspaces_own() {
        let ws_path = workspace_over_the_spm_fixture("project-pkg", false);
        let ws = open(&ws_path).unwrap();

        // The workspace declares no package itself; the project it holds does,
        // and `xcodebuild -list` gives that package's products schemes too.
        assert!(ws.package_refs.is_empty());
        assert_eq!(
            ws.project_package_refs(),
            vec![fixtures_root().join("_synthetic-spm/project/Dep")]
        );
        let _ = fs::remove_dir_all(ws_path.parent().unwrap());
    }

    #[test]
    fn a_package_the_workspace_already_names_is_not_reported_twice() {
        let ws_path = workspace_over_the_spm_fixture("both-ways", true);
        let ws = open(&ws_path).unwrap();

        // Referenced by the workspace *and* by its project: the membership
        // wins, since that is the role that also makes test targets schemes.
        assert_eq!(
            ws.package_refs,
            vec![fixtures_root().join("_synthetic-spm/project/Dep")]
        );
        assert!(ws.project_package_refs().is_empty());
        let _ = fs::remove_dir_all(ws_path.parent().unwrap());
    }

    #[test]
    fn a_package_only_workspace_falls_back_to_debug_and_release() {
        let (ws_path, _proj) = scratch_workspace("pkg-only-configs");
        add_package(&ws_path, "MyLib");
        // Drop the project reference so packages are the only members.
        fs::write(
            ws_path.join("contents.xcworkspacedata"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Workspace version=\"1.0\">\n  <FileRef location=\"group:MyLib\"/>\n</Workspace>\n",
        )
        .unwrap();
        let ws = open(&ws_path).unwrap();
        assert!(ws.project_refs.is_empty());
        assert_eq!(ws.merged_configurations(), vec!["Debug", "Release"]);
        let _ = fs::remove_dir_all(ws_path.parent().unwrap());
    }

    #[test]
    fn a_package_never_adds_to_a_projects_configurations() {
        let (ws_path, _proj) = scratch_workspace("mixed-configs");
        let without_package = open(&ws_path).unwrap().merged_configurations();
        add_package(&ws_path, "MyLib");
        let with_package = open(&ws_path).unwrap();

        assert!(!with_package.package_refs.is_empty());
        assert!(!without_package.is_empty());
        // The fallback applies only when no project names a configuration.
        assert_eq!(with_package.merged_configurations(), without_package);
        let _ = fs::remove_dir_all(ws_path.parent().unwrap());
    }

    #[test]
    fn merged_schemes_autocreates_per_target_when_no_scheme_files() {
        let (ws_path, _proj) = scratch_workspace("autocreate");
        let ws = open(&ws_path).unwrap();
        // Neither the workspace nor the project has any scheme file, so the
        // autocreated per-target schemes surface (matching xcodebuild -list).
        assert_eq!(ws.merged_schemes(), vec!["Scratch"]);
    }

    #[test]
    fn merged_schemes_includes_workspace_and_project_user_schemes() {
        let (ws_path, proj) = scratch_workspace("user-schemes");
        let user = crate::scheme::visible_user();
        let ws_user = ws_path.join(format!("xcuserdata/{user}.xcuserdatad/xcschemes"));
        fs::create_dir_all(&ws_user).unwrap();
        fs::write(ws_user.join("WsPersonal.xcscheme"), b"").unwrap();
        let proj_user = proj.join(format!("xcuserdata/{user}.xcuserdatad/xcschemes"));
        fs::create_dir_all(&proj_user).unwrap();
        fs::write(proj_user.join("ProjPersonal.xcscheme"), b"").unwrap();

        let ws = open(&ws_path).unwrap();
        // User schemes from both the workspace bundle and the member project,
        // plus the autocreated scheme for the project's Scratch target —
        // existing scheme files do NOT suppress per-target autocreation
        // (kingfisher's workspace captures list the schemeless demo apps
        // alongside the shared Kingfisher schemes).
        assert_eq!(
            ws.merged_schemes(),
            vec!["ProjPersonal", "Scratch", "WsPersonal"]
        );
        // And the user scheme makes the project dispatchable by name.
        assert_eq!(ws.project_for_scheme("ProjPersonal"), Some(proj.as_path()));
        // The autocreated scheme stays dispatchable even though scheme files
        // exist elsewhere — every name merged_schemes lists must resolve.
        assert_eq!(ws.project_for_scheme("Scratch"), Some(proj.as_path()));
    }

    #[test]
    fn project_for_scheme_falls_back_to_autocreated_target_schemes() {
        // No scheme file exists anywhere, so the autocreated per-target
        // scheme "Scratch" must dispatch to the member project owning the
        // same-named target.
        let (ws_path, proj) = scratch_workspace("autocreate-dispatch");
        let ws = open(&ws_path).unwrap();
        assert_eq!(ws.project_for_scheme("Scratch"), Some(proj.as_path()));
        assert_eq!(ws.project_for_scheme("NotATarget"), None);
    }

    #[test]
    fn rejects_non_workspace_root() {
        let scheme_path = fixtures_root().join(
            "kingfisher/xcode-26.5.0/raw/Kingfisher.xcodeproj/xcshareddata/xcschemes/Kingfisher.xcscheme",
        );
        let err = open(scheme_path.parent().unwrap()).unwrap_err();
        // The path doesn't have contents.xcworkspacedata — either I/O error
        // or BadWorkspace, but not Ok.
        assert!(matches!(
            err,
            Error::Io(_) | Error::Parse(_) | Error::BadWorkspace(_)
        ));
    }
}
