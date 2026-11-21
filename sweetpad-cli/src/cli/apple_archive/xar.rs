//! XAR, the container of an Xcode `.xip`: a header, a zlib-compressed XML
//! table of contents, then a heap. A `.xip` keeps one file in it,
//! `Content`, stored, with its checksum in the table.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use sha1::Digest;

use super::source::{other, read_full};

/// A `.xip`'s `Content`, checked against its checksum as it's read: the
/// last read fails if the bytes don't match.
/// The most the table of contents may be, packed or not.
const TABLE_LIMIT: u64 = 16 * 1024 * 1024;

pub(super) fn content(path: &Path) -> Result<Content, String> {
    let mut file =
        File::open(path).map_err(|e| format!("couldn't open {}: {e}", path.display()))?;
    let file_size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut header = [0u8; 28];
    if read_full(&mut file, &mut header).map_err(|e| e.to_string())? != 28 {
        return Err("truncated XIP header".into());
    }
    let header_size = u64::from(u16::from_be_bytes([header[4], header[5]]));
    let version = u16::from_be_bytes([header[6], header[7]]);
    let packed_size = u64::from_be_bytes(header[8..16].try_into().unwrap());
    let toc_size = u64::from_be_bytes(header[16..24].try_into().unwrap());
    if &header[..4] != b"xar!" || version != 1 || !(28..=4096).contains(&header_size) {
        return Err("unsupported XIP/XAR header".into());
    }
    if packed_size == 0 || packed_size > TABLE_LIMIT || toc_size == 0 || toc_size > TABLE_LIMIT {
        return Err("XIP table exceeds the memory limit".into());
    }
    file.seek(SeekFrom::Start(header_size))
        .map_err(|e| e.to_string())?;
    let mut packed = vec![0u8; packed_size as usize];
    if read_full(&mut file, &mut packed).map_err(|e| e.to_string())? != packed.len() {
        return Err("truncated XIP table".into());
    }
    let toc = toc(&packed, toc_size as usize)?;
    let entry = content_entry(&toc)?;
    let heap = header_size + packed_size;
    if entry.length < 12 || heap + entry.offset + entry.length > file_size {
        return Err("XIP Content range is outside the archive".into());
    }
    file.seek(SeekFrom::Start(heap + entry.offset))
        .map_err(|e| e.to_string())?;
    Ok(Content {
        file: BufReader::with_capacity(1 << 20, file).take(entry.length),
        remaining: entry.length,
        digest: entry.digest,
        expected: entry.expected,
    })
}

/// The table of contents, which must inflate to exactly its declared size.
fn toc(packed: &[u8], size: usize) -> Result<String, String> {
    let mut decoder = flate2::Decompress::new(true);
    let mut out = Vec::with_capacity(size + 1);
    let status = decoder
        .decompress_vec(packed, &mut out, flate2::FlushDecompress::Finish)
        .map_err(|_| "invalid XIP table compression".to_string())?;
    if status != flate2::Status::StreamEnd
        || decoder.total_in() != packed.len() as u64
        || out.len() != size
    {
        return Err("invalid XIP table compression".into());
    }
    String::from_utf8(out).map_err(|_| "invalid XIP table XML".into())
}

struct Entry {
    offset: u64,
    length: u64,
    digest: Hasher,
    expected: String,
}

fn content_entry(toc: &str) -> Result<Entry, String> {
    let root = sweetpad_lib::xcscheme::parse(toc).map_err(|_| "invalid XIP table XML")?;
    let files: Vec<_> = root
        .children_named("toc")
        .flat_map(|toc| toc.children_named("file"))
        .filter(|file| {
            file.child("name")
                .is_some_and(|name| name.text == "Content")
        })
        .collect();
    let [file] = files[..] else {
        return Err("XIP must contain exactly one Content payload".into());
    };
    let data = file.child("data").ok_or("invalid XIP Content metadata")?;
    if let Some(encoding) = data.child("encoding")
        && encoding.attr("style") != Some("application/octet-stream")
    {
        return Err("XIP Content uses unsupported XAR-level compression".into());
    }
    let number = |name: &str| {
        data.child(name)
            .and_then(|n| n.text.trim().parse::<u64>().ok())
    };
    let (Some(offset), Some(length)) = (number("offset"), number("length")) else {
        return Err("XIP Content range is outside the archive".into());
    };
    let checksum = data.child("archived-checksum");
    let digest = match checksum.and_then(|c| c.attr("style")) {
        Some("sha1") => Hasher::Sha1(sha1::Sha1::new()),
        Some("sha256") => Hasher::Sha256(sha2::Sha256::new()),
        Some("sha512") => Hasher::Sha512(sha2::Sha512::new()),
        _ => return Err("XIP Content must have a supported checksum".into()),
    };
    Ok(Entry {
        offset,
        length,
        digest,
        expected: checksum.map_or("", |c| c.text.trim()).to_ascii_lowercase(),
    })
}

enum Hasher {
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
}

impl Hasher {
    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Sha1(h) => h.update(data),
            Self::Sha256(h) => h.update(data),
            Self::Sha512(h) => h.update(data),
        }
    }

    fn hex(&mut self) -> String {
        let bytes = match self {
            Self::Sha1(h) => h.finalize_reset().to_vec(),
            Self::Sha256(h) => h.finalize_reset().to_vec(),
            Self::Sha512(h) => h.finalize_reset().to_vec(),
        };
        super::hex(&bytes)
    }
}

pub(super) struct Content {
    file: std::io::Take<BufReader<File>>,
    remaining: u64,
    digest: Hasher,
    expected: String,
}

impl Read for Content {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let n = self.file.read(buf)?;
        if n == 0 {
            return Err(other("truncated XIP Content"));
        }
        self.digest.update(&buf[..n]);
        self.remaining -= n as u64;
        if self.remaining == 0 && self.digest.hex() != self.expected {
            return Err(other("XIP Content checksum failed"));
        }
        Ok(n)
    }
}
