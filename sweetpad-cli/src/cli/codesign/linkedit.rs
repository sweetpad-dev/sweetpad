//! The string table alignment Apple's `strip` keeps. `llvm-strip`, which the
//! strip stand-in runs, puts a 64-bit file's string table straight after the
//! indirect symbol table's 4-byte entries, and dyld refuses to load one
//! whose string table isn't 8-byte aligned. This moves the table to the next
//! 8-byte boundary, and signs a linker-signed slice again as the linker
//! signed it, since moving the table changes the bytes its hashes cover.

use super::blobs::{self, CodeHash, Directory};
use super::macho;

const MH_MAGIC_64: u32 = 0xfeed_facf;
const LC_SEGMENT_64: u32 = 0x19;
const LC_SYMTAB: u32 = 0x2;
const LC_CODE_SIGNATURE: u32 = 0x1d;
/// The file types dyld loads: executables, dylibs and bundles.
const LOADED: [u32; 3] = [2, 6, 8];

/// `data` with the string table of every 64-bit slice dyld loads 8-byte
/// aligned, or none when they all are, or it isn't such a file.
pub(crate) fn align_string_tables(data: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let Ok(mut slices) = macho::split(data) else {
        return Ok(None);
    };
    let mut changed = false;
    for slice in &mut slices {
        if let Some(aligned) = align(&slice.data)? {
            slice.data = aligned;
            changed = true;
        }
    }
    Ok(changed.then(|| macho::join(slices)))
}

/// Where a slice's load commands say its `__LINKEDIT` parts are.
struct Commands {
    symtab: usize,
    linkedit: usize,
    signature: Option<usize>,
}

fn commands(data: &[u8]) -> Option<Commands> {
    let count = le32(data, 16)?;
    let mut at = 32;
    let (mut symtab, mut linkedit, mut signature) = (None, None, None);
    for _ in 0..count {
        let (cmd, size) = (le32(data, at)?, le32(data, at + 4)? as usize);
        match cmd {
            LC_SYMTAB => symtab = Some(at),
            LC_CODE_SIGNATURE => signature = Some(at),
            LC_SEGMENT_64 if data.get(at + 8..at + 19)? == b"__LINKEDIT\0" => linkedit = Some(at),
            _ => {}
        }
        at += size.max(8);
    }
    Some(Commands {
        symtab: symtab?,
        linkedit: linkedit?,
        signature,
    })
}

fn align(data: &[u8]) -> Result<Option<Vec<u8>>, String> {
    if le32(data, 0) != Some(MH_MAGIC_64) || !le32(data, 12).is_some_and(|t| LOADED.contains(&t)) {
        return Ok(None);
    }
    let Some(commands) = commands(data) else {
        return Ok(None);
    };
    let truncated = || "a truncated Mach-O file".to_string();
    let stroff = le32(data, commands.symtab + 16).ok_or_else(truncated)? as usize;
    let strsize = le32(data, commands.symtab + 20).ok_or_else(truncated)? as usize;
    if stroff.is_multiple_of(8) || strsize == 0 {
        return Ok(None);
    }
    let strings = data.get(stroff..stroff + strsize).ok_or_else(truncated)?;
    let signature = match commands.signature {
        Some(at) => {
            let offset = le32(data, at + 8).ok_or_else(truncated)? as usize;
            let size = le32(data, at + 12).ok_or_else(truncated)? as usize;
            Some(data.get(offset..offset + size).ok_or_else(truncated)?)
        }
        None => None,
    };
    let aligned = stroff.next_multiple_of(8);
    let mut out = data[..stroff].to_vec();
    out.resize(aligned, 0);
    out.extend_from_slice(strings);
    put32(&mut out, commands.symtab + 16, aligned);
    let fileoff = le64(&out, commands.linkedit + 40)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(truncated)?;
    let Some(signature) = signature else {
        let filesize = (out.len() - fileoff) as u64;
        let vmsize = le64(&out, commands.linkedit + 32).ok_or_else(truncated)?;
        put64(&mut out, commands.linkedit + 48, filesize);
        put64(
            &mut out,
            commands.linkedit + 32,
            vmsize.max(filesize.next_multiple_of(0x1000)),
        );
        return Ok(Some(out));
    };
    let at = commands.signature.unwrap_or_default();
    let Some(linker) = Linker::read(signature) else {
        // Any other signature is broken by the strip either way, and the
        // signing after it replaces it.
        let end = out.len();
        put32(&mut out, at + 8, end);
        out.extend_from_slice(signature);
        let filesize = (out.len() - fileoff) as u64;
        put64(&mut out, commands.linkedit + 48, filesize);
        return Ok(Some(out));
    };
    // The signature goes past the strings, 16-byte aligned, as the linker
    // puts it.
    out.resize(out.len().next_multiple_of(16), 0);
    let end = out.len();
    put32(&mut out, at + 8, end);
    put32(&mut out, at + 12, 0);
    let filesize = (out.len() - fileoff) as u64;
    put64(&mut out, commands.linkedit + 48, filesize);
    let directory = Directory {
        identifier: &linker.identifier,
        team: None,
        flags: linker.flags,
        exec_segment: linker.exec_segment,
        special: Vec::new(),
        page_log2: linker.page_log2,
        runtime: None,
    };
    let layout = macho::Layout::parse(&out)?;
    let unsigned = directory.build(&layout.prepare(&out, 0)?, CodeHash::Sha256);
    let size = blobs::superblob(vec![(blobs::SLOT_CODE_DIRECTORY, unsigned)])
        .len()
        .next_multiple_of(16);
    let mut signed = layout.prepare(&out, size)?;
    let code_directory = directory.build(&signed, CodeHash::Sha256);
    let mut blob = blobs::superblob(vec![(blobs::SLOT_CODE_DIRECTORY, code_directory)]);
    blob.resize(size, 0);
    signed.extend(blob);
    Ok(Some(signed))
}

/// What a linker signature's code directory records besides the hashes,
/// kept for hashing the slice again.
struct Linker {
    identifier: String,
    flags: u32,
    exec_segment: (u64, u64, u64),
    page_log2: u8,
}

impl Linker {
    /// None for any signature but a linker's.
    fn read(signature: &[u8]) -> Option<Self> {
        let be32 = |at: usize| {
            Some(u32::from_be_bytes(
                signature.get(at..at + 4)?.try_into().ok()?,
            ))
        };
        if be32(0)? != blobs::EMBEDDED_SIGNATURE {
            return None;
        }
        let offset = (0..be32(8)? as usize)
            .find(|i| be32(12 + 8 * i) == Some(blobs::SLOT_CODE_DIRECTORY))
            .and_then(|i| be32(16 + 8 * i))? as usize;
        let directory = signature.get(offset..)?;
        let field = |at: usize| {
            Some(u32::from_be_bytes(
                directory.get(at..at + 4)?.try_into().ok()?,
            ))
        };
        let field64 = |at: usize| {
            Some(u64::from_be_bytes(
                directory.get(at..at + 8)?.try_into().ok()?,
            ))
        };
        let flags = field(12)?;
        if flags & blobs::FLAG_LINKER_SIGNED == 0 || field(8)? < 0x2_0400 {
            return None;
        }
        let identifier = directory.get(field(20)? as usize..)?;
        let end = identifier.iter().position(|b| *b == 0)?;
        Some(Self {
            identifier: String::from_utf8_lossy(&identifier[..end]).into_owned(),
            flags,
            exec_segment: (field64(64)?, field64(72)?, field64(80)?),
            page_log2: *directory.get(39)?,
        })
    }
}

fn le32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn le64(data: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(data.get(at..at + 8)?.try_into().ok()?))
}

fn put32(data: &mut [u8], at: usize, value: usize) {
    data[at..at + 4].copy_from_slice(&u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes());
}

fn put64(data: &mut [u8], at: usize, value: u64) {
    data[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::super::crypto::sha256;
    use super::*;

    /// `stripped.c` built for arm64 and x86_64 and stripped by `llvm-strip`
    /// 18: the arm64 slice linker-signed, the x86_64 one unsigned, and both
    /// string tables 4 bytes off an 8-byte boundary.
    fn stripped() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/strip/Stripped");
        std::fs::read(path).unwrap()
    }

    fn strings(data: &[u8]) -> (usize, &[u8]) {
        let symtab = commands(data).unwrap().symtab;
        let (offset, size) = (
            le32(data, symtab + 16).unwrap() as usize,
            le32(data, symtab + 20).unwrap() as usize,
        );
        (offset, &data[offset..offset + size])
    }

    /// That the code directory's page hashes cover the slice as it is.
    fn assert_hashes_match(data: &[u8]) {
        let layout = macho::Layout::parse(data).unwrap();
        let signature = layout.signature(data).unwrap();
        let be =
            |bytes: &[u8], at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(be(signature, 12), blobs::SLOT_CODE_DIRECTORY);
        let directory = &signature[be(signature, 16) as usize..];
        let (hashes, slots, limit) = (
            be(directory, 16) as usize,
            be(directory, 28) as usize,
            be(directory, 32) as usize,
        );
        assert_eq!(limit, layout.signature_offset);
        let page = 1 << directory[39];
        for slot in 0..slots {
            let code = &data[slot * page..((slot + 1) * page).min(limit)];
            assert_eq!(
                &directory[hashes + 32 * slot..hashes + 32 * (slot + 1)],
                sha256(code).as_slice(),
                "page {slot}"
            );
        }
    }

    #[test]
    fn string_tables_move_to_an_8_byte_boundary_and_linker_signatures_follow() {
        let input = stripped();
        let output = align_string_tables(&input)
            .unwrap()
            .expect("misaligned tables");
        let before = macho::split(&input).unwrap();
        let after = macho::split(&output).unwrap();
        assert_eq!(before.len(), 2);
        for (old, new) in before.iter().zip(&after) {
            let ((old_offset, old_strings), (new_offset, new_strings)) =
                (strings(&old.data), strings(&new.data));
            assert_eq!(old_offset % 8, 4);
            assert_eq!(new_offset, old_offset + 4);
            assert_eq!(old_strings, new_strings);
            let linkedit = commands(&new.data).unwrap().linkedit;
            let (fileoff, filesize) = (
                usize::try_from(le64(&new.data, linkedit + 40).unwrap()).unwrap(),
                usize::try_from(le64(&new.data, linkedit + 48).unwrap()).unwrap(),
            );
            assert_eq!(fileoff + filesize, new.data.len());
            let old_linker = macho::Layout::parse(&old.data)
                .unwrap()
                .signature(&old.data)
                .and_then(Linker::read);
            let new_linker = macho::Layout::parse(&new.data)
                .unwrap()
                .signature(&new.data)
                .and_then(Linker::read);
            match (old_linker, new_linker) {
                (Some(old_linker), Some(new_linker)) => {
                    assert_eq!(new_linker.identifier, old_linker.identifier);
                    assert_eq!(new_linker.flags, old_linker.flags);
                    assert_eq!(new_linker.exec_segment, old_linker.exec_segment);
                    assert_eq!(new_linker.page_log2, old_linker.page_log2);
                    assert_hashes_match(&new.data);
                }
                (None, None) => assert!(commands(&new.data).unwrap().signature.is_none()),
                _ => panic!("the slice's signature changed kind"),
            }
        }
        assert!(align_string_tables(&output).unwrap().is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn apple_tools_accept_the_aligned_file() {
        let dir = std::env::temp_dir().join(format!("sweetpad-linkedit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Stripped");
        std::fs::write(&path, align_string_tables(&stripped()).unwrap().unwrap()).unwrap();
        let verify = std::process::Command::new("codesign")
            .args(["--verify", "--arch", "arm64"])
            .arg(&path)
            .output()
            .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            verify.status.success(),
            "{}",
            String::from_utf8_lossy(&verify.stderr)
        );
    }
}
