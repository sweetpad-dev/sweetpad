//! Local package graph oracle: the schemes and targets an Xcode container
//! draws from the Swift packages around it, grounded against real
//! `xcodebuild -list`.
//!
//! `xcodebuild` resolves the whole *local* package graph and synthesizes
//! schemes from it — a workspace's `FileRef` package members, the packages its
//! member projects declare or hold in their group tree, and everything those
//! reach through `.package(path:)`. Naming any of it means evaluating Swift
//! manifests, which is why the walk lives in
//! [`sweetpad_core::package_members`] rather than in the file-format crate.
//!
//! The fixture (`fixtures/_synthetic-spm-graph`) puts one of each shape in
//! reach of a single workspace:
//!
//! - `MultiLib`, a `FileRef` member — products and test targets;
//! - `project/Dep`, declared by the member project and carrying a
//!   `.swiftpm/xcode` scheme container — products, scheme files, test targets;
//! - `NestedDep` and `DepChild`, reached only through `.package(path:)` —
//!   products alone;
//! - `project/Modules/Nest/Synced`, two levels under a synchronized folder the
//!   pbxproj lists no members for — found by scanning the disk;
//! - `project/Modules/Tool`, whose whole manifest is one `executableTarget` —
//!   the implicit product SwiftPM synthesizes for it is a scheme;
//! - `MultiLib`'s `TC`, a target no product exposes, and `NestedDep`'s
//!   `NestedTests`, a test target in a package nobody opened — neither is a
//!   scheme.
//!
//! Both tests need the Swift toolchain (manifests are Swift source) and skip
//! cleanly without it. `SPM_LIVE_ORACLE=1` adds the comparison against
//! `xcodebuild -list -json` itself, which additionally needs Xcode.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::{JsonValue, parse_json};
use sweetpad_core::package_members::{self, PackageRole};
use sweetpad_lib::{project, workspace};

/// `xcodebuild -list -workspace Graph.xcworkspace`, in its own order.
const WORKSPACE_SCHEMES: &[&str] = &[
    "Dep",
    "DepChildLib",
    "DepTests",
    "ExecB",
    "LibA",
    "NestedLib",
    "SpmApp",
    "SyncedLib",
    "SyncedTool",
    "TATests",
];

/// Member-project targets first, then each package's, in the order the graph
/// walk reaches them: the workspace's own member, the member project's
/// package, then their path dependencies.
const WORKSPACE_TARGETS: &[&str] = &[
    "SpmApp",
    "TA",
    "TB",
    "TC",
    "TATests",
    "Dep",
    "DepTests",
    "SA",
    "SyncedTool",
    "Nested",
    "NestedTests",
    "DC",
    "DCTests",
];

/// `xcodebuild -list -project SpmApp.xcodeproj` — the same graph minus the
/// workspace's own member package.
const PROJECT_SCHEMES: &[&str] = &[
    "Dep",
    "DepChildLib",
    "DepTests",
    "SpmApp",
    "SyncedLib",
    "SyncedTool",
];

fn fixture() -> PathBuf {
    common::fixtures_root().join("_synthetic-spm-graph")
}

/// The local packages the member project reaches: the one it declares, then
/// the two under its synchronized folder, in the order the group-tree walk
/// hands them back.
fn project_packages() -> Vec<PathBuf> {
    [
        "project/Dep",
        "project/Modules/Nest/Synced",
        "project/Modules/Tool",
    ]
    .iter()
    .map(|rel| fixture().join(rel))
    .collect()
}

/// Manifests are Swift source, so every assertion here needs the toolchain.
fn have_swift() -> bool {
    Command::new("swift")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

#[test]
fn a_workspace_lists_every_local_package_it_reaches() {
    if !have_swift() {
        eprintln!("skipping: needs the Swift toolchain to evaluate package manifests");
        return;
    }
    let ws = workspace::open(&fixture().join("Graph.xcworkspace")).unwrap();
    assert_eq!(ws.package_refs, vec![fixture().join("MultiLib")]);
    assert_eq!(ws.project_package_refs(), project_packages());

    let members = package_members::resolve_workspace(&ws, None);
    assert_eq!(
        ws.merged_schemes_with_packages(&package_members::scheme_pairs(&members)),
        WORKSPACE_SCHEMES
    );
    assert_eq!(
        ws.merged_targets_with_packages(&package_members::target_pairs(&members)),
        WORKSPACE_TARGETS
    );

    // The roles behind those names: only the `FileRef` member is a member, and
    // a package reached through `.package(path:)` keeps its test targets out
    // of the scheme list.
    let member = members
        .iter()
        .find(|m| m.path.ends_with("MultiLib"))
        .expect("the workspace's own package is in the graph");
    assert_eq!(member.role, PackageRole::WorkspaceMember);
    assert_eq!(member.schemes, vec!["ExecB", "LibA", "TATests"]);
    let nested = members
        .iter()
        .find(|m| m.path.ends_with("NestedDep"))
        .expect("a member's path dependency is in the graph");
    assert_eq!(nested.role, PackageRole::Dependency);
    assert_eq!(nested.schemes, vec!["NestedLib"]);
}

#[test]
fn a_bare_project_lists_the_packages_it_declares() {
    if !have_swift() {
        eprintln!("skipping: needs the Swift toolchain to evaluate package manifests");
        return;
    }
    let proj = project::open(&fixture().join("project/SpmApp.xcodeproj")).unwrap();
    assert_eq!(proj.package_refs, project_packages());

    let members = package_members::resolve_project(&proj, None);
    assert_eq!(
        proj.schemes_with_packages(&package_members::scheme_pairs(&members)),
        PROJECT_SCHEMES
    );

    // `Dep` ships a scheme container naming a target its manifest still has,
    // which is what makes `DepTests` a scheme for a package the project only
    // depends on.
    let dep = members
        .iter()
        .find(|m| m.path.ends_with("Dep"))
        .expect("the declared package is in the graph");
    assert_eq!(dep.role, PackageRole::Dependency);
    assert_eq!(dep.schemes, vec!["Dep", "DepTests"]);
}

#[test]
fn live_xcodebuild_lists_the_same_schemes() {
    if std::env::var("SPM_LIVE_ORACLE").is_err() {
        eprintln!(
            "skipping: set SPM_LIVE_ORACLE=1 to compare against xcodebuild (needs macOS + Xcode + swift)"
        );
        return;
    }
    assert_eq!(
        xcodebuild_schemes("-workspace", "Graph.xcworkspace", "workspace"),
        WORKSPACE_SCHEMES
    );
    assert_eq!(
        xcodebuild_schemes("-project", "project/SpmApp.xcodeproj", "project"),
        PROJECT_SCHEMES
    );
}

/// The `schemes` array of `xcodebuild -list -json`, whose one top-level key is
/// `workspace` or `project` depending on what was listed.
fn xcodebuild_schemes(flag: &str, container_path: &str, container_key: &str) -> Vec<String> {
    let out = Command::new("xcodebuild")
        .args([
            "-list",
            "-json",
            flag,
            &fixture().join(container_path).display().to_string(),
        ])
        .output()
        .expect("xcodebuild runs");
    assert!(
        out.status.success(),
        "xcodebuild -list failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let start = text.find('{').expect("JSON in xcodebuild output");
    let json = parse_json(&text[start..]).expect("xcodebuild -list -json parses");
    json.as_object()
        .and_then(|o| o.get(container_key))
        .and_then(JsonValue::as_object)
        .and_then(|o| o.get("schemes"))
        .and_then(JsonValue::as_array)
        .map(|a| {
            a.iter()
                .filter_map(JsonValue::as_string)
                .map(str::to_owned)
                .collect()
        })
        .expect("a schemes array")
}
