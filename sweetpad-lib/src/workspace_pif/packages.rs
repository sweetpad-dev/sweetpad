//! The Swift packages in a workspace's PIF.
//!
//! Xcode resolves a workspace's package graph and has its embedded SwiftPM
//! emit each package as a project of its own. SwiftPM ships the same builder
//! as `swift package dump-pif`, so the packages come from there: a root
//! manifest that depends on every package the workspace's projects use stands
//! in for the workspace ([`manifest`]), and the package projects SwiftPM
//! emits for it join the workspace's PIF ([`add_packages`]). Their GUIDs are
//! the ones the projects' targets already refer to.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};

use super::Opened;
use crate::project::{self, Error};
use crate::spm::{self, Requirement};
use crate::spm_pbxproj::{self, PackageKind};

/// A Swift package a workspace's projects use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Package {
    /// A git repository's URL, or a registry's `scope.name`, and the
    /// versions the project accepts.
    Remote {
        location: String,
        requirement: Requirement,
    },
    /// A package folder.
    Local(PathBuf),
}

impl Package {
    fn identity(&self) -> String {
        match self {
            Package::Remote { location, .. } => spm::identity_from_url(location),
            Package::Local(path) => spm::identity_from_path(&path.to_string_lossy()),
        }
    }
}

/// The packages the workspace or project at `container` uses: the package
/// folders in the workspace, those its projects declare or show in their
/// navigators, then the remote packages its projects declare. A package
/// folder takes the place of a remote package of the same name, as it does
/// in Xcode, and of two declarations of one package the first counts.
pub fn packages(container: &Path) -> Result<Vec<Package>, Error> {
    let opened = Opened::of(container)?;
    let mut local = opened.package_refs;
    let mut remote = Vec::new();
    for file in &opened.graph.projects {
        let (objects, project_obj) = project::project_root(&file.value)?;
        local.extend(project::local_package_refs(
            &file.value,
            objects,
            project_obj,
            &file.path,
        ));
        for package in spm_pbxproj::list_packages(&file.value) {
            if let (PackageKind::Remote { url }, Some(requirement)) =
                (package.kind, package.requirement)
            {
                remote.push(Package::Remote {
                    location: url,
                    requirement,
                });
            }
        }
    }
    let mut seen = HashSet::new();
    Ok(local
        .into_iter()
        .map(Package::Local)
        .chain(remote)
        .filter(|p| seen.insert(p.identity()))
        .collect())
}

/// A root `Package.swift` that depends on `packages`, which SwiftPM resolves
/// and emits the PIF for the way Xcode's SwiftPM does for the workspace. A
/// remote package whose requirement SwiftPM has no spelling for is left out.
#[must_use]
pub fn manifest(packages: &[Package]) -> String {
    let mut out = String::from(
        "// swift-tools-version:5.9\n\
         // The Swift packages an Xcode workspace uses, for Swift Build on a host without Xcode.\n\
         import PackageDescription\n\n\
         let package = Package(\n    name: \"SweetPadPackages\",\n    dependencies: [\n",
    );
    for dependency in packages.iter().filter_map(dependency) {
        let _ = writeln!(out, "        {dependency},");
    }
    out.push_str("    ]\n)\n");
    out
}

/// The `.package(…)` that declares `package`.
pub(super) fn dependency(package: &Package) -> Option<String> {
    let (location, requirement) = match package {
        Package::Local(path) => {
            return Some(format!(
                ".package(path: {})",
                literal(&path.to_string_lossy())
            ));
        }
        Package::Remote {
            location,
            requirement,
        } => (location, requirement),
    };
    let value = || requirement.value.as_deref().map(literal);
    let versions = match requirement.kind.as_str() {
        "upToNextMajorVersion" => format!("from: {}", value()?),
        "upToNextMinorVersion" => format!(".upToNextMinor(from: {})", value()?),
        "exactVersion" => format!("exact: {}", value()?),
        "versionRange" => format!("{}..<{}", value()?, literal(requirement.upper.as_deref()?)),
        "branch" => format!("branch: {}", value()?),
        "revision" => format!("revision: {}", value()?),
        _ => return None,
    };
    // A registry package is named `scope.name`, with no path in it.
    let source = if location.contains(['/', ':']) {
        "url"
    } else {
        "id"
    };
    Some(format!(
        ".package({source}: {}, {versions})",
        literal(location)
    ))
}

/// `text` as a Swift string literal.
pub(super) fn literal(text: &str) -> String {
    let mut value = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' | '"' => {
                value.push('\\');
                value.push(c);
            }
            c if c.is_control() => {
                let _ = write!(value, "\\u{{{:x}}}", u32::from(c));
            }
            c => value.push(c),
        }
    }
    value.push('"');
    value
}

/// Add the package projects in `dumped`, the PIF `swift package dump-pif`
/// printed for [`manifest`] saved in `root`, to the workspace's `pif`. The
/// root package and the aggregate targets SwiftPM adds for it stay out.
///
/// The one setting that differs from Xcode's SwiftPM is the device families
/// a package builds for, which Xcode sets to every one it has and SwiftPM
/// leaves empty.
pub fn add_packages(pif: &mut Vec<Json>, dumped: Vec<Json>, root: &Path) {
    let root_guid = format!(
        "PACKAGE:{}",
        spm::identity_from_path(&root.to_string_lossy())
    );
    let mut projects = Vec::new();
    let mut targets = HashSet::new();
    let mut kept = Vec::new();
    for mut object in dumped {
        let signature = object["signature"].clone();
        let is_project = object["type"] == "project";
        let contents = &mut object["contents"];
        let guid = contents["guid"].as_str().unwrap_or_default();
        if is_project && guid.starts_with("PACKAGE:") && guid != root_guid {
            for configuration in contents["buildConfigurations"]
                .as_array_mut()
                .into_iter()
                .flatten()
            {
                let family = &mut configuration["buildSettings"]["TARGETED_DEVICE_FAMILY"];
                if family.as_str() == Some("") {
                    *family = json!("1,2,3,4,6,7");
                }
            }
            targets.extend(
                contents["targets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| Some(t.as_str()?.to_string())),
            );
            projects.push(signature);
            kept.push(object);
        } else if object["type"] == "target" {
            kept.push(object);
        }
    }
    kept.retain(|o| {
        o["type"] == "project" || o["signature"].as_str().is_some_and(|s| targets.contains(s))
    });
    if let Some(list) = pif
        .first_mut()
        .and_then(|w| w["contents"]["projects"].as_array_mut())
    {
        list.extend(projects);
    }
    pif.extend(kept);
    remap_missing_products(pif);
}

/// Point the build-tool plugin commands in `dumped` at the tools this build
/// makes. SwiftPM evaluates a package target's plugins while it prints the
/// PIF, and names each tool where its own build would put it, in `products`
/// (`<scratch>/out/Products`) under the configuration it evaluated with. The
/// workspace's build puts host tools in `$(BUILD_DIR)` under its own
/// configuration, which is how Xcode's PIF names them.
pub fn retarget_plugin_tools(dumped: &mut [Json], products: &Path) {
    let prefix = format!("{}/", products.to_string_lossy().trim_end_matches('/'));
    let retarget = |value: &mut Json| {
        let Some(path) = value.as_str() else {
            return;
        };
        let Some(rest) = path.strip_prefix(&prefix) else {
            return;
        };
        // `Debug-linux-aarch64`, or `Debug` for macOS.
        let (folder, file) = rest.split_once('/').unwrap_or((rest, ""));
        let platform = folder.find('-').map_or("", |at| &folder[at..]);
        let file = if file.is_empty() {
            String::new()
        } else {
            format!("/{file}")
        };
        *value = json!(format!("${{BUILD_DIR}}/${{CONFIGURATION}}{platform}{file}"));
    };
    for task in dumped
        .iter_mut()
        .filter(|o| o["type"] == "target")
        .filter_map(|o| o["contents"]["customTasks"].as_array_mut())
        .flatten()
    {
        for key in ["commandLine", "inputFilePaths"] {
            for value in task[key].as_array_mut().into_iter().flatten() {
                retarget(value);
            }
        }
    }
}

/// Xcode also resolves products by name when an old package back-reference
/// names the wrong owner, or when a remote product has no back-reference.
/// Only an absent GUID and a single matching resolved product permit repair.
fn remap_missing_products(pif: &mut [Json]) {
    fn visit(value: &mut Json, products: &HashMap<String, Vec<String>>, present: &HashSet<String>) {
        match value {
            Json::String(guid)
                if guid.starts_with("PACKAGE-PRODUCT:") && !present.contains(guid) =>
            {
                if let Some((_, name)) = guid.rsplit_once('.')
                    && let Some(matches) = products.get(name)
                    && let [resolved] = matches.as_slice()
                {
                    guid.clone_from(resolved);
                }
            }
            Json::Object(fields) => {
                for value in fields.values_mut() {
                    visit(value, products, present);
                }
            }
            Json::Array(values) => {
                for value in values {
                    visit(value, products, present);
                }
            }
            _ => {}
        }
    }
    let mut products: HashMap<String, Vec<String>> = HashMap::new();
    let mut present = HashSet::new();
    for target in pif.iter().filter(|o| o["type"] == "target") {
        let Some(guid) = target["contents"]["guid"]
            .as_str()
            .filter(|g| g.starts_with("PACKAGE-PRODUCT:"))
        else {
            continue;
        };
        present.insert(guid.to_string());
        if let Some((_, name)) = guid.rsplit_once('.') {
            products
                .entry(name.to_string())
                .or_default()
                .push(guid.to_string());
        }
    }
    for object in pif {
        visit(object, &products, &present);
    }
}

/// Replace a dependency package with its root-package PIF, which includes
/// individual test products. Other packages keep the workspace's resolution.
pub fn add_package_root(pif: &mut Vec<Json>, dumped: &[Json], root: &Path) {
    let guid = format!(
        "PACKAGE:{}",
        spm::identity_from_path(&root.to_string_lossy())
    );
    let Some(project) = dumped
        .iter()
        .find(|o| o["type"] == "project" && o["contents"]["guid"] == guid)
    else {
        return;
    };
    let signatures: HashSet<&str> = project["contents"]["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Json::as_str)
        .collect();
    let mut selected = vec![json!({"type":"workspace", "contents":{"projects":[]}})];
    selected.extend(
        dumped
            .iter()
            .filter(|o| {
                o["contents"]["guid"] == guid
                    || o["signature"]
                        .as_str()
                        .is_some_and(|s| signatures.contains(s))
            })
            .cloned(),
    );
    let mut root_pif = vec![json!({"type":"workspace", "contents":{"projects":[]}})];
    // There is no synthetic root in this dump to exclude.
    add_packages(&mut root_pif, selected, Path::new(""));
    for object in root_pif.into_iter().skip(1) {
        let guid = &object["contents"]["guid"];
        if let Some(index) = pif.iter().position(|o| &o["contents"]["guid"] == guid) {
            let old = pif[index]["signature"].clone();
            let new = object["signature"].clone();
            if object["type"] == "project"
                && let Some(projects) = pif[0]["contents"]["projects"].as_array_mut()
            {
                for signature in projects.iter_mut().filter(|s| **s == old) {
                    *signature = new.clone();
                }
            }
            pif[index] = object;
        } else {
            if object["type"] == "project"
                && let Some(projects) = pif[0]["contents"]["projects"].as_array_mut()
            {
                projects.push(object["signature"].clone());
            }
            pif.push(object);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unresolved_products_use_a_unique_resolved_owner() {
        let target = |guid: &str| json!({"type":"target","contents":{"guid":guid}});
        let mut pif = vec![
            target("PACKAGE-PRODUCT:actual_Shared.Shared"),
            target("PACKAGE-PRODUCT:left_Ambiguous.Ambiguous"),
            target("PACKAGE-PRODUCT:right_Ambiguous.Ambiguous"),
            json!({"type":"target","contents":{"dependencies":[{"guid":"PACKAGE-PRODUCT:stale_Shared.Shared"},{"guid":"PACKAGE-PRODUCT:stale_Ambiguous.Ambiguous"}],"buildPhases":[{"buildFiles":[{"targetReference":"PACKAGE-PRODUCT:stale_Shared.Shared"}]}]}}),
        ];
        remap_missing_products(&mut pif);
        assert_eq!(
            pif[3]["contents"]["dependencies"][0]["guid"],
            "PACKAGE-PRODUCT:actual_Shared.Shared"
        );
        assert_eq!(
            pif[3]["contents"]["buildPhases"][0]["buildFiles"][0]["targetReference"],
            "PACKAGE-PRODUCT:actual_Shared.Shared"
        );
        assert_eq!(
            pif[3]["contents"]["dependencies"][1]["guid"],
            "PACKAGE-PRODUCT:stale_Ambiguous.Ambiguous"
        );
    }

    fn remote(location: &str, kind: &str, value: &str, upper: Option<&str>) -> Package {
        Package::Remote {
            location: location.to_string(),
            requirement: Requirement {
                kind: kind.to_string(),
                value: Some(value.to_string()),
                upper: upper.map(str::to_string),
            },
        }
    }

    #[test]
    fn each_requirement_has_its_spelling() {
        let url = "https://github.com/Alamofire/Alamofire.git";
        let cases = [
            (
                remote(url, "upToNextMajorVersion", "5.0.0", None),
                format!(".package(url: \"{url}\", from: \"5.0.0\")"),
            ),
            (
                remote(url, "upToNextMinorVersion", "5.1.0", None),
                format!(".package(url: \"{url}\", .upToNextMinor(from: \"5.1.0\"))"),
            ),
            (
                remote(url, "exactVersion", "5.10.1", None),
                format!(".package(url: \"{url}\", exact: \"5.10.1\")"),
            ),
            (
                remote(url, "versionRange", "5.0.0", Some("5.9.0")),
                format!(".package(url: \"{url}\", \"5.0.0\"..<\"5.9.0\")"),
            ),
            (
                remote(url, "branch", "main", None),
                format!(".package(url: \"{url}\", branch: \"main\")"),
            ),
            (
                remote(url, "revision", "e16d348", None),
                format!(".package(url: \"{url}\", revision: \"e16d348\")"),
            ),
            (
                remote("mona.LinkedList", "upToNextMajorVersion", "1.0.0", None),
                ".package(id: \"mona.LinkedList\", from: \"1.0.0\")".to_string(),
            ),
            (
                Package::Local(PathBuf::from("/work/My \"Kit\"")),
                ".package(path: \"/work/My \\\"Kit\\\"\")".to_string(),
            ),
        ];
        for (package, expected) in cases {
            assert_eq!(dependency(&package), Some(expected));
        }
        assert_eq!(
            dependency(&remote(url, "versionRange", "5.0.0", None)),
            None
        );
        assert_eq!(dependency(&remote(url, "commit", "x", None)), None);
    }

    #[test]
    fn the_manifest_lists_every_dependency() {
        let text = manifest(&[
            Package::Local(PathBuf::from("/work/Kit")),
            remote("https://example.com/a/b", "exactVersion", "1.2.3", None),
        ]);
        assert!(text.starts_with("// swift-tools-version:5.9\n"));
        assert!(text.contains(
            "    dependencies: [\n        .package(path: \"/work/Kit\"),\n        \
             .package(url: \"https://example.com/a/b\", exact: \"1.2.3\"),\n    ]\n)\n"
        ));
    }

    #[test]
    fn root_packages_replace_the_project_signature_and_include_test_products() {
        let project = |signature: &str, targets: Json| json!({"type":"project","signature":signature,"contents":{"guid":"PACKAGE:kit","targets":targets}});
        let target = |signature: &str, guid: &str| json!({"type":"target","signature":signature,"contents":{"guid":guid}});
        let mut pif = vec![
            json!({"type":"workspace","contents":{"projects":["OLD"]}}),
            project("OLD", json!(["MODULE"])),
            target("MODULE", "PACKAGE-TARGET:Kit"),
        ];
        let dumped = vec![
            project("ROOT", json!(["MODULE", "TEST"])),
            target("MODULE", "PACKAGE-TARGET:Kit"),
            target("TEST", "PACKAGE-PRODUCT:kit_KitTests.KitTests"),
            json!({"type":"project","signature":"UNRELATED","contents":{"guid":"PACKAGE:other","targets":[]}}),
        ];
        add_package_root(&mut pif, &dumped, Path::new("/work/Kit"));
        add_package_root(&mut pif, &dumped, Path::new("/work/Kit"));
        assert_eq!(pif[0]["contents"]["projects"], json!(["ROOT"]));
        assert_eq!(pif.len(), 4);
        assert_eq!(pif[1]["contents"]["targets"], json!(["MODULE", "TEST"]));
        assert_eq!(pif[3]["signature"], "TEST");
    }

    #[test]
    fn package_projects_join_the_workspace_without_the_root() {
        let project = |guid: &str, signature: &str, targets: &[&str]| {
            json!({
                "type": "project",
                "signature": signature,
                "contents": {
                    "guid": guid,
                    "projectIsPackage": "true",
                    "targets": targets,
                    "buildConfigurations": [
                        { "buildSettings": { "TARGETED_DEVICE_FAMILY": "" } }
                    ],
                },
            })
        };
        let target = |signature: &str| json!({ "type": "target", "signature": signature });
        let dumped = vec![
            json!({ "type": "workspace", "signature": "W", "contents": {} }),
            project("PACKAGE:alamofire", "P1", &["T1", "T2"]),
            target("T1"),
            target("T2"),
            project("PACKAGE:sweetpadpackages", "P2", &["T3"]),
            target("T3"),
            project("AGGREGATE", "P3", &["T4"]),
            target("T4"),
        ];
        let mut pif = vec![json!({ "type": "workspace", "contents": { "projects": ["X"] } })];
        add_packages(&mut pif, dumped, Path::new("/dd/SweetPad/SweetPadPackages"));
        assert_eq!(pif[0]["contents"]["projects"], json!(["X", "P1"]));
        let signatures: Vec<&Json> = pif[1..].iter().map(|o| &o["signature"]).collect();
        assert_eq!(signatures, ["P1", "T1", "T2"]);
        assert_eq!(
            pif[1]["contents"]["buildConfigurations"][0]["buildSettings"]["TARGETED_DEVICE_FAMILY"],
            "1,2,3,4,6,7"
        );
    }

    #[test]
    fn a_package_plugin_runs_the_tool_the_workspace_build_makes() {
        let tool = "/dd/SourcePackages/out/Products/Debug-linux-aarch64/GenTool";
        let output =
            "/dd/SourcePackages/plugins/outputs/gen/Greeting/destination/GenPlugin/Generated.swift";
        let mut dumped = vec![
            json!({ "type": "project", "contents": { "customTasks": [{ "commandLine": [tool] }] } }),
            json!({ "type": "target", "contents": { "customTasks": [{
                "commandLine": [tool, output],
                "inputFilePaths": [tool],
                "outputFilePaths": [output],
            }] } }),
        ];
        retarget_plugin_tools(&mut dumped, Path::new("/dd/SourcePackages/out/Products"));
        let built = "${BUILD_DIR}/${CONFIGURATION}-linux-aarch64/GenTool";
        let task = &dumped[1]["contents"]["customTasks"][0];
        assert_eq!(task["commandLine"], json!([built, output]));
        assert_eq!(task["inputFilePaths"], json!([built]));
        assert_eq!(task["outputFilePaths"], json!([output]));
        assert_eq!(
            dumped[0]["contents"]["customTasks"][0]["commandLine"],
            json!([tool])
        );

        let mac = "/dd/SourcePackages/out/Products/Debug/GenTool";
        let mut dumped = vec![json!({ "type": "target", "contents": { "customTasks": [{
            "commandLine": [mac],
        }] } })];
        retarget_plugin_tools(&mut dumped, Path::new("/dd/SourcePackages/out/Products/"));
        assert_eq!(
            dumped[0]["contents"]["customTasks"][0]["commandLine"],
            json!(["${BUILD_DIR}/${CONFIGURATION}/GenTool"])
        );
    }
}
