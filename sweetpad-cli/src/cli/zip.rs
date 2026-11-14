//! Reading one member out of a zip file (an `.ipa`), without unpacking the
//! rest: the central directory names every member and where it starts, so
//! only that member's bytes are read. Stored and deflated members, and the
//! zip64 records an archive past 4 GB carries, are understood.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const END: u32 = 0x0605_4b50;
const END64: u32 = 0x0606_4b50;
const END64_LOCATOR: u32 = 0x0706_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap_or([0; 4]))
}

fn u64_at(data: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(data[at..at + 8].try_into().unwrap_or([0; 8]))
}

/// One central-directory record.
struct Member {
    name: String,
    method: u16,
    compressed: u64,
    size: u64,
    offset: u64,
}

/// The first member whose name `wanted` accepts, with its uncompressed
/// bytes, or `None` when no member matches.
pub(crate) fn read_member(
    path: &Path,
    wanted: impl Fn(&str) -> bool,
) -> Result<Option<(String, Vec<u8>)>, String> {
    let mut file = File::open(path).map_err(|e| format!("can't open {}: {e}", path.display()))?;
    let members = central_directory(&mut file)
        .map_err(|e| format!("{} isn't a zip file: {e}", path.display()))?;
    let Some(member) = members.into_iter().find(|m| wanted(&m.name)) else {
        return Ok(None);
    };
    let mut header = [0u8; 30];
    file.seek(SeekFrom::Start(member.offset))
        .map_err(|e| e.to_string())?;
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if u32_at(&header, 0) != LOCAL {
        return Err(format!(
            "{}: {} has no local header",
            path.display(),
            member.name
        ));
    }
    let skip = u64::from(u16_at(&header, 26)) + u64::from(u16_at(&header, 28));
    file.seek(SeekFrom::Current(i64::try_from(skip).unwrap_or(0)))
        .map_err(|e| e.to_string())?;
    let mut raw = vec![0u8; usize::try_from(member.compressed).map_err(|e| e.to_string())?];
    file.read_exact(&mut raw).map_err(|e| e.to_string())?;
    let data = match member.method {
        0 => raw,
        8 => {
            let mut out = Vec::with_capacity(usize::try_from(member.size).unwrap_or(0));
            flate2::read::DeflateDecoder::new(raw.as_slice())
                .read_to_end(&mut out)
                .map_err(|e| format!("{}: {} doesn't inflate: {e}", path.display(), member.name))?;
            out
        }
        other => {
            return Err(format!(
                "{}: {} uses compression method {other}, which sweetpad can't read",
                path.display(),
                member.name
            ));
        }
    };
    Ok(Some((member.name, data)))
}

fn central_directory(file: &mut File) -> Result<Vec<Member>, String> {
    let length = file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    let tail_length = length.min(66_000);
    let mut tail = vec![0u8; usize::try_from(tail_length).unwrap_or(0)];
    file.seek(SeekFrom::Start(length - tail_length))
        .map_err(|e| e.to_string())?;
    file.read_exact(&mut tail).map_err(|e| e.to_string())?;
    let end = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&at| u32_at(&tail, at) == END)
        .ok_or("no end of central directory record")?;
    let mut count = u64::from(u16_at(&tail, end + 10));
    let mut size = u64::from(u32_at(&tail, end + 12));
    let mut offset = u64::from(u32_at(&tail, end + 16));
    if (count == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF)
        && end >= 20
        && u32_at(&tail, end - 20) == END64_LOCATOR
    {
        let at = u64_at(&tail, end - 20 + 8);
        let mut record = [0u8; 56];
        file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
        file.read_exact(&mut record).map_err(|e| e.to_string())?;
        if u32_at(&record, 0) != END64 {
            return Err("a broken zip64 end record".into());
        }
        count = u64_at(&record, 32);
        size = u64_at(&record, 40);
        offset = u64_at(&record, 48);
    }
    let mut directory = vec![0u8; usize::try_from(size).map_err(|e| e.to_string())?];
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    file.read_exact(&mut directory).map_err(|e| e.to_string())?;
    let mut members = Vec::new();
    let mut at = 0;
    for _ in 0..count {
        if at + 46 > directory.len() || u32_at(&directory, at) != CENTRAL {
            return Err("a broken central directory".into());
        }
        let method = u16_at(&directory, at + 10);
        let mut compressed = u64::from(u32_at(&directory, at + 20));
        let mut uncompressed = u64::from(u32_at(&directory, at + 24));
        let name_length = usize::from(u16_at(&directory, at + 28));
        let extra_length = usize::from(u16_at(&directory, at + 30));
        let comment_length = usize::from(u16_at(&directory, at + 32));
        let mut local = u64::from(u32_at(&directory, at + 42));
        let name_end = at + 46 + name_length;
        let extra_end = name_end + extra_length;
        if extra_end > directory.len() {
            return Err("a broken central directory".into());
        }
        let name = String::from_utf8_lossy(&directory[at + 46..name_end]).into_owned();
        // A zip64 extra field holds, in order, whichever of the three
        // sizes overflowed.
        let mut extra = name_end;
        while extra + 4 <= extra_end {
            let id = u16_at(&directory, extra);
            let length = usize::from(u16_at(&directory, extra + 2));
            if id == 1 {
                let mut field = extra + 4;
                for value in [&mut uncompressed, &mut compressed, &mut local] {
                    if *value == 0xFFFF_FFFF && field + 8 <= extra + 4 + length {
                        *value = u64_at(&directory, field);
                        field += 8;
                    }
                }
            }
            extra += 4 + length;
        }
        members.push(Member {
            name,
            method,
            compressed,
            size: uncompressed,
            offset: local,
        });
        at = extra_end + comment_length;
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_stored_and_a_deflated_member() {
        let dir = crate::cli::testdir::TempDir::new("zip-read");
        std::fs::create_dir_all(dir.join("Payload/App.app")).unwrap();
        std::fs::write(dir.join("Payload/App.app/Info.plist"), "plist ".repeat(200)).unwrap();
        std::fs::write(dir.join("Payload/App.app/tiny"), "x").unwrap();
        let ipa = dir.join("App.ipa");
        let status = std::process::Command::new("zip")
            .args(["-qr", ipa.to_str().unwrap(), "Payload"])
            .current_dir(&*dir)
            .status()
            .unwrap();
        assert!(status.success());
        let (name, data) = read_member(&ipa, |n| n.ends_with("App.app/Info.plist"))
            .unwrap()
            .unwrap();
        assert_eq!(name, "Payload/App.app/Info.plist");
        assert_eq!(data, "plist ".repeat(200).into_bytes());
        let (_, tiny) = read_member(&ipa, |n| n.ends_with("/tiny"))
            .unwrap()
            .unwrap();
        assert_eq!(tiny, b"x");
        assert!(read_member(&ipa, |n| n == "missing").unwrap().is_none());
        assert!(read_member(&dir.join("Payload/App.app/tiny"), |_| true).is_err());
    }
}
