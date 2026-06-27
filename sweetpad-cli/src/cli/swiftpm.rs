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
//! ([`sweetpad_lib::project`]): both expose schemes/targets for a container without
//! shelling out to xcodebuild. JSON is a standard format, so we decode it with
//! `serde_json` rather than hand-rolling a parser (per the crate's dependency
//! policy — hand-roll Apple's project-domain formats, never standard ones).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::cli::process;
use crate::cli::resolve::Container;
use crate::cli::{CliError, ErrorContext};

/// The decoded `swift package dump-package` model — only the fields we use.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub products: Vec<Product>,
    #[serde(default)]
    pub targets: Vec<Target>,
    /// The declared dependencies array, kept raw: its encoding varies across
    /// `swift-tools-version`s (tagged unions of `sourceControl`/`fileSystem`,
    /// older `scm`/`local`), so [`Manifest::declared_dependencies`] decodes it
    /// best-effort rather than failing the whole parse on an unknown shape.
    #[serde(default)]
    pub dependencies: Vec<serde_json::Value>,
}

/// A dependency declared in a `Package.swift` manifest — what `dependency list`
/// shows for a Swift-package container. Best-effort, read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredDep {
    /// SwiftPM identity (used to correlate with a `Package.resolved` pin).
    pub identity: String,
    /// Repository URL or local path.
    pub location: String,
    /// A compact requirement rendering (e.g. `1.0.0 ..< 2.0.0`, `branch main`,
    /// `local`), or `(unparsed)` for an encoding we don't recognize.
    pub requirement: String,
    pub remote: bool,
}

impl Manifest {
    /// Scheme candidates for the package, matching `xcodebuild -list`: one
    /// scheme per product, plus the `<name>-Package` aggregate scheme xcodebuild
    /// synthesizes to build the whole package. Falls back to non-test target
    /// names (still plus the aggregate) when a package declares no products, so
    /// scheme selection always has candidates.
    #[must_use]
    pub fn scheme_names(&self) -> Vec<String> {
        let mut names: Vec<String> = if self.products.is_empty() {
            self.targets
                .iter()
                .filter(|t| !t.is_test())
                .map(|t| t.name.clone())
                .collect()
        } else {
            self.products.iter().map(|p| p.name.clone()).collect()
        };
        names.push(format!("{}-Package", self.name));
        names
    }

    /// The package's declared dependencies, decoded best-effort from the raw
    /// `dependencies` array. Unknown entries are skipped (never an error).
    #[must_use]
    pub fn declared_dependencies(&self) -> Vec<DeclaredDep> {
        self.dependencies
            .iter()
            .filter_map(parse_dependency)
            .collect()
    }
}

/// Decode one `dump-package` dependency entry. Handles the modern
/// `sourceControl`/`fileSystem` tagged-union shape; returns `None` for shapes we
/// don't recognize so the caller drops it rather than failing.
fn parse_dependency(dep: &serde_json::Value) -> Option<DeclaredDep> {
    if let Some(sc) = first_of(dep, "sourceControl") {
        let identity = str_at(sc, "identity").unwrap_or_default().to_string();
        let location = sc
            .get("location")
            .and_then(|loc| {
                first_of(loc, "remote")
                    .and_then(|m| str_at(m, "urlString").or_else(|| str_at(m, "url")))
                    .or_else(|| loc.as_str())
            })
            .unwrap_or_default()
            .to_string();
        let requirement = sc
            .get("requirement")
            .map_or_else(|| "(unparsed)".to_string(), requirement_string);
        return Some(DeclaredDep {
            identity,
            location,
            requirement,
            remote: true,
        });
    }
    if let Some(fs) = first_of(dep, "fileSystem") {
        let identity = str_at(fs, "identity").unwrap_or_default().to_string();
        let location = str_at(fs, "path").unwrap_or_default().to_string();
        return Some(DeclaredDep {
            identity,
            location,
            requirement: "local".to_string(),
            remote: false,
        });
    }
    None
}

/// The first element of `dep[key]` when it's a non-empty array (the SwiftPM
/// tagged-union encoding wraps each case's payload in a one-element array).
fn first_of<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    value.get(key)?.as_array()?.first()
}

fn str_at<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

/// Render a `dump-package` requirement object compactly. Tolerates the
/// array-wrapped (`{"exact":["1.0.0"]}`) and bare (`{"exact":"1.0.0"}`) forms.
fn requirement_string(req: &serde_json::Value) -> String {
    if let Some(range) = first_of(req, "range") {
        let lo = str_at(range, "lowerBound").unwrap_or("?");
        let hi = str_at(range, "upperBound").unwrap_or("?");
        return format!("{lo} ..< {hi}");
    }
    for key in ["exact", "branch", "revision"] {
        if let Some(v) = req.get(key) {
            let val = v
                .as_array()
                .and_then(|a| a.first())
                .and_then(serde_json::Value::as_str)
                .or_else(|| v.as_str())
                .unwrap_or_default();
            return format!("{key} {val}");
        }
    }
    "(unparsed)".to_string()
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

/// Evaluate the `Package.swift` at an explicit directory — e.g. a resolved
/// dependency checkout, so `dependency add` can read the package's real products
/// before linking them. Mirrors [`manifest`] but with `--package-path`.
pub fn manifest_at(package_path: &Path) -> Result<Manifest, CliError> {
    let path = package_path.to_string_lossy();
    let stdout = process::capture(
        "swift",
        &["package", "dump-package", "--package-path", &path],
        None,
    )?;
    parse_manifest(&stdout)
}

/// `swift package add-dependency <url> <requirement…>` (Swift 6+). `requirement`
/// is the already-assembled SwiftPM flag list (e.g. `["--from", "1.2.3"]`).
/// Streams output to the terminal.
pub fn add_dependency(
    container: &Container,
    url: &str,
    requirement: &[String],
) -> Result<(), CliError> {
    let cwd = package_dir(container);
    let mut args: Vec<&str> = vec!["package", "add-dependency", url];
    args.extend(requirement.iter().map(String::as_str));
    process::stream("swift", &args, cwd.as_deref()).context("adding the package dependency")
}

/// `swift package add-target-dependency <product> <target> --package <name>`
/// (Swift 6+) — link a product of an added package into a target.
pub fn add_target_dependency(
    container: &Container,
    product: &str,
    target: &str,
    package: &str,
) -> Result<(), CliError> {
    let cwd = package_dir(container);
    process::stream(
        "swift",
        &[
            "package",
            "add-target-dependency",
            product,
            target,
            "--package",
            package,
        ],
        cwd.as_deref(),
    )
    .context("linking the product to the target")
}

/// `swift package resolve` — fetch and pin dependencies into `Package.resolved`.
/// `quiet` discards stdout (for `--json` callers).
pub fn resolve(container: &Container, quiet: bool) -> Result<(), CliError> {
    let cwd = package_dir(container);
    if process::run("swift", &["package", "resolve"], cwd.as_deref(), quiet)? {
        Ok(())
    } else {
        Err(
            CliError::new("swift package resolve exited with a non-zero status")
                .context("resolving package dependencies"),
        )
    }
}

/// `swift package update [name]` — bump pinned versions to the latest the
/// requirements allow (one dependency, or all). `quiet` discards stdout.
pub fn update(container: &Container, name: Option<&str>, quiet: bool) -> Result<(), CliError> {
    let cwd = package_dir(container);
    let mut args = vec!["package", "update"];
    if let Some(name) = name {
        args.push(name);
    }
    if process::run("swift", &args, cwd.as_deref(), quiet)? {
        Ok(())
    } else {
        Err(
            CliError::new("swift package update exited with a non-zero status")
                .context("updating package dependencies"),
        )
    }
}

/// The toolchain's major Swift version (`swift --version`), for gating features
/// like `swift package add-dependency` (Swift 6+). `None` if it can't be read.
#[must_use]
pub fn swift_major_version() -> Option<u32> {
    let out = process::capture("swift", &["--version"], None).ok()?;
    parse_swift_major(&out)
}

/// Parse the major version from `swift --version` output, e.g. "Apple Swift
/// version 6.0.3 (...)" or "Swift version 6.1-dev" → `6`.
fn parse_swift_major(text: &str) -> Option<u32> {
    let after = text.split("Swift version ").nth(1)?;
    let number = after.trim_start();
    let major: String = number.chars().take_while(char::is_ascii_digit).collect();
    major.parse().ok()
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

/// `swift build` for a package. Streams output to the terminal unless `quiet`
/// (for `--json` callers whose stdout must hold only the result envelope).
/// `clean` wipes the build directory first — SwiftPM has no `build --clean`, so
/// it's a separate `package clean`.
pub fn build(
    container: &Container,
    configuration: &str,
    clean: bool,
    quiet: bool,
) -> Result<(), CliError> {
    let cwd = package_dir(container);
    if clean {
        process::run("swift", &["package", "clean"], cwd.as_deref(), quiet)
            .context("cleaning the package build")?;
    }
    let ok = process::run(
        "swift",
        &["build", "--configuration", configuration_arg(configuration)],
        cwd.as_deref(),
        quiet,
    )
    .context("building the package")?;
    if ok {
        Ok(())
    } else {
        Err(CliError::new("swift build exited with a non-zero status")
            .context("building the package"))
    }
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
    process::run("swift", &arg_refs, cwd.as_deref(), quiet).context("running the package tests")
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
        assert!(
            m.products
                .iter()
                .any(|p| p.name == "demo" && p.is_executable())
        );
        assert!(
            m.products
                .iter()
                .any(|p| p.name == "DemoKit" && !p.is_executable())
        );
        assert!(
            m.targets
                .iter()
                .any(|t| t.name == "DemoKitTests" && t.is_test())
        );
    }

    #[test]
    fn schemes_are_products_plus_package_aggregate() {
        let m = parse_manifest(DUMP).unwrap();
        // Products, then xcodebuild's `<name>-Package` aggregate scheme.
        assert_eq!(m.scheme_names(), vec!["DemoKit", "demo", "Demo-Package"]);
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
        assert_eq!(m.scheme_names(), vec!["Lib", "P-Package"]);
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

    #[test]
    fn parses_swift_major_version() {
        assert_eq!(
            parse_swift_major("Apple Swift version 6.0.3 (swiftlang-...)"),
            Some(6)
        );
        assert_eq!(
            parse_swift_major("Swift version 6.1-dev (LLVM ...)"),
            Some(6)
        );
        assert_eq!(
            parse_swift_major("Swift version 5.10 (swift-5.10...)"),
            Some(5)
        );
        assert_eq!(parse_swift_major("garbage"), None);
    }

    #[test]
    fn decodes_declared_dependencies() {
        let m: Manifest = serde_json::from_str(
            r#"{ "name": "P",
                 "dependencies": [
                   { "sourceControl": [ {
                       "identity": "alamofire",
                       "location": { "remote": [ { "urlString": "https://github.com/Alamofire/Alamofire.git" } ] },
                       "requirement": { "range": [ { "lowerBound": "5.9.0", "upperBound": "6.0.0" } ] } } ] },
                   { "fileSystem": [ { "identity": "dep", "path": "/abs/Dep" } ] }
                 ] }"#,
        )
        .unwrap();
        let deps = m.declared_dependencies();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].identity, "alamofire");
        assert!(deps[0].remote);
        assert_eq!(
            deps[0].location,
            "https://github.com/Alamofire/Alamofire.git"
        );
        assert_eq!(deps[0].requirement, "5.9.0 ..< 6.0.0");
        assert_eq!(deps[1].identity, "dep");
        assert!(!deps[1].remote);
        assert_eq!(deps[1].requirement, "local");
    }
}
