//! A project-scoped purge deletes this project's own DerivedData folder and no
//! other. Every checkout, worktree and copy of a project writes a folder with
//! the same `<Name>-` prefix, so these run against a fake home whose store
//! holds the folders of two copies of one project.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::TempDir;
use serde_json::Value;

fn tmp(tag: &str) -> TempDir {
    let dir = TempDir::new(&format!("sweetpad-purge-{tag}"));
    // Stop walk-up discovery at this directory.
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

/// `<home>/Library/Developer/Xcode/DerivedData`, created.
fn store(home: &Path) -> PathBuf {
    let store = home.join("Library/Developer/Xcode/DerivedData");
    std::fs::create_dir_all(&store).unwrap();
    store
}

/// The folder Xcode writes for the container at `container`, created under
/// `store` with a file in it, so a purge has something to delete.
fn folder_for(store: &Path, container: &Path) -> PathBuf {
    let name = container.file_stem().unwrap().to_string_lossy();
    let hash = sweetpad_lib::derived_data::container_hash(container);
    let folder = store.join(sweetpad_lib::derived_data::hashed_folder(&name, &hash));
    std::fs::create_dir_all(folder.join("Build")).unwrap();
    std::fs::write(folder.join("Build/built.txt"), "x").unwrap();
    folder
}

fn sweetpad(args: &[&str], cwd: &Path, home: &Path, path: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sweetpad"));
    cmd.args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_CACHE_HOME", home)
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .env_remove("CLICOLOR_FORCE");
    if let Some(bin) = path {
        cmd.env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
    }
    cmd.output().expect("failed to run the sweetpad binary")
}

fn data(out: &Output) -> Value {
    assert!(out.status.success(), "{out:?}");
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap();
    envelope["data"].clone()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

/// Two copies of `MyApp`, and a prefix collision beside them. Purging one copy
/// takes its folder and names the other copy's as kept; the collision is no
/// copy of this project at all.
#[test]
fn purging_one_copy_keeps_the_other_copys_folder() {
    let home = tmp("home");
    let cwd = tmp("cwd");
    let store = store(&home);
    let first = cwd.join("first/MyApp.xcodeproj");
    let second = cwd.join("second/MyApp.xcodeproj");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let first_folder = folder_for(&store, &first);
    let second_folder = folder_for(&store, &second);
    let helper = store.join("MyAppHelper-abcdefghijklmnopqrstuvwxyzab");
    std::fs::create_dir_all(&helper).unwrap();

    let path = data(&sweetpad(
        &[
            "derived-data",
            "path",
            "--project",
            &shown(&first),
            "--json",
        ],
        &cwd,
        &home,
        None,
    ));
    assert_eq!(strings(&path["paths"]), [shown(&first_folder)]);
    assert_eq!(strings(&path["others"]), [shown(&second_folder)]);

    let purge = data(&sweetpad(
        &[
            "derived-data",
            "purge",
            "--project",
            &shown(&first),
            "--yes",
            "--json",
        ],
        &cwd,
        &home,
        None,
    ));
    assert_eq!(strings(&purge["removed"]), [shown(&first_folder)]);
    assert_eq!(strings(&purge["others"]), [shown(&second_folder)]);
    assert!(!first_folder.exists());
    assert!(second_folder.join("Build/built.txt").exists());
    assert!(helper.exists());

    // The second copy still finds its own folder, and says nothing about the
    // first one's, which is gone.
    let path = data(&sweetpad(
        &[
            "derived-data",
            "path",
            "--project",
            &shown(&second),
            "--json",
        ],
        &cwd,
        &home,
        None,
    ));
    assert_eq!(strings(&path["paths"]), [shown(&second_folder)]);
    assert_eq!(strings(&path["others"]), Vec::<String>::new());
}

/// Human output names what the purge kept, after what it removed.
#[test]
fn a_human_purge_says_how_many_folders_it_kept() {
    let home = tmp("human-home");
    let cwd = tmp("human-cwd");
    let store = store(&home);
    let first = cwd.join("first/MyApp.xcodeproj");
    std::fs::create_dir_all(&first).unwrap();
    folder_for(&store, &first);
    for copy in ["second", "third"] {
        folder_for(&store, &cwd.join(copy).join("MyApp.xcodeproj"));
    }

    let out = sweetpad(
        &[
            "derived-data",
            "purge",
            "--project",
            &shown(&first),
            "--yes",
        ],
        &cwd,
        &home,
        None,
    );
    assert!(out.status.success(), "{out:?}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(
        stderr,
        "purged 1 folder(s)\n\
         kept 2 other 'MyApp-*' folder(s) from other checkouts or same-named projects\n"
    );
    let out = sweetpad(
        &[
            "derived-data",
            "purge",
            "--project",
            &shown(&first),
            "--yes",
        ],
        &cwd,
        &home,
        None,
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.starts_with("nothing to purge\nkept 2 other"),
        "{stderr}"
    );
}

/// `clean --purge` takes the same scope: the project's own folder goes, and
/// the folder another copy of it wrote stays.
#[test]
fn clean_purge_keeps_the_other_copys_folder() {
    use std::os::unix::fs::PermissionsExt;

    let home = tmp("clean-home");
    let cwd = tmp("clean-cwd");
    let bin = cwd.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let stub = bin.join("xcodebuild");
    std::fs::write(&stub, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();

    let project = Path::new(env!("SWEETPAD_LIB_DIR"))
        .join("fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj");
    let store = store(&home);
    let own = folder_for(&store, &project);
    let other = folder_for(&store, &cwd.join("elsewhere/SweetpadCIApp.xcodeproj"));

    let clean = data(&sweetpad(
        &[
            "clean",
            "--project",
            &shown(&project),
            "--scheme",
            "SweetpadCIMac",
            "--configuration",
            "Debug",
            "--purge",
            "--json",
        ],
        &cwd,
        &home,
        Some(&bin),
    ));
    assert_eq!(strings(&clean["purged"]), [shown(&own)]);
    assert!(!own.exists());
    assert!(other.join("Build/built.txt").exists());
}
