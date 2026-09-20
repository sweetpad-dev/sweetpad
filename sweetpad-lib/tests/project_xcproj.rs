//! Reading a project in the `project.xcproj` format: the model `project::open`
//! builds, the layers `project::build_settings` returns, and the two ways a
//! bundle can be malformed.
//!
//! The Xcode-authored fixtures under `fixtures/_xcproj` are bundles without
//! their source trees, so anything that has to resolve a path on disk — an
//! xcconfig, a package directory — is written out in a temporary directory
//! instead.

use std::fs;
use std::path::{Path, PathBuf};

use sweetpad_lib::project::{self, build_settings, open};

const MANIFEST: &str = "// swift-tools-version:5.9\nimport PackageDescription\n";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/_xcproj")
        .join(name)
}

/// Write a project bundle holding `document`, plus the files beside it.
fn scratch(dir: &Path, document: &str, files: &[(&str, &str)]) -> PathBuf {
    let xcodeproj = dir.join("Scratch.xcodeproj");
    fs::create_dir_all(&xcodeproj).unwrap();
    fs::write(xcodeproj.join("project.xcproj"), document).unwrap();
    for (rel, contents) in files {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    xcodeproj
}

/// Paths relative to the project directory, so an assertion reads as the
/// document spells them.
fn relative(paths: &[PathBuf], dir: &Path) -> Vec<String> {
    // Paths come back resolved, and the temporary directory is behind a
    // symlink on macOS (`/var` → `/private/var`).
    let dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    paths
        .iter()
        .map(|p| {
            p.strip_prefix(&dir)
                .unwrap_or(p)
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sweetpad-xcproj-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn opens_an_xcode_authored_document() {
    let project = open(&fixture("SweetpadCIApp.xcodeproj")).unwrap();
    assert_eq!(project.name, "SweetpadCIApp");
    assert_eq!(project.configurations, ["Debug", "Release"]);
    assert_eq!(project.default_configuration.as_deref(), Some("Debug"));
    let names: Vec<&str> = project.targets.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        ["SweetpadCIApp", "SweetpadCIAppTests", "SweetpadCIMac"]
    );
    // Every target builds the project's configurations; a target lists one
    // only to point it at a different xcconfig.
    for target in &project.targets {
        assert_eq!(target.configurations, project.configurations);
    }
}

/// The abbreviated `product-type` regains the prefix, so the product-type
/// rules downstream match whichever format the project is in.
#[test]
fn product_types_and_isa_match_the_pbxproj_spellings() {
    let project = open(&fixture("SweetpadCIApp.xcodeproj")).unwrap();
    let by_name = |name: &str| {
        project
            .targets
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("no target {name}"))
    };
    assert_eq!(
        by_name("SweetpadCIApp").product_type.as_deref(),
        Some("com.apple.product-type.application")
    );
    assert_eq!(
        by_name("SweetpadCIAppTests").product_type.as_deref(),
        Some("com.apple.product-type.bundle.unit-test")
    );
    assert_eq!(by_name("SweetpadCIApp").isa, "PBXNativeTarget");
}

#[test]
fn target_kinds_map_onto_the_pbxproj_isa_names() {
    let dir = tempdir("kinds");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [ "Debug" ],
  "targets": [
    { "name": "Native", "product-type": "application" },
    { "name": "Agg", "kind": "aggregate" },
    { "name": "Legacy", "kind": "external-build-system" },
    { "name": "Foreign", "full-product-type": "com.example.product-type.thing" },
  ],
}
"#,
        &[],
    );
    let project = open(&xcodeproj).unwrap();
    let isa: Vec<&str> = project.targets.iter().map(|t| t.isa.as_str()).collect();
    assert_eq!(
        isa,
        [
            "PBXNativeTarget",
            "PBXAggregateTarget",
            "PBXLegacyTarget",
            "PBXNativeTarget"
        ]
    );
    // A product type Xcode does not recognize is stored whole, not abbreviated.
    assert_eq!(
        project.targets[3].product_type.as_deref(),
        Some("com.example.product-type.thing")
    );
}

/// A configuration moved into the setting's key, which is where the format
/// puts what used to be a `buildSettings` dict per configuration.
#[test]
fn settings_are_narrowed_to_the_configuration() {
    let dir = tempdir("conditions");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [ "Debug", "Release" ],
  "targets": [
    {
      "name": "App",
      "product-type": "application",
      "build-settings": {
        "OTHER_SWIFT_FLAGS[sdk=iphoneos*]": "-DPHONE",
        "SWIFT_OPTIMIZATION_LEVEL": "-O",
        "SWIFT_OPTIMIZATION_LEVEL[config=Debug]": "-Onone",
      },
    },
  ],
}
"#,
        &[],
    );
    let inline = |config: &str| {
        build_settings(&xcodeproj, "App", config).unwrap().layers[3]
            .iter()
            .map(|a| {
                (
                    a.key.clone(),
                    a.value.clone(),
                    a.conditions
                        .iter()
                        .map(|c| format!("{}={}", c.key, c.value))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    // Settings are stored byte-sorted, so the conditional key follows the
    // plain one and wins within the layer — how xcodebuild reads them.
    assert_eq!(
        inline("Debug"),
        [
            (
                "OTHER_SWIFT_FLAGS".to_string(),
                "-DPHONE".to_string(),
                vec!["sdk=iphoneos*".to_string()]
            ),
            (
                "SWIFT_OPTIMIZATION_LEVEL".to_string(),
                "-O".to_string(),
                vec![]
            ),
            (
                "SWIFT_OPTIMIZATION_LEVEL".to_string(),
                "-Onone".to_string(),
                vec![]
            ),
        ]
    );
    let release = inline("Release");
    assert!(
        !release.iter().any(|(_, value, _)| value == "-Onone"),
        "the Debug-only value leaked into Release: {release:?}"
    );
}

/// A list-valued setting joins with spaces, the form xcodebuild consumes.
#[test]
fn list_settings_join_with_spaces() {
    let dir = tempdir("lists");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [ "Debug" ],
  "targets": [
    {
      "name": "App",
      "build-settings": {
        "LD_RUNPATH_SEARCH_PATHS": [ "$(inherited)", "@executable_path/Frameworks" ],
      },
    },
  ],
}
"#,
        &[],
    );
    let layers = build_settings(&xcodeproj, "App", "Debug").unwrap().layers;
    assert_eq!(layers[3][0].key, "LD_RUNPATH_SEARCH_PATHS");
    assert_eq!(
        layers[3][0].value,
        "$(inherited) @executable_path/Frameworks"
    );
}

/// A configuration's `file` is a path through the navigator, not through the
/// filesystem: CocoaPods gives its xcconfigs a display name that is the bare
/// basename of a path several directories down.
#[test]
fn xcconfig_resolves_through_the_navigator_tree() {
    let dir = tempdir("xcconfig");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [
    { "name": "Debug", "file": "Pods/Pods-App.debug.xcconfig" },
  ],
  "files": [
    {
      "kind": "group",
      "path": "Pods",
      "children": [
        { "path": "Target Support Files/Pods-App/Pods-App.debug.xcconfig" },
      ],
    },
  ],
  "targets": [ { "name": "App", "product-type": "application" } ],
}
"#,
        &[(
            "Pods/Target Support Files/Pods-App/Pods-App.debug.xcconfig",
            "PODS_ROOT = ${SRCROOT}/Pods\n",
        )],
    );
    let layers = build_settings(&xcodeproj, "App", "Debug").unwrap().layers;
    assert_eq!(
        layers[0].len(),
        1,
        "project xcconfig layer: {:?}",
        layers[0]
    );
    assert_eq!(layers[0][0].key, "PODS_ROOT");
}

/// An xcconfig inside a synchronized folder has no node of its own, so the
/// document names the folder and the path within it.
#[test]
fn xcconfig_resolves_through_a_synchronized_folder_anchor() {
    let dir = tempdir("anchor");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [
    { "name": "Debug", "file": { "anchor": "config", "relative-path": "Shared.xcconfig" } },
  ],
  "files": [
    { "kind": "folder", "path": "config" },
  ],
  "targets": [ { "name": "App", "product-type": "application" } ],
}
"#,
        &[("config/Shared.xcconfig", "SHARED = yes\n")],
    );
    let layers = build_settings(&xcodeproj, "App", "Debug").unwrap().layers;
    assert_eq!(
        layers[0].len(),
        1,
        "project xcconfig layer: {:?}",
        layers[0]
    );
    assert_eq!(layers[0][0].key, "SHARED");
}

/// An xcconfig the project names but that isn't on disk resolves to an empty
/// layer, the way xcodebuild carries on after warning. A CocoaPods project
/// before `pod install` is the usual case.
#[test]
fn a_missing_xcconfig_is_an_empty_layer() {
    let dir = tempdir("missing-xcconfig");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [ { "name": "Debug", "file": "Gone.xcconfig" } ],
  "targets": [ { "name": "App", "product-type": "application" } ],
}
"#,
        &[],
    );
    assert!(build_settings(&xcodeproj, "App", "Debug").unwrap().layers[0].is_empty());
}

/// A local package reaches the model from either place it can appear: the
/// `packages` list, or a folder in the file tree that holds a `Package.swift`.
#[test]
fn local_packages_come_from_both_the_list_and_the_tree() {
    let dir = tempdir("packages");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [ "Debug" ],
  "packages": [
    { "kind": "local", "path": "Declared" },
    { "kind": "remote", "repository": "https://example.com/Remote.git" },
  ],
  "files": [
    { "kind": "folder", "path": "Modules" },
    { "path": "NotAPackage" },
  ],
  "targets": [ { "name": "App", "product-type": "application" } ],
}
"#,
        &[
            ("Declared/Package.swift", MANIFEST),
            ("Modules/Synced/Package.swift", MANIFEST),
            ("NotAPackage/README.md", "no manifest here\n"),
        ],
    );
    let refs: Vec<String> = open(&xcodeproj)
        .unwrap()
        .package_refs
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(refs, ["Declared", "Synced"]);
}

/// An unknown configuration name is not fatal: xcodebuild warns and uses the
/// project's default, and so does this.
#[test]
fn an_unknown_configuration_falls_back_to_the_default() {
    let dir = tempdir("fallback");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "default-configuration": "Release",
  "configurations": [ "Debug", "Release" ],
  "targets": [
    {
      "name": "App",
      "build-settings": {
        "MARK[config=Debug]": "debug",
        "MARK[config=Release]": "release",
      },
    },
  ],
}
"#,
        &[],
    );
    let layers = build_settings(&xcodeproj, "App", "Nonexistent")
        .unwrap()
        .layers;
    assert_eq!(layers[3].len(), 1);
    assert_eq!(layers[3][0].value, "release");
}

/// Xcode treats a bundle holding both definitions as invalid rather than
/// preferring one, and so does this.
#[test]
fn a_bundle_holding_both_documents_is_an_error() {
    let dir = tempdir("both");
    let xcodeproj = scratch(&dir, "{ \"configurations\": [ \"Debug\" ] }\n", &[]);
    fs::write(xcodeproj.join("project.pbxproj"), "// !$*UTF8*$!\n{}\n").unwrap();
    let error = open(&xcodeproj).unwrap_err().to_string();
    assert!(
        error.contains("both project.pbxproj and project.xcproj"),
        "{error}"
    );
}

/// A caller that reaches for the pbxproj itself is told which format it found,
/// rather than that the pbxproj is a missing file.
#[test]
fn asking_for_the_pbxproj_names_the_format() {
    let error = project::parse_pbxproj(&fixture("SweetpadCIApp.xcodeproj"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("project.xcproj format"), "{error}");
}

#[test]
fn a_missing_target_is_named() {
    let error = build_settings(&fixture("SweetpadCIApp.xcodeproj"), "Nope", "Debug")
        .unwrap_err()
        .to_string();
    assert!(error.contains("Nope"), "{error}");
}

/// Membership lives on the file, not in the phase: a node names
/// `<target>/<phase>`, in a bare string or under `build-phase` when the
/// membership carries attributes.
#[test]
fn sources_come_from_per_file_membership() {
    let dir = tempdir("sources");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [ "Debug" ],
  "files": [
    { "path": "App.swift", "target-membership": [ "App/compile-sources" ] },
    {
      "kind": "group",
      "path": "Sources",
      "children": [
        { "path": "Model.swift", "target-membership": [ "App/compile-sources" ] },
        { "path": "Other.swift", "target-membership": [ "Helper/compile-sources" ] },
        { "path": "Readme.md" },
        {
          "path": "Attributed.swift",
          "target-membership": [ { "build-phase": "App/compile-sources" } ],
        },
      ],
    },
  ],
  "targets": [
    { "name": "App", "product-type": "application" },
    { "name": "Helper", "product-type": "tool" },
  ],
}
"#,
        &[],
    );
    assert_eq!(
        relative(
            &project::target_source_files(&xcodeproj, "App").unwrap(),
            &dir
        ),
        [
            "App.swift",
            "Sources/Model.swift",
            "Sources/Attributed.swift"
        ]
    );
    assert_eq!(
        relative(
            &project::target_source_files(&xcodeproj, "Helper").unwrap(),
            &dir
        ),
        ["Sources/Other.swift"]
    );
}

/// A synchronized folder lists no members. Its `target-membership` names the
/// targets that take everything under it, and `membership-exceptions` adjusts
/// that per target: `exclusions` drop a file from a default member,
/// `inclusions` hand named files to a target that is not one.
#[test]
fn a_synchronized_folder_honours_both_kinds_of_exception() {
    let dir = tempdir("synchronized");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [ "Debug" ],
  "files": [
    {
      "kind": "folder",
      "path": "Shared",
      "target-membership": [ "App" ],
      "membership-exceptions": [
        { "target": "App", "exclusions": [ "MacOnly.swift" ] },
        { "target": "Extension", "inclusions": [ "Common.swift", "Icon.png" ] },
      ],
    },
  ],
  "targets": [
    { "name": "App", "product-type": "application" },
    { "name": "Extension", "product-type": "app-extension" },
  ],
}
"#,
        &[
            ("Shared/Common.swift", "// common\n"),
            ("Shared/MacOnly.swift", "// mac\n"),
            ("Shared/Icon.png", "not really a png\n"),
        ],
    );
    assert_eq!(
        relative(
            &project::target_source_files(&xcodeproj, "App").unwrap(),
            &dir
        ),
        ["Shared/Common.swift"]
    );
    // The extension takes only what the exception set names it, and only the
    // compilable part of that.
    assert_eq!(
        relative(
            &project::target_source_files(&xcodeproj, "Extension").unwrap(),
            &dir
        ),
        ["Shared/Common.swift"]
    );
}

/// A linked binary sits under `<PRODUCTS>` or `<SDK>`, so its name is all there
/// is to read, and one imported from another project is in
/// `imported-products` rather than the file tree.
#[test]
fn linked_binaries_are_read_by_name() {
    let dir = tempdir("linking");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [ "Debug" ],
  "imported-products": [
    {
      "name": "Alamofire.framework",
      "project": "Alamofire.xcodeproj",
      "target": "Alamofire iOS",
      "target-membership": [ "App/frameworks" ],
    },
  ],
  "files": [
    {
      "kind": "group",
      "name": "Frameworks",
      "children": [
        { "path": "<SDK>/System/Library/Frameworks/Foundation.framework", "target-membership": [ "App/frameworks" ] },
        { "path": "<SDK>/usr/lib/libsqlite3.dylib", "target-membership": [ "App/frameworks" ] },
      ],
    },
  ],
  "targets": [ { "name": "App", "product-type": "application" } ],
}
"#,
        &[],
    );
    assert_eq!(
        project::target_linked_frameworks(&xcodeproj, "App").unwrap(),
        ["Alamofire", "Foundation"]
    );
    assert_eq!(
        project::target_linked_libraries(&xcodeproj, "App").unwrap(),
        ["sqlite3"]
    );
}

/// A dependency is a bare target name, or an object carrying a platform
/// filter. A package product and a cross-project target are not targets of
/// this project, so neither is a dependency edge here.
#[test]
fn dependencies_keep_only_same_project_targets() {
    let dir = tempdir("deps");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [ "Debug" ],
  "targets": [
    {
      "name": "App",
      "product-type": "application",
      "dependencies": [
        "Lib",
        { "target": "Filtered", "platforms": [ "ios" ] },
        { "kind": "package", "product-name": "Alamofire" },
        { "kind": "remoteTarget", "project": "Other.xcodeproj", "target": "Elsewhere" },
      ],
    },
    { "name": "Lib", "product-type": "library.static", "dependencies": [ "Core" ] },
    { "name": "Filtered", "product-type": "library.static" },
    { "name": "Core", "product-type": "library.static" },
  ],
}
"#,
        &[],
    );
    assert_eq!(
        project::target_dependencies(&xcodeproj, "App").unwrap(),
        ["Lib", "Filtered"]
    );
    // Post-order: a dependency precedes everything that depends on it.
    assert_eq!(
        project::transitive_dependencies(&xcodeproj, "App").unwrap(),
        ["Core", "Lib", "Filtered"]
    );
    assert!(project::target_has_package_products(&xcodeproj, "App").unwrap());
    assert!(!project::target_has_package_products(&xcodeproj, "Lib").unwrap());
}

/// A linked package product is recorded on the target under
/// `package-product-members`, which is a different list from the `{ kind:
/// package }` dependency edge. Either one counts.
#[test]
fn a_linked_package_product_counts_as_a_package() {
    let dir = tempdir("package-members");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [ "Debug" ],
  "targets": [
    {
      "name": "App",
      "product-type": "application",
      "package-product-members": [
        { "package": "Alamofire", "product-name": "Alamofire", "build-phase": { "build-phase": "frameworks" } },
      ],
    },
  ],
}
"#,
        &[],
    );
    assert!(project::target_has_package_products(&xcodeproj, "App").unwrap());
}

/// A script phase can synthesize sources, so the module is not a plain
/// `swiftc` emit.
#[test]
fn a_script_phase_blocks_self_building() {
    let dir = tempdir("self-buildable");
    let document = r#"{
  "configurations": [ "Debug" ],
  "files": [
    { "path": "App.swift", "target-membership": [ "App/compile-sources" ] },
  ],
  "targets": [
    { "name": "App", "product-type": "application", "build-phases": [ "compile-sources"PHASES ] },
  ],
}
"#;
    let plain = scratch(&dir.join("plain"), &document.replace("PHASES", ""), &[]);
    assert!(project::is_self_buildable(&plain, "App").unwrap());

    let scripted = scratch(
        &dir.join("scripted"),
        &document.replace(
            "PHASES",
            r#", { "kind": "script", "name": "Generate", "shell": "/bin/sh", "script": "true
" }"#,
        ),
        &[],
    );
    assert!(!project::is_self_buildable(&scripted, "App").unwrap());
}

#[test]
fn a_test_bundle_names_its_host_or_falls_back_to_the_app_it_depends_on() {
    let dir = tempdir("test-host");
    let xcodeproj = scratch(
        &dir,
        r#"{
  "configurations": [ "Debug" ],
  "targets": [
    {
      "name": "App",
      "product-type": "application",
    },
    {
      "name": "Declared",
      "product-type": "bundle.unit-test",
      "test-host-target": "App",
    },
    {
      "name": "Inferred",
      "product-type": "bundle.unit-test",
      "dependencies": [ "App" ],
    },
    {
      "name": "Standalone",
      "product-type": "bundle.unit-test",
    },
  ],
}
"#,
        &[],
    );
    let host = |target: &str| {
        build_settings(&xcodeproj, target, "Debug")
            .unwrap()
            .test_host_target
    };
    assert_eq!(host("Declared").as_deref(), Some("App"));
    assert_eq!(host("Inferred").as_deref(), Some("App"));
    assert_eq!(host("Standalone"), None);
    assert_eq!(host("App"), None);
}
