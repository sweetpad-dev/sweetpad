//! The local Swift packages of a workspace or project, read from the PIF
//! `swift package dump-pif` prints for them (#343).
//!
//! [`sweetpad_lib::pif`] reads the PIF; this module produces it. A root
//! `Package.swift` depending on the container's local packages stands in for
//! the workspace, beside a copy of the `Package.resolved` Xcode resolved
//! against, so SwiftPM picks the same remote versions without asking the
//! network (`--disable-automatic-resolution`).
//!
//! **The checkouts are Xcode's, through a clone.** Xcode keeps the remote
//! packages under `<DerivedData>/<container>/SourcePackages`. Pointing
//! `--scratch-path` there would rewrite its `workspace-state.json` under
//! Xcode's feet, so the dump runs on an APFS clone of it (`cp -c`, which
//! copies no data and takes about 0.1 s). A container Xcode never resolved
//! has no checkouts, and a dump that needs them fails: its packages then go
//! without arguments until Xcode resolves them, and the reason is logged.
//!
//! A cold dump takes a few seconds and a warm one under one, so the output is
//! kept on disk with a stamp of everything that changes it: the manifests,
//! `Package.resolved`, Xcode's `workspace-state.json`, and the directories
//! holding each target's sources, since SwiftPM finds sources by listing them.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

use serde_json::{Value, json};

use sweetpad_lib::pif::{self, Package};
use sweetpad_lib::{project, workspace};

use crate::package_members::Toolchain;

/// Which reading of a cached dump this code makes. Bump it when what the dump
/// is read into changes, so a dump an older build cached is redone.
const CACHE_SCHEMA: u64 = 1;

/// The local packages `container` uses directly: a workspace's package
/// members and the packages its projects declare, or a project's own.
#[must_use]
pub fn local_packages(container: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = if container.extension().is_some_and(|e| e == "xcworkspace") {
        workspace::open(container)
            .map(|ws| {
                let mut dirs = ws.package_refs.clone();
                dirs.extend(ws.project_package_refs());
                dirs
            })
            .unwrap_or_default()
    } else {
        project::open(container)
            .map(|p| p.package_refs)
            .unwrap_or_default()
    };
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(project::standardize(d)));
    dirs
}

/// Where Xcode keeps the `Package.resolved` of `container`.
fn package_resolved(container: &Path) -> PathBuf {
    let bundle = if container.extension().is_some_and(|e| e == "xcworkspace") {
        container.to_path_buf()
    } else {
        container.join("project.xcworkspace")
    };
    bundle.join("xcshareddata/swiftpm/Package.resolved")
}

/// The packages `container` uses, from the cached dump when nothing it read
/// has changed, else from a fresh one. `source_packages` is the container's
/// `SourcePackages` folder under DerivedData, and `cache` a directory this
/// container's dump may own. `Ok(vec![])` when it uses no local package.
pub fn load(
    container: &Path,
    source_packages: &Path,
    cache: &Path,
    toolchain: &Toolchain,
) -> Result<Vec<Package>, String> {
    let dirs = local_packages(container);
    if dirs.is_empty() {
        return Ok(Vec::new());
    }
    let root = cache.join("root");
    let scratch = cache.join("scratch");
    // The dump names the checkouts in the clone; they are mapped back to
    // Xcode's, which is where its builds read them from.
    let skip = [root.clone(), scratch.clone(), source_packages.to_path_buf()];
    let resolved = package_resolved(container);
    let state = source_packages.join("workspace-state.json");
    let manifest = pif::root_manifest(&dirs);
    if let Some(dumped) = cached(cache, &manifest, &[&resolved, &state]) {
        return Ok(pif::parse(&dumped, &skip));
    }

    fs::create_dir_all(&root).map_err(|e| format!("create {}: {e}", root.display()))?;
    fs::write(root.join("Package.swift"), &manifest).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(root.join("Package.resolved"));
    if resolved.is_file() {
        fs::copy(&resolved, root.join("Package.resolved")).map_err(|e| e.to_string())?;
    }
    let _ = fs::remove_dir_all(&scratch);
    if source_packages.is_dir() {
        let cloned = Command::new("cp")
            .arg("-cR")
            .arg(source_packages)
            .arg(&scratch)
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("clone {}: {e}", source_packages.display()))?;
        if !cloned.status.success() {
            return Err(format!(
                "clone {}: {}",
                source_packages.display(),
                String::from_utf8_lossy(&cloned.stderr).trim()
            ));
        }
    }

    let mut cmd = Command::new(toolchain.swift());
    cmd.args(["package", "dump-pif", "--disable-automatic-resolution"])
        .arg("--package-path")
        .arg(&root)
        .arg("--scratch-path")
        .arg(&scratch)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    if let Some(dir) = &toolchain.developer_dir {
        cmd.env("DEVELOPER_DIR", dir);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("running swift package dump-pif: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let reason = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("no output");
        return Err(format!(
            "swift package dump-pif exited with {}: {reason}",
            out.status
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout).replace(
        &*scratch.to_string_lossy(),
        &source_packages.to_string_lossy(),
    );
    let dumped: Vec<Value> =
        serde_json::from_str(&text).map_err(|e| format!("parsing swift package dump-pif: {e}"))?;
    let packages = pif::parse(&dumped, &skip);
    let mut inputs: Vec<PathBuf> = vec![resolved, state];
    for package in &packages {
        inputs.push(package.manifest());
        for target in &package.targets {
            inputs.extend(
                target
                    .sources
                    .iter()
                    .filter_map(|s| s.parent().map(Path::to_path_buf)),
            );
        }
    }
    inputs.sort();
    inputs.dedup();
    save(cache, &manifest, &inputs, text.as_bytes());
    Ok(packages)
}

/// `(len, mtime)` of a file or directory, `null` when it is missing, so a
/// file that appears or goes away changes the stamp too.
fn stamp(path: &Path) -> Value {
    fs::metadata(path)
        .ok()
        .and_then(|m| {
            let mtime = m
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_nanos();
            Some(json!([m.len(), mtime.to_string()]))
        })
        .unwrap_or(Value::Null)
}

fn save(cache: &Path, manifest: &str, inputs: &[PathBuf], dumped: &[u8]) {
    let stamps: Vec<Value> = inputs
        .iter()
        .map(|p| json!([p.to_string_lossy(), stamp(p)]))
        .collect();
    let record = json!({ "schema": CACHE_SCHEMA, "manifest": manifest, "inputs": stamps });
    let _ = fs::write(cache.join("pif.json"), dumped);
    let _ = fs::write(cache.join("stamp.json"), record.to_string());
}

/// The cached dump, when it was made for `manifest` and none of its inputs
/// changed. `always` are checked whether or not the record names them.
fn cached(cache: &Path, manifest: &str, always: &[&Path]) -> Option<Vec<Value>> {
    let record: Value = serde_json::from_slice(&fs::read(cache.join("stamp.json")).ok()?).ok()?;
    if record["schema"] != CACHE_SCHEMA || record["manifest"] != manifest {
        return None;
    }
    let inputs = record["inputs"].as_array()?;
    for path in always {
        let named = inputs.iter().any(|i| i[0].as_str() == path.to_str());
        if !named {
            return None;
        }
    }
    let fresh = inputs
        .iter()
        .all(|i| i[0].as_str().is_some_and(|p| stamp(Path::new(p)) == i[1]));
    if !fresh {
        return None;
    }
    serde_json::from_slice(&fs::read(cache.join("pif.json")).ok()?).ok()
}

/// A directory under the state dir that the dump for `container` owns.
#[must_use]
pub fn cache_dir(container: &Path) -> Option<PathBuf> {
    let key = sweetpad_lib::derived_data::container_hash(&project::standardize(container));
    crate::paths::sweetpad_state_dir().map(|d| d.join("package-pif").join(key))
}

/// The files and directories whose change can change the packages of
/// `container`: the manifests and `Package.resolved` it starts from, Xcode's
/// `workspace-state.json`, and for each of `known`, its manifest and the
/// directories holding its sources.
#[must_use]
pub fn inputs(
    container: &Path,
    source_packages: &Path,
    known: &[std::sync::Arc<Package>],
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = local_packages(container)
        .into_iter()
        .map(|d| d.join("Package.swift"))
        .collect();
    out.push(package_resolved(container));
    out.push(source_packages.join("workspace-state.json"));
    for package in known {
        out.push(package.manifest());
        for target in &package.targets {
            out.extend(
                target
                    .sources
                    .iter()
                    .filter_map(|s| s.parent().map(Path::to_path_buf)),
            );
        }
    }
    out.sort();
    out.dedup();
    out
}
