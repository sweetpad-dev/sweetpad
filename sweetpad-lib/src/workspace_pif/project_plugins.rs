//! Evaluate project-target plugin attachments with SwiftPM, then attach the
//! resulting build commands and generated files to their owning Xcode target.
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    Package, object,
    packages::{dependency, literal},
};
use crate::destination::Platform;
use crate::pif::file_paths;

/// An Xcode target and the package plugins it runs for the selected destination.
pub struct PluginTarget {
    pub guid: String,
    pub name: String,
    pub directory: PathBuf,
    pub development_region: String,
    pub sources: Vec<PathBuf>,
    pub resources: Vec<PathBuf>,
    pub plugins: Vec<(String, String)>,
}

/// Find attachments reachable from the selected scheme's target GUIDs.
/// Resolve the plugin package from its product's owning project, so a product
/// whose name differs from its package works for local and remote packages.
#[must_use]
pub fn targets(pif: &[Value], roots: &[String], platform: Option<&Platform>) -> Vec<PluginTarget> {
    let owners: HashMap<_, _> = pif
        .iter()
        .filter(|o| o["type"] == "project")
        .flat_map(|p| {
            p["contents"]["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| Some((s.as_str()?, &p["contents"])))
        })
        .collect();
    let owner = |object: &Value| {
        object["signature"]
            .as_str()
            .and_then(|s| owners.get(s))
            .copied()
    };
    let mut result = Vec::new();
    for (obj, products) in attachments(pif, roots, platform) {
        let Some(project) = owner(obj) else { continue };
        let target = &obj["contents"];
        let plugins = products
            .into_iter()
            .filter_map(|(product, name)| {
                Some((
                    name,
                    owner(product)?["guid"]
                        .as_str()?
                        .strip_prefix("PACKAGE:")?
                        .to_owned(),
                ))
            })
            .collect();
        let directory = PathBuf::from(project["projectDirectory"].as_str().unwrap_or_default());
        let files = file_paths(&project["groupTree"], &directory);
        let inputs = |kind: &str| {
            target["buildPhases"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| p["type"] == kind)
                .flat_map(|p| p["buildFiles"].as_array().into_iter().flatten())
                .filter(|f| enabled(f, platform))
                .filter_map(|f| files.get(f["fileReference"].as_str()?).cloned())
                .collect()
        };
        result.push(PluginTarget {
            guid: target["guid"].as_str().unwrap_or_default().to_owned(),
            name: target["name"].as_str().unwrap_or_default().to_owned(),
            directory,
            development_region: project["developmentRegion"]
                .as_str()
                .unwrap_or("en")
                .to_owned(),
            sources: inputs("com.apple.buildphase.sources"),
            resources: inputs("com.apple.buildphase.resources"),
            plugins,
        });
    }
    result
}

fn enabled(item: &Value, platform: Option<&Platform>) -> bool {
    let Some(filters) = item["platformFilters"].as_array().filter(|f| !f.is_empty()) else {
        return true;
    };
    platform.is_none_or(|p| {
        filters.iter().any(|f| {
            f["platform"].as_str().is_some_and(|os| p.matches_os(os))
                && match f["environment"].as_str().unwrap_or_default() {
                    "" => true,
                    "simulator" => p.simulator,
                    _ => false,
                }
        })
    })
}

type Attachments<'a> = (&'a Value, Vec<(&'a Value, String)>);

fn attachments<'a>(
    pif: &'a [Value],
    roots: &[String],
    platform: Option<&Platform>,
) -> Vec<Attachments<'a>> {
    let by_guid: HashMap<_, _> = pif
        .iter()
        .filter(|o| o["type"] == "target")
        .filter_map(|o| Some((o["contents"]["guid"].as_str()?, o)))
        .collect();
    let mut pending: Vec<_> = roots.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    while let Some(guid) = pending.pop() {
        if !seen.insert(guid) {
            continue;
        }
        let Some(obj) = by_guid.get(guid) else {
            continue;
        };
        let target = &obj["contents"];
        let mut plugins = Vec::new();
        for dependency in target["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|dep| enabled(dep, platform))
        {
            let Some(dep) = dependency["guid"].as_str() else {
                continue;
            };
            pending.push(dep);
            if !guid.starts_with("PACKAGE-")
                && dep.starts_with("PACKAGE-PRODUCT:")
                && let Some(product) = by_guid.get(dep)
                && product["contents"]["type"] == "aggregate"
            {
                let name = dependency["name"].as_str().unwrap_or_else(|| {
                    product["contents"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .trim_end_matches("-product")
                });
                plugins.push((*product, name.to_owned()));
            }
        }
        if !plugins.is_empty() {
            result.push((*obj, plugins));
        }
    }
    result
}

/// The target and plugin names for attachments reachable from these scheme target names.
#[must_use]
pub fn project_target_plugins(pif: &[Value], names: &[String]) -> Vec<(String, String)> {
    let roots = pif
        .iter()
        .filter(|o| o["type"] == "target" && names.iter().any(|n| o["contents"]["name"] == *n))
        .filter_map(|o| Some(o["contents"]["guid"].as_str()?.to_owned()))
        .collect::<Vec<_>>();
    attachments(pif, &roots, None)
        .into_iter()
        .flat_map(|(t, plugins)| {
            plugins.into_iter().map(move |(_, name)| {
                (
                    t["contents"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    name,
                )
            })
        })
        .collect()
}

impl PluginTarget {
    /// SwiftPM evaluates the listed attachments in declaration order.
    #[must_use]
    pub fn manifest(
        &self,
        packages: &[Package],
        sources: &[String],
        resources: &[String],
        exclude: &[String],
    ) -> String {
        let dependencies = packages
            .iter()
            .filter_map(dependency)
            .collect::<Vec<_>>()
            .join(", ");
        let plugins = self
            .plugins
            .iter()
            .map(|(name, identity)| {
                format!(
                    ".plugin(name: {}, package: {})",
                    literal(name),
                    literal(identity)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let strings = |v: &[String]| v.iter().map(|s| literal(s)).collect::<Vec<_>>().join(", ");
        let resources = resources
            .iter()
            .map(|s| format!(".copy({})", literal(s)))
            .collect::<Vec<_>>()
            .join(", ");
        let name = literal(&self.name);
        let region = literal(&self.development_region);
        let sources = strings(sources);
        let exclude = strings(exclude);
        format!(
            r#"// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: {name},
    defaultLocalization: {region},
    dependencies: [{dependencies}],
    targets: [
        .target(
            name: {name}, path: ".", exclude: [{exclude}], sources: [{sources}],
            resources: [{resources}], publicHeadersPath: ".", plugins: [{plugins}]
        )
    ]
)
"#
        )
    }
}

/// Move SwiftPM's commands and the outputs it classified for the proxy into
/// the real target. SwiftPM can add generated resources to a separate bundle
/// target, and prebuild commands can produce files during planning.
pub fn attach(
    pif: &mut [Value],
    target: &PluginTarget,
    dumped: &[Value],
    root: &Path,
    outputs: &Path,
) -> Result<(), String> {
    let project = dumped
        .iter()
        .map(|o| &o["contents"])
        .find(|p| p["projectDirectory"].as_str() == root.to_str())
        .ok_or("missing generated package project")?;
    let members: HashSet<_> = project["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let source_guid = format!("PACKAGE-TARGET:{}", target.name);
    let resource_guid = format!("PACKAGE-RESOURCE:{}", target.name);
    let proxy_targets: Vec<_> = dumped
        .iter()
        .filter(|o| o["signature"].as_str().is_some_and(|s| members.contains(s)))
        .map(|o| &o["contents"])
        .filter(|t| t["guid"] == source_guid || t["guid"] == resource_guid)
        .collect();
    if !proxy_targets.iter().any(|t| t["guid"] == source_guid) {
        return Err(format!(
            "SwiftPM omitted the plugin evaluation target '{}'",
            target.name
        ));
    }
    let paths = file_paths(&project["groupTree"], root);
    let t = pif
        .iter()
        .position(|o| o["contents"]["guid"] == target.guid)
        .ok_or("missing project target")?;
    let old = pif[t]["signature"].clone();
    let p = pif
        .iter()
        .position(|o| {
            o["type"] == "project"
                && o["contents"]["targets"]
                    .as_array()
                    .is_some_and(|a| a.contains(&old))
        })
        .ok_or("missing owning project")?;
    let mut contents = pif[t]["contents"].take();
    // A command that emits a resource lives on SwiftPM's bundle target,
    // even if it also produces a source for the module.
    let tasks = proxy_targets
        .iter()
        .flat_map(|t| t["customTasks"].as_array().into_iter().flatten());
    let mut commands = contents["customTasks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    commands.extend(tasks.cloned());
    contents["customTasks"] = Value::Array(commands);
    let mut found = BTreeSet::new();
    for phase in proxy_targets
        .iter()
        .flat_map(|t| t["buildPhases"].as_array().into_iter().flatten())
    {
        let kind = phase["type"].as_str().unwrap_or_default();
        if !matches!(
            kind,
            "com.apple.buildphase.sources" | "com.apple.buildphase.resources"
        ) {
            continue;
        }
        for file in phase["buildFiles"].as_array().into_iter().flatten() {
            let Some(path) = file["fileReference"].as_str().and_then(|r| paths.get(r)) else {
                continue;
            };
            if path.starts_with(outputs) {
                found.insert((kind, path));
            }
        }
    }
    for (index, (kind, path)) in found.into_iter().enumerate() {
        let id = format!("{}::PLUGIN_OUTPUT_{index}", target.guid);
        add_generated_file(&mut pif[p]["contents"], &mut contents, kind, path, &id)?;
    }
    pif[t] = object("target", contents);
    let new_signature = pif[t]["signature"].clone();
    let old_project = pif[p]["signature"].clone();
    replace_child(&mut pif[p], "project", "targets", &old, &new_signature);
    let new_project = pif[p]["signature"].clone();
    replace_child(
        &mut pif[0],
        "workspace",
        "projects",
        &old_project,
        &new_project,
    );
    Ok(())
}

fn add_generated_file(
    project: &mut Value,
    target: &mut Value,
    kind: &str,
    path: &Path,
    id: &str,
) -> Result<(), String> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let children = project["groupTree"]["children"]
        .as_array_mut()
        .ok_or("missing project group")?;
    children.push(json!({
        "type": "file", "guid": id, "sourceTree": "<absolute>", "path": path,
        "fileType": crate::pif_file_types::for_extension(extension).unwrap_or("file"),
    }));
    let phase_guid = format!("{}::{kind}", target["guid"].as_str().unwrap_or_default());
    let phases = target["buildPhases"]
        .as_array_mut()
        .ok_or("missing target phases")?;
    let index = phases
        .iter()
        .position(|phase| phase["type"] == kind)
        .unwrap_or_else(|| {
            phases.push(json!({"type": kind, "guid": phase_guid, "buildFiles": []}));
            phases.len() - 1
        });
    phases[index]["buildFiles"]
        .as_array_mut()
        .ok_or("missing build file list")?
        .push(json!({"guid": format!("{id}::BUILD"), "fileReference": id}));
    Ok(())
}

// Swift Build caches objects by signature, including the parents that reference them.
fn replace_child(parent: &mut Value, kind: &str, list: &str, old: &Value, new: &Value) {
    for signature in parent["contents"][list]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        if signature == old {
            signature.clone_from(new);
        }
    }
    *parent = object(kind, parent["contents"].take());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resource_bundle_command_and_its_generated_files_join_the_app_once() {
        let root = Path::new("/proxy");
        let outputs = Path::new("/scratch/plugins/outputs");
        let source = outputs.join("Generated.swift");
        let resource = outputs.join("PluginInputs.txt");
        let file = |guid: &str, path: &Path| json!({"guid":guid,"type":"file","sourceTree":"<absolute>", "path":path});
        let phase =
            |kind: &str, guid: &str| json!({"type":kind,"buildFiles":[{"fileReference":guid}]});
        let command = json!({"commandLine":["GenTool", source, resource]});
        let dumped = [
            json!({"type":"project","contents":{"projectDirectory":root,"targets":["module","bundle"],"groupTree":{"children":[file("S",&source),file("R",&resource)]}}}),
            json!({"type":"target","signature":"module","contents":{"guid":"PACKAGE-TARGET:App","buildPhases":[phase("com.apple.buildphase.sources","S")],"customTasks":[]}}),
            json!({"type":"target","signature":"bundle","contents":{"guid":"PACKAGE-RESOURCE:App","buildPhases":[phase("com.apple.buildphase.resources","R")],"customTasks":[command]}}),
        ];
        let mut pif = [
            json!({"type":"workspace","contents":{"projects":["P"]}}),
            json!({"type":"project","signature":"P","contents":{"targets":["T"],"groupTree":{"children":[]}}}),
            json!({"type":"target","signature":"T","contents":{"guid":"APP","customTasks":[{"commandLine":["existing"]}],"buildPhases":[]}}),
        ];
        let target = PluginTarget {
            guid: "APP".into(),
            name: "App".into(),
            directory: "/project".into(),
            development_region: "en".into(),
            sources: vec![],
            resources: vec![],
            plugins: vec![],
        };
        attach(&mut pif, &target, &dumped, root, outputs).unwrap();
        assert_eq!(
            pif[2]["contents"]["customTasks"],
            json!([{"commandLine":["existing"]},command])
        );
        let paths = file_paths(&pif[1]["contents"]["groupTree"], root);
        let phases = pif[2]["contents"]["buildPhases"].as_array().unwrap();
        for (kind, expected) in [
            ("com.apple.buildphase.sources", source),
            ("com.apple.buildphase.resources", resource),
        ] {
            let phase = phases.iter().find(|p| p["type"] == kind).unwrap();
            assert_eq!(
                paths[phase["buildFiles"][0]["fileReference"].as_str().unwrap()],
                expected
            );
        }
        assert_eq!(pif[0]["contents"]["projects"][0], pif[1]["signature"]);
        assert_eq!(pif[1]["contents"]["targets"][0], pif[2]["signature"]);
    }
}
