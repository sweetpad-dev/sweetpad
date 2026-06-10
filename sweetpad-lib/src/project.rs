use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::destination::RunDestination;
use crate::pbxproj::{self, Value};
use crate::resolver;
use crate::xcconfig::{Assignment, Condition};
use crate::xcode_hash::derived_data_hash;

#[derive(Debug, Clone)]
pub struct Project {
    /// The .xcodeproj basename (e.g. `Kingfisher` for `Kingfisher.xcodeproj`).
    pub name: String,
    /// Absolute or working-relative path to the .xcodeproj directory.
    pub path: PathBuf,
    /// Targets in the order they appear in the pbxproj.
    pub targets: Vec<Target>,
    /// Project-level configuration names in pbxproj order (e.g. `Debug`, `Release`).
    pub configurations: Vec<String>,
    /// The project XCConfigurationList's `defaultConfigurationName` (usually
    /// `Release`) — the configuration xcodebuild falls back to when a requested
    /// name isn't in the list. `None` when the pbxproj doesn't declare one.
    pub default_configuration: Option<String>,
    /// Scheme names for this project, sorted alphabetically — the set
    /// `xcodebuild -list` prints: shared (`xcshareddata/xcschemes`) plus
    /// per-user (`xcuserdata/<user>.xcuserdatad/xcschemes`) scheme files,
    /// or one autocreated scheme per target when no scheme file exists at
    /// all (Xcode's scheme autocreation for fresh / never-shared projects).
    pub schemes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    /// The pbxproj `isa` value for this target (`PBXNativeTarget`,
    /// `PBXAggregateTarget`, `PBXLegacyTarget`).
    pub isa: String,
    /// `productType` for native targets (e.g. `com.apple.product-type.application`).
    /// `None` for aggregate/legacy targets that don't declare one.
    pub product_type: Option<String>,
    /// Configuration names declared on the target's `buildConfigurationList`,
    /// in pbxproj order. These usually mirror the project-level list.
    pub configurations: Vec<String>,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Parse(pbxproj::ParseError),
    BadProject(String),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<pbxproj::ParseError> for Error {
    fn from(e: pbxproj::ParseError) -> Self {
        Error::Parse(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Parse(e) => write!(f, "parse error: {e}"),
            Error::BadProject(s) => write!(f, "invalid project: {s}"),
        }
    }
}

impl std::error::Error for Error {}

/// Open an .xcodeproj directory and extract its high-level metadata: name,
/// targets, project configurations, and shared schemes.
pub fn open(xcodeproj_path: &Path) -> Result<Project, Error> {
    let value = parse_pbxproj(xcodeproj_path)?;
    open_from_value(&value, xcodeproj_path)
}

/// Like [`open`] but driven by an already-parsed pbxproj value. Use this when
/// the caller has cached the pbxproj parse for reuse across multiple queries
/// (e.g. [`crate::build_context::BuildContext`]).
pub fn open_from_value(value: &Value, xcodeproj_path: &Path) -> Result<Project, Error> {
    let (objects, project_obj) = project_root(value)?;

    let configurations = extract_project_configurations(objects, project_obj)?;
    let default_configuration = default_configuration_name(objects, project_obj);
    let targets = extract_targets(objects, project_obj)?;
    let mut schemes = crate::scheme::container_schemes(xcodeproj_path);
    if schemes.is_empty() && crate::scheme::autocreation_allowed(xcodeproj_path) {
        // No scheme file on disk anywhere (shared or per-user): mirror Xcode's
        // scheme autocreation — `xcodebuild -list` reports one scheme per
        // target — so fresh / never-shared projects still list schemes. When
        // the workspace settings disable autocreation (XcodeGen / Tuist write
        // the flag), `xcodebuild -list` shows no schemes and so do we.
        schemes = targets.iter().map(|t| t.name.clone()).collect();
        schemes.sort();
        schemes.dedup();
    }

    let name = xcodeproj_path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_string();

    Ok(Project {
        name,
        path: xcodeproj_path.to_path_buf(),
        targets,
        configurations,
        default_configuration,
        schemes,
    })
}

/// The `defaultConfigurationName` declared on a container's
/// `XCConfigurationList`, if any. Each list — the project's and every
/// target's — carries its own; xcodebuild falls back to it when asked for a
/// configuration name the list doesn't contain.
fn default_configuration_name(
    objects: &BTreeMap<String, Value>,
    container: &Value,
) -> Option<String> {
    container
        .get("buildConfigurationList")
        .and_then(Value::as_str)
        .and_then(|id| objects.get(id))
        .and_then(|list| list.get("defaultConfigurationName"))
        .and_then(Value::as_str)
        .map(String::from)
}

/// Parse the `project.pbxproj` under an `.xcodeproj` directory. Served from a
/// shared, mtime-validated cache (see [`pbxproj::parse_file_cached`]) so the
/// same project parsed across many calls reuses the AST; the `Arc<Value>` is
/// freely borrowed as `&Value` by [`open_from_value`] /
/// [`build_settings_from_value`].
pub fn parse_pbxproj(xcodeproj_path: &Path) -> Result<Arc<Value>, Error> {
    let pbxproj_path = xcodeproj_path.join("project.pbxproj");
    pbxproj::parse_file_cached(&pbxproj_path).map_err(|e| match e {
        pbxproj::Error::Io(e) => Error::Io(e),
        pbxproj::Error::Parse(e) => Error::Parse(e),
    })
}

/// Pull the `objects` dict and the root project object out of a parsed
/// pbxproj. Both shapes are stable and required by every higher-level query.
fn project_root(value: &Value) -> Result<(&BTreeMap<String, Value>, &Value), Error> {
    let root = value
        .as_dict()
        .ok_or_else(|| Error::BadProject("pbxproj root is not a dict".into()))?;
    let objects = root
        .get("objects")
        .and_then(Value::as_dict)
        .ok_or_else(|| Error::BadProject("no `objects` dict".into()))?;
    let root_id = root
        .get("rootObject")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::BadProject("no `rootObject` reference".into()))?;
    let project_obj = objects
        .get(root_id)
        .ok_or_else(|| Error::BadProject(format!("rootObject {root_id} not found in objects")))?;
    Ok((objects, project_obj))
}

fn extract_project_configurations(
    objects: &BTreeMap<String, Value>,
    project_obj: &Value,
) -> Result<Vec<String>, Error> {
    let list_id = project_obj
        .get("buildConfigurationList")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::BadProject("project has no buildConfigurationList".into()))?;
    let config_list = objects.get(list_id).ok_or_else(|| {
        Error::BadProject(format!("buildConfigurationList {list_id} not in objects"))
    })?;
    config_names(objects, config_list)
}

fn config_names(
    objects: &BTreeMap<String, Value>,
    config_list: &Value,
) -> Result<Vec<String>, Error> {
    let ids = config_list
        .get("buildConfigurations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            Error::BadProject("XCConfigurationList has no buildConfigurations".into())
        })?;
    let mut names = Vec::with_capacity(ids.len());
    for v in ids {
        let id = v
            .as_str()
            .ok_or_else(|| Error::BadProject("buildConfigurations entry is not a string".into()))?;
        // A dangling configuration id (hand-edited or merge-damaged pbxproj)
        // doesn't fail xcodebuild — the entry just doesn't exist. Skip it.
        let Some(config) = objects.get(id) else {
            continue;
        };
        let name = config
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::BadProject(format!("XCBuildConfiguration {id} has no name")))?;
        names.push(name.to_string());
    }
    Ok(names)
}

fn extract_targets(
    objects: &BTreeMap<String, Value>,
    project_obj: &Value,
) -> Result<Vec<Target>, Error> {
    let Some(target_ids) = project_obj.get("targets").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut targets = Vec::with_capacity(target_ids.len());
    for v in target_ids {
        let id = v
            .as_str()
            .ok_or_else(|| Error::BadProject("target reference is not a string".into()))?;
        let Some(target) = objects.get(id) else {
            // Some projects keep stale target IDs; skip silently rather than fail.
            continue;
        };
        let name = target
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let isa = target
            .get("isa")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let product_type = target
            .get("productType")
            .and_then(Value::as_str)
            .map(String::from);
        // A dangling target buildConfigurationList reads as an empty list
        // (the target then resolves with project-level settings only, like
        // xcodebuild); only the project-level list is load-bearing.
        let configurations = match target
            .get("buildConfigurationList")
            .and_then(Value::as_str)
            .and_then(|list_id| objects.get(list_id))
        {
            Some(config_list) => config_names(objects, config_list)?,
            None => Vec::new(),
        };
        targets.push(Target {
            name,
            isa,
            product_type,
            configurations,
        });
    }
    Ok(targets)
}

/// Layers and target metadata returned by [`build_settings`].
#[derive(Debug, Clone)]
pub struct BuildSettingsContext {
    /// The four user-authored layers in xcodebuild precedence order:
    /// project xcconfig, project `buildSettings`, target xcconfig, target
    /// `buildSettings`. Pass them in order to [`crate::resolver::resolve`].
    pub layers: Vec<Vec<Assignment>>,
    /// The matched target's `productType` (e.g.
    /// `com.apple.product-type.application`) if it declared one. Useful when
    /// layering xcspec ProductType defaults underneath.
    pub product_type: Option<String>,
    /// The matched target's `isa`.
    pub target_isa: String,
    /// Whether the target has a non-empty `packageProductDependencies` list,
    /// i.e. it links one or more Swift Package products. Needed to decide
    /// `ALLOW_TARGET_PLATFORM_SPECIALIZATION` (see [`built_in_overrides`]).
    pub has_package_product_dependencies: bool,
    /// For a test bundle, the `TARGET_NAME` of its host application — the
    /// first target-dependency whose `productType` is an application. `None`
    /// for non-test targets and for library test bundles that depend only on a
    /// framework/library (those have no host app). xcodebuild derives
    /// `TEST_HOST` / `TARGET_BUILD_SUBPATH` from this edge; we use it to
    /// synthesize the subpath when the bundle doesn't author `TEST_TARGET_NAME`
    /// (see [`crate::build_context`]'s `target_graph_layer`).
    pub test_host_target: Option<String>,
}

/// Extract the four user-authored build-settings layers for a target +
/// configuration, plus the target's metadata needed for xcspec lookups.
///
/// Apple's own defaults (the hundreds of settings from xcspec/SDKSettings)
/// are intentionally NOT included here — this returns only what the user
/// authored in the project + xcconfigs. Use [`crate::xcspec::load_catalog`]
/// and [`crate::xcspec::Catalog::layer_for`] to get the defaults layer, and
/// layer it underneath these user layers in the resolver.
pub fn build_settings(
    xcodeproj_path: &Path,
    target_name: &str,
    config_name: &str,
) -> Result<BuildSettingsContext, Error> {
    let value = parse_pbxproj(xcodeproj_path)?;
    build_settings_from_value(&value, xcodeproj_path, target_name, config_name)
}

/// Like [`build_settings`] but driven by an already-parsed pbxproj value.
/// Use this when the caller has cached the pbxproj parse.
pub fn build_settings_from_value(
    value: &Value,
    xcodeproj_path: &Path,
    target_name: &str,
    config_name: &str,
) -> Result<BuildSettingsContext, Error> {
    let (objects, project_obj) = project_root(value)?;

    // An unknown configuration name is not fatal: xcodebuild warns and falls
    // back to each XCConfigurationList's own `defaultConfigurationName` (see
    // [`find_config`]). Only a project with no configurations at all errors.
    let project_config = find_config(objects, project_obj, config_name)?.ok_or_else(|| {
        Error::BadProject(format!(
            "project has no configurations (requested '{config_name}')"
        ))
    })?;

    let target_obj = find_target(objects, project_obj, target_name)?.ok_or_else(|| {
        Error::BadProject(format!("no target named '{target_name}' in the project"))
    })?;

    // A target with no (or an empty/dangling) buildConfigurationList still
    // resolves — xcodebuild uses the project-level settings only — so its two
    // layers are simply empty.
    let target_config = find_config(objects, target_obj, config_name)?;

    let project_xcconfig = load_xcconfig_layer(objects, project_config, xcodeproj_path)?;
    let project_inline = extract_inline_settings(project_config);
    let (target_xcconfig, target_inline) = match target_config {
        Some(config) => (
            load_xcconfig_layer(objects, config, xcodeproj_path)?,
            extract_inline_settings(config),
        ),
        None => (Vec::new(), Vec::new()),
    };
    let product_type = target_obj
        .get("productType")
        .and_then(Value::as_str)
        .map(String::from);
    let target_isa = target_obj
        .get("isa")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let has_package_product_dependencies = target_obj
        .get("packageProductDependencies")
        .and_then(Value::as_array)
        .is_some_and(|deps| !deps.is_empty());
    let test_host_target = if is_test_bundle_product_type(product_type.as_deref()) {
        // Xcode records the authoritative host in the root PBXProject's
        // `attributes.TargetAttributes.<test-target-uuid>.TestTargetID`;
        // the dependency scan is the fallback for projects without it.
        test_target_id_host(objects, project_obj, target_name)
            .or_else(|| find_app_host_target(objects, target_obj))
    } else {
        None
    };
    Ok(BuildSettingsContext {
        layers: vec![
            project_xcconfig,
            project_inline,
            target_xcconfig,
            target_inline,
        ],
        product_type,
        target_isa,
        has_package_product_dependencies,
        test_host_target,
    })
}

/// Convenience wrapper that returns just the layers, dropping the target
/// metadata. Kept for callers that don't need xcspec lookup info.
pub fn build_settings_layers(
    xcodeproj_path: &Path,
    target_name: &str,
    config_name: &str,
) -> Result<Vec<Vec<Assignment>>, Error> {
    build_settings(xcodeproj_path, target_name, config_name).map(|ctx| ctx.layers)
}

/// The ordered, absolute source paths a native target compiles: the file
/// references of its `PBXSourcesBuildPhase`, each resolved through the
/// `PBXGroup` tree to a concrete path. The order matches the pbxproj (which is
/// the order xcodebuild feeds them to the compiler's filelist).
///
/// Every extension is returned — `.swift`, `.m`, `.c`, `.cpp`, … — because the
/// caller decides which tool consumes which (`swiftc` takes the `.swift`s,
/// `clang` the C-family). Auto-generated sources xcodebuild synthesizes at build
/// time (`<Target>_vers.c`, the `*-Swift.h` bridge) are NOT here — they live in
/// DerivedData, not the project graph.
pub fn target_source_files(
    xcodeproj_path: &Path,
    target_name: &str,
) -> Result<Vec<PathBuf>, Error> {
    let value = parse_pbxproj(xcodeproj_path)?;
    target_source_files_from_value(&value, xcodeproj_path, target_name)
}

/// Like [`target_source_files`] but driven by an already-parsed pbxproj value.
pub fn target_source_files_from_value(
    value: &Value,
    xcodeproj_path: &Path,
    target_name: &str,
) -> Result<Vec<PathBuf>, Error> {
    let (objects, project_obj) = project_root(value)?;
    let project_dir = abs_project_dir(xcodeproj_path);

    // Resolve every file reference to an absolute path with one DFS from the
    // project's mainGroup, accumulating each `<group>`'s `path` as we descend.
    let mut file_paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut sync_dirs: BTreeMap<String, PathBuf> = BTreeMap::new();
    if let Some(main_group_id) = project_obj.get("mainGroup").and_then(Value::as_str) {
        resolve_group_paths(
            objects,
            main_group_id,
            &project_dir,
            &project_dir,
            &mut file_paths,
            &mut sync_dirs,
        );
    }

    let target = find_target(objects, project_obj, target_name)?.ok_or_else(|| {
        Error::BadProject(format!("no target named '{target_name}' in the project"))
    })?;

    let mut out = Vec::new();
    let Some(phase_ids) = target.get("buildPhases").and_then(Value::as_array) else {
        return Ok(out);
    };
    for phase_ref in phase_ids {
        let Some(phase) = phase_ref.as_str().and_then(|id| objects.get(id)) else {
            continue;
        };
        if phase.get("isa").and_then(Value::as_str) != Some("PBXSourcesBuildPhase") {
            continue;
        }
        let Some(file_ids) = phase.get("files").and_then(Value::as_array) else {
            continue;
        };
        for build_file_ref in file_ids {
            let Some(file_ref_id) = build_file_ref
                .as_str()
                .and_then(|id| objects.get(id))
                .and_then(|bf| bf.get("fileRef").and_then(Value::as_str))
            else {
                continue;
            };
            if let Some(p) = file_paths.get(file_ref_id) {
                out.push(p.clone());
            }
        }
    }

    // Synchronized folders (Xcode 16+): each id in `fileSystemSynchronizedGroups`
    // names a root folder whose compilable files are implicit target members —
    // they never appear in a `PBXSourcesBuildPhase`. Walk each for sources, minus
    // any file the group's exception sets exclude from this target.
    if let Some(group_ids) = target
        .get("fileSystemSynchronizedGroups")
        .and_then(Value::as_array)
    {
        for group_ref in group_ids {
            let Some(group_id) = group_ref.as_str() else {
                continue;
            };
            let Some(dir) = sync_dirs.get(group_id) else {
                continue;
            };
            let excluded = objects.get(group_id).map_or_else(Vec::new, |group| {
                synchronized_membership_exclusions(objects, group, target_name, dir, &project_dir)
            });
            collect_synchronized_sources(dir, &excluded, &mut out);
        }
    }
    Ok(out)
}

/// The absolute paths a synchronized root group's exception sets exclude from
/// `target_name` — the `membershipExceptions` of each
/// `PBXFileSystemSynchronizedBuildFileExceptionSet` that targets it (a file
/// unchecked from the target's membership). Xcode is inconsistent about whether
/// these relative paths are anchored at the group folder or the project root, so
/// both anchorings are returned and either match excludes the file.
fn synchronized_membership_exclusions(
    objects: &BTreeMap<String, Value>,
    group: &Value,
    target_name: &str,
    group_dir: &Path,
    project_dir: &Path,
) -> Vec<PathBuf> {
    let mut excluded = Vec::new();
    let Some(set_ids) = group.get("exceptions").and_then(Value::as_array) else {
        return excluded;
    };
    for set_ref in set_ids {
        let Some(set) = set_ref.as_str().and_then(|id| objects.get(id)) else {
            continue;
        };
        let applies = set
            .get("target")
            .and_then(Value::as_str)
            .and_then(|id| objects.get(id))
            .and_then(|t| t.get("name").and_then(Value::as_str))
            == Some(target_name);
        if !applies {
            continue;
        }
        if let Some(members) = set.get("membershipExceptions").and_then(Value::as_array) {
            for rel in members.iter().filter_map(Value::as_str) {
                excluded.push(join_normalized(group_dir, rel));
                excluded.push(join_normalized(project_dir, rel));
            }
        }
    }
    excluded
}

/// Compilable source extensions a synchronized folder contributes to a target —
/// the `.swift` and C-family files that would otherwise be listed in a
/// `PBXSourcesBuildPhase`. Headers, resources, and asset catalogs are excluded
/// (they are not compiler inputs). `.C` is the C++ convention, distinct from `.c`.
const SYNCHRONIZED_SOURCE_EXTS: &[&str] = &["swift", "c", "m", "mm", "cc", "cpp", "cxx", "C"];

/// Append every compilable source under `dir` (recursively) to `out`, sorted for
/// determinism, skipping files already present or excluded from the target. A
/// missing directory yields nothing.
fn collect_synchronized_sources(dir: &Path, excluded: &[PathBuf], out: &mut Vec<PathBuf>) {
    let mut found = Vec::new();
    walk_source_tree(dir, &mut found);
    found.sort();
    for p in found {
        if !excluded.contains(&p) && !out.contains(&p) {
            out.push(p);
        }
    }
}

/// Recursively collect compilable sources under `dir`. Symlinks are not followed
/// (`file_type` does not traverse them), so a self-referential link can't loop.
fn walk_source_tree(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            walk_source_tree(&path, out);
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| SYNCHRONIZED_SOURCE_EXTS.contains(&ext))
        {
            out.push(path);
        }
    }
}

/// The frameworks a native target links **explicitly** — the `.framework` file
/// references of its `PBXFrameworksBuildPhase`, by base name (`Kingfisher.framework`
/// → `Kingfisher`), in pbxproj order. These become `-framework <name>` on the
/// link line. Frameworks the sources autolink via `import` are NOT here — they're
/// encoded in the object files, not the project graph.
pub fn target_linked_frameworks(
    xcodeproj_path: &Path,
    target_name: &str,
) -> Result<Vec<String>, Error> {
    let value = parse_pbxproj(xcodeproj_path)?;
    let (objects, project_obj) = project_root(&value)?;
    let target = find_target(objects, project_obj, target_name)?.ok_or_else(|| {
        Error::BadProject(format!("no target named '{target_name}' in the project"))
    })?;

    let mut out = Vec::new();
    let Some(phase_ids) = target.get("buildPhases").and_then(Value::as_array) else {
        return Ok(out);
    };
    for phase_ref in phase_ids {
        let Some(phase) = phase_ref.as_str().and_then(|id| objects.get(id)) else {
            continue;
        };
        if phase.get("isa").and_then(Value::as_str) != Some("PBXFrameworksBuildPhase") {
            continue;
        }
        let Some(file_ids) = phase.get("files").and_then(Value::as_array) else {
            continue;
        };
        for build_file_ref in file_ids {
            let Some(file_ref) = build_file_ref
                .as_str()
                .and_then(|id| objects.get(id))
                .and_then(|bf| bf.get("fileRef").and_then(Value::as_str))
                .and_then(|id| objects.get(id))
            else {
                continue;
            };
            let label = file_ref
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| file_ref.get("path").and_then(Value::as_str));
            if let Some(name) = label
                .and_then(|l| l.rsplit('/').next())
                .and_then(|n| n.strip_suffix(".framework"))
            {
                out.push(name.to_string());
            }
        }
    }
    Ok(out)
}

/// The names of the targets a native target directly depends on — the `target`
/// of each `PBXTargetDependency` in its `dependencies`, in pbxproj order. These
/// are the build graph's edges: sourcekit-lsp uses them to know which dependency
/// modules to prepare. Same-project dependencies only; a cross-project
/// `targetProxy` whose `target` doesn't resolve in this project is skipped.
pub fn target_dependencies(xcodeproj_path: &Path, target_name: &str) -> Result<Vec<String>, Error> {
    let value = parse_pbxproj(xcodeproj_path)?;
    let (objects, project_obj) = project_root(&value)?;
    let target = find_target(objects, project_obj, target_name)?.ok_or_else(|| {
        Error::BadProject(format!("no target named '{target_name}' in the project"))
    })?;
    Ok(target_dependency_names(objects, target))
}

fn target_dependency_names(objects: &BTreeMap<String, Value>, target_obj: &Value) -> Vec<String> {
    let Some(deps) = target_obj.get("dependencies").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dep_ref in deps {
        let name = dep_ref
            .as_str()
            .and_then(|id| objects.get(id))
            .and_then(|dep| dep.get("target").and_then(Value::as_str))
            .and_then(|id| objects.get(id))
            .and_then(|t| t.get("name").and_then(Value::as_str));
        if let Some(name) = name {
            out.push(name.to_string());
        }
    }
    out
}

/// Whether a target links one or more Swift Package products (a non-empty
/// `packageProductDependencies`). Such a target needs the package-products
/// framework search path (`-F …/PackageFrameworks`) for its `import`s to
/// resolve; targets without packages must not emit it.
pub fn target_has_package_products(
    xcodeproj_path: &Path,
    target_name: &str,
) -> Result<bool, Error> {
    let value = parse_pbxproj(xcodeproj_path)?;
    let (objects, project_obj) = project_root(&value)?;
    let target = find_target(objects, project_obj, target_name)?.ok_or_else(|| {
        Error::BadProject(format!("no target named '{target_name}' in the project"))
    })?;
    Ok(target
        .get("packageProductDependencies")
        .and_then(Value::as_array)
        .is_some_and(|deps| !deps.is_empty()))
}

/// `target`'s transitive dependencies in build order — a dependency precedes
/// every target that depends on it (post-order DFS), excluding `target` itself.
/// Each target is visited once, so cycles can't loop. This is the order a custom
/// prepare executor must emit modules in.
pub fn transitive_dependencies(
    xcodeproj_path: &Path,
    target_name: &str,
) -> Result<Vec<String>, Error> {
    let value = parse_pbxproj(xcodeproj_path)?;
    let (objects, project_obj) = project_root(&value)?;
    let mut order = Vec::new();
    let mut visited = BTreeSet::new();
    visit_dependencies(objects, project_obj, target_name, &mut visited, &mut order);
    order.retain(|t| t != target_name);
    Ok(order)
}

fn visit_dependencies(
    objects: &BTreeMap<String, Value>,
    project_obj: &Value,
    target_name: &str,
    visited: &mut BTreeSet<String>,
    order: &mut Vec<String>,
) {
    if !visited.insert(target_name.to_string()) {
        return;
    }
    if let Ok(Some(target)) = find_target(objects, project_obj, target_name) {
        for dep in target_dependency_names(objects, target) {
            visit_dependencies(objects, project_obj, &dep, visited, order);
        }
    }
    order.push(target_name.to_string());
}

/// Whether `target`'s module can be emitted directly with `swiftc` (the v3 fast
/// path), versus needing a full `xcodebuild`. Conservative — true only for a
/// pure-Swift target with no code-generation machinery: no Swift-package
/// products, no C-family sources (which form a clang module with header maps),
/// and no shell-script phases or build rules (which can synthesize sources).
/// A target that slips through but still can't be emitted is caught at build
/// time (the executor falls back to `xcodebuild`).
pub fn is_self_buildable(xcodeproj_path: &Path, target_name: &str) -> Result<bool, Error> {
    if target_has_package_products(xcodeproj_path, target_name)? {
        return Ok(false);
    }
    let value = parse_pbxproj(xcodeproj_path)?;
    let (objects, project_obj) = project_root(&value)?;
    let target = find_target(objects, project_obj, target_name)?.ok_or_else(|| {
        Error::BadProject(format!("no target named '{target_name}' in the project"))
    })?;
    if target_has_script_or_rule_phase(objects, target) {
        return Ok(false);
    }
    let sources = target_source_files_from_value(&value, xcodeproj_path, target_name)?;
    let is_swift = |p: &Path| p.extension().and_then(OsStr::to_str) == Some("swift");
    Ok(sources.iter().any(|p| is_swift(p)) && sources.iter().all(|p| is_swift(p)))
}

/// Whether a target has a `PBXShellScriptBuildPhase` or any build rule — either
/// can generate sources, so the module isn't a pure `swiftc` emit.
fn target_has_script_or_rule_phase(objects: &BTreeMap<String, Value>, target_obj: &Value) -> bool {
    if target_obj
        .get("buildRules")
        .and_then(Value::as_array)
        .is_some_and(|r| !r.is_empty())
    {
        return true;
    }
    let Some(phases) = target_obj.get("buildPhases").and_then(Value::as_array) else {
        return false;
    };
    phases.iter().any(|pid| {
        pid.as_str()
            .and_then(|id| objects.get(id))
            .and_then(|p| p.get("isa").and_then(Value::as_str))
            == Some("PBXShellScriptBuildPhase")
    })
}

/// The absolute directory containing the `.xcodeproj` — the anchor for
/// `<group>` / `SOURCE_ROOT` source trees. Canonicalized when it exists (so the
/// paths match xcodebuild's absolute output), falling back to the input.
fn abs_project_dir(xcodeproj_path: &Path) -> PathBuf {
    let abs = fs::canonicalize(xcodeproj_path).unwrap_or_else(|_| xcodeproj_path.to_path_buf());
    abs.parent().map_or_else(PathBuf::new, Path::to_path_buf)
}

/// DFS the group tree, recording `file_id → absolute path` for every leaf. A
/// group node contributes its own directory to its children; a leaf records its
/// full path. `PBXVariantGroup` / `XCVersionGroup` (localized resources, Core
/// Data model versions) are walked like groups so their members resolve.
fn resolve_group_paths(
    objects: &BTreeMap<String, Value>,
    node_id: &str,
    parent_base: &Path,
    project_dir: &Path,
    out: &mut BTreeMap<String, PathBuf>,
    sync_out: &mut BTreeMap<String, PathBuf>,
) {
    let Some(node) = objects.get(node_id) else {
        return;
    };
    let base = node_base(node, parent_base, project_dir);
    let isa = node.get("isa").and_then(Value::as_str).unwrap_or("");
    match isa {
        "PBXGroup" | "PBXVariantGroup" | "XCVersionGroup" => {
            if let Some(children) = node.get("children").and_then(Value::as_array) {
                for child in children {
                    if let Some(cid) = child.as_str() {
                        resolve_group_paths(objects, cid, &base, project_dir, out, sync_out);
                    }
                }
            }
        }
        // A folder reference (Xcode 16+): its members aren't listed in the
        // pbxproj — they are every file physically under `base`. Record the
        // directory keyed by id; a target that lists it in
        // `fileSystemSynchronizedGroups` resolves its sources by walking it.
        "PBXFileSystemSynchronizedRootGroup" => {
            sync_out.insert(node_id.to_string(), base);
        }
        _ => {
            out.insert(node_id.to_string(), base);
        }
    }
}

/// The absolute path of one group/file node, from its `sourceTree` + `path` and
/// the accumulated parent-group directory. `<group>` is parent-relative,
/// `SOURCE_ROOT` is project-relative, `<absolute>` is literal; build-variable
/// source trees (`BUILT_PRODUCTS_DIR`, …) anchor at the parent as a best effort
/// (they rarely hold compiled sources).
fn node_base(node: &Value, parent_base: &Path, project_dir: &Path) -> PathBuf {
    let path = node.get("path").and_then(Value::as_str).unwrap_or("");
    let source_tree = node
        .get("sourceTree")
        .and_then(Value::as_str)
        .unwrap_or("<group>");
    match source_tree {
        "<absolute>" => PathBuf::from(path),
        "SOURCE_ROOT" => join_normalized(project_dir, path),
        _ if path.is_empty() => parent_base.to_path_buf(),
        _ => join_normalized(parent_base, path),
    }
}

/// Join `rel` onto `base`, collapsing `.` / `..` lexically (without touching the
/// filesystem) so a group path like `../Shared` resolves cleanly.
fn join_normalized(base: &Path, rel: &str) -> PathBuf {
    let mut p = base.to_path_buf();
    for comp in Path::new(rel).components() {
        match comp {
            Component::ParentDir => {
                p.pop();
            }
            Component::CurDir => {}
            Component::Normal(s) => p.push(s),
            Component::RootDir | Component::Prefix(_) => p = PathBuf::from(comp.as_os_str()),
        }
    }
    p
}

/// Produce the "built-in" settings that `xcodebuild` injects from the
/// invocation context (project file location, target name, configuration,
/// host info, environment, SDK platform info). These mirror the values that
/// would otherwise appear empty in our resolver because their definitions
/// live entirely in xcodebuild's internal evaluator rather than any xcspec.
///
/// Layer underneath the user-authored settings (so a user-provided
/// `BUILD_DIR` still wins) and on top of the xcspec catalog defaults (so
/// xcspec defaults that reference `$(PROJECT_NAME)` can resolve).
/// Extract the "natural" SDK of a target from its user-authored layers
/// (the value the user wrote in pbxproj / xcconfig before destination
/// override). Returns the last unconditional `SDKROOT` assignment found
/// across all layers — later layers (target inline > target xcconfig >
/// project inline > project xcconfig) win.
#[must_use]
pub fn natural_sdkroot(layers: &[Vec<Assignment>]) -> Option<String> {
    last_unconditional_setting(layers, "SDKROOT")
}

/// Return the last unconditional value the user-authored layers assign
/// to `key`. Later layers win, matching xcconfig precedence. Conditional
/// (bracketed) assignments are skipped — we only care about the plain
/// default the user wrote.
#[must_use]
pub fn last_unconditional_setting(layers: &[Vec<Assignment>], key: &str) -> Option<String> {
    for layer in layers.iter().rev() {
        for a in layer.iter().rev() {
            if a.key == key && a.conditions.is_empty() {
                return Some(a.value.clone());
            }
        }
    }
    None
}

/// True if the target is being built as Mac Catalyst. xcodebuild flags
/// this with `IS_MACCATALYST=YES` and rewrites a chunk of
/// platform/triple/search-path settings. We detect it when the build
/// destination is macOS AND either:
///
/// - The target's user-authored `SDKROOT` is an iOS-family device SDK
///   (`iphoneos`/`appletvos`/`watchos`/`xros`) — the classic iOS target
///   that opts into Mac Catalyst at build time.
/// - The target's user-authored `SUPPORTS_MACCATALYST = YES` even
///   without an explicit `SDKROOT` — Tuist-generated projects often
///   rely on Xcode inferring SDKROOT and only declare the support flag.
#[must_use]
pub fn detect_catalyst(
    sdk_canonical: &str,
    natural_sdk: Option<&str>,
    supports_maccatalyst: Option<&str>,
) -> bool {
    if canonicalize_sdk_base(sdk_canonical) != "macosx" {
        return false;
    }
    if matches!(
        natural_sdk,
        Some("iphoneos" | "appletvos" | "watchos" | "xros")
    ) {
        return true;
    }
    matches!(supports_maccatalyst, Some(v) if v.eq_ignore_ascii_case("YES"))
}

/// Apple's Mac Catalyst minimum iOS deployment is 13.1. Any user-set
/// value below that floor gets bumped to 13.1 in xcodebuild's
/// `-showBuildSettings` output.
fn apply_catalyst_ios_floor(user_target: &str) -> String {
    if compare_versions(user_target, "13.1") == std::cmp::Ordering::Less {
        "13.1".to_string()
    } else {
        user_target.to_string()
    }
}

/// Map a Catalyst-effective iOS deployment target to the equivalent
/// `MACOSX_DEPLOYMENT_TARGET`. Apple's rule:
///
/// - iOS 13.X (any minor) → macOS 10.15 (Catalina, the Catalyst floor)
/// - iOS X.Y where 14 ≤ X ≤ 18 → macOS (X − 3).Y (the last offset pair is
///   iOS 18 → macOS 15)
/// - iOS X.Y where X ≥ 26 → macOS X.Y — Apple aligned every OS on the same
///   version number starting at 26 (iOS 26 ↔ macOS 26), ending the −3 offset
fn catalyst_macos_target(ios_target: &str) -> String {
    let (major, minor) = parse_version_pair(ios_target);
    if major == 13 {
        "10.15".to_string()
    } else if major >= 26 {
        format!("{major}.{minor}")
    } else {
        format!("{}.{}", major.saturating_sub(3), minor)
    }
}

fn parse_version_pair(v: &str) -> (u32, u32) {
    let mut iter = v.split('.');
    let major = iter.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = iter.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor)
}

fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let (a_major, a_minor) = parse_version_pair(a);
    let (b_major, b_minor) = parse_version_pair(b);
    (a_major, a_minor).cmp(&(b_major, b_minor))
}

#[must_use]
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn built_in_settings(
    xcodeproj_path: &Path,
    target_name: &str,
    config_name: &str,
    product_type: Option<&str>,
    sdk_canonical: &str,
    destination: Option<&RunDestination>,
    is_catalyst: bool,
    auto_no_destination: bool,
    user_iphoneos_deployment_target: Option<&str>,
    user_only_active_arch: Option<&str>,
    user_layers: &[Vec<Assignment>],
    derived_data_path: Option<&Path>,
    xcode_version: Option<&str>,
    xcode_developer_dir: Option<&str>,
) -> Vec<Assignment> {
    // Resolve to an absolute path so PROJECT_DIR / SRCROOT / BUILD_DIR match
    // xcodebuild's behaviour (it always emits absolute paths). Fall back to
    // the input if canonicalization fails — e.g. when the path doesn't exist.
    let abs_path =
        fs::canonicalize(xcodeproj_path).unwrap_or_else(|_| xcodeproj_path.to_path_buf());
    let project_dir = abs_path
        .parent()
        .map_or_else(PathBuf::new, Path::to_path_buf);
    let project_name = abs_path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_string();
    let project_dir_str = project_dir.display().to_string();
    let project_file_path = abs_path.display().to_string();
    let user = host_user();
    let home = host_home();
    // Xcode keys DerivedData by whichever container was opened (an
    // `.xcworkspace` if one sits next to or above the project, else the
    // `.xcodeproj` itself). The 28-char base-26 hash is MD5(container_path).
    // We mirror that here so `BUILD_DIR` and friends match the layout the
    // oracle captures use.
    //
    // `xcodebuild -derivedDataPath PATH` flattens this — it replaces the
    // whole `<home>/.../DerivedData/<container-hash>` segment with `PATH`,
    // so `BUILD_DIR = PATH/Build/Products` directly.
    let derived_container = find_derived_data_container(&abs_path);
    let derived_name = derived_container
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_string();
    let derived_hash = derived_data_hash(&derived_container.display().to_string());
    let derived_root = if let Some(override_path) = derived_data_path {
        override_path.display().to_string()
    } else if home.is_empty() {
        format!("/tmp/DerivedData/{derived_name}-{derived_hash}")
    } else {
        format!("{home}/Library/Developer/Xcode/DerivedData/{derived_name}-{derived_hash}")
    };
    let build_dir = format!("{derived_root}/Build/Products");
    let obj_root = format!("{derived_root}/Build/Intermediates.noindex");
    // Prefer the catalog's recorded DEVELOPER_DIR (the Xcode the capture was
    // taken with) over the host's `xcode-select`ed one, so a 26.0.1 capture
    // resolved on a machine where 16.4 is selected still emits 26.0.1's
    // toolchain paths. This byte-matches the whole DEVELOPER_*_DIR / TOOLCHAIN /
    // `-L$(DT_TOOLCHAIN_DIR)/usr/lib/swift/...` (OTHER_LDFLAGS) family per
    // version; the CLI (no catalog) falls back to the host install.
    let developer_dir = xcode_developer_dir.map_or_else(detect_developer_dir, str::to_owned);
    let host = host_arch();
    let sdk_base = canonicalize_sdk_base(sdk_canonical);
    let (archs, swift_prefix, deployment_target_name) = platform_metadata(&sdk_base);
    let platform_dir_name = platform_dir_name_for(&sdk_base);
    let platform_display = platform_display_name(&sdk_base);
    let effective_platform_name = effective_platform_name_for(&sdk_base);
    // `auto_no_destination` is the caller's verdict on the no-platform mode: a
    // multiplatform target with `SDKROOT = auto`, no `-destination`, and no
    // supported `-sdk` request never resolves a concrete platform —
    // `xcodebuild -showBuildSettings` leaves PLATFORM_NAME / the PackageType
    // (wrapper) chain unresolved and reports a band of sentinel / base-spec
    // defaults instead of the macosx-resolved values our fallback
    // (`sdk_canonical = "macosx"`) would pull in. The affected keys below pin
    // Apple's no-platform output in that mode. It is computed by the caller
    // (see [`crate::build_context::BuildContext`]) rather than re-derived here
    // so the same verdict gates this layer, Catalyst detection, and the
    // SDKROOT pin — a request for an SDK the target *supports* binds the
    // platform (matching `xcodebuild -sdk iphonesimulator`) and must not pin
    // the sentinels.
    // No-destination ARCHS_STANDARD for watchOS *device* can surface the full
    // watchOSDevice.xcspec RealArchitectures = ( arm64, armv7k, arm64_32 ).
    // Two filters apply, both observed only on the no-destination path:
    //   * When any destination is bound, xcodebuild drops the legacy armv7k
    //     (e.g. a macOS-aggregated embedded watch target reports "arm64
    //     arm64_32"), so the injection is gated to `destination.is_none()`.
    //   * armv7k is a 32-bit watch ABI Apple retired with watchOS 9; the
    //     captured oracles keep it only when WATCHOS_DEPLOYMENT_TARGET < 9
    //     (3.0/6.0 include it, 10.0/26.0 drop it). The fixtures bracket the
    //     cutoff at (6, 10]; we pin it to Apple's documented watchOS 9 drop.
    let archs: Vec<&'static str> = if destination.is_none()
        && sdk_base == "watchos"
        && watchos_keeps_armv7k(
            last_unconditional_setting(user_layers, "WATCHOS_DEPLOYMENT_TARGET").as_deref(),
        ) {
        vec!["arm64", "armv7k", "arm64_32"]
    } else {
        archs.to_vec()
    };
    let archs_64: Vec<&str> = archs.iter().copied().filter(|a| is_64bit(a)).collect();
    let arch_list = archs.join(" ");
    let native_32: String = match host.as_str() {
        "arm64" | "arm64e" => "arm".into(),
        "x86_64" => "i386".into(),
        _ => host.clone(),
    };
    let mut out: Vec<Assignment> = Vec::new();
    let mut push = |key: &str, value: String| {
        out.push(Assignment {
            key: key.to_string(),
            conditions: Vec::new(),
            value,
            condition: None,
        });
    };

    // --- Project + target + action -----------------------------------------
    push("ACTION", "build".into());
    push("CONFIGURATION", config_name.into());
    push("PROJECT", project_name.clone());
    push("PROJECT_NAME", project_name.clone());
    push("PROJECT_FILE_PATH", project_file_path);
    push("PROJECT_DIR", project_dir_str.clone());
    push("SRCROOT", project_dir_str.clone());
    push("LOCROOT", project_dir_str.clone());
    push("LOCSYMROOT", project_dir_str.clone());
    push("TARGET_NAME", target_name.into());
    push("TARGETNAME", target_name.into());
    push("PRODUCT_NAME", target_name.into());
    // PRODUCT_MODULE_NAME is intentionally NOT set here. The xcspec
    // default `$(PRODUCT_NAME:c99extidentifier)` correctly derives a
    // valid Swift/ObjC module name from the user-resolved PRODUCT_NAME
    // (which is often "Alamofire" even when the TARGET_NAME is
    // "Alamofire iOS"). Pushing target_name here would override the
    // xcspec default and emit the wrong module name.
    if let Some(pt) = product_type {
        push("PRODUCT_TYPE", pt.into());
    }

    // --- Build / output roots (overridable; we fill in xcodebuild's defaults) -
    // `BUILD_DIR`, `BUILD_ROOT`, and `SYMROOT` all point at the Products
    // directory of the DerivedData container; `OBJROOT` and `TEMP_ROOT`
    // point at the Intermediates directory.
    push("BUILD_DIR", build_dir.clone());
    push("BUILD_ROOT", build_dir.clone());
    push("SYMROOT", build_dir);
    push("OBJROOT", obj_root.clone());
    push("TEMP_ROOT", obj_root);
    // `DSTROOT` is keyed on the *project*, not the target —
    // CoreBuildSystem.xcspec defines it as `/tmp/$(PROJECT_NAME).dst` and the
    // captures confirm it (target `Alamofire iOS` reports `/tmp/Alamofire.dst`).
    // `INSTALL_ROOT` defaults to `$(DSTROOT)`.
    push("DSTROOT", format!("/tmp/{project_name}.dst"));
    push("INSTALL_ROOT", format!("/tmp/{project_name}.dst"));
    // DerivedData root (parent of every container/hash dir). Referenced by
    // xcspec defaults like `MODULE_CACHE_DIR = $(DERIVED_DATA_DIR)/ModuleCache.noindex`.
    // When `-derivedDataPath` is overridden, the override IS the
    // DerivedData root — there's no parent/container split.
    let derived_data_dir = if let Some(override_path) = derived_data_path {
        override_path.display().to_string()
    } else if home.is_empty() {
        "/tmp/DerivedData".to_string()
    } else {
        format!("{home}/Library/Developer/Xcode/DerivedData")
    };
    push("DERIVED_DATA_DIR", derived_data_dir);
    // Per-arch intermediates. xcspec defines these as nested indirect
    // lookups (`$(OBJECT_FILE_DIR_$(CURRENT_VARIANT))/$(CURRENT_ARCH)`),
    // and our resolver handles the nested expansion correctly — we just
    // need to seed the chain.
    push(
        "PER_ARCH_OBJECT_FILE_DIR",
        "$(OBJECT_FILE_DIR_$(CURRENT_VARIANT))/$(CURRENT_ARCH)".into(),
    );
    // xcspec's `PER_ARCH_MODULE_FILE_DIR = $(PER_ARCH_OBJECT_FILE_DIR)/Modules`
    // is stale: every captured oracle reports it as just
    // `$(PER_ARCH_OBJECT_FILE_DIR)` (no trailing /Modules).
    push(
        "PER_ARCH_MODULE_FILE_DIR",
        "$(PER_ARCH_OBJECT_FILE_DIR)".into(),
    );
    // xcspec defines `SHARED_PRECOMPS_DIR = $(OBJROOT)/SharedPrecompiledHeaders`
    // but every captured oracle reports `$(OBJROOT)/PrecompiledHeaders`
    // (no "Shared" prefix). Modern xcodebuild diverges from xcspec.
    push(
        "SHARED_PRECOMPS_DIR",
        "$(OBJROOT)/PrecompiledHeaders".into(),
    );

    // --- Derived paths: expressed as $(...) so the resolver expands them. ---
    push(
        "CONFIGURATION_BUILD_DIR",
        "$(BUILD_DIR)/$(CONFIGURATION)$(EFFECTIVE_PLATFORM_NAME)".into(),
    );
    push("BUILT_PRODUCTS_DIR", "$(CONFIGURATION_BUILD_DIR)".into());
    // Including `$(TARGET_BUILD_SUBPATH)` here is what wires up parent-app
    // embedding. `TARGET_BUILD_SUBPATH` defaults to empty (so this expands
    // to `$(BUILT_PRODUCTS_DIR)` for normal targets), but xcodebuild — and
    // [`crate::build_context::BuildContext`] — set it to e.g.
    // `/App.app/PlugIns` for a unit-test bundle whose host is `App`, which
    // nests the test bundle inside the host app's `PlugIns` directory.
    push(
        "TARGET_BUILD_DIR",
        "$(BUILT_PRODUCTS_DIR)$(TARGET_BUILD_SUBPATH)".into(),
    );
    push(
        "PROJECT_TEMP_DIR",
        "$(OBJROOT)/$(PROJECT_NAME).build".into(),
    );
    push(
        "CONFIGURATION_TEMP_DIR",
        "$(PROJECT_TEMP_DIR)/$(CONFIGURATION)$(EFFECTIVE_PLATFORM_NAME)".into(),
    );
    push(
        "TARGET_TEMP_DIR",
        "$(CONFIGURATION_TEMP_DIR)/$(TARGET_NAME).build".into(),
    );
    push("TEMP_DIR", "$(TARGET_TEMP_DIR)".into());
    push("TEMP_FILE_DIR", "$(TARGET_TEMP_DIR)".into());
    push("TEMP_FILES_DIR", "$(TARGET_TEMP_DIR)".into());
    push(
        "DERIVED_FILE_DIR",
        "$(TARGET_TEMP_DIR)/DerivedSources".into(),
    );
    push("DERIVED_FILES_DIR", "$(DERIVED_FILE_DIR)".into());
    push("DERIVED_SOURCES_DIR", "$(DERIVED_FILE_DIR)".into());
    // xcodebuild synthesizes `OBJECT_FILE_DIR_<variant>` by suffixing the
    // base `OBJECT_FILE_DIR`. The xcspecs reference these via
    // `$(OBJECT_FILE_DIR_$(CURRENT_VARIANT))` and they'd resolve empty
    // without these built-ins.
    push("OBJECT_FILE_DIR_normal", "$(OBJECT_FILE_DIR)-normal".into());
    push("OBJECT_FILE_DIR_debug", "$(OBJECT_FILE_DIR)-debug".into());
    push(
        "OBJECT_FILE_DIR_profile",
        "$(OBJECT_FILE_DIR)-profile".into(),
    );
    push(
        "STRINGSDATA_DIR",
        "$(TARGET_TEMP_DIR)/Objects-$(CURRENT_VARIANT)/$(arch)".into(),
    );
    push("STRINGSDATA_ROOT", "$(TARGET_TEMP_DIR)".into());

    // Product naming (EXECUTABLE_NAME, FULL_PRODUCT_NAME, EXECUTABLE_PATH,
    // CONTENTS_FOLDER_PATH, etc.) is intentionally NOT defined here — the
    // correct values are product-type and package-type-specific (a tool's
    // FULL_PRODUCT_NAME is `$(EXECUTABLE_NAME)`, an application's is
    // `$(WRAPPER_NAME)`). The xcspec catalog provides these via the
    // ProductType + PackageType DefaultBuildProperties chain.

    // --- Environment -------------------------------------------------------
    push("USER", user.clone());
    push("HOME", home.clone());
    push("USER_APPS_DIR", format!("{home}/Applications"));
    push("USER_LIBRARY_DIR", format!("{home}/Library"));
    push("ALTERNATE_OWNER", user.clone());
    push("INSTALL_OWNER", user.clone());
    push("VERSION_INFO_BUILDER", user);
    push("ALTERNATE_GROUP", "staff".into());
    push("INSTALL_GROUP", "staff".into());

    // --- Host + toolchain --------------------------------------------------
    push("HOST_ARCH", host.clone());
    push("NATIVE_ARCH", host.clone());
    push("NATIVE_ARCH_ACTUAL", host.clone());
    push("NATIVE_ARCH_64_BIT", host.clone());
    push("NATIVE_ARCH_32_BIT", native_32);
    push("arch", "undefined_arch".into());
    push("CURRENT_ARCH", "undefined_arch".into());
    push("CURRENT_VARIANT", "normal".into());
    push("ACTIVE_VARIANT", "normal".into());
    push("DEVELOPER_DIR", developer_dir.clone());
    push(
        "DEVELOPER_APPLICATIONS_DIR",
        format!("{developer_dir}/Applications"),
    );
    push("DEVELOPER_BIN_DIR", format!("{developer_dir}/usr/bin"));
    push("DEVELOPER_LIBRARY_DIR", format!("{developer_dir}/Library"));
    push(
        "DEVELOPER_FRAMEWORKS_DIR",
        format!("{developer_dir}/Library/Frameworks"),
    );
    push("DEVELOPER_TOOLS_DIR", format!("{developer_dir}/Tools"));
    push("DEVELOPER_USR_DIR", format!("{developer_dir}/usr"));
    push("DEVELOPMENT_LANGUAGE", "en".into());
    push("TOOLCHAINS", "com.apple.dt.toolchain.XcodeDefault".into());
    let toolchain_dir = format!("{developer_dir}/Toolchains/XcodeDefault.xctoolchain");
    push("TOOLCHAIN_DIR", toolchain_dir.clone());
    // `DT_TOOLCHAIN_DIR` is Apple's legacy alias for `TOOLCHAIN_DIR`. Many
    // Tuist-generated and hand-written xcconfigs reference it directly in
    // their `OTHER_LDFLAGS = $(inherited) -L$(DT_TOOLCHAIN_DIR)/usr/lib/swift/$(PLATFORM_NAME)`
    // recipe. If we don't define it, those references expand to empty
    // and emit the broken `-L/usr/lib/swift/<platform>` form.
    push("DT_TOOLCHAIN_DIR", toolchain_dir);
    // RESIDUAL (SWIFT_INCLUDE_PATHS, 8): tuist injects a path anchored at its own
    // DerivedData build dir, which isn't present in the project inputs we resolve
    // from — not synthesizable here, so we leave the user/SDK value untouched.
    push("GCC_VERSION", "com.apple.compilers.llvm.clang.1_0".into());
    push(
        "GCC_VERSION_IDENTIFIER",
        "com_apple_compilers_llvm_clang_1_0".into(),
    );
    // Xcode version numbers, derived from the catalog's recorded version (the
    // Xcode the corpus was captured from) rather than the host's active Xcode,
    // so version-conditional project settings such as
    // `$(SWIFT_STRICT_CONCURRENCY_XCODE_$(XCODE_VERSION_MAJOR))` resolve against
    // the right Xcode. Falls back to the host install when the catalog didn't
    // record a version. Encoding mirrors xcodebuild: for a version `A.B.C`,
    // MAJOR = A*100, MINOR = A*100+B*10, ACTUAL = A*100+B*10+C (16.4.0 ->
    // 1600/1640/1640; 26.0.1 -> 2600/2600/2601).
    let xcode_short = xcode_version.map_or_else(
        || crate::xcode::active_install().short_version,
        str::to_owned,
    );
    if let Some((major, minor, actual)) = xcode_version_numbers(&xcode_short) {
        push("XCODE_VERSION_ACTUAL", actual);
        push("XCODE_VERSION_MAJOR", major);
        push("XCODE_VERSION_MINOR", minor);
    }

    // --- Standard system paths (xcodebuild fills these from its evaluator) -
    push("SYSTEM_LIBRARY_DIR", "/System/Library".into());
    push("SYSTEM_APPS_DIR", "/Applications".into());
    push("SYSTEM_ADMIN_APPS_DIR", "/Applications/Utilities".into());
    push("SYSTEM_DEMOS_DIR", "/Applications/Extras".into());
    // On modern Xcode, SYSTEM_DEVELOPER_DIR is an alias for the active
    // DEVELOPER_DIR (rather than the legacy `/Developer` path).
    push("SYSTEM_DEVELOPER_DIR", developer_dir.clone());
    push("SYSTEM_DOCUMENTATION_DIR", "/Library/Documentation".into());
    push("LIBRARY_DIR", "/Library".into());
    push("LOCAL_LIBRARY_DIR", "/Library".into());
    push("LOCAL_APPS_DIR", "/Applications".into());
    push("LOCAL_ADMIN_APPS_DIR", "/Applications/Utilities".into());
    push("LOCAL_DEVELOPER_DIR", "/Library/Developer".into());

    // --- Platform / SDK ----------------------------------------------------
    push("SUPPORTED_PLATFORMS", supported_platforms_for(&sdk_base));
    push("PLATFORM_NAME", sdk_base.clone());
    let is_sim =
        destination.is_some_and(RunDestination::is_simulator) || sdk_base.ends_with("simulator");
    let display = if is_sim && sdk_base != "macosx" {
        format!("{platform_display} Simulator")
    } else {
        platform_display.to_string()
    };
    push("PLATFORM_DISPLAY_NAME", display);
    push(
        "PLATFORM_DIR",
        format!("{developer_dir}/Platforms/{platform_dir_name}.platform"),
    );
    // Mac Catalyst forces `-maccatalyst` as the build's
    // EFFECTIVE_PLATFORM_NAME (rather than the empty string macOS native
    // gets), and reports the Swift/triple platform prefix as iOS.
    if auto_no_destination {
        // With `SDKROOT = auto` and no destination there is no resolved
        // platform, so xcodebuild emits its `-unknown` sentinel for
        // EFFECTIVE_PLATFORM_NAME — even when our macosx fallback would
        // otherwise have flagged the target as Catalyst (SUPPORTS_MACCATALYST).
        push("EFFECTIVE_PLATFORM_NAME", "-unknown".into());
        push("SWIFT_PLATFORM_TARGET_PREFIX", swift_prefix.into());
    } else if is_catalyst {
        push("EFFECTIVE_PLATFORM_NAME", "-maccatalyst".into());
        push("SWIFT_PLATFORM_TARGET_PREFIX", "ios".into());
    } else {
        push("EFFECTIVE_PLATFORM_NAME", effective_platform_name);
        push("SWIFT_PLATFORM_TARGET_PREFIX", swift_prefix.into());
    }
    push(
        "DEPLOYMENT_TARGET_SETTING_NAME",
        deployment_target_name.into(),
    );

    // --- Bundle layout (BUNDLE_FORMAT) -------------------------------------
    // macOSCoreBuildSystem.xcspec overrides the generic CoreBuildSystem
    // default (`shallow`) to `deep` for the macosx domain, which expands the
    // `BUNDLE_*_FOLDER_PATH` cascade with a `Contents/` prefix (deep bundle).
    // But a multiplatform target with no resolved platform (`SDKROOT = auto`,
    // no -destination) never enters that domain: xcodebuild reports the bare
    // generic `shallow`/`Frameworks`/... defaults. Our resolution falls back
    // to `sdk_canonical = "macosx"` for these `auto` projects to keep going,
    // which would wrongly pull in the deep override — so when the natural
    // SDKROOT is `auto` and no destination is bound, pin BUNDLE_FORMAT back to
    // `shallow`. The catalog's `BUNDLE_CONTENTS_FOLDER_PATH = $(BUNDLE_CONTENTS_FOLDER_PATH_$(BUNDLE_FORMAT))`
    // recipe then re-expands against `_shallow` (undefined → empty), so the
    // dependent `BUNDLE_FRAMEWORKS/PLUGINS/...` keys drop the prefix on their
    // own. A real macosx SDKROOT keeps the catalog's `deep`.
    if auto_no_destination {
        push("BUNDLE_FORMAT", "shallow".into());
        // The PackageType (`wrapper.application`) chain is what supplies the
        // wrapper / FULL_PRODUCT_NAME recipes and flips
        // `GENERATE_PKGINFO_FILE = YES`; with no resolved platform xcodebuild
        // never applies it, so FULL_PRODUCT_NAME collapses to empty (making
        // `DWARF_DSYM_FILE_NAME = $(FULL_PRODUCT_NAME).dSYM` report a bare
        // `.dSYM`) and GENERATE_PKGINFO_FILE falls back to the base-spec `NO`.
        // Our macosx fallback pulls the PackageType in via the catalog layer,
        // so we pin both back here (DEFAULT layer, still user-overridable).
        push("DWARF_DSYM_FILE_NAME", ".dSYM".into());
        push("GENERATE_PKGINFO_FILE", "NO".into());
        // `TAPI_VERIFY_MODE = Pedantic` is a macOS `SDKSettings.plist`
        // `DefaultProperties` value; the base spec (CoreBuildSystem/TAPI.xcspec)
        // default is `ErrorsOnly`. With no resolved platform xcodebuild never
        // applies the macOS SDK defaults, so it reports the base `ErrorsOnly`.
        // Our macosx fallback pulls the SDK layer in via the catalog, so pin it
        // back to the base default here (DEFAULT layer, still user-overridable).
        push("TAPI_VERIFY_MODE", "ErrorsOnly".into());
        // The macOS SDK's `macabi` Variant in `SDKSettings.plist` ships
        // `ENABLE_HARDENED_RUNTIME = YES`, which xcodebuild applies for a
        // `SUPPORTS_MACCATALYST` target even in this no-platform mode (the only
        // Catalyst-derived value it keeps; the -macabi triple / iOSSupport
        // search paths / Catalyst rpath / deployment-target recompute all stay
        // unset). We treat this mode as non-Catalyst (see `build_layers`), so
        // re-emit the hardened-runtime default here. Evidence: IceCubesApp's
        // `-project` capture reports `ENABLE_HARDENED_RUNTIME = YES`.
        push("ENABLE_HARDENED_RUNTIME", "YES".into());
        // The `application` ProductType ships `CODE_SIGNING_ALLOWED = YES`
        // unconditionally, but with no resolved platform xcodebuild reports NO
        // (a concrete signing identity can't be selected for a target whose
        // platform isn't pinned). Other product types already default to NO via
        // their own specs, so only the application case needs correcting.
        if product_type == Some("com.apple.product-type.application") {
            push("CODE_SIGNING_ALLOWED", "NO".into());
        }
    }

    // RESIDUAL (IPHONEOS_DEPLOYMENT_TARGET, 4): the named deployment target is a
    // pass-through of the user/SDK value; four captures drift 13.0-vs-13.1 in the
    // minor version, a capture-time artifact rather than a resolver rule — left as-is.
    // `ARCHS` resolves to the *active* arch when `ONLY_ACTIVE_ARCH=YES`
    // and to the platform's standard arch list otherwise. The user's
    // unconditional `ONLY_ACTIVE_ARCH` setting wins over the Debug→YES /
    // Release→NO xcspec default. (Cross-platform destination overrides —
    // e.g. iPhone-Sim destination building an embedded watchsimulator
    // target — are applied later, in [`built_in_overrides`], so they sit
    // above user-authored values rather than below.)
    let only_active_arch_yes = user_only_active_arch.map_or_else(
        || config_name.eq_ignore_ascii_case("Debug"),
        |v| v.eq_ignore_ascii_case("YES"),
    );
    // The ONLY_ACTIVE_ARCH collapse to the host arch only happens when the
    // build is pinned to one concrete device: a bound destination, or a
    // simulator SDK (a simulator build always targets a concrete simulator,
    // so xcodebuild collapses it even when the harness can't carry the
    // `id=<uuid>` destination). A plain `xcodebuild -showBuildSettings` on a
    // *device*/macOS SDK with no -destination reports the SDK's full standard
    // arch list regardless of ONLY_ACTIVE_ARCH — it has no active device to
    // single out. The no-destination device/macOS oracles confirm ARCHS ==
    // ARCHS_STANDARD even on Debug.
    let pinned_to_device = destination.is_some() || sdk_base.ends_with("simulator");
    // The collapse target is the *destination's* running arch (a device
    // destination is arm64 even on an Intel host; an explicit
    // `-destination …,arch=x86_64` wins on any host). Only the bare
    // simulator-SDK case — no destination to read — falls back to the host
    // arch, since a simulator executes on the host.
    let active_arch = destination
        .map(|d| d.arch.as_str())
        .filter(|a| !a.is_empty())
        .unwrap_or(host.as_str());
    let archs_value = if pinned_to_device && only_active_arch_yes {
        active_arch.to_string()
    } else {
        arch_list.clone()
    };
    push("ARCHS", archs_value);
    push("ARCHS_STANDARD", arch_list.clone());
    // tvOSDevice.xcspec lists ARCHS_STANDARD_64_BIT = ( arm64, arm64e ) — the
    // device carries the secondary arm64e slice even though ARCHS_STANDARD is
    // just arm64. xrOSDevice.xcspec keeps only ( arm64 ), so visionOS stays on
    // the bare archs_64. Corpus only ever binds the *simulator* SDK for tvOS
    // (appletvsimulator), which keeps the unmodified `arm64 x86_64`.
    let archs_standard_64_bit = if sdk_base == "appletvos" {
        "arm64 arm64e".to_string()
    } else {
        archs_64.join(" ")
    };
    push("ARCHS_STANDARD_64_BIT", archs_standard_64_bit);
    push(
        "ARCHS_STANDARD_32_BIT",
        archs_standard_32_bit_for(&sdk_base).into(),
    );
    push("ARCHS_STANDARD_INCLUDING_64_BIT", arch_list);
    push(
        "ARCHS_STANDARD_32_64_BIT",
        archs_standard_32_64_bit_for(&sdk_base).into(),
    );
    push("VALID_ARCHS", valid_archs_for(&sdk_base).into());
    push(
        "IS_MACCATALYST",
        if is_catalyst { "YES" } else { "NO" }.into(),
    );
    push("INLINE_PRIVATE_FRAMEWORKS", "NO".into());
    // STRIP_INSTALLED_PRODUCT is config-conditional: Debug builds keep
    // symbols for debugging; Release strips them.
    let strip = if config_name.eq_ignore_ascii_case("Release") {
        "YES"
    } else {
        "NO"
    };
    push("STRIP_INSTALLED_PRODUCT", strip.into());
    push("STRIP_SWIFT_SYMBOLS", "YES".into());
    push("BUILD_COMPONENTS", "headers build".into());

    // --- Platform-conditional output format defaults -----------------------
    // macOS preserves the on-disk format (so devs can edit plain text); the
    // device platforms compact to binary plist + UTF-16 strings.
    let plist_format = plist_output_format_for(&sdk_base);
    push("INFOPLIST_OUTPUT_FORMAT", plist_format.into());
    push("PLIST_FILE_OUTPUT_FORMAT", plist_format.into());
    push(
        "STRINGS_FILE_OUTPUT_ENCODING",
        strings_output_encoding_for(&sdk_base).into(),
    );

    // --- Apple-internal platform-class flags -------------------------------
    push("__IS_NOT_MACOS", is_not_macos_for(&sdk_base).into());
    push(
        "__IS_NOT_SIMULATOR",
        is_not_simulator_for(&sdk_base, destination).into(),
    );

    // --- Destination-aware defaults ----------------------------------------
    // `BUILD_ACTIVE_RESOURCES_ONLY` is YES when targeting any non-macOS
    // platform (simulator OR iOS-target running on the macOS host as a
    // Catalyst-style build), and NO for native macOS or generic device
    // archives where no specific destination is bound.
    let active_resources = if sdk_base != "macosx" && destination.is_some() {
        "YES"
    } else {
        "NO"
    };
    push("BUILD_ACTIVE_RESOURCES_ONLY", active_resources.into());

    // PNG asset settings. The xcspec defaults are YES for both, but
    // xcodebuild's `-showBuildSettings` reports `STRIP_PNG_TEXT=NO`
    // universally and `COMPRESS_PNG_FILES=NO` on macOS targets. We
    // emit the corrected values here (above the xcspec defaults).
    push("STRIP_PNG_TEXT", "NO".into());
    if sdk_base == "macosx" {
        push("COMPRESS_PNG_FILES", "NO".into());
    }

    // `SKIP_INSTALL` is `NO` only for top-level installable products —
    // applications and command-line tools. Every other product type
    // (frameworks, app-extensions, bundles, libraries, the embedded
    // watchapp2 / watchkit extensions, …) defaults to `YES` because those
    // build outputs get *embedded* into a parent app rather than installed
    // on their own. The watchapp2-CONTAINER is the exception among the
    // watch product types: it's the top-level iPhone companion app that
    // ships the watch app, so xcodebuild reports `SKIP_INSTALL=NO` for it.
    let skip_install = match product_type {
        // A multiplatform `SDKROOT = auto` target with no resolved platform
        // (no -destination) reports `SKIP_INSTALL = YES` even for an
        // application: with no concrete platform xcodebuild can't treat it as
        // a top-level installable product, so it falls back to the base-spec
        // default. Evidence: IceCubesApp's `-project` capture (auto SDKROOT).
        _ if auto_no_destination => "YES",
        Some(
            "com.apple.product-type.application"
            | "com.apple.product-type.application.watchapp2-container"
            | "com.apple.product-type.tool",
        )
        | None => "NO",
        Some(_) => "YES",
    };
    push("SKIP_INSTALL", skip_install.into());

    // `TARGETED_DEVICE_FAMILY` has no static xcspec default (its value list
    // is "provided dynamically by the UIType implementation"). When a target
    // doesn't author it, xcodebuild fills in the platform's natural device
    // family: tvOS=3, watchOS=4, visionOS=7, iOS=1,2. macOS-native targets
    // get no value. We emit this at the DEFAULT layer so any user-authored
    // value (apps usually pin `1,2` etc.) still wins. The watchapp2-container
    // is the lone exception: it's the iPhone companion stub, so it targets
    // device family 1 (iPhone) even though it lives in the watch project.
    if let Some(device_family) = match product_type {
        Some("com.apple.product-type.application.watchapp2-container") => Some("1"),
        // An unhosted iOS unit/UI-test bundle takes device family `1` (iPhone),
        // not the app's `1,2`: the test product type has no static TDF default
        // in `ProductTypes.xcspec`, and xcodebuild fills just iPhone. Emitted at
        // the DEFAULT layer, so a test target that authors its own TDF wins.
        pt if is_test_bundle_product_type(pt)
            && matches!(sdk_base.as_str(), "iphoneos" | "iphonesimulator") =>
        {
            Some("1")
        }
        _ => match sdk_base.as_str() {
            "iphoneos" | "iphonesimulator" => Some("1,2"),
            "appletvos" | "appletvsimulator" => Some("3"),
            "watchos" | "watchsimulator" => Some("4"),
            "xros" | "xrsimulator" => Some("7"),
            _ => None,
        },
    } {
        push("TARGETED_DEVICE_FAMILY", device_family.into());
    }

    // `LD_RUNPATH_SEARCH_PATHS` default layer. Two product-type/platform
    // combinations have an xcspec-derived default that our indirect
    // expansion misses (the recipes use nested `$(VAR_$(OTHER))` lookups
    // we don't fully resolve), so we synthesize the same value here at the
    // DEFAULT layer — below the user layer, so a target's authored
    // `$(inherited) …` correctly prepends it.
    //
    //   * watchapp2 + watchapp2-container: both the watch app and its iPhone
    //     companion stub get the standard app rpath `@executable_path/Frameworks`
    //     even when they never author one. `DarwinProductTypes.xcspec`'s
    //     application type only defines a Catalyst rpath
    //     (`LD_RUNPATH_SEARCH_PATHS_YES = @loader_path/../Frameworks`); the
    //     non-Catalyst embedded-frameworks default xcodebuild actually emits
    //     isn't in the spec, so we synthesize it. The watch *app* only gets it
    //     when the user authored no unconditional value of its own — when they
    //     do (kingfisher/tuist write `$(inherited) @executable_path/Frameworks`),
    //     adding our default would double the rpath via their `$(inherited)`.
    //     Evidence: alamofire's `watchOS Example WatchKit App` (watchapp2)
    //     authors nothing yet xcodebuild reports ` @executable_path/Frameworks`.
    //   * Catalyst extensions (xpc-service / pluginkit / app-extension
    //     family): `DarwinProductTypes.xcspec`'s xpc-service base defines
    //     `LD_RUNPATH_SEARCH_PATHS_YES_YES = (@loader_path/../Frameworks,
    //     @loader_path/../../../../Frameworks)` selected via
    //     `$(LD_RUNPATH_SEARCH_PATHS_$(IS_MACCATALYST)_$(_BOOL_$(SKIP_INSTALL)))`.
    //     With `IS_MACCATALYST=YES` and an embedded (SKIP_INSTALL=YES)
    //     extension that resolves to the loader_path pair.
    let runpath_default = match product_type {
        // The single leading space mirrors xcodebuild's capture (its own
        // default is `$(inherited) @executable_path/Frameworks` with an
        // empty inherited; we encode the resulting leading space directly).
        Some("com.apple.product-type.application.watchapp2-container") => {
            Some(" @executable_path/Frameworks")
        }
        // The watch app only inherits the default when the user authored no
        // unconditional value; otherwise their `$(inherited)` would double it.
        Some("com.apple.product-type.application.watchapp2")
            if last_unconditional_setting(user_layers, "LD_RUNPATH_SEARCH_PATHS").is_none() =>
        {
            Some(" @executable_path/Frameworks")
        }
        Some(
            "com.apple.product-type.app-extension"
            | "com.apple.product-type.extensionkit-extension"
            | "com.apple.product-type.pluginkit-plugin"
            | "com.apple.product-type.xpc-service",
        ) if is_catalyst => Some("@loader_path/../Frameworks @loader_path/../../../../Frameworks"),
        _ => None,
    };
    if let Some(paths) = runpath_default {
        push("LD_RUNPATH_SEARCH_PATHS", paths.into());
    }

    // `GCC_OBJC_LEGACY_DISPATCH` flips to YES whenever the target is not a
    // native macOS build — Apple wants the legacy ObjC dispatch on every
    // device / Catalyst / simulator code path.
    if sdk_base != "macosx" {
        push("GCC_OBJC_LEGACY_DISPATCH", "YES".into());
    }

    // `ASSETCATALOG_FILTER_FOR_*` are emitted only when the target is
    // non-macOS; xcodebuild forwards the *destination's* hardware identity
    // (a `Family,Variant` model code plus its OS version) into the asset
    // catalog compiler so it can thin assets for that exact device. macOS
    // builds use the Mac as the implicit destination, so we synthesize a
    // `MacFamily20,1` (Apple Silicon Mac) when the destination is macOS but
    // the *target's* platform is iOS — that's the Catalyst-style fallback
    // Apple emits in its captures.
    if sdk_base != "macosx"
        && let Some(d) = destination
    {
        let (model, os_version) = if d.is_macos() {
            ("MacFamily20,1".to_string(), host_os_version())
        } else {
            (
                device_model_for(&d.device_name).to_string(),
                d.os_version.clone(),
            )
        };
        push("ASSETCATALOG_FILTER_FOR_DEVICE_MODEL", model.clone());
        push("ASSETCATALOG_FILTER_FOR_DEVICE_OS_VERSION", os_version);
        push(
            "ASSETCATALOG_FILTER_FOR_THINNING_DEVICE_CONFIGURATION",
            model,
        );
    }

    // --- Mac Catalyst extras -----------------------------------------------
    // When an iOS-natural target builds for the macOS host, xcodebuild
    // emits an extra band of triple/resources/iOSSupport settings on top
    // of the existing macOS defaults. The values are entirely fixed (the
    // ABI suffix is always `-macabi`, the search paths always point at
    // the SDK's `System/iOSSupport` subdirectory, etc.) except for the
    // deployment-target trio (`MACOSX_DEPLOYMENT_TARGET`,
    // `SWIFT_DEPLOYMENT_TARGET`, `LLVM_TARGET_TRIPLE_OS_VERSION`) which
    // depend on the target's iOS deployment target — see below for
    // [`catalyst_deployment_targets`]'s mapping.
    if is_catalyst {
        push("LLVM_TARGET_TRIPLE_SUFFIX", "-macabi".into());
        push("RESOURCES_PLATFORM_NAME", "macosx".into());
        push("RESOURCES_UI_FRAMEWORK_FAMILY", "uikit".into());
        push("SHALLOW_BUNDLE_TRIPLE", "ios-macabi".into());
        // The xcspec ships ENABLE_HARDENED_RUNTIME with DefaultValue=NO
        // (CoreBuildSystem.xcspec), but xcodebuild synthesizes YES for
        // Catalyst builds — the macOS notarization path requires the
        // hardened runtime, so Apple's `-showBuildSettings` emits YES
        // for every iOS-natural target built against the macOS host.
        // Placed in the DEFAULT layer (below user settings) so an
        // explicit user value still wins.
        push("ENABLE_HARDENED_RUNTIME", "YES".into());
        // The deployment-target trio under Catalyst
        // (`MACOSX_DEPLOYMENT_TARGET`, `SWIFT_DEPLOYMENT_TARGET`,
        // `LLVM_TARGET_TRIPLE_OS_VERSION`) is emitted at the OVERRIDE
        // layer (see [`built_in_overrides`]) because Apple's
        // `-showBuildSettings` ignores any user-authored values for
        // these and recomputes them from the iOS deployment target.
        let _ = user_iphoneos_deployment_target;
        // Apple's macOS `SDKSettings.plist` ships a `Variants` array
        // whose Catalyst variant defines these search-path recipes as
        // `$(inherited) $(SDKROOT)/.../iOSSupport/.../Frameworks …`. Our
        // resolver doesn't ingest Variants, so xcodebuild's
        // `-showBuildSettings` output ends up with the iOSSupport paths
        // emitted TWICE (once from the Variant default, once from the
        // user-resolved layer). We mirror that doubled pattern here —
        // single leading space, double space between the two copies —
        // so canonicalization lines our output up with oracle captures.
        // The SubFrameworks segment of the Catalyst recipe is Xcode-version
        // dependent: the macOS SDK's `SDKSettings.plist` Catalyst variant adds
        // `.../System/Library/SubFrameworks` only on Xcode 26+. Xcode 16.4's
        // plist recipe is Frameworks-only, so gate the SubFrameworks copy on the
        // catalog's Xcode major (>= 2600), which is already derived above.
        let xcode_major = xcode_version_numbers(&xcode_short)
            .and_then(|(major, _, _)| major.parse::<u32>().ok())
            .unwrap_or(0);
        let catalyst_framework_paths = if xcode_major >= 2600 {
            " $(SDKROOT)/System/iOSSupport/System/Library/Frameworks \
             $(SDKROOT)/System/iOSSupport/System/Library/SubFrameworks"
        } else {
            " $(SDKROOT)/System/iOSSupport/System/Library/Frameworks"
        };
        let normalized_framework =
            collapse_whitespace_preserving_leading_space(catalyst_framework_paths);
        push(
            "SYSTEM_FRAMEWORK_SEARCH_PATHS",
            format!("{normalized_framework} {normalized_framework}"),
        );
        let catalyst_header_path = " $(SDKROOT)/System/iOSSupport/usr/include";
        push(
            "SYSTEM_HEADER_SEARCH_PATHS",
            format!("{catalyst_header_path} {catalyst_header_path}"),
        );
    }

    // `SYSTEM_FRAMEWORK_SEARCH_PATHS` is also synthesized for XCTest-style
    // targets (`bundle.unit-test`, `bundle.ui-testing`) — xcodebuild adds
    // the platform-bundled Developer framework path so the XCTest module
    // can be found at link time. The leading `$(inherited)` mirrors
    // xcodebuild's recipe; combined with the leading space on
    // `TEST_FRAMEWORK_SEARCH_PATHS`, the resolver expansion produces the
    // double-leading-space pattern Apple's captures show.
    if !is_catalyst && is_test_bundle_product_type(product_type) {
        push(
            "SYSTEM_FRAMEWORK_SEARCH_PATHS",
            "$(inherited) $(TEST_FRAMEWORK_SEARCH_PATHS)".into(),
        );
    }

    // `KASAN_DEFAULT_CFLAGS` is defined per-SDK in `SDKSettings.plist` with
    // `[arch=arm64]` / `[arch=arm64e]` overrides that pick the HW-tagged
    // TBI variant. But xcodebuild's `-showBuildSettings` resolves settings
    // with `arch=undefined_arch`, so those conditionals don't fire — the
    // default (CLASSIC) wins. We mirror that by emitting CLASSIC
    // unconditionally and letting the resolver layer above the SDKSettings
    // defaults to override the per-arch conditionals.
    let kasan_classic = "-DKASAN=1 -DKASAN_CLASSIC=1 -fsanitize=address \
         -mllvm -asan-globals-live-support -mllvm -asan-force-dynamic-shadow";
    let kasan_tbi = "-DKASAN=1 -DKASAN_TBI=1 -fsanitize=kernel-hwaddress \
         -mllvm -hwasan-recover=0 -mllvm -hwasan-instrument-atomics=0 \
         -mllvm -hwasan-instrument-stack=1 -mllvm -hwasan-generate-tags-with-calls=1 \
         -mllvm -hwasan-instrument-with-calls=1 -mllvm -hwasan-use-short-granules=0 \
         -mllvm -hwasan-memory-access-callback-prefix=__asan_";
    push("KASAN_DEFAULT_CFLAGS", normalize_flag_string(kasan_classic));
    push("KASAN_CFLAGS_CLASSIC", normalize_flag_string(kasan_classic));
    push("KASAN_CFLAGS_TBI", normalize_flag_string(kasan_tbi));

    // --- Compiler/module cache paths ---------------------------------------
    // These two settings expose where the user's machine caches per-build
    // state. `CCHROOT` is rooted under `_CS_DARWIN_USER_CACHE_DIR` (the
    // `confstr(3)` value) plus the active Xcode build number; we read it
    // from `$DARWIN_USER_CACHE_DIR` or fall back to `$TMPDIR/../C/`.
    let darwin_cache = darwin_user_cache_dir();
    let xcode_build = xcode_product_build_version();
    push(
        "CCHROOT",
        format!("{darwin_cache}com.apple.DeveloperTools/{xcode_build}/Xcode"),
    );
    if !home.is_empty() {
        push(
            "CLANG_MODULES_BUILD_SESSION_FILE",
            format!(
                "{home}/Library/Developer/Xcode/DerivedData/ModuleCache.noindex/\
                 Session.modulevalidation"
            ),
        );
    }

    // --- Synthesized search paths ------------------------------------------
    // xcodebuild appends BUILT_PRODUCTS_DIR-relative entries with a trailing
    // space, which is what gets surfaced in `-showBuildSettings`.
    push(
        "HEADER_SEARCH_PATHS",
        "$(BUILT_PRODUCTS_DIR)/include ".into(),
    );
    push("LIBRARY_SEARCH_PATHS", "$(BUILT_PRODUCTS_DIR) ".into());
    push("REZ_SEARCH_PATHS", "$(BUILT_PRODUCTS_DIR) ".into());
    push("FRAMEWORK_SEARCH_PATHS", "$(BUILT_PRODUCTS_DIR) ".into());
    // `TEST_FRAMEWORK_SEARCH_PATHS` points at the platform-bundled XCTest
    // frameworks. macOS gets only the platform-level path; every other
    // platform (device OR simulator) also gets the SDK-internal
    // `Developer/Library/Frameworks` directory. The oracle emits a leading
    // space.
    push(
        "TEST_FRAMEWORK_SEARCH_PATHS",
        if sdk_base == "macosx" {
            " $(PLATFORM_DIR)/Developer/Library/Frameworks".into()
        } else {
            " $(PLATFORM_DIR)/Developer/Library/Frameworks \
             $(SDKROOT)/Developer/Library/Frameworks"
                .replace("             ", "")
        },
    );

    // --- Misc per-config / per-platform defaults ---------------------------
    // `STRIP_BITCODE_FROM_COPIED_FILES` is YES only when shipping to a real
    // device (iphoneos/appletvos/watchos/xros). Simulators and native
    // macOS builds preserve bitcode.
    let strip_bitcode = if is_device_platform(&sdk_base) {
        "YES"
    } else {
        "NO"
    };
    push("STRIP_BITCODE_FROM_COPIED_FILES", strip_bitcode.into());

    // `ENABLE_DEBUG_DYLIB` enables the split debug-dylib executable used by
    // previews + incremental relinking. See [`enable_debug_dylib_default`]
    // for the per-product-type rule, which follows Apple's
    // `DarwinProductTypes.xcspec`.
    let is_debug = !config_name.eq_ignore_ascii_case("Release");
    push(
        "ENABLE_DEBUG_DYLIB",
        enable_debug_dylib_default(product_type, is_debug).into(),
    );

    // `DEBUG_INFORMATION_FORMAT` for an installable iPhone application resolved
    // with NO run destination: xcodebuild's no-destination "default-target"
    // view reports `dwarf-with-dsym` (the archive-like default) instead of the
    // documented Debug `dwarf` — the same no-destination installable-product
    // behaviour that drives `ENABLE_DEBUG_DYLIB` above. Emitted at the DEFAULT
    // layer (below the user layer) and gated on `destination.is_none()`, so a
    // destination-bound build and any project that authors its own value are
    // left untouched. Scoped to iphoneos apps — the only product/platform the
    // corpus exhibits this for (iOS-Example, Kingfisher-Demo).
    if is_debug
        && destination.is_none()
        && product_type == Some("com.apple.product-type.application")
        && sdk_base == "iphoneos"
    {
        push("DEBUG_INFORMATION_FORMAT", "dwarf-with-dsym".into());
    }

    // --- Swift defaults that aren't in any xcspec --------------------------
    push("SWIFT_ENABLE_EXPLICIT_MODULES", "YES".into());
    // The AppIntents const-extractable protocol list isn't a static xcspec
    // default — xcodebuild injects it from the AppIntents framework metadata in
    // the active SDK, so it grows (and gets re-sorted) per SDK and we track the
    // captured version's snapshot. This is the Xcode 26.5 SDK's list (sorted
    // alphabetically; 26.5 added `AppUnionValue` + `AppUnionValueCasesProviding`
    // over 26.0.1 and switched from declaration order to sorted). Older majors
    // (16.x/15.x) don't emit this key, so it's only scored on 26.x.
    push(
        "SWIFT_EMIT_CONST_VALUE_PROTOCOLS",
        "AnyResolverProviding AppEntity AppEnum AppExtension AppIntent AppIntentsPackage \
         AppShortcutProviding AppShortcutsProvider AppUnionValue AppUnionValueCasesProviding \
         DynamicOptionsProvider EntityQuery ExtensionPointDefining IntentValueQuery Resolver \
         TransientEntity _AssistantIntentsProvider _GenerativeFunctionExtractable \
         _IntentValueRepresentable"
            .into(),
    );

    out
}

/// Top-priority overrides that xcodebuild forces regardless of user
/// settings when emitting `-showBuildSettings`. Layer this ABOVE the
/// user-authored layers so it wins unconditionally.
///
/// `ENABLE_PREVIEWS`, `LD_EXPORT_GLOBAL_SYMBOLS`, and
/// `GCC_SYMBOLS_PRIVATE_EXTERN` all flip purely on the configuration
/// name in the captured oracles — even when the user explicitly sets
/// them in the pbxproj. Mac Catalyst additionally forces a
/// deployment-target trio (macOS / Swift / triple OS version) that
/// recomputes from `IPHONEOS_DEPLOYMENT_TARGET` and ignores whatever
/// the user wrote for `MACOSX_DEPLOYMENT_TARGET`. They appear here
/// rather than in [`built_in_settings`] because that runs below the
/// user layers.
#[must_use]
// This forced-override layer is driven by many independent build-system
// facts (config, Catalyst, package deps, scheme code coverage, …); each is
// a distinct yes/no condition, so a flat flag list reads clearer here than
// folding them into ad-hoc enums.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub fn built_in_overrides(
    config_name: &str,
    is_catalyst: bool,
    supports_maccatalyst: bool,
    user_supported_platforms: Option<&str>,
    user_iphoneos_deployment_target: Option<&str>,
    product_type: Option<&str>,
    sdk_base: &str,
    destination: Option<&RunDestination>,
    has_package_product_dependencies: bool,
    code_coverage_enabled: bool,
    code_signing_required: bool,
    derive_maccatalyst_bundle_id: bool,
    user_product_bundle_identifier: Option<&str>,
    user_development_team: Option<&str>,
    user_code_sign_identity: Option<&str>,
) -> Vec<Assignment> {
    let is_debug = !config_name.eq_ignore_ascii_case("Release");
    let mut out = Vec::new();
    let mut push = |key: &str, value: &str| {
        out.push(Assignment {
            key: key.to_string(),
            conditions: Vec::new(),
            value: value.to_string(),
            condition: None,
        });
    };
    push("ENABLE_PREVIEWS", if is_debug { "YES" } else { "NO" });
    push(
        "GCC_SYMBOLS_PRIVATE_EXTERN",
        if is_debug { "NO" } else { "YES" },
    );
    if is_debug {
        push("LD_EXPORT_GLOBAL_SYMBOLS", "YES");
    }
    // `DEBUG_INFORMATION_FORMAT` defaults to `dwarf` in xcspec, but
    // xcodebuild reports `dwarf-with-dsym` for Release builds in
    // `-showBuildSettings` regardless of what the user set.
    if !is_debug {
        push("DEBUG_INFORMATION_FORMAT", "dwarf-with-dsym");
    } else if destination.is_none()
        && is_test_bundle_product_type(product_type)
        && canonicalize_sdk_base(sdk_base) != "macosx"
    {
        // Non-macOS unit/UI-test bundles get a dSYM even in Debug *in the
        // no-destination "default-target" view*: there xcodebuild reports
        // `dwarf-with-dsym` for shallow-bundle (iOS/tvOS/watchOS/visionOS) test
        // bundles, while macOS test bundles keep the plain `dwarf` default. This
        // is the same no-destination/destination split that drives
        // `ENABLE_DEBUG_DYLIB` and the no-destination app rule below: once a run
        // destination is bound (an iOS-Simulator scheme build), xcodebuild emits
        // the documented Debug `dwarf` for the very same test bundle. Gating on
        // `destination.is_none()` keeps the per-target/project-defaults captures
        // (no destination) matching without over-firing on destination-bound
        // scheme captures (verified: tuist iOS-Sim test bundles are `dwarf`).
        push("DEBUG_INFORMATION_FORMAT", "dwarf-with-dsym");
    }
    // Watch targets being built under a non-watch run destination (an
    // iPad-Sim destination building an embedded watch extension, or a
    // watchOS-Sim destination building the iPhone "container" target of a
    // watchapp2-container pair) force `ONLY_ACTIVE_ARCH=NO`: xcodebuild
    // can't single out a host arch when the destination can't run the
    // target directly, so it builds the target's full standard arch list.
    // The inverse ("designed-for-iPad" iOS framework on a visionOS-Sim
    // destination) keeps OAA=YES because the destination CAN run the
    // target natively — we don't generalise the rule beyond watch
    // <-> non-watch.
    if let Some(d) = destination {
        let dest_sdk = canonicalize_sdk_base(&d.platform);
        let target_sdk = canonicalize_sdk_base(sdk_base);
        let target_is_watch = matches!(target_sdk.as_str(), "watchos" | "watchsimulator");
        let dest_is_watch = matches!(dest_sdk.as_str(), "watchos" | "watchsimulator");
        if target_is_watch != dest_is_watch {
            push("ONLY_ACTIVE_ARCH", "NO");
            push("ARCHS", "$(ARCHS_STANDARD)");
        }
    }
    // Test bundles get the swift-testing macro plugin path appended to
    // OTHER_SWIFT_FLAGS by xcodebuild regardless of user value. The
    // `-module-alias Testing=_Testing_Unavailable` flag that UI-testing
    // bundles carry on watchOS is NOT synthesized here — the ui-testing
    // ProductType xcspec already defines
    // `OTHER_SWIFT_FLAGS = $(inherited) $(TESTING_FRAMEWORK_MODULE_ALIAS_FLAGS)`,
    // so it arrives through `$(inherited)`. We only append the plugin-path;
    // re-adding the module-alias here would duplicate it.
    if let Some(pt) = product_type {
        let sdk_canon = canonicalize_sdk_base(sdk_base);
        let watch_sdk = matches!(sdk_canon.as_str(), "watchos" | "watchsimulator");
        let suffix = match pt {
            "com.apple.product-type.bundle.unit-test" => {
                Some("-plugin-path $(TOOLCHAIN_DIR)/usr/lib/swift/host/plugins/testing".to_string())
            }
            "com.apple.product-type.bundle.ui-testing" if watch_sdk => {
                Some("-plugin-path $(TOOLCHAIN_DIR)/usr/lib/swift/host/plugins/testing".to_string())
            }
            _ => None,
        };
        if let Some(s) = suffix {
            push("OTHER_SWIFT_FLAGS", &format!("$(inherited) {s}"));
        }
        // UI-testing bundles on watch get `ENTITLEMENTS_REQUIRED=YES`.
        // The xcspec default for these targets is NO; xcodebuild flips
        // it because the test runner needs entitlements to drive the
        // watch simulator/device.
        if pt == "com.apple.product-type.bundle.ui-testing" && watch_sdk {
            push("ENTITLEMENTS_REQUIRED", "YES");
        }
    }
    // `ALLOW_TARGET_PLATFORM_SPECIALIZATION` flips to YES only for an
    // application that both supports Mac Catalyst AND links Swift Package
    // products. The package dependency is the distinguisher xcodebuild uses:
    // IceCubesApp and Kingfisher-Demo declare the same
    // `SUPPORTS_MACCATALYST=YES` + iOS/visionOS `SUPPORTED_PLATFORMS`, but
    // only IceCubesApp (which has packageProductDependencies) gets ATPS=YES
    // in the captures — Kingfisher-Demo (no package products) stays NO.
    // Extensions with package products stay NO because the gate also
    // requires product type == application. The xcspec default is NO and
    // the setting's own description ("build for the platforms of any targets
    // which depend on them") matches this multi-platform-package behavior.
    if supports_maccatalyst
        && product_type == Some("com.apple.product-type.application")
        && has_package_product_dependencies
    {
        push("ALLOW_TARGET_PLATFORM_SPECIALIZATION", "YES");
    }
    // A Catalyst-supporting target built for a macOS destination gets
    // `macosx` appended to its user-authored `SUPPORTED_PLATFORMS` (kept
    // verbatim otherwise). Applies to extensions too — it gates on the
    // Catalyst flag and the macOS destination, not the product type.
    if supports_maccatalyst
        && destination.is_some_and(|d| canonicalize_sdk_base(&d.platform) == "macosx")
        && let Some(user_sp) = user_supported_platforms
        && !user_sp.split_whitespace().any(|p| p == "macosx")
    {
        push("SUPPORTED_PLATFORMS", &format!("{user_sp} macosx"));
    }
    // RESIDUAL (SUPPORTED_PLATFORMS, 1): one capture shows xcodebuild emitting a
    // duplicate "macosx macosx" token; that's an xcodebuild quirk, not a rule —
    // reproducing it would be over-fitting, so we leave the single-token value.
    if is_catalyst && let Some(user_target) = user_iphoneos_deployment_target {
        let ios_effective = apply_catalyst_ios_floor(user_target);
        push("SWIFT_DEPLOYMENT_TARGET", &ios_effective);
        push(
            "LLVM_TARGET_TRIPLE_OS_VERSION",
            &format!("ios{ios_effective}"),
        );
        push(
            "MACOSX_DEPLOYMENT_TARGET",
            &catalyst_macos_target(&ios_effective),
        );
    }
    // A scheme whose `TestAction` has `codeCoverageEnabled="YES"` forces
    // `CLANG_COVERAGE_MAPPING=YES` on every target it resolves, overriding the
    // xcspec default of NO. This is a scheme-level fact, not a per-target
    // setting, so it can only arrive via the query (see ResolveQuery).
    if code_coverage_enabled {
        push("CLANG_COVERAGE_MAPPING", "YES");
    }
    // RESIDUAL (CLANG_COVERAGE_MAPPING, 2): the Alamofire visionOS scheme's
    // TestAction enables coverage via a .xctestplan that isn't captured under
    // fixtures/raw, so `code_coverage_enabled` can't be inferred here — data gap.
    // When `CODE_SIGNING_REQUIRED` resolves to NO — set by the
    // `framework` / `library.dynamic` ProductType `DefaultBuildProperties`
    // in `ProductTypes.xcspec` — xcodebuild forces `CODE_SIGN_IDENTITY = "-"`
    // (ad-hoc) regardless of the per-SDK literal default that
    // `SDKSettings.plist` would otherwise supply (e.g. macOS device's
    // "Apple Development"). Across the corpus every entry with
    // CODE_SIGNING_REQUIRED=NO reports CODE_SIGN_IDENTITY="-" with no
    // exceptions, so this is an unconditional override above the SDK default.
    if !code_signing_required {
        push("CODE_SIGN_IDENTITY", "-");
    } else if is_test_bundle_product_type(product_type)
        && canonicalize_sdk_base(sdk_base) == "macosx"
        && user_code_sign_identity.is_none_or(str::is_empty)
        && user_development_team.is_none_or(str::is_empty)
    {
        // A macOS unit/UI-test bundle with no signing team and no authored
        // identity signs ad-hoc ("Sign to Run Locally"), which xcodebuild
        // reports as CODE_SIGN_IDENTITY="-" even though CODE_SIGNING_REQUIRED
        // stays YES for the test product type (so the branch above doesn't
        // fire). Our per-SDK default would otherwise surface the macOS
        // SDKSettings literal "Apple Development". Scoped to macOS test bundles
        // — the only product type the corpus proves this for — and gated on
        // no-team/no-identity so a team-set bundle keeps its resolved value.
        // (macOS *apps* are deliberately left to the SDK default; see
        // `code_sign_identity_forced_dash_when_signing_not_required`.)
        push("CODE_SIGN_IDENTITY", "-");
    }
    // Mac Catalyst targets that opt into
    // `DERIVE_MACCATALYST_PRODUCT_BUNDLE_IDENTIFIER=YES` (xcspec default NO,
    // iOSDevice.xcspec) get `maccatalyst.` prepended to their resolved
    // `PRODUCT_BUNDLE_IDENTIFIER`. Kingfisher-Demo authors the flag and its
    // macOS captures show `maccatalyst.com.onevcat.Kingfisher-Demo`; the
    // Catalyst IceCubesApp leaves the flag at its NO default and keeps the
    // bare id. The user value may itself be a `$(...)` recipe, so we push it
    // verbatim and let the resolver expand the prefixed form. Guard against
    // double-prefixing in case the user already wrote the prefix.
    if is_catalyst
        && derive_maccatalyst_bundle_id
        && let Some(id) = user_product_bundle_identifier
        && !id.starts_with("maccatalyst.")
    {
        push("PRODUCT_BUNDLE_IDENTIFIER", &format!("maccatalyst.{id}"));
    }
    out
}

fn detect_developer_dir() -> String {
    crate::xcode::detect_developer_dir()
        .to_string_lossy()
        .into_owned()
}

/// Filesystem name of the platform under `<Xcode>/Contents/Developer/Platforms/`.
fn platform_dir_name_for(sdk_base: &str) -> &'static str {
    match sdk_base {
        "iphoneos" => "iPhoneOS",
        "iphonesimulator" => "iPhoneSimulator",
        "appletvos" => "AppleTVOS",
        "appletvsimulator" => "AppleTVSimulator",
        "watchos" => "WatchOS",
        "watchsimulator" => "WatchSimulator",
        "xros" => "XROS",
        "xrsimulator" => "XRSimulator",
        "driverkit" => "DriverKit",
        // macosx and unknown fall through to MacOSX.
        _ => "MacOSX",
    }
}

/// User-facing display name of the platform (what xcodebuild surfaces as
/// `PLATFORM_DISPLAY_NAME`; e.g. `macOS`, `iOS`, `tvOS`).
fn platform_display_name(sdk_base: &str) -> &'static str {
    match sdk_base {
        "iphoneos" | "iphonesimulator" => "iOS",
        "appletvos" | "appletvsimulator" => "tvOS",
        "watchos" | "watchsimulator" => "watchOS",
        "xros" | "xrsimulator" => "visionOS",
        "driverkit" => "DriverKit",
        // macosx and anything unknown fall through to macOS.
        _ => "macOS",
    }
}

fn effective_platform_name_for(sdk_base: &str) -> String {
    // xcodebuild emits an empty string for macosx (so paths like
    // `$(BUILD_DIR)/$(CONFIGURATION)$(EFFECTIVE_PLATFORM_NAME)` collapse to
    // `Debug` rather than `Debug-macosx`). All other platforms get a
    // dash-prefixed SDK base.
    match sdk_base {
        "macosx" => String::new(),
        other => format!("-{other}"),
    }
}

/// Host-derived facts the resolver reports in build settings but that are not
/// a function of any project input: the machine's architecture (`NATIVE_ARCH`
/// family, destination-collapsed `ARCHS`), the login user (`USER`,
/// `VERSION_INFO_BUILDER`, `INSTALL_OWNER`, …), and the home directory (every
/// DerivedData-anchored path). `None` fields keep the live detection.
#[derive(Debug, Default)]
pub struct HostOverride {
    pub arch: Option<String>,
    pub user: Option<String>,
    pub home: Option<String>,
}

static HOST_OVERRIDE: std::sync::OnceLock<HostOverride> = std::sync::OnceLock::new();

/// Pin host-derived facts instead of detecting them from the running process.
/// First call wins for the process. The corpus oracles pin the capture host's
/// identity so `cargo test` scores identically on any machine (the floors were
/// calibrated against captures taken on one specific Mac); the
/// `SWEETPAD_HOST_ARCH` env var pins the arch alone without code.
pub fn set_host_override(host: HostOverride) {
    let _ = HOST_OVERRIDE.set(host);
}

fn host_override(field: impl Fn(&HostOverride) -> Option<&String>) -> Option<String> {
    HOST_OVERRIDE.get().and_then(|o| field(o).cloned())
}

pub(crate) fn host_arch() -> String {
    if let Some(arch) = host_override(|o| o.arch.as_ref()) {
        return arch;
    }
    if let Ok(arch) = std::env::var("SWEETPAD_HOST_ARCH")
        && !arch.is_empty()
    {
        return arch;
    }
    if cfg!(target_arch = "aarch64") {
        "arm64".into()
    } else if cfg!(target_arch = "x86_64") {
        "x86_64".into()
    } else {
        "arm64".into()
    }
}

fn host_user() -> String {
    host_override(|o| o.user.as_ref()).unwrap_or_else(|| std::env::var("USER").unwrap_or_default())
}

fn host_home() -> String {
    host_override(|o| o.home.as_ref()).unwrap_or_else(|| std::env::var("HOME").unwrap_or_default())
}

/// Return the path Xcode would hash for the DerivedData folder name: a
/// standalone `.xcworkspace` if one sits next to (or one directory above)
/// the `.xcodeproj`, else the `.xcodeproj` itself.
///
/// The `.xcodeproj`'s own embedded `project.xcworkspace` (Xcode's auto-
/// generated stub) is skipped — only USER-authored workspaces count.
fn find_derived_data_container(xcodeproj: &Path) -> PathBuf {
    let parent = xcodeproj.parent();
    for dir in [parent, parent.and_then(Path::parent)].iter().flatten() {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut workspaces: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(OsStr::to_str) == Some("xcworkspace")
                    // Skip the `.xcodeproj/project.xcworkspace` stub Xcode
                    // generates inside every project bundle.
                    && p.parent().and_then(Path::extension).and_then(OsStr::to_str)
                        != Some("xcodeproj")
            })
            .collect();
        workspaces.sort();
        if let Some(ws) = workspaces.into_iter().next() {
            return ws;
        }
    }
    xcodeproj.to_path_buf()
}

fn canonicalize_sdk_base(sdk: &str) -> String {
    let trimmed = sdk.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if trimmed.is_empty() {
        sdk.to_string()
    } else {
        trimmed.to_string()
    }
}

fn platform_metadata(sdk_base: &str) -> (&'static [&'static str], &'static str, &'static str) {
    match sdk_base {
        "macosx" => (&["arm64", "x86_64"], "macos", "MACOSX_DEPLOYMENT_TARGET"),
        "iphoneos" => (&["arm64"], "ios", "IPHONEOS_DEPLOYMENT_TARGET"),
        "iphonesimulator" => (&["arm64", "x86_64"], "ios", "IPHONEOS_DEPLOYMENT_TARGET"),
        "appletvos" => (&["arm64"], "tvos", "TVOS_DEPLOYMENT_TARGET"),
        "appletvsimulator" => (&["arm64", "x86_64"], "tvos", "TVOS_DEPLOYMENT_TARGET"),
        "watchos" => (
            &["arm64", "arm64_32"],
            "watchos",
            "WATCHOS_DEPLOYMENT_TARGET",
        ),
        "watchsimulator" => (&["arm64", "x86_64"], "watchos", "WATCHOS_DEPLOYMENT_TARGET"),
        "xros" => (&["arm64"], "xros", "XROS_DEPLOYMENT_TARGET"),
        "xrsimulator" => (&["arm64", "x86_64"], "xros", "XROS_DEPLOYMENT_TARGET"),
        "driverkit" => (
            &["arm64", "x86_64"],
            "driverkit",
            "DRIVERKIT_DEPLOYMENT_TARGET",
        ),
        _ => (&["arm64"], "macos", "MACOSX_DEPLOYMENT_TARGET"),
    }
}

fn is_64bit(arch: &str) -> bool {
    matches!(arch, "arm64" | "arm64e" | "x86_64" | "arm64_32")
}

/// Whether the watchOS *device* standard arch list still carries the legacy
/// 32-bit `armv7k`. Apple retired armv7k with watchOS 9, so the captured
/// no-destination oracles include it only when `WATCHOS_DEPLOYMENT_TARGET`
/// targets watchOS 8 or earlier. An absent value means the SDK default
/// (watchOS 26 in this toolchain) applies, which drops it.
fn watchos_keeps_armv7k(deployment_target: Option<&str>) -> bool {
    deployment_target
        .and_then(|v| v.split('.').next())
        .and_then(|major| major.trim().parse::<u32>().ok())
        .is_some_and(|major| major < 9)
}

/// `VALID_ARCHS` per SDK family. The captured oracles show these are
/// platform-pinned and don't depend on the destination device.
/// The tvOS/visionOS *device* SDKs (`appletvos`/`xros`) report `arm64 arm64e`
/// (their no-destination oracles; cf. tvOSDevice/xrOSDevice xcspec). The
/// corpus oracle only ever binds the *simulator* SDKs for these platforms,
/// so it stays on the `arm64 x86_64` default below.
fn valid_archs_for(sdk_base: &str) -> &'static str {
    match sdk_base {
        "macosx" => "arm64 arm64e i386 x86_64",
        "iphoneos" => "arm64 arm64e armv7 armv7s",
        "appletvos" | "xros" => "arm64 arm64e",
        "watchos" => "arm64 arm64_32 arm64e armv7k",
        _ => "arm64 x86_64",
    }
}

/// `ARCHS_STANDARD_32_64_BIT` per SDK family. On macOS this includes i386;
/// on simulators it's the same arch pair as `ARCHS_STANDARD`. Apple's
/// captured output orders the 32-bit arch FIRST for `iphoneos` (so
/// `armv7 arm64`, not `arm64 armv7`). The tvOS/visionOS *device* SDKs have no
/// 32-bit slice, so their no-destination oracles report the bare `arm64`
/// (cf. tvOSDevice/xrOSDevice xcspec `RealArchitectures = ( arm64 )`); the
/// matching simulators keep the `arm64 x86_64` default below.
fn archs_standard_32_64_bit_for(sdk_base: &str) -> &'static str {
    match sdk_base {
        "macosx" => "arm64 x86_64 i386",
        "iphoneos" => "armv7 arm64",
        "appletvos" | "xros" => "arm64",
        "watchos" => "arm64_32 armv7k",
        _ => "arm64 x86_64",
    }
}

/// `ARCHS_STANDARD_32_BIT` per SDK family. Empty for platforms that have
/// no 32-bit variant in their support matrix.
fn archs_standard_32_bit_for(sdk_base: &str) -> &'static str {
    match sdk_base {
        "macosx" => "i386",
        "iphoneos" => "armv7",
        // watchOS keeps arm64_32 in the 32-bit list: it's a 32-bit ABI on
        // a 64-bit ISA, used by Apple Watch Series 4–8 binaries that have
        // to remain compact.
        "watchos" => "armv7k arm64_32",
        _ => "",
    }
}

/// macOS preserves the on-disk format (devs edit it as XML); every other
/// platform writes a binary plist for runtime size + parse speed.
fn plist_output_format_for(sdk_base: &str) -> &'static str {
    if sdk_base == "macosx" {
        "same-as-input"
    } else {
        "binary"
    }
}

/// Strings file output encoding: macOS writes `UTF-16` text; every other
/// platform compiles to `binary` (the proprietary plist-like format).
fn strings_output_encoding_for(sdk_base: &str) -> &'static str {
    if sdk_base == "macosx" {
        "UTF-16"
    } else {
        "binary"
    }
}

/// Internal Apple flag — `NO` on macOS, `YES` elsewhere. xcspec
/// expressions like `__IS_NOT_MACOS_$(PLATFORM_NAME):default=YES` depend
/// on this being explicit.
fn is_not_macos_for(sdk_base: &str) -> &'static str {
    if sdk_base == "macosx" { "NO" } else { "YES" }
}

/// True for SDKs whose products run on real hardware (versus simulators
/// or the macOS host). Used to pick flags like
/// `STRIP_BITCODE_FROM_COPIED_FILES`.
fn is_device_platform(sdk_base: &str) -> bool {
    matches!(
        sdk_base,
        "iphoneos" | "appletvos" | "watchos" | "xros" | "driverkit"
    )
}

/// True when the product type identifies an XCTest-style test bundle —
/// either a unit-test bundle, a UI-testing bundle, or any subtype that
/// inherits from them. xcodebuild treats these specially by appending
/// the platform-bundled XCTest framework path to
/// `SYSTEM_FRAMEWORK_SEARCH_PATHS`.
#[must_use]
pub fn is_test_bundle_product_type(product_type: Option<&str>) -> bool {
    matches!(
        product_type,
        Some(
            "com.apple.product-type.bundle.unit-test"
                | "com.apple.product-type.bundle.ui-testing"
                | "com.apple.product-type.bundle.external-test"
                | "com.apple.product-type.bundle.ocunit-test"
        )
    )
}

/// True only for unit-style test bundles — the ones xcodebuild nests into a
/// host application's `PlugIns` directory (`TARGET_BUILD_SUBPATH =
/// /<host>.app/PlugIns`). UI-testing bundles are excluded: they build into
/// their *own* XCTRunner app, not the host's PlugIns — the corpus captures
/// report `TARGET_BUILD_SUBPATH = /<PRODUCT_NAME>-Runner.app/PlugIns` and
/// `USES_XCTRUNNER = YES` for them (e.g. tuist `ios_app_with_watchapp2`'s
/// `WatchAppUITests`). Use [`is_test_bundle_product_type`] for behavior
/// shared by every XCTest bundle (XCTest framework search paths, the
/// test-host target edge).
#[must_use]
pub fn is_unit_test_bundle_product_type(product_type: Option<&str>) -> bool {
    matches!(
        product_type,
        Some(
            "com.apple.product-type.bundle.unit-test"
                | "com.apple.product-type.bundle.external-test"
                | "com.apple.product-type.bundle.ocunit-test"
        )
    )
}

/// Default `ENABLE_DEBUG_DYLIB` for a product type, after Apple's
/// `DarwinProductTypes.xcspec`.
///
/// The spec sets the default per product type: `application` and the whole
/// `app-extension` family default `YES`; the stub-binary product types
/// (sticker pack, watch container, Messages app) hard-code `NO` because their
/// thin-stub executable can't host the dylib wrapper.
///
/// One divergence from the spec: plain `application` in Release. The spec says
/// `YES`, but `xcodebuild -showBuildSettings` reports `NO` for most apps and
/// `YES` for a minority, keyed on a build-system heuristic that isn't a
/// function of any input we can see — it splits e.g. NetNewsWire / Kingfisher /
/// Alamofire's iOS Example (`YES`) from ice-cubes / tuist-generated apps
/// (`NO`) with otherwise-identical declared settings, and doesn't track
/// project format, deployment target, or `ONLY_ACTIVE_ARCH`. `Debug→YES /
/// Release→NO` matches the majority for apps. App-extensions, by contrast,
/// keep `YES` in Release across the entire corpus, so they're unconditional.
///
/// This Release→NO branch is therefore best-effort and irreducible. On the
/// corpus it is correct for 59 of 68 Release `application` configs (~87%); the
/// 9 misses are the `YES` minority and span iOS / macOS / tvOS / visionOS, so
/// no observable input (`IPHONEOS_DEPLOYMENT_TARGET`, `SUPPORTS_MACCATALYST`,
/// product type, pbxproj/xcconfig, project format) predicts them. The gate is
/// an opaque xcodebuild runtime decision; matching it exactly would require
/// keying on project identity, which we deliberately don't do. `NO` is kept as
/// the safe majority default.
///
/// Product types that don't emit `ENABLE_DEBUG_DYLIB` at all (frameworks,
/// libraries, tools, test bundles) fall through to `NO`; the value is never
/// compared for them.
fn enable_debug_dylib_default(product_type: Option<&str>, is_debug: bool) -> &'static str {
    match product_type {
        // Stub-binary product types: incompatible with the dylib wrapper.
        Some(
            "com.apple.product-type.application.messages"
            | "com.apple.product-type.application.watchapp2"
            | "com.apple.product-type.application.watchapp2-container"
            | "com.apple.product-type.app-extension.messages-sticker-pack",
        ) => "NO",
        // App-extension family inherits YES and keeps it in every config.
        Some(
            "com.apple.product-type.extensionkit-extension"
            | "com.apple.product-type.watchkit2-extension",
        ) => "YES",
        Some(pt) if pt.starts_with("com.apple.product-type.app-extension") => "YES",
        // Plain applications: YES in Debug, NO in Release (majority match).
        Some("com.apple.product-type.application") => {
            if is_debug {
                "YES"
            } else {
                "NO"
            }
        }
        _ => "NO",
    }
}

/// Hardware identifier xcodebuild forwards to the asset catalog compiler
/// for the named simulator device. Returns `""` when we don't recognise
/// the device label — the caller suppresses the asset-catalog filter
/// settings in that case (rather than emit a wrong value).
fn device_model_for(device_name: &str) -> &'static str {
    match device_name {
        "iPad-A16" => "iPad15,7",
        "iPad-10th-generation" => "iPad13,18",
        "Apple-TV" => "AppleTV5,3",
        "Apple-Vision-Pro" => "RealityDevice14,1",
        "Apple-Watch-SE-3-40mm" => "Watch7,13",
        _ => "",
    }
}

/// Best-effort lookup of the running macOS version. Used to fill
/// `ASSETCATALOG_FILTER_FOR_DEVICE_OS_VERSION` when the destination is
/// macOS but the target's platform isn't.
fn host_os_version() -> String {
    if let Ok(output) = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        && output.status.success()
    {
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    String::new()
}

/// Collapse runs of whitespace introduced by multi-line Rust string
/// literals into single spaces, then trim leading/trailing whitespace.
/// xcodebuild's flag-list outputs use exactly one space between tokens.
fn normalize_flag_string(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Like [`normalize_flag_string`] but preserves a single leading space
/// if the input had any. Used for build-settings whose canonical
/// captured shape starts with one space (e.g.
/// `SYSTEM_FRAMEWORK_SEARCH_PATHS = " /path1 /path2"`).
fn collapse_whitespace_preserving_leading_space(s: &str) -> String {
    let starts_with_ws = s.starts_with(|c: char| c.is_whitespace());
    let collapsed = normalize_flag_string(s);
    if starts_with_ws {
        format!(" {collapsed}")
    } else {
        collapsed
    }
}

/// Default `SUPPORTED_PLATFORMS` for an SDK — typically the device +
/// simulator pair of that family. User-authored xcconfigs override this;
/// our base is what flows through when the target says nothing.
fn supported_platforms_for(sdk_base: &str) -> String {
    match sdk_base {
        "macosx" => "macosx".into(),
        "iphoneos" | "iphonesimulator" => "iphoneos iphonesimulator".into(),
        "appletvos" | "appletvsimulator" => "appletvos appletvsimulator".into(),
        "watchos" | "watchsimulator" => "watchos watchsimulator".into(),
        "xros" | "xrsimulator" => "xros xrsimulator".into(),
        "driverkit" => "driverkit".into(),
        _ => sdk_base.to_string(),
    }
}

/// Internal Apple flag — `NO` on simulators, `YES` for device builds and
/// macOS. The destination wins when available; otherwise we infer from
/// the SDK name's `simulator` suffix.
fn is_not_simulator_for(sdk_base: &str, destination: Option<&RunDestination>) -> &'static str {
    if let Some(d) = destination {
        if d.is_simulator() { "NO" } else { "YES" }
    } else if sdk_base.ends_with("simulator") {
        "NO"
    } else {
        "YES"
    }
}

/// Read `$DARWIN_USER_CACHE_DIR` (which Xcode sets from
/// `confstr(_CS_DARWIN_USER_CACHE_DIR)`). Fall back to `$TMPDIR` with the
/// final `T/` segment swapped for `C/`, which is how macOS lays out per-
/// user caches. Returns a trailing-slash-terminated string so callers
/// can concatenate sub-paths directly.
fn darwin_user_cache_dir() -> String {
    if let Ok(v) = std::env::var("DARWIN_USER_CACHE_DIR")
        && !v.is_empty()
    {
        return ensure_trailing_slash(&v);
    }
    if let Ok(tmp) = std::env::var("TMPDIR")
        && !tmp.is_empty()
    {
        // `$TMPDIR` is usually `/var/folders/<x>/<y>/T/`; swap the last
        // segment to `C/` to get the cache root.
        let trimmed = tmp.trim_end_matches('/');
        if let Some(stripped) = trimmed.strip_suffix("/T") {
            return format!("{stripped}/C/");
        }
        return ensure_trailing_slash(&tmp);
    }
    "/tmp/".into()
}

fn ensure_trailing_slash(s: &str) -> String {
    if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{s}/")
    }
}

/// Active Xcode's build version (e.g. `26.0.1-17A400`).
fn xcode_product_build_version() -> String {
    crate::xcode::active_install().product_build_version()
}

/// Encode an Xcode version string `A.B.C` into xcodebuild's
/// `(XCODE_VERSION_MAJOR, XCODE_VERSION_MINOR, XCODE_VERSION_ACTUAL)` numbers:
/// `A*100`, `A*100 + B*10`, `A*100 + B*10 + C` (missing `B`/`C` default to 0).
/// Returns `None` if the leading major component isn't numeric.
fn xcode_version_numbers(version: &str) -> Option<(String, String, String)> {
    let mut parts = version.split('.');
    let a: u32 = parts.next()?.trim().parse().ok()?;
    let b: u32 = parts
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let c: u32 = parts
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let major = a * 100;
    let minor = major + b * 10;
    let actual = minor + c;
    Some((major.to_string(), minor.to_string(), actual.to_string()))
}

fn find_target<'a>(
    objects: &'a BTreeMap<String, Value>,
    project_obj: &Value,
    name: &str,
) -> Result<Option<&'a Value>, Error> {
    let Some(ids) = project_obj.get("targets").and_then(Value::as_array) else {
        return Ok(None);
    };
    for v in ids {
        let id = v
            .as_str()
            .ok_or_else(|| Error::BadProject("target reference is not a string".into()))?;
        if let Some(target) = objects.get(id)
            && target.get("name").and_then(Value::as_str) == Some(name)
        {
            return Ok(Some(target));
        }
    }
    Ok(None)
}

/// The test-host target named by the root PBXProject's
/// `attributes.TargetAttributes.<test-target-uuid>.TestTargetID` — the
/// authoritative edge Xcode records when the user picks a host app in the
/// test target's settings. The attribute value is a target UUID; resolve it
/// to the target's name. `None` when the project carries no TargetAttributes
/// entry for the test target (older or generated pbxprojs) — callers fall
/// back to scanning the dependency edges.
fn test_target_id_host(
    objects: &BTreeMap<String, Value>,
    project_obj: &Value,
    target_name: &str,
) -> Option<String> {
    // TargetAttributes is keyed by the test target's UUID, so find it first.
    let target_ids = project_obj.get("targets").and_then(Value::as_array)?;
    let test_id = target_ids.iter().filter_map(Value::as_str).find(|id| {
        objects
            .get(*id)
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            == Some(target_name)
    })?;
    let host_id = project_obj
        .get("attributes")?
        .get("TargetAttributes")?
        .get(test_id)?
        .get("TestTargetID")
        .and_then(Value::as_str)?;
    objects
        .get(host_id)?
        .get("name")
        .and_then(Value::as_str)
        .map(String::from)
}

/// Walk a target's `dependencies` (each a `PBXTargetDependency` pointing at a
/// `target`) and return the `name` of the first dependency whose `productType`
/// is an application. That target is the XCTest host: xcodebuild reads its
/// product wrapper to compute the test bundle's `TEST_HOST` /
/// `TARGET_BUILD_SUBPATH`. A library test bundle whose only dependency is a
/// framework/library returns `None` (it has no host app, so no subpath).
fn find_app_host_target(objects: &BTreeMap<String, Value>, target_obj: &Value) -> Option<String> {
    let deps = target_obj.get("dependencies").and_then(Value::as_array)?;
    for dep_ref in deps {
        let Some(host) = dep_ref
            .as_str()
            .and_then(|id| objects.get(id))
            .and_then(|dep| dep.get("target").and_then(Value::as_str))
            .and_then(|id| objects.get(id))
        else {
            continue;
        };
        let is_app = host
            .get("productType")
            .and_then(Value::as_str)
            .is_some_and(|pt| pt.starts_with("com.apple.product-type.application"));
        if is_app {
            return host.get("name").and_then(Value::as_str).map(String::from);
        }
    }
    None
}

/// The `XCBuildConfiguration` a container uses for a requested configuration
/// name, with xcodebuild's fallback semantics: an exact (case-sensitive) name
/// match wins; a name absent from the list resolves to the list's own
/// `defaultConfigurationName` (xcodebuild warns rather than erroring); a
/// missing or dangling default falls back to the list's first configuration.
/// `Ok(None)` only when the container has no usable configuration at all (no
/// list, a dangling list, or an empty one) — the caller decides whether that
/// is fatal (project level) or just an empty layer (target level).
fn find_config<'a>(
    objects: &'a BTreeMap<String, Value>,
    container: &Value,
    config_name: &str,
) -> Result<Option<&'a Value>, Error> {
    let Some(config_list) = container
        .get("buildConfigurationList")
        .and_then(Value::as_str)
        .and_then(|list_id| objects.get(list_id))
    else {
        return Ok(None);
    };
    let Some(ids) = config_list
        .get("buildConfigurations")
        .and_then(Value::as_array)
    else {
        return Ok(None);
    };
    // The list's configurations in order, skipping dangling ids.
    let mut configs = Vec::with_capacity(ids.len());
    for v in ids {
        let id = v
            .as_str()
            .ok_or_else(|| Error::BadProject("buildConfigurations entry is not a string".into()))?;
        if let Some(config) = objects.get(id) {
            configs.push(config);
        }
    }
    let by_name = |name: &str| {
        configs
            .iter()
            .find(|c| c.get("name").and_then(Value::as_str) == Some(name))
            .copied()
    };
    if let Some(config) = by_name(config_name) {
        return Ok(Some(config));
    }
    let default = config_list
        .get("defaultConfigurationName")
        .and_then(Value::as_str)
        .and_then(by_name);
    Ok(default.or_else(|| configs.first().copied()))
}

fn load_xcconfig_layer(
    objects: &BTreeMap<String, Value>,
    config: &Value,
    xcodeproj_path: &Path,
) -> Result<Vec<Assignment>, Error> {
    let xcconfig_path = if let Some(file_ref_id) = config
        .get("baseConfigurationReference")
        .and_then(Value::as_str)
    {
        resolve_file_ref_path(objects, file_ref_id, xcodeproj_path)?
    } else if let Some(anchor_id) = config
        .get("baseConfigurationReferenceAnchor")
        .and_then(Value::as_str)
        && let Some(relative_path) = config
            .get("baseConfigurationReferenceRelativePath")
            .and_then(Value::as_str)
    {
        // Newer pbxproj format used by `PBXFileSystemSynchronizedRootGroup`:
        // the anchor identifies a sync'd folder (group), and the relative
        // path is the xcconfig's path within that folder.
        resolve_anchor_relative_path(objects, anchor_id, relative_path, xcodeproj_path)
    } else {
        return Ok(Vec::new());
    };
    match resolver::flatten_xcconfig(&xcconfig_path) {
        Ok(assignments) => Ok(assignments),
        // The referenced file doesn't exist (classic trigger: a CocoaPods
        // project before `pod install`). xcodebuild warns "Unable to open base
        // configuration reference file" and resolves as if no xcconfig were
        // attached, so the layer is simply empty. A file that exists but can't
        // be read or parsed stays fatal.
        Err(resolver::Error::Io { path, source })
            if source.kind() == io::ErrorKind::NotFound && path == xcconfig_path =>
        {
            Ok(Vec::new())
        }
        Err(e) => Err(Error::BadProject(format!(
            "xcconfig {}: {e}",
            xcconfig_path.display()
        ))),
    }
}

/// Resolve an Xcode-16 `baseConfigurationReferenceAnchor` + relative path to
/// the xcconfig's on-disk location. The anchor (usually a
/// `PBXFileSystemSynchronizedRootGroup`) sits in the group tree like any other
/// node, so its directory accumulates every pathed parent group's segment and
/// honors its own `sourceTree` (`<group>`, `SOURCE_ROOT`, `<absolute>`) — the
/// same walk [`group_dir`] does for file references. A dangling anchor
/// anchors at the project dir as a best effort.
fn resolve_anchor_relative_path(
    objects: &BTreeMap<String, Value>,
    anchor_id: &str,
    relative_path: &str,
    xcodeproj_path: &Path,
) -> PathBuf {
    let project_dir = xcodeproj_path.parent().unwrap_or_else(|| Path::new("."));
    group_dir(objects, anchor_id, project_dir, 0).join(relative_path)
}

fn resolve_file_ref_path(
    objects: &BTreeMap<String, Value>,
    file_ref_id: &str,
    xcodeproj_path: &Path,
) -> Result<PathBuf, Error> {
    let file_ref = objects.get(file_ref_id).ok_or_else(|| {
        Error::BadProject(format!("PBXFileReference {file_ref_id} not in objects"))
    })?;
    let path = file_ref.get("path").and_then(Value::as_str).unwrap_or("");
    let source_tree = file_ref
        .get("sourceTree")
        .and_then(Value::as_str)
        .unwrap_or("<group>");
    let project_dir = xcodeproj_path.parent().unwrap_or_else(|| Path::new("."));
    let resolved = match source_tree {
        "<absolute>" => PathBuf::from(path),
        // `<group>` (the default) is relative to the parent group's path, which
        // is NOT always the root group — CocoaPods nests the Pod xcconfigs under
        // a group whose `path` is "Pods". Walk the parent-group chain to anchor
        // it; a root-group ref still resolves to the project dir.
        "<group>" => parent_group_dir(objects, file_ref_id, project_dir, 0).join(path),
        // `SOURCE_ROOT` is the project dir; build-time trees (BUILT_PRODUCTS_DIR,
        // etc.) don't occur for xcconfig references — anchor at the project dir.
        _ => project_dir.join(path),
    };
    Ok(resolved)
}

/// The on-disk directory a `<group>`-relative child resolves against: its parent
/// `PBXGroup`'s directory, resolved up the group chain. The mainGroup (no parent)
/// anchors at the project dir. Depth-guarded against a malformed cyclic graph.
fn parent_group_dir(
    objects: &BTreeMap<String, Value>,
    child_id: &str,
    project_dir: &Path,
    depth: usize,
) -> PathBuf {
    if depth > 64 {
        return project_dir.to_path_buf();
    }
    match parent_group_of(objects, child_id) {
        Some(parent_id) => group_dir(objects, &parent_id, project_dir, depth + 1),
        None => project_dir.to_path_buf(),
    }
}

/// The on-disk directory of a `PBXGroup`, resolving its `path` up the parent
/// chain (each `<group>` ancestor contributes its `path`).
fn group_dir(
    objects: &BTreeMap<String, Value>,
    group_id: &str,
    project_dir: &Path,
    depth: usize,
) -> PathBuf {
    if depth > 64 {
        return project_dir.to_path_buf();
    }
    let Some(group) = objects.get(group_id) else {
        return project_dir.to_path_buf();
    };
    let path = group.get("path").and_then(Value::as_str).unwrap_or("");
    let source_tree = group
        .get("sourceTree")
        .and_then(Value::as_str)
        .unwrap_or("<group>");
    match source_tree {
        "<absolute>" => PathBuf::from(path),
        "<group>" => parent_group_dir(objects, group_id, project_dir, depth + 1).join(path),
        _ => project_dir.join(path),
    }
}

/// The id of the group (`PBXGroup` / variant / version) listing `child_id` in its
/// `children`.
fn parent_group_of(objects: &BTreeMap<String, Value>, child_id: &str) -> Option<String> {
    objects.iter().find_map(|(id, v)| {
        let isa = v.get("isa").and_then(Value::as_str)?;
        if !matches!(isa, "PBXGroup" | "PBXVariantGroup" | "XCVersionGroup") {
            return None;
        }
        let children = v.get("children").and_then(Value::as_array)?;
        children
            .iter()
            .any(|c| c.as_str() == Some(child_id))
            .then(|| id.clone())
    })
}

fn extract_inline_settings(config: &Value) -> Vec<Assignment> {
    let Some(dict) = config.get("buildSettings").and_then(Value::as_dict) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(dict.len());
    for (key_with_conds, value) in dict {
        let (key, conditions) = split_conditional_key(key_with_conds);
        out.push(Assignment {
            key,
            conditions,
            value: value_to_string(value),
            condition: None,
        });
    }
    out
}

fn split_conditional_key(s: &str) -> (String, Vec<Condition>) {
    let Some(idx) = s.find('[') else {
        return (s.to_string(), Vec::new());
    };
    let key = s[..idx].to_string();
    let mut rest = &s[idx..];
    let mut conditions = Vec::new();
    while let Some(stripped) = rest.strip_prefix('[') {
        let Some(end) = stripped.find(']') else { break };
        for part in stripped[..end].split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some(eq) = part.find('=') {
                conditions.push(Condition {
                    key: part[..eq].trim().to_string(),
                    value: part[eq + 1..].trim().to_string(),
                });
            }
        }
        rest = stripped[end + 1..].trim_start();
    }
    (key, conditions)
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        Value::Dict(_) => String::new(),
    }
}

/// A scheme that builds `target`, for driving an `xcodebuild` build of it
/// (BSP `prepare`). Prefers a scheme file named exactly `target` (Xcode and
/// Tuist create a per-target scheme), shared or per-user; otherwise the first
/// scheme file (shared directory first, then the current user's) whose build
/// action references it. When the container holds no scheme file at all and
/// scheme autocreation is enabled, the target's own name qualifies —
/// `xcodebuild` accepts autocreated scheme names. `None` otherwise —
/// `xcodebuild` needs a scheme (a bare `-target` build doesn't populate the
/// products dir our search paths use).
#[must_use]
pub fn scheme_for_target(xcodeproj_path: &Path, target: &str) -> Option<String> {
    let names = crate::scheme::container_schemes(xcodeproj_path);
    if names.is_empty() {
        // No scheme file anywhere (shared or per-user): xcodebuild resolves
        // the target's autocreated scheme, unless the workspace settings
        // disable autocreation (XcodeGen / Tuist write the flag).
        return crate::scheme::autocreation_allowed(xcodeproj_path).then(|| target.to_string());
    }
    if names.iter().any(|n| n == target) {
        return Some(target.to_string());
    }
    // Resolve each name to its file (shared shadows per-user) and scan the
    // shared schemes before the per-user ones, each set in name order.
    let mut schemes: Vec<(bool, String, PathBuf)> = names
        .into_iter()
        .filter_map(|name| {
            let path = crate::scheme::find_scheme_file(xcodeproj_path, &name)?;
            let user = !path.starts_with(xcodeproj_path.join("xcshareddata"));
            Some((user, name, path))
        })
        .collect();
    schemes.sort();
    schemes
        .into_iter()
        .find(|(_, _, path)| {
            scheme_build_action_targets(path)
                .iter()
                .any(|t| t == target)
        })
        .map(|(_, name, _)| name)
}

/// The blueprint (target) names a scheme's `BuildAction` builds.
fn scheme_build_action_targets(scheme_path: &Path) -> Vec<String> {
    let Ok(root) = crate::xcscheme::parse_file(scheme_path) else {
        return Vec::new();
    };
    let Some(build_action) = root.child("BuildAction") else {
        return Vec::new();
    };
    build_action
        .descendants_named("BuildableReference")
        .iter()
        .filter_map(|e| e.attr("BlueprintName").map(String::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    // tvOS/visionOS *device* SDKs carry arm64e in VALID_ARCHS and collapse the
    // 32_64-bit list to bare arm64; their simulators keep the arm64 x86_64
    // default. Ground truth: per_target oracles for Alamofire-tvOS/visionOS.
    #[test]
    fn arch_helpers_cover_tvos_and_visionos_devices() {
        assert_eq!(valid_archs_for("appletvos"), "arm64 arm64e");
        assert_eq!(valid_archs_for("xros"), "arm64 arm64e");
        assert_eq!(archs_standard_32_64_bit_for("appletvos"), "arm64");
        assert_eq!(archs_standard_32_64_bit_for("xros"), "arm64");

        // Simulators stay on the default (corpus binds these for tvOS/visionOS).
        assert_eq!(valid_archs_for("appletvsimulator"), "arm64 x86_64");
        assert_eq!(valid_archs_for("xrsimulator"), "arm64 x86_64");
        assert_eq!(
            archs_standard_32_64_bit_for("appletvsimulator"),
            "arm64 x86_64"
        );
        assert_eq!(archs_standard_32_64_bit_for("xrsimulator"), "arm64 x86_64");

        // Unrelated platforms are unchanged.
        assert_eq!(valid_archs_for("iphoneos"), "arm64 arm64e armv7 armv7s");
        assert_eq!(archs_standard_32_64_bit_for("iphoneos"), "armv7 arm64");
        assert_eq!(archs_standard_32_64_bit_for("macosx"), "arm64 x86_64 i386");
    }

    #[test]
    fn enable_debug_dylib_per_product_type() {
        let app = Some("com.apple.product-type.application");
        let ext = Some("com.apple.product-type.app-extension");
        let widget = Some("com.apple.product-type.extensionkit-extension");
        let watch_ext = Some("com.apple.product-type.watchkit2-extension");
        let imsg_ext = Some("com.apple.product-type.app-extension.messages");
        let sticker = Some("com.apple.product-type.app-extension.messages-sticker-pack");
        let watch_container = Some("com.apple.product-type.application.watchapp2-container");
        let framework = Some("com.apple.product-type.framework");

        // Plain apps: Debug→YES / Release→NO (majority of captures).
        assert_eq!(enable_debug_dylib_default(app, true), "YES");
        assert_eq!(enable_debug_dylib_default(app, false), "NO");
        // App-extension family: YES in every configuration.
        for ext in [ext, widget, watch_ext, imsg_ext] {
            assert_eq!(enable_debug_dylib_default(ext, true), "YES");
            assert_eq!(enable_debug_dylib_default(ext, false), "YES");
        }
        // Stub-binary product types: always NO.
        for stub in [sticker, watch_container] {
            assert_eq!(enable_debug_dylib_default(stub, true), "NO");
            assert_eq!(enable_debug_dylib_default(stub, false), "NO");
        }
        // Types that don't emit the setting fall through to NO.
        assert_eq!(enable_debug_dylib_default(framework, true), "NO");
        assert_eq!(enable_debug_dylib_default(None, true), "NO");
    }

    #[test]
    fn watchos_armv7k_drops_at_deployment_target_9() {
        // Pre-9 deployment targets keep the legacy 32-bit armv7k; 9+ drop it.
        // Fixture evidence: 3.0/6.0 include it, 10.0/26.0 don't.
        assert!(watchos_keeps_armv7k(Some("3.0")));
        assert!(watchos_keeps_armv7k(Some("6.0")));
        assert!(watchos_keeps_armv7k(Some("8")));
        assert!(!watchos_keeps_armv7k(Some("9.0")));
        assert!(!watchos_keeps_armv7k(Some("10.0")));
        assert!(!watchos_keeps_armv7k(Some("26.0")));
        // Absent → SDK default (watchOS 26 in this toolchain) → dropped.
        assert!(!watchos_keeps_armv7k(None));
        // Garbage parses to None → dropped (safe default).
        assert!(!watchos_keeps_armv7k(Some("")));
        assert!(!watchos_keeps_armv7k(Some("nan")));
    }

    #[test]
    fn detect_catalyst_recognises_ios_target_on_macos_destination() {
        assert!(detect_catalyst("macosx", Some("iphoneos"), None));
        assert!(detect_catalyst("macosx", Some("appletvos"), None));
        assert!(detect_catalyst("macosx26.0", Some("watchos"), None));
        assert!(detect_catalyst("macosx", Some("xros"), None));
    }

    #[test]
    fn detect_catalyst_picks_up_supports_maccatalyst_flag() {
        // Tuist-generated projects often don't declare SDKROOT at the
        // target level — they only set SUPPORTS_MACCATALYST=YES.
        assert!(detect_catalyst("macosx", None, Some("YES")));
        assert!(detect_catalyst("macosx", None, Some("yes")));
        // SUPPORTS_MACCATALYST only triggers Catalyst on the macOS
        // destination — on iOS it's just metadata.
        assert!(!detect_catalyst("iphonesimulator", None, Some("YES")));
    }

    #[test]
    fn detect_catalyst_rejects_native_macos_and_non_macos_destinations() {
        assert!(!detect_catalyst("macosx", Some("macosx"), None));
        assert!(!detect_catalyst("macosx", None, None));
        assert!(!detect_catalyst("macosx", None, Some("NO")));
        assert!(!detect_catalyst("iphonesimulator", Some("iphoneos"), None));
        // Simulators on the iOS family aren't Catalyst — they're native.
        assert!(!detect_catalyst("macosx", Some("iphonesimulator"), None));
    }

    #[test]
    fn catalyst_ios_floor_clamps_below_13_1() {
        assert_eq!(apply_catalyst_ios_floor("10.0"), "13.1");
        assert_eq!(apply_catalyst_ios_floor("13.0"), "13.1");
        assert_eq!(apply_catalyst_ios_floor("13.1"), "13.1");
        assert_eq!(apply_catalyst_ios_floor("13.2"), "13.2");
        assert_eq!(apply_catalyst_ios_floor("18.5"), "18.5");
    }

    #[test]
    fn catalyst_macos_target_maps_ios_versions() {
        // iOS 13.x always maps to 10.15 (Catalina floor).
        assert_eq!(catalyst_macos_target("13.1"), "10.15");
        assert_eq!(catalyst_macos_target("13.5"), "10.15");
        // iOS X.y where 14 <= X <= 18 maps to (X - 3).y.
        assert_eq!(catalyst_macos_target("14.0"), "11.0");
        assert_eq!(catalyst_macos_target("18.5"), "15.5");
        // From 26 the version numbers are aligned (iOS 26 <-> macOS 26);
        // the -3 offset would invent a macOS 23 that never existed.
        assert_eq!(catalyst_macos_target("26.0"), "26.0");
        assert_eq!(catalyst_macos_target("26.2"), "26.2");
    }

    #[test]
    #[allow(clippy::too_many_lines)] // exhaustive truth-table of override gating
    fn mac_specialization_overrides_gate_correctly() {
        let macos_dest = RunDestination {
            platform: "macosx".into(),
            os_version: String::new(),
            device_name: String::new(),
            arch: "arm64".into(),
        };
        let find = |out: &[Assignment], key: &str| -> Option<String> {
            out.iter().find(|a| a.key == key).map(|a| a.value.clone())
        };
        let app = Some("com.apple.product-type.application");
        let ext = Some("com.apple.product-type.app-extension");
        let user_sp = Some("iphoneos iphonesimulator xros xrsimulator");

        // Application + Catalyst + package products on macOS → ATPS=YES and
        // SUPPORTED_PLATFORMS gains macosx (IceCubesApp).
        let ice = built_in_overrides(
            "Release",
            false,
            true,
            user_sp,
            None,
            app,
            "iphoneos",
            Some(&macos_dest),
            true,
            false,
            true,
            false,
            None,
            None,
            None,
        );
        assert_eq!(
            find(&ice, "ALLOW_TARGET_PLATFORM_SPECIALIZATION").as_deref(),
            Some("YES")
        );
        assert_eq!(
            find(&ice, "SUPPORTED_PLATFORMS").as_deref(),
            Some("iphoneos iphonesimulator xros xrsimulator macosx")
        );

        // Application + Catalyst but NO package products → ATPS stays NO
        // (omitted), still gets +macosx (Kingfisher-Demo).
        let kf = built_in_overrides(
            "Debug",
            false,
            true,
            user_sp,
            None,
            app,
            "iphoneos",
            Some(&macos_dest),
            false,
            false,
            true,
            false,
            None,
            None,
            None,
        );
        assert_eq!(find(&kf, "ALLOW_TARGET_PLATFORM_SPECIALIZATION"), None);
        assert_eq!(
            find(&kf, "SUPPORTED_PLATFORMS").as_deref(),
            Some("iphoneos iphonesimulator xros xrsimulator macosx")
        );

        // Extension with package products + Catalyst → no ATPS (not an app),
        // but still gets +macosx on macOS.
        let extension = built_in_overrides(
            "Release",
            false,
            true,
            user_sp,
            None,
            ext,
            "iphoneos",
            Some(&macos_dest),
            true,
            false,
            true,
            false,
            None,
            None,
            None,
        );
        assert_eq!(
            find(&extension, "ALLOW_TARGET_PLATFORM_SPECIALIZATION"),
            None
        );
        assert_eq!(
            find(&extension, "SUPPORTED_PLATFORMS").as_deref(),
            Some("iphoneos iphonesimulator xros xrsimulator macosx")
        );

        // No Catalyst → neither override fires regardless of destination.
        let plain = built_in_overrides(
            "Release",
            false,
            false,
            user_sp,
            None,
            app,
            "iphoneos",
            Some(&macos_dest),
            true,
            false,
            true,
            false,
            None,
            None,
            None,
        );
        assert_eq!(find(&plain, "ALLOW_TARGET_PLATFORM_SPECIALIZATION"), None);
        assert_eq!(find(&plain, "SUPPORTED_PLATFORMS"), None);

        // macosx already present → not appended twice.
        let already = built_in_overrides(
            "Release",
            false,
            true,
            Some("iphoneos macosx"),
            None,
            app,
            "iphoneos",
            Some(&macos_dest),
            false,
            false,
            true,
            false,
            None,
            None,
            None,
        );
        assert_eq!(find(&already, "SUPPORTED_PLATFORMS"), None);
    }

    #[test]
    fn code_sign_identity_forced_dash_when_signing_not_required() {
        let find = |out: &[Assignment], key: &str| -> Option<String> {
            out.iter().find(|a| a.key == key).map(|a| a.value.clone())
        };
        // CODE_SIGNING_REQUIRED=NO (frameworks / dynamic libraries) forces
        // CODE_SIGN_IDENTITY="-"; signable products leave it to the SDK default.
        let framework = Some("com.apple.product-type.framework");
        let app = Some("com.apple.product-type.application");
        let unsigned = built_in_overrides(
            "Debug", false, false, None, None, framework, "macosx", None, false, false, false,
            false, None, None, None,
        );
        assert_eq!(find(&unsigned, "CODE_SIGN_IDENTITY").as_deref(), Some("-"));
        let signed = built_in_overrides(
            "Debug", false, false, None, None, app, "macosx", None, false, false, true, false,
            None, None, None,
        );
        assert_eq!(find(&signed, "CODE_SIGN_IDENTITY"), None);
    }

    #[test]
    fn maccatalyst_bundle_id_prefix_gated_on_derive_flag() {
        let find = |out: &[Assignment], key: &str| -> Option<String> {
            out.iter().find(|a| a.key == key).map(|a| a.value.clone())
        };
        let app = Some("com.apple.product-type.application");

        // Catalyst + DERIVE flag YES → prepend `maccatalyst.` (the user value
        // may be a `$(...)` recipe; we push it verbatim for the resolver).
        let derived = built_in_overrides(
            "Debug",
            true,
            true,
            None,
            None,
            app,
            "iphoneos",
            None,
            false,
            false,
            true,
            true,
            Some("com.onevcat.$(PRODUCT_NAME:rfc1034identifier)"),
            None,
            None,
        );
        assert_eq!(
            find(&derived, "PRODUCT_BUNDLE_IDENTIFIER").as_deref(),
            Some("maccatalyst.com.onevcat.$(PRODUCT_NAME:rfc1034identifier)")
        );

        // Catalyst but DERIVE flag at its NO default (IceCubesApp) → no prefix.
        let not_derived = built_in_overrides(
            "Debug",
            true,
            true,
            None,
            None,
            app,
            "iphoneos",
            None,
            false,
            false,
            true,
            false,
            Some("com.example.App"),
            None,
            None,
        );
        assert_eq!(find(&not_derived, "PRODUCT_BUNDLE_IDENTIFIER"), None);

        // Not Catalyst → no prefix even with the flag set.
        let non_catalyst = built_in_overrides(
            "Debug",
            false,
            false,
            None,
            None,
            app,
            "iphonesimulator",
            None,
            false,
            false,
            true,
            true,
            Some("com.example.App"),
            None,
            None,
        );
        assert_eq!(find(&non_catalyst, "PRODUCT_BUNDLE_IDENTIFIER"), None);

        // Already prefixed → not doubled.
        let already = built_in_overrides(
            "Debug",
            true,
            true,
            None,
            None,
            app,
            "iphoneos",
            None,
            false,
            false,
            true,
            true,
            Some("maccatalyst.com.example.App"),
            None,
            None,
        );
        assert_eq!(find(&already, "PRODUCT_BUNDLE_IDENTIFIER"), None);
    }

    #[test]
    fn natural_sdkroot_returns_last_unconditional_value() {
        let project = vec![Assignment {
            key: "SDKROOT".into(),
            conditions: Vec::new(),
            value: "macosx".into(),
            condition: None,
        }];
        let target = vec![Assignment {
            key: "SDKROOT".into(),
            conditions: Vec::new(),
            value: "iphoneos".into(),
            condition: None,
        }];
        let layers = vec![project, target];
        // Target's iphoneos wins over project's macosx.
        assert_eq!(natural_sdkroot(&layers).as_deref(), Some("iphoneos"));
    }

    #[test]
    fn natural_sdkroot_skips_conditional_assignments() {
        let layers = vec![vec![Assignment {
            key: "SDKROOT".into(),
            conditions: vec![Condition {
                key: "arch".into(),
                value: "arm64".into(),
            }],
            value: "iphoneos".into(),
            condition: None,
        }]];
        // Only the unconditional value counts.
        assert!(natural_sdkroot(&layers).is_none());
    }

    #[test]
    fn auto_sdkroot_no_destination_pins_no_platform_defaults() {
        // A multiplatform `SDKROOT = auto` application with no -destination
        // resolves no concrete platform, so xcodebuild reports the base-spec
        // defaults rather than the macOS SDK / Catalyst values our macosx
        // fallback would otherwise pull in. Evidence: IceCubesApp's `-project`
        // capture (SKIP_INSTALL=YES, TAPI_VERIFY_MODE=ErrorsOnly,
        // ENABLE_HARDENED_RUNTIME=YES).
        let user_layers = vec![vec![Assignment {
            key: "SDKROOT".into(),
            conditions: Vec::new(),
            value: "auto".into(),
            condition: None,
        }]];
        let out = built_in_settings(
            Path::new("/tmp/Auto.xcodeproj"),
            "Auto",
            "Release",
            Some("com.apple.product-type.application"),
            "macosx", // the resolver's fallback SDK for an unresolved `auto`
            None,     // no destination -> no-platform mode
            false,
            true, // the caller's no-platform verdict (auto + no destination)
            Some("18.5"),
            None,
            &user_layers,
            None,
            None,
            None,
        );
        let get = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(get("SKIP_INSTALL"), Some("YES"));
        assert_eq!(get("TAPI_VERIFY_MODE"), Some("ErrorsOnly"));
        assert_eq!(get("ENABLE_HARDENED_RUNTIME"), Some("YES"));
    }

    #[test]
    fn concrete_sdkroot_application_installs_and_keeps_pedantic_tapi() {
        // A normal macOS application (no `auto`) is a top-level installable
        // product (SKIP_INSTALL=NO) and the no-platform pins do not fire, so
        // TAPI_VERIFY_MODE is left to the catalog (not forced to ErrorsOnly).
        let user_layers = vec![vec![Assignment {
            key: "SDKROOT".into(),
            conditions: Vec::new(),
            value: "macosx".into(),
            condition: None,
        }]];
        let out = built_in_settings(
            Path::new("/tmp/Mac.xcodeproj"),
            "Mac",
            "Release",
            Some("com.apple.product-type.application"),
            "macosx",
            None,
            false,
            false, // a concrete SDKROOT resolves a platform
            Some("18.5"),
            None,
            &user_layers,
            None,
            None,
            None,
        );
        let get = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(get("SKIP_INSTALL"), Some("NO"));
        // The no-platform TAPI override is absent, so the catalog's value wins.
        assert_eq!(get("TAPI_VERIFY_MODE"), None);
    }

    #[test]
    fn dstroot_is_keyed_on_the_project_not_the_target() {
        // CoreBuildSystem.xcspec: `DSTROOT = /tmp/$(PROJECT_NAME).dst`,
        // `INSTALL_ROOT = $(DSTROOT)`. The oracle for target `Alamofire iOS`
        // (project `Alamofire`) reports `/tmp/Alamofire.dst` for both.
        let out = built_in_settings(
            Path::new("/tmp/Alamofire.xcodeproj"),
            "Alamofire iOS",
            "Debug",
            Some("com.apple.product-type.framework"),
            "iphoneos",
            None,
            false,
            false,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        let get = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(get("DSTROOT"), Some("/tmp/Alamofire.dst"));
        assert_eq!(get("INSTALL_ROOT"), Some("/tmp/Alamofire.dst"));
    }

    #[test]
    fn only_active_arch_collapses_archs_to_the_destination_arch() {
        // ONLY_ACTIVE_ARCH=YES with a bound destination collapses ARCHS to the
        // *destination's* arch — a device destination is arm64 regardless of
        // the host machine (xcodebuild on an Intel Mac still builds arm64 for
        // an iPhone), and an explicit `-destination …,arch=…` wins likewise.
        let dest = RunDestination {
            platform: "iphoneos".into(),
            os_version: String::new(),
            device_name: String::new(),
            arch: "arm64".into(),
        };
        let out = built_in_settings(
            Path::new("/tmp/App.xcodeproj"),
            "App",
            "Debug", // ONLY_ACTIVE_ARCH defaults YES on Debug
            Some("com.apple.product-type.application"),
            "iphoneos",
            Some(&dest),
            false,
            false,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        let get = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(get("ARCHS"), Some("arm64"));

        // No destination + simulator SDK still collapses, to the host arch
        // (the simulator executes on the host).
        let out = built_in_settings(
            Path::new("/tmp/App.xcodeproj"),
            "App",
            "Debug",
            Some("com.apple.product-type.application"),
            "iphonesimulator",
            None,
            false,
            false,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        let get = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(get("ARCHS").map(String::from), Some(host_arch()));
    }

    #[test]
    fn opens_scratch_project() {
        let path =
            fixtures_root().join("_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj");
        let project = open(&path).unwrap();
        assert_eq!(project.name, "Scratch");
        assert_eq!(project.configurations, vec!["Debug", "Release"]);
        assert_eq!(project.targets.len(), 1);
        let scratch = &project.targets[0];
        assert_eq!(scratch.name, "Scratch");
        assert_eq!(scratch.isa, "PBXNativeTarget");
        assert_eq!(
            scratch.product_type.as_deref(),
            Some("com.apple.product-type.tool")
        );
        assert_eq!(scratch.configurations, vec!["Debug", "Release"]);
        // Synthetic project has no scheme files at all, so the autocreated
        // per-target schemes surface (matching `xcodebuild -list`).
        assert_eq!(project.schemes, vec!["Scratch"]);
    }

    #[test]
    fn opens_kingfisher_project() {
        let path = fixtures_root().join("kingfisher/xcode-26.5.0/raw/Kingfisher.xcodeproj");
        let project = open(&path).unwrap();
        assert_eq!(project.name, "Kingfisher");
        assert!(
            !project.targets.is_empty(),
            "Kingfisher should have targets"
        );
        assert!(
            !project.configurations.is_empty(),
            "Kingfisher should have configurations"
        );
        // We saw exactly one .xcscheme under xcshareddata earlier.
        assert_eq!(project.schemes, vec!["Kingfisher"]);
    }

    #[test]
    fn test_host_target_resolves_app_dependency() {
        // NetNewsWire's unit-test bundles depend on an *application* target —
        // that's the XCTest host, so `test_host_target` is the app's name.
        let nnw = fixtures_root().join("netnewswire/xcode-26.5.0/raw/NetNewsWire.xcodeproj");
        let ios = build_settings(&nnw, "NetNewsWire-iOSTests", "Debug").unwrap();
        assert_eq!(ios.test_host_target.as_deref(), Some("NetNewsWire-iOS"));
        let mac = build_settings(&nnw, "NetNewsWireTests", "Debug").unwrap();
        assert_eq!(mac.test_host_target.as_deref(), Some("NetNewsWire"));

        // Kingfisher's test bundle depends on a *framework* — no host app, so
        // no subpath should ever be synthesized.
        let kf = fixtures_root().join("kingfisher/xcode-26.5.0/raw/Kingfisher.xcodeproj");
        let kft = build_settings(&kf, "KingfisherTests", "Debug").unwrap();
        assert_eq!(kft.test_host_target, None);
    }

    #[test]
    fn opens_icecubes_project() {
        let path = fixtures_root().join("ice-cubes/xcode-26.5.0/raw/IceCubesApp.xcodeproj");
        let project = open(&path).unwrap();
        assert_eq!(project.name, "IceCubesApp");
        assert_eq!(
            project.schemes,
            vec![
                "IceCubesActionExtension",
                "IceCubesApp",
                "IceCubesAppWidgetsExtensionExtension",
                "IceCubesNotifications",
                "IceCubesShareExtension",
            ]
        );
    }

    #[test]
    fn scratch_build_settings_layers_for_debug() {
        let path =
            fixtures_root().join("_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj");
        let layers = build_settings_layers(&path, "Scratch", "Debug").unwrap();
        assert_eq!(layers.len(), 4);
        // Layer 0: project xcconfig (none) ; Layer 2: target xcconfig (none).
        assert!(
            layers[0].is_empty(),
            "project has no baseConfigurationReference"
        );
        assert!(
            layers[2].is_empty(),
            "target has no baseConfigurationReference"
        );
        // Layer 1: project's inline buildSettings.
        let project_keys: std::collections::BTreeSet<&str> =
            layers[1].iter().map(|a| a.key.as_str()).collect();
        for k in [
            "ALWAYS_SEARCH_USER_PATHS",
            "MACOSX_DEPLOYMENT_TARGET",
            "SDKROOT",
            "SWIFT_VERSION",
        ] {
            assert!(
                project_keys.contains(k),
                "expected `{k}` in project layer; got {project_keys:?}"
            );
        }
        // Layer 3: target's inline buildSettings (PRODUCT_NAME).
        let target_keys: std::collections::BTreeSet<&str> =
            layers[3].iter().map(|a| a.key.as_str()).collect();
        assert!(target_keys.contains("PRODUCT_NAME"));
    }

    #[test]
    fn scratch_unknown_target_errors() {
        let path =
            fixtures_root().join("_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj");
        let err = build_settings_layers(&path, "Nonexistent", "Debug").unwrap_err();
        assert!(format!("{err}").contains("no target named"));
    }

    #[test]
    fn scratch_unknown_config_falls_back_to_default() {
        // xcodebuild doesn't error on an unknown configuration name — it warns
        // and falls back to each XCConfigurationList's own
        // `defaultConfigurationName` (Release for Scratch), so the resolved
        // layers match a plain Release resolution.
        let path =
            fixtures_root().join("_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj");
        let fallback = build_settings_layers(&path, "Scratch", "Nonexistent").unwrap();
        let release = build_settings_layers(&path, "Scratch", "Release").unwrap();
        assert_eq!(fallback.len(), release.len());
        for (f, r) in fallback.iter().zip(&release) {
            let keys = |layer: &[Assignment]| {
                layer
                    .iter()
                    .map(|a| (a.key.clone(), a.value.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(keys(f), keys(r), "fallback must mirror the default config");
        }
    }

    #[test]
    fn split_conditional_key_parses_brackets() {
        let (key, conds) = split_conditional_key("OTHER_LDFLAGS[arch=arm64]");
        assert_eq!(key, "OTHER_LDFLAGS");
        assert_eq!(conds.len(), 1);
        assert_eq!(conds[0].key, "arch");
        assert_eq!(conds[0].value, "arm64");
    }

    #[test]
    fn split_conditional_key_handles_stacked() {
        let (key, conds) = split_conditional_key("FOO[sdk=iphoneos*][arch=arm64]");
        assert_eq!(key, "FOO");
        assert_eq!(conds.len(), 2);
    }

    #[test]
    fn split_conditional_key_handles_plain() {
        let (key, conds) = split_conditional_key("FOO");
        assert_eq!(key, "FOO");
        assert!(conds.is_empty());
    }

    #[test]
    fn value_to_string_joins_arrays() {
        let v = Value::Array(vec![
            Value::String("-framework".into()),
            Value::String("Foundation".into()),
        ]);
        assert_eq!(value_to_string(&v), "-framework Foundation");
    }

    #[test]
    fn project_targets_have_per_target_configurations() {
        let path =
            fixtures_root().join("_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj");
        let project = open(&path).unwrap();
        // Project-level configurations should be the same set as the target's.
        let project_set: std::collections::BTreeSet<&String> =
            project.configurations.iter().collect();
        for target in &project.targets {
            let target_set: std::collections::BTreeSet<&String> =
                target.configurations.iter().collect();
            assert_eq!(
                target_set, project_set,
                "target {} has mismatched configurations",
                target.name
            );
        }
    }
}
