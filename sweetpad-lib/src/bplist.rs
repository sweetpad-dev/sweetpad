// The bplist file format speaks in u64 offsets/sizes by spec; we narrow them
// to usize after checking against `data.len()`. The cast-truncation lints fire
// constantly here and aren't actionable on the 64-bit hosts this code runs on.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::format_collect,
    clippy::manual_range_patterns,
    clippy::too_many_lines
)]

//! Minimal Apple binary property list (`bplist00`) reader.
//!
//! Parses the variant used by Xcode's `SDKSettings.plist`, `Info.plist` and
//! similar files. Produces values in [`crate::pbxproj::Value`] so the same
//! `Dict / Array / String` tree shape that the OpenStep plist parser returns
//! can be consumed uniformly by the rest of the codebase.
//!
//! Non-string scalars are stringified: `true` → `"YES"`, `false` → `"NO"`,
//! integers and reals via `Display`, dates as their raw f64 timestamp, data
//! blobs as lowercase hex. That matches how xcodebuild surfaces these values
//! in build settings.
//!
//! Supported object kinds: null, false, true, fill, int (1/2/4/8 bytes),
//! real (4/8 bytes), date, data, ASCII string, UTF-16BE string, array, dict.
//! UID and Set markers are accepted but treated as opaque.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use crate::pbxproj::Value;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Invalid(String),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Invalid(s) => write!(f, "invalid binary plist: {s}"),
        }
    }
}

impl std::error::Error for Error {}

pub fn parse_file(path: &Path) -> Result<Value, Error> {
    let data = fs::read(path)?;
    parse(&data)
}

pub fn parse(data: &[u8]) -> Result<Value, Error> {
    if data.len() < 8 + 32 {
        return Err(Error::Invalid("file shorter than header + trailer".into()));
    }
    if !data.starts_with(b"bplist00") {
        return Err(Error::Invalid("missing bplist00 magic".into()));
    }

    let trailer = &data[data.len() - 32..];
    let offset_size = trailer[6] as usize;
    let ref_size = trailer[7] as usize;
    let num_objects = read_be_u64(&trailer[8..16]) as usize;
    let top_object = read_be_u64(&trailer[16..24]) as usize;
    let offset_table_offset = read_be_u64(&trailer[24..32]) as usize;

    if !(1..=8).contains(&offset_size) || !(1..=8).contains(&ref_size) {
        return Err(Error::Invalid(format!(
            "bad sizes: offset={offset_size}, ref={ref_size}"
        )));
    }
    if num_objects == 0 || top_object >= num_objects {
        return Err(Error::Invalid(format!(
            "bad object counts: num={num_objects}, top={top_object}"
        )));
    }
    let table_end = offset_table_offset
        .checked_add(
            num_objects
                .checked_mul(offset_size)
                .ok_or_else(|| Error::Invalid("offset table size overflow".into()))?,
        )
        .ok_or_else(|| Error::Invalid("offset table end overflow".into()))?;
    if table_end > data.len() {
        return Err(Error::Invalid("offset table extends past EOF".into()));
    }

    let ctx = Ctx {
        data,
        offset_size,
        ref_size,
        offset_table_offset,
        num_objects,
    };
    read_object(&ctx, top_object, 0)
}

struct Ctx<'a> {
    data: &'a [u8],
    offset_size: usize,
    ref_size: usize,
    offset_table_offset: usize,
    num_objects: usize,
}

/// Recursion guard. Apple's plists are not deeply nested in practice; 256 is
/// well past anything real and still finite.
const MAX_DEPTH: usize = 256;

fn read_be_u64(b: &[u8]) -> u64 {
    let mut v = 0u64;
    for &byte in b {
        v = (v << 8) | u64::from(byte);
    }
    v
}

fn object_offset(ctx: &Ctx, idx: usize) -> Result<usize, Error> {
    if idx >= ctx.num_objects {
        return Err(Error::Invalid(format!(
            "object ref {idx} out of range ({})",
            ctx.num_objects
        )));
    }
    let pos = ctx.offset_table_offset + idx * ctx.offset_size;
    if pos + ctx.offset_size > ctx.data.len() {
        return Err(Error::Invalid("offset table read out of bounds".into()));
    }
    let off = read_be_u64(&ctx.data[pos..pos + ctx.offset_size]) as usize;
    if off >= ctx.data.len() {
        return Err(Error::Invalid(format!("object offset {off} out of bounds")));
    }
    Ok(off)
}

fn read_object(ctx: &Ctx, idx: usize, depth: usize) -> Result<Value, Error> {
    if depth > MAX_DEPTH {
        return Err(Error::Invalid("recursion depth exceeded".into()));
    }
    let start = object_offset(ctx, idx)?;
    let marker = ctx.data[start];
    let pos = start + 1;
    let high = marker >> 4;
    let low = marker & 0x0F;

    match high {
        0x0 => match marker {
            0x00 | 0x0F => Ok(Value::String(String::new())),
            0x08 => Ok(Value::String("NO".into())),
            0x09 => Ok(Value::String("YES".into())),
            _ => Err(Error::Invalid(format!(
                "unknown 0x0X marker 0x{marker:02X}"
            ))),
        },
        0x1 => {
            // Int. low = log2(byte_count).
            let nbytes = 1usize << low;
            require(ctx, pos, nbytes, "int")?;
            let bytes = &ctx.data[pos..pos + nbytes];
            // Apple uses signed ints for 8-byte; smaller widths are unsigned.
            let s = if nbytes == 16 {
                // 128-bit int, very rare. Treat as unsigned big-endian decimal.
                u128_to_string(bytes)
            } else if nbytes == 8 {
                let v = read_be_u64(bytes) as i64;
                v.to_string()
            } else {
                read_be_u64(bytes).to_string()
            };
            Ok(Value::String(s))
        }
        0x2 => {
            let nbytes = 1usize << low;
            require(ctx, pos, nbytes, "real")?;
            let s = match nbytes {
                4 => {
                    let arr: [u8; 4] = ctx.data[pos..pos + 4].try_into().unwrap();
                    f32::from_be_bytes(arr).to_string()
                }
                8 => {
                    let arr: [u8; 8] = ctx.data[pos..pos + 8].try_into().unwrap();
                    f64::from_be_bytes(arr).to_string()
                }
                _ => return Err(Error::Invalid(format!("bad real width {nbytes}"))),
            };
            Ok(Value::String(s))
        }
        0x3 => {
            // Date — 8-byte big-endian f64 (Apple epoch). Stringify the raw value.
            require(ctx, pos, 8, "date")?;
            let arr: [u8; 8] = ctx.data[pos..pos + 8].try_into().unwrap();
            Ok(Value::String(f64::from_be_bytes(arr).to_string()))
        }
        0x4 => {
            // Data
            let (len, after) = read_size(ctx, pos, low)?;
            require(ctx, after, len, "data")?;
            let hex: String = ctx.data[after..after + len]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            Ok(Value::String(hex))
        }
        0x5 => {
            // ASCII string
            let (len, after) = read_size(ctx, pos, low)?;
            require(ctx, after, len, "ascii string")?;
            let slice = &ctx.data[after..after + len];
            // bplist ASCII strings are technically Latin-1; treat as UTF-8 for
            // common case and fall back to lossy decode for the rest.
            let s = std::str::from_utf8(slice).map_or_else(
                |_| String::from_utf8_lossy(slice).into_owned(),
                String::from,
            );
            Ok(Value::String(s))
        }
        0x6 => {
            // UTF-16BE string, length is in code units.
            let (nchars, after) = read_size(ctx, pos, low)?;
            let nbytes = nchars
                .checked_mul(2)
                .ok_or_else(|| Error::Invalid("utf16 size overflow".into()))?;
            require(ctx, after, nbytes, "utf16 string")?;
            let mut code_units: Vec<u16> = Vec::with_capacity(nchars);
            for i in 0..nchars {
                let off = after + i * 2;
                code_units.push((u16::from(ctx.data[off]) << 8) | u16::from(ctx.data[off + 1]));
            }
            let s = String::from_utf16(&code_units)
                .map_err(|_| Error::Invalid("invalid UTF-16 string".into()))?;
            Ok(Value::String(s))
        }
        0x8 => {
            // UID — treat as a stringified integer.
            let nbytes = low as usize + 1;
            require(ctx, pos, nbytes, "uid")?;
            let v = read_be_u64(&ctx.data[pos..pos + nbytes]);
            Ok(Value::String(v.to_string()))
        }
        0xA | 0xB | 0xC => {
            // Array / ordered-set / set — all decode the same way.
            let (count, after) = read_size(ctx, pos, low)?;
            let bytes_needed = count
                .checked_mul(ctx.ref_size)
                .ok_or_else(|| Error::Invalid("array size overflow".into()))?;
            require(ctx, after, bytes_needed, "array refs")?;
            let mut items = Vec::with_capacity(count);
            for i in 0..count {
                let off = after + i * ctx.ref_size;
                let r = read_be_u64(&ctx.data[off..off + ctx.ref_size]) as usize;
                items.push(read_object(ctx, r, depth + 1)?);
            }
            Ok(Value::Array(items))
        }
        0xD => {
            // Dict
            let (count, after) = read_size(ctx, pos, low)?;
            let total_refs = count
                .checked_mul(2)
                .and_then(|v| v.checked_mul(ctx.ref_size))
                .ok_or_else(|| Error::Invalid("dict size overflow".into()))?;
            require(ctx, after, total_refs, "dict refs")?;
            let mut dict = BTreeMap::new();
            for i in 0..count {
                let k_off = after + i * ctx.ref_size;
                let v_off = after + (count + i) * ctx.ref_size;
                let k_idx = read_be_u64(&ctx.data[k_off..k_off + ctx.ref_size]) as usize;
                let v_idx = read_be_u64(&ctx.data[v_off..v_off + ctx.ref_size]) as usize;
                let key = match read_object(ctx, k_idx, depth + 1)? {
                    Value::String(s) => s,
                    other => {
                        return Err(Error::Invalid(format!(
                            "dict key is not a string: {other:?}"
                        )));
                    }
                };
                let value = read_object(ctx, v_idx, depth + 1)?;
                dict.insert(key, value);
            }
            Ok(Value::Dict(dict))
        }
        _ => Err(Error::Invalid(format!(
            "unknown high nibble 0x{high:X} (marker 0x{marker:02X})"
        ))),
    }
}

fn require(ctx: &Ctx, pos: usize, len: usize, what: &str) -> Result<(), Error> {
    pos.checked_add(len)
        .filter(|&end| end <= ctx.data.len())
        .map(|_| ())
        .ok_or_else(|| Error::Invalid(format!("{what} read out of bounds")))
}

/// Read a sized header. If `info` is below 0xF the size is the nibble itself.
/// Otherwise the following byte starts an integer object holding the real
/// size; this returns `(size, position_after_size)`.
fn read_size(ctx: &Ctx, pos: usize, info: u8) -> Result<(usize, usize), Error> {
    if info < 0x0F {
        return Ok((info as usize, pos));
    }
    if pos >= ctx.data.len() {
        return Err(Error::Invalid("extended size marker missing".into()));
    }
    let marker = ctx.data[pos];
    if marker >> 4 != 0x1 {
        return Err(Error::Invalid(format!(
            "expected extended-size int marker, got 0x{marker:02X}"
        )));
    }
    let nbytes = 1usize << (marker & 0x0F);
    let body = pos + 1;
    require(ctx, body, nbytes, "extended size")?;
    let val = read_be_u64(&ctx.data[body..body + nbytes]) as usize;
    Ok((val, body + nbytes))
}

fn u128_to_string(bytes: &[u8]) -> String {
    let mut v: u128 = 0;
    for &b in bytes {
        v = (v << 8) | u128::from(b);
    }
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    fn macosx_sdksettings() -> PathBuf {
        fixture(
            "xcspec-cache/xcode-26.5.0/sdksettings/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk/SDKSettings.plist",
        )
    }

    #[test]
    fn parses_macosx_sdksettings_top_dict() {
        let value = parse_file(&macosx_sdksettings()).unwrap();
        let dict = value.as_dict().expect("top should be dict");
        // Canonical/standard keys that every SDKSettings.plist exposes.
        for key in [
            "CanonicalName",
            "DisplayName",
            "Version",
            "DefaultProperties",
        ] {
            assert!(dict.contains_key(key), "missing key {key}");
        }
        assert_eq!(
            dict.get("CanonicalName").and_then(Value::as_str),
            Some("macosx26.5")
        );
    }

    #[test]
    fn macosx_default_properties_has_platform_name() {
        let value = parse_file(&macosx_sdksettings()).unwrap();
        let defaults = value
            .as_dict()
            .unwrap()
            .get("DefaultProperties")
            .and_then(Value::as_dict)
            .unwrap();
        assert_eq!(
            defaults.get("PLATFORM_NAME").and_then(Value::as_str),
            Some("macosx")
        );
    }

    #[test]
    fn all_sdksettings_plists_parse() {
        let root = fixture("xcspec-cache/xcode-26.5.0/sdksettings");
        let mut count = 0;
        walk(&root, &mut |p| {
            if p.file_name().is_some_and(|n| n == "SDKSettings.plist") {
                let v = parse_file(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                assert!(v.as_dict().is_some(), "{}: not a dict", p.display());
                count += 1;
            }
        });
        assert!(
            count >= 5,
            "expected several SDKSettings.plists, got {count}"
        );
    }

    fn walk(p: &Path, f: &mut dyn FnMut(&Path)) {
        if let Ok(entries) = std::fs::read_dir(p) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, f);
                } else {
                    f(&p);
                }
            }
        }
    }

    #[test]
    fn rejects_too_small() {
        assert!(parse(b"bplist00").is_err());
    }

    #[test]
    fn rejects_bad_magic() {
        let mut data = vec![0u8; 64];
        data[..8].copy_from_slice(b"bogus000");
        assert!(parse(&data).is_err());
    }
}
