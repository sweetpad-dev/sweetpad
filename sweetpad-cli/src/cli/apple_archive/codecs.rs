//! The block codecs of Apple's compression containers (pbzx and AEA),
//! named by the letter Apple's tools use: `-` stored, `x` LZMA (xz), `z`
//! zlib, `4` LZ4, `e` LZFSE, `f` LZVN and `m` LZRAVEN. Each block declares
//! its decoded size, and a block that decodes to any other size is an
//! error, so an untrusted archive can't grow a block past the limit.

use std::io::Write;

/// The largest block, packed or decoded, an archive may declare.
pub(crate) const MAX_BLOCK: usize = 64 * 1024 * 1024;

/// Decode one block of `algorithm` that decodes to `expected` bytes.
pub(crate) fn decompress(algorithm: u8, data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    if expected > MAX_BLOCK || data.len() > MAX_BLOCK {
        return Err("archive compression block exceeds the memory limit".into());
    }
    let out = match algorithm {
        b'-' => data.to_vec(),
        b'x' => {
            let mut out = Bounded::new(expected);
            lzma_rs::xz_decompress(&mut &data[..], &mut out)
                .map_err(|_| "invalid LZMA archive block".to_string())?;
            out.data
        }
        b'z' => zlib(data, expected)?,
        b'4' => apple_lz4(data, expected)?,
        b'e' => lzfse(data, expected)?,
        // LZFSE's decoder reads LZVN in its own block framing.
        b'f' => {
            let mut framed = Vec::with_capacity(data.len() + 16);
            framed.extend_from_slice(b"bvxn");
            framed.extend_from_slice(&(expected as u32).to_le_bytes());
            framed.extend_from_slice(&(data.len() as u32).to_le_bytes());
            framed.extend_from_slice(data);
            framed.extend_from_slice(b"bvx$");
            lzfse(&framed, expected)?
        }
        b'm' => super::lzraven::decode(data, expected + 1)
            .map_err(|e| format!("invalid LZRAVEN stream ({e})"))?,
        other => {
            return Err(format!(
                "unsupported Apple archive compression '{}'",
                char::from(other).escape_default()
            ));
        }
    };
    if out.len() != expected {
        return Err("archive block output differs from its declared size".into());
    }
    Ok(out)
}

/// A writer that refuses more than one byte past `limit`, so a decoder that
/// overshoots is caught without holding the overshoot.
struct Bounded {
    data: Vec<u8>,
    limit: usize,
}

impl Bounded {
    fn new(limit: usize) -> Self {
        Self {
            data: Vec::with_capacity(limit),
            limit,
        }
    }
}

impl Write for Bounded {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.data.len() + buf.len() > self.limit + 1 {
            return Err(std::io::Error::other("block exceeds its declared size"));
        }
        self.data.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A zlib stream that ends exactly at the end of `data`.
fn zlib(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut decoder = flate2::Decompress::new(true);
    let mut out = Vec::with_capacity(expected + 1);
    let status = decoder
        .decompress_vec(data, &mut out, flate2::FlushDecompress::Finish)
        .map_err(|_| "invalid zlib archive block".to_string())?;
    if status != flate2::Status::StreamEnd || decoder.total_in() != data.len() as u64 {
        return Err("invalid zlib stream boundary".into());
    }
    Ok(out)
}

fn lzfse(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(expected);
    lzfse_rust::decode_bytes(data, &mut out).map_err(|e| format!("invalid LZFSE block ({e})"))?;
    Ok(out)
}

/// Apple's LZ4 framing: `bv4-` stored and `bv41` LZ4 blocks, each able to
/// match into the 64 KiB before it, ended by `bv4$`.
fn apple_lz4(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let word = |at: usize| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let magic = &data[pos..pos + 4];
        pos += 4;
        if magic == b"bv4$" {
            if pos != data.len() || out.len() != expected {
                return Err("invalid LZ4 stream length".into());
            }
            return Ok(out);
        }
        if pos + 4 > data.len() {
            break;
        }
        let size = word(pos) as usize;
        pos += 4;
        if size > expected - out.len() {
            return Err("LZ4 stream exceeds its declared size".into());
        }
        let packed = match magic {
            b"bv4-" => size,
            b"bv41" if pos + 4 <= data.len() => {
                pos += 4;
                word(pos - 4) as usize
            }
            _ => return Err("unsupported LZ4 stream block".into()),
        };
        if packed > data.len() - pos {
            return Err("truncated LZ4 stream block".into());
        }
        let block = &data[pos..pos + packed];
        pos += packed;
        if magic == b"bv4-" {
            out.extend_from_slice(block);
        } else {
            let dictionary = out.len().saturating_sub(65536);
            let decoded = lz4_flex::block::decompress_with_dict(block, size, &out[dictionary..])
                .map_err(|_| "invalid LZ4 block".to_string())?;
            if decoded.len() != size {
                return Err("LZ4 output size differs from its header".into());
            }
            out.extend_from_slice(&decoded);
        }
    }
    Err("missing LZ4 stream terminator".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_lengths_are_rejected_before_decoding() {
        for algorithm in *b"emxz4f-" {
            assert!(decompress(algorithm, b"bad", MAX_BLOCK + 1).is_err());
            assert!(decompress(algorithm, b"bad", 3).is_err() || algorithm == b'-');
        }
        assert!(decompress(b'q', b"", 0).is_err());
    }

    #[test]
    fn zlib_must_end_where_its_block_does() {
        use flate2::write::ZlibEncoder;
        let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"hello hello hello").unwrap();
        let mut packed = encoder.finish().unwrap();
        assert_eq!(decompress(b'z', &packed, 17).unwrap(), b"hello hello hello");
        assert!(decompress(b'z', &packed, 16).is_err());
        packed.push(0);
        assert!(decompress(b'z', &packed, 17).is_err());
    }
}
