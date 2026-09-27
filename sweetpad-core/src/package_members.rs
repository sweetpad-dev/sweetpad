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
//! A product one of those scheme files covers gets no scheme of its own
//! under its name: a scheme that builds a library product, or runs an
//! executable one, takes its place, the way it does for a project's targets
//! ([`sweetpad_lib::scheme::SchemeReferences`]). On Xcode 27.0 a project whose
//! local package ships a `Beta.xcscheme` building its `Zeta` library lists
//! `Beta` and no `Zeta`. A package opened on its own lists `Zeta` all the
//! same.
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
//! **A package opened on its own lists its own shape.** `xcodebuild -list` in
//! a package directory prints its scheme files beside what the manifest
//! synthesizes, all sorted case-insensitively ([`standalone`]; the shape
//! measured on Xcode 26.5, the order on 27.0):
//!
//! | products | synthesized schemes |
//! |---|---|
//! | none | `<name>-Package` alone |
//! | one | `<name>` alone, the package's own name whatever the product is called |
//! | two or more | `<name>-Package` plus one scheme per product |
//!
//! Its test targets are never schemes of their own, whatever its container
//! holds.
//!
//! **The spawn is slow enough to need a cache.** A cold `dump-package` takes
//! seconds (SwiftPM compiles the manifest); a warm one still costs the `swift`
//! driver's startup. Results are memoized on disk against the manifest's
//! `(len, mtime)` — the same stamp `sweetpad_lib`'s parse caches use — so the
//! cost is paid once per manifest edit instead of once per command. The CLI is
//! one-shot, so an in-process memo alone would never hit.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use sweetpad_lib::project::Project;
use sweetpad_lib::workspace::{Workspace, package_scheme_root};

use crate::scratch::ScratchDir;

/// The toolchain that evaluates a manifest. The default is the `swift` on
/// `PATH` under the process's own `DEVELOPER_DIR`, which is what the CLI
/// wants. The extension host sees neither the user's login shell nor its
/// settings, so it passes both.
#[derive(Debug, Clone, Default)]
pub struct Toolchain {
    /// The `swift` to run, when not the one on `PATH`.
    pub swift: Option<PathBuf>,
    /// The `DEVELOPER_DIR` to run it under.
    pub developer_dir: Option<PathBuf>,
}

impl Toolchain {
    /// The program to spawn.
    #[must_use]
    pub fn swift(&self) -> &Path {
        self.swift.as_deref().unwrap_or(Path::new("swift"))
    }
}

/// Why a manifest could not be read.
#[derive(Debug)]
pub enum DumpError {
    /// `swift` could not be spawned, or its scratch directory made.
    Spawn(io::Error),
    /// The dump ran and failed, most often a manifest that doesn't compile.
    Failed(ExitStatus),
    /// It printed no JSON object.
    NoJson,
    /// It printed JSON that doesn't parse.
    Parse(serde_json::Error),
}

impl fmt::Display for DumpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DumpError::Spawn(e) => write!(f, "running swift package dump-package: {e}"),
            DumpError::Failed(status) => {
                write!(f, "swift package dump-package exited with {status}")
            }
            DumpError::NoJson => f.write_str("swift package dump-package produced no JSON"),
            DumpError::Parse(e) => write!(f, "parsing swift package dump-package: {e}"),
        }
    }
}

impl std::error::Error for DumpError {}

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
const CACHE_SCHEMA: u64 = 2;

/// What one manifest says, before a role turns it into schemes.
#[derive(Debug, Clone)]
struct ManifestNames {
    /// The package's own name, which names its schemes when it is opened on
    /// its own.
    name: String,
    products: Vec<String>,
    /// The products that run: the declared executables and the implicit ones
    /// behind `executableTarget`s.
    executables: Vec<String>,
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
        name: entry.get("name")?.as_str()?.to_string(),
        products: string_list(entry, "products")?,
        executables: string_list(entry, "executables")?,
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
        "name": names.name,
        "products": names.products,
        "executables": names.executables,
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
    let root = package_scheme_root(dir);
    let files = sweetpad_lib::scheme::container_schemes(&root);
    let opened_in_xcode = has_a_scheme_that_resolves(dir, &files, names);
    let references = sweetpad_lib::scheme::SchemeReferences::of(&root);
    let uncovered: Vec<String> = names
        .products
        .iter()
        .filter(|product| {
            files.contains(product)
                || !references.cover(None, product, names.executables.contains(product))
        })
        .cloned()
        .collect();
    let mut out = files;
    out.extend(uncovered);
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
pub fn resolve(roots: &[(PathBuf, PackageRole)], toolchain: &Toolchain) -> Vec<PackageMember> {
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
                        (i, dump_package(&dir, toolchain).map(|m| read_manifest(&m)))
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
pub fn resolve_workspace(ws: &Workspace, toolchain: &Toolchain) -> Vec<PackageMember> {
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
    resolve(&roots, toolchain)
}

/// Every local package a standalone `.xcodeproj` draws schemes and targets
/// from: the ones it declares, then everything those reach through
/// `.package(path:)`. None of them is a workspace member, so none contributes
/// test-target schemes.
#[must_use]
pub fn resolve_project(project: &Project, toolchain: &Toolchain) -> Vec<PackageMember> {
    let roots: Vec<(PathBuf, PackageRole)> = project
        .package_refs
        .iter()
        .map(|p| (p.clone(), PackageRole::Dependency))
        .collect();
    resolve(&roots, toolchain)
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

/// The names a Swift package opened on its own offers, the way
/// `xcodebuild -list` prints them in its directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageNames {
    /// The package's name, from its manifest.
    pub name: String,
    /// Its `.swiftpm/xcode` scheme files and the schemes its manifest
    /// synthesizes (see the module docs), sorted like `xcodebuild -list`.
    pub schemes: Vec<String>,
    /// Every target the manifest declares, test targets included.
    pub targets: Vec<String>,
}

/// What the package in `dir` offers when opened on its own. The manifest is
/// read through the same cache as [`resolve`], so an unchanged
/// `Package.swift` costs no spawn. `stderr` takes the dump's diagnostics: the
/// CLI shows a manifest's compile errors, the extension drops them.
///
/// # Errors
///
/// When the manifest cannot be evaluated ([`DumpError`]).
pub fn standalone(
    dir: &Path,
    toolchain: &Toolchain,
    stderr: Stdio,
) -> Result<PackageNames, DumpError> {
    let dir = canonical(dir);
    let mut entries = read_cache();
    let current = stamp(&dir.join("Package.swift"));
    if let Some(names) = current.and_then(|st| cached_manifest(&entries, &dir, st)) {
        return Ok(standalone_names(&dir, names));
    }
    let names = read_manifest(&dump_manifest(&dir, toolchain, stderr)?);
    if let Some(st) = current {
        entries.insert(dir.to_string_lossy().into_owned(), cache_entry(st, &names));
        write_cache(&entries);
    }
    Ok(standalone_names(&dir, names))
}

/// [`standalone`] for a manifest already dumped: `dump` is the model
/// `swift package dump-package` printed for the package in `dir`.
#[must_use]
pub fn standalone_from_dump(dir: &Path, dump: &Value) -> PackageNames {
    standalone_names(dir, read_manifest(dump))
}

/// [`standalone`] once the manifest is read: the scheme files in `dir`'s
/// container join the synthesized schemes.
fn standalone_names(dir: &Path, names: ManifestNames) -> PackageNames {
    let mut schemes = sweetpad_lib::scheme::container_schemes(&package_scheme_root(dir));
    schemes.extend(names.standalone_schemes());
    sweetpad_lib::scheme::sort_like_xcodebuild(&mut schemes);
    schemes.dedup();
    PackageNames {
        name: names.name,
        schemes,
        targets: names.targets,
    }
}

/// Run `swift package dump-package` in `dir` and parse its JSON. `None` on any
/// failure (no toolchain, manifest doesn't compile, unexpected output) — the
/// caller degrades to the names it can read from files.
fn dump_package(dir: &Path, toolchain: &Toolchain) -> Option<Value> {
    dump_manifest(dir, toolchain, Stdio::null()).ok()
}

/// Evaluate the manifest in `dir` and return the model `dump-package` prints,
/// its stderr sent to `stderr`.
///
/// # Errors
///
/// When `swift` cannot run, the dump fails, or it prints no JSON object.
pub fn dump_manifest(dir: &Path, toolchain: &Toolchain, stderr: Stdio) -> Result<Value, DumpError> {
    let output = run_dump_package(dir, toolchain, stderr).map_err(DumpError::Spawn)?;
    if !output.status.success() {
        return Err(DumpError::Failed(output.status));
    }
    parse_dump(&String::from_utf8_lossy(&output.stdout))
}

/// The JSON object in a dump's stdout, after any leading non-JSON chatter the
/// toolchain prints ahead of it.
fn parse_dump(stdout: &str) -> Result<Value, DumpError> {
    let start = stdout.find('{').ok_or(DumpError::NoJson)?;
    serde_json::from_str(&stdout[start..]).map_err(DumpError::Parse)
}

/// Run `swift package dump-package` for the package in `dir`, its stderr sent
/// to `stderr`, and return what it printed.
///
/// SwiftPM creates its scratch directory even to only evaluate a manifest, so
/// the dump gets a throwaway one: reading a package never leaves a `.build/`
/// inside it. SwiftPM caches evaluated manifests per user, not in the scratch
/// directory, so a fresh one costs no re-evaluation.
///
/// The child's `TMPDIR` is that same throwaway directory. Each dump would
/// otherwise leave a `TemporaryDirectory.*` (from the `swiftc
/// -print-target-info` SwiftPM runs first) and a lock file named for the
/// scratch path in the user's `$TMPDIR`. SwiftPM keeps its
/// lock files for the shared manifest cache there too, so a dump does not
/// take the lock other SwiftPM processes hold; the cache is SQLite, which
/// serializes the writes itself.
///
/// # Errors
///
/// When the scratch directory cannot be made or `swift` cannot be spawned.
/// A dump that runs and fails is an `Ok` with a failed status.
pub fn run_dump_package(dir: &Path, toolchain: &Toolchain, stderr: Stdio) -> io::Result<Output> {
    // `resolve` runs a level's dumps concurrently; each gets its own.
    let scratch = ScratchDir::new("sweetpad-dump-package")?;
    let mut cmd = Command::new(toolchain.swift());
    if let Some(dev) = &toolchain.developer_dir {
        cmd.env("DEVELOPER_DIR", dev);
    }
    cmd.env("TMPDIR", scratch.as_os_str())
        .args(["package", "--scratch-path"])
        .arg(scratch.join("build"))
        .arg("dump-package")
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(stderr)
        .output()
}

fn read_manifest(manifest: &Value) -> ManifestNames {
    ManifestNames {
        name: manifest
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        products: products_with_implicit_executables(manifest),
        executables: executable_products(manifest),
        test_targets: names_in(manifest, "targets", is_test_target),
        targets: names_in(manifest, "targets", |_| true),
        path_deps: path_dependencies(manifest),
    }
}

impl ManifestNames {
    /// The schemes `xcodebuild` synthesizes for the package opened on its own
    /// (see the module docs). The single-product collapse is easy to get wrong
    /// in both directions: a package with one library product answers to
    /// neither that product's name nor the aggregate, and one whose only
    /// product is the implicit executable behind an `executableTarget`
    /// answers to the package name too.
    fn standalone_schemes(&self) -> Vec<String> {
        if self.products.len() == 1 {
            return vec![self.name.clone()];
        }
        let mut names = vec![format!("{}-Package", self.name)];
        names.extend(self.products.iter().cloned());
        names
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

/// The products that run: the declared executables (`"type": {"executable":
/// null}`), and the implicit product behind each `executableTarget` no
/// declared product covers.
fn executable_products(manifest: &Value) -> Vec<String> {
    let declared: Vec<String> = names_in(manifest, "products", |product| {
        product
            .get("type")
            .is_some_and(|t| t.get("executable").is_some())
    });
    let all_declared: HashSet<String> = names_in(manifest, "products", |_| true)
        .into_iter()
        .collect();
    let mut out = declared;
    out.extend(
        products_with_implicit_executables(manifest)
            .into_iter()
            .filter(|name| !all_declared.contains(name)),
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
    fn package_dir(tag: &str, scheme_file: Option<(&str, &str)>) -> ScratchDir {
        let dir = ScratchDir::new(&format!("sweetpad-pkg-{tag}")).unwrap();
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
    }

    fn names(manifest: &str) -> ManifestNames {
        read_manifest(&serde_json::from_str(manifest).unwrap())
    }

    #[test]
    fn two_products_get_the_aggregate_and_a_scheme_each() {
        let m = names(
            r#"{ "name": "Demo", "products": [
                     { "name": "DemoKit", "type": { "library": ["automatic"] }, "targets": ["DemoKit"] },
                     { "name": "demo", "type": { "executable": null }, "targets": ["demo"] } ],
                 "targets": [ { "name": "DemoKit", "type": "regular" },
                              { "name": "demo", "type": "executable" },
                              { "name": "DemoKitTests", "type": "test" } ] }"#,
        );
        assert_eq!(m.standalone_schemes(), ["Demo-Package", "DemoKit", "demo"]);
    }

    /// Grounded on `xcodebuild -list` (26.5): a package whose only product is
    /// a library answers to its own name, not the product's and not the
    /// aggregate's.
    #[test]
    fn a_single_product_collapses_to_the_package_name() {
        let m = names(
            r#"{ "name": "P", "products": [
                     { "name": "Lib", "type": { "library": ["automatic"] }, "targets": ["T"] } ],
                 "targets": [ { "name": "T", "type": "regular" } ] }"#,
        );
        assert_eq!(m.standalone_schemes(), ["P"]);
    }

    /// The implicit executable product SwiftPM synthesizes counts toward that
    /// collapse: `xcodebuild -list` on a package whose whole manifest is one
    /// `executableTarget` prints the package name alone.
    #[test]
    fn an_executable_target_counts_as_a_product() {
        let m = names(
            r#"{ "name": "MyTool", "products": [],
                 "targets": [ { "name": "runner", "type": "executable" } ] }"#,
        );
        assert_eq!(m.standalone_schemes(), ["MyTool"]);
    }

    /// One declared product plus an `executableTarget` it does not cover is
    /// two products, so the aggregate comes back and both are listed.
    #[test]
    fn an_uncovered_executable_target_is_a_product_of_its_own() {
        let m = names(
            r#"{ "name": "D", "products": [
                     { "name": "LibA", "type": { "library": ["automatic"] }, "targets": ["TA"] } ],
                 "targets": [ { "name": "TA", "type": "regular" },
                              { "name": "TC", "type": "executable" } ] }"#,
        );
        assert_eq!(m.standalone_schemes(), ["D-Package", "LibA", "TC"]);
    }

    #[test]
    fn a_package_with_no_products_offers_just_the_aggregate() {
        let m = names(
            r#"{ "name": "P", "products": [],
                 "targets": [ { "name": "Lib", "type": "regular" },
                              { "name": "LibTests", "type": "test" } ] }"#,
        );
        assert_eq!(m.standalone_schemes(), ["P-Package"]);
    }

    #[test]
    fn a_dump_skips_leading_noise_before_its_json() {
        let parsed = parse_dump("Fetching dependencies\n{\"name\": \"Demo\"}").unwrap();
        assert_eq!(parsed.get("name").and_then(Value::as_str), Some("Demo"));
        assert!(matches!(
            parse_dump("not json at all"),
            Err(DumpError::NoJson)
        ));
    }

    /// `xcodebuild -list` in a package that holds a `.swiftpm/xcode` scheme
    /// lists it beside the synthesized ones, all sorted case-insensitively
    /// (Xcode 27.0: `alpha`, `B10Multi-Package`, `Beta`, `runner`, `Zeta`).
    /// The test target stays out even though `Beta` names a declared target.
    #[test]
    fn a_standalone_package_lists_its_scheme_files_beside_its_synthesized_schemes() {
        let dir = package_dir("standalone", Some(("Beta", "Zeta")));
        let manifest = names(
            r#"{ "name": "B10Multi",
                 "products": [
                     { "name": "Zeta", "type": { "library": ["automatic"] }, "targets": ["Zeta"] },
                     { "name": "alpha", "type": { "library": ["automatic"] }, "targets": ["alpha"] } ],
                 "targets": [ { "name": "Zeta", "type": "regular" },
                              { "name": "alpha", "type": "regular" },
                              { "name": "runner", "type": "executable" },
                              { "name": "ZetaTests", "type": "test" } ] }"#,
        );
        let read = standalone_names(&dir, manifest);
        assert_eq!(read.name, "B10Multi");
        assert_eq!(
            read.schemes,
            ["alpha", "B10Multi-Package", "Beta", "runner", "Zeta"]
        );
        assert_eq!(read.targets, ["Zeta", "alpha", "runner", "ZetaTests"]);
    }

    #[test]
    fn a_workspace_member_adds_its_test_targets_to_its_products() {
        let dir = package_dir("member", None);
        let names = read_manifest(&manifest());
        assert_eq!(
            schemes_for(&dir, PackageRole::WorkspaceMember, &names),
            vec!["LibA", "LibATests", "MyPlugin"]
        );
    }

    #[test]
    fn a_dependency_contributes_products_only() {
        let dir = package_dir("dependency", None);
        let names = read_manifest(&manifest());
        assert_eq!(
            schemes_for(&dir, PackageRole::Dependency, &names),
            vec!["LibA", "MyPlugin"]
        );
    }

    #[test]
    fn a_dependency_opened_in_xcode_adds_its_scheme_files_and_test_targets() {
        let dir = package_dir("opened", Some(("LibA", "LibA")));
        let names = read_manifest(&manifest());
        assert_eq!(
            schemes_for(&dir, PackageRole::Dependency, &names),
            vec!["LibA", "LibATests", "MyPlugin"]
        );
    }

    /// A scheme that builds a library product takes its place in a container
    /// that reaches the package (Xcode 27.0: a project over a package whose
    /// `Beta.xcscheme` builds `Zeta` lists `Beta` and no `Zeta`), while the
    /// package opened on its own still lists the product.
    #[test]
    fn a_scheme_that_builds_a_library_product_takes_its_place() {
        let dir = package_dir("covered", Some(("Beta", "LibA")));
        let names = read_manifest(&manifest());
        assert_eq!(
            schemes_for(&dir, PackageRole::Dependency, &names),
            vec!["Beta", "LibATests", "MyPlugin"]
        );
        assert!(
            standalone_names(&dir, names)
                .schemes
                .contains(&"LibA".to_string())
        );
    }

    /// Only a scheme that runs an executable product takes its place; one
    /// that builds it leaves the product its scheme.
    #[test]
    fn a_scheme_that_only_builds_an_executable_product_leaves_it_its_scheme() {
        let dir = package_dir("builds-exec", Some(("Tools", "runner")));
        let names = read_manifest(&serde_json::json!({
            "name": "Tool",
            "products": [],
            "targets": [{"name": "runner", "type": "executable"}],
        }));
        assert_eq!(names.executables, vec!["runner"]);
        assert_eq!(
            schemes_for(&dir, PackageRole::Dependency, &names),
            vec!["Tools", "runner"]
        );
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

    #[test]
    fn a_dump_writes_nothing_into_the_package() {
        // `swift --version` leaves a temp dir in $TMPDIR.
        let tmp = ScratchDir::new("sweetpad-swift-probe").unwrap();
        let have_swift = Command::new("swift")
            .arg("--version")
            .env("TMPDIR", tmp.as_os_str())
            .output()
            .is_ok_and(|out| out.status.success());
        if !have_swift {
            eprintln!("skipping: needs the Swift toolchain to evaluate a manifest");
            return;
        }
        let dir = package_dir("dump", None);
        fs::write(
            dir.join("Package.swift"),
            "// swift-tools-version:5.9\nimport PackageDescription\n\
             let package = Package(name: \"Dumped\")\n",
        )
        .unwrap();
        let manifest = dump_package(&dir, &Toolchain::default()).expect("the manifest evaluates");
        assert_eq!(manifest.get("name").and_then(Value::as_str), Some("Dumped"));
        assert!(!dir.join(".build").exists());
    }
}
