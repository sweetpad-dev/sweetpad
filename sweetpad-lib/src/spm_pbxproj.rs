//! Reading and mutating a project's Swift Package Manager dependencies in a
//! parsed [`crate::pbxproj::Value`] tree.
//!
//! Xcode stores SPM dependencies in `project.pbxproj` as three object kinds —
//! `XCRemoteSwiftPackageReference` / `XCLocalSwiftPackageReference` (the
//! declared packages, listed in `PBXProject.packageReferences`), and
//! `XCSwiftPackageProductDependency` (a product a target consumes, listed in the
//! target's `packageProductDependencies` and linked through a Frameworks
//! `PBXBuildFile` or a static-library `PBXTargetDependency`). There is no Apple
//! CLI to edit these, so this module reads and rewrites the object graph
//! directly; the serializer ([`crate::pbxproj_writer`]) already knows these isa
//! kinds, so a mutated tree round-trips to Xcode's exact on-disk format.
//!
//! Everything here is pure (no I/O): callers parse the file, mutate the tree,
//! and serialize/write it. That keeps the engine unit-testable against `const`
//! fixtures and reusable by any front end (CLI or N-API).

use std::collections::HashMap;

use crate::pbxproj::{Dict, Value};

/// A package declared in `PBXProject.packageReferences`, plus the products it
/// provides and where they are linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredPackage {
    /// The `XC*SwiftPackageReference` object GUID.
    pub guid: String,
    /// SwiftPM identity (lowercased basename of the URL/path), used to correlate
    /// with a `Package.resolved` pin.
    pub identity: String,
    pub kind: PackageKind,
    /// The version requirement, for remote packages (locals have none).
    pub requirement: Option<Requirement>,
    /// `(product, target)` links this package's products participate in.
    pub products: Vec<ProductLink>,
}

/// A declared package is either a remote git repo or a local directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageKind {
    Remote { url: String },
    Local { relative_path: String },
}

impl PackageKind {
    /// The repository URL (remote) or relative path (local) — the human display.
    #[must_use]
    pub fn display(&self) -> &str {
        match self {
            PackageKind::Remote { url } => url,
            PackageKind::Local { relative_path } => relative_path,
        }
    }

    #[must_use]
    pub fn is_remote(&self) -> bool {
        matches!(self, PackageKind::Remote { .. })
    }
}

/// A version requirement parsed from an `XCRemoteSwiftPackageReference`'s
/// `requirement` dict, flattened to the one or two version-ish values each
/// `kind` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// `upToNextMajorVersion` / `upToNextMinorVersion` / `exactVersion` /
    /// `versionRange` / `branch` / `revision`.
    pub kind: String,
    /// `minimumVersion` (ranges), `version` (exact), `branch`, or `revision`.
    pub value: Option<String>,
    /// `maximumVersion`, for `versionRange` only.
    pub upper: Option<String>,
}

impl Requirement {
    /// A compact human rendering, e.g. `from 5.0.0`, `branch main`,
    /// `5.0.0 ..< 6.0.0`.
    #[must_use]
    pub fn display(&self) -> String {
        let v = self.value.as_deref().unwrap_or("?");
        match self.kind.as_str() {
            "upToNextMajorVersion" => format!("from {v}"),
            "upToNextMinorVersion" => format!("up-to-next-minor from {v}"),
            "exactVersion" => format!("exact {v}"),
            "versionRange" => format!("{v} ..< {}", self.upper.as_deref().unwrap_or("?")),
            "branch" => format!("branch {v}"),
            "revision" => format!("revision {v}"),
            other => format!("{other} {v}"),
        }
    }
}

/// A product linked into a target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProductLink {
    pub product: String,
    pub target: String,
}

/// The version requirement to record when adding a remote package. Mirrors the
/// `swift package add-dependency` requirement flags; the CLI maps its flags onto
/// this so this module stays clap-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequirementSpec {
    /// `--from` (up to the next major).
    UpToNextMajor(String),
    /// `--up-to-next-minor-from`.
    UpToNextMinor(String),
    /// `--exact`.
    Exact(String),
    /// `--from … --to …` (half-open range).
    Range { from: String, to: String },
    /// `--branch`.
    Branch(String),
    /// `--revision`.
    Revision(String),
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// SwiftPM's package identity for a repository URL: the last path component,
/// without a trailing `.git`, lowercased. Matches the `identity` key in
/// `Package.resolved` (and the basename the serializer annotates with).
#[must_use]
pub fn identity_from_url(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(url)
        .trim_end_matches(".git")
        .to_ascii_lowercase()
}

/// Identity for a local package: the lowercased last path component of its
/// relative path.
#[must_use]
pub fn identity_from_path(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// The declared packages of a parsed pbxproj, in `packageReferences` order.
/// Returns empty for a malformed tree (caller treats that as "no packages").
#[must_use]
pub fn list_packages(root: &Value) -> Vec<DeclaredPackage> {
    let Some((objects, project_guid)) = root_parts(root) else {
        return Vec::new();
    };
    read_packages(objects, project_guid)
}

fn read_packages(objects: &Dict, project_guid: &str) -> Vec<DeclaredPackage> {
    let by_target = product_dep_targets(objects);

    // product-dependency GUID -> (productName, owning package GUID, if any).
    let mut prod_info: HashMap<&str, (String, Option<String>)> = HashMap::new();
    for (guid, obj) in objects {
        if isa(obj) == "XCSwiftPackageProductDependency" {
            let name = str_field(obj, "productName")
                .unwrap_or_default()
                .to_string();
            let pkg = str_field(obj, "package").map(str::to_string);
            prod_info.insert(guid.as_str(), (name, pkg));
        }
    }

    let mut packages = Vec::new();
    for ref_guid in package_reference_guids(objects, project_guid) {
        let Some(obj) = objects.get(&ref_guid) else {
            continue;
        };
        let (kind, identity) = match isa(obj) {
            "XCRemoteSwiftPackageReference" => {
                let url = str_field(obj, "repositoryURL")
                    .unwrap_or_default()
                    .to_string();
                let id = identity_from_url(&url);
                (PackageKind::Remote { url }, id)
            }
            "XCLocalSwiftPackageReference" => {
                let path = str_field(obj, "relativePath")
                    .unwrap_or_default()
                    .to_string();
                let id = identity_from_path(&path);
                (
                    PackageKind::Local {
                        relative_path: path,
                    },
                    id,
                )
            }
            _ => continue,
        };
        let requirement = obj.get("requirement").and_then(read_requirement);

        let mut products: Vec<ProductLink> = Vec::new();
        for (pg, (name, pkg)) in &prod_info {
            if pkg.as_deref() != Some(ref_guid.as_str()) {
                continue;
            }
            for target in by_target.get(*pg).into_iter().flatten() {
                products.push(ProductLink {
                    product: name.clone(),
                    target: target.clone(),
                });
            }
        }
        products.sort();

        packages.push(DeclaredPackage {
            guid: ref_guid,
            identity,
            kind,
            requirement,
            products,
        });
    }
    packages
}

fn read_requirement(req: &Value) -> Option<Requirement> {
    let d = req.as_dict()?;
    let kind = d.get("kind").and_then(Value::as_str)?.to_string();
    let value = ["minimumVersion", "version", "branch", "revision"]
        .iter()
        .find_map(|k| d.get(k).and_then(Value::as_str))
        .map(str::to_string);
    let upper = d
        .get("maximumVersion")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(Requirement { kind, value, upper })
}

// ---------------------------------------------------------------------------
// Mutation
// ---------------------------------------------------------------------------

/// Add a remote package reference and return its new GUID. Does not link any
/// product — call [`link_product`] after resolving the package's products.
///
/// # Errors
/// Returns a message when the tree has no `objects` dict or `rootObject`.
pub fn add_remote_dependency(
    root: &mut Value,
    url: &str,
    requirement: &RequirementSpec,
) -> Result<String, String> {
    let project_guid = root_guid(root)?;
    let objects = objects_mut(root)?;
    let ref_guid = fresh_guid(objects, url, 0);
    objects.insert(
        ref_guid.clone(),
        vdict([
            ("isa", vstr("XCRemoteSwiftPackageReference")),
            ("repositoryURL", vstr(url)),
            ("requirement", requirement_dict(requirement)),
        ]),
    );
    push_into_array_at(objects, &project_guid, "packageReferences", gid(&ref_guid))?;
    Ok(ref_guid)
}

/// Add a local package reference (`XCLocalSwiftPackageReference`) by its path
/// relative to the project directory, returning its new GUID.
///
/// # Errors
/// Returns a message when the tree has no `objects` dict or `rootObject`.
pub fn add_local_dependency(root: &mut Value, relative_path: &str) -> Result<String, String> {
    let project_guid = root_guid(root)?;
    let objects = objects_mut(root)?;
    let ref_guid = fresh_guid(objects, relative_path, 0);
    objects.insert(
        ref_guid.clone(),
        vdict([
            ("isa", vstr("XCLocalSwiftPackageReference")),
            ("relativePath", vstr(relative_path)),
        ]),
    );
    push_into_array_at(objects, &project_guid, "packageReferences", gid(&ref_guid))?;
    Ok(ref_guid)
}

/// Replace the version requirement of an existing remote package reference (for
/// `dependency update <pkg> <requirement>` — bump, pin, or downgrade). The
/// `requirement` key is replaced in place, so the diff stays minimal.
///
/// # Errors
/// Returns a message when the tree is malformed, the package is missing, or it
/// is a local reference (which carries no requirement).
pub fn set_requirement(
    root: &mut Value,
    ref_guid: &str,
    requirement: &RequirementSpec,
) -> Result<(), String> {
    let objects = objects_mut(root)?;
    let obj = objects
        .get_mut(ref_guid)
        .and_then(Value::as_dict_mut)
        .ok_or_else(|| format!("package reference {ref_guid} not found"))?;
    if obj.get("isa").and_then(Value::as_str) != Some("XCRemoteSwiftPackageReference") {
        return Err("a local package has no version requirement to change".to_string());
    }
    obj.insert("requirement".to_string(), requirement_dict(requirement));
    Ok(())
}

/// Link `product` (provided by the package `ref_guid`) into the target named
/// `target_name`. App/framework/test targets get a Frameworks `PBXBuildFile`;
/// static-library targets get a `PBXTargetDependency` (matching Xcode). Both
/// also push the product into the target's `packageProductDependencies`.
///
/// # Errors
/// Returns a message when the tree is malformed or the target is not found.
pub fn link_product(
    root: &mut Value,
    ref_guid: &str,
    product: &str,
    target_name: &str,
) -> Result<(), String> {
    let objects = objects_mut(root)?;
    let target_guid = find_target_guid(objects, target_name)
        .ok_or_else(|| format!("no target named `{target_name}` in the project"))?;
    let product_type = objects
        .get(&target_guid)
        .and_then(|t| t.get("productType"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let prod_guid = fresh_guid(objects, &format!("{ref_guid}#{product}#{target_guid}"), 0);
    objects.insert(
        prod_guid.clone(),
        vdict([
            ("isa", vstr("XCSwiftPackageProductDependency")),
            ("package", gid(ref_guid)),
            ("productName", vstr(product)),
        ]),
    );
    push_into_array_at(
        objects,
        &target_guid,
        "packageProductDependencies",
        gid(&prod_guid),
    )?;

    if is_static_library(product_type.as_deref()) {
        add_target_dependency(objects, &target_guid, &prod_guid, product);
        return Ok(());
    }
    if let Some(phase_guid) = frameworks_phase_of(objects, &target_guid) {
        let bf_guid = fresh_guid(objects, &format!("{product}#{target_guid}#buildfile"), 0);
        let mut bf = Dict::new();
        bf.insert("isa".into(), vstr("PBXBuildFile"));
        bf.insert("productRef".into(), gid(&prod_guid));
        // Package-product build files are written on one line by Xcode.
        bf.set_single_line(true);
        objects.insert(bf_guid.clone(), Value::Dict(bf));
        push_into_array_at(objects, &phase_guid, "files", gid(&bf_guid))?;
    } else {
        // No Frameworks phase (unusual for app/framework targets) — express the
        // link as a target dependency so it is still recorded.
        add_target_dependency(objects, &target_guid, &prod_guid, product);
    }
    Ok(())
}

fn add_target_dependency(objects: &mut Dict, target_guid: &str, prod_guid: &str, product: &str) {
    let dep_guid = fresh_guid(objects, &format!("{product}#{target_guid}#targetdep"), 0);
    objects.insert(
        dep_guid.clone(),
        vdict([
            ("isa", vstr("PBXTargetDependency")),
            ("productRef", gid(prod_guid)),
        ]),
    );
    // Best-effort: the target was looked up by the caller, so it exists.
    let _ = push_into_array_at(objects, target_guid, "dependencies", gid(&dep_guid));
}

/// Locate a package reference by query: its repository URL, relative path, or
/// SwiftPM identity (so `keychain-swift` matches
/// `https://github.com/evgenyneu/keychain-swift`).
#[must_use]
pub fn find_package(root: &Value, query: &str) -> Option<String> {
    let (objects, project_guid) = root_parts(root)?;
    for ref_guid in package_reference_guids(objects, project_guid) {
        let Some(obj) = objects.get(&ref_guid) else {
            continue;
        };
        let matches = match isa(obj) {
            "XCRemoteSwiftPackageReference" => str_field(obj, "repositoryURL").is_some_and(|url| {
                query == url || identity_from_url(query) == identity_from_url(url)
            }),
            "XCLocalSwiftPackageReference" => str_field(obj, "relativePath")
                .is_some_and(|p| query == p || identity_from_path(query) == identity_from_path(p)),
            _ => false,
        };
        if matches {
            return Some(ref_guid);
        }
    }
    None
}

/// Remove a package reference and everything wiring it in: its product
/// dependencies, every target's link to them, the Frameworks `PBXBuildFile` /
/// `PBXTargetDependency` entries, and the reference itself.
///
/// `orphan_products` matches local-package products that carry no `package`
/// back-ref (Xcode omits it for some local packages): pass the product names the
/// local package declares so they're cleaned up too. Pass `&[]` for remote
/// packages, whose product dependencies are found by their back-ref.
///
/// # Errors
/// Returns a message when the tree is malformed.
pub fn remove_package(
    root: &mut Value,
    ref_guid: &str,
    orphan_products: &[String],
) -> Result<(), String> {
    let project_guid = root_guid(root)?;
    let objects = objects_mut(root)?;
    let prod_guids = product_dep_guids(objects, ref_guid, orphan_products);
    for pg in &prod_guids {
        remove_product_dependency(objects, pg);
    }
    remove_from_array_at(objects, &project_guid, "packageReferences", ref_guid);
    objects.remove(ref_guid);
    Ok(())
}

/// Product-dependency GUIDs belonging to a package: those with a `package`
/// back-ref to `ref_guid`, plus back-ref-less ones whose `productName` is in
/// `orphan_products` (local packages Xcode wrote without the back-ref).
fn product_dep_guids(objects: &Dict, ref_guid: &str, orphan_products: &[String]) -> Vec<String> {
    objects
        .iter()
        .filter(|(_, o)| isa(o) == "XCSwiftPackageProductDependency")
        .filter(|(_, o)| match str_field(o, "package") {
            Some(pkg) => pkg == ref_guid,
            None => str_field(o, "productName")
                .is_some_and(|name| orphan_products.iter().any(|p| p == name)),
        })
        .map(|(g, _)| g.clone())
        .collect()
}

/// Unlink products of `ref_guid` from targets, optionally filtered to one
/// product and/or one target, keeping the package reference itself. Returns the
/// `(product, target)` pairs that were unlinked.
///
/// # Errors
/// Returns a message when the tree is malformed.
pub fn unlink(
    root: &mut Value,
    ref_guid: &str,
    product: Option<&str>,
    target: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let objects = objects_mut(root)?;
    let prod_deps = product_deps_of(objects, ref_guid);
    let by_target = product_dep_targets(objects);

    let mut unlinked: Vec<(String, String)> = Vec::new();
    let mut removed: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (pg, name) in &prod_deps {
        if product.is_some_and(|p| p != name) {
            continue;
        }
        let targets = by_target.get(pg.as_str()).cloned().unwrap_or_default();
        let hits: Vec<String> = targets
            .into_iter()
            .filter(|t| target.is_none_or(|tf| tf == t))
            .collect();
        if hits.is_empty() {
            continue;
        }
        for t in hits {
            unlinked.push((name.clone(), t));
        }
        // Each product dependency is owned by a single target in practice, so
        // removing the dependency object unlinks exactly the matched target(s).
        if removed.insert(pg.clone()) {
            remove_product_dependency(objects, pg);
        }
    }
    Ok(unlinked)
}

// ---------------------------------------------------------------------------
// pbxproj building blocks
// ---------------------------------------------------------------------------

/// Build the `requirement` dict for a package reference, with keys in the order
/// Xcode writes them (alphabetical per kind) so the serialized diff is minimal.
#[must_use]
pub fn requirement_dict(spec: &RequirementSpec) -> Value {
    match spec {
        RequirementSpec::UpToNextMajor(v) => vdict([
            ("kind", vstr("upToNextMajorVersion")),
            ("minimumVersion", vstr(v.clone())),
        ]),
        RequirementSpec::UpToNextMinor(v) => vdict([
            ("kind", vstr("upToNextMinorVersion")),
            ("minimumVersion", vstr(v.clone())),
        ]),
        RequirementSpec::Exact(v) => {
            vdict([("kind", vstr("exactVersion")), ("version", vstr(v.clone()))])
        }
        RequirementSpec::Range { from, to } => vdict([
            ("kind", vstr("versionRange")),
            ("maximumVersion", vstr(to.clone())),
            ("minimumVersion", vstr(from.clone())),
        ]),
        RequirementSpec::Branch(b) => {
            vdict([("branch", vstr(b.clone())), ("kind", vstr("branch"))])
        }
        RequirementSpec::Revision(r) => {
            vdict([("kind", vstr("revision")), ("revision", vstr(r.clone()))])
        }
    }
}

/// A fresh 24-hex-char object GUID (Xcode's id shape) that does not collide with
/// any existing object. Seeded deterministically so re-running the same mutation
/// produces the same id (and the remove-after-add round-trip is byte-exact);
/// `salt` breaks a collision on the rare hash clash.
fn fresh_guid(objects: &Dict, seed: &str, salt: u64) -> String {
    use std::hash::{Hash, Hasher};
    let mut h1 = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut h1);
    salt.hash(&mut h1);
    let a = h1.finish();

    let mut h2 = std::collections::hash_map::DefaultHasher::new();
    a.hash(&mut h2);
    seed.hash(&mut h2);
    (salt ^ 0x9E37_79B9_7F4A_7C15).hash(&mut h2);
    let b = h2.finish();

    let id = format!("{a:016X}{:08X}", (b >> 32) as u32);
    if objects.contains_key(&id) {
        fresh_guid(objects, seed, salt.wrapping_add(1))
    } else {
        id
    }
}

// ---------------------------------------------------------------------------
// Object-graph helpers
// ---------------------------------------------------------------------------

fn isa(obj: &Value) -> &str {
    obj.get("isa").and_then(Value::as_str).unwrap_or("")
}

fn str_field<'a>(obj: &'a Value, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

fn is_target_isa(isa: &str) -> bool {
    matches!(
        isa,
        "PBXNativeTarget" | "PBXAggregateTarget" | "PBXLegacyTarget"
    )
}

fn is_static_library(product_type: Option<&str>) -> bool {
    product_type == Some("com.apple.product-type.library.static")
}

/// `(objects dict, PBXProject GUID)` from a parsed pbxproj.
fn root_parts(root: &Value) -> Option<(&Dict, &str)> {
    let top = root.as_dict()?;
    let objects = top.get("objects")?.as_dict()?;
    let guid = top.get("rootObject")?.as_str()?;
    Some((objects, guid))
}

fn root_guid(root: &Value) -> Result<String, String> {
    root.as_dict()
        .and_then(|d| d.get("rootObject"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "pbxproj has no rootObject".to_string())
}

fn objects_mut(root: &mut Value) -> Result<&mut Dict, String> {
    root.as_dict_mut()
        .and_then(|d| d.get_mut("objects"))
        .and_then(Value::as_dict_mut)
        .ok_or_else(|| "pbxproj has no objects dict".to_string())
}

fn package_reference_guids(objects: &Dict, project_guid: &str) -> Vec<String> {
    objects
        .get(project_guid)
        .and_then(|p| p.get("packageReferences"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// product-dependency GUID -> the names of targets that link it.
fn product_dep_targets(objects: &Dict) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (_, obj) in objects {
        if !is_target_isa(isa(obj)) {
            continue;
        }
        let Some(name) = str_field(obj, "name") else {
            continue;
        };
        if let Some(deps) = obj
            .get("packageProductDependencies")
            .and_then(Value::as_array)
        {
            for d in deps {
                if let Some(g) = d.as_str() {
                    map.entry(g.to_string()).or_default().push(name.to_string());
                }
            }
        }
    }
    map
}

/// `(product-dependency GUID, productName)` for the products a package provides.
fn product_deps_of(objects: &Dict, ref_guid: &str) -> Vec<(String, String)> {
    objects
        .iter()
        .filter(|(_, o)| {
            isa(o) == "XCSwiftPackageProductDependency" && str_field(o, "package") == Some(ref_guid)
        })
        .map(|(g, o)| {
            (
                g.clone(),
                str_field(o, "productName").unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn find_target_guid(objects: &Dict, name: &str) -> Option<String> {
    objects
        .iter()
        .find(|(_, o)| is_target_isa(isa(o)) && str_field(o, "name") == Some(name))
        .map(|(g, _)| g.clone())
}

fn frameworks_phase_of(objects: &Dict, target_guid: &str) -> Option<String> {
    let phases = objects.get(target_guid)?.get("buildPhases")?.as_array()?;
    for p in phases {
        let Some(pg) = p.as_str() else { continue };
        if objects.get(pg).map(isa) == Some("PBXFrameworksBuildPhase") {
            return Some(pg.to_string());
        }
    }
    None
}

/// Delete a product-dependency object: drop its Frameworks build files and
/// target-dependency wrappers, unlink it from every target, then remove it.
fn remove_product_dependency(objects: &mut Dict, prod_guid: &str) {
    for bf in guids_with(objects, "PBXBuildFile", "productRef", prod_guid) {
        remove_guid_from_all_arrays(objects, "files", &bf);
        objects.remove(&bf);
    }
    for td in guids_with(objects, "PBXTargetDependency", "productRef", prod_guid) {
        remove_guid_from_all_arrays(objects, "dependencies", &td);
        objects.remove(&td);
    }
    remove_guid_from_all_arrays(objects, "packageProductDependencies", prod_guid);
    objects.remove(prod_guid);
}

fn guids_with(objects: &Dict, want_isa: &str, key: &str, value: &str) -> Vec<String> {
    objects
        .iter()
        .filter(|(_, o)| isa(o) == want_isa && str_field(o, key) == Some(value))
        .map(|(g, _)| g.clone())
        .collect()
}

fn push_into_array(obj: &mut Dict, key: &str, value: Value) {
    if !obj.contains_key(key) {
        obj.insert(key.to_string(), Value::Array(vec![value]));
        return;
    }
    if let Some(arr) = obj.get_mut(key).and_then(Value::as_array_mut) {
        arr.push(value);
    }
}

fn push_into_array_at(
    objects: &mut Dict,
    owner: &str,
    key: &str,
    value: Value,
) -> Result<(), String> {
    let obj = objects
        .get_mut(owner)
        .and_then(Value::as_dict_mut)
        .ok_or_else(|| format!("object {owner} not found in pbxproj"))?;
    push_into_array(obj, key, value);
    Ok(())
}

fn remove_from_array_at(objects: &mut Dict, owner: &str, key: &str, guid: &str) {
    if let Some(arr) = objects
        .get_mut(owner)
        .and_then(Value::as_dict_mut)
        .and_then(|d| d.get_mut(key))
        .and_then(Value::as_array_mut)
    {
        arr.retain(|v| v.as_str() != Some(guid));
    }
}

fn remove_guid_from_all_arrays(objects: &mut Dict, key: &str, guid: &str) {
    let owners: Vec<String> = objects
        .iter()
        .filter(|(_, o)| o.get(key).and_then(Value::as_array).is_some())
        .map(|(g, _)| g.clone())
        .collect();
    for owner in owners {
        remove_from_array_at(objects, &owner, key, guid);
    }
}

fn vstr(value: impl Into<String>) -> Value {
    Value::String(value.into())
}

fn gid(guid: &str) -> Value {
    Value::String(guid.to_string())
}

fn vdict<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Dict(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbxproj::parse;
    use crate::pbxproj_writer::serialize;

    /// A small but realistic project: a remote major-version package, a local
    /// package, an app target with a Frameworks phase, one remote product linked
    /// (with a `package` back-ref) and one local/implicit product without one.
    const PBXPROJ: &str = r#"// !$*UTF8*$!
{
	archiveVersion = 1;
	objectVersion = 77;
	objects = {
		PROJ = {
			isa = PBXProject;
			buildConfigurationList = CFG;
			packageReferences = (
				REMOTE1,
				LOCAL1,
			);
			targets = (
				TGT,
			);
		};
		TGT = {
			isa = PBXNativeTarget;
			buildPhases = (
				FRAMEWORKS,
			);
			name = App;
			packageProductDependencies = (
				PRODA,
				PRODB,
			);
			productType = "com.apple.product-type.application";
		};
		FRAMEWORKS = {
			isa = PBXFrameworksBuildPhase;
			files = (
				BFA,
			);
		};
		BFA = {isa = PBXBuildFile; productRef = PRODA; };
		REMOTE1 = {
			isa = XCRemoteSwiftPackageReference;
			repositoryURL = "https://github.com/evgenyneu/keychain-swift";
			requirement = {
				kind = upToNextMajorVersion;
				minimumVersion = 5.0.0;
			};
		};
		LOCAL1 = {
			isa = XCLocalSwiftPackageReference;
			relativePath = Dep;
		};
		PRODA = {
			isa = XCSwiftPackageProductDependency;
			package = REMOTE1;
			productName = KeychainSwift;
		};
		PRODB = {
			isa = XCSwiftPackageProductDependency;
			productName = Dep;
		};
	};
	rootObject = PROJ;
}
"#;

    #[test]
    fn identity_normalizes_url_and_path() {
        assert_eq!(
            identity_from_url("https://github.com/mergesort/Bodega"),
            "bodega"
        );
        assert_eq!(
            identity_from_url("https://github.com/kaishin/Gifu.git"),
            "gifu"
        );
        assert_eq!(identity_from_url("https://github.com/foo/Bar/"), "bar");
        assert_eq!(identity_from_url("keychain-swift"), "keychain-swift");
        assert_eq!(identity_from_path("Packages/Env"), "env");
    }

    #[test]
    fn reads_declared_packages_with_links() {
        let root = parse(PBXPROJ).unwrap();
        let pkgs = list_packages(&root);
        assert_eq!(pkgs.len(), 2);

        let remote = &pkgs[0];
        assert_eq!(remote.identity, "keychain-swift");
        assert!(remote.kind.is_remote());
        assert_eq!(remote.requirement.as_ref().unwrap().display(), "from 5.0.0");
        assert_eq!(
            remote.products,
            vec![ProductLink {
                product: "KeychainSwift".into(),
                target: "App".into(),
            }]
        );

        // The local package's product has no `package` back-ref in this fixture,
        // so it isn't attributed to LOCAL1 (which shows no links).
        let local = &pkgs[1];
        assert_eq!(local.identity, "dep");
        assert!(!local.kind.is_remote());
        assert!(local.requirement.is_none());
        assert!(local.products.is_empty());
    }

    #[test]
    fn requirement_dict_keys_match_xcode_order() {
        let branch = requirement_dict(&RequirementSpec::Branch("main".into()));
        let d = branch.as_dict().unwrap();
        assert_eq!(
            d.keys().map(String::as_str).collect::<Vec<_>>(),
            ["branch", "kind"]
        );
        assert_eq!(d.get("kind").and_then(Value::as_str), Some("branch"));

        let range = requirement_dict(&RequirementSpec::Range {
            from: "1.0.0".into(),
            to: "2.0.0".into(),
        });
        let d = range.as_dict().unwrap();
        assert_eq!(
            d.keys().map(String::as_str).collect::<Vec<_>>(),
            ["kind", "maximumVersion", "minimumVersion"]
        );
    }

    #[test]
    fn add_then_remove_round_trips_byte_for_byte() {
        let original = serialize(&parse(PBXPROJ).unwrap(), "App");

        let mut root = parse(PBXPROJ).unwrap();
        let ref_guid = add_remote_dependency(
            &mut root,
            "https://github.com/Alamofire/Alamofire.git",
            &RequirementSpec::UpToNextMajor("5.9.0".into()),
        )
        .unwrap();
        link_product(&mut root, &ref_guid, "Alamofire", "App").unwrap();

        // The mutated project serializes cleanly and mentions the new package.
        let added = serialize(&root, "App");
        assert!(added.contains("repositoryURL = \"https://github.com/Alamofire/Alamofire.git\""));
        assert!(added.contains("Alamofire in Frameworks"));
        assert!(added.contains("productName = Alamofire"));
        // Serialization is idempotent (re-parsing and re-serializing is stable).
        assert_eq!(serialize(&parse(&added).unwrap(), "App"), added);

        // Removing the package restores the original bytes exactly.
        remove_package(&mut root, &ref_guid, &[]).unwrap();
        assert_eq!(serialize(&root, "App"), original);
    }

    #[test]
    fn add_links_into_static_library_via_target_dependency() {
        // An all-static-library project: linking a product wires a
        // PBXTargetDependency, not a Frameworks build file.
        let src = PBXPROJ.replace(
            "\"com.apple.product-type.application\"",
            "\"com.apple.product-type.library.static\"",
        );
        let mut root = parse(&src).unwrap();
        let ref_guid = add_remote_dependency(
            &mut root,
            "https://github.com/apple/swift-log",
            &RequirementSpec::Exact("1.0.0".into()),
        )
        .unwrap();
        link_product(&mut root, &ref_guid, "Logging", "App").unwrap();
        let text = serialize(&root, "App");
        assert!(text.contains("isa = PBXTargetDependency"));
        assert!(text.contains("productName = Logging"));
        // No Frameworks build file was created for the new product.
        assert!(!text.contains("Logging in Frameworks"));
    }

    #[test]
    fn find_package_matches_url_and_identity() {
        let root = parse(PBXPROJ).unwrap();
        assert!(find_package(&root, "keychain-swift").is_some());
        assert!(find_package(&root, "https://github.com/evgenyneu/keychain-swift").is_some());
        assert_eq!(find_package(&root, "Dep"), find_package(&root, "dep"));
        assert!(find_package(&root, "nonexistent").is_none());
    }

    #[test]
    fn round_trips_against_the_real_ice_cubes_project() {
        // Exercise add → link → remove on a full, real 1500-line pbxproj (56
        // product dependencies). Skips cleanly if the corpus isn't checked out.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/corpus/ice-cubes/IceCubesApp.xcodeproj/project.pbxproj"
        );
        let Ok(src) = std::fs::read_to_string(path) else {
            return;
        };
        let baseline = serialize(&parse(&src).unwrap(), "IceCubesApp");

        let mut root = parse(&src).unwrap();
        assert_eq!(
            list_packages(&root).len(),
            4,
            "ice-cubes declares 4 packages"
        );

        let ref_guid = add_remote_dependency(
            &mut root,
            "https://github.com/apple/swift-log",
            &RequirementSpec::UpToNextMajor("1.0.0".into()),
        )
        .unwrap();
        link_product(&mut root, &ref_guid, "Logging", "IceCubesApp").unwrap();
        assert_eq!(list_packages(&root).len(), 5);

        let added = serialize(&root, "IceCubesApp");
        assert!(added.contains("Logging in Frameworks"));
        // Re-parsing and re-serializing the mutated project is stable.
        assert_eq!(serialize(&parse(&added).unwrap(), "IceCubesApp"), added);

        remove_package(&mut root, &ref_guid, &[]).unwrap();
        assert_eq!(
            serialize(&root, "IceCubesApp"),
            baseline,
            "remove cleanly reverses add"
        );
    }

    #[test]
    fn unlink_removes_one_product_from_one_target() {
        let mut root = parse(PBXPROJ).unwrap();
        let ref_guid = find_package(&root, "keychain-swift").unwrap();
        let unlinked = unlink(&mut root, &ref_guid, Some("KeychainSwift"), Some("App")).unwrap();
        assert_eq!(
            unlinked,
            vec![("KeychainSwift".to_string(), "App".to_string())]
        );

        // The package reference survives; its product link is gone.
        let pkgs = list_packages(&root);
        let remote = pkgs
            .iter()
            .find(|p| p.identity == "keychain-swift")
            .unwrap();
        assert!(remote.products.is_empty());
        let text = serialize(&root, "App");
        assert!(text.contains("XCRemoteSwiftPackageReference"));
        assert!(!text.contains("KeychainSwift in Frameworks"));
    }

    #[test]
    fn set_requirement_replaces_in_place() {
        let mut root = parse(PBXPROJ).unwrap();
        let ref_guid = find_package(&root, "keychain-swift").unwrap();
        // master branch → pin to exactly 2.0.0 (a downgrade-style change).
        set_requirement(
            &mut root,
            &ref_guid,
            &RequirementSpec::Exact("2.0.0".into()),
        )
        .unwrap();

        let pkgs = list_packages(&root);
        let remote = pkgs
            .iter()
            .find(|p| p.identity == "keychain-swift")
            .unwrap();
        assert_eq!(
            remote.requirement.as_ref().unwrap().display(),
            "exact 2.0.0"
        );
        let text = serialize(&root, "App");
        assert!(text.contains("kind = exactVersion"));
        assert!(text.contains("version = 2.0.0"));
        assert!(!text.contains("branch = master"));
    }

    #[test]
    fn set_requirement_rejects_local_package() {
        let mut root = parse(PBXPROJ).unwrap();
        let local = find_package(&root, "Dep").unwrap();
        assert!(
            set_requirement(&mut root, &local, &RequirementSpec::Exact("1.0.0".into())).is_err()
        );
    }

    #[test]
    fn remove_local_package_cleans_up_back_ref_less_products() {
        // The local "Dep" package's product (PRODB) has no `package` back-ref, as
        // Xcode writes some local products. Removing by declared product name
        // ("Dep") must still delete it, not leave it orphaned.
        let mut root = parse(PBXPROJ).unwrap();
        let local = find_package(&root, "Dep").unwrap();
        remove_package(&mut root, &local, &["Dep".to_string()]).unwrap();

        let text = serialize(&root, "App");
        assert!(!text.contains("XCLocalSwiftPackageReference"));
        // PRODB (the back-ref-less product dependency named Dep) is gone.
        assert!(!text.contains("productName = Dep"));
        // The remote package and its product are untouched.
        assert!(text.contains("productName = KeychainSwift"));
    }
}
