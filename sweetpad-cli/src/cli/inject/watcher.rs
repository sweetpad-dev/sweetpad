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
    /// Start watching `root` (recursively) for `.swift` saves. The files already
    /// there are snapshotted before this returns, so a save made right after it
    /// counts as one.
    pub fn start(root: &Path, on_change: OnChange) -> Watcher {
        let root = root.to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);

        // The callback runs a whole recompile (seconds — minutes for a big
        // module), so it gets its own detached worker: the poll thread stays
        // joinable and quitting the session never stalls behind an in-flight
        // compile. The worker drains sequentially (saves stay ordered) and
        // exits when the poll thread drops the sender; queued work is skipped
        // once the watcher is stopping.
        let (tx, rx) = std::sync::mpsc::channel::<PathBuf>();
        let worker_stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while let Ok(path) = rx.recv() {
                if worker_stop.load(Ordering::Relaxed) {
                    break;
                }
                on_change(&path);
            }
        });

        // Initial snapshot, so the files that already exist don't fire. Taken
        // here rather than on the poll thread: a save made before that thread
        // got to the file would land in the snapshot and never fire.
        let mut mtimes: HashMap<PathBuf, SystemTime> = HashMap::new();
        scan(&root, &mut |path, mtime| {
            mtimes.insert(path, mtime);
        });

        let handle = std::thread::spawn(move || {
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
                    let _ = tx.send(path);
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
        // Joins only the poll thread (bounded by one poll tick); the compile
        // worker is detached and winds down on its own.
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Walk `dir` recursively, invoking `visit` for each `.swift` file with its
/// modification time. Skips [`IGNORED_DIRS`] and hidden directories. Follows
/// symlinked directories (monorepos commonly link `Sources` elsewhere), with
/// a canonical-path guard so a symlink cycle can't loop the walk.
fn scan(dir: &Path, visit: &mut impl FnMut(PathBuf, SystemTime)) {
    let mut seen_links = std::collections::HashSet::new();
    scan_inner(dir, visit, &mut seen_links);
}

fn scan_inner(
    dir: &Path,
    visit: &mut impl FnMut(PathBuf, SystemTime),
    seen_links: &mut std::collections::HashSet<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        // `file_type()` doesn't follow symlinks — treat a symlink-to-directory
        // as a directory so linked source trees are watched too.
        let is_dir = ft.is_dir() || (ft.is_symlink() && path.is_dir());
        if is_dir {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || IGNORED_DIRS.contains(&name) {
                continue;
            }
            // Every cycle must pass through a symlink — guarding those alone
            // breaks all loops.
            if ft.is_symlink() {
                let Ok(canon) = std::fs::canonicalize(&path) else {
                    continue;
                };
                if !seen_links.insert(canon) {
                    continue;
                }
            }
            scan_inner(&path, visit, seen_links);
        } else if path.extension().is_some_and(|e| e == "swift")
            // `fs::metadata` (follows symlinks), not `entry.metadata()`: a
            // symlinked .swift file must report the *target's* mtime — the
            // link's own mtime never changes on edit, so its saves would
            // otherwise be invisible.
            && let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified())
        {
            visit(path, mtime);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::cli::testdir::TempDir;

    fn temp_dir(tag: &str) -> TempDir {
        TempDir::new(&format!("sweetpad-watch-{tag}"))
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

        // The snapshot is taken by the time `start` returns, so a save made
        // straight after it counts.
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
    }
}
