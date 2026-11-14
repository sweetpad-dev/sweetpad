//! A component package's bill of materials, as `pkgbuild` writes it: each
//! path of the payload with its mode, owner, size, time and checksum, a
//! Mach-O file's architectures with the size and checksum of each, and an
//! empty hard-link list for each path. `lsbom` prints it.

use std::collections::BTreeMap;

use super::payload::{Entry, Kind};
use crate::cli::bom::Bom;

/// How many entries a tree node holds, well inside what a 4096-byte node
/// fits.
const NODE_CAPACITY: usize = 256;

pub(super) fn bill(entries: &[Entry]) -> Result<Vec<u8>, String> {
    let mut bom = Bom::default();
    let info = bom.add(Vec::new());
    let paths = Tree::reserve(&mut bom, entries.len(), 4096);
    let links = Tree::reserve(&mut bom, entries.len(), 4096);
    let volumes = bom.add(Vec::new());
    let volume_tree = Tree::reserve(&mut bom, 0, 128);
    let sizes = Tree::reserve(&mut bom, 0, 4096);

    let mut keyed = Vec::with_capacity(entries.len());
    let mut records = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let id = count(index + 1);
        let record = bom.add(path_info(entry)?);
        let mut key = entry.parent.to_be_bytes().to_vec();
        key.extend_from_slice(entry.name.as_bytes());
        key.push(0);
        let file = bom.add(key.clone());
        let path = bom.add([id.to_be_bytes(), record.to_be_bytes()].concat());
        keyed.push((key, path, file));
        records.push(record);
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    let sorted: Vec<(u32, u32)> = keyed.iter().map(|(_, path, file)| (*path, *file)).collect();
    paths.fill(&mut bom, &sorted, entries.len());

    let mut hard_links = Vec::with_capacity(records.len());
    for record in records {
        let list = Tree::reserve(&mut bom, 0, 64);
        list.fill(&mut bom, &[], 0);
        let key = bom.add(record.to_be_bytes().to_vec());
        let value = bom.add(list.header.to_be_bytes().to_vec());
        hard_links.push((value, key));
    }
    links.fill(&mut bom, &hard_links, entries.len());

    let mut volume = [1, volume_tree.header, 0]
        .iter()
        .flat_map(|n: &u32| n.to_be_bytes())
        .collect::<Vec<_>>();
    volume.push(0);
    bom.set(volumes, volume);
    volume_tree.fill(&mut bom, &[], 0);
    sizes.fill(&mut bom, &[], 0);
    bom.set(info, bom_info(entries));

    bom.variable("BomInfo", info);
    bom.variable("Paths", paths.header);
    bom.variable("HLIndex", links.header);
    bom.variable("VIndex", volumes);
    bom.variable("Size64", sizes.header);
    Ok(bom.finish())
}

/// What a path is: its type, architectures, mode, owner, time, size and
/// checksum, then a Mach-O file's slices or a link's target.
fn path_info(entry: &Entry) -> Result<Vec<u8>, String> {
    let (kind, architecture, format) = match &entry.kind {
        // The root folder records nothing of its own.
        Kind::Root => {
            let mut out = vec![2, 1, 0, 1, 0, 0];
            out.extend_from_slice(&[0; 16]);
            out.push(1);
            out.extend_from_slice(&[0; 8]);
            return Ok(out);
        }
        Kind::Folder => (2u8, 0x000f_u16, 0o040_000_u16),
        Kind::File { slices: None } => (1, 0x000f, 0o100_000),
        Kind::File {
            slices: Some(slices),
        } => (1, if slices.universal { 0x200f } else { 0x100f }, 0o100_000),
        Kind::Link { .. } => (3, 0x000f, 0o120_000),
    };
    let size = u32::try_from(entry.size).map_err(|_| {
        format!(
            "{} is 4 GiB or larger, which a package's bill of materials doesn't record",
            entry.path
        )
    })?;
    let mut out = vec![kind, 1];
    out.extend_from_slice(&architecture.to_be_bytes());
    out.extend_from_slice(&(format | entry.mode).to_be_bytes());
    out.extend_from_slice(&[0; 8]); // owner and group: root and wheel
    out.extend_from_slice(&entry.mtime.to_be_bytes());
    out.extend_from_slice(&size.to_be_bytes());
    out.push(1);
    out.extend_from_slice(&entry.checksum.to_be_bytes());
    match &entry.kind {
        Kind::Folder => out.extend_from_slice(&[0; 4]),
        Kind::File { slices: None } => out.extend_from_slice(&[0; 8]),
        Kind::File {
            slices: Some(slices),
        } => {
            out.push(1);
            out.extend_from_slice(&count(slices.list.len()).to_be_bytes());
            for slice in &slices.list {
                for word in [slice.cputype, slice.subtype, slice.size, slice.checksum] {
                    out.extend_from_slice(&word.to_be_bytes());
                }
            }
            out.extend_from_slice(&[0; 8]);
        }
        Kind::Link { target } => {
            out.extend_from_slice(&count(target.len() + 1).to_be_bytes());
            out.extend_from_slice(target.as_bytes());
            out.push(0);
            out.extend_from_slice(&[0; 8]);
        }
        Kind::Root => {}
    }
    Ok(out)
}

/// The path count and the bytes of each architecture, everything that
/// isn't Mach-O counted under none.
fn bom_info(entries: &[Entry]) -> Vec<u8> {
    let mut other = 0u64;
    let mut architectures = BTreeMap::<u32, u64>::new();
    for entry in entries {
        match &entry.kind {
            Kind::Root => {}
            Kind::File {
                slices: Some(slices),
            } => {
                for slice in &slices.list {
                    *architectures.entry(slice.cputype).or_default() += u64::from(slice.size);
                }
            }
            _ => other += entry.size,
        }
    }
    let mut words = vec![1, count(entries.len() + 1), count(architectures.len() + 1)];
    for (cputype, total) in std::iter::once((0, other)).chain(architectures) {
        words.extend([cputype, 0, u32::try_from(total).unwrap_or(u32::MAX), 0]);
    }
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// A B-tree of a bill of materials: a header naming its root, leaves of
/// (value, key) block pairs linked in key order, and branches above them
/// that key each child but the last by its last key.
struct Tree {
    header: u32,
    /// The nodes of each level, leaves first.
    levels: Vec<Vec<u32>>,
    block_size: usize,
}

impl Tree {
    /// Set aside the blocks of a tree of `entries` entries.
    fn reserve(bom: &mut Bom, entries: usize, block_size: usize) -> Self {
        let header = bom.add(Vec::new());
        let mut levels = Vec::new();
        let mut nodes = entries.div_ceil(NODE_CAPACITY).max(1);
        loop {
            levels.push((0..nodes).map(|_| bom.add(Vec::new())).collect::<Vec<_>>());
            if nodes == 1 {
                break;
            }
            nodes = nodes.div_ceil(NODE_CAPACITY);
        }
        Self {
            header,
            levels,
            block_size,
        }
    }

    /// Fill the tree with `entries`, in key order, recording `paths` in
    /// its header.
    fn fill(&self, bom: &mut Bom, entries: &[(u32, u32)], paths: usize) {
        let node = |leaf: bool, count: usize, forward: u32, backward: u32| {
            let mut out = u16::from(leaf).to_be_bytes().to_vec();
            out.extend_from_slice(&u16::try_from(count).unwrap_or(u16::MAX).to_be_bytes());
            out.extend_from_slice(&forward.to_be_bytes());
            out.extend_from_slice(&backward.to_be_bytes());
            out
        };
        let leaves = &self.levels[0];
        let chunks: Vec<&[(u32, u32)]> = if entries.is_empty() {
            vec![&[]]
        } else {
            entries.chunks(NODE_CAPACITY).collect()
        };
        // Each node of the level below, with the last key under it.
        let mut below = Vec::with_capacity(leaves.len());
        for (index, (block, chunk)) in leaves.iter().zip(chunks).enumerate() {
            let forward = leaves.get(index + 1).copied().unwrap_or(0);
            let backward = index.checked_sub(1).map_or(0, |i| leaves[i]);
            let mut out = node(true, chunk.len(), forward, backward);
            for (value, key) in chunk {
                out.extend_from_slice(&value.to_be_bytes());
                out.extend_from_slice(&key.to_be_bytes());
            }
            out.resize(self.block_size, 0);
            bom.set(*block, out);
            below.push((*block, chunk.last().map_or(0, |entry| entry.1)));
        }
        for level in &self.levels[1..] {
            let mut above = Vec::with_capacity(level.len());
            for (block, children) in level.iter().zip(below.chunks(NODE_CAPACITY)) {
                let Some(((last, last_key), keyed)) = children.split_last() else {
                    continue;
                };
                let mut out = node(false, keyed.len(), 0, 0);
                for (child, key) in keyed {
                    out.extend_from_slice(&child.to_be_bytes());
                    out.extend_from_slice(&key.to_be_bytes());
                }
                out.extend_from_slice(&last.to_be_bytes());
                out.resize(self.block_size, 0);
                bom.set(*block, out);
                above.push((*block, *last_key));
            }
            below = above;
        }
        let root = self.levels.last().map_or(0, |level| level[0]);
        let mut header = b"tree".to_vec();
        for word in [1, root, count(self.block_size), count(paths)] {
            header.extend_from_slice(&word.to_be_bytes());
        }
        header.push(0);
        bom.set(self.header, header);
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
