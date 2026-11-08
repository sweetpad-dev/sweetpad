//! Scheme files in the container's resolved SwiftPM checkouts. Reading the
//! recorded graph never resolves dependencies or scans unrelated caches.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

/// Remote checkouts in this container's resolved dependency graph. An explicit
/// DerivedData path takes precedence over Xcode's configured/default location.
#[must_use]
pub fn package_dirs(container: &Path, derived_data: Option<&Path>) -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let source_packages = crate::derived_data::resolve(
        &crate::derived_data::ContainerKey::of(container),
        &home,
        derived_data,
        true,
    )
    .folder
    .join("SourcePackages");
    checkouts(&source_packages)
}

fn checkouts(source_packages: &Path) -> Vec<PathBuf> {
    let state = std::fs::read(source_packages.join("workspace-state.json"))
        .ok()
        .and_then(|s| serde_json::from_slice::<Value>(&s).ok());
    let Some(state) = state else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    state["object"]["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|dependency| {
            if dependency["state"]["name"] != "sourceControlCheckout" {
                return None;
            }
            let subpath = Path::new(dependency["subpath"].as_str()?);
            if subpath.as_os_str().is_empty()
                || subpath
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
            {
                return None;
            }
            let path = source_packages.join("checkouts").join(subpath);
            (path.join("Package.swift").is_file() && seen.insert(path.clone())).then_some(path)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;
    use serde_json::json;

    #[test]
    fn only_recorded_checkouts_contribute_schemes() {
        let root = TempDir::new("resolved-package-schemes");
        for name in ["Current", "Unrelated"] {
            let path = root.join("checkouts").join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("Package.swift"), "").unwrap();
        }
        let dependency = |name: &str, state: &str| json!({"subpath":name,"state":{"name":state}});
        let state = json!({"object":{"dependencies":[
            dependency("Current", "sourceControlCheckout"),
            dependency("Current", "sourceControlCheckout"),
            dependency("Unrelated", "edited"),
            dependency("../escape", "sourceControlCheckout"),
            dependency("Missing", "sourceControlCheckout"),
        ]}});
        std::fs::write(root.join("workspace-state.json"), state.to_string()).unwrap();
        assert_eq!(checkouts(&root), vec![root.join("checkouts/Current")]);
        std::fs::write(root.join("workspace-state.json"), "invalid").unwrap();
        assert!(checkouts(&root).is_empty());
    }

    #[test]
    fn custom_derived_data_resolves_remote_files_after_project_schemes() {
        let root = TempDir::new("resolved-scheme-lookup");
        let project = root.join("App.xcodeproj");
        let derived_data = root.join("Custom DerivedData");
        let sources = derived_data.join("SourcePackages");
        let package = sources.join("checkouts/Package");
        let schemes = package.join(".swiftpm/xcode/xcshareddata/xcschemes");
        std::fs::create_dir_all(&schemes).unwrap();
        std::fs::write(package.join("Package.swift"), "").unwrap();
        let remote = schemes.join("Remote.xcscheme");
        std::fs::write(&remote, "<Scheme/>").unwrap();
        std::fs::write(sources.join("workspace-state.json"), json!({"object":{"dependencies":[{"subpath":"Package","state":{"name":"sourceControlCheckout"}}]}}).to_string()).unwrap();
        assert_eq!(
            super::super::locate_with_derived_data(&project, "Remote", Some(&derived_data)),
            Some(remote)
        );
        let own = project.join("xcshareddata/xcschemes/Remote.xcscheme");
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        std::fs::write(&own, "<Scheme/>").unwrap();
        assert_eq!(
            super::super::locate_with_derived_data(&project, "Remote", Some(&derived_data)),
            Some(own)
        );
    }
}
