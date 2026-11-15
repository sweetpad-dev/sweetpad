//! A component package's payload: the app as it installs, at `./<App>.app`
//! under a root folder, written as an odc cpio archive and compressed with
//! gzip, and what the bill of materials records of each path. Paths go
//! parent first with each folder's entries in byte order, every one owned
//! by root.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::cli::codesign::Sha1;

/// One path of the payload.
pub(super) struct Entry {
    /// `.`, then `./<App>.app/…`.
    pub path: String,
    /// The bill of materials' id of the parent folder, from 1 for the root,
    /// and 0 for the root itself.
    pub parent: u32,
    pub name: String,
    pub kind: Kind,
    /// The permission bits.
    pub mode: u16,
    pub mtime: u32,
    /// The file's size, the link's target length, or as much as a folder
    /// takes on APFS: 32 bytes for each entry and for `.` and `..`.
    pub size: u64,
    /// `cksum`'s CRC of a file or of a link's target.
    pub checksum: u32,
}

pub(super) enum Kind {
    Root,
    Folder,
    File { slices: Option<Slices> },
    Link { target: String },
}

/// The architectures of a Mach-O file.
pub(super) struct Slices {
    /// A universal file, as against one architecture's.
    pub universal: bool,
    pub list: Vec<Slice>,
}

pub(super) struct Slice {
    pub cputype: u32,
    pub subtype: u32,
    pub size: u32,
    pub checksum: u32,
}

/// What [`write`] wrote.
pub(super) struct Payload {
    pub entries: Vec<Entry>,
    pub sha1: [u8; 20],
    pub length: u64,
}

impl Payload {
    /// The bytes the installed paths take, as `installKBytes` counts them.
    pub fn install_bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }
}

/// Write `app` as the payload's one top-level item to `out`.
pub(super) fn write(app: &Path, out: &Path) -> Result<Payload, String> {
    let failed = |e: std::io::Error| format!("couldn't write {}: {e}", out.display());
    let file = File::create(out).map_err(failed)?;
    let hashing = Hashing {
        inner: BufWriter::new(file),
        sha1: Sha1::default(),
        length: 0,
    };
    let gzip = flate2::GzBuilder::new()
        .operating_system(3)
        .write(hashing, flate2::Compression::default());
    let mut walk = Walk {
        cpio: Cpio::new(gzip),
        entries: Vec::new(),
    };
    let name = file_name(app)?;
    let metadata =
        fs::metadata(app).map_err(|e| format!("couldn't read {}: {e}", app.display()))?;
    walk.entries.push(Entry {
        path: ".".to_string(),
        parent: 0,
        name: ".".to_string(),
        kind: Kind::Root,
        mode: 0o755,
        mtime: mtime(&metadata),
        size: 0,
        checksum: 0,
    });
    walk.cpio
        .entry(".", 0o040_755, mtime(&metadata), 3, &[])
        .map_err(failed)?;
    walk.visit(app, format!("./{name}"), name, 1)?;
    let gzip = walk.cpio.finish().map_err(failed)?;
    let mut hashing = gzip.finish().map_err(failed)?;
    hashing.inner.flush().map_err(failed)?;
    Ok(Payload {
        entries: walk.entries,
        sha1: hashing.sha1.finish(),
        length: hashing.length,
    })
}

struct Walk<W: Write> {
    cpio: Cpio<W>,
    entries: Vec<Entry>,
}

impl<W: Write> Walk<W> {
    fn visit(
        &mut self,
        path: &Path,
        archived: String,
        name: String,
        parent: u32,
    ) -> Result<(), String> {
        let unreadable = |e: std::io::Error| format!("couldn't read {}: {e}", path.display());
        let failed = |e: std::io::Error| format!("couldn't write the payload: {e}");
        let metadata = fs::symlink_metadata(path).map_err(unreadable)?;
        let time = mtime(&metadata);
        if metadata.is_symlink() {
            let target = fs::read_link(path).map_err(unreadable)?;
            let target = target
                .to_str()
                .ok_or_else(|| format!("{}: the link's target isn't UTF-8", path.display()))?
                .to_string();
            self.cpio
                .entry(&archived, 0o120_755, time, 1, target.as_bytes())
                .map_err(failed)?;
            self.entries.push(Entry {
                path: archived,
                parent,
                name,
                mode: 0o755,
                mtime: time,
                size: target.len() as u64,
                checksum: cksum(target.as_bytes()),
                kind: Kind::Link { target },
            });
        } else if metadata.is_dir() {
            let mut children = Vec::new();
            for child in fs::read_dir(path).map_err(unreadable)? {
                let child = child.map_err(unreadable)?;
                let name = child.file_name().into_string().map_err(|name| {
                    format!("{}: the name isn't UTF-8", path.join(name).display())
                })?;
                children.push(name);
            }
            children.sort();
            // A folder's link count as APFS gives it: one for each entry,
            // and for `.` and `..`.
            let nlink = u32::try_from(2 + children.len()).unwrap_or(u32::MAX);
            self.cpio
                .entry(&archived, 0o040_755, time, nlink, &[])
                .map_err(failed)?;
            self.entries.push(Entry {
                path: archived.clone(),
                parent,
                name,
                kind: Kind::Folder,
                mode: 0o755,
                mtime: time,
                size: 32 * (children.len() as u64 + 2),
                checksum: 0,
            });
            let id = u32::try_from(self.entries.len()).unwrap_or(u32::MAX);
            for child in children {
                self.visit(&path.join(&child), format!("{archived}/{child}"), child, id)?;
            }
        } else if metadata.is_file() {
            let data = fs::read(path).map_err(unreadable)?;
            let mode = if metadata.mode() & 0o111 == 0 {
                0o644
            } else {
                0o755
            };
            self.cpio
                .entry(&archived, 0o100_000 | u32::from(mode), time, 1, &data)
                .map_err(failed)?;
            self.entries.push(Entry {
                path: archived,
                parent,
                name,
                kind: Kind::File {
                    slices: slices(&data),
                },
                mode,
                mtime: time,
                size: data.len() as u64,
                checksum: cksum(&data),
            });
        } else {
            return Err(format!(
                "{} isn't a file, folder or symbolic link, which a package can't hold",
                path.display()
            ));
        }
        Ok(())
    }
}

fn file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{} has no UTF-8 name", path.display()))
}

fn mtime(metadata: &fs::Metadata) -> u32 {
    u32::try_from(metadata.mtime().max(0)).unwrap_or(u32::MAX)
}

/// The architectures of `data`, when it is Mach-O.
pub(super) fn slices(data: &[u8]) -> Option<Slices> {
    let word = |at: usize, big: bool| {
        let bytes: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    };
    match word(0, true)? {
        magic @ (0xfeed_face | 0xfeed_facf | 0xcefa_edfe | 0xcffa_edfe) => {
            let big = magic >> 24 == 0xfe;
            Some(Slices {
                universal: false,
                list: vec![Slice {
                    cputype: word(4, big)?,
                    subtype: word(8, big)?,
                    size: u32::try_from(data.len()).ok()?,
                    checksum: cksum(data),
                }],
            })
        }
        magic @ (0xcafe_babe | 0xcafe_babf) => {
            let wide = magic == 0xcafe_babf;
            let count = usize::try_from(word(4, true)?).ok()?;
            // A Java class file starts with the same magic, then a version
            // of at least 45.
            if count == 0 || count > 16 {
                return None;
            }
            let mut list = Vec::with_capacity(count);
            for index in 0..count {
                let at = 8 + index * if wide { 32 } else { 20 };
                let (offset, size) = if wide {
                    let long = |at: usize| {
                        Some(u64::from_be_bytes(data.get(at..at + 8)?.try_into().ok()?))
                    };
                    (long(at + 8)?, long(at + 16)?)
                } else {
                    (
                        u64::from(word(at + 8, true)?),
                        u64::from(word(at + 12, true)?),
                    )
                };
                let start = usize::try_from(offset).ok()?;
                let end = start.checked_add(usize::try_from(size).ok()?)?;
                list.push(Slice {
                    cputype: word(at, true)?,
                    subtype: word(at + 4, true)?,
                    size: u32::try_from(size).ok()?,
                    checksum: cksum(data.get(start..end)?),
                });
            }
            Some(Slices {
                universal: true,
                list,
            })
        }
        _ => None,
    }
}

/// The CRC `cksum` prints: CRC-32 with the POSIX polynomial over the data
/// and then its length.
pub(super) fn cksum(data: &[u8]) -> u32 {
    #[allow(clippy::cast_possible_truncation)] // i < 256, in a const block
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut crc = (i as u32) << 24;
            let mut bit = 0;
            while bit < 8 {
                crc = if crc & 0x8000_0000 == 0 {
                    crc << 1
                } else {
                    (crc << 1) ^ 0x04c1_1db7
                };
                bit += 1;
            }
            table[i] = crc;
            i += 1;
        }
        table
    };
    let step = |crc: u32, byte: u8| (crc << 8) ^ TABLE[usize::from((crc >> 24) as u8 ^ byte)];
    let mut crc = data.iter().fold(0, |crc, byte| step(crc, *byte));
    let mut length = data.len();
    while length != 0 {
        crc = step(crc, length.to_le_bytes()[0]);
        length >>= 8;
    }
    !crc
}

/// An odc cpio archive, as `pkgbuild` writes payloads: inodes numbered in
/// order, every entry owned by root, padded to 512 bytes.
struct Cpio<W: Write> {
    out: W,
    inode: u32,
    written: u64,
}

impl<W: Write> Cpio<W> {
    fn new(out: W) -> Self {
        Self {
            out,
            inode: 0,
            written: 0,
        }
    }

    fn entry(
        &mut self,
        name: &str,
        mode: u32,
        mtime: u32,
        nlink: u32,
        data: &[u8],
    ) -> std::io::Result<()> {
        if data.len() as u64 > 0o777_7777_7777 {
            return Err(std::io::Error::other(format!(
                "{name} is too large for a package's payload"
            )));
        }
        let header = format!(
            "070707{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:011o}{:06o}{:011o}",
            0,
            self.inode % 0o100_0000,
            mode,
            0,
            0,
            nlink,
            0,
            mtime,
            name.len() + 1,
            data.len()
        );
        self.inode += 1;
        for part in [header.as_bytes(), name.as_bytes(), &[0], data] {
            self.out.write_all(part)?;
            self.written += part.len() as u64;
        }
        Ok(())
    }

    fn finish(mut self) -> std::io::Result<W> {
        self.entry("TRAILER!!!", 0, 0, 1, &[])?;
        let padding = self.written.next_multiple_of(512) - self.written;
        self.out
            .write_all(&vec![0; usize::try_from(padding).unwrap_or(0)])?;
        Ok(self.out)
    }
}

/// A writer that hashes and counts what passes through it.
struct Hashing<W: Write> {
    inner: W,
    sha1: Sha1,
    length: u64,
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.sha1.update(&buf[..written]);
        self.length += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
