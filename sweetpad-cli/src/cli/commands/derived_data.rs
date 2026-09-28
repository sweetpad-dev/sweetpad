//! `sweetpad derived-data …` — inspect and purge Xcode's DerivedData.
//!
//! DerivedData (`~/Library/Developer/Xcode/DerivedData`, unless Xcode's
//! settings move it) accumulates module caches, indexes, and build products;
//! "delete DerivedData" is the iOS developer's most common reset. `path`/`size`
//! inspect it, `purge` clears it — whole, or scoped to the resolved project's
//! own `<Name>-<hash>` folder. A folder with the same name that another
//! checkout, worktree or copy of the project wrote (or another project with the
//! same name) is not this project's, so the scoped verbs leave it alone and say
//! how many they left.
//!
//! Both scopes find DerivedData where the build does
//! ([`sweetpad_lib::derived_data`]): `--all` is the store Xcode's Locations
//! setting names, and a project's folder follows its per-user workspace
//! settings too, which can move it out of that store.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use dialoguer::theme::{ColorfulTheme, SimpleTheme, Theme};

use crate::cli::output::Output;
use crate::cli::resolve::Container;
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered, resolve};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Print this project's DerivedData folder(s) (or the whole store: --all).
    Path {
        /// Operate on the whole DerivedData store, not just this project.
        #[arg(long)]
        all: bool,
    },
    /// Report the on-disk size of DerivedData (this project, or --all).
    Size {
        /// Operate on the whole DerivedData store, not just this project.
        #[arg(long)]
        all: bool,
    },
    /// Delete DerivedData — this project's folder(s) by default, or --all.
    Purge {
        /// Operate on the whole DerivedData store, not just this project.
        #[arg(long)]
        all: bool,
        /// Skip the interactive confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Path { all } => path(ctx, *all),
        Action::Size { all } => size(ctx, *all),
        Action::Purge { all, yes } => purge(ctx, *all, *yes),
    }
}

/// What a scoped command acts on: this container's own DerivedData folder(s),
/// and the folders that only share its name.
pub(crate) struct Scope {
    /// The store the folders sit in: the one this container's builds write
    /// into, or under `--all` the app-wide one.
    pub(crate) root: PathBuf,
    /// The folders this container's builds wrote (the store's root under
    /// `--all`).
    pub(crate) own: Vec<PathBuf>,
    /// Folders named like this project's that another container wrote: another
    /// checkout, worktree or copy of the project, or another project with the
    /// same name. Never acted on.
    pub(crate) others: Vec<PathBuf>,
    /// The `<Name>-*` pattern both sets match, for messages.
    pattern: String,
}

impl Scope {
    /// The note naming the same-named folders a scoped command passed over,
    /// led by what it did with them ("kept", "skipped"); `None` when there
    /// were none.
    pub(crate) fn others_note(&self, done: &str) -> Option<String> {
        (!self.others.is_empty()).then(|| {
            format!(
                "{done} {} other '{}' folder(s) from other checkouts or same-named projects",
                self.others.len(),
                self.pattern
            )
        })
    }
}

fn display_all(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| p.display().to_string()).collect()
}

/// The resolved DerivedData folder(s): one path per line in human mode (or a
/// "none found" note when empty), or `{ "root", "paths", "others" }` in the
/// JSON envelope.
struct PathResult {
    root: String,
    paths: Vec<String>,
    others: Vec<String>,
    others_note: Option<String>,
}

impl Render for PathResult {
    fn human(&self, out: &Output) {
        if self.paths.is_empty() {
            out.note("no matching DerivedData folders found");
        }
        for p in &self.paths {
            out.line(p);
        }
        if let Some(note) = &self.others_note {
            out.note(note);
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "root": self.root,
            "paths": self.paths,
            "others": self.others,
        })
    }
}

/// The on-disk size of the resolved DerivedData folder(s): a "<size> across N
/// folder(s)" line in human mode, or `{ "bytes", "human", "folders" }` in JSON.
struct SizeResult {
    bytes: u64,
    folders: usize,
    others_note: Option<String>,
}

impl Render for SizeResult {
    fn human(&self, out: &Output) {
        out.line(&format!(
            "{} across {} folder(s)",
            format_size(self.bytes),
            self.folders
        ));
        if let Some(note) = &self.others_note {
            out.note(note);
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "bytes": self.bytes,
            "human": format_size(self.bytes),
            "folders": self.folders,
        })
    }
}

/// The outcome of a purge: status notes in human mode, or `{ "removed",
/// "others" }` in JSON. `note` is what human mode prints — "purged N
/// folder(s)" on a real purge, or "nothing to purge" on the early path (which
/// carries an empty `removed`, so `--json` still reports an outcome) — and
/// `others_note` follows it when same-named folders were kept.
struct PurgeResult {
    removed: Vec<String>,
    others: Vec<String>,
    note: String,
    others_note: Option<String>,
}

impl Render for PurgeResult {
    fn human(&self, out: &Output) {
        out.note(&self.note);
        if let Some(note) = &self.others_note {
            out.note(note);
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "removed": self.removed,
            "others": self.others,
        })
    }
}

fn path(ctx: &mut Context, all: bool) -> CommandResult {
    let scope = scope(ctx, all)?;

    Ok(Rendered::data(PathResult {
        root: scope.root.display().to_string(),
        paths: display_all(&scope.own),
        others: display_all(&scope.others),
        others_note: scope.others_note("skipped"),
    }))
}

fn size(ctx: &mut Context, all: bool) -> CommandResult {
    let scope = scope(ctx, all)?;
    let bytes: u64 = scope.own.iter().map(|p| dir_size(p)).sum();

    Ok(Rendered::data(SizeResult {
        bytes,
        folders: scope.own.len(),
        others_note: scope.others_note("skipped"),
    }))
}

fn purge(ctx: &mut Context, all: bool, yes: bool) -> CommandResult {
    let scope = scope(ctx, all)?;
    let root = scope.root.clone();
    let others = display_all(&scope.others);
    let others_note = scope.others_note("kept");
    let targets = scope.own;

    if targets.is_empty() {
        return Ok(Rendered::data(PurgeResult {
            removed: Vec::new(),
            others,
            note: "nothing to purge".to_string(),
            others_note,
        }));
    }

    // Deleting needs consent: an interactive confirmation, or an explicit
    // `--yes`. Non-interactive contexts (`--json`, CI, piped) can't prompt, so
    // they *require* `--yes` — a scripted `purge --all --json` must never
    // silently rm -rf the DerivedData store.
    if !yes {
        if !ctx.out.is_interactive() {
            return Err(CliError::new(
                "refusing to delete DerivedData without confirmation; pass --yes to purge non-interactively",
            )
            .kind(ErrorKind::Usage));
        }
        let prompt = if all {
            format!("Delete ALL DerivedData under {}?", root.display())
        } else {
            format!(
                "Delete {} DerivedData folder(s) for this project?",
                targets.len()
            )
        };
        // The same color-aware theme as every other prompt, so `--no-color` holds.
        let colorful = ColorfulTheme::default();
        let simple = SimpleTheme;
        let theme: &dyn Theme = if ctx.out.use_color_stderr() {
            &colorful
        } else {
            &simple
        };
        let confirmed = dialoguer::Confirm::with_theme(theme)
            .with_prompt(prompt)
            .default(false)
            .interact()
            .map_err(|e| {
                CliError::new(format!("confirmation cancelled: {e}")).kind(ErrorKind::UserCancel)
            })?;
        if !confirmed {
            // A declined prompt exits 6 like an Esc'd one — scripts must be
            // able to tell "purged" from "declined" (`help exit-codes`
            // documents this).
            return Err(CliError::new("purge declined").kind(crate::cli::ErrorKind::UserCancel));
        }
    }

    let mut removed = Vec::new();
    for p in &targets {
        std::fs::remove_dir_all(p)
            .map_err(|e| CliError::new(format!("failed to remove {}: {e}", p.display())))?;
        removed.push(p.display().to_string());
    }

    let note = format!("purged {} folder(s)", removed.len());
    Ok(Rendered::data(PurgeResult {
        removed,
        others,
        note,
        others_note,
    }))
}

/// This project's DerivedData scope — shared with `sweetpad clean --purge`.
pub(crate) fn project_scope(ctx: &Context) -> Result<Scope, CliError> {
    scope(ctx, false)
}

/// The account's home, which every DerivedData location is found from, as
/// `xcodebuild` finds it ([`sweetpad_lib::host::home`]): a redirected `$HOME`
/// doesn't move the DerivedData a build writes.
fn home() -> Result<String, CliError> {
    sweetpad_lib::host::home()
        .map(|home| home.to_string_lossy().into_owned())
        .ok_or_else(|| {
            CliError::new("no home directory for this account; cannot locate DerivedData")
        })
}

/// What a command acts on: `--all` is the whole app-wide store — just
/// `[root]` (when it exists); the default is the resolved container's own
/// folder(s), apart from the others sharing its name ([`classify`]).
fn scope(ctx: &Context, all: bool) -> Result<Scope, CliError> {
    use sweetpad_lib::derived_data;

    let home = home()?;
    if all {
        let root = derived_data::app_derived_data_root(&home, true);
        return Ok(Scope {
            own: if root.is_dir() {
                vec![root.clone()]
            } else {
                Vec::new()
            },
            root,
            others: Vec::new(),
            pattern: String::new(),
        });
    }

    let container = resolve::container(ctx)?;
    container_scope(&container, &home)
}

/// The scope of `container`'s own folders, found from `home`. The container is
/// keyed the way Xcode keys it ([`sweetpad_lib::derived_data::ContainerKey`]):
/// a project's embedded workspace by the project, a Swift package by its
/// directory. The folders come from the locator the build names its products
/// through, so a folder Xcode's settings move is found where the build writes
/// it.
fn container_scope(container: &Container, home: &str) -> Result<Scope, CliError> {
    use sweetpad_lib::derived_data;

    let key = derived_data::ContainerKey::of(container.path());
    if key.name.is_empty() {
        return Err(CliError::new(
            "could not determine the project name to scope DerivedData",
        ));
    }
    let locations = derived_data::resolve(&key, home, None, true);
    Ok(classify(
        &locations.derived_data_root,
        &locations.folder,
        &key.name,
        &sweetpad_lib::project::standardize(&key.container),
    ))
}

/// Split the folders under `root` named for `base` into the ones the container
/// at `keyed` wrote and the rest. A name alone cannot tell them apart: every
/// checkout of a project writes a `<Name>-<hash>` folder, with the hash taken
/// over its own path. So a folder is this container's when it is `folder`, the
/// one the container's builds write, or when the `info.plist` Xcode records
/// in it names `keyed` as the workspace it was written for.
fn classify(root: &Path, folder: &Path, base: &str, keyed: &Path) -> Scope {
    use sweetpad_lib::derived_data::{hashed_name, workspace_path};

    let own_name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut own = Vec::new();
    let mut others = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !matches_project(&name, base) {
                continue;
            }
            let path = entry.path();
            let written_here = name == own_name
                || workspace_path(&path)
                    .is_some_and(|wp| sweetpad_lib::project::standardize(&wp) == keyed);
            if written_here {
                own.push(path);
            } else {
                others.push(path);
            }
        }
    }
    own.sort();
    others.sort();
    Scope {
        root: root.to_path_buf(),
        own,
        others,
        pattern: format!("{}-*", hashed_name(base)),
    }
}

/// Whether a folder is named for the project `base`, whoever wrote it: Xcode
/// names a hash-keyed DerivedData folder `<Name>-<hash>`, collapsing
/// whitespace runs in the name to `_` (`My App` → `My_App-<hash>`; see
/// [`sweetpad_lib::derived_data::hashed_name`]). A workspace-relative location
/// writes the bare name instead, which is the exact match below.
fn matches_project(entry: &str, base: &str) -> bool {
    entry == base
        || entry.starts_with(&format!(
            "{}-",
            sweetpad_lib::derived_data::hashed_name(base)
        ))
}

/// Recursively sum the size of regular files under `path` (symlinks are not
/// followed). Unreadable entries are skipped rather than failing the command.
fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file()
                && let Ok(meta) = entry.metadata()
            {
                total += meta.len();
            }
        }
    }
    total
}

/// Human-readable byte size (binary units).
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    #[allow(clippy::cast_precision_loss)]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    #[test]
    fn project_match_is_exact_or_hash_suffixed() {
        assert!(matches_project("MyApp", "MyApp"));
        assert!(matches_project("MyApp-abcdef123", "MyApp"));
        // A different project sharing a prefix must not match.
        assert!(!matches_project("MyAppHelper-abc", "MyApp"));
        assert!(!matches_project("Other-xyz", "MyApp"));
        // Xcode collapses whitespace runs to `_` in the folder name, and
        // touches nothing else — a hyphen is not sanitized.
        assert!(matches_project("My_App-abcdef123", "My App"));
        assert!(matches_project("My-App-abcdef123", "My-App"));
        assert!(!matches_project("My_App-abcdef123", "My-App"));
        assert!(!matches_project("My_AppHelper-abc", "My App"));
    }

    /// A DerivedData root holding the folders two copies of one project wrote,
    /// each named `<Name>-<hash of its own path>`, beside a prefix collision
    /// and an unrelated project.
    struct TwoCopies {
        root: TempDir,
        first: PathBuf,
        second: PathBuf,
    }

    impl TwoCopies {
        fn new(tag: &str, name: &str) -> Self {
            let root = TempDir::new(&format!("sweetpad-dd-{tag}"));
            let mut keyed = Vec::new();
            for copy in ["first", "second"] {
                let project = root.join(copy).join(format!("{name}.xcodeproj"));
                std::fs::create_dir_all(&project).unwrap();
                keyed.push(sweetpad_lib::project::standardize(&project));
            }
            let second = keyed.pop().unwrap();
            let first = keyed.pop().unwrap();
            let store = root.join("DerivedData");
            for path in [&first, &second] {
                std::fs::create_dir_all(store.join(folder_for(name, path))).unwrap();
            }
            std::fs::create_dir_all(store.join(format!("{name}Helper-abc"))).unwrap();
            std::fs::create_dir_all(store.join("Other-abc")).unwrap();
            Self {
                root,
                first,
                second,
            }
        }

        fn store(&self) -> PathBuf {
            self.root.join("DerivedData")
        }
    }

    fn folder_for(name: &str, keyed: &Path) -> String {
        sweetpad_lib::derived_data::hashed_folder(
            name,
            &sweetpad_lib::derived_data::container_hash(keyed),
        )
    }

    /// [`classify`] against a store in the stock layout, where the container
    /// at `keyed` writes the `<Name>-<hash>` folder.
    fn classify_stock(store: &Path, name: &str, keyed: &Path) -> Scope {
        classify(store, &store.join(folder_for(name, keyed)), name, keyed)
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    /// Two copies of one project (two checkouts, a worktree, a temp copy)
    /// write folders with the same name prefix, and each copy's scope is its
    /// own folder alone.
    #[test]
    fn each_copy_of_a_project_owns_only_its_own_folder() {
        let copies = TwoCopies::new("copies", "MyApp");
        let first = folder_for("MyApp", &copies.first);
        let second = folder_for("MyApp", &copies.second);
        assert_ne!(first, second);

        let scope = classify_stock(&copies.store(), "MyApp", &copies.first);
        assert_eq!(names(&scope.own), std::slice::from_ref(&first));
        assert_eq!(names(&scope.others), std::slice::from_ref(&second));
        assert_eq!(
            scope.others_note("kept").as_deref(),
            Some("kept 1 other 'MyApp-*' folder(s) from other checkouts or same-named projects")
        );

        let scope = classify_stock(&copies.store(), "MyApp", &copies.second);
        assert_eq!(names(&scope.own), [second]);
        assert_eq!(names(&scope.others), [first]);
    }

    /// A folder whose name does not carry this container's hash is still its
    /// own when the `info.plist` Xcode wrote into it names the container, and
    /// another's when it names another.
    #[test]
    fn the_recorded_workspace_path_claims_a_folder() {
        let copies = TwoCopies::new("recorded", "MyApp");
        let record = |folder: &str, workspace: &Path| {
            let dir = copies.store().join(folder);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("info.plist"),
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\
                     \t<key>WorkspacePath</key>\n\t<string>{}</string>\n</dict>\n</plist>\n",
                    workspace.display()
                ),
            )
            .unwrap();
        };
        record("MyApp-recordedforfirst", &copies.first);
        record("MyApp-recordedforsecond", &copies.second);

        let scope = classify_stock(&copies.store(), "MyApp", &copies.first);
        let mut own = vec![
            "MyApp-recordedforfirst".to_string(),
            folder_for("MyApp", &copies.first),
        ];
        own.sort();
        assert_eq!(names(&scope.own), own);
        assert!(
            names(&scope.others).contains(&"MyApp-recordedforsecond".to_string()),
            "{:?}",
            scope.others
        );
    }

    /// The collapsed spelling of a name with spaces is the one both sets
    /// match, and the one the note names.
    #[test]
    fn a_name_with_spaces_scopes_by_its_folder_spelling() {
        let copies = TwoCopies::new("spaces", "My App");
        let scope = classify_stock(&copies.store(), "My App", &copies.first);
        assert_eq!(names(&scope.own), [folder_for("My App", &copies.first)]);
        assert!(names(&scope.own)[0].starts_with("My_App-"));
        assert_eq!(names(&scope.others), [folder_for("My App", &copies.second)]);
        assert!(scope.others_note("kept").unwrap().contains("'My_App-*'"));
    }

    /// A project named through its embedded workspace scopes to the
    /// project's own folder: `xcodebuild -workspace
    /// Foo.xcodeproj/project.xcworkspace` builds into `Foo-<hash of the
    /// project>` (Xcode 27.0), and no `project-*` folder is anyone's concern.
    #[test]
    fn an_embedded_workspace_scopes_to_its_projects_folder() {
        let home = TempDir::new("sweetpad-dd-stub");
        let project = home.join("src/MyApp.xcodeproj");
        let stub = project.join("project.xcworkspace");
        std::fs::create_dir_all(&stub).unwrap();
        let store = home.join("Library/Developer/Xcode/DerivedData");
        let own = folder_for("MyApp", &sweetpad_lib::project::standardize(&project));
        std::fs::create_dir_all(store.join(&own)).unwrap();
        std::fs::create_dir_all(store.join("project-abc")).unwrap();

        let scope =
            container_scope(&Container::Workspace(stub), &home.display().to_string()).unwrap();
        assert_eq!(names(&scope.own), [own]);
        assert!(scope.others.is_empty(), "{:?}", scope.others);
    }

    /// A Swift package scopes to the folder named for its directory.
    #[test]
    fn a_package_scopes_to_the_folder_named_for_its_directory() {
        let home = TempDir::new("sweetpad-dd-package");
        let dir = home.join("src/MyLib");
        std::fs::create_dir_all(&dir).unwrap();
        let store = home.join("Library/Developer/Xcode/DerivedData");
        let own = folder_for("MyLib", &sweetpad_lib::project::standardize(&dir));
        std::fs::create_dir_all(store.join(&own)).unwrap();
        std::fs::create_dir_all(store.join("Package-abc")).unwrap();

        let scope = container_scope(
            &Container::SwiftPackage(dir.join("Package.swift")),
            &home.display().to_string(),
        )
        .unwrap();
        assert_eq!(names(&scope.own), [own]);
        assert!(scope.others.is_empty(), "{:?}", scope.others);
    }

    /// A workspace-relative DerivedData location writes the bare `<Name>`, with
    /// no hash to recognise it by, and that folder is the container's own.
    #[test]
    fn the_bare_folder_a_relative_location_writes_is_the_containers_own() {
        let copies = TwoCopies::new("bare", "MyApp");
        std::fs::create_dir_all(copies.store().join("MyApp")).unwrap();
        let scope = classify(
            &copies.store(),
            &copies.store().join("MyApp"),
            "MyApp",
            &copies.first,
        );
        assert_eq!(names(&scope.own), ["MyApp"]);
        assert_eq!(scope.others.len(), 2, "{:?}", scope.others);
        assert_eq!(scope.root, copies.store());
    }

    /// No folder of this project's name, or none at all, is an empty scope
    /// with nothing to note.
    #[test]
    fn a_missing_store_is_an_empty_scope() {
        let dir = TempDir::new("sweetpad-dd-missing");
        let scope = classify_stock(
            &dir.join("DerivedData"),
            "MyApp",
            &dir.join("MyApp.xcodeproj"),
        );
        assert!(scope.own.is_empty() && scope.others.is_empty());
        assert_eq!(scope.others_note("kept"), None);
    }

    #[test]
    fn format_size_scales_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KiB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn dir_size_sums_nested_files() {
        let dir = TempDir::new("sweetpad-dd-size");
        let sub = dir.join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("a.txt"), b"1234").unwrap(); // 4 bytes
        std::fs::write(sub.join("b.txt"), b"567890").unwrap(); // 6 bytes
        assert_eq!(dir_size(&dir), 10);
    }
}
