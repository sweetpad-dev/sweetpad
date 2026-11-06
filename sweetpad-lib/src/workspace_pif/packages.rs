//! The Swift packages in a workspace's PIF.
//!
//! Xcode resolves a workspace's package graph and has its embedded SwiftPM
//! emit each package as a project of its own. SwiftPM ships the same builder
//! as `swift package dump-pif`, so the packages come from there: a root
//! manifest that depends on every package the workspace's projects use stands
//! in for the workspace ([`manifest`]), and the package projects SwiftPM
//! emits for it join the workspace's PIF ([`add_packages`]). Their GUIDs are
//! the ones the projects' targets already refer to.

use std::collections::HashSet;
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
fn dependency(package: &Package) -> Option<String> {
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
fn literal(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
