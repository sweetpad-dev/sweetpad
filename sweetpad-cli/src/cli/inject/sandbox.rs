//! Zero-config sandbox stripping for `--hot` on macOS (CLI_DESIGN §9d).
//!
//! You cannot inject into a sandboxed app, so hot reload on a sandboxed
//! project can only mean automating the un-sandboxing. When the sandbox comes
//! from an explicit `CODE_SIGN_ENTITLEMENTS` plist (which outranks the hot
//! build's `ENABLE_APP_SANDBOX=NO`), this module derives an **ephemeral**
//! copy with `com.apple.security.app-sandbox` removed (and `get-task-allow`
//! ensured) under the hot-reload cache, and the hot build signs with that via
//! a `CODE_SIGN_ENTITLEMENTS=…` override. The user's project is never
//! written; there is nothing to restore on exit or crash.
//!
//! Plist reading/editing shells out to `plutil` and `PlistBuddy` (present on
//! every macOS install), so binary plists work and no plist parser is
//! hand-rolled. Any failure falls back to no override — the built-product
//! preflight then refuses with its usual guidance instead of guessing.

use std::path::{Path, PathBuf};

use crate::cli::process;

/// What a hot macOS build should do about entitlements.
#[derive(Debug, PartialEq)]
pub enum SandboxPlan {
    /// No explicit sandboxed entitlements — the build-setting overrides
    /// already make the product injectable.
    Unneeded,
    /// Sign the hot build with this file (the ephemeral stripped copy, or a
    /// caller-supplied `--hot-entitlements` plist).
    Override(PathBuf),
    /// Stripping is turned off (`--keep-sandbox` / `[run] auto_unsandbox =
    /// false`) — build as before and let the preflight explain a sandboxed
    /// product.
    KeptSandbox,
}

/// Decide the entitlements story for a hot macOS build.
///
/// `entitlements` is the *effective* `CODE_SIGN_ENTITLEMENTS` resolved from
/// build settings (absolute, `None` when the target declares none);
/// `user_file` is `--hot-entitlements`; `keep` is `--keep-sandbox` or the
/// project's `auto_unsandbox = false`. Errors are complete sentences for the
/// session log; the caller treats them as "no override" (the preflight keeps
/// the last word).
pub fn plan(
    entitlements: Option<&Path>,
    user_file: Option<&Path>,
    keep: bool,
    project_key: &str,
    configuration: &str,
) -> Result<SandboxPlan, String> {
    if let Some(file) = user_file {
        if !file.is_file() {
            return Err(format!(
                "--hot-entitlements {} does not exist",
                file.display()
            ));
        }
        return Ok(SandboxPlan::Override(file.to_path_buf()));
    }
    if keep {
        return Ok(SandboxPlan::KeptSandbox);
    }
    let Some(source) = entitlements else {
        return Ok(SandboxPlan::Unneeded);
    };
    if !sandboxed(source)? {
        return Ok(SandboxPlan::Unneeded);
    }
    strip(source, project_key, configuration).map(SandboxPlan::Override)
}

/// Whether the plist asserts `com.apple.security.app-sandbox` = true.
/// `plutil -extract … raw` reads XML and binary plists alike; a missing key
/// exits non-zero, which is simply "not sandboxed". The keypath is
/// dot-separated, so the key's own dots are backslash-escaped.
fn sandboxed(plist: &Path) -> Result<bool, String> {
    if !plist.is_file() {
        return Err(format!(
            "CODE_SIGN_ENTITLEMENTS resolves to {}, which does not exist",
            plist.display()
        ));
    }
    let out = std::process::Command::new("plutil")
        .args([
            "-extract",
            r"com\.apple\.security\.app-sandbox",
            "raw",
            "-o",
            "-",
        ])
        .arg(plist)
        .output()
        .map_err(|e| format!("plutil: {e}"))?;
    if !out.status.success() {
        // Distinguish "key absent" (fine) from "unreadable plist" (give up):
        // probing an unrelated key fails identically only when the file
        // itself doesn't parse.
        let readable = std::process::Command::new("plutil")
            .args(["-lint", "-s"])
            .arg(plist)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        return if readable {
            Ok(false)
        } else {
            Err(format!("{} is not a readable plist", plist.display()))
        };
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim() == "true")
}

/// Write the sandbox-stripped copy of `source` into the hot-reload cache and
/// return its path. Regenerated from the real plist on every hot run, so
/// edits to the project's entitlements propagate.
fn strip(source: &Path, project_key: &str, configuration: &str) -> Result<PathBuf, String> {
    let dir = super::client::cache_root()
        .ok_or("no home directory for the hot-reload cache")?
        .join("entitlements")
        .join(super::client::fnv1a_hex(project_key.as_bytes()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let dest = dir.join(format!("{configuration}-nosandbox.entitlements"));
    std::fs::copy(source, &dest)
        .map_err(|e| format!("copy {} → {}: {e}", source.display(), dest.display()))?;
    // Normalize to XML so the result is inspectable regardless of the
    // source format.
    plutil_ok(&["-convert", "xml1"], &dest)?;
    plist_buddy(&dest, "Delete :com.apple.security.app-sandbox")?;
    // get-task-allow for attach/injection; Delete-then-Add because Add on an
    // existing key fails and Set on a missing one does too.
    let _ = plist_buddy(&dest, "Delete :com.apple.security.get-task-allow");
    plist_buddy(&dest, "Add :com.apple.security.get-task-allow bool true")?;
    Ok(dest)
}

/// Run `plutil <flags> <file>`, mapping failure to a sentence.
fn plutil_ok(flags: &[&str], file: &Path) -> Result<(), String> {
    let mut argv: Vec<&str> = flags.to_vec();
    let file_str = file.to_string_lossy();
    argv.push(&file_str);
    process::capture("plutil", &argv, None)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Run one `PlistBuddy -c <command> <file>`.
fn plist_buddy(file: &Path, command: &str) -> Result<(), String> {
    let file_str = file.to_string_lossy();
    process::capture("/usr/libexec/PlistBuddy", &["-c", command, &file_str], None)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_plist(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sweetpad-sandbox-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    const SANDBOXED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
    <key>com.apple.security.app-sandbox</key><true/>
    <key>com.apple.security.network.client</key><true/>
</dict></plist>"#;

    const UNSANDBOXED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
    <key>com.apple.security.network.client</key><true/>
</dict></plist>"#;

    #[test]
    fn sandboxed_detection_reads_the_key() {
        assert!(sandboxed(&temp_plist("sandboxed.entitlements", SANDBOXED)).unwrap());
        assert!(!sandboxed(&temp_plist("plain.entitlements", UNSANDBOXED)).unwrap());
        // A garbage file is an error, not a silent "not sandboxed".
        let garbage = temp_plist("garbage.entitlements", "this is not a plist {{{");
        assert!(sandboxed(&garbage).unwrap_err().contains("not a readable"));
        // A missing file names the resolution problem.
        let missing = Path::new("/nonexistent/x.entitlements");
        assert!(sandboxed(missing).unwrap_err().contains("does not exist"));
    }

    #[test]
    fn strip_removes_the_sandbox_and_adds_get_task_allow() {
        let source = temp_plist("strip-src.entitlements", SANDBOXED);
        let dest = strip(&source, "/work/App.xcodeproj#test-strip", "Debug").unwrap();
        assert!(dest.ends_with("Debug-nosandbox.entitlements"), "{dest:?}");
        let text = std::fs::read_to_string(&dest).unwrap();
        assert!(!text.contains("app-sandbox"), "{text}");
        // The rest of the entitlements survive; get-task-allow is asserted.
        assert!(text.contains("com.apple.security.network.client"), "{text}");
        assert!(text.contains("com.apple.security.get-task-allow"), "{text}");
        assert!(!sandboxed(&dest).unwrap());
        // Regeneration is idempotent (Delete-then-Add of get-task-allow).
        let again = strip(&source, "/work/App.xcodeproj#test-strip", "Debug").unwrap();
        assert_eq!(dest, again);
    }

    #[test]
    fn plan_matrix() {
        let sandboxed_file = temp_plist("plan-sandboxed.entitlements", SANDBOXED);
        let plain_file = temp_plist("plan-plain.entitlements", UNSANDBOXED);

        // No explicit entitlements → the build settings suffice.
        assert_eq!(
            plan(None, None, false, "/k", "Debug").unwrap(),
            SandboxPlan::Unneeded
        );
        // Explicit but un-sandboxed → nothing to strip.
        assert_eq!(
            plan(Some(&plain_file), None, false, "/k", "Debug").unwrap(),
            SandboxPlan::Unneeded
        );
        // Sandboxed → ephemeral override.
        let SandboxPlan::Override(path) =
            plan(Some(&sandboxed_file), None, false, "/k#plan", "Debug").unwrap()
        else {
            panic!("expected an override");
        };
        assert!(path.ends_with("Debug-nosandbox.entitlements"));
        // Opted out → sandbox kept even though the file is sandboxed.
        assert_eq!(
            plan(Some(&sandboxed_file), None, true, "/k", "Debug").unwrap(),
            SandboxPlan::KeptSandbox
        );
        // A user-supplied file wins over everything (and must exist).
        assert_eq!(
            plan(
                Some(&sandboxed_file),
                Some(&plain_file),
                false,
                "/k",
                "Debug"
            )
            .unwrap(),
            SandboxPlan::Override(plain_file.clone())
        );
        let err = plan(
            None,
            Some(Path::new("/nonexistent.plist")),
            false,
            "/k",
            "Debug",
        )
        .unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
    }
}
