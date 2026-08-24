//! What the local SwiftPM packages in a workspace contribute: scheme and
//! target names.
//!
//! `sweetpad_lib::workspace` reports which members are packages
//! ([`sweetpad_lib::workspace::Workspace::package_refs`]) but cannot name what
//! they hold: a package's schemes and targets come from its manifest, and
//! `Package.swift` is Swift source that only the toolchain can evaluate.
//! `swift package dump-package` compiles it against `libPackageDescription`
//! and runs it, so reading the manifest means spawning — which belongs here
//! rather than in the file-format crate.
//!
//! **The spawn is slow enough to need a cache.** A cold `dump-package` takes
//! seconds (SwiftPM compiles the manifest); a warm one still costs the `swift`
//! driver's startup. Results are memoized on disk against the manifest's
//! `(len, mtime)` — the same stamp `sweetpad_lib`'s parse caches use — so the
//! cost is paid once per manifest edit instead of once per command. The CLI is
//! one-shot, so an in-process memo alone would never hit.
//!
//! **A member package's schemes are its products, with no `<name>-Package`
//! aggregate.** `xcodebuild -list` synthesizes that aggregate for a package
//! opened on its own but not for one inside a workspace, so listing it here
//! would offer a scheme `xcodebuild` then rejects.
//!
//! **Targets include test targets**, unlike schemes: a target list exists to
//! drive `-only-testing:`, where a test target is the whole point.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

use serde_json::Value;

/// What one local package contributes to its workspace.
#[derive(Debug, Clone)]
pub struct PackageMember {
    /// The package directory, as it appeared in `package_refs`.
    pub path: PathBuf,
    /// Scheme names — the manifest's products (see the module docs).
    pub schemes: Vec<String>,
    /// Every target the manifest declares, test targets included.
    pub targets: Vec<String>,
}

/// `(len, mtime_nanos)` of a `Package.swift` — changed either way means the
/// cached names are stale.
type Stamp = (u64, u128);

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((meta.len(), mtime))
}

/// `<state>/sweetpad/package-members.json` — one object keyed by absolute
/// package path. A single file (rather than one per package) keeps the cache
/// inspectable and its rewrite atomic.
fn cache_file() -> Option<PathBuf> {
    crate::paths::sweetpad_state_dir().map(|d| d.join("package-members.json"))
}

fn read_cache() -> BTreeMap<String, Value> {
    let Some(path) = cache_file() else {
        return BTreeMap::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Rewrite the cache via a temp file + rename so a concurrent reader never
/// sees a half-written object. A failure here is silent: the cache is an
/// optimization, and a command that resolved its schemes should not fail
/// because it could not record them.
fn write_cache(entries: &BTreeMap<String, Value>) {
    let Some(path) = cache_file() else {
        return;
    };
    let Some(dir) = path.parent() else {
        return;
    };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = path.with_extension(format!("json.tmp{}", std::process::id()));
    let Ok(text) = serde_json::to_string(entries) else {
        return;
    };
    let written = fs::File::create(&tmp).and_then(|mut f| f.write_all(text.as_bytes()));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
        return;
    }
    if fs::rename(&tmp, &path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

fn string_list(entry: &Value, key: &str) -> Option<Vec<String>> {
    Some(
        entry
            .get(key)?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

/// A cached entry is usable only when the manifest stamp still matches *and*
/// it carries every field this version reads — an entry written before
/// `targets` existed is treated as a miss rather than as a package with no
/// targets.
fn cached_member(
    entries: &BTreeMap<String, Value>,
    path: &Path,
    current: Stamp,
) -> Option<PackageMember> {
    let entry = entries.get(&path.to_string_lossy().into_owned())?;
    let len = entry.get("len")?.as_u64()?;
    let mtime = entry.get("mtime")?.as_str()?.parse::<u128>().ok()?;
    if (len, mtime) != current {
        return None;
    }
    Some(PackageMember {
        path: path.to_path_buf(),
        schemes: string_list(entry, "schemes")?,
        targets: string_list(entry, "targets")?,
    })
}

/// Resolve every package in `package_dirs`, each paired with the path it came
/// from so the `*_with_packages` methods on
/// [`sweetpad_lib::workspace::Workspace`] can match results back to members
/// the workspace still references.
///
/// Cache hits cost a single file read. Misses run `dump-package` for each
/// stale package concurrently, so a workspace with several packages pays one
/// manifest evaluation's latency rather than the sum. A package whose manifest
/// fails to evaluate contributes nothing and is not cached, so the next call
/// retries — a broken manifest is usually mid-edit.
#[must_use]
pub fn resolve(package_dirs: &[PathBuf], developer_dir: Option<&Path>) -> Vec<PackageMember> {
    let mut entries = read_cache();
    let mut out: Vec<PackageMember> = Vec::new();
    let mut stale: Vec<(PathBuf, Stamp)> = Vec::new();

    for dir in package_dirs {
        let Some(current) = stamp(&dir.join("Package.swift")) else {
            continue;
        };
        if let Some(member) = cached_member(&entries, dir, current) {
            out.push(member);
        } else {
            stale.push((dir.clone(), current));
        }
    }

    if stale.is_empty() {
        return out;
    }

    let dumped: Vec<(Stamp, Option<PackageMember>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = stale
            .iter()
            .map(|(dir, st)| {
                let dir = dir.clone();
                let st = *st;
                scope.spawn(move || {
                    let member = dump_package(&dir, developer_dir).map(|m| PackageMember {
                        path: dir,
                        schemes: product_schemes(&m),
                        targets: target_names(&m),
                    });
                    (st, member)
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });

    let mut changed = false;
    for ((len, mtime), member) in dumped {
        let Some(member) = member else {
            continue;
        };
        entries.insert(
            member.path.to_string_lossy().into_owned(),
            serde_json::json!({
                "len": len,
                // u128 exceeds JSON's safe integer range; keep it as a string
                // so a round-trip can't quietly lose precision.
                "mtime": mtime.to_string(),
                "schemes": member.schemes,
                "targets": member.targets,
            }),
        );
        changed = true;
        out.push(member);
    }
    if changed {
        write_cache(&entries);
    }
    out
}

/// The `(path, schemes)` pairs
/// [`sweetpad_lib::workspace::Workspace::merged_schemes_with_packages`] takes.
#[must_use]
pub fn scheme_pairs(members: &[PackageMember]) -> Vec<(PathBuf, Vec<String>)> {
    members
        .iter()
        .map(|m| (m.path.clone(), m.schemes.clone()))
        .collect()
}

/// The `(path, targets)` pairs
/// [`sweetpad_lib::workspace::Workspace::merged_targets_with_packages`] takes.
#[must_use]
pub fn target_pairs(members: &[PackageMember]) -> Vec<(PathBuf, Vec<String>)> {
    members
        .iter()
        .map(|m| (m.path.clone(), m.targets.clone()))
        .collect()
}

/// Run `swift package dump-package` in `dir` and parse its JSON. `None` on any
/// failure (no toolchain, manifest doesn't compile, unexpected output) — the
/// caller degrades to the names it can read from files.
fn dump_package(dir: &Path, developer_dir: Option<&Path>) -> Option<Value> {
    let mut cmd = Command::new("swift");
    if let Some(dev) = developer_dir {
        cmd.env("DEVELOPER_DIR", dev);
    }
    let output = cmd
        .args(["package", "dump-package"])
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // Skip any leading non-JSON chatter, like the CLI's other JSON readers.
    let start = text.find('{')?;
    serde_json::from_str(&text[start..]).ok()
}

/// Product names from a dumped manifest — one scheme each, which is what
/// `xcodebuild -list` reports for a package inside a workspace (a `.plugin`
/// product gets a scheme too). Falls back to non-test target names when a
/// package declares no products, so a target-only package still offers
/// something to select. The `<name>-Package` aggregate is deliberately absent
/// (see the module docs).
fn product_schemes(manifest: &Value) -> Vec<String> {
    let products = names_in(manifest, "products", |_| true);
    if products.is_empty() {
        names_in(manifest, "targets", |t| !is_test_target(t))
    } else {
        products
    }
}

/// Every declared target, tests included — a target list drives
/// `-only-testing:`, so dropping test targets would remove the useful half.
fn target_names(manifest: &Value) -> Vec<String> {
    names_in(manifest, "targets", |_| true)
}

fn names_in(manifest: &Value, key: &str, keep: impl Fn(&Value) -> bool) -> Vec<String> {
    manifest
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| keep(item))
                .filter_map(|item| item.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// A manifest target is a test target when its `type` union carries the `test`
/// tag (`{"test":{}}`); older dumps spell the same thing as a plain string.
fn is_test_target(target: &Value) -> bool {
    match target.get("type") {
        Some(Value::Object(map)) => map.contains_key("test"),
        Some(Value::String(s)) => s == "test",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Value {
        serde_json::json!({
            "name": "MyLib",
            "products": [{"name": "LibA"}, {"name": "MyPlugin"}],
            "targets": [
                {"name": "LibA", "type": {"regular": {}}},
                {"name": "MyPlugin", "type": {"plugin": {}}},
                {"name": "LibATests", "type": {"test": {}}},
            ],
        })
    }

    #[test]
    fn products_become_schemes_without_the_package_aggregate() {
        assert_eq!(product_schemes(&manifest()), vec!["LibA", "MyPlugin"]);
    }

    #[test]
    fn targets_keep_test_targets_that_schemes_drop() {
        assert_eq!(
            target_names(&manifest()),
            vec!["LibA", "MyPlugin", "LibATests"]
        );
    }

    #[test]
    fn targets_stand_in_when_a_package_declares_no_products() {
        let m = serde_json::json!({
            "name": "MyLib",
            "products": [],
            "targets": [
                {"name": "Core", "type": {"regular": {}}},
                {"name": "CoreTests", "type": {"test": {}}},
            ],
        });
        assert_eq!(product_schemes(&m), vec!["Core"]);
        // The same manifest still reports both as targets.
        assert_eq!(target_names(&m), vec!["Core", "CoreTests"]);
    }

    #[test]
    fn a_string_typed_test_target_is_still_recognized() {
        let m = serde_json::json!({
            "name": "MyLib",
            "products": [],
            "targets": [{"name": "Core"}, {"name": "CoreTests", "type": "test"}],
        });
        assert_eq!(product_schemes(&m), vec!["Core"]);
    }

    #[test]
    fn a_cache_entry_is_reused_only_while_the_stamp_matches() {
        let mut entries = BTreeMap::new();
        entries.insert(
            "/pkg".to_string(),
            serde_json::json!({
                "len": 10, "mtime": "99",
                "schemes": ["LibA"], "targets": ["LibA", "LibATests"],
            }),
        );
        let hit = cached_member(&entries, Path::new("/pkg"), (10, 99)).unwrap();
        assert_eq!(hit.schemes, vec!["LibA"]);
        assert_eq!(hit.targets, vec!["LibA", "LibATests"]);
        assert!(cached_member(&entries, Path::new("/pkg"), (11, 99)).is_none());
        assert!(cached_member(&entries, Path::new("/pkg"), (10, 100)).is_none());
        assert!(cached_member(&entries, Path::new("/other"), (10, 99)).is_none());
    }

    #[test]
    fn an_entry_missing_targets_is_a_miss_not_an_empty_target_list() {
        let mut entries = BTreeMap::new();
        entries.insert(
            "/pkg".to_string(),
            serde_json::json!({"len": 10, "mtime": "99", "schemes": ["LibA"]}),
        );
        assert!(cached_member(&entries, Path::new("/pkg"), (10, 99)).is_none());
    }
}
