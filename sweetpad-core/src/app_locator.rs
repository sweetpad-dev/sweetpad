//! Finding the app a build produced: which of a scheme's targets is the one to
//! launch, and where its bundle is, from the build settings that build
//! resolves. The CLI's `app` verbs and the VS Code extension (through the
//! native addon) both locate the app here, so they launch the same bundle.

use std::path::{Path, PathBuf};

use sweetpad_lib::destination::RunDestination;
use sweetpad_lib::project::{absolutize, canonicalize_sdk_base, standardize};
use sweetpad_lib::scheme;

use crate::build_settings::{BuildSettingsOptions, TargetSettings, resolve_build_settings};
use crate::xcodebuild_args;

/// What a build's command line adds to the build settings `xcodebuild`
/// resolves, above every project layer: the `-derivedDataPath` that places
/// the build, the `-xcconfig` overlay, and the `KEY=VALUE` assignments. Each
/// caller that resolves settings for a build reads them from the same
/// arguments the build takes, so the locator, `settings show`, the hot-reload
/// recompiler and the index agree with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandLineSettings {
    pub derived_data_path: Option<PathBuf>,
    /// `xcodebuild` takes one `-xcconfig` and refuses a second, so this is
    /// the last one given.
    pub xcconfig: Option<PathBuf>,
    pub overrides: Vec<(String, String)>,
}

impl CommandLineSettings {
    /// Read them from `args`, the arguments a build passes `xcodebuild`. The
    /// paths of the two flags resolve against `working_dir` (see
    /// [`path_value`]). The settings stay as typed: the resolver reads a
    /// relative `SYMROOT=build` against each target's project directory, as
    /// `xcodebuild` does.
    #[must_use]
    pub fn of(args: &[String], working_dir: Option<&Path>) -> Self {
        Self {
            derived_data_path: path_value(args, "-derivedDataPath", working_dir),
            xcconfig: path_value(args, "-xcconfig", working_dir),
            overrides: xcodebuild_args::settings(args),
        }
    }
}

/// The value after the last `flag` in `args`, as a path `xcodebuild` running
/// in `working_dir` reads it (`None` is the current directory). `xcodebuild`
/// knows that directory by its physical path, so a relative path joins its
/// [`standardize`] spelling: for a project reached through a symlinked `link`,
/// `-derivedDataPath dd` is `real/dd` and `../dd` is the real directory's
/// sibling.
#[must_use]
pub fn path_value(args: &[String], flag: &str, working_dir: Option<&Path>) -> Option<PathBuf> {
    let path = PathBuf::from(xcodebuild_args::last_value(args, flag)?);
    if path.is_absolute() {
        return Some(path);
    }
    let base = working_dir.unwrap_or_else(|| Path::new("."));
    Some(absolutize(&standardize(base).join(path)))
}

/// The launchable product of a build: the `.app` path, its bundle id, and the
/// executable inside it (used to launch macOS apps directly).
#[derive(Debug, Clone)]
pub struct AppBundle {
    pub path: PathBuf,
    pub bundle_id: String,
    /// `TARGET_BUILD_DIR/EXECUTABLE_PATH`, the binary to run for a macOS app.
    pub executable: PathBuf,
}

/// What a located target builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductKind {
    /// A `.app` bundle with a bundle id: installed and launched on any
    /// platform it supports.
    App,
    /// A macOS command-line tool: a bare executable, run in place on the Mac.
    /// Its [`AppBundle`] names the executable as its path, and its bundle id
    /// is empty when the target sets none.
    Tool,
}

/// The product [`locate`] found: its bundle, what kind of product it is, and
/// the resolved settings of the target that builds it.
#[derive(Debug)]
pub struct Located {
    pub app: AppBundle,
    pub kind: ProductKind,
    pub settings: TargetSettings,
}

/// The settings [`pick`] reads. [`locate`] resolves them whatever keys its
/// caller asks for, and trims the result afterwards.
const LOCATOR_KEYS: [&str; 7] = [
    "WRAPPER_NAME",
    "FULL_PRODUCT_NAME",
    "TARGET_BUILD_DIR",
    "PRODUCT_BUNDLE_IDENTIFIER",
    "PRODUCT_TYPE",
    "EXECUTABLE_PATH",
    "SUPPORTED_PLATFORMS",
];

/// The error when no target builds anything to launch.
const NOTHING_TO_LAUNCH: &str = "could not find a launchable .app in the resolved build settings";

/// Resolve `opts` and pick the product a build with them writes (see
/// [`pick`]), from the scheme's Run action and the destination's platform.
/// `opts.keys` trims the returned target's settings, as it trims a
/// [`resolve_build_settings`] result.
///
/// # Errors
/// The resolver's error, or one saying no target builds a launchable `.app`.
pub fn locate(mut opts: BuildSettingsOptions) -> Result<Located, String> {
    let keys = opts.keys.take();
    if let Some(wanted) = &keys {
        let mut all = wanted.clone();
        all.extend(LOCATOR_KEYS.iter().map(|k| (*k).to_string()));
        opts.keys = Some(all);
    }
    let settings = resolve_build_settings(&opts)?;
    let container = opts.workspace.as_deref().or(opts.project.as_deref());
    let launch = container
        .zip(opts.scheme.as_deref())
        .and_then(|(container, name)| launch_target(container, name));
    pick(
        settings,
        platform(opts.destination.as_ref(), &opts.sdk).as_deref(),
        launch.as_deref(),
        keys.as_deref(),
    )
}

/// Pick the product to launch out of a build's resolved `settings`, with the
/// picked target's settings trimmed to `keys` when given. [`locate`] resolves
/// the settings in-process; a caller that resolved them some other way
/// (`xcodebuild -showBuildSettings`) picks with this directly.
///
/// Candidates are targets that build a `.app` wrapper and declare a bundle id.
/// The target the scheme's Run action launches (`launch_target`) wins when it
/// is a candidate that can run on `platform`: a scheme that builds a helper
/// app, a second app or a share extension ahead of the one it runs launches
/// the one Xcode runs. Otherwise one whose `SUPPORTED_PLATFORMS` covers
/// `platform` (an SDK name, `iphonesimulator`) wins: in an iOS + watchOS
/// scheme the watch companion builds *first* (dependency order), and blind
/// first-pick would install the watch app onto the iPhone simulator. Targets
/// that don't state their platforms (or no `platform`) fall back to
/// first-candidate order, and when the filter rejects *every* candidate (Mac
/// Catalyst declaring `iphoneos` under a `platform=macOS` destination), the
/// first `.app` still wins over a nothing-to-launch error.
///
/// With no `.app` at all, on the Mac or with no `platform`, a command-line
/// tool is the product: the Run action's, else the first.
///
/// # Errors
/// When no target builds a launchable `.app` or, on the Mac, a tool.
pub fn pick(
    mut settings: Vec<TargetSettings>,
    platform: Option<&str>,
    launch_target: Option<&str>,
    keys: Option<&[String]>,
) -> Result<Located, String> {
    let (index, kind) = pick_index(&settings, platform, launch_target)?;
    let mut target = settings.swap_remove(index);
    let app = match kind {
        ProductKind::App => app_of(&target),
        ProductKind::Tool => tool_of(&target),
    }
    .expect("pick_index picks a target that builds its kind");
    if let Some(keys) = keys {
        target.settings.retain(|k, _| keys.iter().any(|w| w == k));
    }
    Ok(Located {
        app,
        kind,
        settings: target,
    })
}

/// The platform [`pick`] narrows to for a build: the destination's, else the
/// SDK's, as `SUPPORTED_PLATFORMS` spells it (`iphonesimulator`). `None` when
/// the build names neither.
#[must_use]
pub fn platform(destination: Option<&RunDestination>, sdk: &str) -> Option<String> {
    destination
        .map(|d| d.platform.clone())
        .or_else(|| (!sdk.is_empty()).then(|| canonicalize_sdk_base(sdk)))
}

/// [`pick`]'s choice, as an index into `settings` and what it builds.
fn pick_index(
    settings: &[TargetSettings],
    platform: Option<&str>,
    launch_target: Option<&str>,
) -> Result<(usize, ProductKind), String> {
    // Whether `t` runs on `platform`: `None` when there is no platform to
    // match or the target doesn't state its own.
    let runs_on = |t: &TargetSettings| {
        let platforms = t.settings.get("SUPPORTED_PLATFORMS")?;
        let wanted = platform?;
        Some(platforms.split_whitespace().any(|p| p == wanted))
    };
    let of_kind = |test: fn(&TargetSettings) -> Option<AppBundle>| {
        settings
            .iter()
            .enumerate()
            .filter(move |(_, t)| test(t).is_some())
    };
    let launched = |t: &TargetSettings| launch_target.is_some_and(|name| t.target == name);
    if let Some((i, _)) = of_kind(app_of).find(|(_, t)| launched(t) && runs_on(t) != Some(false)) {
        return Ok((i, ProductKind::App));
    }
    let mut fallback = None;
    let mut first_app = None;
    for (i, t) in of_kind(app_of) {
        first_app.get_or_insert(i);
        match runs_on(t) {
            // The target states its platforms and covers the destination: a
            // definitive pick.
            Some(true) => return Ok((i, ProductKind::App)),
            // States its platforms and the destination is not among them:
            // not this app (the watch-companion case).
            Some(false) => {}
            // No filter requested, or the target doesn't say: a candidate in
            // declaration order.
            None => {
                fallback.get_or_insert(i);
            }
        }
    }
    if let Some(i) = fallback.or(first_app) {
        return Ok((i, ProductKind::App));
    }
    if platform.is_none_or(|p| p == "macosx") {
        let tool = of_kind(tool_of)
            .find(|(_, t)| launched(t))
            .or_else(|| of_kind(tool_of).next());
        if let Some((i, _)) = tool {
            return Ok((i, ProductKind::Tool));
        }
    }
    Err(NOTHING_TO_LAUNCH.to_string())
}

/// The launchable bundle a target's resolved settings describe, if it builds
/// a `.app` wrapper with a bundle id.
fn app_of(t: &TargetSettings) -> Option<AppBundle> {
    let wrapper = t
        .settings
        .get("WRAPPER_NAME")
        .or_else(|| t.settings.get("FULL_PRODUCT_NAME"))?;
    let build_dir = t.settings.get("TARGET_BUILD_DIR")?;
    let bundle_id = t.settings.get("PRODUCT_BUNDLE_IDENTIFIER")?;
    if !Path::new(wrapper)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("app"))
    {
        return None;
    }
    let build_dir = Path::new(build_dir);
    let executable = t
        .settings
        .get("EXECUTABLE_PATH")
        .map_or_else(|| build_dir.join(wrapper), |rel| build_dir.join(rel));
    Some(AppBundle {
        path: build_dir.join(wrapper),
        bundle_id: bundle_id.clone(),
        executable,
    })
}

/// The executable a target's resolved settings describe, if it builds a
/// command-line tool (`com.apple.product-type.tool`), which runs in place.
fn tool_of(t: &TargetSettings) -> Option<AppBundle> {
    if t.settings.get("PRODUCT_TYPE").map(String::as_str) != Some("com.apple.product-type.tool") {
        return None;
    }
    let build_dir = Path::new(t.settings.get("TARGET_BUILD_DIR")?);
    let executable = build_dir.join(
        t.settings
            .get("EXECUTABLE_PATH")
            .or_else(|| t.settings.get("FULL_PRODUCT_NAME"))?,
    );
    Some(AppBundle {
        path: executable.clone(),
        bundle_id: t
            .settings
            .get("PRODUCT_BUNDLE_IDENTIFIER")
            .cloned()
            .unwrap_or_default(),
        executable,
    })
}

/// The target `name`'s Run action launches, from its scheme file. `None`
/// when there is no file to read (an autocreated scheme) or the Run action
/// launches nothing.
#[must_use]
pub fn launch_target(container: &Path, name: &str) -> Option<String> {
    find_scheme(container, name)?
        .launch_target
        .map(|b| b.blueprint_name)
}

/// The parsed scheme file behind `name`, or `None` when there is none to read:
/// an autocreated scheme Xcode never materialized, or a name that doesn't
/// resolve. The file is the one `xcodebuild -list` reads for the container
/// ([`scheme::locate`]): the workspace's own, then its member projects', then
/// its local packages', and only the current user's `xcuserdata`.
#[must_use]
pub fn find_scheme(container: &Path, name: &str) -> Option<scheme::Scheme> {
    let file = scheme::locate(container, name)?;
    scheme::parse_file(&file).ok()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// One target's settings from `KEY=VALUE` pairs.
    fn target(name: &str, pairs: &[(&str, &str)]) -> TargetSettings {
        TargetSettings {
            target: name.to_string(),
            settings: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    /// [`pick`]'s bundle out of `settings`.
    fn app_bundle(
        settings: &[TargetSettings],
        platform: Option<&str>,
        launch_target: Option<&str>,
    ) -> Result<AppBundle, String> {
        pick(settings.to_vec(), platform, launch_target, None).map(|located| located.app)
    }

    fn app(name: &str, id: &str, platforms: Option<&str>) -> TargetSettings {
        let wrapper = format!("{name}.app");
        let mut pairs = vec![
            ("TARGET_BUILD_DIR", "/d"),
            ("WRAPPER_NAME", wrapper.as_str()),
            ("PRODUCT_BUNDLE_IDENTIFIER", id),
        ];
        if let Some(p) = platforms {
            pairs.push(("SUPPORTED_PLATFORMS", p));
        }
        target(name, &pairs)
    }

    #[test]
    fn app_bundle_picks_the_app_target() {
        let settings = [
            target(
                "AppTests",
                &[
                    ("TARGET_BUILD_DIR", "/d"),
                    ("WRAPPER_NAME", "AppTests.xctest"),
                    ("PRODUCT_BUNDLE_IDENTIFIER", "com.x.tests"),
                ],
            ),
            app("App", "com.x.app", None),
        ];
        let app = app_bundle(&settings, None, None).unwrap();
        assert_eq!(app.path, PathBuf::from("/d/App.app"));
        assert_eq!(app.bundle_id, "com.x.app");
    }

    #[test]
    fn app_bundle_resolves_macos_executable() {
        let settings = [target(
            "App",
            &[
                ("TARGET_BUILD_DIR", "/d"),
                ("WRAPPER_NAME", "App.app"),
                ("EXECUTABLE_PATH", "App.app/Contents/MacOS/App"),
                ("PRODUCT_BUNDLE_IDENTIFIER", "com.x.app"),
            ],
        )];
        let app = app_bundle(&settings, None, None).unwrap();
        assert_eq!(
            app.executable,
            PathBuf::from("/d/App.app/Contents/MacOS/App")
        );
    }

    #[test]
    fn app_bundle_errors_without_app() {
        let settings = [target(
            "Lib",
            &[
                ("TARGET_BUILD_DIR", "/d"),
                ("WRAPPER_NAME", "Lib.framework"),
                ("PRODUCT_BUNDLE_IDENTIFIER", "com.x.lib"),
            ],
        )];
        assert!(app_bundle(&settings, None, None).is_err());
    }

    #[test]
    fn app_bundle_prefers_the_destination_platform() {
        // Dependency order builds the watch companion first; the destination
        // platform must pick the iOS app anyway.
        let settings = [
            app("WatchApp", "com.x.watch", Some("watchos watchsimulator")),
            app("App", "com.x.app", Some("iphoneos iphonesimulator")),
        ];
        let ios = app_bundle(&settings, Some("iphonesimulator"), None).unwrap();
        assert_eq!(ios.bundle_id, "com.x.app");
        let watch = app_bundle(&settings, Some("watchsimulator"), None).unwrap();
        assert_eq!(watch.bundle_id, "com.x.watch");
        // No platform (or targets without SUPPORTED_PLATFORMS) keeps the
        // declaration-order pick.
        let first = app_bundle(&settings, None, None).unwrap();
        assert_eq!(first.bundle_id, "com.x.watch");
    }

    /// Xcode launches the Run action's target, which a scheme can list after
    /// another app it builds.
    #[test]
    fn the_scheme_launch_target_wins() {
        let settings = [
            app("Helper", "com.x.helper", Some("macosx")),
            app("App", "com.x.app", Some("macosx")),
        ];
        let launched = app_bundle(&settings, Some("macosx"), Some("App")).unwrap();
        assert_eq!(launched.bundle_id, "com.x.app");
        let launched = app_bundle(&settings, None, Some("App")).unwrap();
        assert_eq!(launched.bundle_id, "com.x.app");
        // A launch target that builds no app, or none the settings name, leaves
        // the pick to the other rules.
        assert_eq!(
            app_bundle(&settings, None, Some("Missing"))
                .unwrap()
                .bundle_id,
            "com.x.helper"
        );
    }

    /// A watch scheme launches the watch app; run on an iPhone simulator, the
    /// app that runs there wins.
    #[test]
    fn a_launch_target_for_another_platform_gives_way() {
        let settings = [
            app("App", "com.x.app", Some("iphoneos iphonesimulator")),
            app("WatchApp", "com.x.watch", Some("watchos watchsimulator")),
        ];
        let on_phone = app_bundle(&settings, Some("iphonesimulator"), Some("WatchApp")).unwrap();
        assert_eq!(on_phone.bundle_id, "com.x.app");
        let on_watch = app_bundle(&settings, Some("watchsimulator"), Some("WatchApp")).unwrap();
        assert_eq!(on_watch.bundle_id, "com.x.watch");
    }

    fn tool(name: &str) -> TargetSettings {
        target(
            name,
            &[
                ("TARGET_BUILD_DIR", "/d"),
                ("FULL_PRODUCT_NAME", name),
                ("EXECUTABLE_PATH", name),
                ("PRODUCT_TYPE", "com.apple.product-type.tool"),
            ],
        )
    }

    /// A scheme that builds a command-line tool and no app runs the tool on the
    /// Mac, and has nothing to launch anywhere else.
    #[test]
    fn a_tool_runs_on_the_mac_when_there_is_no_app() {
        let settings = [tool("Gen"), tool("Cli")];
        let located = pick(settings.to_vec(), Some("macosx"), Some("Cli"), None).unwrap();
        assert_eq!(located.kind, ProductKind::Tool);
        assert_eq!(located.app.executable, PathBuf::from("/d/Cli"));
        assert_eq!(located.app.bundle_id, "");
        assert_eq!(
            pick(settings.to_vec(), None, None, None).unwrap().app.path,
            PathBuf::from("/d/Gen")
        );
        assert!(pick(settings.to_vec(), Some("iphonesimulator"), None, None).is_err());
        // An app wins over a tool.
        let mixed = [tool("Gen"), app("App", "com.x.app", Some("macosx"))];
        let located = pick(mixed.to_vec(), Some("macosx"), Some("Gen"), None).unwrap();
        assert_eq!(located.kind, ProductKind::App);
        assert_eq!(located.app.bundle_id, "com.x.app");
    }

    #[test]
    fn command_line_settings_resolve_paths_where_xcodebuild_runs() {
        let args: Vec<String> = [
            "-derivedDataPath",
            "dd",
            "-xcconfig",
            "/abs/Over.xcconfig",
            "PRODUCT_NAME=Renamed",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let read = CommandLineSettings::of(&args, Some(Path::new("/nonexistent/work")));
        assert_eq!(
            read,
            CommandLineSettings {
                derived_data_path: Some(PathBuf::from("/nonexistent/work/dd")),
                xcconfig: Some(PathBuf::from("/abs/Over.xcconfig")),
                overrides: vec![("PRODUCT_NAME".into(), "Renamed".into())],
            }
        );
        assert_eq!(
            CommandLineSettings::of(&[], None),
            CommandLineSettings::default()
        );
    }

    /// The fixture app's project copied into `dir`, with a scheme `Both` that
    /// builds the macOS app ahead of the iOS app and launches the iOS app.
    fn two_app_project(dir: &Path) -> PathBuf {
        let source = PathBuf::from(env!("SWEETPAD_LIB_DIR"))
            .join("fixtures/_synthetic-objectversion-110/project/SweetpadCIApp.xcodeproj");
        let project = dir.join("SweetpadCIApp.xcodeproj");
        let schemes = project.join("xcshareddata/xcschemes");
        std::fs::create_dir_all(&schemes).unwrap();
        std::fs::copy(
            source.join("project.pbxproj"),
            project.join("project.pbxproj"),
        )
        .unwrap();
        let reference = |name: &str| {
            format!(
                r#"<BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="ID{name}" BuildableName="{name}.app" BlueprintName="{name}" ReferencedContainer="container:SweetpadCIApp.xcodeproj"/>"#
            )
        };
        let entry = |name: &str| {
            format!(
                r#"<BuildActionEntry buildForRunning="YES" buildForTesting="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES">{}</BuildActionEntry>"#,
                reference(name)
            )
        };
        std::fs::write(
            schemes.join("Both.xcscheme"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="1600" version="1.7">
<BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES">
<BuildActionEntries>{}{}</BuildActionEntries>
</BuildAction>
<LaunchAction buildConfiguration="Debug"><BuildableProductRunnable runnableDebuggingMode="0">{}</BuildableProductRunnable></LaunchAction>
</Scheme>"#,
                entry("SweetpadCIMac"),
                entry("SweetpadCIApp"),
                reference("SweetpadCIApp")
            ),
        )
        .unwrap();
        project
    }

    /// Resolved for real: the Run action's app wins over the app the scheme
    /// builds first.
    #[test]
    fn locate_launches_the_run_actions_app() {
        let dir = crate::scratch::ScratchDir::new("sweetpad-locate-two-apps").unwrap();
        let project = two_app_project(&dir);
        let located = |destination: Option<&str>| {
            locate(BuildSettingsOptions {
                project: Some(project.clone()),
                scheme: Some("Both".into()),
                configuration: "Debug".into(),
                destination: destination.and_then(sweetpad_lib::destination::parse_destination_arg),
                // The catalog parsed out of the active Xcode, cached in the
                // test's own directory rather than the user's.
                catalog_cache: Some(dir.join("catalog.bin")),
                keys: Some(vec!["PRODUCT_NAME".into()]),
                ..BuildSettingsOptions::default()
            })
            .unwrap()
        };
        let plain = located(None);
        assert_eq!(plain.app.bundle_id, "dev.sweetpad.ci.app");
        assert_eq!(plain.settings.target, "SweetpadCIApp");
        // The caller's keys, whatever the locator read.
        assert_eq!(
            plain.settings.settings.keys().collect::<Vec<_>>(),
            ["PRODUCT_NAME"]
        );
        let simulator = located(Some("platform=iOS Simulator,id=U"));
        assert_eq!(simulator.app.bundle_id, "dev.sweetpad.ci.app");
        assert!(
            simulator
                .app
                .path
                .ends_with("Debug-iphonesimulator/SweetpadCIApp.app"),
            "{:?}",
            simulator.app.path
        );
    }

    /// The Run action is read from the file `xcodebuild -list` reads: a
    /// workspace member package's `.swiftpm/xcode` scheme counts, a scheme in a
    /// project the workspace doesn't list doesn't.
    #[test]
    fn the_launch_target_comes_from_the_file_xcodebuild_reads() {
        let dir = crate::scratch::ScratchDir::new("sweetpad-locate-find-scheme").unwrap();
        let ws = dir.join("App.xcworkspace");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(
            ws.join("contents.xcworkspacedata"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Workspace version = \"1.0\">\n   \
             <FileRef location = \"group:Pkg\"></FileRef>\n</Workspace>\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("Pkg")).unwrap();
        std::fs::write(dir.join("Pkg/Package.swift"), "").unwrap();
        let scheme = |launches: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme version="1.7">
<LaunchAction><BuildableProductRunnable><BuildableReference BuildableIdentifier="primary"
 BlueprintIdentifier="{launches}" BuildableName="{launches}" BlueprintName="{launches}"
 ReferencedContainer="container:"></BuildableReference></BuildableProductRunnable></LaunchAction>
</Scheme>"#
            )
        };
        for (at, launches) in [
            (
                "Pkg/.swiftpm/xcode/xcshareddata/xcschemes/Tool.xcscheme",
                "runner",
            ),
            (
                "Stray.xcodeproj/xcshareddata/xcschemes/Stray.xcscheme",
                "stray",
            ),
        ] {
            let path = dir.join(at);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, scheme(launches)).unwrap();
        }
        assert_eq!(launch_target(&ws, "Tool").as_deref(), Some("runner"));
        assert_eq!(launch_target(&ws, "Stray"), None);
    }
}
