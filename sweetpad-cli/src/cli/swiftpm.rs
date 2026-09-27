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
use std::process::{Command, Stdio};

use serde::Deserialize;
use sweetpad_core::package_members::{DumpError, PackageNames, Toolchain};
use sweetpad_core::scratch::ScratchDir;

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
        // The location union has a `local` case too (a local *git* URL like
        // `.package(url: "../Sibling", …)`) — without it the location column
        // rendered blank for such dependencies.
        let loc = sc.get("location");
        let remote_url = loc
            .and_then(|l| first_of(l, "remote"))
            .and_then(|m| str_at(m, "urlString").or_else(|| str_at(m, "url")));
        let local_path = loc
            .and_then(|l| first_of(l, "local"))
            .and_then(|m| m.as_str().or_else(|| str_at(m, "path")));
        let location = remote_url
            .or(local_path)
            .or_else(|| loc.and_then(serde_json::Value::as_str))
            .unwrap_or_default()
            .to_string();
        let requirement = sc
            .get("requirement")
            .map_or_else(|| "(unparsed)".to_string(), requirement_string);
        return Some(DeclaredDep {
            identity,
            location,
            requirement,
            remote: remote_url.is_some() || local_path.is_none(),
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

/// A product declared by the package.
#[derive(Debug, Deserialize)]
pub struct Product {
    pub name: String,
}

/// A target declared by the package.
#[derive(Debug, Deserialize)]
pub struct Target {
    pub name: String,
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
    let dir = package_dir(container).unwrap_or_else(|| PathBuf::from("."));
    manifest_at(&dir)
}

/// Evaluate the `Package.swift` at an explicit directory — e.g. a resolved
/// dependency checkout, so `dependency add` can read the package's real products
/// before linking them.
///
/// The dump runs with a throwaway scratch path and `TMPDIR`
/// ([`sweetpad_core::package_members::run_dump_package`]), so reading a package
/// leaves no `.build/` in it and nothing in the user's `$TMPDIR`.
pub fn manifest_at(package_path: &Path) -> Result<Manifest, CliError> {
    let dump = sweetpad_core::package_members::dump_manifest(
        package_path,
        &Toolchain::default(),
        Stdio::inherit(),
    )
    .map_err(dump_error)?;
    serde_json::from_value(dump)
        .map_err(|e| CliError::new(format!("parsing swift package dump-package: {e}")))
}

/// What the package offers when opened on its own: its name, its schemes the
/// way `xcodebuild -list` prints them in its directory (the `.swiftpm/xcode`
/// scheme files included), and its targets.
pub fn package_names(container: &Container) -> Result<PackageNames, CliError> {
    let dir = package_dir(container).unwrap_or_else(|| PathBuf::from("."));
    sweetpad_core::package_members::standalone(&dir, &Toolchain::default(), Stdio::inherit())
        .map_err(dump_error)
}

/// A failed manifest read as the CLI reports it: a `swift` that can't be
/// spawned is a missing tool, anything else says what the dump did.
fn dump_error(error: DumpError) -> CliError {
    match error {
        DumpError::Spawn(e) => process::spawn_error("swift", &e),
        other => CliError::new(other.to_string()),
    }
}

/// `swift package add-dependency <dependency> …` (Swift 6+). `requirement` is
/// the already-assembled SwiftPM flag list for a remote URL (e.g. `["--from",
/// "1.2.3"]`); `None` adds `dependency` as a local path, which SwiftPM writes
/// into the manifest verbatim and resolves against the package root. Streams
/// output to the terminal; `quiet` discards stdout (machine modes own stdout —
/// Swift 6 prints progress lines).
pub fn add_dependency(
    container: &Container,
    dependency: &str,
    requirement: Option<&[String]>,
    quiet: bool,
) -> Result<(), CliError> {
    let cwd = package_dir(container);
    let args = add_dependency_args(dependency, requirement);
    if process::run("swift", &args, cwd.as_deref(), quiet)? {
        Ok(())
    } else {
        Err(
            CliError::new("swift package add-dependency exited with a non-zero status")
                .context("adding the package dependency"),
        )
    }
}

/// The argv for [`add_dependency`]. A local dependency names its type: SwiftPM
/// reads any other argument as a URL and refuses it without a version
/// requirement.
fn add_dependency_args<'a>(dependency: &'a str, requirement: Option<&'a [String]>) -> Vec<&'a str> {
    let mut args = vec!["package", "add-dependency", dependency];
    match requirement {
        Some(flags) => args.extend(flags.iter().map(String::as_str)),
        None => args.extend(["--type", "path"]),
    }
    args
}

/// `swift package add-target-dependency <product> <target> --package <name>`
/// (Swift 6+) — link a product of an added package into a target. `quiet`
/// discards stdout (machine modes).
pub fn add_target_dependency(
    container: &Container,
    product: &str,
    target: &str,
    package: &str,
    quiet: bool,
) -> Result<(), CliError> {
    let cwd = package_dir(container);
    let args = [
        "package",
        "add-target-dependency",
        product,
        target,
        "--package",
        package,
    ];
    if process::run("swift", &args, cwd.as_deref(), quiet)? {
        Ok(())
    } else {
        Err(
            CliError::new("swift package add-target-dependency exited with a non-zero status")
                .context("linking the product to the target"),
        )
    }
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
    parse_swift_major(&swift_version()?)
}

/// What `swift --version` prints to stdout, or `None` when it can't run or
/// fails.
///
/// Stderr is captured too: the driver writes `swift-driver version: 1.168.6 `
/// there with no newline (Swift 6.4), which would otherwise run into the next
/// line sweetpad writes there — under `--json`, the error envelope. The driver
/// also leaves a `TemporaryDirectory.*` in `$TMPDIR`, since it hands
/// `--version` to a `swift-frontend` that takes its place, so the probe gets a
/// `TMPDIR` of its own that goes when it's done.
#[must_use]
pub fn swift_version() -> Option<String> {
    let scratch = ScratchDir::new("sweetpad-swift-version").ok()?;
    let output = Command::new("swift")
        .arg("--version")
        .env("TMPDIR", scratch.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parse the major version from `swift --version` output, e.g. "Apple Swift
/// version 6.0.3 (...)" or "Swift version 6.1-dev" → `6`.
fn parse_swift_major(text: &str) -> Option<u32> {
    let after = text.split("Swift version ").nth(1)?;
    let number = after.trim_start();
    let major: String = number.chars().take_while(char::is_ascii_digit).collect();
    major.parse().ok()
}

/// Scheme candidates for a package, read from its manifest and its scheme
/// container so no xcodebuild (or even a full Xcode) is needed. See
/// [`package_names`].
pub fn schemes(container: &Container) -> Result<Vec<String>, CliError> {
    Ok(package_names(container)?.schemes)
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

/// The `swift build` argv for a package — shared by [`build`] and the
/// `--show-command` preview, so the dry run prints exactly what would run.
/// `build_tests` adds `--build-tests`, which compiles the test targets too.
#[must_use]
pub fn build_args(configuration: &str, build_tests: bool, passthrough: &[String]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "build".into(),
        "--configuration".into(),
        configuration_arg(configuration).into(),
    ];
    if build_tests {
        args.push("--build-tests".into());
    }
    args.extend(passthrough.iter().cloned());
    args
}

/// `swift build` for a package. Streams output to the terminal unless `quiet`
/// (for `--json` callers whose stdout must hold only the result envelope).
/// `clean` wipes the build directory first — SwiftPM has no `build --clean`, so
/// it's a separate `package clean`. `passthrough` (everything after `--`) rides
/// through verbatim, e.g. `-Xswiftc -DFOO`.
pub fn build(
    container: &Container,
    configuration: &str,
    build_tests: bool,
    clean: bool,
    quiet: bool,
    passthrough: &[String],
) -> Result<(), CliError> {
    let cwd = package_dir(container);
    if clean {
        // The user explicitly asked for a clean build — a failed clean
        // followed by an incremental build would silently hand back stale
        // artifacts, so a non-zero `swift package clean` is an error here.
        let cleaned = process::run("swift", &["package", "clean"], cwd.as_deref(), quiet)
            .context("cleaning the package build")?;
        if !cleaned {
            return Err(
                CliError::new("swift package clean exited with a non-zero status")
                    .context("cleaning the package build"),
            );
        }
    }
    let args = build_args(configuration, build_tests, passthrough);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let ok =
        process::run("swift", &arg_refs, cwd.as_deref(), quiet).context("building the package")?;
    if ok {
        Ok(())
    } else {
        Err(CliError::new("swift build exited with a non-zero status")
            .context("building the package"))
    }
}

/// The `swift test` argv for a package — shared by [`test`] and the
/// `--show-command` preview.
#[must_use]
pub fn test_args(
    configuration: &str,
    only: &[String],
    skip: &[String],
    coverage: bool,
    passthrough: &[String],
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "test".into(),
        "--configuration".into(),
        configuration_arg(configuration).into(),
    ];
    if coverage {
        args.push("--enable-code-coverage".into());
    }
    for f in only {
        args.push("--filter".into());
        args.push(f.clone());
    }
    for s in skip {
        args.push("--skip".into());
        args.push(s.clone());
    }
    args.extend(passthrough.iter().cloned());
    args
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
    coverage: bool,
    quiet: bool,
    passthrough: &[String],
) -> Result<bool, CliError> {
    let cwd = package_dir(container);
    let args = test_args(configuration, only, skip, coverage, passthrough);
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
        let m: Manifest = serde_json::from_str(DUMP).unwrap();
        assert_eq!(m.name, "Demo");
        let products: Vec<&str> = m.products.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(products, ["DemoKit", "demo"]);
        let targets: Vec<&str> = m.targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(targets, ["DemoKit", "demo", "DemoKitTests"]);
    }

    #[test]
    fn a_test_build_asks_swift_build_for_the_test_targets() {
        let tail = vec!["-Xswiftc".to_string(), "-DFOO".to_string()];
        assert_eq!(
            build_args("Debug", true, &tail),
            [
                "build",
                "--configuration",
                "debug",
                "--build-tests",
                "-Xswiftc",
                "-DFOO"
            ]
        );
        assert_eq!(
            build_args("Release", false, &[]),
            ["build", "--configuration", "release"]
        );
    }

    #[test]
    fn configuration_maps_to_debug_or_release() {
        assert_eq!(configuration_arg("Release"), "release");
        assert_eq!(configuration_arg("release"), "release");
        assert_eq!(configuration_arg("Debug"), "debug");
        assert_eq!(configuration_arg("Anything"), "debug");
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
    fn a_local_dependency_is_added_as_a_path() {
        assert_eq!(
            add_dependency_args("../Dep", None),
            ["package", "add-dependency", "../Dep", "--type", "path"]
        );
        let from = ["--from".to_string(), "1.2.3".to_string()];
        assert_eq!(
            add_dependency_args("https://example.com/dep.git", Some(&from)),
            [
                "package",
                "add-dependency",
                "https://example.com/dep.git",
                "--from",
                "1.2.3"
            ]
        );
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
