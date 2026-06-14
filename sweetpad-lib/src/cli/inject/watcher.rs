//! A small, dependency-free polling file watcher for `--hot`. It snapshots the
//! `.swift` files under the workspace root and, on each poll, fires a callback
//! for any whose modification time advanced — i.e. a save. Polling (rather than
//! an FS-events crate) keeps the CLI's dependency surface minimal and is more
//! than fast enough for a human edit loop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// Called with each saved `.swift` file.
pub type OnChange = Arc<dyn Fn(&Path) + Send + Sync>;

/// A running watcher; stops its thread on drop.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// Directory names whose subtrees never hold editable sources (build output,
/// VCS, dependency checkouts) — skipped so a poll stays cheap.
const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".build",
    "build",
    "DerivedData",
    "Pods",
    "Carthage",
    ".swiftpm",
    "node_modules",
    ".cache",
];

const POLL_INTERVAL: Duration = Duration::from_millis(300);

impl Watcher {
    /// Start watching `root` (recursively) for `.swift` saves.
    pub fn start(root: &Path, on_change: OnChange) -> Watcher {
        let root = root.to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);

        let handle = std::thread::spawn(move || {
            // Initial snapshot — don't fire for files that already exist.
            let mut mtimes: HashMap<PathBuf, SystemTime> = HashMap::new();
            scan(&root, &mut |path, mtime| {
                mtimes.insert(path, mtime);
            });

            while !stop_thread.load(Ordering::Relaxed) {
                std::thread::sleep(POLL_INTERVAL);
                if stop_thread.load(Ordering::Relaxed) {
                    break;
                }
                let mut changed: Vec<PathBuf> = Vec::new();
                scan(&root, &mut |path, mtime| {
                    let advanced = mtimes.get(&path).is_none_or(|&prev| mtime > prev);
                    if advanced {
                        mtimes.insert(path.clone(), mtime);
                        changed.push(path);
                    }
                });
                for path in changed {
                    on_change(&path);
                }
            }
        });

        Watcher {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Walk `dir` recursively, invoking `visit` for each `.swift` file with its
/// modification time. Skips [`IGNORED_DIRS`] and hidden directories.
fn scan(dir: &Path, visit: &mut impl FnMut(PathBuf, SystemTime)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || IGNORED_DIRS.contains(&name) {
                continue;
            }
            scan(&path, visit);
        } else if path.extension().is_some_and(|e| e == "swift")
            && let Ok(mtime) = entry.metadata().and_then(|m| m.modified())
        {
            visit(path, mtime);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sweetpad-watch-{tag}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fires_on_save_not_on_initial_files() {
        let dir = temp_dir("save");
        std::fs::write(dir.join("Existing.swift"), "// v1").unwrap();
        std::fs::create_dir(dir.join("DerivedData")).unwrap();
        std::fs::write(dir.join("DerivedData/Ignored.swift"), "// build output").unwrap();

        let hits: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&hits);
        let on_change: OnChange = Arc::new(move |p: &Path| {
            sink.lock()
                .unwrap()
                .push(p.file_name().unwrap().to_string_lossy().into_owned());
        });
        let _w = Watcher::start(&dir, on_change);

        // Let the initial snapshot settle, then modify + add files.
        std::thread::sleep(Duration::from_millis(450));
        std::fs::write(dir.join("Existing.swift"), "// v2 changed").unwrap();
        std::fs::write(dir.join("New.swift"), "// brand new").unwrap();
        std::thread::sleep(Duration::from_millis(700));

        let seen = hits.lock().unwrap().clone();
        assert!(
            seen.contains(&"Existing.swift".to_string()),
            "save should fire: {seen:?}"
        );
        assert!(
            seen.contains(&"New.swift".to_string()),
            "new file should fire: {seen:?}"
        );
        assert!(
            !seen.contains(&"Ignored.swift".to_string()),
            "DerivedData must be ignored"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
