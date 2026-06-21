//! The in-app injection *client* dylib: resolving the InjectionNext client and
//! assembling the `SIMCTL_CHILD_*` environment that injects it into the launched
//! simulator app.
//!
//! Resolution order: an explicit override ([`ClientOptions::override_path`] /
//! `SWEETPAD_HOTRELOAD_DYLIB`), then the client **bundled into this binary**
//! (built from the pinned InjectionNext SPM product at release time — see
//! `vendor/injection-client`; XCTest-free, so one prebuilt is portable across
//! Xcode versions, with no clone, no per-Xcode build, and no network), then a
//! fall back to an installed `InjectionNext.app`.

use std::path::{Path, PathBuf};

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
    /// Explicit dylib override (skips the bundled client + fallback).
    pub override_path: Option<PathBuf>,
}

/// Resolve the client dylib to inject: an explicit override, else the client
/// bundled into this binary (materialized to the cache), else an installed
/// `InjectionNext.app`. `notify` reports if it has to fall back.
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

    // The client bundled into this binary. It's built for the iOS simulator (what
    // `--hot` supports) and is XCTest-free, so it needs no clone, no per-Xcode
    // build, and works offline.
    if opts.sdk == "iphonesimulator" {
        match materialize_bundled_client() {
            Ok(p) => return Ok(p),
            Err(e) => notify(&format!(
                "hot reload: bundled client unavailable ({e}); falling back to InjectionNext.app"
            )),
        }
    }

    // Fallback: an installed InjectionNext.app (covers the other simulator SDKs,
    // and the rare case where the bundled client can't be written to the cache).
    let app_dylib = Path::new(INJECTIONNEXT_APP)
        .join("Contents/Resources")
        .join(name);
    if app_dylib.exists() {
        return Ok(app_dylib);
    }

    Err(format!(
        "no injection client available for the {} SDK. Install InjectionNext.app \
         (https://github.com/johnno1962/InjectionNext) or set SWEETPAD_HOTRELOAD_DYLIB.",
        opts.sdk
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
    // The InjectionNext.app fallback dylib links XCTest; point it at the
    // platform's search paths so its deps resolve. The bundled client is
    // XCTest-free and ignores these, so passing them unconditionally is harmless.
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

/// The injection client compiled into this binary. `build.rs` stages it into
/// `OUT_DIR` from `vendor/injection-client/prebuilt/` (produced by its
/// `build.sh`); it is empty when that prebuilt was absent at build time, in which
/// case hot reload falls back to `InjectionNext.app`. The client is XCTest-free,
/// so the single prebuilt is portable across Xcode versions.
static BUNDLED_CLIENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/injection-client.dylib"));

/// Materialize [`BUNDLED_CLIENT`] to a content-addressed path under the cache and
/// return it. A new sweetpad release (new bytes) lands in a fresh directory;
/// stale ones are simply ignored. Idempotent: an existing file of the right size
/// is reused without rewriting.
fn materialize_bundled_client() -> Result<PathBuf, String> {
    let root = cache_root().ok_or("could not resolve the cache directory")?;
    materialize_client(BUNDLED_CLIENT, &root)
}

/// Write `bytes` (the embedded client) to a content-addressed path under
/// `cache_root` and return it. Split from [`materialize_bundled_client`] so tests
/// can drive it with arbitrary bytes and a temp root. Idempotent: an existing
/// file of the right size is reused; different bytes hash to a fresh directory.
fn materialize_client(bytes: &[u8], cache_root: &Path) -> Result<PathBuf, String> {
    if bytes.is_empty() {
        return Err("no injection client is bundled in this build".into());
    }
    let dir = cache_root.join(fnv1a_hex(bytes));
    let dylib = dir.join("SweetpadInjectionClient.dylib");
    if std::fs::metadata(&dylib).map(|m| m.len()).ok() == Some(bytes.len() as u64) {
        return Ok(dylib);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("create client cache dir: {e}"))?;
    // Write to a temp path then rename, so a concurrent session never observes a
    // half-written dylib at the final path.
    let tmp = dir.join(".SweetpadInjectionClient.dylib.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("write injection client: {e}"))?;
    std::fs::rename(&tmp, &dylib).map_err(|e| format!("install injection client: {e}"))?;
    Ok(dylib)
}

/// Root of the client cache: `~/.cache/sweetpad/hot-reload/`. The caller appends
/// a content key for the bundled client.
fn cache_root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| crate::paths::home_dir().map(|h| h.join(".cache")))?;
    Some(base.join("sweetpad").join("hot-reload"))
}

/// FNV-1a (64-bit) of `bytes` as lowercase hex — a tiny, dependency-free,
/// deterministic content key for the cache directory.
#[must_use]
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
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
    fn fnv1a_is_deterministic_and_content_sensitive() {
        let key = fnv1a_hex(b"injection-client");
        assert_eq!(key, fnv1a_hex(b"injection-client"));
        assert_ne!(fnv1a_hex(b"abc"), fnv1a_hex(b"abd"));
        assert_eq!(fnv1a_hex(b"abc").len(), 16);
    }

    fn opts(sdk: &str, override_path: Option<PathBuf>) -> ClientOptions {
        ClientOptions {
            developer_dir: "/Applications/Xcode.app/Contents/Developer".into(),
            sdk: sdk.into(),
            project_root: PathBuf::from("/work/App"),
            override_path,
        }
    }

    #[test]
    fn resolve_override_returns_existing_path() {
        let tmp =
            std::env::temp_dir().join(format!("sweetpad-override-{}.dylib", std::process::id()));
        std::fs::write(&tmp, b"x").unwrap();
        let got =
            resolve_dylib(&opts("iphonesimulator", Some(tmp.clone())), &|_: &str| {}).unwrap();
        assert_eq!(got, tmp);
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn resolve_missing_override_errors() {
        let o = opts(
            "iphonesimulator",
            Some(PathBuf::from("/no/such/client.dylib")),
        );
        assert!(resolve_dylib(&o, &|_: &str| {}).is_err());
    }

    #[test]
    fn resolve_unsupported_sdk_errors() {
        // A device SDK isn't injectable; this errors before touching the bundled
        // client or the InjectionNext.app fallback, so it's deterministic.
        let err = resolve_dylib(&opts("iphoneos", None), &|_: &str| {}).unwrap_err();
        assert!(err.contains("not supported"), "{err}");
    }

    #[test]
    fn materialize_empty_client_errors() {
        let dir = std::env::temp_dir().join(format!("sweetpad-mat-empty-{}", std::process::id()));
        assert!(materialize_client(&[], &dir).is_err());
    }

    #[test]
    fn materialize_writes_then_reuses() {
        let root = std::env::temp_dir().join(format!("sweetpad-mat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bytes = b"fake-injection-client";

        let p1 = materialize_client(bytes, &root).unwrap();
        assert_eq!(std::fs::read(&p1).unwrap(), bytes);

        // Same bytes → same content-addressed path, reused.
        assert_eq!(materialize_client(bytes, &root).unwrap(), p1);
        // Different bytes → a different directory.
        let p3 = materialize_client(b"other-client-bytes", &root).unwrap();
        assert_ne!(p1.parent(), p3.parent());

        std::fs::remove_dir_all(&root).ok();
    }
}
