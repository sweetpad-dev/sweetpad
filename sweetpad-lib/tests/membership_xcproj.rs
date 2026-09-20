//! Target membership in a `project.xcproj` document: the per-file entries a
//! node carries, and the synchronized folders that stand in for them.

use sweetpad_lib::membership::{AddOutcome, IncludeOutcome, Phase, RefKind, RemoveOutcome};
use sweetpad_lib::{membership_xcproj as files, sync_xcproj as folders, xcproj};

fn document() -> xcproj::Value {
    xcproj::parse(
        r#"{
  "configurations": [ "Debug" ],
  "files": [
    { "path": "<PROJECT>/README.md" },
    {
      "kind": "group",
      "path": "Sources",
      "children": [
        {
          "path": "App.swift",
          "target-membership": [
            "App/compile-sources",
            { "build-phase": "Mac/compile-sources", "arguments": "-warnings-as-errors" },
          ],
        },
        {
          "path": "Header.h",
          "target-membership": [
            { "build-phase": "App/headers", "header-role": "public", "platforms": [ "ios" ] },
          ],
        },
        { "kind": "folder", "path": "Shared", "target-membership": [ "App" ] },
      ],
    },
  ],
  "targets": [
    { "name": "App", "product-type": "application" },
    { "name": "Mac", "product-type": "application" },
  ],
}
"#,
    )
    .unwrap_or_else(|e| panic!("parse: {e}"))
}

#[test]
fn a_nodes_entries_read_back_with_their_details() {
    let doc = document();
    let app = files::classic_members(&doc, "App").unwrap();

    let source = &app[0];
    assert_eq!(source.path, "Sources/App.swift");
    assert_eq!(source.phase, Phase::Sources);
    assert_eq!(source.kind, RefKind::File);
    assert_eq!(source.compiler_flags, None);

    let header = &app[1];
    assert_eq!(header.path, "Sources/Header.h");
    assert_eq!(header.phase, Phase::Headers);
    assert_eq!(header.attributes, ["Public"]);
    assert_eq!(header.platform_filters, ["ios"]);

    // The object form carries the per-file compiler flags the pbxproj kept in
    // `settings.COMPILER_FLAGS`.
    let mac = files::classic_members(&doc, "Mac").unwrap();
    assert_eq!(mac.len(), 1);
    assert_eq!(
        mac[0].compiler_flags.as_deref(),
        Some("-warnings-as-errors")
    );

    // A synchronized folder is not a per-file entry.
    assert!(app.iter().all(|e| e.path != "Sources/Shared"));
}

#[test]
fn a_membership_is_added_and_taken_away_again() {
    let mut doc = document();
    let paths = vec!["README.md".to_string()];

    let added = files::add_membership(&mut doc, "Mac", &paths, &Phase::Resources).unwrap();
    assert!(!added[0].already_member);
    assert!(
        files::classic_members(&doc, "Mac")
            .unwrap()
            .iter()
            .any(|e| e.path == "README.md" && e.phase == Phase::Resources)
    );

    // Asking twice is a recorded no-op.
    let again = files::add_membership(&mut doc, "Mac", &paths, &Phase::Resources).unwrap();
    assert!(again[0].already_member);

    let removed = files::remove_membership(&mut doc, "Mac", &paths).unwrap();
    assert_eq!(removed[0].removed_phases, ["resources"]);
    // The node stays in the navigator; only a pbxproj deletes the reference.
    assert!(!removed[0].deleted_reference);
    assert!(
        files::classic_members(&doc, "Mac")
            .unwrap()
            .iter()
            .all(|e| e.path != "README.md")
    );
}

#[test]
fn a_path_the_navigator_does_not_hold_is_an_error() {
    let mut doc = document();
    let err = files::add_membership(
        &mut doc,
        "App",
        &["Sources/Missing.swift".to_string()],
        &Phase::Sources,
    )
    .unwrap_err();
    assert!(err.contains("not in the project's navigator tree"), "{err}");

    let err = files::add_membership(&mut doc, "Nope", &[], &Phase::Sources).unwrap_err();
    assert!(err.contains("no target named Nope"), "{err}");
}

/// Removing a path that was never a member reports the no-op rather than
/// failing, so a re-run script stays green.
#[test]
fn removing_a_non_member_is_a_no_op() {
    let mut doc = document();
    let removed =
        files::remove_membership(&mut doc, "Mac", &["Sources/Header.h".to_string()]).unwrap();
    assert!(removed[0].removed_phases.is_empty());
}

#[test]
fn folders_are_listed_per_target() {
    let doc = document();
    let listed = folders::list(&doc).unwrap();
    let app = listed.iter().find(|t| t.target == "App").unwrap();
    assert_eq!(app.roots.len(), 1);
    assert_eq!(app.roots[0].dir, "Sources/Shared");
    assert!(app.roots[0].exceptions.is_empty());

    let mac = listed.iter().find(|t| t.target == "Mac").unwrap();
    assert!(mac.roots.is_empty(), "targets with none are still listed");
}

#[test]
fn a_folder_is_attached_shared_and_detached() {
    let mut doc = document();

    assert_eq!(
        folders::add_root(&mut doc, "Mac", "Extras").unwrap(),
        AddOutcome::Created
    );
    assert_eq!(
        folders::add_root(&mut doc, "Mac", "Extras").unwrap(),
        AddOutcome::AlreadyAttached
    );
    assert_eq!(
        folders::add_root(&mut doc, "App", "Extras").unwrap(),
        AddOutcome::AttachedExisting
    );

    // Detaching leaves the node: a folder that builds for nothing is an
    // ordinary navigator entry in this format.
    assert_eq!(
        folders::remove_root(&mut doc, "Mac", "Extras").unwrap(),
        RemoveOutcome::Detached {
            deleted_object: false
        }
    );
    assert_eq!(
        folders::remove_root(&mut doc, "Mac", "Extras").unwrap(),
        RemoveOutcome::NotAttached
    );
    folders::remove_root(&mut doc, "App", "Extras").unwrap();
    assert!(
        xcproj::serialize(&doc).contains("Extras"),
        "the folder node survives"
    );
}

#[test]
fn an_exclusion_is_added_and_dropped() {
    let mut doc = document();

    folders::exclude(&mut doc, "App", "Sources/Shared/Ignored.swift").unwrap();
    let listed = folders::list(&doc).unwrap();
    let app = listed.iter().find(|t| t.target == "App").unwrap();
    assert_eq!(app.roots[0].exceptions, ["Ignored.swift"]);

    assert_eq!(
        folders::include(&mut doc, "App", "Sources/Shared/Ignored.swift").unwrap(),
        IncludeOutcome::Removed {
            root_dir: "Sources/Shared".into(),
            exception: "Ignored.swift".into(),
        }
    );
    // Back to where it started, key and all.
    assert_eq!(xcproj::serialize(&doc), xcproj::serialize(&document()));

    assert_eq!(
        folders::include(&mut doc, "App", "Sources/Shared/Ignored.swift").unwrap(),
        IncludeOutcome::NotExcluded
    );
}

#[test]
fn excluding_outside_every_folder_names_the_ones_there_are() {
    let mut doc = document();
    let err = folders::exclude(&mut doc, "App", "Sources/App.swift").unwrap_err();
    assert!(err.contains("not inside a synchronized folder"), "{err}");
    assert!(err.contains("Sources/Shared"), "{err}");

    let err = folders::exclude(&mut doc, "Mac", "Sources/App.swift").unwrap_err();
    assert!(err.contains("has no synchronized folders"), "{err}");
}

#[test]
fn a_folder_says_which_of_its_files_a_target_builds() {
    let doc = document();
    assert_eq!(
        folders::folder_of(&doc, "App", "Sources/Shared/Thing.swift").as_deref(),
        Some("Sources/Shared")
    );
    assert_eq!(
        folders::folder_of(&doc, "Mac", "Sources/Shared/Thing.swift"),
        None
    );
    assert_eq!(folders::folder_of(&doc, "App", "Sources/App.swift"), None);
}
