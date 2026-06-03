//! Process-global, mtime-validated parse caches.
//!
//! The node addon is long-lived, so the same `project.pbxproj` / `.xcconfig`
//! is read on many `build-settings` and `list` calls. Unlike the xcspec
//! catalog — a pure function of the Xcode version, cached once by
//! [`crate::catalog_cache`] — these files are user-mutable, so each entry is
//! validated against the file's current `(len, mtime)` and transparently
//! reparsed when it changes on disk.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::UNIX_EPOCH;

/// `(len, mtime_nanos)` — a cheap stand-in for the file's contents. A changed
/// length or modification time means "reparse"; both matching means "reuse".
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

/// A path-keyed, mtime-validated cache of `Arc<T>` parses. Hold one in a
/// `static` via [`LazyLock`](std::sync::LazyLock) and call
/// [`Self::get_or_parse`].
pub(crate) struct ParseCache<T> {
    entries: Mutex<HashMap<PathBuf, (Stamp, Arc<T>)>>,
}

impl<T> ParseCache<T> {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Return the cached parse for `path` when the file is unchanged since it
    /// was cached; otherwise run `parse`, store it under the current stamp, and
    /// return it. A file we can't `stat` is parsed every call (and never
    /// cached), so a missing stamp can never serve a stale value. `parse` runs
    /// outside the lock, so concurrent first-parses of distinct files don't
    /// serialize (a rare race may parse the same file twice; last writer wins).
    pub(crate) fn get_or_parse<E>(
        &self,
        path: &Path,
        parse: impl FnOnce(&Path) -> Result<T, E>,
    ) -> Result<Arc<T>, E> {
        let current = stamp(path);
        if let Some(current) = current {
            let entries = self.lock();
            if let Some((cached, value)) = entries.get(path)
                && *cached == current
            {
                return Ok(Arc::clone(value));
            }
        }
        let value = Arc::new(parse(path)?);
        if let Some(current) = current {
            self.lock()
                .insert(path.to_path_buf(), (current, Arc::clone(&value)));
        }
        Ok(value)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<PathBuf, (Stamp, Arc<T>)>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<T> Default for ParseCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn reuses_parse_until_the_file_changes() {
        let dir = std::env::temp_dir().join(format!("sweetpad-fc-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.txt");
        fs::write(&path, "one").unwrap();

        let cache: ParseCache<String> = ParseCache::new();
        let calls = Cell::new(0);
        let read = |p: &Path| -> Result<String, std::io::Error> {
            calls.set(calls.get() + 1);
            fs::read_to_string(p)
        };

        let a = cache.get_or_parse(&path, read).unwrap();
        let b = cache.get_or_parse(&path, read).unwrap();
        assert_eq!(*a, "one");
        assert!(Arc::ptr_eq(&a, &b), "second call should reuse the Arc");
        assert_eq!(calls.get(), 1, "unchanged file parsed once");

        // A new mtime + content invalidates the entry. Sleep past the
        // filesystem's mtime resolution so the stamp is guaranteed to differ.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, "two").unwrap();
        let c = cache.get_or_parse(&path, read).unwrap();
        assert_eq!(*c, "two");
        assert_eq!(calls.get(), 2, "changed file reparsed");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unstattable_path_is_never_cached() {
        let cache: ParseCache<String> = ParseCache::new();
        let missing = Path::new("/nonexistent/sweetpad/does-not-exist");
        let calls = Cell::new(0);
        let read = |p: &Path| -> Result<String, std::io::Error> {
            calls.set(calls.get() + 1);
            fs::read_to_string(p)
        };
        assert!(cache.get_or_parse(missing, read).is_err());
        assert!(cache.get_or_parse(missing, read).is_err());
        assert_eq!(calls.get(), 2, "no stamp → parsed (and failed) every call");
    }
}
