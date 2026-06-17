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

impl Manifest {
    /// Scheme candidates for the package: its product names — the same set
    /// xcodebuild synthesizes from the manifest. Falls back to non-test target
    /// names when a package declares no products, so scheme selection always
    /// has candidates.
    #[must_use]
    pub fn scheme_names(&self) -> Vec<String> {
        if self.products.is_empty() {
            self.targets
                .iter()
                .filter(|t| !t.is_test())
                .map(|t| t.name.clone())
                .collect()
        } else {
            self.products.iter().map(|p| p.name.clone()).collect()
        }
    }
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

/// Scheme candidates for a package, read directly from the manifest so no
/// xcodebuild (or even a full Xcode) is needed. See [`Manifest::scheme_names`].
pub fn schemes(container: &Container) -> Result<Vec<String>, CliError> {
    Ok(manifest(container)?.scheme_names())
}

/// Map an Xcode configuration name to SwiftPM's `--configuration` value.
/// SwiftPM only knows `debug`/`release`; anything that isn't "Release"
/// (case-insensitive) builds debug, matching `swift build`'s default.
#[must_use]
pub fn configuration_arg(configuration: &str) -> &'static str {
    if configuration.eq_ignore_ascii_case("release") {
        "release"
    } else {
        "debug"
    }
}

/// `swift build` for a package, streaming output to the terminal. `clean` wipes
/// the build directory first — SwiftPM has no `build --clean`, so it's a
/// separate `package clean`.
pub fn build(container: &Container, configuration: &str, clean: bool) -> Result<(), CliError> {
    let cwd = package_dir(container);
    if clean {
        process::stream("swift", &["package", "clean"], cwd.as_deref())?;
    }
    process::stream(
        "swift",
        &["build", "--configuration", configuration_arg(configuration)],
        cwd.as_deref(),
    )
}

/// `swift test` for a package. Returns whether the suite passed (a non-zero
/// exit is a result, not a spawn error). `only`/`skip` map to SwiftPM's
/// `--filter`/`--skip` regex selectors — the closest equivalent to xcodebuild's
/// `-only-testing`/`-skip-testing` identifiers. `quiet` discards stdout (for
/// `--json` callers whose stdout must hold only the summary).
pub fn test(
    container: &Container,
    configuration: &str,
    only: &[String],
    skip: &[String],
    quiet: bool,
) -> Result<bool, CliError> {
    let cwd = package_dir(container);
    let mut args: Vec<String> = vec![
        "test".into(),
        "--configuration".into(),
        configuration_arg(configuration).into(),
    ];
    for f in only {
        args.push("--filter".into());
        args.push(f.clone());
    }
    for s in skip {
        args.push("--skip".into());
        args.push(s.clone());
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    process::run("swift", &arg_refs, cwd.as_deref(), quiet)
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
        assert_eq!(m.scheme_names(), vec!["DemoKit", "demo"]);
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
        assert_eq!(m.scheme_names(), vec!["Lib"]);
    }

    #[test]
    fn configuration_maps_to_debug_or_release() {
        assert_eq!(configuration_arg("Release"), "release");
        assert_eq!(configuration_arg("release"), "release");
        assert_eq!(configuration_arg("Debug"), "debug");
        assert_eq!(configuration_arg("Anything"), "debug");
    }

    #[test]
    fn errors_without_json() {
        assert!(parse_manifest("not json at all").is_err());
    }
}
