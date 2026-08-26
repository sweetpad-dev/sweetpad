//! What the local SwiftPM packages around an Xcode container contribute:
//! scheme and target names.
//!
//! `sweetpad_lib` reports which directories are packages
//! ([`sweetpad_lib::workspace::Workspace::package_refs`],
//! [`sweetpad_lib::project::Project::package_refs`]) but cannot name what they
//! hold: a package's schemes and targets come from its manifest, and
//! `Package.swift` is Swift source that only the toolchain can evaluate.
//! `swift package dump-package` compiles it against `libPackageDescription`
//! and runs it, so reading the manifest means spawning — which belongs here
//! rather than in the file-format crate.
//!
//! **The graph is wider than the workspace's own members.** Xcode resolves the
//! whole local package graph and gives every package in it schemes: the
//! `FileRef` members of the `.xcworkspace`, the packages a member project
//! declares (`XCLocalSwiftPackageReference` — dragging a package into an app
//! project rather than into the workspace), and everything those reach through
//! `.package(path:)`. Only a manifest names a path dependency, so the walk
//! lives here, one round of concurrent dumps per level.
//!
//! **Every local package contributes its products and its scheme files; only
//! some contribute their test targets.** Measured against `xcodebuild -list`:
//!
//! | | products | scheme files | test targets |
//! |---|---|---|---|
//! | `FileRef` in the `.xcworkspace` | ✅ | ✅ | ✅ |
//! | package whose scheme container still resolves | ✅ | ✅ | ✅ |
//! | any other local package | ✅ | ✅ | — |
//!
//! A `.swiftpm/xcode` scheme container holding a scheme that names a buildable
//! the manifest still declares is Xcode's mark that somebody opened the
//! package in it, which promotes the package from a dependency to something
//! you build and test in its own right. ice-cubes' `Packages/Env` ships one
//! `Env.xcscheme` naming its `Env` target, and `xcodebuild` lists `EnvTests`
//! next to it; move that container away and neither appears. Its
//! `Packages/NetworkClient` ships only a `NetworkTests.xcscheme` left over
//! from a rename — it names a target no longer in the manifest, and
//! `xcodebuild` autocreates nothing for that package.
//!
//! A target that no product exposes never gets a scheme, so a package that
//! declares no products contributes nothing but its test targets — and,
//! without a container or a membership, nothing at all. An `executableTarget`
//! is exposed even when the manifest says nothing: SwiftPM synthesizes a
//! product of the same name for it, and `xcodebuild` schedules a scheme for
//! that product like any other.
//!
//! The `<name>-Package` aggregate belongs to a package opened on its own;
//! `xcodebuild` does not synthesize it for a package inside a container, so
//! listing it here would offer a scheme `xcodebuild` then rejects.
//!
//! Not modeled: a *remote* package's scheme files. `xcodebuild` lists those
//! too (ice-cubes gets `RevenueCatUI` from the RevenueCat checkout's
//! container), but they live in a `SourcePackages` checkout under DerivedData
//! that only a resolved build knows the path to.
//!
//! **Targets include test targets** in every case, unlike schemes: a target
//! list exists to drive `-only-testing:`, where a test target is the point.
//!
//! **The spawn is slow enough to need a cache.** A cold `dump-package` takes
//! seconds (SwiftPM compiles the manifest); a warm one still costs the `swift`
//! driver's startup. Results are memoized on disk against the manifest's
//! `(len, mtime)` — the same stamp `sweetpad_lib`'s parse caches use — so the
//! cost is paid once per manifest edit instead of once per command. The CLI is
//! one-shot, so an in-process memo alone would never hit.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use sweetpad_lib::project::Project;
use sweetpad_lib::workspace::{Workspace, package_scheme_root};

/// How a package was reached. A workspace member's test targets are schemes
/// whether or not it has been opened in Xcode; every other package's are only
/// once it has (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageRole {
    /// A `FileRef` in the `.xcworkspace` — the workspace's own package.
    WorkspaceMember,
    /// Reached from a project's `XCLocalSwiftPackageReference` or its group
    /// tree, or as another package's `.package(path:)` dependency.
    Dependency,
}

/// What one local package contributes to the container that reached it.
#[derive(Debug, Clone)]
pub struct PackageMember {
    /// The package directory, canonicalized so the same package reached two
    /// ways is resolved once.
    pub path: PathBuf,
    /// How the walk reached this package.
    pub role: PackageRole,
    /// Scheme names — the package's scheme files and products, plus its test
    /// targets where those count (see the module docs).
    pub schemes: Vec<String>,
    /// Every target the manifest declares, test targets included.
    pub targets: Vec<String>,
}

/// `(len, mtime_nanos)` of a `Package.swift` — changed either way means the
/// cached names are stale.
type Stamp = (u64, u128);

/// Which reading of the cached fields an entry holds. Bump it whenever a field
/// starts meaning something different, so entries a build with the older
/// reading wrote are a miss rather than trusted: the stamp catches an edited
/// manifest, and only this catches an unedited one whose names this code
/// derives differently — `products`, for one, counts implicit executables that
/// a plain read of `dump-package` does not.
const CACHE_SCHEMA: u64 = 1;

/// What one manifest says, before a role turns it into schemes.
#[derive(Debug, Clone)]
struct ManifestNames {
    products: Vec<String>,
    test_targets: Vec<String>,
    targets: Vec<String>,
    /// Absolute directories of the manifest's `.package(path:)` dependencies.
    path_deps: Vec<PathBuf>,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((meta.len(), mtime))
}

/// Resolve symlinks and `..` so the same package reached through a workspace
/// `FileRef` and through a sibling's `.package(path:)` is one cache key and
/// one dump. Falls back to the path as given when it cannot be canonicalized.
fn canonical(dir: &Path) -> PathBuf {
    fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// `<state>/sweetpad/package-members.json` — one object keyed by absolute
/// package path. A single file (rather than one per package) keeps the cache
/// inspectable and its rewrite atomic.
fn cache_file() -> Option<PathBuf> {
    crate::paths::sweetpad_state_dir().map(|d| d.join("package-members.json"))
}

fn read_cache() -> BTreeMap<String, Value> {
    let Some(path) = cache_file() else {
        return BTreeMap::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Rewrite the cache via a temp file + rename so a concurrent reader never
/// sees a half-written object. A failure here is silent: the cache is an
/// optimization, and a command that resolved its schemes should not fail
/// because it could not record them.
fn write_cache(entries: &BTreeMap<String, Value>) {
    let Some(path) = cache_file() else {
        return;
    };
    let Some(dir) = path.parent() else {
        return;
    };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = path.with_extension(format!("json.tmp{}", std::process::id()));
    let Ok(text) = serde_json::to_string(entries) else {
        return;
    };
    let written = fs::File::create(&tmp).and_then(|mut f| f.write_all(text.as_bytes()));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
        return;
    }
    if fs::rename(&tmp, &path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

fn string_list(entry: &Value, key: &str) -> Option<Vec<String>> {
    Some(
        entry
            .get(key)?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

/// A cached entry is usable only when the manifest stamp still matches *and*
/// it carries every field this version reads — an entry written by a build
/// that recorded fewer fields is treated as a miss rather than as a package
/// with nothing in it.
fn cached_manifest(
    entries: &BTreeMap<String, Value>,
    path: &Path,
    current: Stamp,
) -> Option<ManifestNames> {
    let entry = entries.get(&path.to_string_lossy().into_owned())?;
    if entry.get("schema").and_then(Value::as_u64) != Some(CACHE_SCHEMA) {
        return None;
    }
    let len = entry.get("len")?.as_u64()?;
    let mtime = entry.get("mtime")?.as_str()?.parse::<u128>().ok()?;
    if (len, mtime) != current {
        return None;
    }
    Some(ManifestNames {
        products: string_list(entry, "products")?,
        test_targets: string_list(entry, "testTargets")?,
        targets: string_list(entry, "targets")?,
        path_deps: string_list(entry, "pathDependencies")?
            .into_iter()
            .map(PathBuf::from)
            .collect(),
    })
}

fn cache_entry(current: Stamp, names: &ManifestNames) -> Value {
    let (len, mtime) = current;
    serde_json::json!({
        "schema": CACHE_SCHEMA,
        "len": len,
        // u128 exceeds JSON's safe integer range; keep it as a string so a
        // round-trip can't quietly lose precision.
        "mtime": mtime.to_string(),
        "products": names.products,
        "testTargets": names.test_targets,
        "targets": names.targets,
        "pathDependencies": names.path_deps
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    })
}

/// The scheme names one package contributes: whatever its `.swiftpm/xcode`
/// container names, its products, and — for a workspace member, or a package
/// somebody has opened in Xcode — its test targets.
fn schemes_for(dir: &Path, role: PackageRole, names: &ManifestNames) -> Vec<String> {
    let files = sweetpad_lib::scheme::container_schemes(&package_scheme_root(dir));
    let opened_in_xcode = has_a_scheme_that_resolves(dir, &files, names);
    let mut out = files;
    out.extend(names.products.iter().cloned());
    if role == PackageRole::WorkspaceMember || opened_in_xcode {
        out.extend(names.test_targets.iter().cloned());
    }
    out.sort();
    out.dedup();
    out
}

/// Whether any scheme in the package's container names a buildable the
/// manifest still declares (see the module docs). A scheme that fails to parse
/// counts as not resolving, so a malformed file cannot turn autocreation on.
fn has_a_scheme_that_resolves(dir: &Path, files: &[String], names: &ManifestNames) -> bool {
    if files.is_empty() {
        return false;
    }
    let declared: HashSet<&str> = names
        .products
        .iter()
        .chain(names.targets.iter())
        .map(String::as_str)
        .collect();
    let root = package_scheme_root(dir);
    files.iter().any(|name| {
        sweetpad_lib::scheme::find_scheme_file(&root, name)
            .and_then(|path| sweetpad_lib::scheme::parse_file(&path).ok())
            .is_some_and(|scheme| {
                scheme
                    .build_entries
                    .iter()
                    .any(|entry| declared.contains(entry.buildable.blueprint_name.as_str()))
            })
    })
}

/// Walk the local package graph from `roots` and report what each package
/// contributes, roots first and then each level of `.package(path:)`
/// dependencies.
///
/// A package reached twice is resolved once, keeping the role it was first
/// reached with — so pass workspace members ahead of everything else, since
/// theirs is the role that adds test-target schemes.
///
/// Cache hits cost a single file read. Misses run `dump-package` for each
/// stale package in the level concurrently, so a graph pays one manifest
/// evaluation's latency per level rather than the sum. A package whose
/// manifest fails to evaluate contributes nothing and is not cached, so the
/// next call retries — a broken manifest is usually mid-edit.
#[must_use]
pub fn resolve(
    roots: &[(PathBuf, PackageRole)],
    developer_dir: Option<&Path>,
) -> Vec<PackageMember> {
    let mut entries = read_cache();
    let mut changed = false;
    let mut out: Vec<PackageMember> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    let mut level: Vec<(PathBuf, PackageRole)> = Vec::new();
    for (dir, role) in roots {
        let dir = canonical(dir);
        if seen.insert(dir.clone()) {
            level.push((dir, *role));
        }
    }

    while !level.is_empty() {
        let stamps: Vec<Option<Stamp>> = level
            .iter()
            .map(|(dir, _)| stamp(&dir.join("Package.swift")))
            .collect();
        let mut names: Vec<Option<ManifestNames>> = level
            .iter()
            .zip(&stamps)
            .map(|((dir, _), st)| st.and_then(|st| cached_manifest(&entries, dir, st)))
            .collect();

        let misses: Vec<usize> = (0..level.len())
            .filter(|&i| stamps[i].is_some() && names[i].is_none())
            .collect();
        let dumped: Vec<(usize, Option<ManifestNames>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = misses
                .iter()
                .map(|&i| {
                    let dir = level[i].0.clone();
                    scope.spawn(move || {
                        (
                            i,
                            dump_package(&dir, developer_dir).map(|m| read_manifest(&m)),
                        )
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        for (i, dumped) in dumped {
            let Some(dumped) = dumped else {
                continue;
            };
            if let Some(st) = stamps[i] {
                entries.insert(
                    level[i].0.to_string_lossy().into_owned(),
                    cache_entry(st, &dumped),
                );
                changed = true;
            }
            names[i] = Some(dumped);
        }

        let mut next: Vec<(PathBuf, PackageRole)> = Vec::new();
        for ((dir, role), names) in level.drain(..).zip(names) {
            let Some(names) = names else {
                continue;
            };
            for dep in &names.path_deps {
                let dep = canonical(dep);
                if seen.insert(dep.clone()) {
                    next.push((dep, PackageRole::Dependency));
                }
            }
            out.push(PackageMember {
                schemes: schemes_for(&dir, role, &names),
                path: dir,
                role,
                targets: names.targets,
            });
        }
        level = next;
    }

    if changed {
        write_cache(&entries);
    }
    out
}

/// Every local package a workspace draws schemes and targets from: its own
/// `FileRef` members first (so they keep the role that adds test-target
/// schemes), then the packages its member projects declare, then everything
/// those reach through `.package(path:)`.
#[must_use]
pub fn resolve_workspace(ws: &Workspace, developer_dir: Option<&Path>) -> Vec<PackageMember> {
    let mut roots: Vec<(PathBuf, PackageRole)> = ws
        .package_refs
        .iter()
        .map(|p| (p.clone(), PackageRole::WorkspaceMember))
        .collect();
    roots.extend(
        ws.project_package_refs()
            .into_iter()
            .map(|p| (p, PackageRole::Dependency)),
    );
    resolve(&roots, developer_dir)
}

/// Every local package a standalone `.xcodeproj` draws schemes and targets
/// from: the ones it declares, then everything those reach through
/// `.package(path:)`. None of them is a workspace member, so none contributes
/// test-target schemes.
#[must_use]
pub fn resolve_project(project: &Project, developer_dir: Option<&Path>) -> Vec<PackageMember> {
    let roots: Vec<(PathBuf, PackageRole)> = project
        .package_refs
        .iter()
        .map(|p| (p.clone(), PackageRole::Dependency))
        .collect();
    resolve(&roots, developer_dir)
}

/// The `(path, schemes)` pairs
/// [`sweetpad_lib::workspace::Workspace::merged_schemes_with_packages`] takes.
#[must_use]
pub fn scheme_pairs(members: &[PackageMember]) -> Vec<(PathBuf, Vec<String>)> {
    members
        .iter()
        .map(|m| (m.path.clone(), m.schemes.clone()))
        .collect()
}

/// The `(path, targets)` pairs
/// [`sweetpad_lib::workspace::Workspace::merged_targets_with_packages`] takes.
#[must_use]
pub fn target_pairs(members: &[PackageMember]) -> Vec<(PathBuf, Vec<String>)> {
    members
        .iter()
        .map(|m| (m.path.clone(), m.targets.clone()))
        .collect()
}

/// Run `swift package dump-package` in `dir` and parse its JSON. `None` on any
/// failure (no toolchain, manifest doesn't compile, unexpected output) — the
/// caller degrades to the names it can read from files.
fn dump_package(dir: &Path, developer_dir: Option<&Path>) -> Option<Value> {
    let mut cmd = Command::new("swift");
    if let Some(dev) = developer_dir {
        cmd.env("DEVELOPER_DIR", dev);
    }
    let output = cmd
        .args(["package", "dump-package"])
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // Skip any leading non-JSON chatter, like the CLI's other JSON readers.
    let start = text.find('{')?;
    serde_json::from_str(&text[start..]).ok()
}

fn read_manifest(manifest: &Value) -> ManifestNames {
    ManifestNames {
        products: products_with_implicit_executables(manifest),
        test_targets: names_in(manifest, "targets", is_test_target),
        targets: names_in(manifest, "targets", |_| true),
        path_deps: path_dependencies(manifest),
    }
}

/// The products `xcodebuild` gives schemes to: the ones the manifest declares,
/// plus the one SwiftPM synthesizes for each `executableTarget` that no
/// declared product already covers.
///
/// The implicit ones are as real as the rest — a package whose whole manifest
/// is `targets: [.executableTarget(name: "runner")]` contributes a `runner`
/// scheme to the container that reaches it. `swift package describe` reports
/// them, but it resolves the dependency graph to do so; `dump-package` only
/// evaluates the manifest, and reports what the manifest wrote. Reconstructing
/// them here keeps the walk offline.
fn products_with_implicit_executables(manifest: &Value) -> Vec<String> {
    let covered: HashSet<&str> = manifest
        .get("products")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("targets")?.as_array())
                .flatten()
                .filter_map(Value::as_str)
                .collect()
        })
        .unwrap_or_default();
    let mut out = names_in(manifest, "products", |_| true);
    out.extend(
        names_in(manifest, "targets", |target| {
            target.get("type").and_then(Value::as_str) == Some("executable")
        })
        .into_iter()
        .filter(|name| !covered.contains(name.as_str())),
    );
    out
}

fn names_in(manifest: &Value, key: &str, keep: impl Fn(&Value) -> bool) -> Vec<String> {
    manifest
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| keep(item))
                .filter_map(|item| item.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The directories of the manifest's `.package(path:)` dependencies.
/// `dump-package` writes them as `{"fileSystem": [{"path": "/abs/dir", …}]}`,
/// already absolute; a git dependency (`sourceControl`) lives in a checkout
/// Xcode manages and gets no schemes, so it is skipped.
fn path_dependencies(manifest: &Value) -> Vec<PathBuf> {
    let Some(deps) = manifest.get("dependencies").and_then(Value::as_array) else {
        return Vec::new();
    };
    deps.iter()
        .filter_map(|dep| dep.get("fileSystem")?.as_array())
        .flatten()
        .filter_map(|entry| entry.get("path")?.as_str())
        .map(PathBuf::from)
        .collect()
}

/// A manifest target is a test target when its `type` is the `test` tag —
/// a plain string in current dumps, a single-key union (`{"test":{}}`) in
/// older ones.
fn is_test_target(target: &Value) -> bool {
    match target.get("type") {
        Some(Value::Object(map)) => map.contains_key("test"),
        Some(Value::String(s)) => s == "test",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Value {
        serde_json::json!({
            "name": "MyLib",
            "products": [{"name": "LibA"}, {"name": "MyPlugin"}],
            "targets": [
                {"name": "LibA", "type": "regular"},
                {"name": "MyPlugin", "type": "plugin"},
                {"name": "LibATests", "type": "test"},
            ],
        })
    }

    /// A package directory with nothing in it but, optionally, a scheme
    /// container holding `<file>.xcscheme` naming `<blueprint>` — Xcode's mark
    /// that the package has been opened in it.
    fn package_dir(tag: &str, scheme_file: Option<(&str, &str)>) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sweetpad-pkg-{tag}-{}-{n}", std::process::id()));
        let schemes = package_scheme_root(&dir).join("xcshareddata/xcschemes");
        fs::create_dir_all(&schemes).unwrap();
        if let Some((file, blueprint)) = scheme_file {
            fs::write(
                schemes.join(format!("{file}.xcscheme")),
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme version = "1.7">
   <BuildAction>
      <BuildActionEntries>
         <BuildActionEntry buildForRunning = "YES">
            <BuildableReference
               BuildableIdentifier = "primary"
               BlueprintIdentifier = "{blueprint}"
               BuildableName = "{blueprint}"
               BlueprintName = "{blueprint}"
               ReferencedContainer = "container:">
            </BuildableReference>
         </BuildActionEntry>
      </BuildActionEntries>
   </BuildAction>
</Scheme>
"#
                ),
            )
            .unwrap();
        }
        dir
    }

    /// An `executableTarget` no declared product covers is a product all the
    /// same — `dump-package` does not write it, `xcodebuild -list` schedules a
    /// scheme for it, so the walk reconstructs it.
    #[test]
    fn an_executable_target_no_product_covers_is_a_product() {
        let names = read_manifest(&serde_json::json!({
            "name": "Tool",
            "products": [{"name": "tool", "targets": ["e1"]}],
            "targets": [
                {"name": "e1", "type": "executable"},
                {"name": "e2", "type": "executable"},
                {"name": "Core", "type": "regular"},
            ],
        }));
        assert_eq!(names.products, vec!["tool", "e2"]);
    }

    /// A manifest whose whole body is one `executableTarget` still contributes
    /// that target's scheme to the container that reached it.
    #[test]
    fn a_product_less_executable_package_still_has_a_scheme() {
        let dir = package_dir("exec-only", None);
        let names = read_manifest(&serde_json::json!({
            "name": "Tool",
            "products": [],
            "targets": [{"name": "runner", "type": "executable"}],
        }));
        assert_eq!(
            schemes_for(&dir, PackageRole::Dependency, &names),
            vec!["runner"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_workspace_member_adds_its_test_targets_to_its_products() {
        let dir = package_dir("member", None);
        let names = read_manifest(&manifest());
        assert_eq!(
            schemes_for(&dir, PackageRole::WorkspaceMember, &names),
            vec!["LibA", "LibATests", "MyPlugin"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dependency_contributes_products_only() {
        let dir = package_dir("dependency", None);
        let names = read_manifest(&manifest());
        assert_eq!(
            schemes_for(&dir, PackageRole::Dependency, &names),
            vec!["LibA", "MyPlugin"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dependency_opened_in_xcode_adds_its_scheme_files_and_test_targets() {
        let dir = package_dir("opened", Some(("LibA", "LibA")));
        let names = read_manifest(&manifest());
        assert_eq!(
            schemes_for(&dir, PackageRole::Dependency, &names),
            vec!["LibA", "LibATests", "MyPlugin"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_scheme_left_over_from_a_rename_does_not_turn_autocreation_on() {
        // ice-cubes' NetworkClient ships only `NetworkTests.xcscheme`, naming a
        // target its manifest no longer has; `xcodebuild` autocreates nothing
        // for it, so its `NetworkClientTests` never becomes a scheme.
        let dir = package_dir("stale", Some(("NetworkTests", "NetworkTests")));
        let names = read_manifest(&manifest());
        assert_eq!(
            schemes_for(&dir, PackageRole::Dependency, &names),
            vec!["LibA", "MyPlugin", "NetworkTests"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_scheme_file_naming_nothing_in_the_manifest_is_a_scheme_all_the_same() {
        // ice-cubes' StatusKit ships `StatusKit-Package.xcscheme`, an
        // aggregate no product or target declares.
        let dir = package_dir("aggregate", Some(("MyLib-Package", "MyLib")));
        let names = read_manifest(&manifest());
        assert!(
            schemes_for(&dir, PackageRole::Dependency, &names)
                .contains(&"MyLib-Package".to_string())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn targets_keep_test_targets_that_a_dependency_role_drops_from_schemes() {
        assert_eq!(
            read_manifest(&manifest()).targets,
            vec!["LibA", "MyPlugin", "LibATests"]
        );
    }

    #[test]
    fn a_target_no_product_exposes_is_not_a_scheme() {
        let m = serde_json::json!({
            "name": "MyLib",
            "products": [],
            "targets": [
                {"name": "Core", "type": "regular"},
                {"name": "CoreTests", "type": "test"},
            ],
        });
        let names = read_manifest(&m);
        // A product-less package offers its tests to a workspace that owns it,
        // and nothing at all to a container that only depends on it.
        let dir = package_dir("no-products", None);
        assert_eq!(
            schemes_for(&dir, PackageRole::WorkspaceMember, &names),
            vec!["CoreTests"]
        );
        assert!(schemes_for(&dir, PackageRole::Dependency, &names).is_empty());
        let _ = fs::remove_dir_all(&dir);
        // The same manifest still reports both as targets.
        assert_eq!(names.targets, vec!["Core", "CoreTests"]);
    }

    #[test]
    fn a_union_typed_test_target_is_still_recognized() {
        let m = serde_json::json!({
            "name": "MyLib",
            "products": [],
            "targets": [{"name": "Core"}, {"name": "CoreTests", "type": {"test": {}}}],
        });
        let names = read_manifest(&m);
        assert_eq!(names.test_targets, vec!["CoreTests"]);
    }

    #[test]
    fn path_dependencies_are_followed_and_git_ones_are_not() {
        let m = serde_json::json!({
            "name": "MyLib",
            "dependencies": [
                {"fileSystem": [{"identity": "sibling", "path": "/pkgs/Sibling"}]},
                {"sourceControl": [{"identity": "alamofire", "location": {}}]},
            ],
        });
        assert_eq!(
            read_manifest(&m).path_deps,
            vec![PathBuf::from("/pkgs/Sibling")]
        );
    }

    #[test]
    fn a_cache_entry_is_reused_only_while_the_stamp_matches() {
        let names = read_manifest(&manifest());
        let mut entries = BTreeMap::new();
        entries.insert("/pkg".to_string(), cache_entry((10, 99), &names));

        let hit = cached_manifest(&entries, Path::new("/pkg"), (10, 99)).unwrap();
        assert_eq!(hit.products, vec!["LibA", "MyPlugin"]);
        assert_eq!(hit.test_targets, vec!["LibATests"]);
        assert!(cached_manifest(&entries, Path::new("/pkg"), (11, 99)).is_none());
        assert!(cached_manifest(&entries, Path::new("/pkg"), (10, 100)).is_none());
        assert!(cached_manifest(&entries, Path::new("/other"), (10, 99)).is_none());
    }

    #[test]
    fn an_entry_missing_a_field_is_a_miss_not_an_empty_list() {
        let mut entries = BTreeMap::new();
        entries.insert(
            "/pkg".to_string(),
            serde_json::json!({"len": 10, "mtime": "99", "products": ["LibA"]}),
        );
        assert!(cached_manifest(&entries, Path::new("/pkg"), (10, 99)).is_none());
    }
}
