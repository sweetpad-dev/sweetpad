//! The in-app injection *client* dylib: locating (and, per CLI_DESIGN §9d,
//! eventually building + caching) the InjectionNext client, and assembling the
//! `SIMCTL_CHILD_*` environment that injects it into the launched simulator app.
//!
//! Resolution order: an explicit override, then the per-Xcode cached build, then
//! a fall back to an installed `InjectionNext.app` (the path proven by the
//! Milestone-1 spike). The cached build from vendored source — the long-term
//! distribution — is wired here behind [`build_and_cache`]; until the source is
//! vendored it returns a clear error and we use the fallback.

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

/// Resolve the client dylib to inject, building + caching it from vendored
/// source on a miss (or, until that's vendored, falling back to a local
/// `InjectionNext.app`).
pub fn resolve_dylib(opts: &ClientOptions) -> Result<PathBuf, String> {
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

    // 1. Per-Xcode cached build (the vendored-source distribution).
    if let Some(cached) = cached_dylib(name) {
        return Ok(cached);
    }
    match build_and_cache(opts, name) {
        Ok(p) => return Ok(p),
        Err(BuildError::NoVendoredSource) => {} // fall through to the app fallback
        Err(BuildError::Failed(e)) => return Err(e),
    }

    // 2. Fallback: an installed InjectionNext.app (Milestone-1's proven path).
    let app_dylib = Path::new(INJECTIONNEXT_APP)
        .join("Contents/Resources")
        .join(name);
    if app_dylib.exists() {
        return Ok(app_dylib);
    }

    Err(format!(
        "no injection client dylib found. Install InjectionNext.app (\
         https://github.com/johnno1962/InjectionNext) or set the hot-reload \
         dylib override. (Expected the vendored build cache or {})",
        app_dylib.display()
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

enum BuildError {
    /// The vendored InjectionNext source isn't present in this build — use the
    /// fallback. (Removed once the source is vendored under `vendor/`.)
    NoVendoredSource,
    Failed(String),
}

/// Where a built-per-Xcode client is cached: `~/.cache/sweetpad/hot-reload/<id>/`.
fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| crate::paths::home_dir().map(|h| h.join(".cache")))?;
    let id = xcode_build_id().unwrap_or_else(|| "unknown".into());
    Some(base.join("sweetpad").join("hot-reload").join(id))
}

fn cached_dylib(name: &str) -> Option<PathBuf> {
    let p = cache_dir()?.join(name);
    p.exists().then_some(p)
}

/// Build the client from vendored source with `xcodebuild` and cache it under
/// the active Xcode's build id (CLI_DESIGN §9d: builder = xcodebuild). Until the
/// source tree is vendored this is a no-op signalling the fallback.
fn build_and_cache(_opts: &ClientOptions, _name: &str) -> Result<PathBuf, BuildError> {
    let Some(source) = vendored_source() else {
        return Err(BuildError::NoVendoredSource);
    };
    // Implementation note (executed once the source is vendored): `xcodebuild`
    // the InjectionNext project for `-sdk <opts.sdk>`, then copy the produced
    // `iOSInjection.bundle`/dylib into `cache_dir()`. Errors map to
    // `BuildError::Failed`. Kept behind `vendored_source()` so the binary builds
    // and runs (via the fallback) before the vendoring lands.
    let _ = source;
    Err(BuildError::Failed(
        "vendored client build not yet implemented".into(),
    ))
}

/// The vendored InjectionNext source tree, if present in this build.
fn vendored_source() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/InjectionNext");
    p.is_dir().then_some(p)
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
}
