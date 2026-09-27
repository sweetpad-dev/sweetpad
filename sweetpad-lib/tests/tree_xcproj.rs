//! The navigator tree of a `project.xcproj` document: nodes addressed by their
//! path through it, since the format writes an id only on the products a
//! target points at and on a node whose path another node shares.

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

    assert!(
        groups
            .iter()
            .all(|g| g.navigator_path.as_deref() == Some(g.address.as_str())),
        "a node's address is its navigator path"
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
            navigator_path: Some("Sources/Views".into()),
        }
    );
    // A group that titles rather than paths resolves to its parent's directory.
    assert_eq!(
        tree::add_group(&mut doc, "Frameworks", None, None, "<group>").unwrap(),
        AddGroupOutcome::Created {
            address: "Frameworks".into(),
            resolved: String::new(),
            navigator_path: Some("Frameworks".into()),
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

/// A group whose directory is named apart from it shows as its name, and
/// that is the address it is created at and found at again.
#[test]
fn a_group_is_addressed_by_its_name_when_its_directory_differs() {
    let mut doc = document();
    let created = AddGroupOutcome::Created {
        address: "Sources/Views".into(),
        resolved: "Sources/UI".into(),
        navigator_path: Some("Sources/Views".into()),
    };
    assert_eq!(
        tree::add_group(&mut doc, "Views", Some("Sources"), Some("UI"), "<group>").unwrap(),
        created
    );
    assert_eq!(
        tree::add_group(&mut doc, "Views", Some("Sources"), Some("UI"), "<group>").unwrap(),
        AddGroupOutcome::AlreadyExists {
            address: "Sources/Views".into(),
            resolved: "Sources/UI".into(),
            navigator_path: Some("Sources/Views".into()),
        }
    );
    let groups = tree::list_groups(&doc).unwrap();
    assert!(groups.iter().any(|g| g.address == "Sources/Views"));
}

/// An add finds an existing node of its own kind only. A file, a group or a
/// synchronized folder of the same name would share the new node's navigator
/// path, which Xcode allows but no address could then tell apart, so the add
/// is refused and says which it met.
#[test]
fn an_add_beside_a_node_of_another_kind_with_its_name_is_refused() {
    let mut doc = document();
    let before = xcproj::serialize(&doc);

    let err = tree::add_group(&mut doc, "Deep.swift", None, None, "<group>").unwrap_err();
    assert_eq!(
        err,
        "'Deep.swift' is already the navigator path of a file. A group beside it would \
         share that path, and no argument could then tell the two apart; pick another name"
    );
    let err = tree::add_group(&mut doc, "Shared", None, None, "<group>").unwrap_err();
    assert!(
        err.contains("already the navigator path of a synchronized folder"),
        "{err}"
    );
    let err = tree::add_fileref(&mut doc, "Sources", None, "<group>", None).unwrap_err();
    assert!(
        err.contains("already the navigator path of a group"),
        "{err}"
    );
    assert!(err.contains("add it under another group"), "{err}");
    assert_eq!(xcproj::serialize(&doc), before, "nothing was written");

    // The same names are free one level down.
    assert!(matches!(
        tree::add_group(&mut doc, "Deep.swift", Some("Sources"), None, "<group>").unwrap(),
        AddGroupOutcome::Created { .. }
    ));
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

/// Nodes the rest of the document names by navigator path: the xcconfig a
/// configuration is based on, one anchored in a synchronized folder, the
/// products, and the products group Xcode reads as `Products` when the
/// document writes none. Xcode 27.0 and 27.2 refuse to open a document where
/// one of these names nothing ("Invalid reference").
const REFERENCED: &str = r#"{
  "configurations": [
    { "name": "Debug", "file": "Config/Base.xcconfig" },
    "Release",
  ],
  "files": [
    {
      "kind": "group",
      "path": "Config",
      "children": [
        { "path": "Base.xcconfig" },
        { "path": "Release.xcconfig", "id": "0000000000000000000000C2" },
      ],
    }, {
      "kind": "group",
      "path": "Sources",
      "children": [
        { "path": "App.swift" },
      ],
    },
    { "kind": "folder", "path": "Shared", "target-membership": [ "App" ] },
    {
      "kind": "group",
      "name": "Products",
      "children": [
        { "path": "<PRODUCTS>/App.app", "id": "958E16DD736EB85C16C05DE6", "index": false },
      ],
    },
  ],
  "targets": [
    {
      "name": "App",
      "product": "Products/App.app",
      "product-type": "application",
      "specialized-configurations": [
        { "name": "Debug", "file": { "anchor": "Shared", "relative-path": "App.xcconfig" } },
        { "name": "Release", "file": "id:0000000000000000000000C2" },
      ],
    },
  ],
}
"#;

/// A move takes every reference into the moved subtree along, so the
/// document still opens; one written as `id:` needs nothing. Moving back
/// restores the references, and a products group back at `Products` is left
/// unwritten again.
#[test]
fn a_move_takes_the_references_into_it_along() {
    let mut doc = xcproj::parse(REFERENCED).unwrap_or_else(|e| panic!("parse: {e}"));
    tree::move_node(&mut doc, "Config", Some("Sources")).unwrap();
    tree::move_node(&mut doc, "Products", Some("Sources")).unwrap();
    tree::move_node(&mut doc, "Shared", Some("Sources")).unwrap();
    let text = xcproj::serialize(&doc);
    for moved in [
        r#"{ "name": "Debug", "file": "Sources/Config/Base.xcconfig" },"#,
        r#""product": "Sources/Products/App.app","#,
        r#""anchor": "Sources/Shared""#,
        r#"{ "name": "Release", "file": "id:0000000000000000000000C2" },"#,
        r#""products-group": "Sources/Products","#,
    ] {
        assert!(text.contains(moved), "{moved} in {text}");
    }

    tree::move_node(&mut doc, "Sources/Products", None).unwrap();
    let text = xcproj::serialize(&doc);
    assert!(text.contains(r#""product": "Products/App.app","#), "{text}");
    assert!(!text.contains("products-group"), "{text}");
}

/// A delete that would leave a reference naming nothing is refused, and says
/// which reference, instead of writing a document Xcode cannot open.
#[test]
fn a_node_a_reference_names_is_not_deleted() {
    let mut doc = xcproj::parse(REFERENCED).unwrap_or_else(|e| panic!("parse: {e}"));
    let err = tree::remove_fileref(&mut doc, "id:0000000000000000000000C2", false).unwrap_err();
    assert_eq!(
        err,
        "Config/Release.xcconfig is still the xcconfig that the 'Release' configuration of \
         target 'App' is based on; deleting it would leave that reference naming nothing"
    );
    let err = tree::remove_fileref(&mut doc, "Products/App.app", true).unwrap_err();
    assert!(
        err.contains("Products/App.app is still the product of target 'App';"),
        "{err}"
    );

    // The group Xcode reads as the products group with no key naming it.
    let mut doc = xcproj::parse(
        r#"{ "files": [ { "kind": "group", "name": "Products", "children": [ ] } ] }"#,
    )
    .unwrap_or_else(|e| panic!("parse: {e}"));
    let err = tree::remove_group(&mut doc, "Products", false).unwrap_err();
    assert!(
        err.contains("Products is still the project's products group"),
        "{err}"
    );
}

/// A synchronized folder added beside a node with its name shares that
/// node's navigator path. Xcode opens such a document until a reference runs
/// through the path, and then refuses it, so only that add is refused.
#[test]
fn a_folder_sharing_a_referenced_navigator_path_is_refused() {
    let mut doc = xcproj::parse(REFERENCED).unwrap_or_else(|e| panic!("parse: {e}"));
    let before = xcproj::serialize(&doc);
    let err = sweetpad_lib::sync_xcproj::add_root(&mut doc, "App", "Config").unwrap_err();
    assert!(
        err.contains(
            "'Config' is already the navigator path of a group, and the document names the xcconfig"
        ),
        "{err}"
    );
    assert_eq!(xcproj::serialize(&doc), before, "the refusal wrote nothing");
    sweetpad_lib::sync_xcproj::add_root(&mut doc, "App", "Sources").unwrap();
}

/// A move onto a navigator path another node already has would leave two
/// nodes no argument tells apart, and a reference naming one of them would
/// then name both. It is refused, as an add there is.
#[test]
fn a_move_onto_a_navigator_path_in_use_is_refused() {
    let mut doc = document();
    tree::add_fileref(&mut doc, "App.swift", None, "<group>", Some("Sources/App")).unwrap();
    let before = xcproj::serialize(&doc);
    let err = tree::move_node(&mut doc, "Sources/App/App.swift", Some("Sources")).unwrap_err();
    assert!(
        err.contains("'Sources/App.swift' is already the navigator path of a file"),
        "{err}"
    );
    assert_eq!(xcproj::serialize(&doc), before, "the refusal wrote nothing");
}

/// `""` and `/` name the navigator root, as they name the mainGroup in a
/// `project.pbxproj`, so a root spelling carries across the two formats.
#[test]
fn the_navigator_root_answers_to_an_empty_path_and_a_slash() {
    let mut doc = document();
    let moved = tree::move_node(&mut doc, "Sources/App/ContentView.swift", Some("/")).unwrap();
    let MoveOutcome::Moved { address, to, .. } = moved else {
        panic!("expected a move");
    };
    assert_eq!((address.as_str(), to.as_str()), ("ContentView.swift", ""));

    let AddGroupOutcome::Created { address, .. } =
        tree::add_group(&mut doc, "Library", Some(""), None, "<group>").unwrap()
    else {
        panic!("expected a new group");
    };
    assert_eq!(address, "Library");
    let AddRefOutcome::Created { address, .. } =
        tree::add_fileref(&mut doc, "Extra.swift", None, "<group>", Some("/")).unwrap()
    else {
        panic!("expected a new node");
    };
    assert_eq!(address, "Extra.swift");
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
    let err = tree::remove_fileref(&mut doc, "0123456789ABCDEF01234567", false).unwrap_err();
    assert!(err.contains("looks like a pbxproj object id"), "{err}");
    assert!(err.contains("navigator path"), "{err}");

    // The product carries its id, and it names the node with 'id:' in front.
    let err = tree::remove_fileref(&mut doc, "958E16DD736EB85C16C05DE6", false).unwrap_err();
    assert!(
        err.contains("name it as 'id:958E16DD736EB85C16C05DE6'"),
        "{err}"
    );

    // An ordinary miss points at the node it probably meant.
    let err = tree::remove_fileref(&mut doc, "App.swift", false).unwrap_err();
    assert!(err.contains("did you mean Sources/App.swift"), "{err}");
}

/// Two groups at the root with no name give their children one navigator
/// path. Xcode 27.2 then writes an id on the node a reference has to name,
/// and `id:` with that id names it here too.
#[test]
fn a_node_sharing_its_navigator_path_is_named_by_its_id() {
    let mut doc = xcproj::parse(
        r#"{
  "files": [
    {
      "kind": "group",
      "children": [
        {
          "kind": "group",
          "path": "Config",
          "children": [
            { "path": "Base.xcconfig", "id": "0000000000000000000000F2" },
          ],
        },
      ],
    }, {
      "kind": "group",
      "children": [
        {
          "kind": "group",
          "name": "Config",
          "path": "Other",
          "children": [
            { "path": "Base.xcconfig" },
          ],
        },
      ],
    },
  ],
}
"#,
    )
    .unwrap_or_else(|e| panic!("parse: {e}"));
    assert_eq!(
        addresses(&doc),
        ["/Config/Base.xcconfig", "/Config/Base.xcconfig"]
    );

    let err = tree::remove_fileref(&mut doc, "/Config/Base.xcconfig", false).unwrap_err();
    assert!(
        err.contains("'/Config/Base.xcconfig' is the navigator path of 2 nodes"),
        "{err}"
    );
    assert!(
        err.contains("Name one by its id: id:0000000000000000000000F2"),
        "{err}"
    );
    let err = tree::remove_fileref(&mut doc, "0000000000000000000000F2", false).unwrap_err();
    assert!(
        err.contains("name it as 'id:0000000000000000000000F2'"),
        "{err}"
    );
    let err = tree::remove_fileref(&mut doc, "id:0000000000000000000000F3", false).unwrap_err();
    assert!(
        err.contains("no node in the project's navigator tree has the id"),
        "{err}"
    );

    let removed = tree::remove_fileref(&mut doc, "id:0000000000000000000000F2", false).unwrap();
    assert_eq!(removed.address, "/Config/Base.xcconfig");
    let left = tree::list_filerefs(&doc).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(
        left[0].resolved, "Other/Base.xcconfig",
        "the other one stays"
    );
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

/// Groups with neither a name nor a path, laid out the way Xcode 27.2 writes
/// them when it converts such a project: one at the root holding `Products`,
/// and one inside `Sources` holding `App` and `Config`. The document names the
/// product `/Products/App.app` and the xcconfig `Sources//Config/Base.xcconfig`.
const NAMELESS: &str = r#"{
  "configurations": [
    { "name": "Debug", "file": "Sources//Config/Base.xcconfig" },
  ],
  "files": [
    {
      "kind": "group",
      "path": "Sources",
      "children": [
        {
          "kind": "group",
          "children": [
            {
              "kind": "group",
              "path": "App",
              "children": [
                { "path": "ContentView.swift" },
              ],
            }, {
              "kind": "group",
              "path": "Config",
              "children": [
                { "path": "Base.xcconfig" },
              ],
            },
          ],
        },
      ],
    }, {
      "kind": "group",
      "children": [
        {
          "kind": "group",
          "name": "Products",
          "children": [
            { "path": "<PRODUCTS>/App.app", "id": "958E16DD736EB85C16C05DE6", "index": false },
          ],
        },
      ],
    },
  ],
  "targets": [
    { "name": "App", "product": "/Products/App.app", "product-type": "application" },
  ],
  "products-group": "/Products",
}
"#;

/// Such a group is still a node holding its children, so its subtree is
/// listed, with the empty component Xcode spells, and each address it prints
/// names its node back.
#[test]
fn a_group_with_no_name_and_no_path_is_walked_through() {
    let mut doc = xcproj::parse(NAMELESS).unwrap_or_else(|e| panic!("parse: {e}"));
    assert_eq!(
        addresses(&doc),
        [
            "Sources//App/ContentView.swift",
            "Sources//Config/Base.xcconfig",
            "/Products/App.app",
        ]
    );
    let groups = tree::list_groups(&doc).unwrap();
    assert_eq!(
        groups
            .iter()
            .map(|g| g.address.as_str())
            .collect::<Vec<_>>(),
        [
            "Sources",
            "Sources/",
            "Sources//App",
            "Sources//Config",
            "",
            "/Products"
        ]
    );
    assert_eq!(groups[0].children, ["Sources/"], "the group it holds");
    assert_eq!(groups[1].resolved, "Sources", "it adds no directory");
    assert_eq!(groups[2].resolved, "Sources/App");
    assert!(groups.iter().all(|g| !g.is_navigator_root));

    // An address matches as typed before its slashes are trimmed, and a
    // trimmed one still finds a node nothing else is called.
    let outcome =
        tree::add_fileref(&mut doc, "New.swift", None, "<group>", Some("Sources//App")).unwrap();
    assert_eq!(
        outcome,
        AddRefOutcome::Created {
            address: "Sources//App/New.swift".into(),
            resolved: "Sources/App/New.swift".into(),
            attached_to: Some("Sources//App".into()),
        }
    );
    let AddGroupOutcome::Created { address, .. } =
        tree::add_group(&mut doc, "Extra", Some("Products"), None, "<group>").unwrap()
    else {
        panic!("expected a new group");
    };
    assert_eq!(address, "/Products/Extra");

    // `""` is also the address of the group with no name at the root, so it
    // names two groups there, and `/` still names the navigator root.
    let err = tree::add_group(&mut doc, "Top", Some(""), None, "<group>").unwrap_err();
    assert!(
        err.contains("'' names both the navigator root and a group with no name at the root"),
        "{err}"
    );
    assert!(err.contains("pass '/' for the navigator root"), "{err}");
    let AddGroupOutcome::Created { address, .. } =
        tree::add_group(&mut doc, "Top", Some("/"), None, "<group>").unwrap()
    else {
        panic!("expected a new group");
    };
    assert_eq!(address, "Top");

    // Deleting a group counts the group with no name it holds.
    let err = tree::remove_group(&mut doc, "Sources", false).unwrap_err();
    assert!(err.contains("still holds 1 child node(s)"), "{err}");

    // A move would have to give it a path, and so a name.
    let err = tree::move_node(&mut doc, "Sources/", Some("Top")).unwrap_err();
    assert!(err.contains("has neither a name nor a path"), "{err}");

    let moved = tree::move_node(&mut doc, "Sources//App/New.swift", Some("/Products")).unwrap();
    assert_eq!(
        moved,
        MoveOutcome::Moved {
            address: "/Products/New.swift".into(),
            from: Some("Sources//App".into()),
            to: "/Products".into(),
            resolved: "Sources/App/New.swift".into(),
        }
    );

    // A group whose directory is already its own takes it with no path, so
    // it keeps no name.
    tree::add_group(&mut doc, "Org", Some("Sources"), None, "<group>").unwrap();
    let moved = tree::move_node(&mut doc, "Sources/", Some("Sources/Org")).unwrap();
    assert_eq!(
        moved,
        MoveOutcome::Moved {
            address: "Sources/Org/".into(),
            from: Some("Sources".into()),
            to: "Sources/Org".into(),
            resolved: "Sources".into(),
        }
    );
    let groups = tree::list_groups(&doc).unwrap();
    let nameless = groups.iter().find(|g| g.address == "Sources/Org/").unwrap();
    assert_eq!(
        (nameless.name.as_deref(), nameless.path.as_deref()),
        (None, None)
    );
}

/// The same navigator in both formats lists each group in the same directory:
/// a group with no path sits in its parent's directory, spelled as the
/// parent's is, and a path that climbs out of the project keeps its `..`.
#[test]
fn both_formats_place_a_group_in_the_same_directory() {
    let pbxproj = sweetpad_lib::pbxproj::parse(
        "// !$*UTF8*$!
{
\tobjects = {
\t\tMAIN = { isa = PBXGroup; sourceTree = \"<group>\"; children = (SRC, UP); };
\t\tSRC = { isa = PBXGroup; path = Sources; sourceTree = \"<group>\"; children = (INNER, NONE); };
\t\tINNER = { isa = PBXGroup; name = Inner; sourceTree = \"<group>\"; children = (); };
\t\tNONE = { isa = PBXGroup; sourceTree = \"<group>\"; children = (DEEP); };
\t\tDEEP = { isa = PBXGroup; path = Deep; sourceTree = \"<group>\"; children = (); };
\t\tUP = { isa = PBXGroup; path = ../Shared; sourceTree = \"<group>\"; children = (); };
\t\tPROJ = { isa = PBXProject; mainGroup = MAIN; targets = (); };
\t};
\trootObject = PROJ;
}
",
    )
    .unwrap_or_else(|e| panic!("parse: {e}"));
    let xcproj = xcproj::parse(
        r#"{
  "files": [
    {
      "kind": "group",
      "path": "Sources",
      "children": [
        { "kind": "group", "name": "Inner", "children": [] },
        {
          "kind": "group",
          "children": [
            { "kind": "group", "path": "Deep", "children": [] },
          ],
        },
      ],
    },
    { "kind": "group", "path": "../Shared", "children": [] },
  ],
}
"#,
    )
    .unwrap_or_else(|e| panic!("parse: {e}"));

    let by_path = |rows: Vec<sweetpad_lib::tree::GroupRow>| {
        rows.into_iter()
            .filter_map(|g| Some((g.navigator_path.filter(|p| !p.is_empty())?, g.resolved)))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let classic = by_path(sweetpad_lib::tree_pbxproj::list_groups(&pbxproj).unwrap());
    let document = by_path(tree::list_groups(&xcproj).unwrap());
    assert_eq!(classic, document);
    assert_eq!(classic["Sources/Inner"], "Sources");
    assert_eq!(classic["Sources/"], "Sources");
    assert_eq!(classic["Sources//Deep"], "Sources/Deep");
    assert_eq!(classic["Shared"], "../Shared");
}
