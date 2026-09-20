//! Building a [`crate::project::Project`] from a `project.xcproj` document.
//!
//! The model is the same one `project.rs` builds from a pbxproj — callers do
//! not learn which format the bundle used — so the work here is mapping the
//! JSON document's spellings onto it:
//!
//! * A target's kind is `native`, `aggregate` or `external-build-system`, with
//!   `native` implied when the key is absent, where the pbxproj spelled these
//!   as `isa`.
//! * A product type is stored abbreviated under `product-type`, with
//!   `com.apple.product-type.` dropped, and in full under `full-product-type`
//!   when it is not one of Apple's.
//! * Configurations belong to the project. A target's
//!   `specialized-configurations` names only the ones whose xcconfig it
//!   overrides, so its configuration list is the project's.
//! * A file's location is one string, with the anchor that `sourceTree` used
//!   to carry written into it as a leading `<PROJECT>`, `<PRODUCTS>`, `<SDK>`
//!   or `<DEVELOPER>` token. No token means the parent group's directory, the
//!   old `<group>`.
//! * One `build-settings` map per scope replaces a map per configuration, with
//!   the configuration moved into the key as `[config=Debug]`. The resolver
//!   already matches that condition, so a scope's map becomes one layer as it
//!   stands and the four-layer order is unchanged.

use std::path::{Path, PathBuf};

use crate::project::{BuildSettingsContext, Error, Project, Target};
use crate::schema_xcproj::{self as schema, Scope};
use crate::xcconfig::{Assignment, Condition};
use crate::xcproj::{self, Value};

/// The prefix Xcode drops from a product type it recognizes.
const APPLE_PRODUCT_TYPE_PREFIX: &str = "com.apple.product-type.";

pub(crate) fn open(xcodeproj_path: &Path) -> Result<Project, Error> {
    let value = parse(xcodeproj_path)?;
    open_from_value(&value, xcodeproj_path)
}

/// The bundle's parsed document, or `None` when it holds a pbxproj instead.
///
/// Lets a pbxproj-shaped entry point take the other path without first
/// stat-ing for the file itself.
pub(crate) fn parse_if_present(
    xcodeproj_path: &Path,
) -> Result<Option<std::sync::Arc<Value>>, Error> {
    if xcodeproj_path.join("project.pbxproj").exists()
        || !xcodeproj_path.join(xcproj::DOCUMENT_NAME).exists()
    {
        return Ok(None);
    }
    parse(xcodeproj_path).map(Some)
}

fn parse(xcodeproj_path: &Path) -> Result<std::sync::Arc<Value>, Error> {
    let document_path = xcodeproj_path.join(xcproj::DOCUMENT_NAME);
    xcproj::parse_file_cached(&document_path).map_err(|e| match e {
        xcproj::Error::Io(e) => Error::Io(e),
        xcproj::Error::Parse(e) => Error::BadProject(format!("{}: {e}", xcproj::DOCUMENT_NAME)),
    })
}

pub(crate) fn open_from_value(value: &Value, xcodeproj_path: &Path) -> Result<Project, Error> {
    if value.as_object().is_none() {
        return Err(Error::BadProject(format!(
            "{} root is not an object",
            xcproj::DOCUMENT_NAME
        )));
    }

    let configurations = schema::configuration_names(value);
    let default_configuration = schema::default_configuration(value).map(str::to_string);
    let targets = extract_targets(value, &configurations);

    let mut schemes = crate::scheme::container_schemes(xcodeproj_path);
    if crate::scheme::autocreation_allowed(xcodeproj_path) {
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
    }
    crate::scheme::sort_like_xcodebuild(&mut schemes);
    schemes.dedup();

    let name = xcodeproj_path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("")
        .to_string();

    Ok(Project {
        name,
        path: xcodeproj_path.to_path_buf(),
        targets,
        configurations,
        default_configuration,
        schemes,
        package_refs: local_package_refs(value, xcodeproj_path),
    })
}

fn extract_targets(value: &Value, configurations: &[String]) -> Vec<Target> {
    value
        .get("targets")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|target| {
            let name = target.get("name").and_then(Value::as_str)?.to_string();
            Some(Target {
                name,
                isa: isa_for(target).to_string(),
                product_type: product_type(target),
                // Every target builds the project's configurations; a target
                // lists one only to point it at a different xcconfig.
                configurations: configurations.to_vec(),
            })
        })
        .collect()
}

/// The `isa` the pbxproj would have used, so downstream rules that match on it
/// keep working whichever format the project is in.
fn isa_for(target: &Value) -> &'static str {
    match target.get("kind").and_then(Value::as_str) {
        Some("aggregate") => "PBXAggregateTarget",
        Some("external-build-system") => "PBXLegacyTarget",
        // `native` is the default and is left out of the document.
        _ => "PBXNativeTarget",
    }
}

fn product_type(target: &Value) -> Option<String> {
    if let Some(abbreviated) = target.get("product-type").and_then(Value::as_str) {
        return Some(format!("{APPLE_PRODUCT_TYPE_PREFIX}{abbreviated}"));
    }
    target
        .get("full-product-type")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Directories of the local Swift packages the project references, keeping
/// only the ones that hold a `Package.swift`.
///
/// Two sources, the same two the pbxproj path reads: the `packages` list, and
/// the file tree, where a package can appear as a folder dragged into a group
/// or as anything under a synchronized folder.
fn local_package_refs(value: &Value, xcodeproj_path: &Path) -> Vec<PathBuf> {
    let project_dir = crate::project::abs_project_dir(xcodeproj_path);
    let declared = value
        .get("packages")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter(|p| p.get("kind").and_then(Value::as_str) == Some("local"))
        .filter_map(|p| p.get("path").and_then(Value::as_str))
        .filter_map(|rel| resolve_path(rel, &project_dir, &project_dir));

    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    for dir in declared.chain(file_tree_package_refs(value, &project_dir)) {
        if dir.join("Package.swift").is_file() && seen.insert(dir.clone()) {
            out.push(dir);
        }
    }
    out
}

/// Package directories reachable from the `files` tree, in navigator order.
fn file_tree_package_refs(value: &Value, project_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_for_packages(
        value
            .get("files")
            .and_then(Value::as_array)
            .unwrap_or_default(),
        project_dir,
        project_dir,
        &mut out,
        0,
    );
    out
}

fn walk_for_packages(
    nodes: &[Value],
    parent_base: &Path,
    project_dir: &Path,
    out: &mut Vec<PathBuf>,
    depth: usize,
) {
    if depth >= crate::project::MAX_GROUP_DEPTH {
        return;
    }
    for node in nodes {
        let Some(base) = node_base(node, parent_base, project_dir) else {
            continue;
        };
        match node.get("kind").and_then(Value::as_str) {
            Some("group" | "variant-group" | "version-group") => walk_for_packages(
                node.get("children")
                    .and_then(Value::as_array)
                    .unwrap_or_default(),
                &base,
                project_dir,
                out,
                depth + 1,
            ),
            // A synchronized folder lists none of its members, so a package
            // under it is found by looking at the disk.
            Some("folder") => {
                if base.join("Package.swift").is_file() {
                    out.push(base);
                } else {
                    crate::project::packages_under_synchronized_folder(&base, out, 0);
                }
            }
            // A leaf that declares no `type` could be a directory, which is
            // how a package dragged into the navigator appears.
            _ => {
                if node.get("type").is_none() {
                    out.push(base);
                }
            }
        }
    }
}

/// Where a node sits on disk: its own path resolved against the parent's, or
/// the parent's directory unchanged for a group that has only a name. `None`
/// for a node anchored somewhere that is not the source tree.
fn node_base(node: &Value, parent_base: &Path, project_dir: &Path) -> Option<PathBuf> {
    match node.get("path").and_then(Value::as_str) {
        None => Some(parent_base.to_path_buf()),
        Some(path) => resolve_path(path, parent_base, project_dir),
    }
}

/// Resolve a stored path, which may open with an anchor token — the `<PROJECT>`,
/// `<PRODUCTS>`, `<SDK>` or `<DEVELOPER>` that `sourceTree` used to carry.
///
/// All but `<PROJECT>` name build-time locations rather than places in the
/// source tree, and resolve to `None`.
fn resolve_path(path: &str, parent_base: &Path, project_dir: &Path) -> Option<PathBuf> {
    if let Some(rest) = path.strip_prefix("<PROJECT>/") {
        return Some(crate::project::join_normalized(project_dir, rest));
    }
    if path.starts_with('<') {
        return None;
    }
    if path.starts_with('/') {
        return Some(PathBuf::from(path));
    }
    Some(crate::project::join_normalized(parent_base, path))
}

/// Xcode's scheme autocreation rules, matching
/// `project::autocreates_scheme_for_target`: test bundles, watch extensions and
/// watch containers get no scheme, an app extension gets one unless it is a
/// Safari extension, and everything else does.
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
    let Some(plist_rel) =
        crate::project::last_unconditional_setting(&bundle.layers, "INFOPLIST_FILE")
    else {
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
    crate::project::element_has_safari_extension_point(&plist)
}

/// The four user-authored build-settings layers for a target + configuration,
/// in the same order and with the same meaning as the pbxproj path's: project
/// xcconfig, project settings, target xcconfig, target settings.
pub(crate) fn build_settings_from_value(
    value: &Value,
    xcodeproj_path: &Path,
    target_name: &str,
    config_name: &str,
) -> Result<BuildSettingsContext, Error> {
    let configurations = schema::configuration_names(value);
    let config = effective_configuration(value, &configurations, config_name)
        .ok_or_else(|| Error::NoConfigurations(config_name.to_string()))?;

    let target =
        find_target(value, target_name).ok_or_else(|| Error::no_such_target(target_name))?;
    let target_scope = Scope::Target(target_name.to_string());

    Ok(BuildSettingsContext {
        layers: vec![
            xcconfig_layer(value, &Scope::Project, config, xcodeproj_path)?,
            settings_layer(value, &Scope::Project, config)?,
            xcconfig_layer(value, &target_scope, config, xcodeproj_path)?,
            settings_layer(value, &target_scope, config)?,
        ],
        product_type: product_type(target),
        target_isa: isa_for(target).to_string(),
        has_package_product_dependencies: dependencies(target)
            .iter()
            .any(|d| d.get("kind").and_then(Value::as_str) == Some("package")),
        // The pbxproj hid this in the root object's `TargetAttributes`; here
        // the test bundle names its host itself.
        test_host_target: target
            .get("test-host-target")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// The configuration a build actually uses: the one asked for, else the
/// project's default, else the first. An unknown name is not fatal — the same
/// fallback the pbxproj path applies, and what xcodebuild does after warning.
fn effective_configuration<'a>(
    value: &Value,
    configurations: &'a [String],
    requested: &str,
) -> Option<&'a str> {
    let by_name = |name: &str| {
        configurations
            .iter()
            .find(|c| *c == name)
            .map(String::as_str)
    };
    by_name(requested)
        .or_else(|| schema::default_configuration(value).and_then(by_name))
        .or_else(|| configurations.first().map(String::as_str))
}

fn find_target<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    value
        .get("targets")
        .and_then(Value::as_array)?
        .iter()
        .find(|t| t.get("name").and_then(Value::as_str) == Some(name))
}

fn dependencies(target: &Value) -> &[Value] {
    target
        .get("dependencies")
        .and_then(Value::as_array)
        .unwrap_or_default()
}

/// A scope's stored settings as one layer, narrowed to the configuration.
///
/// `[config=…]` is dropped from each key it matches and the entry is skipped
/// when it does not, which leaves the layer holding what the pbxproj's
/// per-configuration `buildSettings` dict held: one value per setting, with
/// only the conditions xcodebuild still has to evaluate (`[sdk=…]`,
/// `[arch=…]`) left on it. Settings are stored byte-sorted, so a conditional
/// key follows the plain one and wins, which is how xcodebuild reads them.
fn settings_layer(value: &Value, scope: &Scope, config: &str) -> Result<Vec<Assignment>, Error> {
    let entries = schema::raw(value, scope).map_err(Error::BadProject)?;
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let (key, conditions) = crate::project::split_conditional_key(&entry.key());
        let Some(conditions) = without_matching_config(conditions, config) else {
            continue;
        };
        out.push(Assignment {
            key,
            conditions,
            value: entry.value.display(),
            condition: None,
        });
    }
    Ok(out)
}

/// The conditions minus every `[config=…]` clause, or `None` when one of them
/// names a different configuration.
fn without_matching_config(conditions: Vec<Condition>, config: &str) -> Option<Vec<Condition>> {
    let mut kept = Vec::with_capacity(conditions.len());
    for cond in conditions {
        if matches!(cond.key.as_str(), "config" | "configuration") {
            if !crate::resolver::glob_match(&cond.value, config) {
                return None;
            }
        } else {
            kept.push(cond);
        }
    }
    Some(kept)
}

/// The xcconfig a scope bases a configuration on, flattened.
///
/// A missing file is an empty layer, not an error: xcodebuild warns and
/// resolves as if none were attached, which is what a CocoaPods project before
/// `pod install` relies on.
fn xcconfig_layer(
    value: &Value,
    scope: &Scope,
    config: &str,
    xcodeproj_path: &Path,
) -> Result<Vec<Assignment>, Error> {
    let project_dir = crate::project::abs_project_dir(xcodeproj_path);
    let Some(path) = schema::base_xcconfig(value, scope, config)
        .and_then(|file| xcconfig_path(file, value, &project_dir))
    else {
        return Ok(Vec::new());
    };
    match crate::resolver::flatten_xcconfig(&path) {
        Ok(assignments) => Ok(assignments),
        Err(crate::resolver::Error::Io { path: p, source })
            if source.kind() == std::io::ErrorKind::NotFound && p == path =>
        {
            Ok(Vec::new())
        }
        Err(e) => Err(Error::BadProject(format!(
            "xcconfig {}: {e}",
            path.display()
        ))),
    }
}

/// Where a configuration's `file` points.
///
/// Both forms name a place in the navigator rather than on disk: a path of
/// display names, or an `{ anchor, relative-path }` pair whose anchor is the
/// navigator path of a synchronized folder, which lists no members of its own.
/// The two come apart whenever a file's display name differs from its path —
/// CocoaPods writes `Pods/Pods-App.debug.xcconfig` for a file that lives at
/// `Pods/Target Support Files/Pods-App/Pods-App.debug.xcconfig`.
fn xcconfig_path(file: &Value, root: &Value, project_dir: &Path) -> Option<PathBuf> {
    if let Some(path) = file.as_str() {
        return navigator_path(root, path, project_dir);
    }
    let anchor = file.get("anchor").and_then(Value::as_str)?;
    let relative = file.get("relative-path").and_then(Value::as_str)?;
    let anchor_dir = navigator_path(root, anchor, project_dir)?;
    Some(crate::project::join_normalized(&anchor_dir, relative))
}

/// The location on disk of the node a navigator path names.
///
/// Matched by walking the tree and rebuilding each node's navigator path from
/// its display name, rather than by splitting the input: a group's name can
/// itself hold a `/` (`App/Sources`), so the segments are not separable.
fn navigator_path(root: &Value, navigator: &str, project_dir: &Path) -> Option<PathBuf> {
    fn walk(
        nodes: &[Value],
        parent_nav: &str,
        parent_base: &Path,
        project_dir: &Path,
        target: &str,
        depth: usize,
    ) -> Option<PathBuf> {
        if depth >= crate::project::MAX_GROUP_DEPTH {
            return None;
        }
        for node in nodes {
            let Some(name) = display_name(node) else {
                continue;
            };
            let nav = if parent_nav.is_empty() {
                name.to_string()
            } else {
                format!("{parent_nav}/{name}")
            };
            if !target.starts_with(nav.as_str()) {
                continue;
            }
            let Some(base) = node_base(node, parent_base, project_dir) else {
                continue;
            };
            if nav == target {
                return Some(base);
            }
            let found = walk(
                node.get("children")
                    .and_then(Value::as_array)
                    .unwrap_or_default(),
                &nav,
                &base,
                project_dir,
                target,
                depth + 1,
            );
            if found.is_some() {
                return found;
            }
        }
        None
    }

    walk(
        root.get("files")
            .and_then(Value::as_array)
            .unwrap_or_default(),
        "",
        project_dir,
        project_dir,
        navigator,
        0,
    )
    // A document that names a file the tree does not hold — nothing Xcode
    // writes, but a hand-edited one might — is read as a plain path.
    .or_else(|| resolve_path(navigator, project_dir, project_dir))
}

/// What the navigator shows for a node: its name, or the last component of its
/// path.
fn display_name(node: &Value) -> Option<&str> {
    if let Some(name) = node.get("name").and_then(Value::as_str) {
        return Some(name);
    }
    let path = node.get("path").and_then(Value::as_str)?;
    Some(path.rsplit('/').next().unwrap_or(path))
}
