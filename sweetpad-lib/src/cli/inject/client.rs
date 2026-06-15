//! The in-app injection *client* dylib: locating (and, per CLI_DESIGN §9d,
//! eventually building + caching) the InjectionNext client, and assembling the
//! `SIMCTL_CHILD_*` environment that injects it into the launched simulator app.
//!
//! Resolution order: an explicit override, then the per-Xcode cached build, then
//! a fall back to an installed `InjectionNext.app` (the path proven by the
//! Milestone-1 spike). The cached build ([`build_and_cache`]) clones + builds
//! the pinned InjectionNext from source against the active Xcode and caches the
//! resulting `.app` per Xcode build id — the long-term distribution.

use std::path::{Path, PathBuf};

use crate::cli::process;

const INJECTIONNEXT_APP: &str = "/Applications/InjectionNext.app";

/// Map a simulator SDK to the InjectionNext dylib that injects into it. Returns
/// `None` for SDKs InjectionNext can't inject (devices strip
/// `DYLD_INSERT_LIBRARIES`; watchOS ships no dylib).
#[must_use]
pub fn dylib_name_for(sdk: &str) -> Option<&'static str> {
    match sdk {
        "iphonesimulator" => Some("libiphonesimulatorInjection.dylib"),
        "appletvsimulator" => Some("libappletvsimulatorInjection.dylib"),
        "xrsimulator" => Some("libxrsimulatorInjection.dylib"),
        "macosx" => Some("libmacosxInjection.dylib"),
        _ => None,
    }
}

/// The `<Platform>.platform` directory name for an SDK, used to find XCTest.
#[must_use]
pub fn platform_dir_for(sdk: &str) -> Option<&'static str> {
    match sdk {
        "iphonesimulator" => Some("iPhoneSimulator"),
        "appletvsimulator" => Some("AppleTVSimulator"),
        "xrsimulator" => Some("XRSimulator"),
        "macosx" => Some("MacOSX"),
        _ => None,
    }
}

/// Inputs for resolving/injecting the client.
pub struct ClientOptions {
    /// Active Xcode `Contents/Developer`.
    pub developer_dir: String,
    /// Simulator SDK short name (e.g. `iphonesimulator`).
    pub sdk: String,
    /// Workspace root, exported as `INJECTION_PROJECT_ROOT`.
    pub project_root: PathBuf,
    /// Explicit dylib override (skips cache + fallback).
    pub override_path: Option<PathBuf>,
}

/// Resolve the client dylib to inject: an explicit override, else a per-Xcode
/// cached build, else build it from source and cache it, else an installed
/// `InjectionNext.app`. `notify` reports slow steps (e.g. the first-run build).
pub fn resolve_dylib(opts: &ClientOptions, notify: &dyn Fn(&str)) -> Result<PathBuf, String> {
    if let Some(p) = &opts.override_path {
        if p.exists() {
            return Ok(p.clone());
        }
        return Err(format!(
            "hot-reload dylib override does not exist: {}",
            p.display()
        ));
    }

    let name = dylib_name_for(&opts.sdk)
        .ok_or_else(|| format!("hot reload is not supported for the {} SDK", opts.sdk))?;

    // The per-Xcode cache dir, probing the active Xcode build id exactly once. If
    // the build id can't be determined we deliberately get `None`: caching under
    // a non-discriminating id would let two Xcodes share one client and silently
    // reintroduce the ABI skew the from-source build exists to eliminate.
    let cache = xcode_build_id().and_then(|id| cache_root().map(|r| r.join(id)));

    // 1. Per-Xcode cached build (built from source against the active Xcode).
    if let Some(cached) = cache.as_deref().and_then(|dir| cached_dylib(dir, name)) {
        return Ok(cached);
    }
    // 2. Build it from source now and cache it (CLI_DESIGN §9d, Milestone 5).
    //    Opt out with SWEETPAD_HOTRELOAD_NO_BUILD to force the fallback.
    let mut build_err = None;
    if std::env::var_os("SWEETPAD_HOTRELOAD_NO_BUILD").is_none() {
        match &cache {
            Some(dir) => {
                notify(
                    "hot reload: building the injection client from source against the active \
                     Xcode (first run only; this can take a few minutes)…",
                );
                match build_and_cache(dir, name) {
                    Ok(p) => return Ok(p),
                    Err(e) => build_err = Some(e),
                }
            }
            None => build_err = Some("could not determine the active Xcode build id".into()),
        }
    }

    // 3. Fallback: an installed InjectionNext.app (Milestone-1's proven path).
    let app_dylib = Path::new(INJECTIONNEXT_APP)
        .join("Contents/Resources")
        .join(name);
    if app_dylib.exists() {
        return Ok(app_dylib);
    }

    Err(format!(
        "no injection client dylib available.{} Install InjectionNext.app \
         (https://github.com/johnno1962/InjectionNext), set SWEETPAD_HOTRELOAD_DYLIB, \
         or ensure git + Xcode can build the client.",
        build_err
            .map(|e| format!(" Building from source failed: {e}."))
            .unwrap_or_default()
    ))
}

/// The `SIMCTL_CHILD_*` env that injects `dylib` into the launched app and
/// points its client at our server. `simctl` forwards these (prefix stripped)
/// into the child process.
#[must_use]
pub fn launch_env(dylib: &Path, opts: &ClientOptions) -> Vec<(String, String)> {
    let mut env = vec![
        (
            "SIMCTL_CHILD_DYLD_INSERT_LIBRARIES".into(),
            dylib.display().to_string(),
        ),
        ("SIMCTL_CHILD_INJECTION_HOST".into(), "127.0.0.1".into()),
        // Only ever talk to our server — never fall back to the in-app standalone
        // watcher (which would inject without us and mask failures).
        ("SIMCTL_CHILD_INJECTION_NOSTANDALONE".into(), "1".into()),
        (
            "SIMCTL_CHILD_INJECTION_PROJECT_ROOT".into(),
            opts.project_root.display().to_string(),
        ),
    ];
    // The injection dylib's XCTest deps resolve via the platform's search paths.
    if let Some((fw, lib)) = xctest_search_paths(&opts.developer_dir, &opts.sdk) {
        env.push(("SIMCTL_CHILD_DYLD_FRAMEWORK_PATH".into(), fw));
        env.push(("SIMCTL_CHILD_DYLD_LIBRARY_PATH".into(), lib));
    }
    env
}

/// The Platform-specific XCTest framework + library search paths.
fn xctest_search_paths(developer_dir: &str, sdk: &str) -> Option<(String, String)> {
    let platform = platform_dir_for(sdk)?;
    let dev = Path::new(developer_dir)
        .join("Platforms")
        .join(format!("{platform}.platform"))
        .join("Developer");
    let framework = format!(
        "{}:{}",
        dev.join("Library/Frameworks").display(),
        dev.join("Library/PrivateFrameworks").display()
    );
    let library = dev.join("usr/lib").display().to_string();
    Some((framework, library))
}

/// Pinned InjectionNext revision built from source (kept in lockstep with the
/// version the e2e validates; bump deliberately).
const INJECTIONNEXT_REV: &str = "2.0.1RC8";

/// Root of the per-Xcode client cache: `~/.cache/sweetpad/hot-reload/`. The
/// caller appends the Xcode build id.
fn cache_root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| crate::paths::home_dir().map(|h| h.join(".cache")))?;
    Some(base.join("sweetpad").join("hot-reload"))
}

/// File written *last* by [`build_and_cache`] holding the built revision; its
/// presence means the cached `.app` (bundle + companions) is complete, and its
/// contents let a revision bump invalidate the cache.
const CACHE_MARKER: &str = ".client-ready";

/// A cached client dylib, but only if the cache is *complete and current*: the
/// ready-marker must exist and name the pinned revision. This rejects a
/// half-written cache (interrupted `ditto`) or one left by an older revision —
/// both would otherwise load + connect but fail to inject — forcing a rebuild.
fn cached_dylib(cache: &Path, name: &str) -> Option<PathBuf> {
    let marker = std::fs::read_to_string(cache.join(CACHE_MARKER)).ok()?;
    if marker.trim() != INJECTIONNEXT_REV {
        return None;
    }
    let p = cached_app_dylib(cache, name);
    p.exists().then_some(p)
}

/// The injection dylib *inside* a cached `InjectionNext.app`. The client must be
/// loaded with its companion `*.bundle`/`Frameworks` next to it — see
/// [`build_and_cache`] — so we always point at the symlink within the bundle,
/// the same on-disk shape as an installed `InjectionNext.app`.
fn cached_app_dylib(cache: &Path, name: &str) -> PathBuf {
    cache
        .join("InjectionNext.app")
        .join("Contents/Resources")
        .join(name)
}

/// Build the InjectionNext client from source against the **active Xcode** and
/// cache it under the Xcode build id (CLI_DESIGN §9d, Milestone 5): clone the
/// pinned revision with submodules and `xcodebuild` the app, then cache the
/// whole built `InjectionNext.app`. Building against the user's own Xcode makes
/// the client's XCTest ABI match — no prebuilt-binary version skew.
///
/// We cache the *entire* `.app`, not just the dylib, on purpose:
/// `lib<sdk>Injection.dylib` is a symlink into a companion `*.bundle` whose
/// Swift/XCTest dependencies are resolved at load time via `@loader_path`
/// (`build_bundles.sh`). Copying the lone dereferenced Mach-O out and dropping
/// the bundle loads + connects but then fails to inject. Keeping the bundle
/// intact mirrors the proven installed-app and prebuilt-release layouts.
///
/// `cache` is the per-Xcode cache dir. The ready-marker is written *last*, so an
/// interrupted build never leaves a cache that [`cached_dylib`] would trust.
fn build_and_cache(cache: &Path, name: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(cache).map_err(|e| format!("create cache dir: {e}"))?;
    // Invalidate any prior marker up front: a crash mid-build must not leave the
    // (now stale) marker pointing at a half-rebuilt cache.
    let _ = std::fs::remove_file(cache.join(CACHE_MARKER));
    let work = cache.join("build");
    // Must be literally "InjectionNext": App/feedcommands uses a relative
    // `#import "../../../InjectionNext/..."` that assumes that dir name.
    let src = work.join("InjectionNext");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("create build dir: {e}"))?;

    run_quiet(
        "git",
        &[
            "clone",
            "--quiet",
            "--recurse-submodules",
            "--depth",
            "1",
            "--shallow-submodules",
            "--branch",
            INJECTIONNEXT_REV,
            "https://github.com/johnno1962/InjectionNext",
            &src.to_string_lossy(),
        ],
        None,
        "clone InjectionNext",
    )?;

    let app = src.join("App");
    run_quiet(
        "xcrun",
        &[
            "xcodebuild",
            "-project",
            "InjectionNext.xcodeproj",
            "-scheme",
            "InjectionNext",
            "-configuration",
            "Debug",
            "-destination",
            "platform=macOS",
            "-derivedDataPath",
            "build",
            "CODE_SIGNING_ALLOWED=NO",
            "CODE_SIGNING_REQUIRED=NO",
            "build",
            "-quiet",
        ],
        Some(&app),
        "build InjectionNext client",
    )?;

    // The product path is deterministic for this Debug/macOS build invocation.
    let built_app = app.join("build/Build/Products/Debug/InjectionNext.app");
    if !built_app.exists() {
        return Err(format!(
            "InjectionNext.app not found at {}",
            built_app.display()
        ));
    }
    // Cache the whole built .app (bundle + symlinks intact). `ditto` preserves
    // the symlinked dylib and its companion bundle faithfully.
    let dest_app = cache.join("InjectionNext.app");
    let _ = std::fs::remove_dir_all(&dest_app);
    run_quiet(
        "ditto",
        &[&built_app.to_string_lossy(), &dest_app.to_string_lossy()],
        None,
        "cache InjectionNext.app",
    )?;
    // Drop the multi-GB clone/derived-data; keep only the cached .app.
    let _ = std::fs::remove_dir_all(&work);

    let dylib = cached_app_dylib(cache, name);
    if !dylib.exists() {
        return Err(format!(
            "{name} missing from cached client at {}",
            dylib.display()
        ));
    }
    // Mark the cache complete *last* — this is what `cached_dylib` trusts.
    std::fs::write(cache.join(CACHE_MARKER), INJECTIONNEXT_REV)
        .map_err(|e| format!("write cache marker: {e}"))?;
    Ok(dylib)
}

/// Run `program args` (optional cwd) with stdout suppressed, mapping a missing
/// binary or non-zero exit to a `String` error. Delegates spawning to
/// [`crate::cli::process::run`] (shared command-not-found diagnostics); stdout
/// is suppressed so build chatter never pollutes the CLI's own output stream.
fn run_quiet(program: &str, args: &[&str], cwd: Option<&Path>, what: &str) -> Result<(), String> {
    match process::run(program, args, cwd, true) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!("{what}: {program} exited with a non-zero status")),
        Err(e) => Err(format!("{what}: {e}")),
    }
}

/// `Build version` from `xcodebuild -version`, the per-Xcode cache key.
fn xcode_build_id() -> Option<String> {
    let out = process::capture("xcrun", &["xcodebuild", "-version"], None).ok()?;
    out.lines()
        .find_map(|l| l.strip_prefix("Build version "))
        .map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dylib_and_platform_map_simulator_sdks() {
        assert_eq!(
            dylib_name_for("iphonesimulator"),
            Some("libiphonesimulatorInjection.dylib")
        );
        assert_eq!(platform_dir_for("iphonesimulator"), Some("iPhoneSimulator"));
        // Devices / unknown SDKs aren't injectable.
        assert_eq!(dylib_name_for("iphoneos"), None);
        assert_eq!(platform_dir_for("watchsimulator"), None);
    }

    #[test]
    fn launch_env_sets_dyld_and_injection_vars() {
        let opts = ClientOptions {
            developer_dir: "/Applications/Xcode.app/Contents/Developer".into(),
            sdk: "iphonesimulator".into(),
            project_root: PathBuf::from("/work/App"),
            override_path: None,
        };
        let env = launch_env(Path::new("/cache/lib.dylib"), &opts);
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(
            get("SIMCTL_CHILD_DYLD_INSERT_LIBRARIES").as_deref(),
            Some("/cache/lib.dylib")
        );
        assert_eq!(
            get("SIMCTL_CHILD_INJECTION_PROJECT_ROOT").as_deref(),
            Some("/work/App")
        );
        assert_eq!(
            get("SIMCTL_CHILD_INJECTION_NOSTANDALONE").as_deref(),
            Some("1")
        );
        // XCTest framework path points into the iPhoneSimulator platform.
        assert!(
            get("SIMCTL_CHILD_DYLD_FRAMEWORK_PATH")
                .unwrap()
                .contains("iPhoneSimulator.platform/Developer/Library/Frameworks")
        );
    }

    #[test]
    fn cached_dylib_requires_a_current_ready_marker() {
        let name = "libiphonesimulatorInjection.dylib";
        let cache = std::env::temp_dir().join(format!("sweetpad-hot-{}", std::process::id()));
        let resources = cache.join("InjectionNext.app/Contents/Resources");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(resources.join(name), b"dylib").unwrap();

        // Dylib present but no marker (e.g. interrupted ditto) → rebuild.
        assert!(cached_dylib(&cache, name).is_none());

        // Marker from a different revision → rebuild (no version skew).
        std::fs::write(cache.join(CACHE_MARKER), "0.0.0-old").unwrap();
        assert!(cached_dylib(&cache, name).is_none());

        // Complete + current cache → served.
        std::fs::write(cache.join(CACHE_MARKER), INJECTIONNEXT_REV).unwrap();
        assert_eq!(cached_dylib(&cache, name), Some(resources.join(name)));

        // Marker current but the dylib went missing → rebuild.
        std::fs::remove_file(resources.join(name)).unwrap();
        assert!(cached_dylib(&cache, name).is_none());

        let _ = std::fs::remove_dir_all(&cache);
    }
}
