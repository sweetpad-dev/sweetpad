//! The navigator tree of a `project.xcproj` document: nodes addressed by their
//! path through it, since the format writes an id only on the products a
//! target points at.

use sweetpad_lib::tree::{AddGroupOutcome, AddRefOutcome, MoveOutcome};
use sweetpad_lib::{tree_xcproj as tree, xcproj};

const SOURCE: &str = r#"{
  "configurations": [ "Debug" ],
  "files": [
    { "path": "<PROJECT>/Sources/Deep/Nested/Deep.swift" },
    {
      "kind": "group",
      "path": "Sources",
      "children": [
        { "path": "App.swift", "target-membership": [ "App/compile-sources" ] },
        {
          "kind": "group",
          "path": "App",
          "children": [
            { "path": "ContentView.swift", "type": "sourcecode.swift" },
          ],
        },
      ],
    }, {
      "kind": "group",
      "name": "Products",
      "children": [
        { "path": "<PRODUCTS>/App.app", "id": "958E16DD736EB85C16C05DE6", "index": false },
      ],
    },
    { "kind": "folder", "path": "Shared", "target-membership": [ "App" ] },
  ],
  "targets": [
    { "name": "App", "product-type": "application" },
  ],
}
"#;

fn document() -> xcproj::Value {
    xcproj::parse(SOURCE).unwrap_or_else(|e| panic!("parse: {e}"))
}

fn addresses(doc: &xcproj::Value) -> Vec<String> {
    tree::list_filerefs(doc)
        .unwrap()
        .into_iter()
        .map(|r| r.address)
        .collect()
}

#[test]
fn a_node_is_addressed_by_where_it_sits_not_by_where_it_lives() {
    let doc = document();
    let refs = tree::list_filerefs(&doc).unwrap();
    assert_eq!(
        refs.iter().map(|r| r.address.as_str()).collect::<Vec<_>>(),
        [
            "Deep.swift",
            "Sources/App.swift",
            "Sources/App/ContentView.swift",
            "Products/App.app",
        ]
    );

    // A file listed at the navigator root but stored somewhere else on disk
    // shows under its basename and resolves to the path it actually has.
    let deep = &refs[0];
    assert_eq!(deep.resolved, "Sources/Deep/Nested/Deep.swift");
    assert_eq!(deep.source_tree, "<PROJECT>");
    assert_eq!(deep.parent, None);

    let app = &refs[1];
    assert_eq!(app.resolved, "Sources/App.swift");
    assert_eq!(app.source_tree, "<group>");
    assert_eq!(app.parent.as_deref(), Some("Sources"));
    assert_eq!(app.build_files, 1, "one membership names it");

    // The id the format does write rides along; it is not the address.
    let product = &refs[3];
    assert_eq!(product.id.as_deref(), Some("958E16DD736EB85C16C05DE6"));
    assert_eq!(product.resolved, "<PRODUCTS>/App.app");
    assert_eq!(product.source_tree, "<PRODUCTS>");

    assert_eq!(
        refs[2].file_type.as_deref(),
        Some("sourcecode.swift"),
        "the node's own type, where it states one"
    );
    // A synchronized folder is `pbxproj folder`'s node, not a file reference.
    assert!(refs.iter().all(|r| r.address != "Shared"));
}

#[test]
fn groups_list_with_their_directories_and_their_children() {
    let doc = document();
    let groups = tree::list_groups(&doc).unwrap();
    assert_eq!(
        groups
            .iter()
            .map(|g| g.address.as_str())
            .collect::<Vec<_>>(),
        ["Sources", "Sources/App", "Products"]
    );

    let sources = &groups[0];
    assert_eq!(sources.resolved, "Sources");
    assert_eq!(sources.children, ["Sources/App.swift", "Sources/App"]);

    // An organizational group contributes no directory, so it resolves to its
    // parent's — which is why the resolved directory cannot be the address.
    let products = &groups[2];
    assert_eq!(products.name.as_deref(), Some("Products"));
    assert_eq!(products.path, None);
    assert_eq!(products.resolved, "");
}

#[test]
fn a_file_is_added_under_a_group_and_taken_away_again() {
    let mut doc = document();
    let outcome =
        tree::add_fileref(&mut doc, "New.swift", None, "<group>", Some("Sources/App")).unwrap();
    assert_eq!(
        outcome,
        AddRefOutcome::Created {
            address: "Sources/App/New.swift".into(),
            resolved: "Sources/App/New.swift".into(),
            attached_to: Some("Sources/App".into()),
        }
    );
    assert!(addresses(&doc).contains(&"Sources/App/New.swift".to_string()));

    // Asking twice says so rather than listing the file twice.
    assert_eq!(
        tree::add_fileref(&mut doc, "New.swift", None, "<group>", Some("Sources/App")).unwrap(),
        AddRefOutcome::AlreadyExists {
            address: "Sources/App/New.swift".into(),
            resolved: "Sources/App/New.swift".into(),
        }
    );

    tree::remove_fileref(&mut doc, "Sources/App/New.swift", false).unwrap();
    assert_eq!(
        xcproj::serialize(&doc),
        SOURCE,
        "add then remove is byte for byte where it started"
    );
}

#[test]
fn an_anchor_is_written_in_this_formats_spelling() {
    let mut doc = document();
    tree::add_fileref(&mut doc, "Vendor/Lib.a", None, "SOURCE_ROOT", None).unwrap();
    assert!(
        xcproj::serialize(&doc).contains(r#"{ "path": "<PROJECT>/Vendor/Lib.a" }"#),
        "{}",
        xcproj::serialize(&doc)
    );

    let err = tree::add_fileref(&mut doc, "Gen.swift", None, "DERIVED_FILE_DIR", None).unwrap_err();
    assert!(err.contains("DERIVED_FILE_DIR has no spelling"), "{err}");
    assert!(err.contains("<PROJECT>"), "{err}");
}

#[test]
fn a_group_is_added_with_or_without_a_directory() {
    let mut doc = document();
    assert_eq!(
        tree::add_group(&mut doc, "Views", Some("Sources"), Some("Views"), "<group>").unwrap(),
        AddGroupOutcome::Created {
            address: "Sources/Views".into(),
            resolved: "Sources/Views".into(),
        }
    );
    // A group that titles rather than paths resolves to its parent's directory.
    assert_eq!(
        tree::add_group(&mut doc, "Frameworks", None, None, "<group>").unwrap(),
        AddGroupOutcome::Created {
            address: "Frameworks".into(),
            resolved: String::new(),
        }
    );
    let text = xcproj::serialize(&doc);
    assert!(text.contains(r#""path": "Views""#), "{text}");
    assert!(
        !text.contains(r#""name": "Views""#),
        "a name that only repeats the path"
    );
    assert!(text.contains(r#""name": "Frameworks""#), "{text}");
}

#[test]
fn a_group_still_holding_children_is_not_deleted_out_from_under_them() {
    let mut doc = document();
    let err = tree::remove_group(&mut doc, "Sources", false).unwrap_err();
    assert!(err.contains("still holds 2 child node(s)"), "{err}");
    assert!(err.contains("pbxproj group move"), "{err}");

    // `--orphan-children` is the pbxproj's override and has nothing to
    // override here: these children are inside the group, not listed by it.
    let forced = tree::remove_group(&mut doc, "Sources", true).unwrap_err();
    assert!(
        forced.contains("--orphan-children cannot apply"),
        "{forced}"
    );

    tree::remove_fileref(&mut doc, "Sources/App/ContentView.swift", false).unwrap();
    tree::remove_group(&mut doc, "Sources/App", false).unwrap();
    assert!(
        tree::list_groups(&doc)
            .unwrap()
            .iter()
            .all(|g| g.address != "Sources/App")
    );
}

#[test]
fn deleting_a_node_a_target_builds_asks_first() {
    let mut doc = document();
    let err = tree::remove_fileref(&mut doc, "Sources/App.swift", false).unwrap_err();
    assert!(err.contains("still built by 1 target membership"), "{err}");
    assert!(err.contains("--dangling"), "{err}");

    tree::remove_fileref(&mut doc, "Sources/App.swift", true).unwrap();
    assert!(!addresses(&doc).contains(&"Sources/App.swift".to_string()));
}

#[test]
fn the_wrong_verb_for_the_node_names_the_right_one() {
    let mut doc = document();
    let err = tree::remove_fileref(&mut doc, "Sources", false).unwrap_err();
    assert!(err.contains("pbxproj group remove"), "{err}");

    let err = tree::remove_fileref(&mut doc, "Shared", false).unwrap_err();
    assert!(err.contains("pbxproj folder remove"), "{err}");

    let err = tree::remove_group(&mut doc, "Sources/App.swift", false).unwrap_err();
    assert!(err.contains("pbxproj fileref remove"), "{err}");
}

#[test]
fn moving_a_node_keeps_the_file_it_resolves_to() {
    let mut doc = document();

    // To the navigator root, where a relative path is read from the project
    // directory — so the spelling that reaches the file is the plain one.
    assert_eq!(
        tree::move_node(&mut doc, "Sources/App/ContentView.swift", None).unwrap(),
        MoveOutcome::Moved {
            address: "ContentView.swift".into(),
            from: Some("Sources/App".into()),
            to: String::new(),
            resolved: "Sources/App/ContentView.swift".into(),
        }
    );
    assert!(
        xcproj::serialize(&doc).contains(r#""path": "Sources/App/ContentView.swift""#),
        "{}",
        xcproj::serialize(&doc)
    );

    // Back under the group whose directory contains it: the directory comes
    // off the front again, and the document is where it started.
    tree::move_node(&mut doc, "ContentView.swift", Some("Sources/App")).unwrap();
    assert_eq!(
        xcproj::serialize(&doc),
        SOURCE,
        "a move and its reverse leave the document as it was"
    );
}

/// Under a group that does not contain the file, no relative spelling reaches
/// it, so the node is anchored at the project root instead.
#[test]
fn a_move_that_relative_spelling_cannot_follow_anchors_the_path() {
    let mut doc = document();
    tree::add_group(&mut doc, "Tests", None, Some("Tests"), "<group>").unwrap();
    tree::move_node(&mut doc, "Sources/App/ContentView.swift", Some("Tests")).unwrap();

    let text = xcproj::serialize(&doc);
    assert!(
        text.contains(r#""path": "<PROJECT>/Sources/App/ContentView.swift""#),
        "{text}"
    );
    let moved = tree::list_filerefs(&doc)
        .unwrap()
        .into_iter()
        .find(|r| r.address == "Tests/ContentView.swift")
        .expect("the node moved");
    assert_eq!(moved.resolved, "Sources/App/ContentView.swift");
}

#[test]
fn an_already_anchored_path_is_left_alone_by_a_move() {
    let mut doc = document();
    let moved = tree::move_node(&mut doc, "Deep.swift", Some("Sources")).unwrap();
    assert_eq!(
        moved,
        MoveOutcome::Moved {
            address: "Sources/Deep.swift".into(),
            from: None,
            to: "Sources".into(),
            resolved: "Sources/Deep/Nested/Deep.swift".into(),
        }
    );
    assert!(
        xcproj::serialize(&doc).contains(r#""path": "<PROJECT>/Sources/Deep/Nested/Deep.swift""#)
    );

    assert_eq!(
        tree::move_node(&mut doc, "Sources/Deep.swift", Some("Sources")).unwrap(),
        MoveOutcome::AlreadyThere {
            address: "Sources/Deep.swift".into(),
            group: "Sources".into(),
        }
    );
}

#[test]
fn a_group_cannot_be_moved_inside_itself() {
    let mut doc = document();
    let err = tree::move_node(&mut doc, "Sources", Some("Sources/App")).unwrap_err();
    assert!(err.contains("cannot hold itself"), "{err}");
}

#[test]
fn an_object_id_as_an_address_says_which_format_this_is() {
    let mut doc = document();
    let err = tree::remove_fileref(&mut doc, "958E16DD736EB85C16C05DE6", false).unwrap_err();
    assert!(err.contains("looks like a pbxproj object id"), "{err}");
    assert!(err.contains("navigator path"), "{err}");

    // An ordinary miss points at the node it probably meant.
    let err = tree::remove_fileref(&mut doc, "App.swift", false).unwrap_err();
    assert!(err.contains("did you mean Sources/App.swift"), "{err}");
}

/// Xcode fixes a node's key order by position, not alphabetically: `type`
/// before `target-membership`, `children` last. 1,903 corpus nodes agree, so a
/// node this crate writes has to land the same way or every later Xcode touch
/// shows up as a reordering diff.
#[test]
fn a_written_node_keeps_xcodes_key_order() {
    let mut doc = document();
    tree::add_fileref(
        &mut doc,
        "New.swift",
        Some("sourcecode.swift"),
        "<group>",
        Some("Sources"),
    )
    .unwrap();
    sweetpad_lib::membership_xcproj::add_membership(
        &mut doc,
        "App",
        &["Sources/New.swift".to_string()],
        &sweetpad_lib::membership::Phase::Sources,
    )
    .unwrap();
    assert!(
        xcproj::serialize(&doc).contains(
            r#"{ "path": "New.swift", "type": "sourcecode.swift", "target-membership": [ "App/compile-sources" ] }"#
        ),
        "{}",
        xcproj::serialize(&doc)
    );

    // A folder states what it builds before the exceptions to it, and an
    // exception states the target before the files.
    let mut doc = document();
    sweetpad_lib::sync_xcproj::exclude(&mut doc, "App", "Shared/Skip.swift").unwrap();
    let text = xcproj::serialize(&doc);
    let folder = text
        .split(r#""path": "Shared""#)
        .nth(1)
        .expect("the folder");
    let order: Vec<&str> = [
        "target-membership",
        "membership-exceptions",
        "target",
        "exclusions",
    ]
    .into_iter()
    .filter(|k| folder.contains(&format!("\"{k}\"")))
    .collect();
    assert_eq!(
        order,
        [
            "target-membership",
            "membership-exceptions",
            "target",
            "exclusions"
        ],
        "{folder}"
    );
    let positions: Vec<usize> = order
        .iter()
        .map(|k| folder.find(&format!("\"{k}\"")).expect("present"))
        .collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{folder}");
}
