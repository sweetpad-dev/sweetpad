//! Swift Build's project interchange format (PIF) for an Xcode workspace or
//! project.
//!
//! Swift Build (`swiftlang/swift-build`) never reads a project file. Xcode
//! translates the workspace into PIF before handing it over, and the
//! open-source engine's own `.xcodeproj` support shells out to
//! `xcodebuild -dumpPIF` for that translation. Emitting PIF here is what lets
//! the engine build an unmodified project on a host without Xcode.
//! `xcodebuild -dumpPIF` is the oracle: GUIDs, key spellings and value shapes
//! follow what it prints, and Swift Build's `PIFLoader` decides which keys are
//! required.
//!
//! SwiftPM packages come from SwiftPM itself ([`packages`]): Xcode has its
//! embedded SwiftPM emit each package as a project of its own, and the
//! projects here refer to their products by the GUIDs SwiftPM gives them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value as Json, json};

use crate::catalog_cache;
use crate::pbxproj::{Dict, Value};
use crate::pif_file_types;
use crate::pif_sync::{self, Kind, Node};
use crate::project::{self, Error, Parents};
use crate::spm;
use crate::workspace;
use crate::xcode_hash::{md5_hex, sha256_base64};
use crate::xcspec;

mod packages;
pub use packages::{Package, add_packages, manifest, packages};

/// The PIF `xcodebuild -dumpPIF` prints for a `.xcworkspace` or an
/// `.xcodeproj`: a workspace object, then each project followed by its
/// targets.
///
/// The projects are the workspace's members plus every project they reach
/// through project references, in path order. A bare project stands in a
/// workspace of its own, `X.xcodeproj/project.xcworkspace`.
pub fn pif(container: &Path) -> Result<Vec<Json>, Error> {
    let Opened {
        name,
        path: workspace_path,
        graph,
        ..
    } = Opened::of(container)?;
    let mut projects = Vec::new();
    for file in &graph.projects {
        projects.push(Emitter::new(&graph, file)?.emit()?);
    }
    let workspace = object(
        "workspace",
        json!({
            "guid": md5_hex(name.as_bytes()),
            "name": name,
            "path": workspace_path,
            "projects": projects.iter().map(|(p, _)| p["signature"].clone()).collect::<Vec<_>>(),
        }),
    );
    let mut out = vec![workspace];
    for (project, targets) in projects {
        out.push(project);
        out.extend(targets);
    }
    Ok(out)
}

/// The workspace a container stands for.
struct Opened {
    name: String,
    path: PathBuf,
    graph: Graph,
    /// The package folders the workspace itself holds.
    package_refs: Vec<PathBuf>,
}

impl Opened {
    /// A `.xcworkspace`, or a bare project standing in a workspace of its own,
    /// `X.xcodeproj/project.xcworkspace`.
    fn of(container: &Path) -> Result<Self, Error> {
        let container = project::standardize(container);
        let base_dir = container
            .parent()
            .map_or_else(PathBuf::new, Path::to_path_buf);
        if container.extension().is_some_and(|e| e == "xcworkspace") {
            let ws = workspace::open(&container).map_err(|e| Error::BadProject(e.to_string()))?;
            let roots = ws
                .project_refs
                .iter()
                .map(|p| (project::standardize(p), false))
                .collect();
            Ok(Self {
                name: ws.name,
                path: container.clone(),
                graph: Graph::load(roots, base_dir)?,
                package_refs: ws.package_refs,
            })
        } else {
            Ok(Self {
                name: stem(&container),
                path: container.join("project.xcworkspace"),
                graph: Graph::load(vec![(container.clone(), true)], base_dir)?,
                package_refs: Vec::new(),
            })
        }
    }
}

/// One `.xcodeproj` of the workspace.
struct ProjectFile {
    /// Standardized: symlinks resolved and a leading `/private` dropped.
    path: PathBuf,
    guid: String,
    value: Arc<Value>,
}

/// Every project the PIF covers, and the directory their GUIDs are relative to.
struct Graph {
    base_dir: PathBuf,
    projects: Vec<ProjectFile>,
    /// Xcode's build-setting defaults, for the few values a product type
    /// decides.
    defaults: Option<xcspec::Catalog>,
}

impl Graph {
    /// The projects in `roots` and every project they reference, transitively.
    /// A required root must parse; any other project that isn't there is
    /// skipped, as Xcode shows it missing and carries on.
    fn load(roots: Vec<(PathBuf, bool)>, base_dir: PathBuf) -> Result<Self, Error> {
        let mut queue: VecDeque<(PathBuf, bool)> = roots.into();
        let mut seen = HashSet::new();
        let mut projects = Vec::new();
        while let Some((path, required)) = queue.pop_front() {
            if !seen.insert(path.clone()) {
                continue;
            }
            if !required && !path.join("project.pbxproj").is_file() {
                continue;
            }
            let value = project::parse_pbxproj(&path)?;
            for referenced in referenced_projects(&value, &path) {
                queue.push_back((referenced, false));
            }
            projects.push(ProjectFile {
                guid: project_guid(&path, &base_dir),
                path,
                value,
            });
        }
        projects.sort_by(|a, b| a.path.as_os_str().cmp(b.path.as_os_str()));
        Ok(Self {
            base_dir,
            projects,
            defaults: catalog_cache::embedded().ok(),
        })
    }

    fn get(&self, path: &Path) -> Option<&ProjectFile> {
        self.projects.iter().find(|p| p.path == path)
    }
}

/// Xcode's GUID for a project: the MD5 of its path relative to the
/// workspace's directory, so it survives moving the whole tree.
fn project_guid(xcodeproj: &Path, base_dir: &Path) -> String {
    md5_hex(relative_path(xcodeproj, base_dir).as_bytes())
}

/// The projects a project's `projectReferences` name, standardized.
fn referenced_projects(value: &Value, xcodeproj: &Path) -> Vec<PathBuf> {
    let Ok((objects, project_obj)) = project::project_root(value) else {
        return Vec::new();
    };
    let dir = project_directory(project_obj, xcodeproj);
    let parents = Parents::of(objects);
    project_obj
        .get("projectReferences")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r.get("ProjectRef").and_then(Value::as_str))
        .map(|id| project::standardize(&parents.group_dir(id, &dir)))
        .collect()
}

struct Emitter<'a> {
    graph: &'a Graph,
    file: &'a ProjectFile,
    objects: &'a Dict,
    root_id: &'a str,
    project_obj: &'a Value,
    project_dir: PathBuf,
    parents: Parents<'a>,
    class_prefix: String,
    /// Each product file reference, and the target that builds it.
    producers: HashMap<&'a str, &'a str>,
    /// What each synchronized folder holds.
    synced: HashMap<&'a str, Vec<Node>>,
    /// The synchronized folders in navigator order.
    synced_order: Vec<&'a str>,
}

impl<'a> Emitter<'a> {
    fn new(graph: &'a Graph, file: &'a ProjectFile) -> Result<Self, Error> {
        let (objects, project_obj) = project::project_root(&file.value)?;
        let root_id = file
            .value
            .get("rootObject")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let project_dir = project_directory(project_obj, &file.path);
        let parents = Parents::of(objects);
        let producers = objects
            .iter()
            .filter_map(|(id, o)| Some((o.get("productReference")?.as_str()?, id.as_str())))
            .collect();
        let region = development_region(project_obj);
        let synced: HashMap<&str, Vec<Node>> = objects
            .iter()
            .filter(|(_, o)| isa(o) == "PBXFileSystemSynchronizedRootGroup")
            .filter_map(|(id, o)| {
                let dir = parents.group_dir(id, &project_dir);
                Some((id.as_str(), pif_sync::expand(&dir, o.as_dict()?, region)))
            })
            .collect();
        let mut synced_order = Vec::new();
        if let Some(main_group) = project_obj.get("mainGroup").and_then(Value::as_str) {
            navigator_order(objects, main_group, &mut synced_order, &mut HashSet::new());
        }
        synced_order.retain(|id| synced.contains_key(id));
        let mut unreachable: Vec<&str> = synced
            .keys()
            .copied()
            .filter(|id| !synced_order.contains(id))
            .collect();
        unreachable.sort_unstable();
        synced_order.extend(unreachable);
        Ok(Self {
            graph,
            file,
            objects,
            root_id,
            project_obj,
            project_dir,
            parents,
            class_prefix: str_at(project_obj, "attributes", "CLASSPREFIX").unwrap_or_default(),
            producers,
            synced,
            synced_order,
        })
    }

    /// Xcode's GUID for a pbxproj object: the project's GUID followed by the
    /// MD5 of the object's ID, so a GUID is stable across dumps and unique
    /// across the projects of a workspace.
    fn guid(&self, id: &str) -> String {
        format!("{}{}", self.file.guid, md5_hex(id.as_bytes()))
    }

    /// The GUID of something found inside the synchronized folder `root`:
    /// a base64 SHA-256 over the project GUID, the folder's ID and `rest`.
    fn synced_guid(&self, root: &str, rest: &str) -> String {
        sha256_base64(format!("{}{root}{rest}", self.file.guid).as_bytes())
    }

    fn get(&self, id: &str) -> Option<&'a Dict> {
        self.objects.get(id).and_then(Value::as_dict)
    }

    fn emit(&self) -> Result<(Json, Vec<Json>), Error> {
        let mut targets = Vec::new();
        for id in ids(self.project_obj, "targets") {
            if let Some(target) = self.target(id)? {
                targets.push(target);
            }
        }
        Ok((self.project(&targets)?, targets))
    }

    fn project(&self, targets: &[Json]) -> Result<Json, Error> {
        let main_group = self
            .project_obj
            .get("mainGroup")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::BadProject("project has no mainGroup".into()))?;
        let mut group_tree = self
            .reference(main_group, false)
            .ok_or_else(|| Error::BadProject(format!("mainGroup {main_group} is not a group")))?;
        // The main group always takes the project's name, whatever it's called
        // in the file.
        group_tree["name"] = json!(stem(&self.file.path));
        if let Some(recovered) = self.recovered_references(main_group) {
            match group_tree.get_mut("children").and_then(Json::as_array_mut) {
                Some(children) => children.push(recovered),
                None => group_tree["children"] = json!([recovered]),
            }
        }

        let config_list = self.configuration_list(self.project_obj);
        let contents = json!({
            "appPreferencesBuildSettings": {},
            "buildConfigurations": self.configurations(self.project_obj)?,
            "classPrefix": self.class_prefix,
            "defaultConfigurationName": config_list
                .and_then(|l| l.get("defaultConfigurationName"))
                .and_then(Value::as_str)
                .unwrap_or("Release"),
            "developmentRegion": development_region(self.project_obj),
            "groupTree": group_tree,
            "guid": self.file.guid,
            "knownRegions": match self.project_obj.get("knownRegions") {
                Some(regions) => json!(strings(Some(regions))),
                None => json!(["en", "Base"]),
            },
            "path": self.file.path,
            "projectDirectory": self.project_dir,
            "targets": targets.iter().map(|t| t["signature"].clone()).collect::<Vec<_>>(),
        });
        Ok(object("project", contents))
    }

    fn reference(&self, id: &str, in_variant_group: bool) -> Option<Json> {
        let obj = self.get(id)?;
        let source_tree = obj
            .get("sourceTree")
            .and_then(Value::as_str)
            .unwrap_or("<group>");
        let path = obj.get("path").and_then(Value::as_str);
        let name = obj.get("name").and_then(Value::as_str);
        let mut out = Map::new();
        out.insert("guid".into(), json!(self.guid(id)));
        out.insert("sourceTree".into(), json!(source_tree));
        match isa_of(obj) {
            "PBXFileReference" => {
                // A file is known by its path alone; Xcode drops a display name.
                let path = path.or(name).unwrap_or_default();
                let file_type = file_ref_type(obj, path);
                out.insert("type".into(), json!("file"));
                out.insert("path".into(), json!(path));
                if let Some(encoding) = obj
                    .get("fileEncoding")
                    .and_then(Value::as_str)
                    .filter(|_| pif_file_types::is_text(file_type))
                    .and_then(text_encoding_name)
                {
                    out.insert("fileTextEncoding".into(), json!(encoding));
                }
                if in_variant_group {
                    out.insert("regionVariantName".into(), json!(name.unwrap_or_default()));
                }
                out.insert("fileType".into(), json!(file_type));
            }
            "PBXGroup" => {
                out.insert("type".into(), json!("group"));
                out.insert("name".into(), json!(name.or(path).unwrap_or_default()));
                out.insert("path".into(), json!(path.unwrap_or_default()));
                self.insert_children(obj, false, &mut out);
            }
            "PBXVariantGroup" => {
                out.insert("type".into(), json!("variantGroup"));
                out.insert("name".into(), json!(name.or(path).unwrap_or_default()));
                out.insert("path".into(), json!(path.unwrap_or_default()));
                self.insert_children(obj, true, &mut out);
            }
            "XCVersionGroup" => {
                let path = path.unwrap_or_default();
                out.insert("type".into(), json!("versionGroup"));
                out.insert(
                    "name".into(),
                    json!(path.rsplit('/').next().unwrap_or(path)),
                );
                out.insert("path".into(), json!(path));
                out.insert("fileType".into(), json!("wrapper.xcdatamodeld"));
                self.insert_children(obj, false, &mut out);
            }
            "PBXFileSystemSynchronizedRootGroup" => {
                out.insert("type".into(), json!("group"));
                out.insert("name".into(), json!(name.or(path).unwrap_or_default()));
                out.insert("path".into(), json!(path.unwrap_or_default()));
                let children: Vec<Json> = self
                    .synced
                    .get(id)
                    .map(|nodes| nodes.iter().map(|n| self.synced_node(id, n)).collect())
                    .unwrap_or_default();
                if !children.is_empty() {
                    out.insert("children".into(), Json::Array(children));
                }
            }
            _ => return None,
        }
        Some(Json::Object(out))
    }

    /// A group's children, without the product references Xcode keeps out of
    /// the tree (the target's `productReference` carries them). Xcode leaves
    /// out an empty `children`.
    fn insert_children(&self, group: &Dict, in_variant_group: bool, out: &mut Map<String, Json>) {
        let children: Vec<Json> = ids_in(group, "children")
            .filter(|child| !self.producers.contains_key(child))
            .filter_map(|child| self.reference(child, in_variant_group))
            .collect();
        if !children.is_empty() {
            out.insert("children".into(), Json::Array(children));
        }
    }

    fn synced_node(&self, root: &str, node: &Node) -> Json {
        let mut out = Map::new();
        let guid = match node.kind {
            Kind::VariantGroup => self.synced_guid(root, &format!("{}VariantGroup", node.rel)),
            _ => self.synced_guid(root, &node.rel),
        };
        out.insert("guid".into(), json!(guid));
        out.insert("sourceTree".into(), json!("<group>"));
        out.insert("path".into(), json!(node.path));
        match node.kind {
            Kind::File => {
                out.insert("type".into(), json!("file"));
                out.insert("fileType".into(), json!(node.file_type));
                if let Some(region) = &node.region {
                    out.insert("regionVariantName".into(), json!(region));
                }
            }
            Kind::Group => {
                out.insert("type".into(), json!("group"));
                out.insert("name".into(), json!(node.name));
            }
            Kind::VariantGroup => {
                out.insert("type".into(), json!("variantGroup"));
                out.insert("name".into(), json!(node.name));
            }
            Kind::VersionGroup => {
                out.insert("type".into(), json!("versionGroup"));
                out.insert("name".into(), json!(node.name));
                out.insert("fileType".into(), json!(node.file_type));
            }
        }
        if node.kind != Kind::File && !node.children.is_empty() {
            out.insert(
                "children".into(),
                Json::Array(
                    node.children
                        .iter()
                        .map(|c| self.synced_node(root, c))
                        .collect(),
                ),
            );
        }
        Json::Object(out)
    }

    /// The group Xcode appends to the tree for files that build files use
    /// but no group lists. Build files resolve only against the tree, so
    /// without it they would point at nothing. Xcode gives it a random GUID.
    fn recovered_references(&self, main_group: &str) -> Option<Json> {
        let mut reachable = HashSet::new();
        self.collect_reachable(main_group, &mut reachable, 0);
        let mut orphans: Vec<&str> = Vec::new();
        for target in ids(self.project_obj, "targets").filter_map(|t| self.objects.get(t)) {
            for phase in ids(target, "buildPhases").filter_map(|p| self.get(p)) {
                for file_ref in ids_in(phase, "files")
                    .filter_map(|f| self.get(f))
                    .filter_map(|f| f.get("fileRef").and_then(Value::as_str))
                {
                    let referable = self.get(file_ref).is_some_and(|r| {
                        matches!(
                            isa_of(r),
                            "PBXFileReference" | "PBXVariantGroup" | "XCVersionGroup"
                        )
                    });
                    if referable
                        && !reachable.contains(file_ref)
                        && !self.producers.contains_key(file_ref)
                        && !orphans.contains(&file_ref)
                    {
                        orphans.push(file_ref);
                    }
                }
            }
        }
        if orphans.is_empty() {
            return None;
        }
        Some(json!({
            "children": orphans.iter().filter_map(|id| self.reference(id, false)).collect::<Vec<_>>(),
            "guid": self.guid("Recovered References"),
            "name": "Recovered References",
            "path": "",
            "sourceTree": "<group>",
            "type": "group",
        }))
    }

    fn collect_reachable(&self, id: &'a str, out: &mut HashSet<&'a str>, depth: usize) {
        if depth > 256 || !out.insert(id) {
            return;
        }
        if let Some(group) = self.get(id) {
            for child in ids_in(group, "children") {
                self.collect_reachable(child, out, depth + 1);
            }
        }
    }

    fn configuration_list(&self, owner: &Value) -> Option<&'a Dict> {
        owner
            .get("buildConfigurationList")
            .and_then(Value::as_str)
            .and_then(|id| self.get(id))
    }

    fn configurations(&self, owner: &Value) -> Result<Json, Error> {
        let list = self
            .configuration_list(owner)
            .ok_or_else(|| Error::BadProject("object has no buildConfigurationList".into()))?;
        let mut out = Vec::new();
        for id in ids_in(list, "buildConfigurations") {
            let Some(config) = self.get(id) else { continue };
            let settings: Map<String, Json> = config
                .get("buildSettings")
                .and_then(Value::as_dict)
                .map(|d| {
                    d.iter()
                        .map(|(k, v)| (k.clone(), json!(setting_string(v))))
                        .collect()
                })
                .unwrap_or_default();
            let mut entry = json!({
                "buildSettings": settings,
                "guid": self.guid(id),
                "name": config.get("name").and_then(Value::as_str).unwrap_or_default(),
            });
            if let Some(base) = config
                .get("baseConfigurationReference")
                .and_then(Value::as_str)
            {
                entry["baseConfigurationFileReference"] = json!(self.guid(base));
            } else if let (Some(anchor), Some(rel)) = (
                config
                    .get("baseConfigurationReferenceAnchor")
                    .and_then(Value::as_str),
                config
                    .get("baseConfigurationReferenceRelativePath")
                    .and_then(Value::as_str),
            ) {
                entry["baseConfigurationFileReference"] = json!(self.synced_guid(anchor, rel));
            }
            out.push(entry);
        }
        Ok(Json::Array(out))
    }

    fn target(&self, id: &str) -> Result<Option<Json>, Error> {
        let Some(obj) = self.objects.get(id) else {
            return Ok(None);
        };
        let isa = isa(obj);
        let name = obj.get("name").and_then(Value::as_str).unwrap_or_default();
        let mut c = Map::new();
        c.insert("guid".into(), json!(self.guid(id)));
        c.insert("name".into(), json!(name));
        c.insert("buildConfigurations".into(), self.configurations(obj)?);
        c.insert("dependencies".into(), self.dependencies(obj));
        match isa {
            "PBXNativeTarget" => {
                let product_type = obj
                    .get("productType")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let phases = self.build_phases(id, obj);
                if let Some(language) = predominant_language(&phases, &self.file_types(&phases)) {
                    c.insert("predominantSourceCodeLanguage".into(), json!(language));
                }
                c.insert("type".into(), json!("standard"));
                c.insert("buildPhases".into(), Json::Array(phases));
                c.insert("buildRules".into(), self.build_rules(obj));
                c.insert("productTypeIdentifier".into(), json!(product_type));
                if let Some(product) = obj.get("productReference").and_then(Value::as_str) {
                    let product_name = self
                        .get(product)
                        .and_then(|p| p.get("path").or_else(|| p.get("name")))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    c.insert(
                        "productReference".into(),
                        json!({ "guid": self.guid(product), "name": product_name, "type": "product" }),
                    );
                }
                if matches!(
                    product_type,
                    "com.apple.product-type.bundle.unit-test"
                        | "com.apple.product-type.bundle.ui-testing"
                ) {
                    c.insert(
                        "performanceTestsBaselinesPath".into(),
                        json!(
                            self.file
                                .path
                                .join("xcshareddata/xcbaselines")
                                .join(format!("{id}.xcbaseline"))
                        ),
                    );
                }
                c.insert("provisioningSourceData".into(), self.provisioning(id, obj));
            }
            "PBXAggregateTarget" => {
                c.insert("type".into(), json!("aggregate"));
                c.insert(
                    "buildPhases".into(),
                    Json::Array(self.build_phases(id, obj)),
                );
                c.insert("buildRules".into(), json!([]));
                c.insert("provisioningSourceData".into(), self.provisioning(id, obj));
            }
            "PBXLegacyTarget" => {
                c.insert("type".into(), json!("external"));
                c.insert("toolPath".into(), json!(str_of(obj, "buildToolPath")));
                c.insert(
                    "arguments".into(),
                    json!(str_of(obj, "buildArgumentsString")),
                );
                c.insert(
                    "workingDirectory".into(),
                    json!(str_of(obj, "buildWorkingDirectory")),
                );
                c.insert(
                    "passBuildSettingsInEnvironment".into(),
                    json!(pif_bool(
                        obj.get("passBuildSettingsInEnvironment")
                            .and_then(Value::as_str)
                            != Some("0")
                    )),
                );
            }
            _ => return Ok(None),
        }
        if isa != "PBXLegacyTarget" && !self.class_prefix.is_empty() {
            c.insert("classPrefix".into(), json!(self.class_prefix));
        }
        Ok(Some(object("target", Json::Object(c))))
    }

    /// The target's dependencies on other targets, then one per SwiftPM
    /// product it links.
    fn dependencies(&self, target: &Value) -> Json {
        let mut out = Vec::new();
        for dep in ids(target, "dependencies").filter_map(|id| self.get(id)) {
            let resolved = match dep.get("target").and_then(Value::as_str) {
                Some(target_id) => Some((self.guid(target_id), self.target_name(target_id))),
                None => dep
                    .get("targetProxy")
                    .and_then(Value::as_str)
                    .and_then(|p| self.proxied_target(p)),
            };
            let Some((guid, name)) = resolved else {
                continue;
            };
            let mut entry = json!({ "guid": guid, "name": name });
            if let Some(filters) = platform_filters(dep) {
                entry["platformFilters"] = filters;
            }
            out.push(entry);
        }
        // Products follow the order the Frameworks phases link them in, then
        // the ones nothing links.
        let mut products: Vec<&str> = ids(target, "buildPhases")
            .filter_map(|id| self.get(id))
            .filter(|p| isa_of(p) == "PBXFrameworksBuildPhase")
            .flat_map(|p| ids_in(p, "files"))
            .filter_map(|f| self.get(f)?.get("productRef")?.as_str())
            .filter(|p| ids(target, "packageProductDependencies").any(|d| d == *p))
            .collect();
        for dep_id in ids(target, "packageProductDependencies") {
            if !products.contains(&dep_id) {
                products.push(dep_id);
            }
        }
        for dep_id in products {
            let Some(guid) = self.package_product(dep_id) else {
                continue;
            };
            let name = self
                .get(dep_id)
                .and_then(|d| d.get("productName"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut entry = json!({ "guid": guid, "name": name });
            if let Some(filters) = self
                .linking_build_file(target, dep_id)
                .and_then(platform_filters)
            {
                entry["platformFilters"] = filters;
            }
            out.push(entry);
        }
        Json::Array(out)
    }

    fn target_name(&self, target_id: &str) -> String {
        self.get(target_id)
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    /// The target a `PBXContainerItemProxy` of type 1 names: its GUID and the
    /// name Xcode shows, which spells out the project of a target in another
    /// one (`Lib (from Lib.xcodeproj)`).
    fn proxied_target(&self, proxy_id: &str) -> Option<(String, String)> {
        let proxy = self.get(proxy_id)?;
        let portal = proxy.get("containerPortal").and_then(Value::as_str)?;
        let remote_id = proxy.get("remoteGlobalIDString").and_then(Value::as_str)?;
        if portal == self.root_id {
            return Some((self.guid(remote_id), self.target_name(remote_id)));
        }
        let path = self.portal_path(portal);
        let guid = format!(
            "{}{}",
            project_guid(&path, &self.graph.base_dir),
            md5_hex(remote_id.as_bytes())
        );
        let remote_name = self
            .graph
            .get(&path)
            .and_then(|p| project_object(p, remote_id))
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            .or_else(|| proxy.get("remoteInfo").and_then(Value::as_str))
            .unwrap_or_default();
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Some((guid, format!("{remote_name} (from {file_name})")))
    }

    /// The target in another project that builds the product a
    /// `PBXReferenceProxy` stands for.
    fn proxied_producer(&self, reference_proxy: &Dict) -> Option<String> {
        let proxy = self.get(reference_proxy.get("remoteRef")?.as_str()?)?;
        let portal = proxy.get("containerPortal").and_then(Value::as_str)?;
        let product_id = proxy.get("remoteGlobalIDString").and_then(Value::as_str)?;
        let path = self.portal_path(portal);
        let remote = self.graph.get(&path)?;
        let (objects, _) = project::project_root(&remote.value).ok()?;
        let target_id = objects.iter().find_map(|(id, o)| {
            (o.get("productReference").and_then(Value::as_str) == Some(product_id))
                .then_some(id.as_str())
        })?;
        Some(format!("{}{}", remote.guid, md5_hex(target_id.as_bytes())))
    }

    /// The `.xcodeproj` a container proxy's `containerPortal` file reference
    /// points at.
    fn portal_path(&self, portal: &str) -> PathBuf {
        project::standardize(&self.parents.group_dir(portal, &self.project_dir))
    }

    /// SwiftPM's GUID for the product an `XCSwiftPackageProductDependency`
    /// names: `PACKAGE-PRODUCT:<package identity>_<product>.<product>`.
    fn package_product(&self, dep_id: &str) -> Option<String> {
        let dep = self.get(dep_id)?;
        let product = dep.get("productName").and_then(Value::as_str)?;
        let identity = match dep
            .get("package")
            .and_then(Value::as_str)
            .and_then(|p| self.get(p))
        {
            Some(package) if isa_of(package) == "XCRemoteSwiftPackageReference" => {
                spm::identity_from_url(&str_of_dict(package, "repositoryURL"))
            }
            Some(package) => spm::identity_from_path(&str_of_dict(package, "relativePath")),
            None => self.local_package_identity(product),
        };
        Some(format!("PACKAGE-PRODUCT:{identity}_{product}.{product}"))
    }

    /// The identity of the local package a product with no `package` comes
    /// from: the package folder named like the product, else the one whose
    /// manifest mentions the product's name, else the only one. Knowing for
    /// sure means evaluating the manifests.
    fn local_package_identity(&self, product: &str) -> String {
        let mut folders: Vec<PathBuf> = self
            .objects
            .iter()
            .filter(|(_, o)| isa(o) == "PBXFileReference")
            .map(|(id, _)| self.parents.group_dir(id, &self.project_dir))
            .filter(|dir| dir.join("Package.swift").is_file())
            .collect();
        for (root, nodes) in &self.synced {
            let root_dir = self.parents.group_dir(root, &self.project_dir);
            folders.extend(
                nodes
                    .iter()
                    .flat_map(Node::walk)
                    .filter(|n| n.package)
                    .map(|n| root_dir.join(&n.rel)),
            );
        }
        let name = |dir: &PathBuf| {
            dir.file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default()
        };
        let quoted = format!("\"{product}\"");
        folders
            .iter()
            .find(|dir| name(dir) == product.to_ascii_lowercase())
            .or_else(|| {
                folders.iter().find(|dir| {
                    std::fs::read_to_string(dir.join("Package.swift"))
                        .is_ok_and(|manifest| manifest.contains(&quoted))
                })
            })
            .or(if folders.len() == 1 {
                folders.first()
            } else {
                None
            })
            .map_or_else(|| product.to_ascii_lowercase(), name)
    }

    /// The build file in the target's Frameworks phases that links the
    /// SwiftPM product `dep_id`.
    fn linking_build_file(&self, target: &Value, dep_id: &str) -> Option<&'a Dict> {
        ids(target, "buildPhases")
            .filter_map(|id| self.get(id))
            .filter(|p| isa_of(p) == "PBXFrameworksBuildPhase")
            .flat_map(|p| ids_in(p, "files"))
            .filter_map(|f| self.get(f))
            .find(|f| f.get("productRef").and_then(Value::as_str) == Some(dep_id))
    }

    fn build_phases(&self, target_id: &str, target: &Value) -> Vec<Json> {
        let phase_ids: Vec<&str> = ids(target, "buildPhases").collect();
        let mut synced = self.synced_build_files(target_id, target, &phase_ids);
        phase_ids
            .iter()
            .filter_map(|id| self.build_phase(id, synced.remove(*id).unwrap_or_default()))
            .collect()
    }

    fn build_phase(&self, id: &str, synced: Vec<Json>) -> Option<Json> {
        let phase = self.get(id)?;
        let kind = phase_kind(isa_of(phase))?;
        let mut files: Vec<Json> = ids_in(phase, "files")
            .filter_map(|f| self.build_file(f, kind))
            .collect();
        // Where Xcode puts a synchronized folder's files among the listed ones
        // has no build meaning: after the first in Resources, else at the end.
        let at = if kind == "resources" {
            files.len().min(1)
        } else {
            files.len()
        };
        files.splice(at..at, synced);
        let mut out = Map::new();
        out.insert("guid".into(), json!(self.guid(id)));
        out.insert("type".into(), json!(format!("com.apple.buildphase.{kind}")));
        match kind {
            "shell-script" => {
                out.insert("buildFiles".into(), json!([]));
                shell_script_keys(phase, id, &mut out);
            }
            "copy-files" => {
                out.insert("buildFiles".into(), Json::Array(files));
                copy_files_keys(phase, &mut out);
            }
            _ => {
                out.insert("buildFiles".into(), Json::Array(files));
            }
        }
        Some(Json::Object(out))
    }

    fn build_file(&self, id: &str, phase_kind: &str) -> Option<Json> {
        let file = self.get(id)?;
        let mut out = Map::new();
        if let Some(file_ref) = file.get("fileRef").and_then(Value::as_str) {
            out.insert("guid".into(), json!(self.guid(id)));
            let referenced = self.get(file_ref);
            if let Some(proxy) = referenced.filter(|r| isa_of(r) == "PBXReferenceProxy") {
                out.insert(
                    "targetReference".into(),
                    json!(self.proxied_producer(proxy)?),
                );
            } else if let Some(producer) = self.producers.get(file_ref) {
                out.insert("targetReference".into(), json!(self.guid(producer)));
            } else {
                out.insert("fileReference".into(), json!(self.guid(file_ref)));
                let path = referenced
                    .and_then(|r| r.get("path").or_else(|| r.get("name")))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let file_type = referenced.map_or("", |r| file_ref_type(r, path));
                if phase_kind == "sources" && file_type == "file.intentdefinition" {
                    out.insert("intentsCodegenVisibility".into(), json!("public"));
                }
            }
        } else if let Some(product) = file.get("productRef").and_then(Value::as_str) {
            // SwiftPM product build files take the dependency's ID, unprefixed.
            out.insert("guid".into(), json!(md5_hex(product.as_bytes())));
            out.insert(
                "targetReference".into(),
                json!(self.package_product(product)?),
            );
        } else {
            return None;
        }
        let settings = file.get("settings").and_then(Value::as_dict);
        apply_attributes(
            &strings(settings.and_then(|s| s.get("ATTRIBUTES"))),
            &mut out,
        );
        if let Some(flags) = settings
            .and_then(|s| s.get("COMPILER_FLAGS"))
            .and_then(Value::as_str)
        {
            out.insert("additionalCompilerOptions".into(), json!(flags));
        }
        if let Some(filters) = platform_filters(file) {
            out.insert("platformFilters".into(), filters);
        }
        Some(Json::Object(out))
    }

    /// The build files the target's synchronized folders add, by phase ID.
    ///
    /// Every file in a folder is a member unless the folder's exception set
    /// for this target lists it, and lands in the phase its type defaults
    /// to, if the target has one. A phase exception set adds files to that
    /// phase on top.
    fn synced_build_files(
        &self,
        target_id: &str,
        target: &Value,
        phase_ids: &[&str],
    ) -> HashMap<String, Vec<Json>> {
        let mut out: HashMap<String, Vec<Json>> = HashMap::new();
        let first_phase = |isa_name: &str| -> Option<&str> {
            phase_ids
                .iter()
                .copied()
                .find(|p| self.get(p).is_some_and(|ph| isa_of(ph) == isa_name))
        };
        let own: Vec<&str> = ids(target, "fileSystemSynchronizedGroups").collect();
        // A folder the target doesn't list can still give it files: its
        // exception set for the target then names what joins, not what stays
        // out. Those come after the target's own folders.
        let roots = own.iter().copied().chain(
            self.synced_order
                .iter()
                .copied()
                .filter(|r| !own.contains(r)),
        );
        for root in roots {
            let (Some(root_obj), Some(nodes)) = (self.get(root), self.synced.get(root)) else {
                continue;
            };
            let listed = own.contains(&root);
            let exceptions: Vec<&Dict> = ids_in(root_obj, "exceptions")
                .filter_map(|e| self.get(e))
                .collect();
            let target_set = exceptions.iter().copied().find(|e| {
                isa_of(e) == "PBXFileSystemSynchronizedBuildFileExceptionSet"
                    && e.get("target").and_then(Value::as_str) == Some(target_id)
            });
            if !listed && target_set.is_none() {
                continue;
            }
            let named = target_set
                .map(|e| strings(e.get("membershipExceptions")))
                .unwrap_or_default();
            for node in nodes.iter().flat_map(Node::walk) {
                if node.kind == Kind::Group
                    || node.file_type == "text.plist.entitlements"
                    || is_named(node, &named) == listed
                {
                    continue;
                }
                let phase_isa = default_phase(&node.file_type, node.kind);
                let Some(phase) = first_phase(phase_isa) else {
                    continue;
                };
                let mut file = self.synced_build_file(root, phase, node);
                if let Some(set) = target_set {
                    apply_synced_settings(set, &exception_key(node), &mut file);
                    if phase_isa == "PBXHeadersBuildPhase" {
                        if strings(set.get("publicHeaders")).contains(&node.rel) {
                            file.insert("headerVisibility".into(), json!("public"));
                        } else if strings(set.get("privateHeaders")).contains(&node.rel) {
                            file.insert("headerVisibility".into(), json!("private"));
                        }
                    }
                }
                out.entry(phase.to_string())
                    .or_default()
                    .push(Json::Object(file));
            }
            for set in exceptions.iter().filter(|e| {
                isa_of(e) == "PBXFileSystemSynchronizedGroupBuildPhaseMembershipExceptionSet"
            }) {
                let Some(phase) = set
                    .get("buildPhase")
                    .and_then(Value::as_str)
                    .filter(|p| phase_ids.contains(p))
                else {
                    continue;
                };
                for rel in strings(set.get("membershipExceptions")) {
                    let Some(node) = nodes
                        .iter()
                        .flat_map(Node::walk)
                        .find(|n| exception_key(n) == rel)
                    else {
                        continue;
                    };
                    let mut file = self.synced_build_file(root, phase, node);
                    apply_synced_settings(set, &rel, &mut file);
                    out.entry(phase.to_string())
                        .or_default()
                        .push(Json::Object(file));
                }
            }
        }
        out
    }

    fn synced_build_file(&self, root: &str, phase: &str, node: &Node) -> Map<String, Json> {
        let reference = match node.kind {
            Kind::VariantGroup => self.synced_guid(root, &format!("{}VariantGroup", node.rel)),
            _ => self.synced_guid(root, &node.rel),
        };
        let mut file = Map::new();
        file.insert(
            "guid".into(),
            json!(self.synced_guid(root, &format!("{phase}{}", node.rel))),
        );
        file.insert("fileReference".into(), json!(reference));
        file
    }

    /// The file type of each `fileReference` the phases name.
    fn file_types(&self, phases: &[Json]) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for (id, obj) in self.objects {
            if isa(obj) == "PBXFileReference" {
                let path = obj
                    .get("path")
                    .or_else(|| obj.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(dict) = obj.as_dict() {
                    out.insert(self.guid(id), file_ref_type(dict, path).to_string());
                }
            }
        }
        for (root, nodes) in &self.synced {
            for node in nodes.iter().flat_map(Node::walk) {
                if node.kind == Kind::File {
                    out.insert(self.synced_guid(root, &node.rel), node.file_type.clone());
                }
            }
        }
        let used: HashSet<&str> = phases
            .iter()
            .filter_map(|p| p["buildFiles"].as_array())
            .flatten()
            .filter_map(|f| f["fileReference"].as_str())
            .collect();
        out.retain(|guid, _| used.contains(guid.as_str()));
        out
    }

    fn build_rules(&self, target: &Value) -> Json {
        Json::Array(
            ids(target, "buildRules")
                .filter_map(|id| Some(build_rule(self.get(id)?, &self.guid(id))))
                .collect(),
        )
    }

    /// Per configuration, how the target signs and the bundle identifier its
    /// Info.plist declares. Swift Build reads the style from `CODE_SIGN_STYLE`
    /// whenever that's set, so the fallback matters little.
    fn provisioning(&self, target_id: &str, target: &Value) -> Json {
        let Some(list) = self.configuration_list(target) else {
            return json!([]);
        };
        let project_configs: Vec<&Dict> = self
            .configuration_list(self.project_obj)
            .map(|l| {
                ids_in(l, "buildConfigurations")
                    .filter_map(|id| self.get(id))
                    .collect()
            })
            .unwrap_or_default();
        let attribute_style = self
            .project_obj
            .get("attributes")
            .and_then(|a| a.get("TargetAttributes"))
            .and_then(|t| t.get(target_id))
            .and_then(|t| t.get("ProvisioningStyle"))
            .and_then(Value::as_str);
        let target_name = target
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let product_defaults = self
            .graph
            .defaults
            .as_ref()
            .map(|c| c.layer_for(target.get("productType").and_then(Value::as_str), None))
            .unwrap_or_default();
        let entries: Vec<Json> = ids_in(list, "buildConfigurations")
            .filter_map(|id| self.get(id))
            .map(|config| {
                let name = config
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let project_config = project_configs
                    .iter()
                    .find(|c| c.get("name").and_then(Value::as_str) == Some(name));
                let mut layers = vec![product_defaults.clone()];
                layers.extend(
                    project::build_settings_layers(&self.file.path, target_name, name)
                        .unwrap_or_default(),
                );
                let setting = |key: &str| {
                    project::last_unconditional_setting(&layers, key).or_else(|| {
                        setting_of(config, key)
                            .or_else(|| project_config.and_then(|c| setting_of(c, key)))
                            .map(str::to_string)
                    })
                };
                let style = match setting("CODE_SIGN_STYLE").as_deref().or(attribute_style) {
                    Some("Manual") => 1,
                    _ => 0,
                };
                json!({
                    "bundleIdentifierFromInfoPlist": self.plist_bundle_identifier(
                        setting("INFOPLIST_FILE").as_deref(),
                        setting("GENERATE_INFOPLIST_FILE").as_deref() == Some("YES"),
                    ),
                    "configurationName": name,
                    "provisioningStyle": style,
                })
            })
            .collect();
        Json::Array(entries)
    }

    /// The `CFBundleIdentifier` written in a target's Info.plist, unexpanded.
    /// One Xcode generates (or completes) says `$(PRODUCT_BUNDLE_IDENTIFIER)`;
    /// without either, it is empty.
    fn plist_bundle_identifier(&self, infoplist: Option<&str>, generated: bool) -> String {
        let fallback = || {
            if generated {
                "$(PRODUCT_BUNDLE_IDENTIFIER)".to_string()
            } else {
                String::new()
            }
        };
        let Some(infoplist) = infoplist.filter(|p| !p.is_empty()) else {
            return fallback();
        };
        let relative = [
            "$(SRCROOT)/",
            "$(PROJECT_DIR)/",
            "${SRCROOT}/",
            "${PROJECT_DIR}/",
        ]
        .iter()
        .find_map(|prefix| infoplist.strip_prefix(prefix))
        .unwrap_or(infoplist);
        if relative.contains("$(") || relative.contains("${") {
            return fallback();
        }
        crate::derived_data::plist_strings(&self.project_dir.join(relative))
            .into_iter()
            .find_map(|(key, value)| (key == "CFBundleIdentifier").then_some(value))
            .unwrap_or_else(fallback)
    }
}

/// Every node under `id`, parents first, in navigator order.
fn navigator_order<'v>(
    objects: &'v Dict,
    id: &'v str,
    out: &mut Vec<&'v str>,
    seen: &mut HashSet<&'v str>,
) {
    if seen.len() > 100_000 || !seen.insert(id) {
        return;
    }
    out.push(id);
    let children = objects
        .get(id)
        .and_then(|o| o.get("children"))
        .and_then(Value::as_array)
        .unwrap_or_default();
    for child in children.iter().filter_map(Value::as_str) {
        navigator_order(objects, child, out, seen);
    }
}

/// A pbxproj object in another project of the graph.
fn project_object<'p>(project: &'p ProjectFile, id: &str) -> Option<&'p Value> {
    project::project_root(&project.value)
        .ok()
        .and_then(|(objects, _)| objects.get(id))
}

/// A PIF top-level object. The signature is the content hash: Swift Build
/// caches loaded objects by signature, so it must change whenever the
/// contents do.
fn object(kind: &str, contents: Json) -> Json {
    let hash = md5_hex(contents.to_string().as_bytes());
    let mut out = Map::new();
    out.insert(
        "signature".into(),
        json!(format!("{}@v11_hash={hash}", kind.to_uppercase())),
    );
    out.insert("type".into(), json!(kind));
    out.insert("contents".into(), contents);
    Json::Object(out)
}

/// PIF's name for a build phase `isa`.
fn phase_kind(isa: &str) -> Option<&'static str> {
    Some(match isa {
        "PBXSourcesBuildPhase" => "sources",
        "PBXFrameworksBuildPhase" => "frameworks",
        "PBXHeadersBuildPhase" => "headers",
        "PBXResourcesBuildPhase" => "resources",
        "PBXCopyFilesBuildPhase" => "copy-files",
        "PBXShellScriptBuildPhase" => "shell-script",
        "PBXRezBuildPhase" => "rez",
        _ => return None,
    })
}

/// The phase a synchronized folder's file joins by default: Sources for
/// what compiles (and data models, catalogs and intent definitions), Headers
/// for headers, Resources for the rest.
fn default_phase(file_type: &str, kind: Kind) -> &'static str {
    if kind == Kind::VariantGroup {
        return "PBXResourcesBuildPhase";
    }
    match file_type {
        "sourcecode.c.h" | "sourcecode.cpp.h" => "PBXHeadersBuildPhase",
        "sourcecode.swift"
        | "sourcecode.c.c"
        | "sourcecode.c.objc"
        | "sourcecode.cpp.cpp"
        | "sourcecode.cpp.objcpp"
        | "sourcecode.c.c.preprocessed"
        | "sourcecode.c.objc.preprocessed"
        | "sourcecode.cpp.cpp.preprocessed"
        | "sourcecode.cpp.objcpp.preprocessed"
        | "sourcecode.metal"
        | "sourcecode.applescript"
        | "sourcecode.asm"
        | "sourcecode.asm.asm"
        | "sourcecode.asm.llvm"
        | "sourcecode.nasm"
        | "sourcecode.lex"
        | "sourcecode.yacc"
        | "sourcecode.mig"
        | "sourcecode.iig"
        | "sourcecode.dtrace"
        | "sourcecode.opencl"
        | "wrapper.xcdatamodeld"
        | "wrapper.xcdatamodel"
        | "wrapper.xcmappingmodel"
        | "folder.documentationcatalog"
        | "file.intentdefinition"
        | "file.mlmodel"
        | "folder.mlpackage" => "PBXSourcesBuildPhase",
        _ => "PBXResourcesBuildPhase",
    }
}

/// Whether an exception set's paths name a synchronized node: the node
/// itself, a folder above it, or one of a variant group's translations.
fn is_named(node: &Node, paths: &[String]) -> bool {
    let names = |rel: &str| {
        paths.iter().any(|e| {
            rel == e
                || rel
                    .strip_prefix(e.as_str())
                    .is_some_and(|r| r.starts_with('/'))
        })
    };
    names(&exception_key(node)) || node.children.iter().any(|c| names(&c.rel))
}

/// How an exception set spells a synchronized node's path. A variant group
/// is `/Localized/<folder holding the .lproj folders>/<group name>`.
fn exception_key(node: &Node) -> String {
    if node.kind != Kind::VariantGroup {
        return node.rel.clone();
    }
    let lproj_dir = node.rel.rsplit_once('/').map_or("", |(dir, _)| dir);
    match lproj_dir.rsplit_once('/') {
        Some((dir, _)) => format!("/Localized/{dir}/{}", node.name),
        None => format!("/Localized/{}", node.name),
    }
}

/// Per-file settings an exception set gives a synchronized file, spelled
/// like a build file's own.
fn apply_synced_settings(set: &Dict, rel: &str, file: &mut Map<String, Json>) {
    let by_path = |key: &str| {
        set.get(key)
            .and_then(Value::as_dict)
            .and_then(|d| d.get(rel))
    };
    if let Some(flags) = by_path("additionalCompilerFlagsByRelativePath").and_then(Value::as_str) {
        file.insert("additionalCompilerOptions".into(), json!(flags));
    }
    if let Some(attributes) = by_path("attributesByRelativePath") {
        apply_attributes(&strings(Some(attributes)), file);
    }
    if let Some(filters) = by_path("platformFiltersByRelativePath")
        .map(|f| strings(Some(f)))
        .and_then(|names| filters_json(&names))
    {
        file.insert("platformFilters".into(), filters);
    }
}

/// A build file's `ATTRIBUTES` as PIF flags. Xcode writes only the ones
/// that are set, and leaves a project header's visibility out.
fn apply_attributes(attributes: &[String], out: &mut Map<String, Json>) {
    for attribute in attributes {
        match attribute.as_str() {
            "Public" | "Private" => {
                out.insert("headerVisibility".into(), json!(attribute.to_lowercase()));
            }
            "Weak" => {
                out.insert("shouldLinkWeakly".into(), json!("true"));
            }
            "CodeSignOnCopy" => {
                out.insert("codeSignOnCopy".into(), json!("true"));
            }
            "RemoveHeadersOnCopy" => {
                out.insert("removeHeadersOnCopy".into(), json!("true"));
            }
            _ => {}
        }
    }
}

/// An object's `platformFilter`/`platformFilters` as PIF platform filters.
fn platform_filters(obj: &Dict) -> Option<Json> {
    let mut names = strings(obj.get("platformFilters"));
    names.extend(strings(obj.get("platformFilter")));
    filters_json(&names)
}

/// Platform names as PIF platform filters, sorted by environment, then
/// platform. Mac Catalyst is iOS in the `maccatalyst` environment.
fn filters_json(names: &[String]) -> Option<Json> {
    if names.is_empty() {
        return None;
    }
    let mut pairs: Vec<(&str, &str)> = names
        .iter()
        .map(|n| match n.as_str() {
            "maccatalyst" => ("maccatalyst", "ios"),
            other => ("", other),
        })
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    Some(Json::Array(
        pairs
            .into_iter()
            .map(|(environment, platform)| {
                if environment.is_empty() {
                    json!({ "platform": platform })
                } else {
                    json!({ "environment": environment, "platform": platform })
                }
            })
            .collect(),
    ))
}

fn build_rule(rule: &Dict, guid: &str) -> Json {
    let file_type = str_of_dict(rule, "fileType");
    let compiler = str_of_dict(rule, "compilerSpec");
    let patterns = rule.get("filePatterns").and_then(Value::as_str);
    let name = rule.get("name").and_then(Value::as_str).map_or_else(
        || {
            let using = if compiler == "com.apple.compilers.proxy.script" {
                "Script"
            } else {
                compiler.as_str()
            };
            match patterns {
                Some(p) => format!("Files '{p}' using {using}"),
                None => format!("{file_type} using {using}"),
            }
        },
        str::to_string,
    );
    let mut out = Map::new();
    out.insert("guid".into(), json!(guid));
    out.insert("name".into(), json!(name));
    out.insert("fileTypeIdentifier".into(), json!(file_type));
    out.insert("compilerSpecificationIdentifier".into(), json!(compiler));
    out.insert(
        "inputFilePaths".into(),
        json!(strings(rule.get("inputFiles"))),
    );
    out.insert(
        "outputFilePaths".into(),
        json!(strings(rule.get("outputFiles"))),
    );
    // Xcode leaves a rule's file lists out of the PIF.
    out.insert("inputFileListPaths".into(), json!([]));
    out.insert("outputFileListPaths".into(), json!([]));
    out.insert(
        "runOncePerArchitecture".into(),
        json!(flag(rule, "runOncePerArchitecture", true)),
    );
    if let Some(patterns) = patterns {
        out.insert("filePatterns".into(), json!(patterns));
    }
    if let Some(script) = rule.get("script").and_then(Value::as_str) {
        out.insert("scriptContents".into(), json!(script));
    }
    if let Some(flags) = rule.get("outputFilesCompilerFlags") {
        let split: Vec<Vec<&str>> = match flags {
            Value::Array(items) => items
                .iter()
                .filter_map(Value::as_str)
                .map(|f| f.split_whitespace().collect())
                .collect(),
            _ => Vec::new(),
        };
        out.insert("outputFilesCompilerFlags".into(), json!(split));
    }
    dependency_file_keys(rule, &mut out);
    Json::Object(out)
}

/// The keys a copy-files phase adds: where its files land.
fn copy_files_keys(phase: &Dict, out: &mut Map<String, Json>) {
    let destination = match phase.get("dstSubfolder").and_then(Value::as_str) {
        Some(name) => copy_destination_named(name),
        None => copy_destination(
            phase
                .get("dstSubfolderSpec")
                .and_then(Value::as_str)
                .unwrap_or("16"),
        ),
    };
    out.insert("destinationSubfolder".into(), json!(destination));
    out.insert(
        "destinationSubpath".into(),
        json!(str_of_dict(phase, "dstPath")),
    );
    install_only_key(phase, out);
}

/// `runOnlyForDeploymentPostprocessing`, which Xcode writes only when set.
fn install_only_key(phase: &Dict, out: &mut Map<String, Json>) {
    if flag(phase, "runOnlyForDeploymentPostprocessing", false) == "true" {
        out.insert("runOnlyForDeploymentPostprocessing".into(), json!("true"));
    }
}

/// The keys a run-script phase adds: the script, its shell, and the inputs
/// and outputs dependency analysis reads.
fn shell_script_keys(phase: &Dict, id: &str, out: &mut Map<String, Json>) {
    let text =
        |key: &str, default: &str| json!(phase.get(key).and_then(Value::as_str).unwrap_or(default));
    out.insert("name".into(), text("name", "Run Script"));
    out.insert("shellPath".into(), text("shellPath", "/bin/sh"));
    out.insert("scriptContents".into(), text("shellScript", ""));
    out.insert("originalObjectID".into(), json!(id));
    for (key, pif_key) in [
        ("inputPaths", "inputFilePaths"),
        ("outputPaths", "outputFilePaths"),
        ("inputFileListPaths", "inputFileListPaths"),
        ("outputFileListPaths", "outputFileListPaths"),
    ] {
        out.insert(pif_key.into(), json!(strings(phase.get(key))));
    }
    out.insert(
        "emitEnvironment".into(),
        json!(flag(phase, "showEnvVarsInLog", true)),
    );
    out.insert(
        "alwaysOutOfDate".into(),
        json!(flag(phase, "alwaysOutOfDate", false)),
    );
    out.insert(
        "alwaysRunForInstallHdrs".into(),
        json!(flag(phase, "alwaysRunForInstallHdrs", false)),
    );
    out.insert("sandboxingOverride".into(), json!("basedOnBuildSetting"));
    install_only_key(phase, out);
    dependency_file_keys(phase, out);
}

/// A `dependencyFile` as the makefile-format dependency info PIF names.
fn dependency_file_keys(obj: &Dict, out: &mut Map<String, Json>) {
    if let Some(file) = obj.get("dependencyFile").and_then(Value::as_str) {
        out.insert("dependencyFileFormat".into(), json!("makefile"));
        out.insert("dependencyFilePaths".into(), json!([file]));
    }
}

/// A pbxproj `0`/`1` flag as PIF's `"false"`/`"true"`, `default` when absent.
fn flag(obj: &Dict, key: &str, default: bool) -> &'static str {
    let value = match obj.get(key).and_then(Value::as_str) {
        Some("1") => true,
        Some("0") => false,
        _ => default,
    };
    pif_bool(value)
}

/// The directory `<group>` and `SOURCE_ROOT` paths resolve against:
/// the `.xcodeproj`'s parent, moved by the project's `projectDirPath`.
fn project_directory(project_obj: &Value, xcodeproj: &Path) -> PathBuf {
    let parent = xcodeproj
        .parent()
        .map_or_else(PathBuf::new, Path::to_path_buf);
    match project_obj.get("projectDirPath").and_then(Value::as_str) {
        Some(dir) if !dir.is_empty() => project::standardize(&parent.join(dir)),
        _ => parent,
    }
}

fn development_region(project_obj: &Value) -> &str {
    project_obj
        .get("developmentRegion")
        .and_then(Value::as_str)
        .unwrap_or("en")
}

/// A file reference's type: what the project says, else what Xcode would
/// detect from the name. No extension reads as text, an unknown one as a
/// plain file.
fn file_ref_type<'v>(obj: &'v Dict, path: &str) -> &'v str {
    if let Some(declared) = obj
        .get("explicitFileType")
        .or_else(|| obj.get("lastKnownFileType"))
        .and_then(Value::as_str)
    {
        return declared;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.strip_prefix('.').unwrap_or(name).rsplit_once('.') {
        None => "text",
        Some((_, ext)) => pif_file_types::for_extension(ext).unwrap_or("file"),
    }
}

/// The language Xcode reports for a target: the one most of its files are
/// written in, counted across every phase. Xcode's tie-break is unknown.
fn predominant_language(
    phases: &[Json],
    file_types: &HashMap<String, String>,
) -> Option<&'static str> {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for file_ref in phases
        .iter()
        .filter_map(|p| p["buildFiles"].as_array())
        .flatten()
        .filter_map(|f| f["fileReference"].as_str())
    {
        let language = match file_types.get(file_ref).map(String::as_str) {
            Some("sourcecode.swift") => "Swift",
            Some("sourcecode.c.c") => "C",
            Some("sourcecode.c.objc") => "Objective-C",
            Some("sourcecode.cpp.cpp") => "C-Plus-Plus",
            Some("sourcecode.cpp.objcpp") => "Objective-C-Plus-Plus",
            Some("sourcecode.javascript") => "JavaScript",
            _ => continue,
        };
        match counts.iter_mut().find(|(l, _)| *l == language) {
            Some((_, n)) => *n += 1,
            None => counts.push((language, 1)),
        }
    }
    let best = counts
        .iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))?;
    Some(match best.0 {
        "Swift" => "Xcode.SourceCodeLanguage.Swift",
        "C" => "Xcode.SourceCodeLanguage.C",
        "Objective-C" => "Xcode.SourceCodeLanguage.Objective-C",
        "C-Plus-Plus" => "Xcode.SourceCodeLanguage.C-Plus-Plus",
        "Objective-C-Plus-Plus" => "Xcode.SourceCodeLanguage.Objective-C-Plus-Plus",
        _ => "Xcode.SourceCodeLanguage.JavaScript",
    })
}

/// A pbxproj setting as the single string PIF stores. A list setting joins
/// with single spaces as written, without quoting: Xcode doesn't either, so an
/// element with a space of its own splits into two words.
fn setting_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        Value::Dict(_) => String::new(),
    }
}

fn setting_of<'v>(config: &'v Dict, key: &str) -> Option<&'v str> {
    config
        .get("buildSettings")
        .and_then(|s| s.get(key))
        .and_then(Value::as_str)
}

/// PIF's spelling of a copy-files phase's `dstSubfolderSpec`.
fn copy_destination(spec: &str) -> &'static str {
    match spec {
        "0" => "",
        "1" => "$(WRAPPER_NAME)",
        "2" => "$(INFOPLIST_PATH)",
        "3" => "$(PKGINFO_PATH)",
        "4" | "17" => "$(EXECUTABLE_PATH)",
        "5" => "$(CONTENTS_FOLDER_PATH)",
        "6" => "$(EXECUTABLE_FOLDER_PATH)",
        "7" => "$(UNLOCALIZED_RESOURCES_FOLDER_PATH)",
        "8" => "$(PUBLIC_HEADERS_FOLDER_PATH)",
        "9" => "$(PRIVATE_HEADERS_FOLDER_PATH)",
        "10" => "$(FRAMEWORKS_FOLDER_PATH)",
        "11" => "$(SHARED_FRAMEWORKS_FOLDER_PATH)",
        "12" => "$(SHARED_SUPPORT_FOLDER_PATH)",
        "13" => "$(PLUGINS_FOLDER_PATH)",
        "14" => "$(SCRIPTS_FOLDER_PATH)",
        "15" => "$(JAVA_FOLDER_PATH)",
        _ => "<builtProductsDir>",
    }
}

/// PIF's spelling of the newer, named `dstSubfolder`.
fn copy_destination_named(name: &str) -> &'static str {
    match name {
        "None" => "",
        "Wrapper" => "$(WRAPPER_NAME)",
        "InfoPlist" => "$(INFOPLIST_PATH)",
        "PkgInfo" => "$(PKGINFO_PATH)",
        "MainExecutable" => "$(EXECUTABLE_PATH)",
        "Contents" => "$(CONTENTS_FOLDER_PATH)",
        "Executables" => "$(EXECUTABLE_FOLDER_PATH)",
        "Resources" => "$(UNLOCALIZED_RESOURCES_FOLDER_PATH)",
        "Frameworks" => "$(FRAMEWORKS_FOLDER_PATH)",
        "SharedFrameworks" => "$(SHARED_FRAMEWORKS_FOLDER_PATH)",
        "SharedSupport" => "$(SHARED_SUPPORT_FOLDER_PATH)",
        "PlugIns" => "$(PLUGINS_FOLDER_PATH)",
        "AppleScripts" => "$(SCRIPTS_FOLDER_PATH)",
        "Java" => "$(JAVA_FOLDER_PATH)",
        _ => "<builtProductsDir>",
    }
}

/// The IANA name PIF uses for a pbxproj `fileEncoding`, an `NSStringEncoding`
/// number, as `CFStringConvertEncodingToIANACharSetName` spells it. `None` for
/// an encoding without a name, which Xcode leaves out.
fn text_encoding_name(encoding: &str) -> Option<&'static str> {
    Some(match encoding.parse::<u64>().ok()? {
        1 => "us-ascii",
        2 => "x-nextstep",
        3 => "euc-jp",
        4 => "utf-8",
        5 => "iso-8859-1",
        6 => "x-mac-symbol",
        8 => "cp932",
        9 => "iso-8859-2",
        10 => "utf-16",
        11 => "windows-1251",
        12 => "windows-1252",
        13 => "windows-1253",
        14 => "windows-1254",
        15 => "windows-1250",
        21 => "iso-2022-jp",
        30 => "macintosh",
        0x8000_0001 => "x-mac-japanese",
        0x9000_0100 => "utf-16be",
        0x9400_0100 => "utf-16le",
        0x8c00_0100 => "utf-32",
        0x9800_0100 => "utf-32be",
        0x9c00_0100 => "utf-32le",
        _ => return None,
    })
}

/// `path` relative to `base`, `/`-joined, climbing with `..` where `base`
/// isn't an ancestor. Both are absolute.
fn relative_path(path: &Path, base: &Path) -> String {
    let path: Vec<_> = path.components().collect();
    let base: Vec<_> = base.components().collect();
    let common = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); base.len() - common];
    parts.extend(
        path[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

fn pif_bool(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn isa(obj: &Value) -> &str {
    obj.get("isa").and_then(Value::as_str).unwrap_or_default()
}

fn isa_of(obj: &Dict) -> &str {
    obj.get("isa").and_then(Value::as_str).unwrap_or_default()
}

fn ids<'v>(obj: &'v Value, key: &str) -> impl Iterator<Item = &'v str> {
    obj.get(key)
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
}

fn ids_in<'v>(obj: &'v Dict, key: &str) -> impl Iterator<Item = &'v str> {
    obj.get(key)
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn str_of(obj: &Value, key: &str) -> String {
    obj.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn str_of_dict(obj: &Dict, key: &str) -> String {
    obj.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn str_at(obj: &Value, outer: &str, inner: &str) -> Option<String> {
    obj.get(outer)
        .and_then(|o| o.get(inner))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}
