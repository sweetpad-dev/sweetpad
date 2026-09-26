//! Swift package dependencies in a `project.xcproj` document. See
//! [`sweetpad_lib::spm_xcproj`].
//!
//! The `PackageProbe` and `SpmStaticLibrary` fixture pairs are Xcode 27.2's
//! conversions of one pbxproj project each, before and after `dependency add`
//! edited it through the pbxproj backend. Making the same edits here has to
//! give the document Xcode's converter wrote, byte for byte.

use std::fs;
use std::path::PathBuf;

use sweetpad_lib::spm_xcproj::{self as spm, PackageKind, ProductLink, RequirementSpec};
use sweetpad_lib::xcproj;

fn text(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/_xcproj")
        .join(format!("{name}.xcodeproj/project.xcproj"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn fixture(name: &str) -> xcproj::Value {
    xcproj::parse(&text(name)).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

fn document(source: &str) -> xcproj::Value {
    xcproj::parse(source).unwrap_or_else(|e| panic!("parse: {e}"))
}

/// The packages the probe adds, in the order it added them, with the products
/// each links into `Probe`.
fn probe_packages() -> Vec<(&'static str, Option<RequirementSpec>, Vec<&'static str>)> {
    vec![
        (
            "https://github.com/apple/swift-collections.git",
            Some(RequirementSpec::UpToNextMajor("1.1.0".into())),
            vec!["Collections"],
        ),
        (
            "https://github.com/apple/swift-algorithms",
            Some(RequirementSpec::UpToNextMinor("1.2.0".into())),
            vec!["Algorithms"],
        ),
        (
            "https://github.com/apple/swift-numerics.git",
            Some(RequirementSpec::Exact("1.0.2".into())),
            vec!["Numerics"],
        ),
        (
            "https://github.com/apple/swift-argument-parser.git",
            Some(RequirementSpec::Range {
                from: "1.3.0".into(),
                to: "1.5.0".into(),
            }),
            vec!["ArgumentParser"],
        ),
        (
            "https://github.com/pointfreeco/swift-case-paths",
            Some(RequirementSpec::Branch("main".into())),
            vec!["CasePaths"],
        ),
        (
            "https://github.com/apple/swift-log.git",
            Some(RequirementSpec::Revision(
                "96a2f8a0fa41e9e09af4585e2724c4e825410b91".into(),
            )),
            vec!["Logging"],
        ),
        ("Packages/LocalKit", None, vec!["LocalKit", "LocalExtra"]),
    ]
}

#[test]
fn adding_every_requirement_kind_writes_what_xcode_writes() {
    let mut doc = fixture("PackageProbe");
    for (location, requirement, products) in probe_packages() {
        let name = match requirement {
            Some(spec) => spm::add_remote_dependency(&mut doc, location, &spec).unwrap(),
            None => spm::add_local_dependency(&mut doc, location).unwrap(),
        };
        for product in products {
            spm::link_product(&mut doc, &name, product, "Probe").unwrap();
        }
    }
    assert_eq!(xcproj::serialize(&doc), text("PackageProbeLinked"));
}

#[test]
fn removing_every_package_gives_back_the_document_without_them() {
    let mut doc = fixture("PackageProbeLinked");
    for (location, _, _) in probe_packages() {
        let name = spm::find_package(&doc, location).unwrap();
        spm::remove_package(&mut doc, &name, &[]).unwrap();
    }
    assert_eq!(xcproj::serialize(&doc), text("PackageProbe"));
}

/// A static library takes a package product as a dependency rather than as a
/// member of a Frameworks phase, which is what the pbxproj's
/// `PBXTargetDependency` with a `productRef` converts to.
#[test]
fn a_static_library_depends_on_the_product() {
    let mut doc = fixture("SpmStaticLibrary");
    let name = spm::add_remote_dependency(
        &mut doc,
        "https://github.com/apple/swift-collections.git",
        &RequirementSpec::UpToNextMajor("1.1.0".into()),
    )
    .unwrap();
    spm::link_product(&mut doc, &name, "Collections", "SpmApp").unwrap();
    spm::link_product(&mut doc, &name, "DequeModule", "SpmApp").unwrap();
    assert_eq!(xcproj::serialize(&doc), text("SpmStaticLibraryLinked"));

    spm::remove_package(&mut doc, &name, &[]).unwrap();
    assert_eq!(xcproj::serialize(&doc), text("SpmStaticLibrary"));
}

#[test]
fn declared_packages_read_back_with_their_requirements_and_links() {
    let doc = fixture("PackageProbeLinked");
    let packages = spm::list_packages(&doc);
    let summary: Vec<(String, String, String)> = packages
        .iter()
        .map(|p| {
            let requirement = p.requirement.as_ref().map_or_else(
                || "local".to_string(),
                sweetpad_lib::spm::Requirement::display,
            );
            (p.id.clone(), p.identity.clone(), requirement)
        })
        .collect();
    let expected = [
        ("swift-collections", "swift-collections", "from 1.1.0"),
        (
            "swift-algorithms",
            "swift-algorithms",
            "up-to-next-minor from 1.2.0",
        ),
        ("swift-numerics", "swift-numerics", "exact 1.0.2"),
        (
            "swift-argument-parser",
            "swift-argument-parser",
            "1.3.0 ..< 1.5.0",
        ),
        ("swift-case-paths", "swift-case-paths", "branch main"),
        (
            "swift-log",
            "swift-log",
            "revision 96a2f8a0fa41e9e09af4585e2724c4e825410b91",
        ),
        ("LocalKit", "localkit", "local"),
    ]
    .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()));
    assert_eq!(summary, expected);

    let local = &packages[6];
    assert_eq!(
        local.kind,
        PackageKind::Local {
            relative_path: "Packages/LocalKit".into()
        }
    );
    let link = |product: &str| ProductLink {
        product: product.into(),
        target: "Probe".into(),
    };
    assert_eq!(local.products, [link("LocalExtra"), link("LocalKit")]);
}

/// A dependency without a `package` is a local package's product that the
/// pbxproj recorded without a back-reference. It belongs to no declared
/// package until one names it by product.
#[test]
fn a_product_reference_without_a_package_is_attributed_to_none() {
    let doc = fixture("SpmStaticLibrary");
    let packages = spm::list_packages(&doc);
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].id, "Dep");
    assert!(packages[0].products.is_empty());

    let mut doc = doc;
    spm::remove_package(&mut doc, "Dep", &["Dep".to_string()]).unwrap();
    let out = xcproj::serialize(&doc);
    assert!(!out.contains("\"packages\""), "{out}");
    assert!(!out.contains("\"dependencies\""), "{out}");
}

#[test]
fn a_package_is_found_by_url_identity_or_path() {
    let doc = fixture("PackageProbeLinked");
    assert_eq!(
        spm::find_package(&doc, "https://github.com/apple/swift-log.git").as_deref(),
        Some("swift-log")
    );
    assert_eq!(
        spm::find_package(&doc, "Swift-Numerics").as_deref(),
        Some("swift-numerics")
    );
    assert_eq!(
        spm::find_package(&doc, "Packages/LocalKit").as_deref(),
        Some("LocalKit")
    );
    assert_eq!(
        spm::find_package(&doc, "localkit").as_deref(),
        Some("LocalKit")
    );
    assert_eq!(spm::find_package(&doc, "swift-nio"), None);
}

#[test]
fn a_requirement_is_replaced_in_place() {
    let mut doc = fixture("PackageProbeLinked");
    spm::set_requirement(
        &mut doc,
        "swift-numerics",
        &RequirementSpec::UpToNextMajor("1.1.0".into()),
    )
    .unwrap();
    let out = xcproj::serialize(&doc);
    assert!(
        out.contains(
            "\"repository\": \"https://github.com/apple/swift-numerics.git\",\n      \"version\": {\n        \"up-to-next-major-version\": \"1.1.0\",\n      },"
        ),
        "{out}"
    );
    let numerics = spm::list_packages(&doc)
        .into_iter()
        .find(|p| p.id == "swift-numerics")
        .unwrap();
    assert_eq!(numerics.requirement.unwrap().display(), "from 1.1.0");

    let err = spm::set_requirement(
        &mut doc,
        "LocalKit",
        &RequirementSpec::Exact("1.0.0".into()),
    )
    .unwrap_err();
    assert!(err.contains("local package"), "{err}");
}

/// Xcode writes a range as one `version-range` string only when both bounds
/// are plain dotted numbers, and as a pair of keys otherwise.
#[test]
fn a_range_with_a_prerelease_bound_is_written_as_a_pair() {
    let mut doc = fixture("PackageProbe");
    let name = spm::add_remote_dependency(
        &mut doc,
        "https://github.com/apple/swift-syntax.git",
        &RequirementSpec::Range {
            from: "600.0.0-prerelease-2024-06-12".into(),
            to: "601.0.0".into(),
        },
    )
    .unwrap();
    let out = xcproj::serialize(&doc);
    assert!(
        out.contains(
            "      \"version\": {\n        \"version-range-min\": \"600.0.0-prerelease-2024-06-12\",\n        \"version-range-max\": \"601.0.0\",\n      },"
        ),
        "{out}"
    );
    let syntax = spm::list_packages(&doc)
        .into_iter()
        .find(|p| p.id == name)
        .unwrap();
    assert_eq!(
        syntax.requirement.unwrap().display(),
        "600.0.0-prerelease-2024-06-12 ..< 601.0.0"
    );
}

#[test]
fn unlinking_narrows_to_one_product_or_one_target() {
    let mut doc = document(
        r#"{
  "packages": [
    {
      "kind": "remote",
      "repository": "https://github.com/pointfreeco/swift-composable-architecture",
      "version": {
        "up-to-next-major-version": "1.0.0",
      },
    },
  ],
  "targets": [
    {
      "name": "App",
      "build-phases": [
        "frameworks",
      ],
      "package-product-members": [
        {
          "package": "swift-composable-architecture",
          "product-name": "ComposableArchitecture",
          "build-phase": { "build-phase": "frameworks" },
        }, {
          "package": "swift-composable-architecture",
          "product-name": "Dependencies",
          "build-phase": { "build-phase": "frameworks" },
        },
      ],
    }, {
      "name": "Widget",
      "build-phases": [
        "frameworks",
      ],
      "package-product-members": [
        {
          "package": "swift-composable-architecture",
          "product-name": "ComposableArchitecture",
          "build-phase": { "build-phase": "frameworks" },
        },
      ],
    },
  ],
}
"#,
    );
    let name = "swift-composable-architecture";
    let pair = |product: &str, target: &str| (product.to_string(), target.to_string());

    let unlinked = spm::unlink(&mut doc, name, Some("Dependencies"), None).unwrap();
    assert_eq!(unlinked, [pair("Dependencies", "App")]);

    let unlinked = spm::unlink(&mut doc, name, None, Some("Widget")).unwrap();
    assert_eq!(unlinked, [pair("ComposableArchitecture", "Widget")]);

    // The emptied list goes; the package stays declared.
    let out = xcproj::serialize(&doc);
    assert_eq!(out.matches("package-product-members").count(), 1, "{out}");
    assert_eq!(spm::list_packages(&doc).len(), 1);

    assert!(
        spm::unlink(&mut doc, name, Some("Nope"), None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_target_without_a_frameworks_phase_depends_on_the_product() {
    let mut doc = document(
        r#"{
  "targets": [
    {
      "name": "Tool",
      "product-type": "tool",
      "build-phases": [
        "compile-sources",
      ],
      "build-settings": {
        "PRODUCT_NAME": "$(TARGET_NAME)",
      },
    },
  ],
}
"#,
    );
    let name = spm::add_local_dependency(&mut doc, "../Shared").unwrap();
    spm::link_product(&mut doc, &name, "Shared", "Tool").unwrap();
    // Linking again changes nothing.
    spm::link_product(&mut doc, &name, "Shared", "Tool").unwrap();
    assert_eq!(
        xcproj::serialize(&doc),
        r#"{
  "packages": [
    {
      "kind": "local",
      "path": "../Shared",
    },
  ],
  "targets": [
    {
      "name": "Tool",
      "product-type": "tool",
      "dependencies": [
        { "kind": "package", "package": "Shared", "product-name": "Shared" },
      ],
      "build-phases": [
        "compile-sources",
      ],
      "build-settings": {
        "PRODUCT_NAME": "$(TARGET_NAME)",
      },
    },
  ],
}
"#
    );
}

#[test]
fn a_package_whose_name_is_taken_is_refused() {
    let mut doc = fixture("PackageProbeLinked");
    let err = spm::add_remote_dependency(
        &mut doc,
        "https://github.com/someone-else/swift-log",
        &RequirementSpec::UpToNextMajor("1.0.0".into()),
    )
    .unwrap_err();
    assert!(err.contains("swift-log"), "{err}");

    let err = spm::link_product(&mut doc, "swift-nio", "NIO", "Probe").unwrap_err();
    assert!(err.contains("swift-nio"), "{err}");
    let err = spm::link_product(&mut doc, "swift-log", "Logging", "Nope").unwrap_err();
    assert!(err.contains("Nope"), "{err}");
}
