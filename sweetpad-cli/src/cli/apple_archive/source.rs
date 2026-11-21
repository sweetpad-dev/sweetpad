//! The decoded byte stream of an archive, and the reader the entry parsers
//! walk it with.

use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::Path;

use super::{Keys, MAX_OUTPUT, codecs};

/// An archive's payload, with its outer layers taken off: XAR's or AEA's
/// around it, then up to three pbzx compression layers.
pub(super) fn open(path: &Path, keys: &Keys) -> Result<Box<dyn Read>, String> {
    let mut file =
        File::open(path).map_err(|e| format!("couldn't open {}: {e}", path.display()))?;
    let mut magic = [0u8; 4];
    let n = read_full(&mut file, &mut magic).map_err(|e| e.to_string())?;
    drop(file);
    let mut raw: Box<dyn Read> = match &magic[..n] {
        b"xar!" => Box::new(super::xar::content(path)?),
        b"AEA1" => Box::new(super::aea::Reader::open(path, keys)?),
        _ => Box::new(BufReader::with_capacity(
            1 << 20,
            File::open(path).map_err(|e| format!("couldn't open {}: {e}", path.display()))?,
        )),
    };
    for _ in 0..3 {
        let mut head = [0u8; 4];
        let n = read_full(&mut raw, &mut head).map_err(|e| e.to_string())?;
        let prefix = head[..n].to_vec();
        let pbz = prefix.starts_with(b"pbz");
        raw = Box::new(Cursor::new(prefix).chain(raw));
        if !pbz {
            break;
        }
        raw = Box::new(Pbz::new(raw)?);
    }
    Ok(raw)
}

/// Read until `buf` is full or the stream ends; the count read.
pub(super) fn read_full(source: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match source.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// A pbzx stream: `pbz` and the codec's letter, a big-endian block size,
/// then blocks, each a decoded and a packed size and the packed bytes.
struct Pbz {
    inner: Box<dyn Read>,
    algorithm: u8,
    block_size: u64,
    block: Vec<u8>,
    at: usize,
    total: u64,
}

impl Pbz {
    fn new(mut inner: Box<dyn Read>) -> Result<Self, String> {
        let mut header = [0u8; 12];
        if read_full(&mut inner, &mut header).map_err(|e| e.to_string())? != 12 {
            return Err("truncated Apple archive compression header".into());
        }
        if !matches!(
            &header[..4],
            b"pbzx" | b"pbzm" | b"pbze" | b"pbzz" | b"pbz4"
        ) {
            return Err("unsupported Apple archive compression container".into());
        }
        let block_size = u64::from_be_bytes(header[4..12].try_into().unwrap());
        if block_size == 0 || block_size > codecs::MAX_BLOCK as u64 {
            return Err("invalid Apple archive compression block size".into());
        }
        Ok(Self {
            inner,
            algorithm: header[3],
            block_size,
            block: Vec::new(),
            at: 0,
            total: 0,
        })
    }

    /// The next block, or `false` at the end of the stream.
    fn next_block(&mut self) -> std::io::Result<bool> {
        let mut sizes = [0u8; 16];
        match read_full(&mut self.inner, &mut sizes)? {
            0 => return Ok(false),
            16 => {}
            _ => return Err(other("truncated Apple archive compression header")),
        }
        let original = u64::from_be_bytes(sizes[..8].try_into().unwrap());
        let packed = u64::from_be_bytes(sizes[8..].try_into().unwrap());
        if original == 0
            || original > self.block_size
            || packed == 0
            || packed > codecs::MAX_BLOCK as u64
        {
            return Err(other("invalid Apple archive compression range"));
        }
        let mut data = vec![0u8; packed as usize];
        if read_full(&mut self.inner, &mut data)? != data.len() {
            return Err(other("truncated Xcode archive"));
        }
        self.block = if packed == original {
            data
        } else {
            codecs::decompress(self.algorithm, &data, original as usize).map_err(other)?
        };
        self.at = 0;
        self.total += original;
        if self.total > MAX_OUTPUT {
            return Err(other("Xcode archive exceeds the 64 GiB decoded-size limit"));
        }
        Ok(true)
    }
}

impl Read for Pbz {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.at == self.block.len() {
            if !self.next_block()? {
                return Ok(0);
            }
        }
        let n = buf.len().min(self.block.len() - self.at);
        buf[..n].copy_from_slice(&self.block[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

pub(super) fn other(message: impl Into<String>) -> std::io::Error {
    std::io::Error::other(message.into())
}

/// The decoded stream as the entry parsers read it: a position, and
/// progress reported as it grows.
pub(super) struct Source<'p> {
    inner: Box<dyn Read>,
    pub pos: u64,
    progress: Option<&'p mut dyn FnMut(u64)>,
}

impl<'p> Source<'p> {
    pub(super) fn new(inner: Box<dyn Read>, progress: Option<&'p mut dyn FnMut(u64)>) -> Self {
        Self {
            inner,
            pos: 0,
            progress,
        }
    }

    fn advanced(&mut self, n: usize) -> Result<(), String> {
        self.pos += n as u64;
        if self.pos > MAX_OUTPUT {
            return Err("Xcode archive exceeds the 64 GiB decoded-size limit".into());
        }
        if let Some(progress) = self.progress.as_mut() {
            progress(self.pos);
        }
        Ok(())
    }

    /// Up to `buf.len()` bytes; fewer only at the end.
    pub(super) fn read_some(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        let n = read_full(&mut self.inner, buf).map_err(|e| e.to_string())?;
        self.advanced(n)?;
        Ok(n)
    }

    pub(super) fn exact(&mut self, size: usize) -> Result<Vec<u8>, String> {
        let mut buf = vec![0u8; size];
        if self.read_some(&mut buf)? != size {
            return Err("truncated Xcode archive".into());
        }
        Ok(buf)
    }

    pub(super) fn exact_into(&mut self, buf: &mut [u8]) -> Result<(), String> {
        if self.read_some(buf)? != buf.len() {
            return Err("truncated Xcode archive".into());
        }
        Ok(())
    }

    pub(super) fn skip(&mut self, mut size: u64) -> Result<(), String> {
        let mut buf = vec![0u8; 1 << 20];
        while size > 0 {
            let n = size.min(buf.len() as u64) as usize;
            self.exact_into(&mut buf[..n])?;
            size -= n as u64;
        }
        Ok(())
    }

    /// Read to the end; with `zero_padding`, anything but zeroes is an
    /// error.
    pub(super) fn drain(&mut self, zero_padding: bool) -> Result<(), String> {
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = self.read_some(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            if zero_padding && buf[..n].iter().any(|b| *b != 0) {
                return Err("unexpected data after the CPIO trailer".into());
            }
        }
    }
}
