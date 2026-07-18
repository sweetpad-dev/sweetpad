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
//! Two behaviours here are counter-intuitive and are pinned by tests:
//!
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

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::file_cache::ParseCache;
use crate::pbxproj::Value;

/// Where a container's build output lands, with every Xcode location setting
/// already applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// `SYMROOT` — and `BUILD_DIR` / `BUILD_ROOT` through it.
    pub products: PathBuf,
    /// `OBJROOT` / `TEMP_ROOT`.
    pub intermediates: PathBuf,
    /// `DERIVED_DATA_DIR`: the root holding every per-container folder, not
    /// this container's own folder. Seeds xcspec defaults like
    /// `MODULE_CACHE_DIR = $(DERIVED_DATA_DIR)/ModuleCache.noindex`.
    pub derived_data_root: PathBuf,
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

/// Resolve the output locations for `container`.
///
/// `name` / `hash` are the container's stem and its 28-char DerivedData hash
/// (see [`crate::xcode_hash`]); `home` is the user's home directory, passed in
/// rather than read so callers can pin it in tests. `derived_data_flag` is
/// `xcodebuild -derivedDataPath`. `consult_xcode` gates every read of host
/// state: with it `false` this is a pure function of its arguments and yields
/// Xcode's stock layout, which is what the oracle suites resolve against.
#[must_use]
pub fn resolve(
    container: &Path,
    name: &str,
    hash: &str,
    home: &str,
    derived_data_flag: Option<&Path>,
    consult_xcode: bool,
) -> Locations {
    let settings = if consult_xcode {
        read_workspace_settings(container)
    } else {
        WorkspaceSettings::default()
    };
    apply(
        &settings,
        container,
        name,
        hash,
        &app_derived_data_root(home, consult_xcode),
        derived_data_flag,
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
            // segment only exists to disambiguate a shared root.
            Some((base, false)) => (base.join(name), base),
            Some((base, true)) => (base.join(format!("{name}-{hash}")), base),
            None => (
                app_root.join(format!("{name}-{hash}")),
                app_root.to_path_buf(),
            ),
        }
    };

    // A custom build location replaces the products/intermediates dirs
    // outright: no `Build/Products` suffix, no container segment, and it wins
    // over `-derivedDataPath`.
    if let Some((products, intermediates)) = custom_build_location(settings, container, app_root) {
        return Locations {
            products,
            intermediates,
            derived_data_root: root,
        };
    }

    Locations {
        products: folder.join("Build/Products"),
        intermediates: folder.join("Build/Intermediates.noindex"),
        derived_data_root: root,
    }
}

/// The root every per-container folder sits in: the app-wide custom location
/// when set, else Xcode's stock path. A caller with no `$HOME` (the sandboxed
/// test harness) gets `/tmp` so paths stay absolute.
fn app_derived_data_root(home: &str, consult_xcode: bool) -> PathBuf {
    if consult_xcode && let Some(custom) = read_xcode_pref() {
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

/// The directory a "relative to workspace" path hangs off: the one holding the
/// container, not the container itself.
fn container_dir(container: &Path) -> PathBuf {
    container
        .parent()
        .map_or_else(PathBuf::new, Path::to_path_buf)
}

static PREF_CACHE: LazyLock<ParseCache<Option<PathBuf>>> = LazyLock::new(ParseCache::new);
static SETTINGS_CACHE: LazyLock<ParseCache<WorkspaceSettings>> = LazyLock::new(ParseCache::new);

/// `IDECustomDerivedDataLocation` from the user's Xcode preferences.
///
/// Read straight off disk rather than through `defaults`: the preferences file
/// is a binary plist that [`crate::bplist`] already handles, and macOS writes
/// the key through on change. A value Xcode has set but not yet flushed from
/// `cfprefsd` is invisible here until it lands, which in practice means a
/// just-changed setting can take a moment to be seen.
fn read_xcode_pref() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let path = Path::new(&home).join("Library/Preferences/com.apple.dt.Xcode.plist");
    let parsed = PREF_CACHE
        .get_or_parse(&path, |path| -> Result<_, ()> {
            let Ok(value) = crate::bplist::parse_file(path) else {
                return Ok(None);
            };
            let location = value
                .as_dict()
                .and_then(|dict| dict.get("IDECustomDerivedDataLocation"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            Ok(location)
        })
        .ok()?;
    (*parsed).clone()
}

/// The container's per-user workspace settings. A `.xcodeproj` keeps them in
/// its embedded `project.xcworkspace`; a `.xcworkspace` holds them directly.
/// Xcode writes one directory per user, so read whichever matches `$USER` and
/// fall back to a lone directory when the name doesn't line up (a home moved
/// between accounts).
fn read_workspace_settings(container: &Path) -> WorkspaceSettings {
    let base = if container.extension().is_some_and(|e| e == "xcworkspace") {
        container.to_path_buf()
    } else {
        container.join("project.xcworkspace")
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
    let mine = std::env::var_os("USER")
        .map(|user| xcuserdata.join(format!("{}.xcuserdatad", user.to_string_lossy())));
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
    let Ok(bytes) = std::fs::read(path) else {
        return WorkspaceSettings::default();
    };
    let pairs = if bytes.starts_with(b"bplist00") {
        binary_plist_strings(&bytes)
    } else {
        std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| crate::xcscheme::parse(text).ok())
            .map(|root| xml_plist_strings(&root))
            .unwrap_or_default()
    };
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

    const NAME: &str = "MyApp";
    const HASH: &str = "hflzcfrhwsudrtecqhfwedxhnshc";
    const HOME: &str = "/Users/someone";

    fn container() -> PathBuf {
        PathBuf::from("/src/wstest/MyApp.xcworkspace")
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
        let out = resolve(
            &container(),
            NAME,
            HASH,
            HOME,
            None,
            /* consult_xcode */ false,
        );
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
    }

    #[test]
    fn derived_data_path_flag_drops_the_container_segment() {
        let out = resolve(
            &container(),
            NAME,
            HASH,
            HOME,
            Some(Path::new("/flag-dd")),
            false,
        );
        assert_eq!(out.products, PathBuf::from("/flag-dd/Build/Products"));
        assert_eq!(out.derived_data_root, PathBuf::from("/flag-dd"));
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
    /// mtime-keyed caches can't serve one case's parse to another.
    fn scratch_container(case: &str, kind: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sweetpad-dd-{}-{case}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let container = root.join(format!("MyApp.{kind}"));
        std::fs::create_dir_all(&container).expect("create container");
        container
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
        let container = scratch_container("ws-user", "xcworkspace");
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
        let container = scratch_container("proj-user", "xcodeproj");
        write_settings(
            &container.join("project.xcworkspace/xcuserdata/someone.xcuserdatad"),
            ABSOLUTE_DD,
        );
        let settings = read_workspace_settings(&container);
        assert_eq!(settings.derived_data_style.as_deref(), Some("AbsolutePath"));
    }

    #[test]
    fn ignores_the_shared_settings_copy() {
        // xcodebuild honours these keys only in `xcuserdata`; the shared file
        // carries scheme-autocreation settings and nothing we act on.
        let container = scratch_container("shared", "xcworkspace");
        write_settings(&container.join("xcshareddata"), ABSOLUTE_DD);
        assert_eq!(
            read_workspace_settings(&container),
            WorkspaceSettings::default()
        );
    }

    #[test]
    fn a_container_without_settings_reads_as_stock() {
        let container = scratch_container("bare", "xcworkspace");
        assert_eq!(
            read_workspace_settings(&container),
            WorkspaceSettings::default()
        );
    }

    #[test]
    fn a_malformed_settings_file_reads_as_stock() {
        let container = scratch_container("malformed", "xcworkspace");
        let dir = container.join("xcuserdata/someone.xcuserdatad");
        std::fs::create_dir_all(&dir).expect("create settings dir");
        std::fs::write(dir.join("WorkspaceSettings.xcsettings"), "not a plist")
            .expect("write settings");
        assert_eq!(
            read_workspace_settings(&container),
            WorkspaceSettings::default()
        );
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
