//! The parts of a Mach-O file a signature touches: the code signature load
//! command, the `__LINKEDIT` segment the signature ends, and the `__TEXT`
//! segment a code directory names. A universal file is split into its
//! slices and joined again, each slice where its alignment puts it.

const MH_MAGIC_64: u32 = 0xfeed_facf;
const FAT_MAGIC: u32 = 0xcafe_babe;
const LC_SEGMENT_64: u32 = 0x19;
const LC_CODE_SIGNATURE: u32 = 0x1d;
const LC_UUID: u32 = 0x1b;
pub(super) const MH_EXECUTE: u32 = 2;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;

/// A slice of a universal file, or the whole of a thin one.
pub(super) struct Slice {
    /// The cputype, cpusubtype and alignment from the universal header.
    pub arch: Option<(u32, u32, u32)>,
    pub data: Vec<u8>,
}

/// The slices of a Mach-O file.
pub(super) fn split(data: &[u8]) -> Result<Vec<Slice>, String> {
    if be32(data, 0) == Some(FAT_MAGIC) {
        let count = be32(data, 4).ok_or("a truncated universal header")?;
        let mut slices = Vec::new();
        for i in 0..count as usize {
            let at = 8 + 20 * i;
            let field = |n: usize| be32(data, at + 4 * n).ok_or("a truncated universal header");
            let (cputype, subtype, offset, size, align) =
                (field(0)?, field(1)?, field(2)?, field(3)?, field(4)?);
            let bytes = data
                .get(offset as usize..(offset as usize).saturating_add(size as usize))
                .ok_or("a universal slice past the end of the file")?;
            slices.push(Slice {
                arch: Some((cputype, subtype, align)),
                data: bytes.to_vec(),
            });
        }
        return Ok(slices);
    }
    if le32(data, 0) != Some(MH_MAGIC_64) {
        return Err("not a 64-bit Mach-O file".to_string());
    }
    Ok(vec![Slice {
        arch: None,
        data: data.to_vec(),
    }])
}

/// The slices as one file again.
pub(super) fn join(slices: Vec<Slice>) -> Vec<u8> {
    if slices.len() == 1 && slices[0].arch.is_none() {
        return slices
            .into_iter()
            .next()
            .map(|s| s.data)
            .unwrap_or_default();
    }
    let mut header = Vec::new();
    header.extend_from_slice(&FAT_MAGIC.to_be_bytes());
    header.extend_from_slice(&u32::try_from(slices.len()).unwrap_or(0).to_be_bytes());
    let mut offset = 8 + 20 * slices.len();
    let mut body: Vec<u8> = Vec::new();
    for slice in &slices {
        let (cputype, subtype, align) = slice.arch.unwrap_or((0, 0, 14));
        let alignment = 1usize << align;
        let start = offset.next_multiple_of(alignment);
        body.resize(start - 8 - 20 * slices.len(), 0);
        body.extend_from_slice(&slice.data);
        for value in [
            cputype,
            subtype,
            u32::try_from(start).unwrap_or(0),
            u32::try_from(slice.data.len()).unwrap_or(0),
            align,
        ] {
            header.extend_from_slice(&value.to_be_bytes());
        }
        offset = start + slice.data.len();
    }
    header.extend(body);
    header
}

/// Where the signature goes in a slice, and what else of it a signature
/// needs.
#[derive(Debug, Clone)]
pub(super) struct Layout {
    pub cputype: u32,
    pub filetype: u32,
    /// The offset of the signature: its load command's, or past the end of
    /// `__LINKEDIT` for a slice that has none yet.
    pub signature_offset: usize,
    /// The offset of the code signature load command, and of the end of the
    /// load commands for a slice that needs one added.
    signature_command: Option<usize>,
    commands_end: usize,
    first_section: usize,
    /// The offset of `__LINKEDIT`'s command, and its file offset.
    linkedit_command: usize,
    linkedit_fileoff: u64,
    /// `__TEXT`'s file offset and size, the executable segment.
    pub text: (u64, u64),
    pub uuid: Option<[u8; 16]>,
}

impl Layout {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        let truncated = || "a truncated Mach-O file".to_string();
        if le32(data, 0) != Some(MH_MAGIC_64) {
            return Err("not a 64-bit Mach-O file".to_string());
        }
        let cputype = le32(data, 4).ok_or_else(truncated)?;
        let filetype = le32(data, 12).ok_or_else(truncated)?;
        let count = le32(data, 16).ok_or_else(truncated)?;
        let mut at = 32;
        let mut signature = None;
        let mut linkedit = None;
        let mut text = None;
        let mut uuid = None;
        let mut first_section = usize::MAX;
        for _ in 0..count {
            let cmd = le32(data, at).ok_or_else(truncated)?;
            let size = le32(data, at + 4).ok_or_else(truncated)? as usize;
            if size < 8 {
                return Err("a malformed Mach-O load command".to_string());
            }
            match cmd {
                LC_SEGMENT_64 => {
                    let name = data.get(at + 8..at + 24).ok_or_else(truncated)?;
                    let fileoff = le64(data, at + 40).ok_or_else(truncated)?;
                    let filesize = le64(data, at + 48).ok_or_else(truncated)?;
                    let sections = le32(data, at + 64).ok_or_else(truncated)?;
                    for i in 0..sections as usize {
                        let offset = le32(data, at + 72 + 80 * i + 48).ok_or_else(truncated)?;
                        if offset > 0 {
                            first_section = first_section.min(offset as usize);
                        }
                    }
                    if name.starts_with(b"__LINKEDIT\0") {
                        linkedit = Some((at, fileoff, filesize));
                    } else if name.starts_with(b"__TEXT\0") {
                        text = Some((fileoff, filesize));
                    }
                }
                LC_CODE_SIGNATURE => {
                    let offset = le32(data, at + 8).ok_or_else(truncated)?;
                    signature = Some((at, offset as usize));
                }
                LC_UUID => {
                    let bytes = data.get(at + 8..at + 24).ok_or_else(truncated)?;
                    uuid = bytes.try_into().ok();
                }
                _ => {}
            }
            at += size;
        }
        let (linkedit_command, linkedit_fileoff, linkedit_size) =
            linkedit.ok_or("the Mach-O file has no __LINKEDIT segment")?;
        let linkedit_end =
            usize::try_from(linkedit_fileoff + linkedit_size).map_err(|_| truncated())?;
        // A slice without a signature gets one past the end of
        // `__LINKEDIT`, aligned to 16 bytes as `codesign_allocate` aligns
        // it, which can be past the end of the file.
        let signature_offset = match signature {
            Some((_, offset)) => offset,
            None => linkedit_end.next_multiple_of(16),
        };
        if signature.map_or(linkedit_end, |(_, offset)| offset) > data.len() {
            return Err(truncated());
        }
        Ok(Self {
            cputype,
            filetype,
            signature_offset,
            signature_command: signature.map(|(at, _)| at),
            commands_end: at,
            first_section,
            linkedit_command,
            linkedit_fileoff,
            text: text.unwrap_or((0, 0)),
            uuid,
        })
    }

    /// The signature the slice carries, if any: its superblob.
    pub fn signature<'a>(&self, data: &'a [u8]) -> Option<&'a [u8]> {
        let at = self.signature_command?;
        let size = le32(data, at + 12)? as usize;
        data.get(self.signature_offset..self.signature_offset + size)
    }

    /// The slice up to where its signature starts, its headers saying a
    /// signature of `size` bytes follows: the bytes the code directory
    /// hashes.
    pub fn prepare(&self, data: &[u8], size: usize) -> Result<Vec<u8>, String> {
        let mut out = data[..self.signature_offset.min(data.len())].to_vec();
        out.resize(self.signature_offset, 0);
        let offset = u32::try_from(self.signature_offset).map_err(|_| "a Mach-O file too large")?;
        let size32 = u32::try_from(size).map_err(|_| "a signature too large")?;
        if let Some(at) = self.signature_command {
            put32(&mut out, at + 8, offset);
            put32(&mut out, at + 12, size32);
        } else {
            // Add the command after the others, where the headers leave
            // room before the first section.
            let at = self.commands_end;
            if at + 16 > self.first_section {
                return Err(
                    "no room in the Mach-O headers for a code signature; link with \
                     '-headerpad' or let the linker sign it"
                        .to_string(),
                );
            }
            put32(&mut out, at, LC_CODE_SIGNATURE);
            put32(&mut out, at + 4, 16);
            put32(&mut out, at + 8, offset);
            put32(&mut out, at + 12, size32);
            let count = le32(&out, 16).unwrap_or(0) + 1;
            let commands = le32(&out, 20).unwrap_or(0) + 16;
            put32(&mut out, 16, count);
            put32(&mut out, 20, commands);
        }
        let filesize = (self.signature_offset + size) as u64 - self.linkedit_fileoff;
        let page = if self.cputype == CPU_TYPE_ARM64 {
            0x4000
        } else {
            0x1000
        };
        let at = self.linkedit_command;
        put64(&mut out, at + 32, filesize.next_multiple_of(page));
        put64(&mut out, at + 48, filesize);
        Ok(out)
    }
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn le32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn le64(data: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(data.get(at..at + 8)?.try_into().ok()?))
}

fn put32(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(data: &mut [u8], at: usize, value: u64) {
    data[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
