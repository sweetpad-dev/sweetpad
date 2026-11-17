//! Taking an SDK's inputs out of an Xcode archive, in two passes over the
//! decoded stream: the first reads every entry, so a selected hard link can
//! find data stored under a path the SDK doesn't take; the second copies
//! the selected data in stream order. Files are written before symbolic
//! links, and nothing in the archive runs.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use sha2::Digest;

use super::source::{Source, open, read_full};
use super::{Keys, MAX_ENTRIES, aa, cpio, select::selected};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum Kind {
    #[default]
    File,
    Dir,
    Symlink,
    Hardlink,
    Special,
}

/// An archive entry, as both formats describe one.
#[derive(Clone, Debug, Default)]
pub(super) struct Entry {
    pub name: String,
    pub kind: Kind,
    /// The size of the data stored with the entry.
    pub size: u64,
    /// Where that data starts in the decoded stream.
    pub offset: u64,
    pub mode: u32,
    /// A symbolic link's target, or the path a hard link names.
    pub link: Option<String>,
    /// CPIO's hard-link group: the entries share the data of one of them.
    pub group: Option<(u64, u64, u64)>,
    /// Apple Archive's data clusters (see [`Clusters`]).
    pub clusters: Vec<(&'static str, u64)>,
    /// Whether an Apple Archive entry stores its data.
    pub has_data: bool,
    /// An Apple Archive file's size, when it says.
    pub logical_size: Option<u64>,
    /// The SHA-256 of the stored data, when an Apple Archive entry needs it.
    pub content_hash: Option<[u8; 32]>,
    /// The SHA-256 an Apple Archive entry declares.
    pub expected_hash: Option<[u8; 32]>,
}

/// `name` as a relative path with no `.` parts, refusing what could leave
/// the folder it's written into.
pub(super) fn clean_name(name: &str) -> Result<String, String> {
    let unsafe_name = || format!("unsafe Xcode archive path: {name:?}");
    if name.contains('\0') || name.starts_with('/') {
        return Err(unsafe_name());
    }
    let mut parts = Vec::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(unsafe_name()),
            part => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

/// Progress lines, at most one every ten seconds unless forced.
struct Reporter<'p> {
    progress: Option<&'p mut dyn FnMut(&str)>,
    next: Instant,
}

impl Reporter<'_> {
    fn report(&mut self, stage: &str, decoded: u64, force: bool) {
        let now = Instant::now();
        if let Some(progress) = self.progress.as_mut()
            && (force || now >= self.next)
        {
            progress(&format!(
                "{stage}: {:.1} GiB decoded",
                decoded as f64 / f64::from(1u32 << 30)
            ));
            self.next = now + Duration::from_secs(10);
        }
    }
}

/// Take the SDK's inputs out of the archive at `path` into
/// `destination/Xcode.app`, which mustn't exist yet; the app's path.
#[allow(clippy::too_many_lines)] // the two passes, with every check between them in order
pub(crate) fn extract(
    path: &Path,
    destination: &Path,
    keys: &Keys,
    progress: Option<&mut dyn FnMut(&str)>,
) -> Result<PathBuf, String> {
    let mut reporter = Reporter {
        progress,
        next: Instant::now() + Duration::from_secs(10),
    };
    let (stream, is_cpio) = decoded(path, keys)?;
    let mut entries: Vec<Entry> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut groups: HashMap<(u64, u64, u64), usize> = HashMap::new();
    let mut apps: HashSet<String> = HashSet::new();
    let decoded_size = {
        let mut report = |pos: u64| reporter.report("Reading archive metadata", pos, false);
        let mut source = Source::new(stream, Some(&mut report));
        let mut add = |entry: Entry| -> Result<(), String> {
            if entries.len() >= MAX_ENTRIES {
                return Err("Xcode archive entry count exceeds the limit".into());
            }
            if selected(&entry.name).is_some() {
                apps.insert(entry.name.split('/').next().unwrap_or("").to_string());
            }
            let at = match index.get(&entry.name) {
                Some(&at) if entries[at].kind == Kind::Dir && entry.kind == Kind::Dir => {
                    entries[at] = entry;
                    at
                }
                Some(_) => return Err(format!("duplicate Xcode archive path: {}", entry.name)),
                None => {
                    index.insert(entry.name.clone(), entries.len());
                    entries.push(entry);
                    entries.len() - 1
                }
            };
            let entry = &entries[at];
            if let Some(group) = entry.group
                && entry.size > 0
            {
                let first = *groups.entry(group).or_insert(at);
                if entries[first].size != entry.size {
                    return Err("inconsistent XIP hard link sizes".into());
                }
            }
            Ok(())
        };
        if is_cpio {
            cpio::entries(&mut source, &mut add)?;
        } else {
            aa::entries(&mut source, &mut add)?;
        }
        source.drain(false)?;
        source.pos
    };
    reporter.report("Read archive metadata", decoded_size, true);
    if apps.len() != 1 {
        return Err("Xcode archive must contain one app with SDK inputs".into());
    }
    let clusters = Clusters::new(&entries)?;

    // The data of a file or hard link: through hard links to the file they
    // name, then to the first of its CPIO group or its cluster's provider.
    let data_entry = |entry: &Entry| -> Result<(u64, u64), String> {
        let mut at = None;
        let mut visited = HashSet::new();
        let mut current = entry;
        while current.kind == Kind::Hardlink {
            if !visited.insert(current.name.clone()) {
                return Err("cyclic Apple Archive hard link".into());
            }
            let target = clean_name(current.link.as_deref().unwrap_or(""))?;
            let &next = index
                .get(&target)
                .ok_or("missing Apple Archive hard link target")?;
            at = Some(next);
            current = &entries[next];
        }
        let mut current = at.map_or(entry, |at| &entries[at]);
        if current.kind != Kind::File {
            return Err("archive hard link target is not a regular file".into());
        }
        if let Some(group) = current.group
            && let Some(&first) = groups.get(&group)
        {
            current = &entries[first];
        }
        clusters.resolve(current, &entries)
    };

    let mut regular: Vec<(u64, u64, PathBuf, u32)> = Vec::new();
    let mut links: Vec<(PathBuf, String)> = Vec::new();
    let mut directories: Vec<(PathBuf, u32)> = Vec::new();
    for entry in &entries {
        let Some(name) = selected(&entry.name) else {
            continue;
        };
        let target = destination.join(&name);
        match entry.kind {
            Kind::File | Kind::Hardlink => {
                let (offset, size) = data_entry(entry)?;
                regular.push((offset, size, target, entry.mode));
            }
            Kind::Dir => directories.push((target, entry.mode)),
            Kind::Symlink => {
                let link = entry.link.clone().unwrap_or_default();
                if link.contains('\0') || link.starts_with('/') {
                    return Err("unsafe Xcode symbolic link".into());
                }
                let parent = name.rsplit_once('/').map_or("", |(parent, _)| parent);
                if !normalize(&format!("{parent}/{link}")).starts_with("Xcode.app/") {
                    return Err("Xcode symbolic link escapes the app".into());
                }
                links.push((target, link));
            }
            Kind::Special => {
                return Err("unsupported special file in selected Xcode inputs".into());
            }
        }
    }
    let leaves: HashSet<&Path> = regular
        .iter()
        .map(|(_, _, target, _)| target.as_path())
        .chain(links.iter().map(|(target, _)| target.as_path()))
        .collect();
    let all = regular
        .iter()
        .map(|(_, _, target, _)| target)
        .chain(links.iter().map(|(target, _)| target))
        .chain(directories.iter().map(|(target, _)| target));
    for target in all {
        if target
            .ancestors()
            .skip(1)
            .any(|parent| leaves.contains(parent))
        {
            return Err("conflicting Xcode archive parent path".into());
        }
    }
    let total: u64 = regular.iter().map(|(_, size, _, _)| size).sum();

    let io = |e: std::io::Error, path: &Path| format!("couldn't write {}: {e}", path.display());
    fs::create_dir_all(destination).map_err(|e| io(e, destination))?;
    let app = destination.join("Xcode.app");
    if app.symlink_metadata().is_ok() {
        return Err("extract Xcode inputs into an empty staging directory".into());
    }
    if free_space(destination)? < 2 * total + 512 * 1024 * 1024 {
        return Err("not enough disk space for the extracted Xcode inputs and staged SDK".into());
    }
    for (target, _) in &directories {
        fs::create_dir_all(target).map_err(|e| io(e, target))?;
    }
    // Hard-linked files can come before their data, so copy by data range.
    let mut ranges: BTreeMap<(u64, u64), Vec<(&Path, u32)>> = BTreeMap::new();
    for (offset, size, target, mode) in &regular {
        ranges
            .entry((*offset, *size))
            .or_default()
            .push((target.as_path(), *mode));
    }
    let (stream, _) = decoded(path, keys)?;
    let decoded_size = {
        let mut report = |pos: u64| reporter.report("Extracting SDK inputs", pos, false);
        let mut source = Source::new(stream, Some(&mut report));
        let mut buf = vec![0u8; 1 << 20];
        for ((offset, size), targets) in &ranges {
            let gap = offset
                .checked_sub(source.pos)
                .ok_or("invalid archive offset")?;
            source.skip(gap)?;
            let (target, mode) = targets[0];
            make_parent(target)?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .map_err(|e| io(e, target))?;
            let mut remaining = *size;
            while remaining > 0 {
                let n = remaining.min(buf.len() as u64) as usize;
                source.exact_into(&mut buf[..n])?;
                output.write_all(&buf[..n]).map_err(|e| io(e, target))?;
                remaining -= n as u64;
            }
            drop(output);
            set_mode(target, mode)?;
            // Copies, so each keeps its own mode.
            for (alias, mode) in &targets[1..] {
                make_parent(alias)?;
                fs::copy(target, alias).map_err(|e| io(e, alias))?;
                set_mode(alias, *mode)?;
            }
        }
        source.drain(false)?;
        source.pos
    };
    reporter.report("Verified complete archive", decoded_size, true);
    for (target, link) in &links {
        make_parent(target)?;
        std::os::unix::fs::symlink(link, target).map_err(|e| io(e, target))?;
    }
    let root = resolve(&app).ok_or("Xcode symbolic link contains a cycle")?;
    for (target, _) in &links {
        if !resolve(target).is_some_and(|resolved| resolved.starts_with(&root)) {
            return Err("Xcode symbolic link resolves outside the app or contains a cycle".into());
        }
    }
    for (target, mode) in directories.iter().rev() {
        set_mode(target, mode | 0o700)?;
    }
    Ok(app)
}

/// The archive's decoded stream, and whether it's CPIO rather than Apple
/// Archive.
fn decoded(path: &Path, keys: &Keys) -> Result<(Box<dyn Read>, bool), String> {
    let mut stream = open(path, keys)?;
    let mut head = [0u8; 6];
    let n = read_full(&mut stream, &mut head).map_err(|e| e.to_string())?;
    let prefix = head[..n].to_vec();
    let is_cpio = prefix.starts_with(b"0707");
    Ok((Box::new(Cursor::new(prefix).chain(stream)), is_cpio))
}

fn make_parent(path: &Path) -> Result<(), String> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| format!("couldn't create {}: {e}", parent.display()))
}

fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| format!("couldn't set the mode of {}: {e}", path.display()))
}

pub(crate) fn free_space(path: &Path) -> Result<u64, String> {
    use std::os::unix::ffi::OsStrExt as _;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: `c` is a valid C string and `stat` a zeroed struct statvfs
    // fills.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &raw mut stat) } != 0 {
        return Err(format!(
            "couldn't read the free space of {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    #[allow(
        clippy::useless_conversion,
        reason = "the field types differ by platform"
    )]
    Ok(u64::from(stat.f_bavail) * u64::from(stat.f_frsize))
}

/// `path` with `.` and `..` worked out lexically, keeping the `..` that
/// climb above where it starts.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// `path` with every symbolic link in it followed, as far as the links
/// lead: a dangling link resolves to where it points. `None` for a cycle.
pub(crate) fn resolve(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    let parts = |path: &Path| -> Vec<OsString> {
        path.components()
            .filter_map(|c| match c {
                Component::Normal(name) => Some(name.to_os_string()),
                Component::ParentDir => Some(OsString::from("..")),
                _ => None,
            })
            .collect()
    };
    let mut queue: VecDeque<OsString> = parts(&absolute).into();
    let mut out = PathBuf::from("/");
    let mut hops = 0;
    while let Some(part) = queue.pop_front() {
        if part == ".." {
            out.pop();
            continue;
        }
        out.push(&part);
        if let Ok(target) = fs::read_link(&out) {
            hops += 1;
            if hops > 64 {
                return None;
            }
            out.pop();
            if target.is_absolute() {
                out = PathBuf::from("/");
            }
            for part in parts(&target).into_iter().rev() {
                queue.push_front(part);
            }
        }
    }
    Some(out)
}

/// Apple Archive's data clusters: entries sharing an `HLC` (hard link),
/// `CLC` (clone) or `SLC` (same data) identifier share one entry's data,
/// and with deduplication that entry can be outside the SDK. An entry can
/// be in several clusters, which joins them.
struct Clusters {
    parents: HashMap<(&'static str, u64), (&'static str, u64)>,
    ranks: HashMap<(&'static str, u64), u32>,
    /// Each cluster's entry with data, by index.
    providers: HashMap<(&'static str, u64), usize>,
}

impl Clusters {
    fn new(entries: &[Entry]) -> Result<Self, String> {
        let mut this = Self {
            parents: HashMap::new(),
            ranks: HashMap::new(),
            providers: HashMap::new(),
        };
        for entry in entries {
            for key in &entry.clusters {
                this.parents.entry(*key).or_insert(*key);
                this.ranks.entry(*key).or_insert(0);
            }
            for key in entry.clusters.iter().skip(1) {
                this.union(entry.clusters[0], *key);
            }
        }
        for (at, entry) in entries.iter().enumerate() {
            let Some(first) = entry.clusters.first() else {
                continue;
            };
            if !entry.has_data {
                continue;
            }
            let root = this.find(*first);
            let previous = &entries[*this.providers.entry(root).or_insert(at)];
            if previous.size != entry.size || previous.content_hash != entry.content_hash {
                return Err("conflicting Apple Archive shared file data".into());
            }
        }
        Ok(this)
    }

    fn find(&mut self, key: (&'static str, u64)) -> (&'static str, u64) {
        let mut root = key;
        while self.parents[&root] != root {
            root = self.parents[&root];
        }
        let mut key = key;
        while self.parents[&key] != root {
            let parent = self.parents[&key];
            self.parents.insert(key, root);
            key = parent;
        }
        root
    }

    fn union(&mut self, left: (&'static str, u64), right: (&'static str, u64)) {
        let (mut left, mut right) = (self.find(left), self.find(right));
        if left == right {
            return;
        }
        if self.ranks[&left] < self.ranks[&right] {
            std::mem::swap(&mut left, &mut right);
        }
        self.parents.insert(right, left);
        if self.ranks[&left] == self.ranks[&right] {
            *self.ranks.get_mut(&left).unwrap() += 1;
        }
    }

    /// Where `entry`'s data is in the stream: its own, or its cluster's.
    fn resolve(&self, entry: &Entry, entries: &[Entry]) -> Result<(u64, u64), String> {
        let Some(first) = entry.clusters.first() else {
            return Ok((entry.offset, entry.size));
        };
        let mut root = *first;
        while self.parents[&root] != root {
            root = self.parents[&root];
        }
        let (provider, content_hash) = match self.providers.get(&root) {
            Some(&at) => (&entries[at], entries[at].content_hash),
            None if entry.logical_size == Some(0) => {
                (entry, Some(<[u8; 32]>::from(sha2::Sha256::digest(b""))))
            }
            None => return Err("missing Apple Archive shared file data".into()),
        };
        if entry.logical_size.is_some_and(|size| size != provider.size) {
            return Err("Apple Archive shared file size differs from its declared size".into());
        }
        if entry.expected_hash.is_some() && content_hash != entry.expected_hash {
            return Err("Apple Archive shared file checksum failed".into());
        }
        Ok((provider.offset, provider.size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_stay_inside_the_folder() {
        assert_eq!(clean_name("./a//b/./c/").unwrap(), "a/b/c");
        assert_eq!(clean_name("").unwrap(), "");
        for name in ["/etc/passwd", "a/../../b", "..", "a\0b"] {
            assert!(clean_name(name).is_err(), "{name}");
        }
    }

    #[test]
    fn a_dangling_link_resolves_to_where_it_points_and_a_cycle_to_none() {
        let dir = std::env::temp_dir().join(format!("sweetpad-resolve-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("app/a")).unwrap();
        std::os::unix::fs::symlink("../missing", dir.join("app/a/dangling")).unwrap();
        std::os::unix::fs::symlink("loop2", dir.join("app/loop1")).unwrap();
        std::os::unix::fs::symlink("loop1", dir.join("app/loop2")).unwrap();
        let root = resolve(&dir.join("app")).unwrap();
        assert_eq!(
            resolve(&dir.join("app/a/dangling")).unwrap(),
            root.join("missing")
        );
        assert_eq!(resolve(&dir.join("app/loop1")), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
