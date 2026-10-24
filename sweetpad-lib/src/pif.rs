//! Local Swift packages as the PIF describes them, so their sources resolve
//! the way Xcode builds them (#343).
//!
//! Xcode has its embedded SwiftPM turn each package into a PIF project and
//! builds that. `swift package dump-pif` prints the same projects, so a
//! package's targets, sources and settings come from there rather than from a
//! `.xcodeproj`: [`root_manifest`] stands in for the workspace, a caller that
//! can spawn the toolchain dumps its PIF, and [`parse`] keeps the projects of
//! the local packages. Spawning belongs to `sweetpad-core`; this module only
//! reads.
//!
//! A package is addressed by its `Package.swift`, which stands where a
//! `.xcodeproj` path does everywhere else: [`register`] files the parsed
//! packages under the container whose build they belong to, and
//! [`crate::project::Document::parse`] finds one there. `PROJECT_DIR` is then
//! the package directory, as it is in Xcode.
//!
//! Each package has a Debug and a Release configuration, and Xcode builds the
//! packages with Debug settings when the app's configuration name contains
//! `debug` in any case (`Debug`, `Debug-UAT`, `UAT-Debug`) and with Release
//! settings otherwise (`UAT`, `Beta Release`), measured on Xcode 27.0. The
//! build directories keep the app's configuration name (`UAT-iphonesimulator`),
//! so only the settings layers follow [`package_configuration`].

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value as Json;

use crate::project::{self, BuildSettingsContext, Error, Project, Target as ProjectTarget};
use crate::xcconfig::{Assignment, Condition};

/// One local package's PIF project.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    /// The project name, which is the package's name.
    pub name: String,
    /// The package directory.
    pub dir: PathBuf,
    /// Settings per configuration name, `Debug` and `Release`.
    configurations: Vec<(String, Vec<Assignment>)>,
    default_configuration: Option<String>,
    /// The targets a build compiles, in PIF order: no tests, no dynamic
    /// variants, no product aggregates.
    pub targets: Vec<Target>,
    /// Each product's name and the targets it builds.
    pub products: Vec<(String, Vec<String>)>,
}

/// A compiled target of a [`Package`].
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub name: String,
    /// `org.swift.product-type.library.static` and the like.
    pub product_type: String,
    configurations: Vec<(String, Vec<Assignment>)>,
    /// What the targets it depends on impart to it, per configuration: the
    /// module maps and header paths of the C targets among them, remote ones
    /// included.
    imparted: Vec<(String, Vec<Assignment>)>,
    /// Absolute paths of the files its sources phase compiles.
    pub sources: Vec<PathBuf>,
    /// The local package targets it depends on directly, through a product or
    /// not. Targets of remote packages are left out.
    pub dependencies: Vec<String>,
}

impl Package {
    /// The `Package.swift` that stands for this package's project path.
    #[must_use]
    pub fn manifest(&self) -> PathBuf {
        self.dir.join("Package.swift")
    }

    fn target(&self, name: &str) -> Result<&Target, Error> {
        self.targets
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| Error::NoSuchTarget(name.to_string()))
    }

    /// The package as a [`Project`]: its targets and its two configurations.
    #[must_use]
    pub fn open(&self) -> Project {
        let configurations: Vec<String> =
            self.configurations.iter().map(|(n, _)| n.clone()).collect();
        Project {
            name: self.name.clone(),
            path: self.manifest(),
            targets: self
                .targets
                .iter()
                .map(|t| ProjectTarget {
                    name: t.name.clone(),
                    isa: "PBXNativeTarget".to_string(),
                    product_type: Some(t.product_type.clone()),
                    configurations: configurations.clone(),
                })
                .collect(),
            configurations,
            default_configuration: self.default_configuration.clone(),
            schemes: Vec::new(),
            package_refs: Vec::new(),
        }
    }

    /// The settings layers of `target` for the app configuration `config`:
    /// the package's, what its dependencies impart, then the target's, each
    /// in the configuration [`package_configuration`] picks. `PROJECT_NAME` is the package's name,
    /// which the `Package.swift` path does not spell.
    pub fn build_settings(
        &self,
        target: &str,
        config: &str,
    ) -> Result<BuildSettingsContext, Error> {
        let target = self.target(target)?;
        let mut project_layer = vec![Assignment {
            key: "PROJECT_NAME".to_string(),
            conditions: Vec::new(),
            value: self.name.clone(),
            condition: None,
        }];
        project_layer.extend(pick(&self.configurations, config));
        Ok(BuildSettingsContext {
            layers: vec![
                Vec::new(),
                project_layer,
                pick(&target.imparted, config),
                pick(&target.configurations, config),
            ],
            product_type: Some(target.product_type.clone()),
            target_isa: "PBXNativeTarget".to_string(),
            has_package_product_dependencies: false,
            test_host_target: None,
            development_region: None,
        })
    }

    pub fn source_files(&self, target: &str) -> Result<Vec<PathBuf>, Error> {
        Ok(self.target(target)?.sources.clone())
    }

    pub fn dependencies(&self, target: &str) -> Result<Vec<String>, Error> {
        Ok(self.target(target)?.dependencies.clone())
    }
}

/// The package configuration Xcode builds with for the app configuration
/// `config`: `Debug` when its name contains `debug` in any case, else
/// `Release`.
#[must_use]
pub fn package_configuration(config: &str) -> &'static str {
    if config.to_ascii_lowercase().contains("debug") {
        "Debug"
    } else {
        "Release"
    }
}

fn pick(configurations: &[(String, Vec<Assignment>)], config: &str) -> Vec<Assignment> {
    let wanted = package_configuration(config);
    configurations
        .iter()
        .find(|(name, _)| name == wanted)
        .map(|(_, settings)| settings.clone())
        .unwrap_or_default()
}

/// A root `Package.swift` that depends on the local packages in `dirs`, for
/// `swift package dump-pif` to describe them the way Xcode's SwiftPM does
/// for the workspace. The packages they depend on in turn come along.
#[must_use]
pub fn root_manifest(dirs: &[PathBuf]) -> String {
    let mut out = String::from(
        "// swift-tools-version:5.9\n\
         // The local Swift packages an Xcode workspace uses, for SweetPad's build server.\n\
         import PackageDescription\n\n\
         let package = Package(\n    name: \"SweetPadPackages\",\n    dependencies: [\n",
    );
    for dir in dirs {
        let _ = writeln!(
            out,
            "        .package(path: {}),",
            literal(&dir.to_string_lossy())
        );
    }
    out.push_str("    ]\n)\n");
    out
}

/// `text` as a Swift string literal.
fn literal(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The local packages in `pif`, the objects `swift package dump-pif` printed:
/// every package whose directory is outside `skip`, the root manifest's
/// directory and the scratch path holding the remote checkouts. Edges to the
/// targets of skipped packages are dropped.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn parse(pif: &[Json], skip: &[PathBuf]) -> Vec<Package> {
    let skip: Vec<PathBuf> = skip.iter().map(|d| project::standardize(d)).collect();
    let targets: HashMap<&str, &Json> = pif
        .iter()
        .filter(|o| o["type"] == "target")
        .filter_map(|o| Some((o["signature"].as_str()?, &o["contents"])))
        .collect();
    let by_guid: HashMap<&str, &Json> = targets
        .values()
        .filter_map(|t| Some((t["guid"].as_str()?, *t)))
        .collect();
    let dynamic: HashSet<&str> = targets
        .values()
        .filter_map(|t| t["dynamicTargetVariantGuid"].as_str())
        .collect();
    // A target is kept when it belongs to a local package and is compiled
    // into the app: a library or executable, not a test bundle.
    let mut kept: HashMap<&str, String> = HashMap::new();
    let mut packages = Vec::new();
    for object in pif.iter().filter(|o| o["type"] == "project") {
        let project = &object["contents"];
        let Some(guid) = project["guid"].as_str() else {
            continue;
        };
        let dir = PathBuf::from(project["projectDirectory"].as_str().unwrap_or_default());
        let standardized = project::standardize(&dir);
        if !guid.starts_with("PACKAGE:") || skip.iter().any(|s| standardized.starts_with(s)) {
            continue;
        }
        let files = file_paths(&project["groupTree"], &dir);
        let mut package = Package {
            name: project["projectName"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            configurations: configurations(&project["buildConfigurations"]),
            default_configuration: project["defaultConfigurationName"]
                .as_str()
                .map(str::to_string),
            dir,
            targets: Vec::new(),
            products: Vec::new(),
        };
        for target in project["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| targets.get(s.as_str()?))
        {
            let target_guid = target["guid"].as_str().unwrap_or_default();
            let name = target["name"].as_str().unwrap_or_default().to_string();
            if target["type"] == "packageProduct" {
                let product = name.strip_suffix("-product").unwrap_or(&name).to_string();
                package.products.push((product, Vec::new()));
                continue;
            }
            let Some(product_type) = target["productTypeIdentifier"].as_str() else {
                continue;
            };
            if dynamic.contains(target_guid) || product_type.contains("test") {
                continue;
            }
            kept.insert(target_guid, name.clone());
            package.targets.push(Target {
                name,
                product_type: product_type.to_string(),
                configurations: configurations(&target["buildConfigurations"]),
                imparted: Vec::new(),
                sources: sources(target, &files),
                dependencies: Vec::new(),
            });
        }
        packages.push((project, package));
    }
    // Edges resolve once every local target is known: a product stands for
    // the targets it builds, and a target outside the local packages drops.
    let resolve = |guid: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut queue = vec![guid];
        let mut seen = HashSet::new();
        while let Some(guid) = queue.pop() {
            if !seen.insert(guid) {
                continue;
            }
            if let Some(name) = kept.get(guid) {
                out.push(name.clone());
            } else if let Some(product) =
                by_guid.get(guid).filter(|t| t["type"] == "packageProduct")
            {
                queue.extend(guids(&product["dependencies"]).into_iter().rev());
            }
        }
        out
    };
    packages
        .into_iter()
        .map(|(project, mut package)| {
            for target in project["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| targets.get(s.as_str()?))
            {
                let name = target["name"].as_str().unwrap_or_default();
                let edges: Vec<String> = guids(&target["dependencies"])
                    .into_iter()
                    .flat_map(&resolve)
                    .collect();
                if target["type"] == "packageProduct" {
                    let product = name.strip_suffix("-product").unwrap_or(name);
                    if let Some((_, built)) =
                        package.products.iter_mut().find(|(p, _)| p == product)
                    {
                        *built = edges;
                    }
                } else if kept.contains_key(target["guid"].as_str().unwrap_or_default())
                    && let Some(kept) = package.targets.iter_mut().find(|t| t.name == name)
                {
                    kept.dependencies = edges;
                    kept.imparted = imparted(target, &by_guid);
                }
            }
            package.products.retain(|(_, built)| !built.is_empty());
            package
        })
        .collect()
}

/// The settings every target `target` depends on imparts to it, directly or
/// through other targets and products, merged per configuration in
/// dependency order.
fn imparted(target: &Json, by_guid: &HashMap<&str, &Json>) -> Vec<(String, Vec<Assignment>)> {
    let mut out: Vec<(String, Vec<Assignment>)> = Vec::new();
    let mut queue: Vec<&str> = guids(&target["dependencies"]);
    queue.reverse();
    let mut seen = HashSet::new();
    while let Some(guid) = queue.pop() {
        if !seen.insert(guid) {
            continue;
        }
        let Some(dependency) = by_guid.get(guid) else {
            continue;
        };
        for config in dependency["buildConfigurations"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let Some(name) = config["name"].as_str() else {
                continue;
            };
            let Some(settings) = config["impartedBuildProperties"]["buildSettings"].as_object()
            else {
                continue;
            };
            if !out.iter().any(|(n, _)| n == name) {
                out.push((name.to_string(), Vec::new()));
            }
            if let Some((_, merged)) = out.iter_mut().find(|(n, _)| n == name) {
                merged.extend(settings.iter().flat_map(|(k, v)| assignments(k, v)));
            }
        }
        let mut next = guids(&dependency["dependencies"]);
        next.reverse();
        queue.extend(next);
    }
    out
}

fn guids(dependencies: &Json) -> Vec<&str> {
    dependencies
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["guid"].as_str())
        .collect()
}

/// Every file reference in a group tree, by GUID, as an absolute path.
fn file_paths(tree: &Json, dir: &Path) -> HashMap<String, PathBuf> {
    fn visit(node: &Json, parent: &Path, dir: &Path, out: &mut HashMap<String, PathBuf>) {
        let path = node["path"].as_str().unwrap_or_default();
        let here = match node["sourceTree"].as_str() {
            Some("<absolute>") => PathBuf::from(path),
            Some("SOURCE_ROOT") => dir.join(path),
            _ => parent.join(path),
        };
        if node["type"] == "file"
            && let Some(guid) = node["guid"].as_str()
        {
            out.insert(guid.to_string(), here.clone());
        }
        for child in node["children"].as_array().into_iter().flatten() {
            visit(child, &here, dir, out);
        }
    }
    let mut out = HashMap::new();
    visit(tree, dir, dir, &mut out);
    out
}

fn sources(target: &Json, files: &HashMap<String, PathBuf>) -> Vec<PathBuf> {
    target["buildPhases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|phase| phase["type"] == "com.apple.buildphase.sources")
        .flat_map(|phase| phase["buildFiles"].as_array().into_iter().flatten())
        .filter_map(|file| files.get(file["fileReference"].as_str()?).cloned())
        .collect()
}

fn configurations(list: &Json) -> Vec<(String, Vec<Assignment>)> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let name = c["name"].as_str()?.to_string();
            let settings = c["buildSettings"].as_object()?;
            Some((
                name,
                settings
                    .iter()
                    .flat_map(|(k, v)| assignments(k, v))
                    .collect(),
            ))
        })
        .collect()
}

/// One PIF setting as assignments. `SDKROOT` and `SDK_VARIANT` set to `auto`
/// drop: Xcode builds a package for the target that uses it, so the SDK the
/// query resolves for stands. A `[__platform_filter=ios;ios-simulator]`
/// condition becomes one `[sdk=…*]` assignment per SDK it names, since the
/// resolver matches SDKs, not PIF platform filters; Mac Catalyst has no SDK
/// of its own and drops.
fn assignments(key: &str, value: &Json) -> Vec<Assignment> {
    let value = match value {
        Json::String(s) => s.clone(),
        Json::Array(items) => items
            .iter()
            .filter_map(Json::as_str)
            .map(|s| {
                if s.contains(char::is_whitespace) {
                    format!("\"{s}\"")
                } else {
                    s.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => return Vec::new(),
    };
    let (key, conditions) = project::split_conditional_key(key);
    if matches!(key.as_str(), "SDKROOT" | "SDK_VARIANT") && value == "auto" {
        return Vec::new();
    }
    let (filters, mut conditions): (Vec<Condition>, Vec<Condition>) = conditions
        .into_iter()
        .partition(|c| c.key == "__platform_filter");
    let Some(filter) = filters.first() else {
        return vec![Assignment {
            key,
            conditions,
            value,
            condition: None,
        }];
    };
    conditions.push(Condition {
        key: "sdk".to_string(),
        value: String::new(),
    });
    filter
        .value
        .split(';')
        .filter_map(platform_sdk)
        .map(|sdk| {
            let mut conditions = conditions.clone();
            if let Some(last) = conditions.last_mut() {
                last.value = format!("{sdk}*");
            }
            Assignment {
                key: key.clone(),
                conditions,
                value: value.clone(),
                condition: None,
            }
        })
        .collect()
}

/// The SDK a PIF platform filter names.
fn platform_sdk(filter: &str) -> Option<&'static str> {
    Some(match filter {
        "macos" => "macosx",
        "ios" => "iphoneos",
        "ios-simulator" => "iphonesimulator",
        "tvos" => "appletvos",
        "tvos-simulator" => "appletvsimulator",
        "watchos" => "watchos",
        "watchos-simulator" => "watchsimulator",
        "xros" => "xros",
        "xros-simulator" => "xrsimulator",
        "driverkit" => "driverkit",
        _ => return None,
    })
}

/// The packages filed under each container, by manifest.
static REGISTRY: Mutex<BTreeMap<PathBuf, (Arc<Package>, PathBuf)>> = Mutex::new(BTreeMap::new());

/// File `packages` as the local packages of `container`, the workspace or
/// project whose build they are part of, in place of what it had before.
pub fn register(container: &Path, packages: Vec<Package>) {
    let mut registry = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
    registry.retain(|_, (_, owner)| owner != container);
    for package in packages {
        registry.insert(
            project::standardize(&package.manifest()),
            (Arc::new(package), container.to_path_buf()),
        );
    }
}

/// The package whose `Package.swift` is `manifest`, and the container it was
/// filed under.
#[must_use]
pub fn registered(manifest: &Path) -> Option<(Arc<Package>, PathBuf)> {
    if manifest.file_name()? != "Package.swift" {
        return None;
    }
    REGISTRY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&project::standardize(manifest))
        .cloned()
}

/// The packages filed under `container`.
#[must_use]
pub fn packages_of(container: &Path) -> Vec<Arc<Package>> {
    REGISTRY
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .filter(|(_, owner)| owner == container)
        .map(|(package, _)| Arc::clone(package))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conditions(a: &Assignment) -> Vec<(String, String)> {
        a.conditions
            .iter()
            .map(|c| (c.key.clone(), c.value.clone()))
            .collect()
    }

    #[test]
    fn a_platform_filter_becomes_one_sdk_condition_per_sdk() {
        let out = assignments(
            "CLANG_ENABLE_MODULES[__platform_filter=ios;ios-simulator]",
            &json!("YES"),
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].key, "CLANG_ENABLE_MODULES");
        assert_eq!(conditions(&out[0]), [("sdk".into(), "iphoneos*".into())]);
        assert_eq!(
            conditions(&out[1]),
            [("sdk".into(), "iphonesimulator*".into())]
        );
        assert!(assignments("X[__platform_filter=ios-maccatalyst]", &json!("YES")).is_empty());
        assert!(assignments("SDKROOT", &json!("auto")).is_empty());
    }

    #[test]
    fn a_list_setting_joins_and_quotes_what_has_spaces() {
        let out = assignments(
            "HEADER_SEARCH_PATHS",
            &json!(["/work/My Kit/include", "$(inherited)"]),
        );
        assert_eq!(out[0].value, "\"/work/My Kit/include\" $(inherited)");
    }

    #[test]
    fn a_configuration_containing_debug_builds_packages_as_debug() {
        for config in ["Debug", "debug", "Debug-UAT", "UAT-Debug"] {
            assert_eq!(package_configuration(config), "Debug", "{config}");
        }
        for config in ["Release", "UAT", "Beta Release"] {
            assert_eq!(package_configuration(config), "Release", "{config}");
        }
    }

    #[test]
    fn the_root_manifest_depends_on_each_package_by_path() {
        let text = root_manifest(&[PathBuf::from("/work/Packages/My \"Kit\"")]);
        assert!(text.contains(".package(path: \"/work/Packages/My \\\"Kit\\\"\"),"));
    }

    /// The shape `dump-pif` prints: a package whose library has a dynamic
    /// variant and a test target, exposed through a product another local
    /// package depends on.
    fn dumped() -> Vec<Json> {
        let config =
            |name: &str, settings: Json| json!({ "name": name, "buildSettings": settings });
        let target = |sig: &str, contents: Json| json!({ "type": "target", "signature": sig, "contents": contents });
        vec![
            json!({ "type": "project", "signature": "p1", "contents": {
                "guid": "PACKAGE:core", "projectName": "Core", "projectDirectory": "/work/Core",
                "defaultConfigurationName": "Release",
                "buildConfigurations": [config("Debug", json!({"DEBUG_SETTING": "YES"})), config("Release", json!({}))],
                "groupTree": { "type": "group", "path": "/", "sourceTree": "<absolute>", "children": [
                    { "type": "group", "path": "/work/Core/Sources/Core", "sourceTree": "<absolute>", "children": [
                        { "type": "file", "guid": "F1", "path": "Core.swift", "sourceTree": "<group>" }
                    ]}
                ]},
                "targets": ["t1", "t2", "t3", "t4"],
            }}),
            target(
                "t1",
                json!({ "guid": "PACKAGE-TARGET:Core", "name": "Core", "type": "standard",
                "productTypeIdentifier": "org.swift.product-type.library.static",
                "dynamicTargetVariantGuid": "PACKAGE-TARGET:Core-dynamic",
                "buildConfigurations": [config("Debug", json!({"SWIFT_PACKAGE_NAME": "core"}))],
                "buildPhases": [{ "type": "com.apple.buildphase.sources", "buildFiles": [{ "fileReference": "F1" }] }],
                "dependencies": [] }),
            ),
            target(
                "t2",
                json!({ "guid": "PACKAGE-TARGET:Core-dynamic", "name": "Core", "type": "standard",
                "productTypeIdentifier": "com.apple.product-type.library.dynamic", "dependencies": [] }),
            ),
            target(
                "t3",
                json!({ "guid": "PACKAGE-PRODUCT:core_Core.Core", "name": "Core-product", "type": "packageProduct",
                "dependencies": [{ "guid": "PACKAGE-TARGET:Core" }] }),
            ),
            target(
                "t4",
                json!({ "guid": "PACKAGE-TARGET:CoreTests", "name": "CoreTests", "type": "standard",
                "productTypeIdentifier": "com.apple.product-type.bundle.unit-test", "dependencies": [] }),
            ),
            json!({ "type": "project", "signature": "p2", "contents": {
                "guid": "PACKAGE:feature", "projectName": "Feature", "projectDirectory": "/work/Feature",
                "buildConfigurations": [], "groupTree": { "type": "group", "path": "", "sourceTree": "<group>" },
                "targets": ["t5"],
            }}),
            target(
                "t5",
                json!({ "guid": "PACKAGE-TARGET:Feature", "name": "Feature", "type": "standard",
                "productTypeIdentifier": "org.swift.product-type.library.static",
                "dependencies": [{ "guid": "PACKAGE-PRODUCT:core_Core.Core" }, { "guid": "PACKAGE-PRODUCT:remote_X.X" }] }),
            ),
            json!({ "type": "project", "signature": "p3", "contents": {
                "guid": "PACKAGE:root", "projectName": "SweetPadPackages", "projectDirectory": "/scratch/root",
                "targets": [] }}),
        ]
    }

    #[test]
    fn local_packages_keep_their_compiled_targets_and_local_edges() {
        let packages = parse(&dumped(), &[PathBuf::from("/scratch")]);
        assert_eq!(packages.len(), 2);
        let core = &packages[0];
        assert_eq!(core.targets.len(), 1);
        assert_eq!(
            core.targets[0].sources,
            [PathBuf::from("/work/Core/Sources/Core/Core.swift")]
        );
        assert_eq!(
            core.products,
            [("Core".to_string(), vec!["Core".to_string()])]
        );
        assert_eq!(packages[1].targets[0].dependencies, ["Core"]);
    }

    #[test]
    fn settings_follow_the_package_configuration_for_the_app_one() {
        let packages = parse(&dumped(), &[PathBuf::from("/scratch")]);
        let settings = packages[0].build_settings("Core", "UAT-Debug").unwrap();
        let keys: Vec<&str> = settings.layers[1].iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, ["PROJECT_NAME", "DEBUG_SETTING"]);
        assert_eq!(settings.layers[3][0].key, "SWIFT_PACKAGE_NAME");
        let release = packages[0].build_settings("Core", "UAT").unwrap();
        assert_eq!(release.layers[1].len(), 1);
    }
}
