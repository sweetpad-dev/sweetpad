//! Reading and mutating a project's Swift package dependencies in a parsed
//! [`crate::xcproj::Value`] document — the `project.xcproj` counterpart of
//! [`crate::spm_pbxproj`].
//!
//! The pbxproj's three object kinds become two places. A declared package is
//! an entry in the top-level `packages` list, `{ kind: remote, repository,
//! version }` or `{ kind: local, path }`. A product a target consumes is a
//! reference on the target itself, naming its package by [`package_name`]
//! rather than pointing at an object: an entry in `package-product-members`
//! for a product linked through a build phase, which is the pbxproj's
//! Frameworks `PBXBuildFile`, and a `{ kind: package }` entry in
//! `dependencies` for one the target only depends on, the pbxproj's
//! `PBXTargetDependency` with a `productRef`. `packageProductDependencies` has
//! no counterpart; the references are the whole record. A product reference
//! with no `package` is a local package's product that the pbxproj recorded
//! without a back-reference, and it converts that way.
//!
//! The requirement kinds, measured by converting a project carrying each with
//! Xcode 27.2 and matching Apple's published schema:
//!
//! | pbxproj `requirement.kind` | `version` key |
//! | --- | --- |
//! | `upToNextMajorVersion` | `up-to-next-major-version` |
//! | `upToNextMinorVersion` | `up-to-next-minor-version` |
//! | `exactVersion` | `version` |
//! | `versionRange` | `version-range`, `"1.3.0..<1.5.0"` |
//! | `branch` | `branch` |
//! | `revision` | `revision` |
//!
//! A range whose bounds are not plain dotted numbers is stored as the
//! `version-range-min` / `version-range-max` pair instead.
//!
//! Since a package is named rather than pointed at, its name is also the handle
//! this module hands back and takes. Two packages sharing a name could not be
//! told apart by a product reference, so an add whose name is taken is
//! refused.
//!
//! Everything here is pure (no I/O): callers parse the file, mutate the tree,
//! and serialize/write it.

use std::cmp::Ordering;

use crate::schema_xcproj::{insert_document_key, insert_target_key};
use crate::xcproj::{Array, Object, Value};

pub use crate::spm::{
    DeclaredPackage, PackageKind, ProductLink, Requirement, RequirementSpec, identity_from_path,
    identity_from_url, package_name,
};

/// The declared packages, in `packages` order.
#[must_use]
pub fn list_packages(root: &Value) -> Vec<DeclaredPackage> {
    packages(root)
        .iter()
        .filter_map(|entry| {
            let kind = package_kind(entry)?;
            let identity = match &kind {
                PackageKind::Remote { url } => identity_from_url(url),
                PackageKind::Local { relative_path } => identity_from_path(relative_path),
            };
            let name = package_name(&kind);
            let mut products = links_of(root, &name);
            products.sort();
            products.dedup();
            Some(DeclaredPackage {
                requirement: entry.get("version").and_then(read_requirement),
                id: name,
                identity,
                kind,
                products,
            })
        })
        .collect()
}

/// Locate a package by query: its repository URL, relative path, or SwiftPM
/// identity (so `keychain-swift` matches
/// `https://github.com/evgenyneu/keychain-swift`). Returns its name.
#[must_use]
pub fn find_package(root: &Value, query: &str) -> Option<String> {
    packages(root)
        .iter()
        .filter_map(package_kind)
        .find(|kind| match kind {
            PackageKind::Remote { url } => {
                query == url || identity_from_url(query) == identity_from_url(url)
            }
            PackageKind::Local { relative_path } => {
                query == relative_path
                    || identity_from_path(query) == identity_from_path(relative_path)
            }
        })
        .map(|kind| package_name(&kind))
}

/// Declare a remote package and return its name. Links no product — call
/// [`link_product`] once the package's products are known.
///
/// # Errors
/// Returns a message when the document is malformed or a package of the same
/// name is already declared.
pub fn add_remote_dependency(
    root: &mut Value,
    url: &str,
    requirement: &RequirementSpec,
) -> Result<String, String> {
    let mut entry = Object::new();
    entry.insert("kind".to_string(), string("remote"));
    entry.insert("repository".to_string(), string(url));
    entry.insert("version".to_string(), version_object(requirement));
    add_package(
        root,
        &PackageKind::Remote {
            url: url.to_string(),
        },
        entry,
    )
}

/// Declare a local package by its path relative to the project directory, and
/// return its name.
///
/// # Errors
/// Returns a message when the document is malformed or a package of the same
/// name is already declared.
pub fn add_local_dependency(root: &mut Value, relative_path: &str) -> Result<String, String> {
    let mut entry = Object::new();
    entry.insert("kind".to_string(), string("local"));
    entry.insert("path".to_string(), string(relative_path));
    add_package(
        root,
        &PackageKind::Local {
            relative_path: relative_path.to_string(),
        },
        entry,
    )
}

fn add_package(root: &mut Value, kind: &PackageKind, entry: Object) -> Result<String, String> {
    let name = package_name(kind);
    let taken = packages(root)
        .iter()
        .filter_map(package_kind)
        .any(|existing| package_name(&existing).eq_ignore_ascii_case(&name));
    if taken {
        return Err(format!("a package named `{name}` is already declared"));
    }
    let document = root
        .as_object_mut()
        .ok_or_else(|| "the document is not an object".to_string())?;
    if document.get("packages").is_none() {
        insert_document_key(document, "packages", Value::Array(Array::new()));
    }
    document
        .get_mut("packages")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "packages is not an array".to_string())?
        .push(Value::Object(entry));
    Ok(name)
}

/// Replace a remote package's version requirement in place (for `dependency
/// update <pkg> <requirement>` — bump, pin, or downgrade).
///
/// # Errors
/// Returns a message when the package is missing, or it is local and so
/// carries no requirement.
pub fn set_requirement(
    root: &mut Value,
    name: &str,
    requirement: &RequirementSpec,
) -> Result<(), String> {
    let entry = root
        .get_mut("packages")
        .and_then(Value::as_array_mut)
        .and_then(|list| {
            list.iter_mut()
                .find(|entry| package_kind(entry).is_some_and(|kind| package_name(&kind) == name))
        })
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("no package named `{name}`"))?;
    if entry.get("kind").and_then(Value::as_str) != Some("remote") {
        return Err("a local package has no version requirement to change".to_string());
    }
    let version = version_object(requirement);
    // `traits` is the one key written after `version`.
    let traits_at = entry.iter().position(|(k, _)| k == "traits");
    match traits_at {
        Some(at) if !entry.contains_key("version") => {
            entry.insert_at(at, "version".to_string(), version);
        }
        _ => entry.insert("version".to_string(), version),
    }
    Ok(())
}

/// Link `product`, provided by the package `name`, into the target
/// `target_name`.
///
/// A target with a Frameworks phase gets a `package-product-members` entry for
/// that phase; a static library, or a target with no Frameworks phase, gets a
/// `{ kind: package }` dependency instead — the same split the pbxproj path
/// makes between a build file and a target dependency. Linking a product that
/// is already linked the same way changes nothing.
///
/// # Errors
/// Returns a message when the package or the target is missing.
pub fn link_product(
    root: &mut Value,
    name: &str,
    product: &str,
    target_name: &str,
) -> Result<(), String> {
    if !packages(root)
        .iter()
        .filter_map(package_kind)
        .any(|kind| package_name(&kind) == name)
    {
        return Err(format!("no package named `{name}`"));
    }
    let target = targets_mut(root)
        .and_then(|targets| {
            targets
                .iter_mut()
                .find(|t| t.get("name").and_then(Value::as_str) == Some(target_name))
        })
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("no target named `{target_name}` in the project"))?;

    if is_static_library(target) || !has_frameworks_phase(target) {
        let mut dependency = Object::new();
        dependency.insert("kind".to_string(), string("package"));
        dependency.insert("package".to_string(), string(name));
        dependency.insert("product-name".to_string(), string(product));
        dependency.set_compact(true);
        let dependencies = list_mut(target, "dependencies")?;
        let dependency = Value::Object(dependency);
        if !dependencies.contains(&dependency) {
            dependencies.push(dependency);
        }
        return Ok(());
    }

    let mut phase = Object::new();
    phase.insert("build-phase".to_string(), string("frameworks"));
    phase.set_compact(true);
    let mut member = Object::new();
    member.insert("package".to_string(), string(name));
    member.insert("product-name".to_string(), string(product));
    member.insert("build-phase".to_string(), Value::Object(phase));
    let member = Value::Object(member);
    let members = list_mut(target, "package-product-members")?;
    if members.contains(&member) {
        return Ok(());
    }
    // Xcode writes the list sorted, by the order the published schema gives a
    // member: package, product, product type, then build phase.
    let key = member_order(&member);
    let at = members
        .iter()
        .position(|m| member_order(m).cmp(&key) == Ordering::Greater)
        .unwrap_or(members.len());
    members.insert(at, member);
    Ok(())
}

/// Remove a package and every reference to its products.
///
/// `orphan_products` names local-package products whose references carry no
/// `package` (the pbxproj's missing back-reference, carried through
/// conversion): pass the product names the local package declares so they go
/// too. Pass `&[]` for a remote package.
///
/// # Errors
/// Returns a message when the document is malformed.
pub fn remove_package(
    root: &mut Value,
    name: &str,
    orphan_products: &[String],
) -> Result<(), String> {
    let document = root
        .as_object_mut()
        .ok_or_else(|| "the document is not an object".to_string())?;
    if let Some(list) = document.get_mut("packages").and_then(Value::as_array_mut) {
        list.retain(|entry| package_kind(entry).is_none_or(|kind| package_name(&kind) != name));
        if list.is_empty() {
            document.remove("packages");
        }
    }
    let belongs = |reference: &Value| match reference.get("package").and_then(Value::as_str) {
        Some(package) => package == name,
        None => product_name(reference).is_some_and(|p| orphan_products.iter().any(|o| o == p)),
    };
    for target in targets_mut(root).into_iter().flat_map(|t| t.iter_mut()) {
        drop_references(target, |r| belongs(r));
    }
    Ok(())
}

/// Unlink products of the package `name` from targets, optionally narrowed to
/// one product and/or one target, keeping the package declared. Returns the
/// `(product, target)` pairs that were unlinked.
///
/// # Errors
/// Returns a message when the document is malformed.
pub fn unlink(
    root: &mut Value,
    name: &str,
    product: Option<&str>,
    target: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let mut unlinked: Vec<(String, String)> = Vec::new();
    for t in targets_mut(root).into_iter().flat_map(|t| t.iter_mut()) {
        let Some(target_name) = t.get("name").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if target.is_some_and(|wanted| wanted != target_name) {
            continue;
        }
        let matches = |reference: &Value| {
            reference.get("package").and_then(Value::as_str) == Some(name)
                && product_name(reference).is_some_and(|p| product.is_none_or(|w| w == p))
        };
        for removed in drop_references(t, matches) {
            let pair = (removed, target_name.clone());
            if !unlinked.contains(&pair) {
                unlinked.push(pair);
            }
        }
    }
    Ok(unlinked)
}

/// Drop the package-product references on a target that `matches` accepts,
/// from both lists, returning the product names they carried. An emptied list
/// is dropped, since Xcode omits empty collections.
fn drop_references(target: &mut Value, matches: impl Fn(&Value) -> bool) -> Vec<String> {
    let mut removed = Vec::new();
    let Some(target) = target.as_object_mut() else {
        return removed;
    };
    for key in ["package-product-members", "dependencies"] {
        let Some(list) = target.get_mut(key).and_then(Value::as_array_mut) else {
            continue;
        };
        list.retain(|reference| {
            let is_package = key == "package-product-members" || is_package_dependency(reference);
            if is_package && matches(reference) {
                removed.extend(product_name(reference).map(str::to_string));
                false
            } else {
                true
            }
        });
        if list.is_empty() {
            target.remove(key);
        }
    }
    removed
}

/// The `(product, target)` links naming `name`, across every target, in
/// document order.
fn links_of(root: &Value, name: &str) -> Vec<ProductLink> {
    let mut links = Vec::new();
    for target in root
        .get("targets")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        let Some(target_name) = target.get("name").and_then(Value::as_str) else {
            continue;
        };
        let members = target
            .get("package-product-members")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter();
        let dependencies = target
            .get("dependencies")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .filter(|d| is_package_dependency(d));
        for reference in members.chain(dependencies) {
            if reference.get("package").and_then(Value::as_str) != Some(name) {
                continue;
            }
            if let Some(product) = product_name(reference) {
                links.push(ProductLink {
                    product: product.to_string(),
                    target: target_name.to_string(),
                });
            }
        }
    }
    links
}

fn read_requirement(version: &Value) -> Option<Requirement> {
    let text = |key: &str| version.get(key).and_then(Value::as_str).map(str::to_string);
    let single = |kind: &str, value: Option<String>| {
        value.map(|value| Requirement {
            kind: kind.to_string(),
            value: Some(value),
            upper: None,
        })
    };
    // The precedence the published schema's decoder gives the keys.
    single("revision", text("revision"))
        .or_else(|| single("branch", text("branch")))
        .or_else(|| single("exactVersion", text("version")))
        .or_else(|| single("upToNextMinorVersion", text("up-to-next-minor-version")))
        .or_else(|| single("upToNextMajorVersion", text("up-to-next-major-version")))
        .or_else(|| {
            let range = text("version-range")?;
            let (lower, upper) = range.split_once("..<")?;
            Some(Requirement {
                kind: "versionRange".to_string(),
                value: Some(lower.to_string()),
                upper: Some(upper.to_string()),
            })
        })
        .or_else(|| {
            let lower = text("version-range-min")?;
            Some(Requirement {
                kind: "versionRange".to_string(),
                value: Some(lower),
                upper: text("version-range-max"),
            })
        })
}

/// The `version` object for a requirement, spelled as Xcode spells it.
fn version_object(spec: &RequirementSpec) -> Value {
    let mut version = Object::new();
    let mut put = |key: &str, value: &str| version.insert(key.to_string(), string(value));
    match spec {
        RequirementSpec::UpToNextMajor(v) => put("up-to-next-major-version", v),
        RequirementSpec::UpToNextMinor(v) => put("up-to-next-minor-version", v),
        RequirementSpec::Exact(v) => put("version", v),
        RequirementSpec::Range { from, to } if is_plain_version(from) && is_plain_version(to) => {
            put("version-range", &format!("{from}..<{to}"));
        }
        RequirementSpec::Range { from, to } => {
            put("version-range-min", from);
            put("version-range-max", to);
        }
        RequirementSpec::Branch(b) => put("branch", b),
        RequirementSpec::Revision(r) => put("revision", r),
    }
    Value::Object(version)
}

/// Whether a version is only digits and dots — the schema's test for writing
/// a range as one `version-range` string.
fn is_plain_version(version: &str) -> bool {
    version.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// The sort key the published schema gives a `package-product-members` entry:
/// package name (none sorts first), product name, product type, then the build
/// phase as kind and name, with a phase addressed by id last among equals.
fn member_order(member: &Value) -> (String, String, String, String, String, String) {
    let text = |key: &str| {
        member
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let product_type = member
        .get("product-type")
        .and_then(Value::as_str)
        .unwrap_or("other")
        .to_string();
    let phase = member.get("build-phase");
    let spelling = phase
        .and_then(Value::as_str)
        .or_else(|| {
            phase
                .and_then(|p| p.get("build-phase"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    let (kind, name, id) = match spelling.split_once('/') {
        Some((kind, name)) if PHASE_KINDS.contains(&kind) => (kind, name, ""),
        None if PHASE_KINDS.contains(&spelling) => (spelling, "", ""),
        _ => ("", "", spelling),
    };
    (
        text("package"),
        text("product-name"),
        product_type,
        kind.to_string(),
        name.to_string(),
        id.to_string(),
    )
}

/// Every build phase kind the format names, so a phase spelling can be told
/// from an object id.
const PHASE_KINDS: [&str; 9] = [
    "apple-script",
    "compile-sources",
    "copy",
    "frameworks",
    "headers",
    "java-archive",
    "resources",
    "rez",
    "script",
];

fn packages(root: &Value) -> &[Value] {
    root.get("packages")
        .and_then(Value::as_array)
        .unwrap_or_default()
}

fn package_kind(entry: &Value) -> Option<PackageKind> {
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    match entry.get("kind").and_then(Value::as_str)? {
        "remote" => Some(PackageKind::Remote {
            url: text("repository")?,
        }),
        "local" => Some(PackageKind::Local {
            relative_path: text("path")?,
        }),
        _ => None,
    }
}

fn product_name(reference: &Value) -> Option<&str> {
    reference.get("product-name").and_then(Value::as_str)
}

fn is_package_dependency(dependency: &Value) -> bool {
    dependency.get("kind").and_then(Value::as_str) == Some("package")
}

fn is_static_library(target: &Object) -> bool {
    target.get("product-type").and_then(Value::as_str) == Some("library.static")
        || target.get("full-product-type").and_then(Value::as_str)
            == Some("com.apple.product-type.library.static")
}

/// A Frameworks phase is the bare `"frameworks"` until it carries properties,
/// and then an object of that kind.
fn has_frameworks_phase(target: &Object) -> bool {
    target
        .get("build-phases")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .any(|phase| {
            phase.as_str() == Some("frameworks")
                || phase.get("kind").and_then(Value::as_str) == Some("frameworks")
        })
}

fn targets_mut(root: &mut Value) -> Option<&mut Array> {
    root.get_mut("targets").and_then(Value::as_array_mut)
}

/// The list under `key` on a target, created where Xcode would put it.
fn list_mut<'a>(target: &'a mut Object, key: &str) -> Result<&'a mut Array, String> {
    if target.get(key).is_none() {
        insert_target_key(target, key, Value::Array(Array::new()));
    }
    target
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("{key} is not an array"))
}

fn string(value: &str) -> Value {
    Value::String(value.to_string())
}
