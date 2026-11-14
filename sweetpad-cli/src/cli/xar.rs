//! Reading XAR archives, the container of a flat installer package (`.pkg`).
//!
//! A XAR file is a 28-byte big-endian header, a zlib-compressed XML table of
//! contents that names every file and where its bytes sit, and a heap of
//! those bytes after it.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use sweetpad_lib::xcscheme::Element;

/// An opened archive: its table of contents and where its heap starts.
pub(crate) struct Xar {
    path: PathBuf,
    toc: Element,
    heap: u64,
}

impl Xar {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path).map_err(|e| format!("can't open {}: {e}", path.display()))?;
        let mut header = [0u8; 28];
        file.read_exact(&mut header)
            .map_err(|_| format!("{} is too short to be a package", path.display()))?;
        if &header[..4] != b"xar!" {
            return Err(format!("{} isn't a flat package (no XAR header)", path.display()));
        }
        let header_size = u64::from(u16::from_be_bytes([header[4], header[5]]));
        let compressed = u64::from_be_bytes(header[8..16].try_into().unwrap_or([0; 8]));
        let mut raw = vec![0u8; usize::try_from(compressed).map_err(|e| e.to_string())?];
        file.seek(SeekFrom::Start(header_size)).map_err(|e| e.to_string())?;
        file.read_exact(&mut raw).map_err(|e| e.to_string())?;
        let mut xml = String::new();
        flate2::read::ZlibDecoder::new(raw.as_slice())
            .read_to_string(&mut xml)
            .map_err(|e| format!("{}: the table of contents doesn't inflate: {e}", path.display()))?;
        let toc = sweetpad_lib::xcscheme::parse(&xml)
            .map_err(|e| format!("{}: the table of contents isn't XML: {e}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            toc,
            heap: header_size + compressed,
        })
    }

    /// Every file entry with its path inside the archive (`App.pkg/Bom`).
    pub(crate) fn files(&self) -> Vec<(String, &Element)> {
        fn walk<'a>(element: &'a Element, prefix: &str, out: &mut Vec<(String, &'a Element)>) {
            for file in element.children_named("file") {
                let name = file.child("name").map_or("", |n| n.text.trim());
                let path = if prefix.is_empty() {
                    name.to_string()
                } else {
                    format!("{prefix}/{name}")
                };
                walk(file, &path, out);
                out.push((path, file));
            }
        }
        let mut out = Vec::new();
        if let Some(toc) = self.toc.child("toc") {
            walk(toc, "", &mut out);
        }
        out
    }

    /// The decoded bytes of a file entry.
    pub(crate) fn read(&self, file: &Element) -> Result<Vec<u8>, String> {
        let data = file.child("data").ok_or("the entry has no data")?;
        let number = |name: &str| -> Result<u64, String> {
            data.child(name)
                .and_then(|e| e.text.trim().parse().ok())
                .ok_or_else(|| format!("the entry has no {name}"))
        };
        let (offset, length) = (number("offset")?, number("length")?);
        let mut raw = vec![0u8; usize::try_from(length).map_err(|e| e.to_string())?];
        let mut handle = File::open(&self.path).map_err(|e| e.to_string())?;
        handle
            .seek(SeekFrom::Start(self.heap + offset))
            .map_err(|e| e.to_string())?;
        handle.read_exact(&mut raw).map_err(|e| e.to_string())?;
        match data
            .child("encoding")
            .and_then(|e| e.attr("style"))
            .unwrap_or("application/octet-stream")
        {
            "application/octet-stream" => Ok(raw),
            // XAR's "gzip" is a zlib stream.
            "application/x-gzip" => {
                let mut out = Vec::new();
                flate2::read::ZlibDecoder::new(raw.as_slice())
                    .read_to_end(&mut out)
                    .map_err(|e| format!("an entry doesn't inflate: {e}"))?;
                Ok(out)
            }
            other => Err(format!("an entry is encoded as {other}, which sweetpad can't read")),
        }
    }
}
