//! Apple Archive (`AA01`, or the older `YAA1`): per entry, a header of typed
//! fields (a 3-letter key and a type letter each), then the blobs those
//! fields declare, `DAT` being the file's data. Files can share data
//! through cluster identifiers (`HLC`, `CLC`, `SLC`), and a deduplicated
//! archive stores each shared payload once (see `extract::Clusters`).

use sha2::Digest;

use super::extract::{Entry, Kind, clean_name};
use super::source::Source;

#[derive(Debug)]
enum Field {
    Flag,
    Number(u64),
    Text(String),
    Blob(u64),
    Bytes(Vec<u8>),
}

#[allow(clippy::too_many_lines)] // one record: its fields, its blobs, then its checks
pub(super) fn entries(
    source: &mut Source<'_>,
    mut each: impl FnMut(Entry) -> Result<(), String>,
) -> Result<(), String> {
    loop {
        let mut prefix = [0u8; 6];
        let n = source.read_some(&mut prefix)?;
        if n == 0 {
            return Ok(());
        }
        if n != 6 || !matches!(&prefix[..4], b"AA01" | b"YAA1") {
            return Err("unsupported Apple Archive entry header".into());
        }
        let size = usize::from(u16::from_le_bytes([prefix[4], prefix[5]]));
        if size < 6 {
            return Err("invalid Apple Archive header length".into());
        }
        let header = source.exact(size - 6)?;
        let fields = parse_fields(&header)?;
        let mut offsets = std::collections::HashMap::new();
        let mut content_hash = None;
        let checked = ["HLC", "CLC", "SLC", "SH2"]
            .iter()
            .any(|key| fields.iter().any(|(k, _)| k == key));
        for (key, field) in &fields {
            let Field::Blob(length) = field else { continue };
            offsets.insert(key.clone(), source.pos);
            if key == "DAT" && checked {
                let mut digest = sha2::Sha256::new();
                let mut remaining = *length;
                let mut buf = vec![0u8; 1 << 20];
                while remaining > 0 {
                    let n = remaining.min(buf.len() as u64) as usize;
                    source.exact_into(&mut buf[..n])?;
                    digest.update(&buf[..n]);
                    remaining -= n as u64;
                }
                content_hash = Some(<[u8; 32]>::from(digest.finalize()));
            } else {
                source.skip(*length)?;
            }
        }
        let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        let kind = match get("TYP") {
            Some(Field::Number(t)) => match u8::try_from(*t).unwrap_or(0) {
                b'F' => Kind::File,
                b'D' => Kind::Dir,
                b'L' => Kind::Symlink,
                b'H' => Kind::Hardlink,
                b'M' => continue,
                _ => Kind::Special,
            },
            _ => Kind::Special,
        };
        let name = match get("PAT") {
            Some(Field::Text(path)) => clean_name(path)?,
            None => clean_name("")?,
            _ => return Err("invalid Apple Archive path".into()),
        };
        let mode = match get("MOD") {
            None => {
                if kind == Kind::Dir {
                    0o755
                } else {
                    0o644
                }
            }
            Some(Field::Number(mode)) => *mode as u32,
            _ => return Err("invalid Apple Archive file attributes".into()),
        };
        let data_size = match get("DAT") {
            None => None,
            Some(Field::Blob(size)) => Some(*size),
            _ => return Err("invalid Apple Archive file attributes".into()),
        };
        let has_data = offsets.contains_key("DAT");
        let size = data_size.unwrap_or(0);
        let clusters = if kind == Kind::File {
            let mut clusters = Vec::new();
            for key in ["HLC", "CLC", "SLC"] {
                match get(key) {
                    None => {}
                    Some(Field::Number(id)) => clusters.push((key, *id)),
                    _ => return Err("invalid Apple Archive data cluster identifier".into()),
                }
            }
            clusters
        } else {
            Vec::new()
        };
        let logical_size = match get("SIZ") {
            Some(Field::Number(size)) if *size <= super::MAX_OUTPUT => Some(*size),
            Some(_) => return Err("invalid Apple Archive file size".into()),
            None if has_data || clusters.is_empty() => Some(size),
            None => None,
        };
        let expected_hash = match get("SH2") {
            None => None,
            Some(Field::Bytes(hash)) if hash.len() == 32 => {
                Some(<[u8; 32]>::try_from(&hash[..]).unwrap())
            }
            _ => return Err("invalid Apple Archive file checksum".into()),
        };
        if kind == Kind::File {
            if has_data && logical_size != Some(size) {
                return Err("Apple Archive file data size differs from its declared size".into());
            }
            if !has_data && logical_size.is_some_and(|s| s > 0) && clusters.is_empty() {
                return Err("Apple Archive file is missing its data".into());
            }
            if has_data && expected_hash.is_some() && content_hash != expected_hash {
                return Err("Apple Archive file checksum failed".into());
            }
        }
        let link = match get("LNK") {
            Some(Field::Text(link)) => Some(link.clone()),
            _ => None,
        };
        each(Entry {
            name,
            kind,
            size,
            offset: offsets.get("DAT").copied().unwrap_or(source.pos),
            mode: mode & 0o777,
            link,
            group: None,
            clusters,
            has_data,
            logical_size,
            content_hash,
            expected_hash,
        })?;
    }
}

fn parse_fields(header: &[u8]) -> Result<Vec<(String, Field)>, String> {
    let mut fields: Vec<(String, Field)> = Vec::new();
    let mut pos = 0;
    while pos < header.len() {
        if pos + 4 > header.len() {
            return Err("truncated Apple Archive field".into());
        }
        let key = std::str::from_utf8(&header[pos..pos + 3])
            .map_err(|_| "invalid Apple Archive field key")?
            .to_string();
        let subtype = header[pos + 3];
        pos += 4;
        if fields.iter().any(|(k, _)| *k == key) {
            return Err("duplicate Apple Archive field".into());
        }
        if subtype == b'*' {
            fields.push((key, Field::Flag));
            continue;
        }
        let length = match subtype {
            b'1' => 1,
            b'2' | b'A' | b'P' => 2,
            b'4' | b'F' | b'B' => 4,
            b'8' | b'S' | b'C' => 8,
            b'T' => 12,
            b'G' => 20,
            b'H' => 32,
            b'I' => 48,
            b'J' => 64,
            _ => return Err("invalid Apple Archive field encoding".into()),
        };
        if pos + length > header.len() {
            return Err("invalid Apple Archive field encoding".into());
        }
        let data = &header[pos..pos + length];
        pos += length;
        let little = |bytes: &[u8]| {
            bytes
                .iter()
                .rev()
                .fold(0u64, |value, byte| (value << 8) | u64::from(*byte))
        };
        let field = match subtype {
            b'P' => {
                let length = little(data) as usize;
                if pos + length > header.len() {
                    return Err("truncated Apple Archive string".into());
                }
                let mut text = &header[pos..pos + length];
                pos += length;
                while let [rest @ .., 0] = text {
                    text = rest;
                }
                Field::Text(
                    String::from_utf8(text.to_vec()).map_err(|_| "invalid Apple Archive string")?,
                )
            }
            b'A' | b'B' | b'C' => {
                let length = little(data);
                if length > super::MAX_OUTPUT {
                    return Err("Apple Archive blob exceeds the size limit".into());
                }
                Field::Blob(length)
            }
            b'1' | b'2' | b'4' | b'8' => Field::Number(little(data)),
            _ => Field::Bytes(data.to_vec()),
        };
        fields.push((key, field));
    }
    Ok(fields)
}
