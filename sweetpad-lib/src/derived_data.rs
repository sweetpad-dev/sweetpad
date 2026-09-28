//! Xcode's DerivedData and build-location configuration.
//!
//! Xcode lets a user move build output in two places, and `xcodebuild` honours
//! them even though they're set through the IDE:
//!
//! - **App-wide** (Xcode → Settings → Locations → Derived Data):
//!   `IDECustomDerivedDataLocation` in `com.apple.dt.Xcode`.
//! - **Per-container** (File → Workspace Settings): `DerivedDataLocationStyle`
//!   and `BuildLocationStyle` in the container's `WorkspaceSettings.xcsettings`.
//!
//! The keys only take effect in the container's **`xcuserdata`** copy; the
//! `xcshareddata` copy that [`crate::scheme`] reads for scheme autocreation is
//! ignored for these. Xcode's own naming agrees — `IDEFoundation` spells them
//! `IDEWorkspaceUserSettings_BuildLocationStyle` and friends.
//!
//! Three behaviours here are counter-intuitive and are pinned by tests:
//!
//! - The `<Name>` of a `<Name>-<hash>` folder is whitespace-collapsed (see
//!   [`hashed_name`]); the bare `<Name>` a workspace-relative location writes
//!   is not.
//! - `BuildLocationStyle = CustomLocation` outranks `-derivedDataPath`. The
//!   flag still moves DerivedData, but products and intermediates stay where
//!   the container's custom location puts them.
//! - `CustomBuildLocationType = RelativeToDerivedData` resolves against the
//!   *app-wide* DerivedData root, ignoring the container's own
//!   `DerivedDataCustomLocation`.
//!
//! `BuildLocationStyle = DeterminedByTargets` (the legacy "place output next to
//! the project" style) reads as a no-op: `xcodebuild` leaves output in
//! DerivedData. App-wide `IDEBuildLocationStyle` is likewise ignored — only
//! DerivedData has an app-wide setting `xcodebuild` respects.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::file_cache::ParseCache;

/// Where a container's build output lands, with every Xcode location setting
/// already applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// The container's own DerivedData folder — `<root>/<Name>-<hash>` in the
    /// stock layout. Xcode keeps the container's index, logs and package
    /// checkouts here even when a custom build location moves its products.
    pub folder: PathBuf,
    /// `SYMROOT` — and `BUILD_DIR` / `BUILD_ROOT` through it.
    pub products: PathBuf,
    /// `OBJROOT` / `TEMP_ROOT`.
    pub intermediates: PathBuf,
    /// `DERIVED_DATA_DIR`: the root holding every per-container folder, not
    /// this container's own folder. Seeds xcspec defaults like
    /// `MODULE_CACHE_DIR = $(DERIVED_DATA_DIR)/ModuleCache.noindex`.
    pub derived_data_root: PathBuf,
}

/// What Xcode keys a container's DerivedData folder by: the container it
/// opened, the name the folder starts with, and the hash of its path.
///
/// Everything that names a container's DerivedData starts from
/// [`ContainerKey::of`], so every spelling a caller can hand in lands on the
/// folder `xcodebuild` writes:
///
/// - A project's embedded `Foo.xcodeproj/project.xcworkspace` stands for the
///   project ([`crate::workspace::normalize_stub_workspace`]).
/// - A Swift package is keyed by its directory and named for it, so its
///   `Package.swift` stands for the directory holding it.
/// - The hash is taken over the standardized path ([`container_hash`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerKey {
    /// The container, absolute and spelled as the caller spelled it: a
    /// `.xcworkspace`, a `.xcodeproj`, or a Swift package's directory. Its
    /// per-user workspace settings are read from here.
    pub container: PathBuf,
    /// The container's name, before [`hashed_name`] rewrites its whitespace.
    pub name: String,
    /// The 28-char [`container_hash`] of the container.
    pub hash: String,
}

impl ContainerKey {
    /// The key for the container at `path`: a `.xcworkspace` (a project's
    /// embedded one included), a `.xcodeproj`, a Swift package directory, or
    /// the `Package.swift` inside one. A relative path anchors at the current
    /// directory.
    #[must_use]
    pub fn of(path: &Path) -> Self {
        let absolute = crate::project::absolutize(path);
        let container = if absolute.file_name() == Some(OsStr::new("Package.swift")) {
            absolute
                .parent()
                .map_or_else(|| absolute.clone(), Path::to_path_buf)
        } else {
            crate::workspace::normalize_stub_workspace(&absolute)
        };
        let name = if is_package(&container) {
            container.file_name()
        } else {
            container.file_stem()
        }
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
        let hash = container_hash(&container);
        Self {
            container,
            name,
            hash,
        }
    }

    /// The `<Name>-<hash>` folder a hash-keyed location writes.
    #[must_use]
    pub fn folder_name(&self) -> String {
        hashed_folder(&self.name, &self.hash)
    }
}

/// Whether `container` is a Swift package directory rather than an Xcode
/// bundle.
fn is_package(container: &Path) -> bool {
    !matches!(
        container.extension().and_then(OsStr::to_str),
        Some("xcodeproj" | "xcworkspace")
    )
}

/// The per-container `WorkspaceSettings.xcsettings` keys we act on. Absent
/// keys stay `None`, which reads as "inherit the app-wide setting".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WorkspaceSettings {
    derived_data_style: Option<String>,
    derived_data_location: Option<String>,
    build_location_style: Option<String>,
    build_location_type: Option<String>,
    products_path: Option<String>,
    intermediates_path: Option<String>,
}

/// Resolve the output locations for the container `key` names.
///
/// `home` is the user's home directory, passed in rather than read so callers
/// can pin it in tests. `derived_data_flag` is `xcodebuild -derivedDataPath`.
/// `consult_xcode` gates every read of host state: with it `false` this is a
/// pure function of its arguments and yields Xcode's stock layout, which is
/// what the oracle suites resolve against.
#[must_use]
pub fn resolve(
    key: &ContainerKey,
    home: &str,
    derived_data_flag: Option<&Path>,
    consult_xcode: bool,
) -> Locations {
    let settings = if consult_xcode {
        read_workspace_settings(&key.container)
    } else {
        WorkspaceSettings::default()
    };
    // `xcodebuild` reports a `-derivedDataPath` under `/private/tmp` as
    // `/tmp/…`, the way it spells the project, with its symlinks resolved once
    // the directory exists.
    let derived_data_flag = derived_data_flag.map(crate::project::derived_data_spelling);
    apply(
        &settings,
        &key.container,
        &key.name,
        &key.hash,
        &app_derived_data_root(home, consult_xcode),
        derived_data_flag.as_deref(),
    )
}

/// The resolution itself, once every setting has been read. Split out so the
/// precedence rules can be tested without writing into the runner's home.
fn apply(
    settings: &WorkspaceSettings,
    container: &Path,
    name: &str,
    hash: &str,
    app_root: &Path,
    derived_data_flag: Option<&Path>,
) -> Locations {
    // The container's own DerivedData folder — `<root>/<Name>-<hash>` in the
    // stock layout. `-derivedDataPath` replaces the whole thing (no container
    // segment underneath it), which is why it also becomes the root below.
    let (folder, root) = if let Some(flag) = derived_data_flag {
        (flag.to_path_buf(), flag.to_path_buf())
    } else {
        match derived_data_override(settings, container) {
            // "Relative to workspace" keys the folder by bare name — the hash
            // segment only exists to disambiguate a shared root. The name is
            // spelled verbatim here, unlike the hash-keyed arms below.
            Some((base, false)) => (base.join(name), base),
            Some((base, true)) => (base.join(hashed_folder(name, hash)), base),
            None => (
                app_root.join(hashed_folder(name, hash)),
                app_root.to_path_buf(),
            ),
        }
    };

    // A custom build location replaces the products/intermediates dirs
    // outright: no `Build/Products` suffix, no container segment, and it wins
    // over `-derivedDataPath`.
    if let Some((products, intermediates)) = custom_build_location(settings, container, app_root) {
        return Locations {
            folder,
            products,
            intermediates,
            derived_data_root: root,
        };
    }

    Locations {
        products: folder.join("Build/Products"),
        intermediates: folder.join("Build/Intermediates.noindex"),
        folder,
        derived_data_root: root,
    }
}

/// The `<Name>-<hash>` folder a hash-keyed DerivedData location writes for a
/// container named `name` whose path hashes to `hash` (see
/// [`container_hash`]).
#[must_use]
pub fn hashed_folder(name: &str, hash: &str) -> String {
    format!("{}-{hash}", hashed_name(name))
}

/// The 28-char hash `xcodebuild` keys `container`'s DerivedData folder by: the
/// [`crate::xcode_hash`] of the container's standardized path (see
/// [`crate::project::standardize`]). Hashing any other spelling names a folder
/// `xcodebuild` never writes when the container is reached through a symlink
/// or spelled `/private/tmp/…`.
///
/// `container` is the `.xcodeproj` or `.xcworkspace` the build opened, or a
/// Swift package's directory.
#[must_use]
pub fn container_hash(container: &Path) -> String {
    crate::xcode_hash::derived_data_hash(
        &crate::project::standardize(container).display().to_string(),
    )
}

/// Xcode's spelling of a container name inside a `<Name>-<hash>` DerivedData
/// folder: every run of whitespace becomes a single `_`, so `ARTA NYC` keys
/// `ARTA_NYC-<hash>` and `A  B` keys `A_B-<hash>`.
///
/// Whitespace is the only thing touched, and only these four characters count
/// as whitespace: space, tab, LF, CR. `IDEFoundation` builds the folder name in
/// `+[IDEWorkspaceArena nameForWorkspaceArenaWithBaseName:gristInput:]`, which
/// calls `dvt_stringByReplacingWhitespaceRunsWithCharacter:'_' range:` — and
/// that tests each UTF-16 unit against the bitmask `0x1_0000_2600` under a
/// `c <= 0x20` guard. So vertical tab and form feed pass through, as does every
/// Unicode space (U+00A0, U+2002, …) — this is not Foundation's
/// `whitespaceAndNewlineCharacterSet`. Punctuation (`+ @ ( ) , & ' . - % ~ ! =
/// # :`) and non-ASCII letters are untouched, there is no case folding, and
/// there is no length cap: a name long enough to push the folder past 255 bytes
/// fails the build outright rather than being truncated.
///
/// The hash itself is computed from the container's *unsanitized* path (see
/// [`crate::xcode_hash`]), so only this one segment differs.
#[must_use]
pub fn hashed_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut in_run = false;
    for c in name.chars() {
        let is_whitespace = matches!(c, ' ' | '\t' | '\n' | '\r');
        if !is_whitespace {
            out.push(c);
        } else if !in_run {
            out.push('_');
        }
        in_run = is_whitespace;
    }
    out
}

/// The root every per-container folder sits in unless the container moves its
/// own: the app-wide custom location (`IDECustomDerivedDataLocation` in
/// `<home>`'s Xcode preferences) when `consult_xcode` is set and it is,
/// else Xcode's stock path. A caller with no `$HOME` (the sandboxed test
/// harness) gets `/tmp` so paths stay absolute.
#[must_use]
pub fn app_derived_data_root(home: &str, consult_xcode: bool) -> PathBuf {
    if consult_xcode
        && !home.is_empty()
        && let Some(custom) = read_xcode_pref(Path::new(home))
    {
        return custom;
    }
    if home.is_empty() {
        return PathBuf::from("/tmp/DerivedData");
    }
    PathBuf::from(format!("{home}/Library/Developer/Xcode/DerivedData"))
}

/// The container's own DerivedData base, as `(base, keyed_by_hash)`. `None`
/// leaves the app-wide root in charge.
fn derived_data_override(
    settings: &WorkspaceSettings,
    container: &Path,
) -> Option<(PathBuf, bool)> {
    let location = settings.derived_data_location.as_deref()?;
    match settings.derived_data_style.as_deref()? {
        "AbsolutePath" => Some((PathBuf::from(location), true)),
        "WorkspaceRelativePath" => Some((container_dir(container).join(location), false)),
        // `Default` — and anything unrecognised — defers to the app-wide root.
        _ => None,
    }
}

/// The products/intermediates pair a `CustomLocation` build style pins, or
/// `None` when the container leaves the build location alone.
fn custom_build_location(
    settings: &WorkspaceSettings,
    container: &Path,
    app_root: &Path,
) -> Option<(PathBuf, PathBuf)> {
    if settings.build_location_style.as_deref() != Some("CustomLocation") {
        return None;
    }
    let products = settings.products_path.as_deref()?;
    let intermediates = settings.intermediates_path.as_deref()?;
    // `RelativeToDerivedData` deliberately reads the app-wide root, not the
    // container's own DerivedData override.
    let base = match settings.build_location_type.as_deref() {
        Some("Absolute") => PathBuf::new(),
        Some("RelativeToDerivedData") => app_root.to_path_buf(),
        Some("RelativeToWorkspace") => container_dir(container),
        _ => return None,
    };
    Some((base.join(products), base.join(intermediates)))
}

/// The directory a "relative to workspace" path hangs off: the one holding an
/// Xcode container, or a Swift package's own directory. Xcode 27 builds a
/// package whose settings say `WorkspaceRelativePath` `DD` into
/// `<package>/DD/<package name>`.
fn container_dir(container: &Path) -> PathBuf {
    if is_package(container) {
        return container.to_path_buf();
    }
    container
        .parent()
        .map_or_else(PathBuf::new, Path::to_path_buf)
}

static PREF_CACHE: LazyLock<ParseCache<Option<PathBuf>>> = LazyLock::new(ParseCache::new);
static SETTINGS_CACHE: LazyLock<ParseCache<WorkspaceSettings>> = LazyLock::new(ParseCache::new);

/// `IDECustomDerivedDataLocation` from the Xcode preferences under `home`.
///
/// Read straight off disk rather than through `defaults`: the preferences file
/// is a binary plist that [`crate::bplist`] already handles, and macOS writes
/// the key through on change. A value Xcode has set but not yet flushed from
/// `cfprefsd` is invisible here until it lands, which in practice means a
/// just-changed setting can take a moment to be seen. An XML copy of the file
/// reads the same.
fn read_xcode_pref(home: &Path) -> Option<PathBuf> {
    let path = home.join("Library/Preferences/com.apple.dt.Xcode.plist");
    let parsed = PREF_CACHE
        .get_or_parse(&path, |path| -> Result<_, ()> {
            let location = plist_strings(path)
                .into_iter()
                .find_map(|(key, value)| (key == "IDECustomDerivedDataLocation").then_some(value))
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            Ok(location)
        })
        .ok()?;
    (*parsed).clone()
}

/// The container's per-user workspace settings. A `.xcodeproj` keeps them in
/// its embedded `project.xcworkspace`, a Swift package in
/// `.swiftpm/xcode/package.xcworkspace`, and a `.xcworkspace` holds them
/// directly. Xcode writes one directory per user, so read whichever matches
/// the account's name ([`crate::host::user`]) and fall back to a lone
/// directory when the name doesn't line up (a home moved between accounts).
fn read_workspace_settings(container: &Path) -> WorkspaceSettings {
    let base = match container.extension().and_then(OsStr::to_str) {
        Some("xcworkspace") => container.to_path_buf(),
        Some("xcodeproj") => container.join("project.xcworkspace"),
        _ => crate::workspace::package_scheme_root(container).join("package.xcworkspace"),
    };
    let Some(dir) = user_data_dir(&base.join("xcuserdata")) else {
        return WorkspaceSettings::default();
    };
    let path = dir.join("WorkspaceSettings.xcsettings");
    SETTINGS_CACHE
        .get_or_parse(&path, |path| -> Result<_, ()> {
            Ok(parse_workspace_settings(path))
        })
        .map(|parsed| (*parsed).clone())
        .unwrap_or_default()
}

/// The `<user>.xcuserdatad` directory to read: the current user's when it
/// exists, else the only one present.
fn user_data_dir(xcuserdata: &Path) -> Option<PathBuf> {
    let mine = crate::host::user().map(|user| xcuserdata.join(format!("{user}.xcuserdatad")));
    if let Some(mine) = mine
        && mine.is_dir()
    {
        return Some(mine);
    }
    let mut dirs = std::fs::read_dir(xcuserdata)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "xcuserdatad"));
    let only = dirs.next()?;
    dirs.next().is_none().then_some(only)
}

/// Pull the location keys out of a `WorkspaceSettings.xcsettings`. Xcode writes
/// XML, but the file is a plist so accept the binary spelling too. Anything
/// unreadable yields the stock layout rather than an error — a malformed
/// settings file shouldn't stop a build from resolving.
fn parse_workspace_settings(path: &Path) -> WorkspaceSettings {
    let pairs = plist_strings(path);
    let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    WorkspaceSettings {
        derived_data_style: get("DerivedDataLocationStyle"),
        derived_data_location: get("DerivedDataCustomLocation"),
        build_location_style: get("BuildLocationStyle"),
        build_location_type: get("CustomBuildLocationType"),
        products_path: get("CustomBuildProductsPath"),
        intermediates_path: get("CustomBuildIntermediatesPath"),
    }
}

/// The container a DerivedData folder was written for: the `WorkspacePath` its
/// `info.plist` records, spelled the way Xcode hashed it (a standardized
/// `.xcodeproj`, `.xcworkspace`, or package directory). Xcode writes the file
/// once it builds, so a folder only ever resolved has none, and `None` then
/// says nothing about whose folder it is.
#[must_use]
pub fn workspace_path(folder: &Path) -> Option<PathBuf> {
    plist_strings(&folder.join("info.plist"))
        .into_iter()
        .find_map(|(key, value)| (key == "WorkspacePath").then(|| PathBuf::from(value)))
}

/// Every string-valued top-level key of the plist at `path`, XML or binary.
/// An unreadable or malformed file has none.
fn plist_strings(path: &Path) -> Vec<(String, String)> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    if bytes.starts_with(b"bplist00") {
        binary_plist_strings(&bytes)
    } else {
        std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| crate::xcscheme::parse(text).ok())
            .map(|root| xml_plist_strings(&root))
            .unwrap_or_default()
    }
}

fn binary_plist_strings(bytes: &[u8]) -> Vec<(String, String)> {
    let Ok(value) = crate::bplist::parse(bytes) else {
        return Vec::new();
    };
    let Some(dict) = value.as_dict() else {
        return Vec::new();
    };
    dict.iter()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
        .collect()
}

/// An XML plist dict is a flat `<key>k</key><string>v</string>` run, so pair
/// each key with the element that follows it.
fn xml_plist_strings(root: &crate::xcscheme::Element) -> Vec<(String, String)> {
    let Some(dict) = root.child("dict") else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    let mut children = dict.children.iter();
    while let Some(child) = children.next() {
        if child.name != "key" {
            continue;
        }
        if let Some(value) = children.next()
            && value.name == "string"
        {
            pairs.push((child.text.clone(), value.text.clone()));
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;

    const NAME: &str = "MyApp";
    const HASH: &str = "hflzcfrhwsudrtecqhfwedxhnshc";
    const HOME: &str = "/Users/someone";

    fn container() -> PathBuf {
        PathBuf::from("/src/wstest/MyApp.xcworkspace")
    }

    fn key() -> ContainerKey {
        ContainerKey {
            container: container(),
            name: NAME.into(),
            hash: HASH.into(),
        }
    }

    /// Drive the same resolution [`resolve`] runs, with the settings supplied
    /// instead of read — pinning them on disk would mean writing into the
    /// runner's real home.
    fn resolve_with(settings: &WorkspaceSettings, flag: Option<&Path>) -> Locations {
        let app_root = PathBuf::from(format!("{HOME}/Library/Developer/Xcode/DerivedData"));
        apply(settings, &container(), NAME, HASH, &app_root, flag)
    }

    #[test]
    fn stock_layout_when_nothing_is_configured() {
        let out = resolve(&key(), HOME, None, /* consult_xcode */ false);
        assert_eq!(
            out.products,
            PathBuf::from(format!(
                "{HOME}/Library/Developer/Xcode/DerivedData/{NAME}-{HASH}/Build/Products"
            ))
        );
        assert_eq!(
            out.intermediates,
            PathBuf::from(format!(
                "{HOME}/Library/Developer/Xcode/DerivedData/{NAME}-{HASH}/Build/Intermediates.noindex"
            ))
        );
        assert_eq!(
            out.derived_data_root,
            PathBuf::from(format!("{HOME}/Library/Developer/Xcode/DerivedData"))
        );
        assert_eq!(
            out.folder,
            PathBuf::from(format!(
                "{HOME}/Library/Developer/Xcode/DerivedData/{NAME}-{HASH}"
            ))
        );
    }

    /// The whitespace rule, pinned against live `xcodebuild -showBuildSettings`
    /// 26.5 and against the bitmask `dvt_stringByReplacingWhitespaceRunsWithCharacter:`
    /// tests each character with.
    #[test]
    fn hashed_name_collapses_whitespace_runs_only() {
        assert_eq!(hashed_name("ARTA NYC"), "ARTA_NYC");
        assert_eq!(hashed_name("MyApp"), "MyApp");
        // A run of any length — of mixed members, too — is one `_`.
        assert_eq!(hashed_name("A  B"), "A_B");
        assert_eq!(hashed_name("A   B"), "A_B");
        assert_eq!(hashed_name("A B  C"), "A_B_C");
        assert_eq!(hashed_name("A \t\r\nB"), "A_B");
        // Tab, LF and CR each count; the edges are replaced, not trimmed.
        assert_eq!(hashed_name("T\tS"), "T_S");
        assert_eq!(hashed_name("T\nS"), "T_S");
        assert_eq!(hashed_name("T\rS"), "T_S");
        assert_eq!(hashed_name(" AB"), "_AB");
        assert_eq!(hashed_name("AB "), "AB_");
        assert_eq!(hashed_name("   "), "_");
    }

    /// Everything the bitmask's `c <= 0x20` guard lets through, which is
    /// everything a "replace non-alphanumerics" reading would have mangled.
    #[test]
    fn hashed_name_leaves_every_other_character_alone() {
        for name in [
            "A+B",
            "A@B",
            "A(B)",
            "A,B",
            "A&B",
            "A'B",
            "A.B",
            "A-B",
            "A%B",
            "A~B",
            "A!B",
            "A=B",
            "A#B",
            "A:B",
            "Å",
            "Kingfisher-Demo",
        ] {
            assert_eq!(hashed_name(name), name, "{name} should pass through");
        }
        // Vertical tab and form feed sit outside the mask despite being ASCII
        // whitespace, so they are not Foundation's whitespace set.
        assert_eq!(hashed_name("T\u{b}S"), "T\u{b}S");
        assert_eq!(hashed_name("T\u{c}S"), "T\u{c}S");
        // Nor is it `whitespaceAndNewlineCharacterSet`: Unicode spaces stay.
        assert_eq!(hashed_name("T\u{a0}S"), "T\u{a0}S");
        assert_eq!(hashed_name("T\u{2002}S"), "T\u{2002}S");
    }

    /// The whole of issue #329: a spaced container builds into `ARTA_NYC-…`,
    /// so resolving `ARTA NYC-…` names a directory that never exists.
    #[test]
    fn stock_layout_collapses_whitespace_in_the_folder_name() {
        let key = ContainerKey {
            container: PathBuf::from("/src/ARTA NYC.xcworkspace"),
            name: "ARTA NYC".into(),
            hash: HASH.into(),
        };
        let out = resolve(&key, HOME, None, /* consult_xcode */ false);
        assert_eq!(
            out.products,
            PathBuf::from(format!(
                "{HOME}/Library/Developer/Xcode/DerivedData/ARTA_NYC-{HASH}/Build/Products"
            ))
        );
    }

    #[test]
    fn absolute_style_collapses_whitespace_in_the_folder_name() {
        let out = apply(
            &WorkspaceSettings {
                derived_data_style: Some("AbsolutePath".into()),
                derived_data_location: Some("/level2".into()),
                ..WorkspaceSettings::default()
            },
            &container(),
            "ARTA NYC",
            HASH,
            Path::new("/app-root"),
            None,
        );
        assert_eq!(
            out.products,
            PathBuf::from(format!("/level2/ARTA_NYC-{HASH}/Build/Products"))
        );
    }

    /// The bare-name style keeps the space. `IDEWorkspaceArena` only sanitizes
    /// on the branch that appends the hash, and `xcodebuild` agrees: a
    /// workspace-relative location writes `MyDD/ARTA NYC/Build/Products`.
    #[test]
    fn workspace_relative_style_keeps_whitespace_verbatim() {
        let out = apply(
            &WorkspaceSettings {
                derived_data_style: Some("WorkspaceRelativePath".into()),
                derived_data_location: Some("MyDD".into()),
                ..WorkspaceSettings::default()
            },
            Path::new("/src/ARTA NYC.xcworkspace"),
            "ARTA NYC",
            HASH,
            Path::new("/app-root"),
            None,
        );
        assert_eq!(
            out.products,
            PathBuf::from("/src/MyDD/ARTA NYC/Build/Products")
        );
    }

    /// A Swift package's "relative to workspace" location hangs off the
    /// package directory itself: Xcode 27 built a package at `…/spm` whose
    /// settings said `WorkspaceRelativePath` `DDRel` into `…/spm/DDRel/spm`.
    #[test]
    fn a_packages_relative_location_hangs_off_its_own_directory() {
        let out = apply(
            &WorkspaceSettings {
                derived_data_style: Some("WorkspaceRelativePath".into()),
                derived_data_location: Some("DDRel".into()),
                ..WorkspaceSettings::default()
            },
            Path::new("/src/spm"),
            "spm",
            HASH,
            Path::new("/app-root"),
            None,
        );
        assert_eq!(out.folder, PathBuf::from("/src/spm/DDRel/spm"));
    }

    /// A project named through its embedded workspace is keyed by the
    /// project. `xcodebuild -workspace B10PkgApp.xcodeproj/project.xcworkspace`
    /// on Xcode 27.0 built into this folder, and its `info.plist` recorded the
    /// `.xcodeproj` as the `WorkspacePath`.
    #[test]
    fn an_embedded_workspace_is_keyed_by_its_project() {
        let project = Path::new(
            "/tmp/claude-503/-Users-hyzyla-home-Developer-sweetpad/\
             e7fd5499-ae07-4b48-b81b-2d5c20c73fc0/scratchpad/b10-pkg/app/B10PkgApp.xcodeproj",
        );
        let key = ContainerKey::of(&project.join("project.xcworkspace"));
        assert_eq!(key.container, project);
        assert_eq!(key.name, "B10PkgApp");
        assert_eq!(key.folder_name(), "B10PkgApp-ciimrmjrntfnzldgbikhgsjrzxed");
        assert_eq!(key, ContainerKey::of(project));
    }

    /// Xcode keys a package's folder by the package directory, not its
    /// manifest: `xcodebuild -list` in a package at this path wrote
    /// `HashProbeLib-ddiyxpwzwovtfpgqlinqmouqyzjg`.
    #[test]
    fn a_package_is_keyed_by_its_directory() {
        let dir = Path::new(
            "/tmp/claude-503/-Users-hyzyla-home-Developer-sweetpad/\
             e7fd5499-ae07-4b48-b81b-2d5c20c73fc0/scratchpad/spmprobe/HashProbeLib",
        );
        let key = ContainerKey::of(&dir.join("Package.swift"));
        assert_eq!(key.container, dir);
        assert_eq!(key.name, "HashProbeLib");
        assert_eq!(
            key.folder_name(),
            "HashProbeLib-ddiyxpwzwovtfpgqlinqmouqyzjg"
        );
        assert_eq!(key, ContainerKey::of(dir));
    }

    /// A package directory whose name has a dot is named in full, not by its
    /// stem, and a workspace is named by its stem.
    #[test]
    fn a_container_is_named_for_its_bundle_stem_or_package_directory() {
        assert_eq!(ContainerKey::of(Path::new("/src/My.Lib")).name, "My.Lib");
        assert_eq!(
            ContainerKey::of(Path::new("/src/App.xcworkspace")).name,
            "App"
        );
    }

    #[test]
    fn derived_data_path_flag_drops_the_container_segment() {
        let out = resolve(&key(), HOME, Some(Path::new("/flag-dd")), false);
        assert_eq!(out.products, PathBuf::from("/flag-dd/Build/Products"));
        assert_eq!(out.derived_data_root, PathBuf::from("/flag-dd"));
        assert_eq!(out.folder, PathBuf::from("/flag-dd"));
    }

    #[test]
    fn absolute_style_keeps_the_hash() {
        let out = resolve_with(
            &WorkspaceSettings {
                derived_data_style: Some("AbsolutePath".into()),
                derived_data_location: Some("/level2".into()),
                ..WorkspaceSettings::default()
            },
            None,
        );
        assert_eq!(
            out.products,
            PathBuf::from(format!("/level2/{NAME}-{HASH}/Build/Products"))
        );
        assert_eq!(out.derived_data_root, PathBuf::from("/level2"));
        assert_eq!(out.folder, PathBuf::from(format!("/level2/{NAME}-{HASH}")));
    }

    #[test]
    fn workspace_relative_style_drops_the_hash() {
        let out = resolve_with(
            &WorkspaceSettings {
                derived_data_style: Some("WorkspaceRelativePath".into()),
                derived_data_location: Some("MyDD".into()),
                ..WorkspaceSettings::default()
            },
            None,
        );
        assert_eq!(
            out.products,
            PathBuf::from(format!("/src/wstest/MyDD/{NAME}/Build/Products"))
        );
        assert_eq!(out.derived_data_root, PathBuf::from("/src/wstest/MyDD"));
        assert_eq!(
            out.folder,
            PathBuf::from(format!("/src/wstest/MyDD/{NAME}"))
        );
    }

    #[test]
    fn explicit_default_style_defers_to_the_app_root() {
        let out = resolve_with(
            &WorkspaceSettings {
                derived_data_style: Some("Default".into()),
                derived_data_location: Some("/ignored".into()),
                ..WorkspaceSettings::default()
            },
            None,
        );
        assert_eq!(
            out.derived_data_root,
            PathBuf::from(format!("{HOME}/Library/Developer/Xcode/DerivedData"))
        );
    }

    fn custom_location(kind: &str) -> WorkspaceSettings {
        WorkspaceSettings {
            build_location_style: Some("CustomLocation".into()),
            build_location_type: Some(kind.into()),
            products_path: Some(
                if kind == "Absolute" {
                    "/abs-prod"
                } else {
                    "prod"
                }
                .into(),
            ),
            intermediates_path: Some(
                if kind == "Absolute" {
                    "/abs-inter"
                } else {
                    "inter"
                }
                .into(),
            ),
            ..WorkspaceSettings::default()
        }
    }

    #[test]
    fn custom_location_absolute_is_verbatim() {
        let out = resolve_with(&custom_location("Absolute"), None);
        assert_eq!(out.products, PathBuf::from("/abs-prod"));
        assert_eq!(out.intermediates, PathBuf::from("/abs-inter"));
        // The DerivedData folder itself stays where it was.
        assert_eq!(
            out.folder,
            PathBuf::from(format!(
                "{HOME}/Library/Developer/Xcode/DerivedData/{NAME}-{HASH}"
            ))
        );
    }

    #[test]
    fn custom_location_relative_to_derived_data_uses_the_app_root() {
        let out = resolve_with(&custom_location("RelativeToDerivedData"), None);
        assert_eq!(
            out.products,
            PathBuf::from(format!("{HOME}/Library/Developer/Xcode/DerivedData/prod"))
        );
    }

    #[test]
    fn custom_location_relative_to_derived_data_ignores_the_container_override() {
        let mut settings = custom_location("RelativeToDerivedData");
        settings.derived_data_style = Some("AbsolutePath".into());
        settings.derived_data_location = Some("/level2".into());
        let out = resolve_with(&settings, None);
        // The container's own DerivedData override moves the root but not the
        // base this style resolves against.
        assert_eq!(
            out.products,
            PathBuf::from(format!("{HOME}/Library/Developer/Xcode/DerivedData/prod"))
        );
        assert_eq!(out.derived_data_root, PathBuf::from("/level2"));
    }

    #[test]
    fn custom_location_relative_to_workspace_hangs_off_the_container_dir() {
        let out = resolve_with(&custom_location("RelativeToWorkspace"), None);
        assert_eq!(out.products, PathBuf::from("/src/wstest/prod"));
        assert_eq!(out.intermediates, PathBuf::from("/src/wstest/inter"));
    }

    #[test]
    fn custom_location_outranks_the_derived_data_path_flag() {
        let out = resolve_with(&custom_location("Absolute"), Some(Path::new("/flag-dd")));
        assert_eq!(out.products, PathBuf::from("/abs-prod"));
        assert_eq!(out.intermediates, PathBuf::from("/abs-inter"));
        // The flag still owns DerivedData itself.
        assert_eq!(out.derived_data_root, PathBuf::from("/flag-dd"));
    }

    #[test]
    fn determined_by_targets_is_a_no_op() {
        let out = resolve_with(
            &WorkspaceSettings {
                build_location_style: Some("DeterminedByTargets".into()),
                ..WorkspaceSettings::default()
            },
            None,
        );
        assert_eq!(
            out.products,
            PathBuf::from(format!(
                "{HOME}/Library/Developer/Xcode/DerivedData/{NAME}-{HASH}/Build/Products"
            ))
        );
    }

    #[test]
    fn custom_location_needs_both_paths() {
        let mut settings = custom_location("Absolute");
        settings.intermediates_path = None;
        let out = resolve_with(&settings, None);
        assert!(out.products.ends_with("Build/Products"));
    }

    /// A container skeleton under a directory unique to this test, so the
    /// mtime-keyed caches can't serve one case's parse to another. The
    /// directory goes when the returned guard drops.
    fn scratch_container(case: &str, kind: &str) -> (TempDir, PathBuf) {
        let root = TempDir::new(&format!("sweetpad-dd-{case}"));
        let container = root.join(format!("MyApp.{kind}"));
        std::fs::create_dir_all(&container).expect("create container");
        (root, container)
    }

    fn write_settings(dir: &Path, body: &str) {
        std::fs::create_dir_all(dir).expect("create settings dir");
        std::fs::write(
            dir.join("WorkspaceSettings.xcsettings"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
{body}
</dict></plist>"#
            ),
        )
        .expect("write settings");
    }

    const ABSOLUTE_DD: &str = "<key>DerivedDataLocationStyle</key><string>AbsolutePath</string>
<key>DerivedDataCustomLocation</key><string>/level2</string>";

    #[test]
    fn reads_a_workspace_containers_user_settings() {
        let (_root, container) = scratch_container("ws-user", "xcworkspace");
        write_settings(
            &container.join("xcuserdata/someone.xcuserdatad"),
            ABSOLUTE_DD,
        );
        let settings = read_workspace_settings(&container);
        assert_eq!(settings.derived_data_style.as_deref(), Some("AbsolutePath"));
        assert_eq!(settings.derived_data_location.as_deref(), Some("/level2"));
    }

    #[test]
    fn reads_a_project_containers_settings_through_its_inner_workspace() {
        let (_root, container) = scratch_container("proj-user", "xcodeproj");
        write_settings(
            &container.join("project.xcworkspace/xcuserdata/someone.xcuserdatad"),
            ABSOLUTE_DD,
        );
        let settings = read_workspace_settings(&container);
        assert_eq!(settings.derived_data_style.as_deref(), Some("AbsolutePath"));
    }

    /// Xcode keeps a Swift package's workspace settings in
    /// `.swiftpm/xcode/package.xcworkspace`, and `xcodebuild` honours them.
    #[test]
    fn reads_a_packages_settings_through_its_swiftpm_workspace() {
        let root = TempDir::new("sweetpad-dd-package-user");
        let package = root.join("MyLib");
        write_settings(
            &package.join(".swiftpm/xcode/package.xcworkspace/xcuserdata/someone.xcuserdatad"),
            ABSOLUTE_DD,
        );
        let settings = read_workspace_settings(&package);
        assert_eq!(settings.derived_data_style.as_deref(), Some("AbsolutePath"));
    }

    #[test]
    fn ignores_the_shared_settings_copy() {
        // xcodebuild honours these keys only in `xcuserdata`; the shared file
        // carries scheme-autocreation settings and nothing we act on.
        let (_root, container) = scratch_container("shared", "xcworkspace");
        write_settings(&container.join("xcshareddata"), ABSOLUTE_DD);
        assert_eq!(
            read_workspace_settings(&container),
            WorkspaceSettings::default()
        );
    }

    #[test]
    fn a_container_without_settings_reads_as_stock() {
        let (_root, container) = scratch_container("bare", "xcworkspace");
        assert_eq!(
            read_workspace_settings(&container),
            WorkspaceSettings::default()
        );
    }

    #[test]
    fn a_malformed_settings_file_reads_as_stock() {
        let (_root, container) = scratch_container("malformed", "xcworkspace");
        let dir = container.join("xcuserdata/someone.xcuserdatad");
        std::fs::create_dir_all(&dir).expect("create settings dir");
        std::fs::write(dir.join("WorkspaceSettings.xcsettings"), "not a plist")
            .expect("write settings");
        assert_eq!(
            read_workspace_settings(&container),
            WorkspaceSettings::default()
        );
    }

    /// A binary plist holding one top-level dict of string pairs, the format
    /// macOS writes `com.apple.dt.Xcode.plist` in. Every string is ASCII and
    /// under 256 bytes, and the file stays under 256 bytes.
    fn binary_plist(pairs: &[(&str, &str)]) -> Vec<u8> {
        fn string(out: &mut Vec<u8>, s: &str) {
            if s.len() < 15 {
                out.push(0x50 | u8::try_from(s.len()).unwrap());
            } else {
                out.extend([0x5F, 0x10, u8::try_from(s.len()).unwrap()]);
            }
            out.extend(s.as_bytes());
        }
        let count = u8::try_from(pairs.len()).unwrap();
        let mut out = b"bplist00".to_vec();
        let mut offsets = vec![out.len()];
        out.push(0xD0 | count);
        out.extend((1..=count).chain(count + 1..=2 * count));
        let strings = pairs
            .iter()
            .map(|(k, _)| k)
            .chain(pairs.iter().map(|(_, v)| v));
        for s in strings {
            offsets.push(out.len());
            string(&mut out, s);
        }
        let table = out.len();
        out.extend(offsets.iter().map(|&o| u8::try_from(o).unwrap()));
        out.extend([0; 6]);
        out.extend([1, 1]);
        out.extend((offsets.len() as u64).to_be_bytes());
        out.extend(0u64.to_be_bytes());
        out.extend((table as u64).to_be_bytes());
        out
    }

    /// Xcode's Settings → Locations → Derived Data, read out of the
    /// preferences under the home it's given, in the binary form macOS writes
    /// and in XML.
    #[test]
    fn the_app_wide_root_is_the_custom_location_in_xcodes_preferences() {
        let home = TempDir::new("sweetpad-dd-prefs");
        let prefs = home.join("Library/Preferences");
        std::fs::create_dir_all(&prefs).expect("create prefs dir");
        let home_str = home.display().to_string();
        let stock = PathBuf::from(format!("{home_str}/Library/Developer/Xcode/DerivedData"));
        assert_eq!(app_derived_data_root(&home_str, true), stock);

        let file = prefs.join("com.apple.dt.Xcode.plist");
        std::fs::write(
            &file,
            binary_plist(&[
                ("IDEBuildLocationStyle", "Unique"),
                ("IDECustomDerivedDataLocation", "/Volumes/Fast/DD"),
            ]),
        )
        .expect("write prefs");
        assert_eq!(
            app_derived_data_root(&home_str, true),
            PathBuf::from("/Volumes/Fast/DD")
        );
        // Without `consult_xcode` the preferences go unread.
        assert_eq!(app_derived_data_root(&home_str, false), stock);

        // A rewrite is picked up, and XML reads the same.
        std::fs::write(
            &file,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>IDECustomDerivedDataLocation</key><string>/Volumes/Other/DerivedData</string>
</dict></plist>"#,
        )
        .expect("rewrite prefs");
        assert_eq!(
            app_derived_data_root(&home_str, true),
            PathBuf::from("/Volumes/Other/DerivedData")
        );
    }

    /// The record Xcode writes into a DerivedData folder it built into, as
    /// captured from one.
    #[test]
    fn reads_the_container_a_folder_was_written_for() {
        let root = TempDir::new("sweetpad-dd-info");
        let folder = root.join("MacGen-cdprgyivuobbvbdieefufitplmbl");
        std::fs::create_dir_all(&folder).expect("create folder");
        std::fs::write(
            folder.join("info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>LastAccessedDate</key>
	<date>2026-09-26T19:00:39Z</date>
	<key>WorkspacePath</key>
	<string>/tmp/R&amp;D/MacGen/MacGen.xcodeproj</string>
</dict>
</plist>
"#,
        )
        .expect("write info.plist");
        assert_eq!(
            workspace_path(&folder),
            Some(PathBuf::from("/tmp/R&D/MacGen/MacGen.xcodeproj"))
        );
        std::fs::remove_file(folder.join("info.plist")).expect("remove info.plist");
        assert_eq!(workspace_path(&folder), None);
    }

    #[test]
    fn xml_plist_pairs_keys_with_following_strings() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>DerivedDataLocationStyle</key><string>AbsolutePath</string>
<key>IDEWorkspaceSharedSettings_AutocreateContextsIfNeeded</key><false/>
<key>DerivedDataCustomLocation</key><string>/level2</string>
</dict></plist>"#;
        let root = crate::xcscheme::parse(xml).expect("valid plist");
        let pairs = xml_plist_strings(&root);
        assert_eq!(
            pairs,
            vec![
                (
                    "DerivedDataLocationStyle".to_string(),
                    "AbsolutePath".to_string()
                ),
                (
                    "DerivedDataCustomLocation".to_string(),
                    "/level2".to_string()
                ),
            ]
        );
    }
}
