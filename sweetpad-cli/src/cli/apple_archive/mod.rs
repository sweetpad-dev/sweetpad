//! Reading Apple's archives without Xcode, for the SDK import on Linux: an
//! Xcode `.xip` (XAR around pbzx around CPIO), an Apple Archive (`.aar`,
//! often inside pbzx), and AEA, Apple's encrypted and signed wrapper
//! (`.aea`). [`extract`] takes the files an SDK needs out of any of them
//! (see [`select`]) without running anything in the archive.
//!
//! Every layer is untrusted: names can't leave the staging folder, links
//! can't point out of the app, every size is bounded, and every checksum
//! and MAC the format carries is checked before the import uses a byte.

// The formats speak in 64-bit sizes and offsets, narrowed only after a
// check against the limits below, on the 64-bit hosts sweetpad runs on.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

mod aa;
mod aea;
pub(crate) mod codecs;
mod cpio;
mod extract;
mod fcs;
mod lzraven;
pub(crate) mod select;
mod source;
#[cfg(test)]
pub(crate) mod tests;
mod xar;

pub(crate) use extract::{extract, free_space, resolve as resolve_links};

/// The most an archive may decode to.
pub(crate) const MAX_OUTPUT: u64 = 64 * 1024 * 1024 * 1024;
/// The most entries an archive may hold.
const MAX_ENTRIES: usize = 1_000_000;

/// Keys for an AEA archive that doesn't carry its own: the archive key
/// (`--aea-key-file`) and the signer's public key
/// (`--aea-public-key-file`), as file contents.
#[derive(Default, Clone)]
pub(crate) struct Keys {
    pub archive: Option<Vec<u8>>,
    pub public: Option<Vec<u8>>,
}

impl Keys {
    /// Read the key files, which are small.
    pub(crate) fn read(
        archive: Option<&std::path::Path>,
        public: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        let load = |path: Option<&std::path::Path>| -> Result<Option<Vec<u8>>, String> {
            let Some(path) = path else { return Ok(None) };
            let size = std::fs::metadata(path)
                .map_err(|e| format!("couldn't read {}: {e}", path.display()))?
                .len();
            if size > 128 * 1024 {
                return Err(format!("AEA key file {} is too large", path.display()));
            }
            std::fs::read(path)
                .map(Some)
                .map_err(|e| format!("couldn't read {}: {e}", path.display()))
        };
        Ok(Self {
            archive: load(archive)?,
            public: load(public)?,
        })
    }
}

/// `bytes` as lowercase hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// What kind of archive `magic`, its first four bytes, starts.
pub(crate) fn kind(magic: &[u8]) -> Option<&'static str> {
    match magic {
        b"xar!" => Some("xip"),
        b"AEA1" => Some("aea"),
        b"AA01" | b"YAA1" | b"pbzx" | b"pbzm" | b"pbze" | b"pbzz" | b"pbz4" => {
            Some("apple-archive")
        }
        _ => None,
    }
}
