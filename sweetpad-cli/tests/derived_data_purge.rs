//! A project-scoped purge deletes this project's own DerivedData folder and no
//! other. Every checkout, worktree and copy of a project writes a folder with
//! the same `<Name>-` prefix, so these run against a fake home whose store
//! holds the folders of two copies of one project. The fake home also carries
//! the Xcode settings that move DerivedData, which the verbs follow the way
//! the build does.

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
        .env("CFFIXED_USER_HOME", home)
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

/// The account name per-user Xcode settings are filed under: the one
/// `xcodebuild` reads them for, from the user database.
fn user() -> String {
    sweetpad_lib::host::user().expect("the test runs as an account with a name")
}

/// Xcode's Settings → Locations → Derived Data, set to `location` in the fake
/// home's preferences.
fn set_app_location(home: &Path, location: &Path) {
    let prefs = home.join("Library/Preferences");
    std::fs::create_dir_all(&prefs).unwrap();
    std::fs::write(
        prefs.join("com.apple.dt.Xcode.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\
             \t<key>IDECustomDerivedDataLocation</key>\n\t<string>{}</string>\n</dict>\n</plist>\n",
            location.display()
        ),
    )
    .unwrap();
}

/// A project's own Derived Data setting, as Xcode writes it into the
/// project's per-user settings for [`user`]: `style` is `AbsolutePath` or
/// `WorkspaceRelativePath`.
fn set_project_location(project: &Path, style: &str, location: &str) {
    let dir = project.join(format!(
        "project.xcworkspace/xcuserdata/{}.xcuserdatad",
        user()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("WorkspaceSettings.xcsettings"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\
             \t<key>DerivedDataCustomLocation</key>\n\t<string>{location}</string>\n\
             \t<key>DerivedDataLocationStyle</key>\n\t<string>{style}</string>\n</dict>\n</plist>\n"
        ),
    )
    .unwrap();
}

/// A stub `name` in `<cwd>/bin` that exits 0 after appending its arguments to
/// `bin/<name>.args`, for putting ahead of the real tool on `PATH`. Returns
/// the `bin` directory.
fn stub(cwd: &Path, name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let bin = cwd.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let path = bin.join(name);
    std::fs::write(
        &path,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$0.args\"\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// A copy of the committed fixture project under `dir`, for a test that
/// writes per-user settings into it.
fn fixture_copy(dir: &Path) -> PathBuf {
    let source = Path::new(env!("SWEETPAD_LIB_DIR"))
        .join("fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj");
    let copy = dir.join("SweetpadCIApp.xcodeproj");
    let status = Command::new("cp")
        .arg("-R")
        .arg(&source)
        .arg(&copy)
        .status()
        .unwrap();
    assert!(status.success());
    copy
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

/// Xcode's app-wide Derived Data location moves the whole store, and the verbs
/// look for the project's folder there, where its builds put it. A folder the
/// stock store still holds from before the move is no longer the build's.
#[test]
fn the_verbs_follow_xcodes_derived_data_location() {
    let home = tmp("app-home");
    let cwd = tmp("app-cwd");
    let custom = cwd.join("Fast/DerivedData");
    std::fs::create_dir_all(&custom).unwrap();
    set_app_location(&home, &custom);
    let project = cwd.join("app/MyApp.xcodeproj");
    std::fs::create_dir_all(&project).unwrap();
    let own = folder_for(&custom, &project);
    let stock = folder_for(&store(&home), &project);

    let path = data(&sweetpad(
        &[
            "derived-data",
            "path",
            "--project",
            &shown(&project),
            "--json",
        ],
        &cwd,
        &home,
        None,
    ));
    assert_eq!(path["root"], shown(&custom));
    assert_eq!(strings(&path["paths"]), [shown(&own)]);
    assert_eq!(strings(&path["others"]), Vec::<String>::new());

    let all = data(&sweetpad(
        &["derived-data", "path", "--all", "--json"],
        &cwd,
        &home,
        None,
    ));
    assert_eq!(all["root"], shown(&custom));
    assert_eq!(strings(&all["paths"]), [shown(&custom)]);

    let size = data(&sweetpad(
        &[
            "derived-data",
            "size",
            "--project",
            &shown(&project),
            "--json",
        ],
        &cwd,
        &home,
        None,
    ));
    assert_eq!(size["folders"], 1);
    assert_eq!(size["bytes"], 1);

    let purge = data(&sweetpad(
        &[
            "derived-data",
            "purge",
            "--project",
            &shown(&project),
            "--yes",
            "--json",
        ],
        &cwd,
        &home,
        None,
    ));
    assert_eq!(strings(&purge["removed"]), [shown(&own)]);
    assert!(!own.exists());
    assert!(stock.join("Build/built.txt").exists());
}

/// A project's own Derived Data setting (an absolute location) outranks the
/// app-wide one: `open dd` opens the folder there and `clean --purge` deletes
/// it, leaving the stock store alone.
#[test]
fn open_and_clean_follow_the_projects_own_location() {
    let home = tmp("proj-home");
    let cwd = tmp("proj-cwd");
    let project = fixture_copy(&cwd);
    let custom = cwd.join("ProjectDD");
    std::fs::create_dir_all(&custom).unwrap();
    set_project_location(&project, "AbsolutePath", &shown(&custom));
    let own = folder_for(&custom, &project);
    let stock = folder_for(&store(&home), &project);
    let bin = stub(&cwd, "open");
    stub(&cwd, "xcodebuild");

    let open = data(&sweetpad(
        &["open", "dd", "--project", &shown(&project), "--json"],
        &cwd,
        &home,
        Some(&bin),
    ));
    assert_eq!(open["opened"], shown(&own));
    assert_eq!(
        std::fs::read_to_string(bin.join("open.args")).unwrap(),
        format!("{}\n", shown(&own))
    );

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
    assert!(stock.join("Build/built.txt").exists());
}

/// A workspace-relative location writes the bare project name beside the
/// project, with no hash in it, and that folder is the project's.
#[test]
fn a_workspace_relative_location_is_the_bare_folder_beside_the_project() {
    let home = tmp("rel-home");
    let cwd = tmp("rel-cwd");
    let project = cwd.join("app/MyApp.xcodeproj");
    std::fs::create_dir_all(&project).unwrap();
    set_project_location(&project, "WorkspaceRelativePath", "DerivedData");
    let own = cwd.join("app/DerivedData/MyApp");
    std::fs::create_dir_all(&own).unwrap();

    let path = data(&sweetpad(
        &[
            "derived-data",
            "path",
            "--project",
            &shown(&project),
            "--json",
        ],
        &cwd,
        &home,
        None,
    ));
    assert_eq!(path["root"], shown(&cwd.join("app/DerivedData")));
    assert_eq!(strings(&path["paths"]), [shown(&own)]);
}

/// Off a terminal a purge can't ask, so without '--yes' it refuses as a usage
/// error and deletes nothing. With nothing to delete there is nothing to ask,
/// and no '--yes' is needed.
#[test]
fn a_purge_without_yes_off_a_terminal_is_a_usage_error() {
    let home = tmp("noyes-home");
    let cwd = tmp("noyes-cwd");
    let store = store(&home);
    let project = cwd.join("MyApp.xcodeproj");
    std::fs::create_dir_all(&project).unwrap();

    let args = ["derived-data", "purge", "--project", &shown(&project)];
    let empty = sweetpad(&args, &cwd, &home, None);
    assert!(empty.status.success(), "{empty:?}");

    let folder = folder_for(&store, &project);
    let human = sweetpad(&args, &cwd, &home, None);
    assert_eq!(human.status.code(), Some(2), "{human:?}");
    assert!(
        String::from_utf8_lossy(&human.stderr).contains("pass --yes"),
        "{human:?}"
    );
    let json = sweetpad(&[&args[..], &["--json"]].concat(), &cwd, &home, None);
    assert_eq!(json.status.code(), Some(2), "{json:?}");
    let envelope: Value = serde_json::from_slice(&json.stderr).unwrap();
    assert_eq!(envelope["error"]["code"], "usage_error", "{envelope}");
    assert!(
        folder.join("Build/built.txt").exists(),
        "nothing was deleted"
    );
}
