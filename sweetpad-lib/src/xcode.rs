//! What Xcode installation is active right now.
//!
//! The library shells out the same way `xcrun xcodebuild -version` and
//! `xcode-select -p` would, then reads `version.plist` next to the active
//! Developer directory.
//!
//! Both detections are memoized for the life of the process: the node addon is
//! long-lived and resolves against the same Xcode on every call, so the
//! `xcode-select` subprocess ([`detect_developer_dir`]) and the per-install
//! `version.plist` read ([`locate`]) each run once and are served from memory
//! after. `DEVELOPER_DIR` is still read live on every [`detect_developer_dir`]
//! call, so an env override always wins. The trade-off is session staleness:
//! switching the active Xcode (`xcode-select -s`) or updating one in place
//! isn't observed until [`flush_caches`] drops the memos (the extension calls
//! it from "Refresh shell environment") or the process restarts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

/// Snapshot of the active Xcode toolchain.
#[derive(Debug, Clone)]
pub struct ActiveInstall {
    /// Absolute path to the `Developer` directory (what `xcode-select -p`
    /// prints; what `DEVELOPER_DIR` env var overrides).
    pub developer_dir: PathBuf,
    /// `CFBundleShortVersionString` from `version.plist` (e.g. `26.0.1`).
    /// Empty when the plist can't be read.
    pub short_version: String,
    /// `ProductBuildVersion` from `version.plist` (e.g. `17A400`).
    /// Empty when the plist can't be read.
    pub build_version: String,
}

impl ActiveInstall {
    /// Parsed major version (`26` for Xcode 26.0.1). Returns 0 when
    /// [`Self::short_version`] is empty or unparseable.
    #[must_use]
    pub fn major_version(&self) -> u32 {
        self.short_version
            .split('.')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    /// Combined `<short>-<build>` string. xcodebuild's
    /// `-showBuildSettings` reports `XCODE_PRODUCT_BUILD_VERSION` in this
    /// form. Returns `"Unknown"` when either component is missing —
    /// mirroring xcodebuild's own fallback.
    #[must_use]
    pub fn product_build_version(&self) -> String {
        if self.short_version.is_empty() || self.build_version.is_empty() {
            "Unknown".into()
        } else {
            format!("{}-{}", self.short_version, self.build_version)
        }
    }
}

/// Detect the active Xcode by honouring `DEVELOPER_DIR`, then
/// `xcode-select -p`, then a hard-coded fallback. Reads `version.plist`
/// for short + build version when available.
#[must_use]
pub fn active_install() -> ActiveInstall {
    install_at(&detect_developer_dir())
}

/// The install snapshot for a specific Developer directory — the caller-supplied
/// equivalent of [`active_install`], for hosts that resolve the toolchain
/// themselves (e.g. the extension passing its login shell's `DEVELOPER_DIR`)
/// instead of relying on this process's environment.
///
/// The directory is spelled through its canonical path, as [`locate`] spells
/// a layout, so an `Xcode-27.0.0.app` symlink to `Xcode.app` reports the
/// `DEVELOPER_DIR` the build settings resolved against it do. A path that
/// doesn't resolve is kept as given.
#[must_use]
pub fn install_at(developer_dir: &Path) -> ActiveInstall {
    let developer_dir =
        std::fs::canonicalize(developer_dir).unwrap_or_else(|_| developer_dir.to_path_buf());
    let (short_version, build_version) = read_version_plist(&developer_dir);
    ActiveInstall {
        developer_dir,
        short_version,
        build_version,
    }
}

/// Just the active Developer directory — `DEVELOPER_DIR` if set, else
/// `xcode-select -p`, else the Darwin SDK's off macOS, else the standard
/// `/Applications/Xcode.app` path.
#[must_use]
pub fn detect_developer_dir() -> PathBuf {
    if let Ok(val) = std::env::var("DEVELOPER_DIR")
        && !val.is_empty()
    {
        return PathBuf::from(val);
    }
    selected_developer_dir()
}

/// `xcode-select -p` (with the hard-coded fallback when the tool is missing or
/// nothing is selected), memoized for the process. `DEVELOPER_DIR` is honoured
/// ahead of this in [`detect_developer_dir`], so only the subprocess result is
/// frozen — an env override stays live. [`flush_caches`] drops the memo.
fn selected_developer_dir() -> PathBuf {
    let mut cached = selected_cache();
    cached
        .get_or_insert_with(|| {
            if let Ok(output) = Command::new("xcode-select").arg("-p").output()
                && output.status.success()
            {
                let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !s.is_empty() {
                    return PathBuf::from(s);
                }
            }
            darwin_sdk_developer_dir()
                .unwrap_or_else(|| PathBuf::from("/Applications/Xcode.app/Contents/Developer"))
        })
        .clone()
}

/// Where the Darwin Swift SDK bundle is: `SWEETPAD_DARWIN_SDK`, else the one
/// `sweetpad setup` imports. It has the Apple SDKs and Xcode's runtime
/// libraries, and stands in for Xcode on a host without one.
#[must_use]
pub fn darwin_sdk_bundle() -> PathBuf {
    std::env::var_os("SWEETPAD_DARWIN_SDK").map_or_else(
        || {
            PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".swiftpm/swift-sdks/darwin.artifactbundle")
        },
        PathBuf::from,
    )
}

/// Apple's macro plugins that `sweetpad-macros` stands in for. Xcode has
/// them as macOS libraries. The toolchain's own `FoundationMacros` names
/// `FoundationEssentials` in its expansions, which the SDK's Foundation
/// doesn't have.
pub const DARWIN_MACRO_MODULES: &[&str] = &[
    "FoundationMacros",
    "FoundationModelsMacros",
    "PreviewsMacros",
    "SwiftDataMacros",
    "SwiftUIMacros",
    "TipKitMacros",
];

/// `sweetpad-macros`, built from `sweetpad-cli/macros`: see [`linux_helper`].
/// Without it a compile fails only where a macro of [`DARWIN_MACRO_MODULES`]
/// is used, with the compiler naming the module.
#[must_use]
pub fn darwin_macro_plugin() -> Option<PathBuf> {
    linux_helper("sweetpad-macros", "SWEETPAD_MACROS")
}

/// A Swift helper the Linux package ships, like `swbuild` or
/// `sweetpad-macros`: the path in the `override_var` environment variable,
/// else the one installed with the running binary (see
/// [`packaged_helper_beside`]), else the one on `PATH`.
#[must_use]
pub fn linux_helper(name: &str, override_var: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(override_var) {
        return Some(PathBuf::from(path));
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| packaged_helper_beside(&exe, name))
        .or_else(|| {
            std::env::var_os("PATH")
                .iter()
                .flat_map(std::env::split_paths)
                .map(|dir| dir.join(name))
                .find(|candidate| candidate.is_file())
        })
}

/// `name` in the `libexec/sweetpad` folder installed with the binary at
/// `exe`: beside it, as the release archive unpacks, or in its parent, as
/// `/usr/bin/sweetpad` and `/usr/libexec/sweetpad` from the Debian package.
/// A link to the binary is followed to where the package put it.
#[must_use]
pub fn packaged_helper_beside(exe: &Path, name: &str) -> Option<PathBuf> {
    let exe = exe.canonicalize().ok()?;
    let dir = exe.parent()?;
    [
        dir.join("libexec/sweetpad"),
        dir.parent()?.join("libexec/sweetpad"),
    ]
    .into_iter()
    .map(|folder| folder.join(name))
    .find(|candidate| candidate.is_file())
}

/// The `swiftc` flags a build on the Darwin SDK adds to the SDK's own, in
/// the `-Xfrontend` form an editor's arguments take: cross-import overlays,
/// which Xcode's compiler loads by default and the open-source one only when
/// asked, and the plugin for [`DARWIN_MACRO_MODULES`] when it is installed.
#[must_use]
pub fn darwin_sdk_swift_flags() -> Vec<String> {
    let mut flags = vec![
        "-Xfrontend".to_string(),
        "-enable-cross-import-overlays".to_string(),
    ];
    if let Some(plugin) = darwin_macro_plugin() {
        flags.extend([
            "-Xfrontend".to_string(),
            "-load-plugin-executable".to_string(),
            "-Xfrontend".to_string(),
            format!("{}#{}", plugin.display(), DARWIN_MACRO_MODULES.join(",")),
        ]);
    }
    flags
}

/// The Darwin SDK's `Developer` directory, laid out as Xcode's is, off
/// macOS when the SDK is installed. Its `version.plist` sits beside it, where
/// Xcode's sits beside `Contents/Developer`.
#[must_use]
pub fn darwin_sdk_developer_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return None;
    }
    let developer = darwin_sdk_bundle().join("Developer");
    developer.join("Platforms").is_dir().then_some(developer)
}

static SELECTED_CACHE: Mutex<Option<PathBuf>> = Mutex::new(None);

fn selected_cache() -> MutexGuard<'static, Option<PathBuf>> {
    SELECTED_CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Forget the session-memoized Xcode state: the `xcode-select -p` result and
/// every cached [`XcodeLayout`] (`version.plist` reads). The next detection
/// re-observes the host, so a long-lived process (the node addon inside the
/// VS Code extension) can pick up `xcode-select -s` switches or an in-place
/// Xcode update without restarting.
pub fn flush_caches() {
    *selected_cache() = None;
    layout_cache().clear();
}

/// Spec + SDK roots discovered inside one Xcode install, so a `build-settings`
/// run can resolve against a *specific* Xcode (via `--xcode`) instead of the
/// catalog baked into the binary. [`locate`] spells every root through the
/// install's canonical path, whichever spelling it was given.
#[derive(Debug, Clone)]
pub struct XcodeLayout {
    /// `…/Xcode.app/Contents/Developer` — feeds `DEVELOPER_DIR`.
    pub developer_dir: PathBuf,
    /// `…/Contents/SharedFrameworks` — recursively walked for `*.xcspec`.
    pub xcspec_root: PathBuf,
    /// `…/Contents/Developer/Platforms` — walked for the `SDKSettings.plist`
    /// of each `*.sdk` directory, without descending into the SDKs.
    pub sdksettings_root: PathBuf,
    /// `CFBundleShortVersionString` (e.g. `26.5`); empty if unreadable.
    pub short_version: String,
    /// `ProductBuildVersion` (e.g. `17F6`); empty if unreadable.
    pub build_version: String,
}

impl XcodeLayout {
    /// A stable identity for cache validation: the build + short version and
    /// the install path. Cheaper and more robust than stat-ing every spec —
    /// the specs are a pure function of which Xcode this is. The path is the
    /// canonical one the cache file is named from, so two spellings of one
    /// Xcode share a cache file and agree on what it holds.
    #[must_use]
    pub fn cache_key(&self) -> String {
        format!(
            "{}|{}|{}",
            self.build_version,
            self.short_version,
            self.developer_dir.display()
        )
    }
}

/// Process-global cache of resolved [`XcodeLayout`]s, keyed by the input path,
/// so `version.plist` is read once per Xcode. Only successful resolves are
/// cached; see the module note on staleness.
static LAYOUT_CACHE: LazyLock<Mutex<HashMap<PathBuf, XcodeLayout>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn layout_cache() -> MutexGuard<'static, HashMap<PathBuf, XcodeLayout>> {
    LAYOUT_CACHE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Resolve an `--xcode` argument into the directories the catalog loader needs.
///
/// Accepts an `Xcode.app`, its `Contents`, or a `Contents/Developer`
/// (`DEVELOPER_DIR`) — the `Contents` dir holding both `SharedFrameworks` (where
/// the xcspecs live) and `Developer` is the anchor we search for. The layout
/// is spelled through that dir's canonical path.
///
/// Cached: a second call for the same path returns the stored layout without
/// re-reading `version.plist`.
pub fn locate(xcode_path: &Path) -> Result<XcodeLayout, String> {
    if let Some(layout) = layout_cache().get(xcode_path) {
        return Ok(layout.clone());
    }
    let layout = locate_uncached(xcode_path)?;
    layout_cache().insert(xcode_path.to_path_buf(), layout.clone());
    Ok(layout)
}

fn locate_uncached(xcode_path: &Path) -> Result<XcodeLayout, String> {
    let contents = [
        xcode_path.to_path_buf(),
        xcode_path.join("Contents"),
        xcode_path.parent().map(Path::to_path_buf).unwrap_or_default(),
    ]
    .into_iter()
    .find(|c| c.join("SharedFrameworks").is_dir() && c.join("Developer").is_dir())
    .ok_or_else(|| {
        format!(
            "{} is not an Xcode install (no Contents/SharedFrameworks alongside Contents/Developer)",
            xcode_path.display()
        )
    })?;
    // Every spelling of one install (an `Xcode-27.0.0.app` symlink to
    // `Xcode.app`, say) resolves to the same roots: xcodebuild reports
    // DEVELOPER_DIR and SDKROOT through the resolved path too, and the catalog
    // cache names its file from these roots and validates it by `cache_key`.
    let contents = std::fs::canonicalize(&contents).unwrap_or(contents);

    let developer_dir = contents.join("Developer");
    let (short_version, build_version) = read_version_plist(&developer_dir);
    Ok(XcodeLayout {
        xcspec_root: contents.join("SharedFrameworks"),
        sdksettings_root: developer_dir.join("Platforms"),
        developer_dir,
        short_version,
        build_version,
    })
}

fn read_version_plist(developer_dir: &Path) -> (String, String) {
    let plist_path = developer_dir.parent().map(|p| p.join("version.plist"));
    let Some(path) = plist_path else {
        return (String::new(), String::new());
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return (String::new(), String::new());
    };
    (
        extract_plist_string(&contents, "CFBundleShortVersionString").unwrap_or_default(),
        extract_plist_string(&contents, "ProductBuildVersion").unwrap_or_default(),
    )
}

/// Cheap XML scrape — version.plist always emits `<key>K</key><string>V</string>`
/// once per key in our captured corpus.
fn extract_plist_string(xml: &str, key: &str) -> Option<String> {
    let needle = format!("<key>{key}</key>");
    let start = xml.find(&needle)?;
    let after = &xml[start + needle.len()..];
    let open = after.find("<string>")?;
    let close = after.find("</string>")?;
    if close <= open + "<string>".len() {
        return None;
    }
    Some(after[open + "<string>".len()..close].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;

    #[test]
    fn major_version_parses_from_short_version() {
        let i = ActiveInstall {
            developer_dir: PathBuf::from("/tmp"),
            short_version: "26.0.1".into(),
            build_version: "17A400".into(),
        };
        assert_eq!(i.major_version(), 26);
        assert_eq!(i.product_build_version(), "26.0.1-17A400");
    }

    /// The archive keeps helpers in `libexec/sweetpad` beside the binary, the
    /// Debian package in `/usr/libexec/sweetpad` beside `/usr/bin`, and a
    /// link on `PATH` resolves to whichever the package installed.
    #[cfg(unix)]
    #[test]
    fn packaged_helpers_are_found_beside_the_binary_and_through_links() {
        let root = TempDir::new("sweetpad-packaged-helper");
        let helper = |folder: &Path| {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join("swbuild"), "").unwrap();
            folder.join("swbuild").canonicalize().unwrap()
        };
        let archive = root.join("archive");
        let in_archive = helper(&archive.join("libexec/sweetpad"));
        std::fs::write(archive.join("sweetpad"), "").unwrap();
        assert_eq!(
            packaged_helper_beside(&archive.join("sweetpad"), "swbuild"),
            Some(in_archive.clone())
        );
        let usr = root.join("usr");
        let in_usr = helper(&usr.join("libexec/sweetpad"));
        std::fs::create_dir_all(usr.join("bin")).unwrap();
        std::fs::write(usr.join("bin/sweetpad"), "").unwrap();
        assert_eq!(
            packaged_helper_beside(&usr.join("bin/sweetpad"), "swbuild"),
            Some(in_usr)
        );
        std::fs::create_dir_all(root.join("home/bin")).unwrap();
        std::os::unix::fs::symlink(archive.join("sweetpad"), root.join("home/bin/sweetpad"))
            .unwrap();
        assert_eq!(
            packaged_helper_beside(&root.join("home/bin/sweetpad"), "swbuild"),
            Some(in_archive)
        );
        assert_eq!(
            packaged_helper_beside(&archive.join("sweetpad"), "sweetpad-macros"),
            None
        );
    }

    #[test]
    fn product_build_version_falls_back_to_unknown() {
        let i = ActiveInstall {
            developer_dir: PathBuf::from("/tmp"),
            short_version: String::new(),
            build_version: String::new(),
        };
        assert_eq!(i.product_build_version(), "Unknown");
        assert_eq!(i.major_version(), 0);
    }

    #[test]
    fn extracts_plist_string_value() {
        let xml = r"<plist>
            <dict>
              <key>CFBundleShortVersionString</key>
              <string>26.0.1</string>
              <key>ProductBuildVersion</key>
              <string>17A400</string>
            </dict>
          </plist>";
        assert_eq!(
            extract_plist_string(xml, "CFBundleShortVersionString").as_deref(),
            Some("26.0.1"),
        );
        assert_eq!(
            extract_plist_string(xml, "ProductBuildVersion").as_deref(),
            Some("17A400"),
        );
        assert!(extract_plist_string(xml, "Missing").is_none());
    }

    #[test]
    fn locate_finds_roots_from_any_entry_point() {
        // Minimal Xcode.app skeleton in a temp dir.
        let root = TempDir::new("sweetpad-xcode");
        let app = root.join("Xcode.app");
        let contents = app.join("Contents");
        std::fs::create_dir_all(contents.join("SharedFrameworks")).unwrap();
        std::fs::create_dir_all(contents.join("Developer/Platforms")).unwrap();
        std::fs::write(
            contents.join("version.plist"),
            "<plist><dict><key>CFBundleShortVersionString</key><string>26.5</string>\
             <key>ProductBuildVersion</key><string>17F6</string></dict></plist>",
        )
        .unwrap();
        // The temp dir itself may sit behind a symlink (`/var` on macOS).
        let real = std::fs::canonicalize(&contents).unwrap();

        for entry in [&app, &contents, &contents.join("Developer")] {
            let layout =
                locate(entry).unwrap_or_else(|e| panic!("locate {}: {e}", entry.display()));
            assert_eq!(layout.developer_dir, real.join("Developer"));
            assert_eq!(layout.xcspec_root, real.join("SharedFrameworks"));
            assert_eq!(layout.sdksettings_root, real.join("Developer/Platforms"));
            assert_eq!(layout.short_version, "26.5");
            assert_eq!(layout.build_version, "17F6");
        }

        assert!(
            locate(&root).is_err(),
            "bare dir without Contents should fail"
        );
    }

    #[test]
    fn a_symlinked_spelling_locates_the_same_install() {
        // `/Applications/Xcode-27.0.0.app` pointing at `Xcode.app`: one
        // install, so one layout and one catalog cache key for both.
        let root = TempDir::new("sweetpad-xcode-link");
        let contents = root.join("Xcode.app/Contents");
        std::fs::create_dir_all(contents.join("SharedFrameworks")).unwrap();
        std::fs::create_dir_all(contents.join("Developer/Platforms")).unwrap();
        std::fs::write(
            contents.join("version.plist"),
            "<plist><dict><key>CFBundleShortVersionString</key><string>27.0</string>\
             <key>ProductBuildVersion</key><string>18A5</string></dict></plist>",
        )
        .unwrap();
        let link = root.join("Xcode-27.0.0.app");
        std::os::unix::fs::symlink(root.join("Xcode.app"), &link).unwrap();

        let real = locate(&contents.join("Developer")).unwrap();
        let linked = locate(&link.join("Contents/Developer")).unwrap();
        assert_eq!(
            linked.developer_dir,
            std::fs::canonicalize(contents.join("Developer")).unwrap()
        );
        assert_eq!(linked.developer_dir, real.developer_dir);
        assert_eq!(linked.xcspec_root, real.xcspec_root);
        assert_eq!(linked.sdksettings_root, real.sdksettings_root);
        assert_eq!(linked.cache_key(), real.cache_key());
    }

    /// The snapshot the extension reads for a symlinked install names the
    /// Developer directory the way the layout its build settings come from
    /// does, and reads that install's version.
    #[test]
    fn install_at_spells_a_symlinked_install_as_locate_does() {
        let root = TempDir::new("sweetpad-xcode-install");
        let contents = root.join("Xcode.app/Contents");
        std::fs::create_dir_all(contents.join("SharedFrameworks")).unwrap();
        std::fs::create_dir_all(contents.join("Developer/Platforms")).unwrap();
        std::fs::write(
            contents.join("version.plist"),
            "<plist><dict><key>CFBundleShortVersionString</key><string>27.0</string>\
             <key>ProductBuildVersion</key><string>18A5</string></dict></plist>",
        )
        .unwrap();
        let link = root.join("Xcode-27.0.0.app");
        std::os::unix::fs::symlink(root.join("Xcode.app"), &link).unwrap();

        let through_link = link.join("Contents/Developer");
        let install = install_at(&through_link);
        assert_eq!(
            install.developer_dir,
            locate(&through_link).unwrap().developer_dir
        );
        assert_eq!(
            install.developer_dir,
            std::fs::canonicalize(contents.join("Developer")).unwrap()
        );
        assert_eq!(install.short_version, "27.0");
        assert_eq!(install.build_version, "18A5");

        // A directory that isn't there keeps its spelling.
        let missing = root.join("Missing.app/Contents/Developer");
        assert_eq!(install_at(&missing).developer_dir, missing);
    }

    #[test]
    fn flush_caches_drops_memoized_layouts() {
        let root = TempDir::new("sweetpad-xcode-flush");
        let contents = root.join("Xcode.app/Contents");
        std::fs::create_dir_all(contents.join("SharedFrameworks")).unwrap();
        std::fs::create_dir_all(contents.join("Developer/Platforms")).unwrap();
        let plist = |ver: &str| {
            format!(
                "<plist><dict><key>CFBundleShortVersionString</key><string>{ver}</string>\
                 <key>ProductBuildVersion</key><string>17F6</string></dict></plist>"
            )
        };
        std::fs::write(contents.join("version.plist"), plist("26.5")).unwrap();

        let app = root.join("Xcode.app");
        assert_eq!(locate(&app).unwrap().short_version, "26.5");

        // An in-place update isn't observed while the layout is memoized…
        std::fs::write(contents.join("version.plist"), plist("27.0")).unwrap();
        assert_eq!(locate(&app).unwrap().short_version, "26.5");

        // …until the session caches are flushed.
        flush_caches();
        assert_eq!(locate(&app).unwrap().short_version, "27.0");
    }
}
