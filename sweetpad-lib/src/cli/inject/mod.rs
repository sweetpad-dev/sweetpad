//! Built-in hot reload for `app run --hot` (CLI_DESIGN §9d).
//!
//! The CLI acts as the InjectionNext *server*: it binds `:8887`, the in-app
//! client (injected via `DYLD_INSERT_LIBRARIES`) connects out, and on each Swift
//! save the [`watcher`] drives the [`server`] to recompile the file (via the
//! [`recompiler`], resolver-default / build-log-switchable) and `.load` it into
//! the running app. [`client`] resolves the client dylib + launch env.
//!
//! The socket protocol and recompile→load→inject chain were validated end-to-end
//! on a real simulator (Milestone 1; see `ci/hot-reload-spike`).

use std::path::Path;
use std::sync::Arc;

pub mod client;
pub mod protocol;
pub mod recompiler;
pub mod server;
pub mod watcher;

use server::InjectServer;
use watcher::Watcher;

/// A live hot-reload session: the running server plus the file watcher wired to
/// drive it. Dropping it (or [`HotSession::shutdown`]) stops both.
pub struct HotSession {
    server: Arc<InjectServer>,
    _watcher: Watcher,
}

impl HotSession {
    /// Wire `root`'s `.swift` saves to `server.inject`.
    #[must_use]
    pub fn start(server: Arc<InjectServer>, root: &Path) -> HotSession {
        let inject_server = Arc::clone(&server);
        let on_change: watcher::OnChange = Arc::new(move |path: &Path| {
            inject_server.inject(path);
        });
        let watcher = Watcher::start(root, on_change);
        HotSession {
            server,
            _watcher: watcher,
        }
    }

    /// Stop the watcher and tear down the server connection.
    pub fn shutdown(self) {
        self.server.shutdown();
        // `_watcher` drops here, joining its thread.
    }
}

/// Map an `xcodebuild` `-destination` specifier to the simulator SDK short name
/// (the value SDK conditionals and the client dylib lookup key on). Returns
/// `None` for non-simulator destinations (devices, generic).
#[must_use]
pub fn sdk_for_destination(destination: &str) -> Option<&'static str> {
    let platform = destination
        .split(',')
        .find_map(|kv| kv.trim().strip_prefix("platform="))
        .unwrap_or("")
        .trim();
    match platform {
        "iOS Simulator" => Some("iphonesimulator"),
        "tvOS Simulator" => Some("appletvsimulator"),
        "visionOS Simulator" => Some("xrsimulator"),
        "macOS" => Some("macosx"),
        _ => None,
    }
}

/// Whether the project depends on the `Inject` package (krzysztofzablocki/Inject),
/// which SwiftUI views need (`@ObserveInjection` + `.enableInjection()`) to
/// actually redraw on injection. Returns `None` when no `Package.resolved`
/// exists yet (can't tell — likely pre-resolve), so callers stay quiet then.
#[must_use]
pub fn inject_dependency_present(root: &Path) -> Option<bool> {
    let files = package_resolved_files(root);
    if files.is_empty() {
        return None;
    }
    Some(files.iter().any(|f| {
        std::fs::read_to_string(f)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .is_some_and(|json| pins_contain_inject(&json))
    }))
}

/// Likely `Package.resolved` locations: the SPM root, and the swiftpm dirs
/// inside `*.xcworkspace` / `*.xcodeproj` bundles in `root`.
fn package_resolved_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let direct = root.join("Package.resolved");
    if direct.exists() {
        out.push(direct);
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let inner = match path.extension().and_then(|e| e.to_str()) {
                Some("xcworkspace") => path.join("xcshareddata/swiftpm/Package.resolved"),
                Some("xcodeproj") => {
                    path.join("project.xcworkspace/xcshareddata/swiftpm/Package.resolved")
                }
                _ => continue,
            };
            if inner.exists() {
                out.push(inner);
            }
        }
    }
    out
}

/// Whether a parsed `Package.resolved` pins krzysztofzablocki/Inject.
fn pins_contain_inject(json: &serde_json::Value) -> bool {
    json.get("pins")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|pins| {
            pins.iter().any(|pin| {
                pin.get("location")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|loc| {
                        let l = loc.to_ascii_lowercase();
                        l.contains("krzysztofzablocki/inject")
                    })
            })
        })
}

/// The host arch in Apple's spelling — the arch a simulator runs (the sim uses
/// the host slice) and that we resolve/link injection dylibs for.
#[must_use]
pub fn host_arch() -> String {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_for_destination_maps_simulators_only() {
        assert_eq!(
            sdk_for_destination("platform=iOS Simulator,id=ABC"),
            Some("iphonesimulator")
        );
        assert_eq!(
            sdk_for_destination("platform=visionOS Simulator,name=X"),
            Some("xrsimulator")
        );
        // Physical device / unknown → unsupported.
        assert_eq!(sdk_for_destination("platform=iOS,id=ABC"), None);
        assert_eq!(sdk_for_destination("generic/platform=iOS"), None);
    }

    #[test]
    fn host_arch_is_apple_spelling() {
        let a = host_arch();
        assert!(a == "arm64" || a == "x86_64", "unexpected arch {a}");
    }

    #[test]
    fn inject_dependency_detection() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sweetpad-inject-dep-{n}"));
        std::fs::create_dir_all(&dir).unwrap();

        // No Package.resolved → unknown (stay quiet).
        assert_eq!(inject_dependency_present(&dir), None);

        // Present but without Inject → false (warn).
        std::fs::write(
            dir.join("Package.resolved"),
            r#"{"pins":[{"identity":"swift-foo","location":"https://github.com/x/swift-foo"}]}"#,
        )
        .unwrap();
        assert_eq!(inject_dependency_present(&dir), Some(false));

        // With Inject → true (quiet).
        std::fs::write(
            dir.join("Package.resolved"),
            r#"{"pins":[{"identity":"inject","location":"https://github.com/krzysztofzablocki/Inject.git"}]}"#,
        )
        .unwrap();
        assert_eq!(inject_dependency_present(&dir), Some(true));

        std::fs::remove_dir_all(&dir).ok();
    }
}
