//! The parts of a Mach-O file a signature touches: the code signature load
//! command, the `__LINKEDIT` segment the signature ends, the `__TEXT`
//! segment a code directory names, and the SDK a hardened runtime records.
//! A universal file is split into its slices and joined again, each slice
//! where its alignment puts it. A slice is 64-bit, or 32-bit like a watch
//! app's arm64_32.

const MH_MAGIC: u32 = 0xfeed_face;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const FAT_MAGIC: u32 = 0xcafe_babe;
const LC_SEGMENT: u32 = 0x1;
const LC_SEGMENT_64: u32 = 0x19;
const LC_CODE_SIGNATURE: u32 = 0x1d;
const LC_UUID: u32 = 0x1b;
const LC_BUILD_VERSION: u32 = 0x32;
const LC_VERSION_MIN_MACOSX: u32 = 0x24;
const LC_VERSION_MIN_IPHONEOS: u32 = 0x25;
const LC_VERSION_MIN_TVOS: u32 = 0x2f;
const LC_VERSION_MIN_WATCHOS: u32 = 0x30;
const PLATFORM_IOS: u32 = 2;
const PLATFORM_TVOS: u32 = 3;
const PLATFORM_WATCHOS: u32 = 4;
const CPU_TYPE_ARM64_32: u32 = 0x0200_000c;
const CPU_TYPE_X86_64: u32 = 0x0100_0007;
pub(super) const MH_EXECUTE: u32 = 2;
pub(super) const CPU_TYPE_ARM64: u32 = 0x0100_000c;

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
    if !matches!(le32(data, 0), Some(MH_MAGIC | MH_MAGIC_64)) {
        return Err("not a Mach-O file".to_string());
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
        // codesign_allocate puts every slice on a 16 KiB boundary at least.
        let (cputype, subtype, align) = slice.arch.unwrap_or((0, 0, 14));
        let align = align.max(14);
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
    /// Whether the slice is 64-bit, whose segment commands have 64-bit
    /// sizes.
    wide: bool,
    /// `__TEXT`'s file offset and size, the executable segment.
    pub text: (u64, u64),
    pub uuid: Option<[u8; 16]>,
    /// The file offset and size of the Info.plist a standalone executable
    /// or library embeds in `__TEXT,__info_plist`.
    pub info_plist: Option<(usize, usize)>,
    /// The SDK it was built with, `xxxx.yy.zz` in nibbles, as the build
    /// version command records it.
    pub sdk: Option<u32>,
    /// The platform and the deployment target, `xxxx.yy.zz` in nibbles.
    deployment: Option<(u32, u32)>,
}

/// What a segment command says: its name, its file range, the lowest file
/// offset of its sections, and where an `__info_plist` section is, if any.
struct Segment<'a> {
    name: &'a [u8],
    fileoff: u64,
    filesize: u64,
    first_section: usize,
    info_plist: Option<(usize, usize)>,
}

/// The segment command at `at`, 64-bit when `wide`.
fn segment(data: &[u8], at: usize, wide: bool) -> Result<Segment<'_>, String> {
    let truncated = || "a truncated Mach-O file".to_string();
    let name = data.get(at + 8..at + 24).ok_or_else(truncated)?;
    // The file offset and size, the section count, where the sections
    // start, a section's size, and the offset of its file offset.
    let (fileoff, filesize, sections, first, stride, field) = if wide {
        (
            le64(data, at + 40),
            le64(data, at + 48),
            le32(data, at + 64),
            72,
            80,
            48,
        )
    } else {
        (
            le32(data, at + 32).map(u64::from),
            le32(data, at + 36).map(u64::from),
            le32(data, at + 48),
            56,
            68,
            40,
        )
    };
    let mut segment = Segment {
        name,
        fileoff: fileoff.ok_or_else(truncated)?,
        filesize: filesize.ok_or_else(truncated)?,
        first_section: usize::MAX,
        info_plist: None,
    };
    for i in 0..sections.ok_or_else(truncated)? as usize {
        let section = at + first + stride * i;
        let offset = le32(data, section + field).ok_or_else(truncated)?;
        if offset > 0 {
            segment.first_section = segment.first_section.min(offset as usize);
        }
        if data.get(section..section + 32)
            == Some(b"__info_plist\0\0\0\0__TEXT\0\0\0\0\0\0\0\0\0\0")
        {
            // The size, just before the offset: 64-bit in a 64-bit segment.
            let size = if wide {
                le64(data, section + field - 8)
            } else {
                le32(data, section + field - 4).map(u64::from)
            };
            let size = size.ok_or_else(truncated)?;
            segment.info_plist = Some((
                offset as usize,
                usize::try_from(size).map_err(|_| truncated())?,
            ));
        }
    }
    Ok(segment)
}

impl Layout {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        let truncated = || "a truncated Mach-O file".to_string();
        let wide = match le32(data, 0) {
            Some(MH_MAGIC_64) => true,
            Some(MH_MAGIC) => false,
            _ => return Err("not a Mach-O file".to_string()),
        };
        let cputype = le32(data, 4).ok_or_else(truncated)?;
        let filetype = le32(data, 12).ok_or_else(truncated)?;
        let count = le32(data, 16).ok_or_else(truncated)?;
        // The 64-bit header ends with a reserved field.
        let mut at = if wide { 32 } else { 28 };
        let mut signature = None;
        let mut linkedit = None;
        let mut text = None;
        let mut uuid = None;
        let mut info_plist = None;
        let mut sdk = None;
        let mut deployment = None;
        let mut first_section = usize::MAX;
        for _ in 0..count {
            let cmd = le32(data, at).ok_or_else(truncated)?;
            let size = le32(data, at + 4).ok_or_else(truncated)? as usize;
            if size < 8 {
                return Err("a malformed Mach-O load command".to_string());
            }
            match cmd {
                LC_SEGMENT_64 | LC_SEGMENT => {
                    let segment = segment(data, at, cmd == LC_SEGMENT_64)?;
                    first_section = first_section.min(segment.first_section);
                    if segment.info_plist.is_some() {
                        info_plist = segment.info_plist;
                    }
                    if segment.name.starts_with(b"__LINKEDIT\0") {
                        linkedit = Some((at, segment.fileoff, segment.filesize));
                    } else if segment.name.starts_with(b"__TEXT\0") {
                        text = Some((segment.fileoff, segment.filesize));
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
                LC_BUILD_VERSION => {
                    let platform = le32(data, at + 8).ok_or_else(truncated)?;
                    let minimum = le32(data, at + 12).ok_or_else(truncated)?;
                    deployment = Some((platform, minimum));
                    sdk = Some(le32(data, at + 16).ok_or_else(truncated)?);
                }
                LC_VERSION_MIN_MACOSX => sdk = Some(le32(data, at + 12).ok_or_else(truncated)?),
                LC_VERSION_MIN_IPHONEOS | LC_VERSION_MIN_TVOS | LC_VERSION_MIN_WATCHOS => {
                    let platform = match cmd {
                        LC_VERSION_MIN_IPHONEOS => PLATFORM_IOS,
                        LC_VERSION_MIN_TVOS => PLATFORM_TVOS,
                        _ => PLATFORM_WATCHOS,
                    };
                    deployment = Some((platform, le32(data, at + 8).ok_or_else(truncated)?));
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
            wide,
            text: text.unwrap_or((0, 0)),
            uuid,
            info_plist,
            sdk,
            deployment,
        })
    }

    /// Whether `codesign` gives the slice a SHA-1 code directory beside the
    /// SHA-256 one, as Apple's does for a deployment target before iOS or
    /// tvOS 11, or before watchOS 12 on arm64_32.
    pub fn needs_sha1(&self) -> bool {
        let below = |major: u32| self.deployment.is_some_and(|(_, v)| v < major << 16);
        match self.deployment.map(|(platform, _)| platform) {
            Some(PLATFORM_IOS | PLATFORM_TVOS) => below(11),
            Some(PLATFORM_WATCHOS) => self.cputype == CPU_TYPE_ARM64_32 && below(12),
            _ => false,
        }
    }

    /// The page size its code directory hashes in, as a power of two, as
    /// Apple's `codesign` picks it: 4 KiB for Intel code, for tvOS, and
    /// for a deployment target before iOS 16 or watchOS 9; 16 KiB for the
    /// rest, the Mac, simulators and visionOS among them.
    pub fn page_log2(&self) -> u8 {
        let below = |major: u32| self.deployment.is_some_and(|(_, v)| v < major << 16);
        let small = self.cputype == CPU_TYPE_X86_64
            || match self.deployment.map(|(platform, _)| platform) {
                Some(PLATFORM_TVOS) => true,
                Some(PLATFORM_IOS) => below(16),
                Some(PLATFORM_WATCHOS) => below(9),
                _ => false,
            };
        if small { 12 } else { 14 }
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
        // codesign_allocate rounds the segment to 16 KiB, Intel code's too.
        let at = self.linkedit_command;
        let vmsize = filesize.next_multiple_of(0x4000);
        if self.wide {
            put64(&mut out, at + 32, vmsize);
            put64(&mut out, at + 48, filesize);
        } else {
            let narrow = |n: u64| u32::try_from(n).map_err(|_| "a Mach-O file too large");
            put32(&mut out, at + 28, narrow(vmsize)?);
            put32(&mut out, at + 36, narrow(filesize)?);
        }
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
