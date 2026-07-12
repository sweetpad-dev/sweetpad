//! Built-in hot reload for `app run --hot` (CLI_DESIGN §9d).
//!
//! The CLI acts as the InjectionNext *server*: it binds `:8887`, the in-app
//! client (injected via `DYLD_INSERT_LIBRARIES`) connects out, and on each Swift
//! save the [`watcher`] drives the [`server`] to recompile the file (via the
//! [`recompiler`], resolver-default / build-log-switchable) and `.load` it into
//! the running app. [`client`] resolves the client dylib + launch env.
//!
//! The socket protocol and recompile→load→inject chain are validated end-to-end
//! on a real simulator and a native mac app by `ci/hot-reload-e2e.sh` (the
//! `--hot-selfcheck` nonce round-trip, both recompilers).

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

impl Drop for HotSession {
    fn drop(&mut self) {
        // Keeps the doc contract above: a plain drop (e.g. a `?` early return
        // in the caller) also stops the server's accept loop and connection,
        // not just the watcher. `InjectServer::shutdown` is idempotent, so the
        // explicit `shutdown` path re-running this in its drop is harmless.
        self.server.shutdown();
    }
}

/// Map an `xcodebuild` `-destination` specifier to the SDK short name (the
/// value SDK conditionals and the client dylib lookup key on) for the
/// injectable destinations: simulators and native macOS. Returns `None` for
/// the rest (devices, generic).
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

/// Preflight a built macOS app for injectability. The hot build disables the
/// hardened runtime and App Sandbox via build settings, but an explicit
/// entitlements file or a re-signing build phase can put them back — and either
/// one kills the session silently (dyld strips `DYLD_INSERT_LIBRARIES` under the
/// hardened runtime; the sandbox blocks the client's socket and dlopen from
/// outside the container). Checked here, before launch, so the failure is a
/// named cause with the fix instead of a dead session.
pub fn mac_preflight(app: &Path) -> Result<(), String> {
    let app_str = app.to_string_lossy();
    // `codesign -d -vv` prints the CodeDirectory info (incl. the `runtime` flag)
    // to stderr; an unsigned bundle fails, which means nothing is enforced.
    let info = match std::process::Command::new("codesign")
        .args(["-d", "-vv", &app_str])
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stderr).into_owned(),
        _ => return Ok(()),
    };
    let entitlements = std::process::Command::new("codesign")
        .args(["-d", "--entitlements", "-", "--xml", &app_str])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    preflight_verdict(&info, &entitlements)
}

/// The pure verdict behind [`mac_preflight`], split out for tests: `info` is
/// `codesign -d -vv` output, `entitlements` the signed entitlements plist.
fn preflight_verdict(info: &str, entitlements: &str) -> Result<(), String> {
    if entitlement_true(entitlements, "com.apple.security.app-sandbox") {
        return Err(
            "the built app is sandboxed (com.apple.security.app-sandbox): the injection \
             client can't reach the CLI's socket or load recompiled dylibs from outside \
             its container. The hot build passes ENABLE_APP_SANDBOX=NO, so the entitlement \
             comes from an explicit .entitlements file — turn App Sandbox off for the \
             Debug configuration (Signing & Capabilities) and rerun"
                .into(),
        );
    }
    let hardened = hardened_runtime_flagged(info);
    let opted_out = entitlement_true(
        entitlements,
        "com.apple.security.cs.allow-dyld-environment-variables",
    ) && entitlement_true(
        entitlements,
        "com.apple.security.cs.disable-library-validation",
    );
    if hardened && !opted_out {
        return Err(
            "the built app has the hardened runtime, so dyld strips DYLD_INSERT_LIBRARIES \
             and library validation rejects the recompiled dylibs. The hot build passes \
             ENABLE_HARDENED_RUNTIME=NO, so something re-signs the product (a run-script \
             phase or OTHER_CODE_SIGN_FLAGS) — remove that for Debug, or add both the \
             com.apple.security.cs.allow-dyld-environment-variables and \
             com.apple.security.cs.disable-library-validation entitlements"
                .into(),
        );
    }
    Ok(())
}

/// Whether the CodeDirectory flags carry `runtime` (hardened runtime), e.g.
/// `flags=0x10000(runtime)` or `flags=0x10002(adhoc,runtime)`.
fn hardened_runtime_flagged(codesign_info: &str) -> bool {
    codesign_info.lines().any(|l| {
        l.trim_start().starts_with("CodeDirectory")
            && l.split("flags=").nth(1).is_some_and(|flags| {
                flags
                    .split_once(')')
                    .is_some_and(|(inside, _)| inside.contains("runtime"))
            })
    })
}

/// Whether an entitlements plist sets `key` to `<true/>`. Matches the XML form
/// codesign prints; a key that is absent or `<false/>` is not set.
fn entitlement_true(plist: &str, key: &str) -> bool {
    let needle = format!("<key>{key}</key>");
    plist.match_indices(&needle).any(|(i, _)| {
        plist[i + needle.len()..]
            .trim_start()
            .starts_with("<true/>")
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
    fn sdk_for_destination_maps_injectable_destinations() {
        assert_eq!(
            sdk_for_destination("platform=iOS Simulator,id=ABC"),
            Some("iphonesimulator")
        );
        assert_eq!(
            sdk_for_destination("platform=visionOS Simulator,name=X"),
            Some("xrsimulator")
        );
        assert_eq!(sdk_for_destination("platform=macOS"), Some("macosx"));
        // Physical device / unknown → unsupported.
        assert_eq!(sdk_for_destination("platform=iOS,id=ABC"), None);
        assert_eq!(sdk_for_destination("generic/platform=iOS"), None);
    }

    #[test]
    fn hardened_runtime_flag_detection() {
        let hardened = "Executable=/x/App\nIdentifier=dev.x.app\n\
                        CodeDirectory v=20400 size=768 flags=0x10000(runtime) hashes=13+7 location=embedded\n";
        let adhoc_hardened =
            "CodeDirectory v=20400 size=768 flags=0x10002(adhoc,runtime) hashes=13+7\n";
        let plain = "CodeDirectory v=20400 size=768 flags=0x2(adhoc) hashes=13+7\n";
        let linker =
            "CodeDirectory v=20400 size=768 flags=0x20002(adhoc,linker-signed) hashes=13+7\n";
        assert!(hardened_runtime_flagged(hardened));
        assert!(hardened_runtime_flagged(adhoc_hardened));
        assert!(!hardened_runtime_flagged(plain));
        assert!(!hardened_runtime_flagged(linker));
    }

    #[test]
    fn entitlement_true_matches_only_true_keys() {
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
    <key>com.apple.security.app-sandbox</key>
    <true/>
    <key>com.apple.security.network.client</key>
    <false/>
</dict></plist>"#;
        assert!(entitlement_true(plist, "com.apple.security.app-sandbox"));
        assert!(!entitlement_true(
            plist,
            "com.apple.security.network.client"
        ));
        assert!(!entitlement_true(
            plist,
            "com.apple.security.get-task-allow"
        ));
        assert!(!entitlement_true("", "com.apple.security.app-sandbox"));
    }

    #[test]
    fn preflight_verdicts() {
        let sandboxed = "<dict><key>com.apple.security.app-sandbox</key><true/></dict>";
        let hardened_info = "CodeDirectory v=20400 flags=0x10000(runtime) hashes=1+1\n";
        let plain_info = "CodeDirectory v=20400 flags=0x2(adhoc) hashes=1+1\n";
        let opted_out = "<dict>\
            <key>com.apple.security.cs.allow-dyld-environment-variables</key><true/>\
            <key>com.apple.security.cs.disable-library-validation</key><true/>\
            </dict>";

        // Sandbox → refused with the entitlements-file fix.
        let err = preflight_verdict(plain_info, sandboxed).unwrap_err();
        assert!(err.contains("app-sandbox"), "{err}");
        // Hardened without the opt-out entitlements → refused.
        let err = preflight_verdict(hardened_info, "").unwrap_err();
        assert!(err.contains("hardened runtime"), "{err}");
        // Hardened but opted out via entitlements → injectable.
        assert!(preflight_verdict(hardened_info, opted_out).is_ok());
        // Plain Debug product (or unsigned) → injectable.
        assert!(preflight_verdict(plain_info, "").is_ok());
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
