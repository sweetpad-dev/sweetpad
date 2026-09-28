//! `pbxproj group attach`/`detach` name the node they touched the way the
//! rest of the fileref and group verbs do, so one command's output feeds the
//! next without a rename in between.

mod common;

use std::path::Path;
use std::process::Command;

use common::TempDir;
use serde_json::Value;

/// `ContentView.swift`'s reference, and the group listing it, in the
/// committed classic fixture.
const CONTENT_VIEW: &str = "85BB78F9ECC9184F5BA8114B";
const SOURCES_APP: &str = "71376D09ABE451C1E73CAAE7";
const MAIN_GROUP: &str = "2EB79799EB980C9382F9E6B0";

fn run(args: &[&str], home: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sweetpad"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("CFFIXED_USER_HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home)
        .output()
        .expect("failed to run the sweetpad binary")
}

fn sweetpad(args: &[&str], home: &Path) -> Value {
    let out = run(args, home);
    assert!(out.status.success(), "{args:?}: {out:?}");
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
    envelope["data"].clone()
}

/// Human output, from a command that has to succeed.
fn human(args: &[&str], home: &Path) -> String {
    let out = run(args, home);
    assert!(out.status.success(), "{args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

/// A scratch copy of the committed project.xcproj fixture, and its project
/// path.
fn xcproj_fixture(name: &str) -> (TempDir, String) {
    let dir = TempDir::new(name);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let project = dir.join("SweetpadCIApp.xcodeproj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::copy(
        Path::new(env!("SWEETPAD_LIB_DIR"))
            .join("fixtures/_xcproj/SweetpadCIApp.xcodeproj/project.xcproj"),
        project.join("project.xcproj"),
    )
    .unwrap();
    let project = project.to_str().unwrap().to_string();
    (dir, project)
}

/// A scratch copy of the committed classic fixture, and its project path.
fn fixture(name: &str) -> (TempDir, String) {
    let dir = TempDir::new(name);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let project = dir.join("SweetpadCIApp.xcodeproj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::copy(
        Path::new(env!("SWEETPAD_LIB_DIR")).join(
            "fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj/project.pbxproj",
        ),
        project.join("project.pbxproj"),
    )
    .unwrap();
    let project = project.to_str().unwrap().to_string();
    (dir, project)
}

/// Both verbs report the child as `address` and the group by its address,
/// even when '--group' named it by directory.
#[test]
fn attach_and_detach_report_the_node_by_address() {
    let (dir, project) = fixture("sweetpad-group-link");
    let project = project.as_str();

    let refs = sweetpad(
        &["pbxproj", "fileref", "list", "--project", project, "--json"],
        &dir,
    );
    assert!(
        refs["refs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["address"] == CONTENT_VIEW && r["group"] == SOURCES_APP),
        "{refs}"
    );

    for (verb, changed) in [("detach", true), ("detach", false), ("attach", true)] {
        let data = sweetpad(
            &[
                "pbxproj",
                "group",
                verb,
                CONTENT_VIEW,
                "--group",
                "Sources/App",
                "--project",
                project,
                "--json",
            ],
            &dir,
        );
        assert_eq!(
            data,
            serde_json::json!({
                "action": verb,
                "address": CONTENT_VIEW,
                "group": SOURCES_APP,
                "changed": changed,
            }),
            "{verb}"
        );
    }
}

/// 'group list' prints the navigator path beside the id and the directory, so
/// every spelling a group argument takes is on the row, and the miss hint that
/// sends people there is true.
#[test]
fn group_list_prints_each_groups_navigator_path() {
    let (dir, project) = fixture("sweetpad-group-list");
    let project = project.as_str();

    let data = sweetpad(
        &["pbxproj", "group", "list", "--project", project, "--json"],
        &dir,
    );
    let groups = data["groups"].as_array().unwrap();
    let navigator_path = |address: &str| {
        groups
            .iter()
            .find(|g| g["address"] == address)
            .map(|g| g["navigatorPath"].clone())
    };
    assert_eq!(
        navigator_path(SOURCES_APP),
        Some(Value::from("Sources/App")),
        "{data}"
    );
    // An organizational group resolves to the project directory, so only its
    // navigator path tells it apart from the root.
    let recovered = groups
        .iter()
        .find(|g| g["name"] == "Recovered References")
        .unwrap();
    assert_eq!(recovered["navigatorPath"], "Recovered References");
    assert_eq!(recovered["resolved"], "");

    let out = run(&["pbxproj", "group", "list", "--project", project], &dir);
    assert!(out.status.success(), "{out:?}");
    let human = String::from_utf8(out.stdout).unwrap();
    assert!(
        human.contains(&format!(
            "{SOURCES_APP}  Sources/App  [Sources/App, 2 child(ren)]"
        )),
        "{human}"
    );
    // The root shows '/', the spelling that selects it, while JSON keeps its
    // empty navigator path.
    assert!(
        human.contains(&format!(
            "{MAIN_GROUP}  / (navigator root)  [(project root), 5 child(ren)]"
        )),
        "{human}"
    );
    assert_eq!(navigator_path(MAIN_GROUP), Some(Value::from("")), "{data}");

    let out = run(
        &[
            "pbxproj",
            "group",
            "attach",
            CONTENT_VIEW,
            "--group",
            "Sources/Nope",
            "--project",
            project,
        ],
        &dir,
    );
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains(
            "no group with id, navigator path, or directory Sources/Nope; \
             'pbxproj group list' shows all three"
        ),
        "{stderr}"
    );
}

/// 'group move' says where the node still resolves the way 'group list' says
/// it, so an organizational group, which resolves to the project directory,
/// reads '(project root)' rather than nothing.
#[test]
fn group_move_names_the_directory_the_node_keeps() {
    const RECOVERED: &str = "BCD3F34330601BEC003C7AE7";
    const SOURCES: &str = "670938FBEBAD9090CCFDAD2B";
    let (dir, project) = fixture("sweetpad-group-move");
    let project = project.as_str();

    let line = human(
        &[
            "pbxproj",
            "group",
            "move",
            RECOVERED,
            "--to",
            "Sources",
            "--project",
            project,
        ],
        &dir,
    );
    assert_eq!(
        line.trim_end(),
        format!("{RECOVERED} now under {SOURCES}, still at (project root)")
    );
}

/// A project.xcproj has no node for its navigator root, and a move there says
/// so in words rather than as a directory.
#[test]
fn group_move_to_the_xcproj_root_names_the_navigator_root() {
    let (dir, project) = xcproj_fixture("sweetpad-group-move-xcproj");
    let project = project.as_str();

    let move_to_root = |node: &str| {
        human(
            &[
                "pbxproj",
                "group",
                "move",
                node,
                "--to",
                "/",
                "--project",
                project,
            ],
            &dir,
        )
    };
    assert_eq!(
        move_to_root("Sources/App/ContentView.swift").trim_end(),
        "ContentView.swift now under the navigator root, still at Sources/App/ContentView.swift"
    );
    assert_eq!(
        move_to_root("ContentView.swift").trim_end(),
        "ContentView.swift is already under the navigator root"
    );
}

/// A project.xcproj group with neither a name nor a path is listed with what
/// it holds, and at the root its empty address reads '(unnamed)' rather than
/// passing for the navigator root, which has no row in that format.
#[test]
fn group_list_walks_through_an_xcproj_group_with_no_name() {
    let (dir, project) = xcproj_fixture("sweetpad-group-nameless-xcproj");
    std::fs::write(
        Path::new(&project).join("project.xcproj"),
        r#"{
  "files": [
    {
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
}
"#,
    )
    .unwrap();
    let project = project.as_str();

    let groups = human(&["pbxproj", "group", "list", "--project", project], &dir);
    assert_eq!(
        groups.lines().collect::<Vec<_>>(),
        [
            "(unnamed)  [(project root), 1 child(ren)]",
            "/Products  [(project root), 1 child(ren)]",
        ]
    );
    let refs = human(&["pbxproj", "fileref", "list", "--project", project], &dir);
    assert!(
        refs.starts_with("/Products/App.app  <PRODUCTS>/App.app  ["),
        "{refs}"
    );
}

/// 'group add' names the group it made the way its 'group list' row does: by
/// address and navigator path, then the directory. An organizational group
/// resolves to its parent's directory, so the directory alone says nothing.
#[test]
fn group_add_names_the_new_group() {
    let (dir, project) = fixture("sweetpad-group-add");
    let project = project.as_str();
    let add = ["pbxproj", "group", "add", "Foo", "--project", project];

    let data = sweetpad(&[&add[..], &["--json"]].concat(), &dir);
    let address = data["address"].as_str().unwrap().to_string();
    assert_eq!(data["navigatorPath"], "Foo", "{data}");
    assert_eq!(data["resolved"], "", "{data}");
    assert_eq!(
        human(&add, &dir).trim_end(),
        format!("{address}  Foo  [(project root)] (already a group)")
    );

    let line = human(
        &[
            "pbxproj",
            "group",
            "add",
            "Views",
            "--parent",
            "Sources/App",
            "--path",
            "Views",
            "--project",
            project,
        ],
        &dir,
    );
    assert!(
        line.trim_end()
            .ends_with("  Sources/App/Views  [Sources/App/Views]"),
        "{line}"
    );

    let (dir, project) = xcproj_fixture("sweetpad-group-add-xcproj");
    let line = human(
        &[
            "pbxproj",
            "group",
            "add",
            "Foo",
            "--project",
            project.as_str(),
        ],
        &dir,
    );
    assert_eq!(line.trim_end(), "Foo  [(project root)]");
}
