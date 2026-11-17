//! CPIO, the payload of an Xcode `.xip`: odc (`070707`, octal) or newc
//! (`070701`, hex, 4-byte aligned) headers, each a name and its data, ended
//! by `TRAILER!!!` and zero padding.

use super::extract::{Entry, Kind, clean_name};
use super::source::Source;

pub(super) fn entries(
    source: &mut Source<'_>,
    mut each: impl FnMut(Entry) -> Result<(), String>,
) -> Result<(), String> {
    loop {
        let prefix = source.exact(6)?;
        let (mode, links, name_size, size, alignment, key) = match &prefix[..] {
            b"070707" => {
                let header = source.exact(70)?;
                let octal = |a: usize, b: usize| number(&header[a - 6..b - 6], 8);
                let (device, inode, mode, links) = (
                    octal(6, 12)?,
                    octal(12, 18)?,
                    octal(18, 24)?,
                    octal(36, 42)?,
                );
                let (name_size, size) = (octal(59, 65)?, octal(65, 76)?);
                (mode, links, name_size, size, 1, (0, device, inode))
            }
            b"070701" => {
                let header = source.exact(104)?;
                let field = |index: usize| number(&header[index * 8..index * 8 + 8], 16);
                let (inode, mode, links, size) = (field(0)?, field(1)?, field(4)?, field(6)?);
                let (major, minor, name_size) = (field(7)?, field(8)?, field(11)?);
                (mode, links, name_size, size, 4, (major, minor, inode))
            }
            b"070702" => return Err("CRC CPIO is not an Xcode XIP payload format".into()),
            _ => return Err("unsupported XIP CPIO header".into()),
        };
        if name_size == 0 || name_size > 65536 || size > super::MAX_OUTPUT {
            return Err("invalid XIP CPIO entry size".into());
        }
        let raw = source.exact(name_size as usize)?;
        let Some((0, name)) = raw.split_last() else {
            return Err("XIP entry name is not terminated".into());
        };
        let name = String::from_utf8(name.to_vec()).map_err(|_| "XIP entry name isn't UTF-8")?;
        let name = clean_name(&name)?;
        align(source, alignment)?;
        if name == "TRAILER!!!" {
            if size != 0 {
                return Err("invalid CPIO trailer".into());
            }
            return source.drain(true);
        }
        let offset = source.pos;
        let kind = match mode & 0o170_000 {
            0o100_000 => Kind::File,
            0o040_000 => Kind::Dir,
            0o120_000 => Kind::Symlink,
            _ => Kind::Special,
        };
        let link = if kind == Kind::Symlink {
            if size > 65536 {
                return Err("XIP symlink target is too large".into());
            }
            let target = source.exact(size as usize)?;
            Some(String::from_utf8(target).map_err(|_| "XIP symlink target isn't UTF-8")?)
        } else {
            source.skip(size)?;
            None
        };
        align(source, alignment)?;
        each(Entry {
            name,
            kind,
            size,
            offset,
            mode: (mode & 0o777) as u32,
            link,
            group: (kind == Kind::File && links > 1).then_some(key),
            ..Entry::default()
        })?;
    }
}

fn align(source: &mut Source<'_>, alignment: u64) -> Result<(), String> {
    source.skip((alignment - source.pos % alignment) % alignment)
}

fn number(digits: &[u8], radix: u32) -> Result<u64, String> {
    std::str::from_utf8(digits)
        .ok()
        .and_then(|text| u64::from_str_radix(text, radix).ok())
        .ok_or_else(|| "invalid XIP CPIO header".to_string())
}
