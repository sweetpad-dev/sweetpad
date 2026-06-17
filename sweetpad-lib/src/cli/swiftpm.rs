//! Reading a Swift package's structure straight from its manifest, without
//! xcodebuild.
//!
//! `Package.swift` is executable Swift, not a declarative file, so it can't be
//! parsed statically (products/targets may be computed in loops, guarded by
//! `#if os(…)`, etc.). Instead we let the Swift toolchain evaluate the manifest
//! and emit its model as JSON — `swift package dump-package` — and deserialize
//! that. Dumping only *evaluates* the manifest; unlike `swift package describe`
//! it doesn't resolve the dependency graph, so it's offline and fast.
//!
//! This is the SwiftPM counterpart to the in-process pbxproj reader
//! ([`crate::project`]): both expose schemes/targets for a container without
//! shelling out to xcodebuild. JSON is a standard format, so we decode it with
//! `serde_json` rather than hand-rolling a parser (per the crate's dependency
//! policy — hand-roll Apple's project-domain formats, never standard ones).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::cli::CliError;
use crate::cli::process;
use crate::cli::resolve::Container;

/// The decoded `swift package dump-package` model — only the fields we use.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub products: Vec<Product>,
    #[serde(default)]
    pub targets: Vec<Target>,
}

/// A product declared by the package. In the dump, `type` is a single-key
/// object (`{"library":[…]}`, `{"executable":null}`, `{"plugin":…}`, …); we
/// keep it raw and inspect the key, which is robust against new product kinds.
#[derive(Debug, Deserialize)]
pub struct Product {
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: serde_json::Value,
}

impl Product {
    /// Whether this product is an executable (the only kind `swift run` and
    /// `app run` can launch).
    #[must_use]
    pub fn is_executable(&self) -> bool {
        self.kind.get("executable").is_some()
    }
}

/// A target declared by the package. `type` is a plain string here: `regular`,
/// `executable`, `test`, `system`, `binary`, `plugin`, or `macro`.
#[derive(Debug, Deserialize)]
pub struct Target {
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
}

impl Target {
    #[must_use]
    pub fn is_test(&self) -> bool {
        self.kind == "test"
    }
}

/// The package root — the directory holding `Package.swift`, where `swift` must
/// run. A relative `Package.swift` has an empty parent meaning the current
/// directory, so return `None` rather than `chdir("")` (which fails the spawn
/// and looks like a missing tool). Mirrors `xcodebuild::working_dir`.
#[must_use]
pub fn package_dir(container: &Container) -> Option<PathBuf> {
    container
        .path()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

/// Evaluate `Package.swift` and decode its manifest model. Runs `swift` from the
/// package root; stderr (e.g. fetch progress) is inherited, stdout is the JSON.
pub fn manifest(container: &Container) -> Result<Manifest, CliError> {
    let cwd = package_dir(container);
    let stdout = process::capture("swift", &["package", "dump-package"], cwd.as_deref())?;
    parse_manifest(&stdout)
}

/// Parse the JSON object emitted by `swift package dump-package`, skipping any
/// leading non-JSON the toolchain may print before it.
fn parse_manifest(stdout: &str) -> Result<Manifest, CliError> {
    let json = stdout
        .find('{')
        .map(|i| &stdout[i..])
        .ok_or_else(|| CliError::new("swift package dump-package produced no JSON"))?;
    serde_json::from_str(json)
        .map_err(|e| CliError::new(format!("parsing swift package dump-package: {e}")))
}

/// Scheme candidates for a package: its product names — the same set xcodebuild
/// synthesizes from the manifest, but read directly so no xcodebuild (or even a
/// full Xcode) is needed. Falls back to non-test target names when a package
/// declares no products, so scheme selection always has candidates.
pub fn schemes(container: &Container) -> Result<Vec<String>, CliError> {
    let manifest = manifest(container)?;
    let mut names: Vec<String> = manifest.products.iter().map(|p| p.name.clone()).collect();
    if names.is_empty() {
        names = manifest
            .targets
            .iter()
            .filter(|t| !t.is_test())
            .map(|t| t.name.clone())
            .collect();
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A representative `swift package dump-package` payload: a library product,
    // an executable product, and a test target (which is not a product).
    const DUMP: &str = r#"{
        "name": "Demo",
        "products": [
            { "name": "DemoKit", "type": { "library": ["automatic"] }, "targets": ["DemoKit"] },
            { "name": "demo",    "type": { "executable": null },       "targets": ["demo"] }
        ],
        "targets": [
            { "name": "DemoKit",      "type": "regular" },
            { "name": "demo",         "type": "executable" },
            { "name": "DemoKitTests", "type": "test" }
        ]
    }"#;

    #[test]
    fn parses_products_and_targets() {
        let m = parse_manifest(DUMP).unwrap();
        assert_eq!(m.name, "Demo");
        assert_eq!(m.products.len(), 2);
        assert!(m.products.iter().any(|p| p.name == "demo" && p.is_executable()));
        assert!(m.products.iter().any(|p| p.name == "DemoKit" && !p.is_executable()));
        assert!(m.targets.iter().any(|t| t.name == "DemoKitTests" && t.is_test()));
    }

    #[test]
    fn schemes_are_product_names() {
        let m = parse_manifest(DUMP).unwrap();
        let names: Vec<&str> = m.products.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["DemoKit", "demo"]);
    }

    #[test]
    fn skips_leading_noise_before_json() {
        let noisy = format!("Fetching dependencies\n{DUMP}");
        assert_eq!(parse_manifest(&noisy).unwrap().name, "Demo");
    }

    #[test]
    fn falls_back_to_non_test_targets_when_no_products() {
        let m = parse_manifest(
            r#"{ "name": "P", "products": [],
                 "targets": [ { "name": "Lib", "type": "regular" },
                              { "name": "LibTests", "type": "test" } ] }"#,
        )
        .unwrap();
        let names: Vec<String> = m
            .targets
            .iter()
            .filter(|t| !t.is_test())
            .map(|t| t.name.clone())
            .collect();
        assert_eq!(names, vec!["Lib"]);
    }

    #[test]
    fn errors_without_json() {
        assert!(parse_manifest("not json at all").is_err());
    }
}
