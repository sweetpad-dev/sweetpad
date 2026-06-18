use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::destination::RunDestination;
use crate::pbxproj::{self, Dict, Value};
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
    /// plus one autocreated scheme per eligible target not already named by
    /// a scheme file (Xcode's scheme autocreation; see
    /// [`autocreates_scheme_for_target`] for the eligibility rules). Schemes
    /// that `xcodebuild` additionally synthesizes from Swift *package*
    /// manifests are out of scope — they aren't derivable from the pbxproj.
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
    /// The project parsed fine but declares no target with the requested
    /// name. Distinct from [`Error::BadProject`] so workspace loops can treat
    /// it as "the target lives in another member project" (see
    /// [`Error::is_lookup_miss`]) without also swallowing real IO/parse
    /// failures.
    NoSuchTarget(String),
    /// The project has no usable build configuration at all (no list, a
    /// dangling list, or an empty one) for the requested name. Like
    /// [`Error::NoSuchTarget`], a lookup miss rather than a broken project.
    NoConfigurations(String),
}

impl Error {
    /// Whether this is a target/configuration *lookup* miss: the project is
    /// readable and well-formed, it just doesn't declare what was asked for.
    /// Workspace member loops swallow exactly these (the target may live in
    /// another member) and propagate everything else — an unreadable or
    /// malformed member is a real error, not "target elsewhere".
    #[must_use]
    pub fn is_lookup_miss(&self) -> bool {
        matches!(self, Error::NoSuchTarget(_) | Error::NoConfigurations(_))
    }

    /// The canonical "no target named X" lookup-miss error.
    fn no_such_target(target_name: &str) -> Self {
        Error::NoSuchTarget(target_name.to_string())
    }
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
            Error::NoSuchTarget(t) => write!(f, "no target named '{t}' in the project"),
            Error::NoConfigurations(c) => {
                write!(f, "project has no configurations (requested '{c}')")
            }
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
    if crate::scheme::autocreation_allowed(xcodeproj_path) {
        // Mirror Xcode's per-target scheme autocreation: `xcodebuild -list`
        // reports one scheme per eligible target that no scheme file already
        // names, even when other targets DO have scheme files (kingfisher's
        // Demo project ships only `Kingfisher-Demo.xcscheme` yet lists its
        // macOS/tvOS/watchOS demo apps too; NetNewsWire lists its
        // extension targets). When the workspace settings disable
        // autocreation (XcodeGen / Tuist write the flag), `xcodebuild -list`
        // shows only the scheme files and so do we.
        let existing: std::collections::BTreeSet<&str> =
            schemes.iter().map(String::as_str).collect();
        let first_config = configurations.first().cloned();
        let autocreated: Vec<String> = targets
            .iter()
            .filter(|t| !existing.contains(t.name.as_str()))
            .filter(|t| {
                autocreates_scheme_for_target(value, xcodeproj_path, t, first_config.as_deref())
            })
            .map(|t| t.name.clone())
            .collect();
        schemes.extend(autocreated);
        schemes.dedup();
    }
    crate::scheme::sort_like_xcodebuild(&mut schemes);
    schemes.dedup();

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

/// Whether Xcode's scheme autocreation materializes a scheme for `target`.
///
/// Derived from the `xcodebuild -list` captures (`metadata/**/list.json`,
/// scored by `tests/discovery_oracle.rs`):
///
/// * test bundles never autocreate — they ride their host scheme's
///   TestAction (NetNewsWireTests / KingfisherTests / alamofire's per-
///   platform test targets are all absent from the captures);
/// * WatchKit extensions and the iPhone companion stub of a watchapp2 pair
///   never autocreate — neither runs standalone (kingfisher's watch demo
///   extension, alamofire's `watchOS Example` container). The watch *app*
///   itself does autocreate (kingfisher's `Kingfisher-watchOS-Demo`);
/// * Safari legacy extensions (`NSExtensionPointIdentifier =
///   com.apple.Safari.extension`) never autocreate — they only run inside
///   Safari (NetNewsWire's `Subscribe to Feed` is absent while its sibling
///   share/widget/intents extensions are listed);
/// * everything else — applications, the non-Safari app-extension family,
///   frameworks, libraries, tools, aggregate targets — autocreates.
fn autocreates_scheme_for_target(
    value: &Value,
    xcodeproj_path: &Path,
    target: &Target,
    first_config: Option<&str>,
) -> bool {
    match target.product_type.as_deref() {
        Some(
            "com.apple.product-type.bundle.unit-test"
            | "com.apple.product-type.bundle.ui-testing"
            | "com.apple.product-type.bundle.external-test"
            | "com.apple.product-type.bundle.ocunit-test"
            | "com.apple.product-type.watchkit2-extension"
            | "com.apple.product-type.watchkit-extension"
            | "com.apple.product-type.application.watchapp2-container",
        ) => false,
        Some(pt) if pt.starts_with("com.apple.product-type.app-extension") => {
            !is_safari_extension_target(value, xcodeproj_path, &target.name, first_config)
        }
        _ => true,
    }
}

/// Best-effort detection of a Safari legacy app extension: resolve the
/// target's authored `INFOPLIST_FILE` through its settings layers (covers a
/// value supplied by an xcconfig, e.g. NetNewsWire) and look for
/// `NSExtensionPointIdentifier = com.apple.Safari.extension` in the plist.
/// Any failure — recipe-valued path, unreadable or binary plist — counts as
/// "not Safari", so a target is never wrongly dropped from the scheme list.
fn is_safari_extension_target(
    value: &Value,
    xcodeproj_path: &Path,
    target_name: &str,
    first_config: Option<&str>,
) -> bool {
    let Some(config) = first_config else {
        return false;
    };
    let Ok(bundle) = build_settings_from_value(value, xcodeproj_path, target_name, config) else {
        return false;
    };
    let Some(plist_rel) = last_unconditional_setting(&bundle.layers, "INFOPLIST_FILE") else {
        return false;
    };
    if plist_rel.contains("$(") {
        return false;
    }
    let Some(project_dir) = xcodeproj_path.parent() else {
        return false;
    };
    let Ok(plist) = crate::xcscheme::parse_file(&project_dir.join(plist_rel.trim())) else {
        return false;
    };
    element_has_safari_extension_point(&plist)
}

/// Recursive search for a `<key>NSExtensionPointIdentifier</key>` followed by
/// `<string>com.apple.Safari.extension</string>` anywhere in an XML plist.
fn element_has_safari_extension_point(el: &crate::xcscheme::Element) -> bool {
    let mut children = el.children.iter().peekable();
    while let Some(child) = children.next() {
        if child.name == "key"
            && child.text == "NSExtensionPointIdentifier"
            && children
                .peek()
                .is_some_and(|v| v.name == "string" && v.text == "com.apple.Safari.extension")
        {
            return true;
        }
        if element_has_safari_extension_point(child) {
            return true;
        }
    }
    false
}

/// The `defaultConfigurationName` declared on a container's
/// `XCConfigurationList`, if any. Each list — the project's and every
/// target's — carries its own; xcodebuild falls back to it when asked for a
/// configuration name the list doesn't contain.
fn default_configuration_name(objects: &Dict, container: &Value) -> Option<String> {
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
fn project_root(value: &Value) -> Result<(&Dict, &Value), Error> {
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
    objects: &Dict,
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

fn config_names(objects: &Dict, config_list: &Value) -> Result<Vec<String>, Error> {
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

fn extract_targets(objects: &Dict, project_obj: &Value) -> Result<Vec<Target>, Error> {
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
    let project_config = find_config(objects, project_obj, config_name)?
        .ok_or_else(|| Error::NoConfigurations(config_name.to_string()))?;

    let target_obj = find_target(objects, project_obj, target_name)?
        .ok_or_else(|| Error::no_such_target(target_name))?;

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
            &mut BTreeSet::new(),
            0,
        );
    }

    let target = find_target(objects, project_obj, target_name)?
        .ok_or_else(|| Error::no_such_target(target_name))?;

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
    objects: &Dict,
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
    let target = find_target(objects, project_obj, target_name)?
        .ok_or_else(|| Error::no_such_target(target_name))?;

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

/// The dynamic-library products in a target's Frameworks build phase, by
/// product-base name (`libDynamicLib.dylib` → `DynamicLib`). The companion to
/// [`target_linked_frameworks`] for the `-l<name>` side of the link line.
pub fn target_linked_libraries(
    xcodeproj_path: &Path,
    target_name: &str,
) -> Result<Vec<String>, Error> {
    let value = parse_pbxproj(xcodeproj_path)?;
    let (objects, project_obj) = project_root(&value)?;
    let target = find_target(objects, project_obj, target_name)?
        .ok_or_else(|| Error::no_such_target(target_name))?;

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
                .and_then(|n| n.strip_suffix(".dylib"))
            {
                out.push(name.trim_start_matches("lib").to_string());
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
    let target = find_target(objects, project_obj, target_name)?
        .ok_or_else(|| Error::no_such_target(target_name))?;
    Ok(target_dependency_names(objects, target))
}

fn target_dependency_names(objects: &Dict, target_obj: &Value) -> Vec<String> {
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
    let target = find_target(objects, project_obj, target_name)?
        .ok_or_else(|| Error::no_such_target(target_name))?;
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
    objects: &Dict,
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
    let target = find_target(objects, project_obj, target_name)?
        .ok_or_else(|| Error::no_such_target(target_name))?;
    if target_has_script_or_rule_phase(objects, target) {
        return Ok(false);
    }
    let sources = target_source_files_from_value(&value, xcodeproj_path, target_name)?;
    let is_swift = |p: &Path| p.extension().and_then(OsStr::to_str) == Some("swift");
    Ok(sources.iter().any(|p| is_swift(p)) && sources.iter().all(|p| is_swift(p)))
}

/// Whether a target has a `PBXShellScriptBuildPhase` or any build rule — either
/// can generate sources, so the module isn't a pure `swiftc` emit.
fn target_has_script_or_rule_phase(objects: &Dict, target_obj: &Value) -> bool {
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

/// Group nesting deeper than this is treated as malformed. Real group trees
/// are tens of levels at most — and since groups are objects referenced by
/// id, their depth is NOT bounded by the pbxproj parser's nesting cap, so a
/// corrupt chain of groups could otherwise overflow the stack.
const MAX_GROUP_DEPTH: usize = 256;

/// DFS the group tree, recording `file_id → absolute path` for every leaf. A
/// group node contributes its own directory to its children; a leaf records its
/// full path. `PBXVariantGroup` / `XCVersionGroup` (localized resources, Core
/// Data model versions) are walked like groups so their members resolve.
///
/// `visited` breaks reference cycles (`G1 → G2 → G1`) and bounds the walk to
/// one visit per group even when a corrupt file shares subtrees (a crafted
/// `children = (G, G)` at every level would otherwise re-walk shared nodes
/// 2^depth times); `depth` bounds the stack on a non-cyclic chain.
// The two walk-state params push this over clippy's arity limit; a state
// struct for an internal DFS helper would be heavier than the flag list.
#[allow(clippy::too_many_arguments)]
fn resolve_group_paths<'a>(
    objects: &'a Dict,
    node_id: &'a str,
    parent_base: &Path,
    project_dir: &Path,
    out: &mut BTreeMap<String, PathBuf>,
    sync_out: &mut BTreeMap<String, PathBuf>,
    visited: &mut BTreeSet<&'a str>,
    depth: usize,
) {
    let Some(node) = objects.get(node_id) else {
        return;
    };
    let base = node_base(node, parent_base, project_dir);
    let isa = node.get("isa").and_then(Value::as_str).unwrap_or("");
    match isa {
        "PBXGroup" | "PBXVariantGroup" | "XCVersionGroup" => {
            if depth >= MAX_GROUP_DEPTH || !visited.insert(node_id) {
                return;
            }
            if let Some(children) = node.get("children").and_then(Value::as_array) {
                for child in children {
                    if let Some(cid) = child.as_str() {
                        resolve_group_paths(
                            objects,
                            cid,
                            &base,
                            project_dir,
                            out,
                            sync_out,
                            visited,
                            depth + 1,
                        );
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

/// Make `path` absolute WITHOUT resolving symlinks: a relative path anchors
/// at the current directory and `.` / `..` segments collapse lexically.
/// Xcode keys DerivedData by the container path *as opened* — a project under
/// a symlinked root (`/tmp` → `/private/tmp`) hashes the symlink spelling —
/// so the hash input must not go through [`fs::canonicalize`] (which is still
/// used for `PROJECT_DIR`/`SRCROOT`, where xcodebuild emits resolved absolute
/// paths and the VS Code extension realpaths its side).
#[must_use]
pub fn absolutize(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    };
    let mut out = PathBuf::new();
    for comp in joined.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
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

/// Pre-resolve the user-authored layers ALONE (no xcspec defaults, no
/// built-ins) into the "effective authored value" map the gating probes
/// read — the Catalyst/previews/optimization flips xcodebuild keys on what
/// the user's settings *resolve* to, not on any single raw assignment.
/// Running the real resolver over the layers gives every gate the same
/// semantics as the main pass: `[sdk=…]`/`[config=…]` conditionals match the
/// canonical bindings in `ctx`, `$(inherited)` chains fold across layers,
/// and `$(VAR)` references expand against the layered values (a reference to
/// a setting only the defaults define expands empty — the catalog isn't in
/// scope here; use [`last_matching_setting`] where the raw recipe matters).
#[must_use]
pub fn effective_authored_settings(
    layers: &[Vec<Assignment>],
    ctx: &crate::resolver::ResolveContext,
) -> BTreeMap<String, String> {
    let refs: Vec<&[Assignment]> = layers.iter().map(Vec::as_slice).collect();
    crate::resolver::resolve(&refs, ctx)
}

/// The raw value of the last assignment to `key` whose `[…]` conditions all
/// match `ctx` — later layers and later assignments win, like the resolver
/// proper, but the value comes back verbatim (`$(…)` references intact).
/// For the probes that must inspect or re-push the authored *recipe* rather
/// than its user-layer expansion: an authored `ARCHS = $(ARCHS_STANDARD)`
/// names a catalog setting that [`effective_authored_settings`] would
/// expand to empty.
#[must_use]
pub fn last_matching_setting(
    layers: &[Vec<Assignment>],
    key: &str,
    ctx: &crate::resolver::ResolveContext,
) -> Option<String> {
    for layer in layers.iter().rev() {
        for a in layer.iter().rev() {
            if a.key == key && a.conditions.iter().all(|c| ctx.matches(c)) {
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
    if matches!(supports_maccatalyst, Some(v) if v.eq_ignore_ascii_case("NO")) {
        return false;
    }
    if matches!(supports_maccatalyst, Some(v) if v.eq_ignore_ascii_case("YES")) {
        return true;
    }
    matches!(
        natural_sdk,
        Some("iphoneos" | "appletvos" | "watchos" | "xros")
    )
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
    // The effective authored values (see [`effective_authored_settings`]) the
    // remaining authored-value gates below read: the pre-resolved user layers
    // + `-xcconfig` overlay, WITHOUT the CLI `KEY=VALUE` overrides — the
    // configuration-level view xcodebuild's optimization gate evaluates (the
    // gcc-optimization-s synthetic capture keeps the debug-shaped flips with
    // a CLI-forced `s` level).
    authored: &BTreeMap<String, String>,
    derived_data_path: Option<&Path>,
    // The container the build was opened with (a `-workspace` invocation's
    // `.xcworkspace`), when it isn't this project itself. DerivedData hashes
    // this path for every member project; `None` infers the container from
    // the project's own location (see [`find_derived_data_container`]).
    derived_data_container: Option<&Path>,
    xcode_version: Option<&str>,
    // The catalog Xcode's `ProductBuildVersion` (e.g. `17F42`), from the
    // xcspec capture's `meta.json`. Feeds `XCODE_PRODUCT_BUILD_VERSION` and
    // the `<short>-<build>` segment of `CCHROOT` / `CACHE_ROOT`.
    xcode_build_version: Option<&str>,
    xcode_developer_dir: Option<&str>,
    capture_host_macos: Option<&str>,
    macos_destination_unbound: bool,
    // The driving scheme's LaunchAction sanitizer toggles (see
    // `ResolveQuery::scheme_sanitizers`).
    scheme_sanitizers: crate::scheme::SanitizerEnables,
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
    // The hash input is the container path *as opened* — absolute, but with
    // symlinks intact (Xcode under a symlinked root, `/tmp` → `/private/tmp`,
    // hashes the `/tmp` spelling), so it must NOT use the canonicalized
    // `abs_path` that PROJECT_DIR/SRCROOT report. Callers that realpath their
    // side (the VS Code extension does) see no difference.
    //
    // `xcodebuild -derivedDataPath PATH` flattens this — it replaces the
    // whole `<home>/.../DerivedData/<container-hash>` segment with `PATH`,
    // so `BUILD_DIR = PATH/Build/Products` directly.
    let derived_container = derived_data_container.map_or_else(
        || find_derived_data_container(&absolutize(xcodeproj_path)),
        |c| normalize_stub_workspace(&absolutize(c)),
    );
    let derived_name = derived_container
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_string();
    let derived_hash = derived_data_hash(&derived_container.display().to_string());
    // We model the default "Unique" build location (`<Name>-<hash>`) plus the
    // explicit `-derivedDataPath` override below. NOT modeled: the non-default
    // styles a user can set in Xcode's Locations pref, persisted to
    // `WorkspaceSettings.xcsettings` (`BuildLocationStyle` = `Shared` /
    // `CustomLocation` {Absolute, RelativeToDerivedData, RelativeToWorkspace} /
    // legacy `DeterminedByTargets`→`$(SRCROOT)/build`). When any of those is
    // set the `<Name>-<hash>` segment doesn't apply and BUILD_DIR diverges, so
    // the launcher would look in the wrong tree. We already locate + parse that
    // plist (see `scheme.rs`), so wiring the build-location keys in here is
    // plumbing — left demand-driven since it's rare and each style needs a real
    // xcodebuild capture to pin.
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
    // The Xcode whose behaviour we're mirroring: the catalog's recorded
    // version when one is attached, the host's active install otherwise.
    // Computed up front because several defaults below are version-gated.
    let xcode_short = xcode_version.map_or_else(
        || crate::xcode::active_install().short_version,
        str::to_owned,
    );
    // Xcode 15's build system predates several 16+ `-showBuildSettings`
    // behaviours (synthesized $(BUILT_PRODUCTS_DIR) search paths, the
    // device-platform bitcode strip, the unoptimized-build
    // STRIP_INSTALLED_PRODUCT flip, device-first SUPPORTED_PLATFORMS
    // ordering, the no-destination full-ARCHS view). An unknown version
    // (no catalog, no Xcode) is treated as modern.
    let legacy_xcode15 = matches!(xcode_major(&xcode_short), Some(major) if major < 16);
    // Whether this (target, config) resolves an unoptimized build —
    // `GCC_OPTIMIZATION_LEVEL = 0`. xcodebuild keys its "debug build" output
    // flips (STRIP_INSTALLED_PRODUCT, GCC_SYMBOLS_PRIVATE_EXTERN,
    // ENABLE_PREVIEWS, ...) on this resolved value, NOT on the configuration
    // *name*: the synthetic custom-config fixture's `Debug`/`Profile` configs
    // author no optimization settings and get the optimized (Release-shaped)
    // values from xcodebuild on every captured Xcode version, while tuist's
    // lowercase `debug` (which authors `GCC_OPTIMIZATION_LEVEL = 0`) gets the
    // debug-shaped ones. Corpus-wide the correlation holds with zero
    // exceptions (294 per-target captures + the custom-config fixture).
    let unoptimized = is_unoptimized_build(authored);
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
            authored
                .get("WATCHOS_DEPLOYMENT_TARGET")
                .map(String::as_str),
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
    // `LOCROOT` / `LOCSYMROOT` point at the project directory on Xcode 16+;
    // Xcode 15.x's `-showBuildSettings` reported them as empty strings in
    // every capture (kingfisher + tuist, all configs and destinations).
    if legacy_xcode15 {
        push("LOCROOT", String::new());
        push("LOCSYMROOT", String::new());
    } else {
        push("LOCROOT", project_dir_str.clone());
        push("LOCSYMROOT", project_dir_str.clone());
    }
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
    // We model the BUILD action, where `DEPLOYMENT_LOCATION=NO` and the product
    // sits in `BUILT_PRODUCTS_DIR`. REMINDER: if we ever resolve for `install`
    // / `archive`, xcodebuild flips `DEPLOYMENT_LOCATION=YES` and the product
    // moves to `$(DSTROOT)$(INSTALL_PATH)` (and `SKIP_INSTALL=YES` targets land
    // in `$(TARGET_TEMP_DIR)/UninstalledProducts/<platform>` instead) — these
    // path keys must branch on the action, not stay hard-wired to the build set.
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
    //
    // When any sanitizer is enabled the per-variant dir gets a further
    // suffix so sanitized and unsanitized objects don't mix — Swift Build's
    // `Settings.swift` appends `-asan` / `-tsan` / `-ubsan` / `-mtasan` (in
    // that order) to `OBJECT_FILE_DIR_<variant>`, and every dir derived
    // through it (`PER_ARCH_OBJECT_FILE_DIR`, `LD_DEPENDENCY_INFO_FILE`,
    // `STRINGSDATA_DIR`, …) follows. Swift Build evaluates the resolved
    // `ENABLE_*_SANITIZER` macros; our view of those is the authored value
    // plus the scheme's LaunchAction toggles, which xcodebuild layers above
    // any authored value (see `built_in_overrides`). Corpus-pinned by the
    // Alamofire `iOS Example` / `watchOS Example WatchKit App` schemes
    // (`enableThreadSanitizer="YES"` → `Objects-normal-tsan`).
    let authored_yes = |key: &str| {
        authored
            .get(key)
            .is_some_and(|v| v.eq_ignore_ascii_case("YES"))
    };
    let mut sanitizer_suffix = String::new();
    if scheme_sanitizers.address || authored_yes("ENABLE_ADDRESS_SANITIZER") {
        sanitizer_suffix.push_str("-asan");
    }
    if scheme_sanitizers.thread || authored_yes("ENABLE_THREAD_SANITIZER") {
        sanitizer_suffix.push_str("-tsan");
    }
    if scheme_sanitizers.undefined_behavior || authored_yes("ENABLE_UNDEFINED_BEHAVIOR_SANITIZER") {
        sanitizer_suffix.push_str("-ubsan");
    }
    if authored_yes("ENABLE_MEMORY_TAGGING_ADDRESS_SANITIZER") {
        sanitizer_suffix.push_str("-mtasan");
    }
    push(
        "OBJECT_FILE_DIR_normal",
        format!("$(OBJECT_FILE_DIR)-normal{sanitizer_suffix}"),
    );
    push(
        "OBJECT_FILE_DIR_debug",
        format!("$(OBJECT_FILE_DIR)-debug{sanitizer_suffix}"),
    );
    push(
        "OBJECT_FILE_DIR_profile",
        format!("$(OBJECT_FILE_DIR)-profile{sanitizer_suffix}"),
    );
    // Routed through `OBJECT_FILE_DIR_<variant>` (not a literal
    // `Objects-$(CURRENT_VARIANT)`) so the sanitizer suffix propagates —
    // the captures show `STRINGSDATA_DIR = …/Objects-normal-tsan/<arch>`.
    // Without a sanitizer this expands to the identical
    // `$(TARGET_TEMP_DIR)/Objects-normal/$(arch)` value as before.
    push(
        "STRINGSDATA_DIR",
        "$(OBJECT_FILE_DIR_$(CURRENT_VARIANT))/$(arch)".into(),
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
    // Xcode 15.x reported the *raw hardware* architecture for the host
    // family — `arm64e` on every Apple Silicon Mac (all of which expose
    // pointer authentication), with the Intel-era `i386` as the 32-bit
    // counterpart. Xcode 16 normalized the whole family to the plain
    // `arm64` / `arm` pair.
    let reported_host = if legacy_xcode15 && host == "arm64" {
        "arm64e".to_string()
    } else {
        host.clone()
    };
    let reported_32 = if legacy_xcode15 {
        "i386".to_string()
    } else {
        native_32
    };
    push("HOST_ARCH", reported_host.clone());
    push("NATIVE_ARCH", reported_host.clone());
    push("NATIVE_ARCH_ACTUAL", reported_host.clone());
    push("NATIVE_ARCH_64_BIT", reported_host);
    push("NATIVE_ARCH_32_BIT", reported_32);
    // `CURRENT_ARCH` (and its lowercase `arch` alias) is `undefined_arch` in
    // the modern aggregated `-showBuildSettings` view; Xcode 15.x reported a
    // concrete arch — the last element of the resolved `ARCHS` list (the
    // active arch when ONLY_ACTIVE_ARCH collapsed the list, `x86_64` of an
    // `arm64 x86_64` pair otherwise; both shapes appear in the 15.4 corpus).
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
    push(
        "SUPPORTED_PLATFORMS",
        supported_platforms_for(&sdk_base, legacy_xcode15),
    );
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
        // With no resolved platform the per-SDK `SDKSettings.plist` defaults
        // never apply, so `CODE_SIGN_IDENTITY` falls back to the base ad-hoc
        // `-` instead of the macOS SDK's "Apple Development" our macosx
        // fallback would pull in.
        push("CODE_SIGN_IDENTITY", "-".into());
        // Same wrapper-collapse as DWARF_DSYM_FILE_NAME above: with no
        // PackageType chain the wrapper path is empty, so the XPC services
        // folder reports the bare `/XPCServices`.
        push("XPCSERVICES_FOLDER_PATH", "/XPCServices".into());
        // No platform domain also means no macOS deep-bundle rpath: an
        // application reports the generic `@executable_path/Frameworks`
        // instead of the macosx fallback's `@executable_path/../Frameworks`.
        if product_type == Some("com.apple.product-type.application") {
            push(
                "LD_RUNPATH_SEARCH_PATHS",
                "@executable_path/Frameworks".into(),
            );
        }
        // The `application` ProductType ships `CODE_SIGNING_ALLOWED = YES`
        // unconditionally, but with no resolved platform xcodebuild reports NO
        // (a concrete signing identity can't be selected for a target whose
        // platform isn't pinned). Other product types already default to NO via
        // their own specs, so only the application case needs correcting.
        if product_type == Some("com.apple.product-type.application") {
            push("CODE_SIGNING_ALLOWED", "NO".into());
        }
    }

    // `ARCHS` resolves to the *active* arch when `ONLY_ACTIVE_ARCH=YES`
    // and to the platform's standard arch list otherwise. The user's
    // unconditional `ONLY_ACTIVE_ARCH` setting wins; when nothing is
    // authored, the effective default tracks the unoptimized-build flag
    // (the custom-config fixture's template-less `Debug` reports
    // `ONLY_ACTIVE_ARCH = NO`, so the configuration *name* is not the
    // gate). (Cross-platform destination overrides — e.g. iPhone-Sim
    // destination building an embedded watchsimulator target — are
    // applied later, in [`built_in_overrides`], so they sit above
    // user-authored values rather than below.)
    let only_active_arch_yes =
        user_only_active_arch.map_or(unoptimized, |v| v.eq_ignore_ascii_case("YES"));
    // The ONLY_ACTIVE_ARCH collapse to the host arch only happens when the
    // build is pinned to one concrete device: a bound destination, or a
    // simulator SDK (a simulator build always targets a concrete simulator,
    // so xcodebuild collapses it even when the harness can't carry the
    // `id=<uuid>` destination). On Xcode 16+ a plain `xcodebuild
    // -showBuildSettings` on a *device*/macOS SDK with no -destination
    // reports the SDK's full standard arch list regardless of
    // ONLY_ACTIVE_ARCH — it has no active device to single out (the
    // no-destination device/macOS oracles confirm ARCHS == ARCHS_STANDARD
    // even on Debug). Xcode 15.x had no such no-destination view: its
    // captures collapse to the build machine's arch whenever
    // ONLY_ACTIVE_ARCH is on, destination or not.
    let pinned_to_device =
        destination.is_some() || sdk_base.ends_with("simulator") || legacy_xcode15;
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
    } else if legacy_xcode15 {
        // Xcode 15.4's build view drops the retired 32-bit `armv7k` from
        // ARCHS even when ARCHS_STANDARD still reports it (Kingfisher's
        // watch demo, deployment target 6.0: ARCHS_STANDARD = `arm64
        // armv7k arm64_32` but ARCHS = `arm64 arm64_32` in Release).
        archs
            .iter()
            .copied()
            .filter(|a| *a != "armv7k")
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        arch_list.clone()
    };
    // Xcode 15.x reports a concrete `CURRENT_ARCH` / `arch` instead of the
    // modern `undefined_arch` placeholder: the last element of the resolved
    // ARCHS list (the collapsed active arch in Debug, the trailing `x86_64`
    // of a full `arm64 x86_64` pair in Release). Re-pushed here — after the
    // ARCHS collapse is known — to override the placeholder above.
    if legacy_xcode15 && let Some(last) = archs_value.split_whitespace().next_back() {
        push("CURRENT_ARCH", last.to_string());
        push("arch", last.to_string());
    }
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
    // `VALID_ARCHS` per SDK family. Xcode 15.4 reported different lists for
    // the device families — macOS kept the Intel-first historical list, the
    // iOS/tvOS device SDKs the bare 64-bit pair (arm64e first, no armv7/s),
    // and watchOS dropped the retired armv7k with arm64e in the middle —
    // all replaced by the modern unified lists in 16+.
    let valid_archs = match (legacy_xcode15, sdk_base.as_str()) {
        (true, "macosx") => "x86_64 x86_64h arm64 arm64e",
        (true, "iphoneos" | "appletvos") => "arm64e arm64",
        (true, "watchos") => "arm64 arm64e arm64_32",
        _ => valid_archs_for(&sdk_base),
    };
    push("VALID_ARCHS", valid_archs.into());
    push(
        "IS_MACCATALYST",
        if is_catalyst { "YES" } else { "NO" }.into(),
    );
    push("INLINE_PRIVATE_FRAMEWORKS", "NO".into());
    // STRIP_INSTALLED_PRODUCT: the xcspec default is YES; on Xcode 16+
    // xcodebuild flips it to NO for *unoptimized* builds (keyed on the
    // resolved `GCC_OPTIMIZATION_LEVEL = 0`, not the configuration name —
    // the custom-config fixture's template-less `Debug` stays YES). Xcode
    // 15.x reported the plain YES default in every capture, optimized or
    // not, so the flip is version-gated.
    let strip = if !legacy_xcode15 && unoptimized {
        "NO"
    } else {
        "YES"
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
    // platform that can actually run: any simulator SDK (a simulator build
    // always executes on a concrete simulator, so the flip happens even when
    // no parseable `-destination` reached us — the `id=<uuid>` synthetic
    // captures), or a non-macOS target bound to a real destination
    // (including the designed-for-iPad device fallback a macOS destination
    // produces for an iOS-only target). It stays NO for native macOS,
    // destination-less device archives, and the non-binding shape where a
    // macOS-natural target is forced onto a device SDK by an `-xcconfig`
    // (see `macos_destination_unbound`). Xcode 15.x reported NO in every
    // capture (Catalyst and native alike), so the flip is version-gated
    // to 16+.
    let active_resources = if !legacy_xcode15
        && (sdk_base.ends_with("simulator")
            || (sdk_base != "macosx" && destination.is_some() && !macos_destination_unbound))
    {
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
        // value of their own; otherwise their `$(inherited)` would double it.
        Some("com.apple.product-type.application.watchapp2")
            if !authored.contains_key("LD_RUNPATH_SEARCH_PATHS") =>
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
        let model_and_os = if d.is_macos() {
            // Prefer the catalog's recorded capture-host macOS version so a
            // capture resolves to the machine it was taken on; fall back to
            // querying the local host for the catalog-less CLI path.
            Some((
                "MacFamily20,1".to_string(),
                capture_host_macos.map_or_else(host_os_version, str::to_owned),
            ))
        } else {
            // A destination naming a device the model table doesn't know
            // (any real CLI `name=iPhone 16` destination) suppresses all
            // three filter settings — emitting an empty model would feed the
            // asset catalog compiler garbage, while omitting the keys just
            // skips the thinning, which is what `device_model_for` promises.
            let model = device_model_for(&d.device_name);
            (!model.is_empty()).then(|| (model.to_string(), d.os_version.clone()))
        };
        if let Some((model, os_version)) = model_and_os {
            push("ASSETCATALOG_FILTER_FOR_DEVICE_MODEL", model.clone());
            push("ASSETCATALOG_FILTER_FOR_DEVICE_OS_VERSION", os_version);
            push(
                "ASSETCATALOG_FILTER_FOR_THINNING_DEVICE_CONFIGURATION",
                model,
            );
        }
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

    // (`KASAN_DEFAULT_CFLAGS` needs no special-casing here: `SDKSettings.plist`
    // defines it with `[arch=arm64]`/`[arch=arm64e]` overrides picking the
    // HW-tagged TBI variant, and the showBuildSettings resolve binds
    // `arch=undefined_arch` — see [`crate::build_context::BuildContext`] — so
    // the catalog's unconditional CLASSIC default wins, exactly as xcodebuild
    // reports.)

    // --- Compiler/module cache paths ---------------------------------------
    // These settings expose where the user's machine caches per-build
    // state. `CCHROOT` and `CACHE_ROOT` are the same value (verified across
    // every capture of every Xcode version): the Darwin per-user cache dir
    // (`confstr(_CS_DARWIN_USER_CACHE_DIR)`, the `/var/folders/<x>/<id>/C/`
    // tree) + `com.apple.DeveloperTools/<short>-<build>/Xcode`, where
    // `<short>-<build>` is the running Xcode's marketing version and
    // ProductBuildVersion (`26.5-17F42`, `15.4-15F31d`). We compose the
    // version segment from the catalog's recorded Xcode when one is attached
    // (so a capture resolves against ITS Xcode, not the host's) and fall
    // back to the active install otherwise. The cache dir is host state —
    // read from `$DARWIN_USER_CACHE_DIR`, `$TMPDIR/../C/`, or the pinned
    // [`HostOverride::darwin_user_cache`].
    let darwin_cache = darwin_user_cache_dir();
    let xcode_build = match (xcode_version, xcode_build_version) {
        (Some(version), Some(build)) => {
            format!("{}-{build}", xcode_marketing_version(version))
        }
        _ => xcode_product_build_version(),
    };
    let cache_root = format!("{darwin_cache}com.apple.DeveloperTools/{xcode_build}/Xcode");
    push("CCHROOT", cache_root.clone());
    push("CACHE_ROOT", cache_root);
    // The bare ProductBuildVersion (`17F42`) xcodebuild surfaces alongside.
    // Only emitted when actually known — catalog meta first, the host
    // install's `version.plist` otherwise — never a guessed placeholder.
    let bare_build = xcode_build_version.map_or_else(
        || crate::xcode::active_install().build_version,
        str::to_owned,
    );
    if !bare_build.is_empty() {
        push("XCODE_PRODUCT_BUILD_VERSION", bare_build);
    }
    push(
        "XCODE_APP_SUPPORT_DIR",
        format!("{developer_dir}/Library/Xcode"),
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
    // Xcode 16+ appends BUILT_PRODUCTS_DIR-relative entries with a trailing
    // space, which is what gets surfaced in `-showBuildSettings`. Xcode 15.x
    // never synthesized these — its captures report the keys only when the
    // user authored a value (or, for LIBRARY_SEARCH_PATHS on a test bundle,
    // via the test recipe below).
    if !legacy_xcode15 {
        push(
            "HEADER_SEARCH_PATHS",
            "$(BUILT_PRODUCTS_DIR)/include ".into(),
        );
        push("LIBRARY_SEARCH_PATHS", "$(BUILT_PRODUCTS_DIR) ".into());
        push("REZ_SEARCH_PATHS", "$(BUILT_PRODUCTS_DIR) ".into());
        push("FRAMEWORK_SEARCH_PATHS", "$(BUILT_PRODUCTS_DIR) ".into());
    }
    // (XCTest bundles additionally gain `$(inherited)
    // $(TEST_LIBRARY_SEARCH_PATHS)` — but ABOVE the user layers, so it lives
    // in [`built_in_overrides`].)
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
    // device (iphoneos/appletvos/watchos/xros) — an Xcode 16+ behaviour.
    // Xcode 15.4 reports the plain CoreBuildSystem.xcspec `NO` everywhere,
    // device platforms included (all 98 of its captures).
    let strip_bitcode = if is_device_platform(&sdk_base) && !legacy_xcode15 {
        "YES"
    } else {
        "NO"
    };
    push("STRIP_BITCODE_FROM_COPIED_FILES", strip_bitcode.into());

    // `ENABLE_DEBUG_DYLIB` enables the split debug-dylib executable used by
    // previews + incremental relinking. See [`enable_debug_dylib_default`]
    // for the per-product-type rule, which follows Apple's
    // `DarwinProductTypes.xcspec`. For applications the optimized-build
    // flip only applies when the target authors `ENABLE_PREVIEWS` itself.
    let is_debug = unoptimized;
    let authored_enable_previews = authored.contains_key("ENABLE_PREVIEWS");
    push(
        "ENABLE_DEBUG_DYLIB",
        enable_debug_dylib_default(product_type, is_debug, authored_enable_previews).into(),
    );

    // Xcode 15.x wraps a watchOS UI-testing bundle in its XCTRunner app at
    // `-showBuildSettings` time, emitting the runner's Info.plist
    // preprocessor definitions and the `--deep` codesign flag (tuist's
    // watchapp2 `WatchAppUITests` per-target captures, both configs; 16+
    // stopped reporting either key). The trailing space in `--deep ` is
    // verbatim from the captures.
    if legacy_xcode15
        && product_type == Some("com.apple.product-type.bundle.ui-testing")
        && matches!(sdk_base.as_str(), "watchos" | "watchsimulator")
    {
        push(
            "INFOPLIST_PREPROCESSOR_DEFINITIONS",
            "TESTPRODUCTNAME=$(PRODUCT_NAME) \
             WRAPPEDPRODUCTBUNDLEIDENTIFIER=$(PRODUCT_BUNDLE_IDENTIFIER).xctrunner \
             TESTPRODUCTBUNDLEIDENTIFIER=$(PRODUCT_BUNDLE_IDENTIFIER) \
             WRAPPEDPRODUCTNAME=$(PRODUCT_NAME)-Runner"
                .into(),
        );
        push("OTHER_CODE_SIGN_FLAGS", "--deep ".into());
    }

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

    // `DEBUG_INFORMATION_FORMAT` for an application bound to a run
    // destination: the un-authored default is `dwarf-with-dsym` in *every*
    // configuration (iOS-Example and Kingfisher-Demo report it on simulator
    // destinations, on the device fallback an unsupported macOS destination
    // produces, and — on Xcode 15.x — for the Catalyst-capable iOS target on
    // a macOS destination), with one exception: a 16+ Catalyst build reports
    // the plain spec `dwarf` instead, in Debug AND Release (Kingfisher-Demo
    // `__macOS` on 16.4/26.5; see also the optimized-build override, which
    // is suppressed for that same shape). Native macOS apps are excluded —
    // the corpus has no unauthored native-macOS app capture, so we leave the
    // spec default in place there. DEFAULT layer, so authored values win
    // (ice-cubes/tuist/netnewswire author `dwarf` in Debug and keep it).
    if destination.is_some()
        && product_type == Some("com.apple.product-type.application")
        && (sdk_base != "macosx" || is_catalyst)
        && (!is_catalyst || legacy_xcode15)
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
    // (16.x/15.x) don't emit this key at all, so it's gated to 26+ — an
    // unknown version (no catalog, no Xcode) is treated as modern, like the
    // other version gates.
    if xcode_major(&xcode_short).is_none_or(|major| major >= 26) {
        push(
            "SWIFT_EMIT_CONST_VALUE_PROTOCOLS",
            "AnyResolverProviding AppEntity AppEnum AppExtension AppIntent AppIntentsPackage \
             AppShortcutProviding AppShortcutsProvider AppUnionValue AppUnionValueCasesProviding \
             DynamicOptionsProvider EntityQuery ExtensionPointDefining IntentValueQuery Resolver \
             TransientEntity _AssistantIntentsProvider _GenerativeFunctionExtractable \
             _IntentValueRepresentable"
                .into(),
        );
    }

    out
}

/// Top-priority overrides that xcodebuild forces regardless of user
/// settings when emitting `-showBuildSettings`. Layer this ABOVE the
/// user-authored layers so it wins unconditionally.
///
/// `ENABLE_PREVIEWS`, `LD_EXPORT_GLOBAL_SYMBOLS`, and
/// `GCC_SYMBOLS_PRIVATE_EXTERN` all flip on whether the build is
/// *unoptimized* (the resolved `GCC_OPTIMIZATION_LEVEL = 0`, see
/// [`is_unoptimized_build`]) — even when the user explicitly sets them in
/// the pbxproj, and regardless of the configuration's *name* (the
/// custom-config fixture's template-less `Debug`/`Profile` get the
/// optimized values). Mac Catalyst additionally forces a
/// deployment-target trio (macOS / Swift / triple OS version) that
/// recomputes from `IPHONEOS_DEPLOYMENT_TARGET` and ignores whatever
/// the user wrote for `MACOSX_DEPLOYMENT_TARGET`. They appear here
/// rather than in [`built_in_settings`] because that runs below the
/// user layers.
///
/// `xcode_major_version` is the major version of the Xcode being mirrored (16, 26,
/// …; 0 when unknown) — a few overrides are version-gated, e.g. Xcode 15.x
/// keeps `ENABLE_PREVIEWS = YES` in optimized builds and predates the
/// swift-testing macro plugin path.
#[must_use]
// This forced-override layer is driven by many independent build-system
// facts (config, Catalyst, package deps, scheme code coverage, …); each is
// a distinct yes/no condition, so a flat flag list reads clearer here than
// folding them into ad-hoc enums.
#[allow(
    clippy::too_many_arguments,
    clippy::fn_params_excessive_bools,
    clippy::too_many_lines
)]
pub fn built_in_overrides(
    xcode_major_version: u32,
    unoptimized: bool,
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
    user_authored_enable_previews: bool,
    user_authored_debug_information_format: bool,
    user_archs: Option<&str>,
    user_ld_runpath_search_paths: Option<&str>,
    mergeable_library: bool,
    macos_destination_unbound: bool,
    // The driving scheme's LaunchAction sanitizer toggles (see
    // `ResolveQuery::scheme_sanitizers`).
    scheme_sanitizers: crate::scheme::SanitizerEnables,
) -> Vec<Assignment> {
    let legacy_xcode15 = xcode_major_version != 0 && xcode_major_version < 16;
    let is_debug = unoptimized;
    let mut out = Vec::new();
    let mut push = |key: &str, value: &str| {
        out.push(Assignment {
            key: key.to_string(),
            conditions: Vec::new(),
            value: value.to_string(),
            condition: None,
        });
    };
    // Xcode 15.x reported `ENABLE_PREVIEWS = YES` for previews-capable
    // products in optimized (Release) builds too; the unoptimized-only flip
    // arrived with Xcode 16.
    push(
        "ENABLE_PREVIEWS",
        if is_debug || legacy_xcode15 {
            "YES"
        } else {
            "NO"
        },
    );
    push(
        "GCC_SYMBOLS_PRIVATE_EXTERN",
        if is_debug { "NO" } else { "YES" },
    );
    if is_debug {
        push("LD_EXPORT_GLOBAL_SYMBOLS", "YES");
    }
    // `DEBUG_INFORMATION_FORMAT` defaults to `dwarf` in xcspec, but
    // xcodebuild reports `dwarf-with-dsym` for optimized builds of bundled
    // products in `-showBuildSettings` regardless of what the user set.
    // Command-line tools are exempt: the custom-config fixture's tool
    // reports the plain `dwarf` default in its optimized configs on every
    // captured Xcode version. A 16+ Catalyst application that authors no
    // DEBUG_INFORMATION_FORMAT of its own is also exempt — it reports the
    // plain spec `dwarf` even optimized (Kingfisher-Demo `Release__macOS`
    // on 16.4 and 26.5; the matching Debug default lives in
    // [`built_in_settings`]).
    let catalyst_unauthored_app_dwarf = is_catalyst
        && !legacy_xcode15
        && product_type == Some("com.apple.product-type.application")
        && !user_authored_debug_information_format;
    if !is_debug
        && product_type != Some("com.apple.product-type.tool")
        && !catalyst_unauthored_app_dwarf
    {
        push("DEBUG_INFORMATION_FORMAT", "dwarf-with-dsym");
    } else if is_debug
        && (destination.is_none()
            // A macOS destination that an iOS-family test bundle can't bind
            // natively (no Catalyst) falls back to the device build — and
            // xcodebuild reports the same no-destination dSYM default there
            // (every tuist iOS test bundle's `Debug__macOS` capture).
            || (destination
                .is_some_and(|d| canonicalize_sdk_base(&d.platform) == "macosx")
                && !is_catalyst
                && canonicalize_sdk_base(sdk_base) != "macosx"))
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
        // App extensions get the same treatment on ANY simulator destination
        // of a different platform family (an iOS-only extension built for a
        // visionOS-Simulator destination, where the host app runs
        // designed-for-iPad): the extension itself can't be singled out as
        // the run target, so xcodebuild builds its full standard arch list
        // (ice-cubes' Action/Share/Widgets extensions, `Debug__visionOS-
        // Simulator`: ARCHS `arm64 x86_64`, OAA NO, while IceCubesApp —
        // which supports xros natively — keeps the active-arch collapse).
        // Applications are NOT generalized: an iOS-only *app* on the same
        // destination runs designed-for-iPad natively and keeps OAA=YES
        // (alamofire's iOS Example).
        let foreign_sim_extension = is_app_extension_family_product_type(product_type)
            && d.is_simulator()
            && dest_sdk != target_sdk
            && target_is_watch == dest_is_watch;
        // A macOS destination on a macOS-natural target that an `-xcconfig`
        // forced onto a device SDK never binds (`macos_destination_unbound`,
        // computed by the caller): xcodebuild reports the destination-less
        // device view there too — full standard ARCHS, no active-arch
        // collapse — overriding even an authored `ONLY_ACTIVE_ARCH = YES`
        // (NetNewsWire's iOS xcconfigs captured against the macOS scheme).
        if target_is_watch != dest_is_watch || foreign_sim_extension || macos_destination_unbound {
            push("ONLY_ACTIVE_ARCH", "NO");
            push("ARCHS", "$(ARCHS_STANDARD)");
        }
    }
    // An authored *literal* ARCHS list that excludes the build's active arch
    // can't honour ONLY_ACTIVE_ARCH — there is no active slice to single
    // out — so xcodebuild reports NO even in Debug (the `archs-arm64e`
    // synthetic override under a generic iOS destination: ARCHS=arm64e,
    // active arch arm64, OAA=NO). Recipe values (`$(ARCHS_STANDARD)`) are
    // left alone — we can't token-check them without a full expansion.
    if let Some(archs) = user_archs
        && !archs.contains("$(")
    {
        let active = destination
            .map(|d| d.arch.clone())
            .filter(|a| !a.is_empty())
            .unwrap_or_else(host_arch);
        if !archs.split_whitespace().any(|a| a == active) {
            push("ONLY_ACTIVE_ARCH", "NO");
        }
    }
    // XCTest bundles that author an LD_RUNPATH_SEARCH_PATHS without the
    // standard `@executable_path/Frameworks` entry get it appended by
    // xcodebuild on the iOS device SDK (NetNewsWire's iOSTests authors only
    // the loader-relative paths; both its per-target captures carry the
    // appended entry). Authored lists that already contain the token are
    // left untouched (tuist's generated tests author it explicitly).
    if is_unit_test_bundle_product_type(product_type)
        && canonicalize_sdk_base(sdk_base) == "iphoneos"
        && let Some(rp) = user_ld_runpath_search_paths
        && !rp
            .split_whitespace()
            .any(|t| t == "@executable_path/Frameworks")
    {
        push(
            "LD_RUNPATH_SEARCH_PATHS",
            "$(inherited) @executable_path/Frameworks",
        );
    }
    // A target resolving `MERGEABLE_LIBRARY = YES` gets the mergeable-library
    // treatment from xcodebuild: the Swift bundle-lookup helper condition is
    // appended in every configuration, and optimized builds additionally
    // become mergeable and skip the install strip (the `mergeable-library`
    // synthetic override; in Debug the library is built normally for the
    // debugger, so MAKE_MERGEABLE keeps its NO default and the unoptimized
    // STRIP_INSTALLED_PRODUCT=NO flip already applies).
    if mergeable_library {
        push(
            "SWIFT_ACTIVE_COMPILATION_CONDITIONS",
            "$(inherited) SWIFT_BUNDLE_LOOKUP_HELPER_AVAILABLE",
        );
        if !is_debug {
            push("MAKE_MERGEABLE", "YES");
            push("STRIP_INSTALLED_PRODUCT", "NO");
        }
    }
    // Test bundles get the swift-testing macro plugin path appended to
    // OTHER_SWIFT_FLAGS by xcodebuild (16+; Xcode 15.x predates the
    // swift-testing toolchain plugin and its captures carry no such flag)
    // regardless of user value. The
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
            _ if legacy_xcode15 => None,
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
        // XCTest bundles (unit and UI) link against the platform's bundled
        // libraries: xcodebuild appends `$(inherited)
        // $(TEST_LIBRARY_SEARCH_PATHS)` to the bundle's resolved
        // LIBRARY_SEARCH_PATHS on every captured Xcode version
        // (`TEST_LIBRARY_SEARCH_PATHS` itself is the SDK's
        // `DefaultProperties` value ` $(PLATFORM_DIR)/Developer/usr/lib`,
        // which arrives through the catalog layer). It applies ABOVE the
        // user layers — tuist's ATests authors `LIBRARY_SEARCH_PATHS =
        // $(inherited) <path>` and its capture reports the user path FIRST,
        // then the platform path — so it belongs in this override layer,
        // where `$(inherited)` folds in the user-resolved value.
        if is_test_bundle_product_type(product_type) {
            push(
                "LIBRARY_SEARCH_PATHS",
                "$(inherited) $(TEST_LIBRARY_SEARCH_PATHS)",
            );
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
    // `macosx` appended verbatim to its resolved `SUPPORTED_PLATFORMS` —
    // the user-authored list when there is one, the SDK default otherwise.
    // The append does NOT deduplicate: an unauthored Catalyst extension on
    // a macOS destination resolves the `macosx` SDK default and ends up
    // with the literal `macosx macosx` (ice-cubes' widget extension, both
    // configs). Applies to extensions too — it gates on the Catalyst flag
    // and the macOS destination, not the product type. Xcode 15.x didn't do
    // this append (Kingfisher-Demo's 15.4 `__macOS` captures keep the
    // authored list untouched), so it's gated to 16+.
    if supports_maccatalyst
        && !legacy_xcode15
        && destination.is_some_and(|d| canonicalize_sdk_base(&d.platform) == "macosx")
    {
        let base = user_supported_platforms.map_or_else(
            || supported_platforms_for(sdk_base, legacy_xcode15),
            str::to_owned,
        );
        push("SUPPORTED_PLATFORMS", &format!("{base} macosx"));
    }
    // The macOS SDK's Catalyst variant (`SDKSettings.plist`, `Name = iosmac`)
    // defines `LIBRARY_SEARCH_PATHS = $(inherited)
    // $(SDKROOT)$(IOS_UNZIPPERED_TWIN_PREFIX_PATH)/usr/lib
    // $(TOOLCHAIN_DIR)/usr/lib/swift/maccatalyst` — the identical recipe on
    // every captured Xcode (15.4 / 16.4 / 26.5). Our resolver doesn't ingest
    // SDK Variants, and xcodebuild's `-showBuildSettings` emits the pair
    // TWICE — the same doubled emission the SYSTEM_FRAMEWORK_SEARCH_PATHS /
    // SYSTEM_HEADER_SEARCH_PATHS built-ins mirror. Every captured Catalyst
    // value (28/28 across 16.4 + 26.5) is `<resolved>  <pair>  <pair>`: the
    // resolved lower-stack value keeps its trailing space, hence the double
    // space before each copy. 15.4 never surfaces the key for Catalyst
    // builds, so the push is unscored there.
    if is_catalyst {
        let pair =
            "$(SDKROOT)/System/iOSSupport/usr/lib $(TOOLCHAIN_DIR)/usr/lib/swift/maccatalyst";
        push(
            "LIBRARY_SEARCH_PATHS",
            &format!("$(inherited) {pair}  {pair}"),
        );
    }
    if is_catalyst && let Some(user_target) = user_iphoneos_deployment_target {
        let ios_effective = apply_catalyst_ios_floor(user_target);
        // The Catalyst floor applies to the named iOS deployment target
        // itself, not just the derived trio: xcodebuild reports 13.1 for
        // targets authoring anything lower (alamofire's 10.0, kingfisher's
        // 13.0). Xcode 15.x passed the authored value through untouched.
        if !legacy_xcode15 {
            push("IPHONEOS_DEPLOYMENT_TARGET", &ios_effective);
        }
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
    // The debug dylib can't be signed with the hardened runtime: for a
    // Catalyst-capable target built on a simulator platform, xcodebuild
    // forces `ENABLE_HARDENED_RUNTIME = NO` whenever `ENABLE_DEBUG_DYLIB`
    // resolves YES, overriding an authored YES (ice-cubes, SUPPORTS_
    // MACCATALYST=YES on every target: its simulator captures report NO
    // exactly where the debug dylib is active — every Debug build — and
    // YES where it isn't: the app's Release builds and every Catalyst/macOS
    // build). Targets WITHOUT Catalyst support keep their authored value:
    // NetNewsWire's iOS extensions author an unconditional YES and report
    // YES on every simulator capture, debug dylib active or not — for them
    // the hardened runtime can never apply anyway, so xcodebuild passes the
    // setting through untouched.
    if !legacy_xcode15
        && supports_maccatalyst
        && sdk_base.ends_with("simulator")
        && enable_debug_dylib_default(product_type, is_debug, user_authored_enable_previews)
            == "YES"
    {
        push("ENABLE_HARDENED_RUNTIME", "NO");
    }
    // A scheme whose `TestAction` has `codeCoverageEnabled="YES"` forces
    // `CLANG_COVERAGE_MAPPING=YES` on every target it resolves, overriding the
    // xcspec default of NO. This is a scheme-level fact, not a per-target
    // setting, so it can only arrive via the query (see ResolveQuery).
    if code_coverage_enabled {
        push("CLANG_COVERAGE_MAPPING", "YES");
    }
    // A scheme whose `LaunchAction` enables a sanitizer forces the matching
    // `ENABLE_*_SANITIZER = YES` on every target it resolves, overriding any
    // authored value (the corpus pins both directions: Alamofire's `iOS
    // Example` LaunchAction TSan toggle reports YES on every capture, while
    // `Alamofire watchOS` puts the same toggle on its TestAction and stays
    // NO). Scheme-level facts arriving via the query, like coverage above.
    if scheme_sanitizers.address {
        push("ENABLE_ADDRESS_SANITIZER", "YES");
    }
    if scheme_sanitizers.thread {
        push("ENABLE_THREAD_SANITIZER", "YES");
    }
    if scheme_sanitizers.undefined_behavior {
        push("ENABLE_UNDEFINED_BEHAVIOR_SANITIZER", "YES");
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
    } else if (is_test_bundle_product_type(product_type)
        || product_type == Some("com.apple.product-type.tool"))
        && canonicalize_sdk_base(sdk_base) == "macosx"
        && user_code_sign_identity.is_none_or(str::is_empty)
        && user_development_team.is_none_or(str::is_empty)
    {
        // A macOS unit/UI-test bundle or command-line tool with no signing
        // team and no authored identity signs ad-hoc ("Sign to Run Locally"),
        // which xcodebuild reports as CODE_SIGN_IDENTITY="-" even though
        // CODE_SIGNING_REQUIRED stays YES for these product types (so the
        // branch above doesn't fire). Our per-SDK default would otherwise
        // surface the macOS SDKSettings literal "Apple Development". Scoped
        // to macOS test bundles and tools — the product types the corpus
        // proves this for (the synthetic xcconfig/custom-config Scratch tools
        // report "-" in every capture) — and gated on no-team/no-identity so
        // a team-set target keeps its resolved value. (macOS *apps* are
        // deliberately left to the SDK default; see
        // `code_sign_identity_forced_dash_when_signing_not_required`.)
        push("CODE_SIGN_IDENTITY", "-");
    }
    // Mac Catalyst targets that opt into
    // `DERIVE_MACCATALYST_PRODUCT_BUNDLE_IDENTIFIER=YES` (xcspec default NO,
    // iOSDevice.xcspec) get `maccatalyst.` prepended to their resolved
    // `PRODUCT_BUNDLE_IDENTIFIER`. Kingfisher-Demo authors the flag and its
    // macOS captures show `maccatalyst.com.onevcat.Kingfisher-Demo`; the
    // Catalyst IceCubesApp leaves the flag at its NO default and keeps the
    // bare id. The prefix is pushed onto `$(inherited)`, which folds in the
    // full lower-stack value at merge time — so an id that is itself a
    // `$(...)` recipe resolves against every layer below, not just the user
    // layers the probe pre-resolved. `user_product_bundle_identifier` (the
    // probe's expanded view) gates on an id existing at all and guards
    // against double-prefixing when the user already wrote the prefix.
    if is_catalyst
        && derive_maccatalyst_bundle_id
        && let Some(id) = user_product_bundle_identifier
        && !id.starts_with("maccatalyst.")
    {
        push("PRODUCT_BUNDLE_IDENTIFIER", "maccatalyst.$(inherited)");
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
    /// The Darwin per-user cache dir (`confstr(_CS_DARWIN_USER_CACHE_DIR)`,
    /// e.g. `/var/folders/<x>/<id>/C/`) that anchors `CCHROOT` /
    /// `CACHE_ROOT`. Host state like `home` — the corpus oracles pin the
    /// capture host's value.
    pub darwin_user_cache: Option<String>,
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

/// Collapse the `.xcodeproj/project.xcworkspace` stub Xcode auto-generates
/// inside every project bundle down to its containing `.xcodeproj`.
///
/// A caller can declare that stub as the DerivedData container — e.g. a
/// `xcodeWorkspacePath` pointed straight at `Foo.xcodeproj/project.xcworkspace`.
/// Xcode never keys DerivedData by the stub: opening such a project hashes the
/// `.xcodeproj` itself, producing `Foo-<hash>`. Hashing the stub instead yields
/// the wrong folder name (`project-<hash>`) AND the wrong hash, so the built
/// app can't be found (issue #285). `find_derived_data_container` already skips
/// the stub during inference; this normalizes the explicit case to match.
#[must_use]
fn normalize_stub_workspace(container: &Path) -> PathBuf {
    let is_stub = container.file_name().and_then(OsStr::to_str) == Some("project.xcworkspace")
        && container
            .parent()
            .and_then(Path::extension)
            .and_then(OsStr::to_str)
            == Some("xcodeproj");
    if is_stub && let Some(parent) = container.parent() {
        return parent.to_path_buf();
    }
    container.to_path_buf()
}

/// Return the path Xcode would hash for the DerivedData folder name: a
/// standalone `.xcworkspace` this `.xcodeproj` is a *member* of (one sitting
/// next to it or one directory above), else the `.xcodeproj` itself.
///
/// The `.xcodeproj`'s own embedded `project.xcworkspace` (Xcode's auto-
/// generated stub) is skipped — only USER-authored workspaces count. A
/// workspace that merely sits in a parent directory without referencing this
/// project is **not** adopted: Xcode keys DerivedData by such a workspace only
/// when the project actually belongs to it. (A bare `.xcodeproj` nested under
/// an unrelated project's `.xcworkspace` must hash by itself, or the resolved
/// build path points into the wrong DerivedData folder.)
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
        if let Some(ws) = workspaces
            .into_iter()
            .find(|ws| workspace_contains_project(ws, xcodeproj))
        {
            return ws;
        }
    }
    xcodeproj.to_path_buf()
}

/// Whether `workspace` lists `xcodeproj` among its `FileRef`s. Gates
/// DerivedData-container adoption on real membership rather than mere directory
/// proximity; a workspace that fails to parse counts as "does not contain".
fn workspace_contains_project(workspace: &Path, xcodeproj: &Path) -> bool {
    crate::workspace::open(workspace).is_ok_and(|ws| {
        ws.project_refs
            .iter()
            .any(|member| paths_equivalent(member, xcodeproj))
    })
}

/// Whether two paths point at the same location: by `fs::canonicalize` when
/// both exist (handles symlinks and `/tmp` aliasing), else by lexical
/// [`absolutize`].
fn paths_equivalent(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => absolutize(a) == absolutize(b),
    }
}

pub(crate) fn canonicalize_sdk_base(sdk: &str) -> String {
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

/// True when the product type identifies an app extension that embeds into a
/// host application — the classic `app-extension` family (including its
/// subtypes like Messages stickers) plus the ExtensionKit, PlugInKit, and
/// WatchKit-2 extension types.
#[must_use]
pub fn is_app_extension_family_product_type(product_type: Option<&str>) -> bool {
    match product_type {
        Some(
            "com.apple.product-type.extensionkit-extension"
            | "com.apple.product-type.pluginkit-plugin"
            | "com.apple.product-type.watchkit2-extension",
        ) => true,
        Some(pt) => pt.starts_with("com.apple.product-type.app-extension"),
        None => false,
    }
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
/// One refinement over the spec: for `application` (and `watchkit2-extension`)
/// the optimized-build `NO` flip only happens when the target *authors*
/// `ENABLE_PREVIEWS` somewhere in its layers — opting into the previews
/// machinery hands the debug dylib to the same unoptimized-build gate that
/// drives `ENABLE_PREVIEWS` itself. Targets that never mention
/// `ENABLE_PREVIEWS` keep the spec `YES` in every configuration. This is the
/// input that cleanly separates what previously looked like an opaque
/// heuristic: ice-cubes / tuist apps author `ENABLE_PREVIEWS = YES` and
/// report `NO` in Release; alamofire / kingfisher / netnewswire apps author
/// none and report `YES`; alamofire's watch project authors it and its watch
/// extension flips, while kingfisher/tuist watch extensions don't and stay
/// `YES`. The non-watch app-extension family keeps `YES` in Release across
/// the entire corpus even when an authored `ENABLE_PREVIEWS` arrives via the
/// project level (tuist's AppExtension), so it stays unconditional.
///
/// Product types that don't emit `ENABLE_DEBUG_DYLIB` at all (frameworks,
/// libraries, tools, test bundles) fall through to `NO`; the value is never
/// compared for them.
fn enable_debug_dylib_default(
    product_type: Option<&str>,
    is_debug: bool,
    authored_enable_previews: bool,
) -> &'static str {
    match product_type {
        // Stub-binary product types: incompatible with the dylib wrapper.
        Some(
            "com.apple.product-type.application.messages"
            | "com.apple.product-type.application.watchapp2"
            | "com.apple.product-type.application.watchapp2-container"
            | "com.apple.product-type.app-extension.messages-sticker-pack",
        ) => "NO",
        // App-extension family inherits YES and keeps it in every config —
        // even when an authored ENABLE_PREVIEWS arrives via the project
        // level (tuist's AppExtension reports YES in Release). WatchKit
        // extensions are the exception: they follow the same authored-
        // previews gate as applications below (alamofire's watch project
        // authors ENABLE_PREVIEWS and its extension reports NO in Release;
        // kingfisher/tuist author none and stay YES across the corpus).
        Some("com.apple.product-type.extensionkit-extension") => "YES",
        Some("com.apple.product-type.watchkit2-extension") => {
            if is_debug || !authored_enable_previews {
                "YES"
            } else {
                "NO"
            }
        }
        Some(pt) if pt.starts_with("com.apple.product-type.app-extension") => "YES",
        // Plain applications: the spec default YES survives into every
        // configuration *unless* the target authors `ENABLE_PREVIEWS` —
        // opting into the previews machinery hands the debug dylib to the
        // same unoptimized-build gate that drives ENABLE_PREVIEWS itself
        // (YES in Debug, NO in optimized builds). The corpus separates the
        // two cleanly: alamofire/kingfisher/netnewswire apps (no authored
        // ENABLE_PREVIEWS) report YES in Release, ice-cubes/tuist apps
        // (authored ENABLE_PREVIEWS = YES) report NO.
        Some("com.apple.product-type.application") => {
            if is_debug || !authored_enable_previews {
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
///
/// Pair ordering is Xcode-version dependent: 16+ lists the device first for
/// every family, while 15.4's captures report the *simulator* first for the
/// iPhone and Watch families (`iphonesimulator iphoneos`, `watchsimulator
/// watchos`) yet keep the tvOS pair device-first (`appletvos
/// appletvsimulator` — Kingfisher's unauthored tvOS demo). No 15.x visionOS
/// capture exists, so that family keeps the modern order.
fn supported_platforms_for(sdk_base: &str, legacy_xcode15: bool) -> String {
    match sdk_base {
        "macosx" => "macosx".into(),
        "iphoneos" | "iphonesimulator" if legacy_xcode15 => "iphonesimulator iphoneos".into(),
        "watchos" | "watchsimulator" if legacy_xcode15 => "watchsimulator watchos".into(),
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
    if let Some(pinned) = host_override(|o| o.darwin_user_cache.as_ref()) {
        return ensure_trailing_slash(&pinned);
    }
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

/// The marketing form of a corpus-normalized Xcode version: the corpus dirs
/// and `meta.json` record three components (`15.4.0`), while Xcode's own
/// `CFBundleShortVersionString` — what `CCHROOT`'s `<short>-<build>` segment
/// embeds — drops a zero patch (`15.4`, `16.4`, `26.5`; a non-zero patch is
/// kept: `26.0.1`). Verified against every captured `CCHROOT`.
fn xcode_marketing_version(version: &str) -> String {
    version.strip_suffix(".0").map_or_else(
        || version.to_string(),
        |trimmed| {
            if trimmed.contains('.') {
                trimmed.to_string()
            } else {
                version.to_string()
            }
        },
    )
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

/// The leading major component of an Xcode version string (`"15.4.0"` → 15).
/// `None` when it isn't numeric (including the empty string from a host with
/// no Xcode).
fn xcode_major(version: &str) -> Option<u32> {
    version.split('.').next()?.trim().parse().ok()
}

/// The major version of the Xcode a resolution mirrors: the catalog's
/// recorded version when one is attached, the active install's otherwise.
/// `0` when no version can be determined (callers treat that as modern).
#[must_use]
pub fn effective_xcode_major(catalog_xcode_version: Option<&str>) -> u32 {
    let short = catalog_xcode_version.map_or_else(
        || crate::xcode::active_install().short_version,
        str::to_owned,
    );
    xcode_major(&short).unwrap_or(0)
}

/// Whether the authored settings resolve `GCC_OPTIMIZATION_LEVEL = 0` —
/// xcodebuild's effective "debug build" gate (see [`built_in_overrides`]).
/// `authored` is the [`effective_authored_settings`] pre-resolve, so
/// `[config=…]` / `[sdk=…]` conditionals (matched against the same canonical
/// bindings as the main resolve), `$(inherited)` chains, and `$(VAR)`
/// indirection are all folded before the comparison. An unauthored level
/// means the optimized xcspec default (`s`) applies.
#[must_use]
pub fn is_unoptimized_build(authored: &BTreeMap<String, String>) -> bool {
    authored
        .get("GCC_OPTIMIZATION_LEVEL")
        .is_some_and(|v| v.trim() == "0")
}

fn find_target<'a>(
    objects: &'a Dict,
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
fn test_target_id_host(objects: &Dict, project_obj: &Value, target_name: &str) -> Option<String> {
    // TargetAttributes is keyed by the test target's UUID, so find it first.
    let target_ids = project_obj.get("targets").and_then(Value::as_array)?;
    let test_id = target_ids.iter().filter_map(Value::as_str).find(|id| {
        objects
            .get(id)
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
    let host = objects.get(host_id)?;
    // Only an application can host a test bundle. This also breaks the
    // recursion a malformed edge would otherwise set up: a `TestTargetID`
    // pointing at the test bundle itself (or another test bundle pointing
    // back) would recurse resolve → test_bundle_subpath → resolve forever.
    let is_app = host
        .get("productType")
        .and_then(Value::as_str)
        .is_some_and(|pt| pt.starts_with("com.apple.product-type.application"));
    if !is_app {
        return None;
    }
    host.get("name").and_then(Value::as_str).map(String::from)
}

/// Walk a target's `dependencies` (each a `PBXTargetDependency` pointing at a
/// `target`) and return the `name` of the first dependency whose `productType`
/// is an application. That target is the XCTest host: xcodebuild reads its
/// product wrapper to compute the test bundle's `TEST_HOST` /
/// `TARGET_BUILD_SUBPATH`. A library test bundle whose only dependency is a
/// framework/library returns `None` (it has no host app, so no subpath).
fn find_app_host_target(objects: &Dict, target_obj: &Value) -> Option<String> {
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
    objects: &'a Dict,
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
    objects: &Dict,
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
    objects: &Dict,
    anchor_id: &str,
    relative_path: &str,
    xcodeproj_path: &Path,
) -> PathBuf {
    let project_dir = xcodeproj_path.parent().unwrap_or_else(|| Path::new("."));
    group_dir(objects, anchor_id, project_dir, 0).join(relative_path)
}

fn resolve_file_ref_path(
    objects: &Dict,
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
fn parent_group_dir(objects: &Dict, child_id: &str, project_dir: &Path, depth: usize) -> PathBuf {
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
fn group_dir(objects: &Dict, group_id: &str, project_dir: &Path, depth: usize) -> PathBuf {
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
fn parent_group_of(objects: &Dict, child_id: &str) -> Option<String> {
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

    /// Shorthand for the [`effective_authored_settings`]-shaped map the
    /// built-in/override gates read.
    fn authored_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

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

        // Plain apps that author ENABLE_PREVIEWS: Debug→YES / Release→NO
        // (ice-cubes / tuist). Without authored previews the spec default
        // YES survives every configuration (alamofire / kingfisher /
        // netnewswire apps).
        assert_eq!(enable_debug_dylib_default(app, true, true), "YES");
        assert_eq!(enable_debug_dylib_default(app, false, true), "NO");
        assert_eq!(enable_debug_dylib_default(app, true, false), "YES");
        assert_eq!(enable_debug_dylib_default(app, false, false), "YES");
        // App-extension family: YES in every configuration, regardless of
        // an (inherited) authored ENABLE_PREVIEWS (tuist's AppExtension).
        for ext in [ext, widget, imsg_ext] {
            for previews in [true, false] {
                assert_eq!(enable_debug_dylib_default(ext, true, previews), "YES");
                assert_eq!(enable_debug_dylib_default(ext, false, previews), "YES");
            }
        }
        // WatchKit extensions follow the application gate: authored
        // ENABLE_PREVIEWS flips optimized builds to NO (alamofire's watch
        // project), unauthored stays YES (kingfisher / tuist watch demos).
        assert_eq!(enable_debug_dylib_default(watch_ext, true, true), "YES");
        assert_eq!(enable_debug_dylib_default(watch_ext, false, true), "NO");
        assert_eq!(enable_debug_dylib_default(watch_ext, true, false), "YES");
        assert_eq!(enable_debug_dylib_default(watch_ext, false, false), "YES");
        // Stub-binary product types: always NO.
        for stub in [sticker, watch_container] {
            assert_eq!(enable_debug_dylib_default(stub, true, false), "NO");
            assert_eq!(enable_debug_dylib_default(stub, false, false), "NO");
        }
        // Types that don't emit the setting fall through to NO.
        assert_eq!(enable_debug_dylib_default(framework, true, false), "NO");
        assert_eq!(enable_debug_dylib_default(None, true, false), "NO");
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
        assert!(!detect_catalyst("macosx", Some("iphoneos"), Some("NO")));
        assert!(!detect_catalyst("macosx", Some("iphoneos"), Some("no")));
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
            26,
            false,
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
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
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
            26,
            true,
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
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
        );
        assert_eq!(find(&kf, "ALLOW_TARGET_PLATFORM_SPECIALIZATION"), None);
        assert_eq!(
            find(&kf, "SUPPORTED_PLATFORMS").as_deref(),
            Some("iphoneos iphonesimulator xros xrsimulator macosx")
        );

        // Extension with package products + Catalyst → no ATPS (not an app),
        // but still gets +macosx on macOS.
        let extension = built_in_overrides(
            26,
            false,
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
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
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
            26,
            false,
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
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
        );
        assert_eq!(find(&plain, "ALLOW_TARGET_PLATFORM_SPECIALIZATION"), None);
        assert_eq!(find(&plain, "SUPPORTED_PLATFORMS"), None);

        // The append is verbatim — a list already containing macosx gets a
        // duplicate token, matching xcodebuild (ice-cubes' unauthored widget
        // extension resolves the `macosx` SDK default and its captures
        // report the literal `macosx macosx`).
        let already = built_in_overrides(
            26,
            false,
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
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
        );
        assert_eq!(
            find(&already, "SUPPORTED_PLATFORMS").as_deref(),
            Some("iphoneos macosx macosx")
        );

        // Unauthored SUPPORTED_PLATFORMS on a Catalyst macOS build: the SDK
        // default (`macosx`) is the append base → `macosx macosx`.
        let unauthored = built_in_overrides(
            26,
            false,
            true,
            true,
            None,
            None,
            ext,
            "macosx",
            Some(&macos_dest),
            false,
            false,
            true,
            false,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
        );
        assert_eq!(
            find(&unauthored, "SUPPORTED_PLATFORMS").as_deref(),
            Some("macosx macosx")
        );
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
            26,
            true,
            false,
            false,
            None,
            None,
            framework,
            "macosx",
            None,
            false,
            false,
            false,
            false,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
        );
        assert_eq!(find(&unsigned, "CODE_SIGN_IDENTITY").as_deref(), Some("-"));
        let signed = built_in_overrides(
            26,
            true,
            false,
            false,
            None,
            None,
            app,
            "macosx",
            None,
            false,
            false,
            true,
            false,
            None,
            None,
            None,
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
        );
        assert_eq!(find(&signed, "CODE_SIGN_IDENTITY"), None);
    }

    #[test]
    #[allow(clippy::too_many_lines)] // the extra built_in_overrides args inflate each call site
    fn maccatalyst_bundle_id_prefix_gated_on_derive_flag() {
        let find = |out: &[Assignment], key: &str| -> Option<String> {
            out.iter().find(|a| a.key == key).map(|a| a.value.clone())
        };
        let app = Some("com.apple.product-type.application");

        // Catalyst + DERIVE flag YES → prepend `maccatalyst.` onto
        // `$(inherited)`, which folds in the full lower-stack id at merge
        // time (so even a `$(...)` recipe resolves against every layer).
        let derived = built_in_overrides(
            26,
            true,
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
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
        );
        assert_eq!(
            find(&derived, "PRODUCT_BUNDLE_IDENTIFIER").as_deref(),
            Some("maccatalyst.$(inherited)")
        );

        // Catalyst but DERIVE flag at its NO default (IceCubesApp) → no prefix.
        let not_derived = built_in_overrides(
            26,
            true,
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
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
        );
        assert_eq!(find(&not_derived, "PRODUCT_BUNDLE_IDENTIFIER"), None);

        // Not Catalyst → no prefix even with the flag set.
        let non_catalyst = built_in_overrides(
            26,
            true,
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
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
        );
        assert_eq!(find(&non_catalyst, "PRODUCT_BUNDLE_IDENTIFIER"), None);

        // Already prefixed → not doubled.
        let already = built_in_overrides(
            26,
            true,
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
            false,
            false,
            None,
            None,
            false,
            false,
            crate::scheme::SanitizerEnables::default(),
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
            &authored_map(&[("SDKROOT", "auto")]),
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            crate::scheme::SanitizerEnables::default(),
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
            &authored_map(&[("SDKROOT", "macosx")]),
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            crate::scheme::SanitizerEnables::default(),
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
            &BTreeMap::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            crate::scheme::SanitizerEnables::default(),
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
        // The effective ONLY_ACTIVE_ARCH default tracks the unoptimized-build
        // flag (`GCC_OPTIMIZATION_LEVEL = 0`, the Debug template's value),
        // NOT the configuration name — the custom-config fixture's
        // template-less `Debug` reports ONLY_ACTIVE_ARCH=NO and a full ARCHS.
        let debug_template = authored_map(&[("GCC_OPTIMIZATION_LEVEL", "0")]);
        let dest = RunDestination {
            platform: "iphoneos".into(),
            os_version: String::new(),
            device_name: String::new(),
            arch: "arm64".into(),
        };
        let out = built_in_settings(
            Path::new("/tmp/App.xcodeproj"),
            "App",
            "Debug",
            Some("com.apple.product-type.application"),
            "iphoneos",
            Some(&dest),
            false,
            false,
            None,
            None,
            &debug_template,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            crate::scheme::SanitizerEnables::default(),
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
            &debug_template,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            crate::scheme::SanitizerEnables::default(),
        );
        let get = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(get("ARCHS").map(String::from), Some(host_arch()));

        // A config that authors NO optimization level — however it's named —
        // is an optimized build: no collapse even on a simulator SDK
        // (xcodebuild's custom-config captures report the full pair).
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
            &BTreeMap::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            false,
            crate::scheme::SanitizerEnables::default(),
        );
        let get = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(get("ARCHS"), Some("arm64 x86_64"));
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

    // ----- the shared authored-value pre-resolve (the gating probe) ---------

    fn probe_ctx(sdk: &str, config: &str) -> crate::resolver::ResolveContext {
        crate::resolver::ResolveContext {
            sdk: sdk.into(),
            arch: "undefined_arch".into(),
            configuration: config.into(),
            variant: "normal".into(),
        }
    }

    fn assignment(key: &str, conds: &[(&str, &str)], value: &str) -> Assignment {
        Assignment {
            key: key.into(),
            conditions: conds
                .iter()
                .map(|(k, v)| Condition {
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            value: value.into(),
            condition: None,
        }
    }

    /// A conditional assignment whose condition matches the query bindings is
    /// visible to the gates — `GCC_OPTIMIZATION_LEVEL[config=Debug] = 0` is a
    /// debug build under Debug and an optimized one under Release.
    #[test]
    fn unoptimized_gate_sees_matching_conditional_assignments() {
        let layers = vec![vec![assignment(
            "GCC_OPTIMIZATION_LEVEL",
            &[("config", "Debug")],
            "0",
        )]];
        let debug = effective_authored_settings(&layers, &probe_ctx("macosx26.0", "Debug"));
        assert!(is_unoptimized_build(&debug));
        let release = effective_authored_settings(&layers, &probe_ctx("macosx26.0", "Release"));
        assert!(!is_unoptimized_build(&release));
    }

    /// `[sdk=macosx26*]` matches the canonical (versioned) SDK name the
    /// resolve binds — the same binding the main pass uses, so the gate and
    /// the reported `GCC_OPTIMIZATION_LEVEL` can never disagree.
    #[test]
    fn unoptimized_gate_matches_the_versioned_sdk_binding() {
        let layers = vec![vec![assignment(
            "GCC_OPTIMIZATION_LEVEL",
            &[("sdk", "macosx26*")],
            "0",
        )]];
        let authored = effective_authored_settings(&layers, &probe_ctx("macosx26.0", "Debug"));
        assert!(is_unoptimized_build(&authored));
        // A different platform's binding leaves the conditional unmatched.
        let ios = effective_authored_settings(&layers, &probe_ctx("iphoneos18.2", "Debug"));
        assert!(!is_unoptimized_build(&ios));
    }

    /// `$(VAR)` indirection and `$(inherited)` chains fold before the gate
    /// compares — `GCC_OPTIMIZATION_LEVEL = $(MY_LEVEL)` with `MY_LEVEL = 0`
    /// is a debug build.
    #[test]
    fn unoptimized_gate_expands_variable_references() {
        let layers = vec![
            vec![
                assignment("MY_LEVEL", &[], "0"),
                assignment("GCC_OPTIMIZATION_LEVEL", &[], "$(MY_LEVEL)"),
            ],
            // An upper layer restating the key via $(inherited) keeps it.
            vec![assignment("GCC_OPTIMIZATION_LEVEL", &[], "$(inherited)")],
        ];
        let authored = effective_authored_settings(&layers, &probe_ctx("macosx26.0", "Debug"));
        assert!(is_unoptimized_build(&authored));
    }

    /// [`last_matching_setting`] honors conditions but keeps the raw recipe —
    /// the ARCHS probe must see `$(ARCHS_STANDARD)` verbatim, not its (empty)
    /// user-layer expansion.
    #[test]
    fn last_matching_setting_keeps_the_raw_recipe() {
        let layers = vec![vec![
            assignment("ARCHS", &[], "$(ARCHS_STANDARD)"),
            assignment("ARCHS", &[("sdk", "iphoneos*")], "arm64e"),
        ]];
        let ctx = probe_ctx("iphoneos18.2", "Debug");
        assert_eq!(
            last_matching_setting(&layers, "ARCHS", &ctx).as_deref(),
            Some("arm64e")
        );
        let ctx = probe_ctx("macosx26.0", "Debug");
        assert_eq!(
            last_matching_setting(&layers, "ARCHS", &ctx).as_deref(),
            Some("$(ARCHS_STANDARD)")
        );
    }

    /// An unknown destination device name suppresses the asset-catalog filter
    /// settings entirely (a real CLI `name=iPhone 16` destination) — known
    /// labels keep emitting the model triple.
    #[test]
    fn assetcatalog_filters_suppressed_for_unknown_device_model() {
        let resolve_with_device = |device_name: &str| {
            let dest = RunDestination {
                platform: "iphonesimulator".into(),
                os_version: "26.0".into(),
                device_name: device_name.into(),
                arch: "arm64".into(),
            };
            built_in_settings(
                Path::new("/tmp/App.xcodeproj"),
                "App",
                "Debug",
                Some("com.apple.product-type.application"),
                "iphonesimulator",
                Some(&dest),
                false,
                false,
                None,
                None,
                &BTreeMap::new(),
                None,
                None,
                None,
                None,
                None,
                None,
                false,
                crate::scheme::SanitizerEnables::default(),
            )
        };
        let known = resolve_with_device("iPad-A16");
        assert_eq!(
            known
                .iter()
                .find(|a| a.key == "ASSETCATALOG_FILTER_FOR_DEVICE_MODEL")
                .map(|a| a.value.as_str()),
            Some("iPad15,7")
        );
        let unknown = resolve_with_device("iPhone 16");
        assert!(
            unknown
                .iter()
                .all(|a| !a.key.starts_with("ASSETCATALOG_FILTER_FOR_")),
            "an unknown device model must suppress the filter settings"
        );
    }

    /// `SWIFT_EMIT_CONST_VALUE_PROTOCOLS` is a 26+-only key — 16.x/15.x
    /// captures never carry it.
    #[test]
    fn swift_emit_const_value_protocols_gated_to_xcode26() {
        let resolve_with_version = |version: &str| {
            built_in_settings(
                Path::new("/tmp/App.xcodeproj"),
                "App",
                "Debug",
                Some("com.apple.product-type.application"),
                "macosx",
                None,
                false,
                false,
                None,
                None,
                &BTreeMap::new(),
                None,
                None,
                Some(version),
                None,
                None,
                None,
                false,
                crate::scheme::SanitizerEnables::default(),
            )
        };
        let has_key = |out: &[Assignment]| {
            out.iter()
                .any(|a| a.key == "SWIFT_EMIT_CONST_VALUE_PROTOCOLS")
        };
        assert!(has_key(&resolve_with_version("26.5")));
        assert!(!has_key(&resolve_with_version("16.4")));
        assert!(!has_key(&resolve_with_version("15.4")));
    }

    /// [`absolutize`] anchors relative paths and collapses dot segments but
    /// must NOT resolve symlinks — Xcode hashes the DerivedData container
    /// path as opened.
    #[test]
    fn absolutize_keeps_symlinks_and_collapses_lexically() {
        assert_eq!(
            absolutize(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
        assert!(
            absolutize(Path::new("some/relative")).is_absolute(),
            "relative input must come back absolute"
        );

        // A symlinked directory keeps its symlink spelling.
        let root = std::env::temp_dir().join(format!("sweetpad-abs-{}", std::process::id()));
        let target_dir = root.join("target");
        let link = root.join("link");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&target_dir).unwrap();
        std::os::unix::fs::symlink(&target_dir, &link).unwrap();
        let through_link = link.join("Proj.xcodeproj");
        assert_eq!(absolutize(&through_link), through_link);
        assert_ne!(
            absolutize(&through_link),
            target_dir.join("Proj.xcodeproj"),
            "must not resolve the symlink"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// `normalize_stub_workspace` is pure-lexical (no filesystem), so pin it
    /// with a table. It must collapse ONLY the `.xcodeproj/project.xcworkspace`
    /// stub down to its bundle; everything else passes through untouched.
    #[test]
    fn normalize_stub_workspace_collapses_only_the_bundle_stub() {
        let cases: &[(&str, &str)] = &[
            // The auto-generated stub collapses to its containing bundle…
            (
                "/root/Foo.xcodeproj/project.xcworkspace",
                "/root/Foo.xcodeproj",
            ),
            // …even spelled with a trailing slash (Path ignores it).
            (
                "/root/Foo.xcodeproj/project.xcworkspace/",
                "/root/Foo.xcodeproj",
            ),
            // A real, user-authored workspace is left untouched.
            ("/root/Foo.xcworkspace", "/root/Foo.xcworkspace"),
            // A `project.xcworkspace` NOT inside an `.xcodeproj` is not the
            // stub — don't eat a real directory that merely shares the name.
            (
                "/root/weird/project.xcworkspace",
                "/root/weird/project.xcworkspace",
            ),
            // A bare `.xcodeproj` is already the container.
            ("/root/Foo.xcodeproj", "/root/Foo.xcodeproj"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                normalize_stub_workspace(Path::new(input)),
                PathBuf::from(expected),
                "normalize_stub_workspace({input})"
            );
        }
    }

    /// The container-*inference* matrix (`find_derived_data_container`), which
    /// chooses which path Xcode would hash for the DerivedData folder. Each
    /// shape is built on disk because the function reads the tree — including
    /// each candidate workspace's `contents.xcworkspacedata`, since only a
    /// workspace this project actually belongs to keys its DerivedData.
    #[test]
    fn find_derived_data_container_selects_the_keyed_container() {
        let root = std::env::temp_dir().join(format!("sweetpad-ddc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        // Write a `.xcworkspace` whose `contents.xcworkspacedata` lists `refs`
        // (each a `group:`-relative `.xcodeproj`).
        let mk_ws = |path: &Path, refs: &[&str]| {
            fs::create_dir_all(path).unwrap();
            let body: String = refs
                .iter()
                .map(|r| format!("  <FileRef location = \"group:{r}\"></FileRef>\n"))
                .collect();
            fs::write(
                path.join("contents.xcworkspacedata"),
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Workspace version=\"1.0\">\n{body}</Workspace>\n"
                ),
            )
            .unwrap();
        };

        // (a) Bare project, no workspace anywhere -> the `.xcodeproj` itself.
        let a = root.join("a");
        fs::create_dir_all(a.join("Foo.xcodeproj")).unwrap();
        assert_eq!(
            find_derived_data_container(&a.join("Foo.xcodeproj")),
            a.join("Foo.xcodeproj"),
            "no workspace: container is the project"
        );

        // (b) A sibling workspace that lists the project as a member -> the
        //     workspace keys DerivedData (Xcode opens the project through it).
        let b = root.join("b");
        fs::create_dir_all(b.join("Foo.xcodeproj")).unwrap();
        mk_ws(&b.join("App.xcworkspace"), &["Foo.xcodeproj"]);
        assert_eq!(
            find_derived_data_container(&b.join("Foo.xcodeproj")),
            b.join("App.xcworkspace"),
            "member sibling workspace wins over the project"
        );

        // (c) A member workspace one directory ABOVE the project -> the
        //     workspace (the grandparent leg). The folder prefix is the
        //     WORKSPACE stem (`App`), not the project stem (`Foo`).
        let c = root.join("c");
        fs::create_dir_all(c.join("Sub/Foo.xcodeproj")).unwrap();
        mk_ws(&c.join("App.xcworkspace"), &["Sub/Foo.xcodeproj"]);
        assert_eq!(
            find_derived_data_container(&c.join("Sub/Foo.xcodeproj")),
            c.join("App.xcworkspace"),
            "member grandparent workspace wins; name != project name"
        );

        // (d) A sub-project nested inside another `.xcodeproj` bundle: the only
        //     workspace in view is that bundle's auto-generated stub, which the
        //     search skips -> fall back to the sub-project itself.
        let d = root.join("d");
        fs::create_dir_all(d.join("Outer.xcodeproj/Sub.xcodeproj")).unwrap();
        fs::create_dir_all(d.join("Outer.xcodeproj/project.xcworkspace")).unwrap();
        assert_eq!(
            find_derived_data_container(&d.join("Outer.xcodeproj/Sub.xcodeproj")),
            d.join("Outer.xcodeproj/Sub.xcodeproj"),
            "the bundle stub is skipped during inference too"
        );

        // (e) Two member workspaces beside the project: pick the
        //     alphabetically-first. A documented heuristic, not captured Xcode
        //     behaviour — pinned so any change is deliberate (DOCS open item).
        let e = root.join("e");
        fs::create_dir_all(e.join("Foo.xcodeproj")).unwrap();
        mk_ws(&e.join("Beta.xcworkspace"), &["Foo.xcodeproj"]);
        mk_ws(&e.join("Alpha.xcworkspace"), &["Foo.xcodeproj"]);
        assert_eq!(
            find_derived_data_container(&e.join("Foo.xcodeproj")),
            e.join("Alpha.xcworkspace"),
            "two member workspaces: alphabetically-first heuristic"
        );

        // (f) A nearby workspace that does NOT list the project -> the project
        //     keys its own DerivedData. A bare `.xcodeproj` scaffolded beneath
        //     an unrelated project's workspace must not borrow its folder (the
        //     `app run` install-path regression).
        let f = root.join("f");
        fs::create_dir_all(f.join("Sub/Foo.xcodeproj")).unwrap();
        mk_ws(&f.join("Other.xcworkspace"), &["Sub/Bar.xcodeproj"]);
        assert_eq!(
            find_derived_data_container(&f.join("Sub/Foo.xcodeproj")),
            f.join("Sub/Foo.xcodeproj"),
            "non-member workspace is ignored; the project keys itself"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// MD5 is byte-sensitive, so `Foo.xcodeproj` and `Foo.xcodeproj/` would
    /// hash to different DerivedData folders if the trailing slash survived.
    /// `absolutize` normalizes both spellings, keeping the folder stable
    /// regardless of how the caller spelled the container path.
    #[test]
    fn trailing_slash_does_not_change_the_derived_data_hash() {
        let with = absolutize(Path::new("/root/Foo.xcodeproj/"));
        let without = absolutize(Path::new("/root/Foo.xcodeproj"));
        assert_eq!(with, without, "trailing slash must normalize away");
        assert_eq!(
            derived_data_hash(&with.display().to_string()),
            derived_data_hash(&without.display().to_string()),
        );
    }
}
