//! `lipo`, for universal (fat) Mach-O files: Swift Build runs `-create` to
//! join the slices of a multi-architecture build, and `-extract` to keep the
//! architectures a test bundle builds for out of the XCTest runner it copies.
//! `-thin`, `-remove`, `-info`, `-archs` and `-verify_arch` are here for the
//! run scripts that use them.
//!
//! A fat file is a big-endian header listing each slice's CPU type, offset,
//! size and alignment, then the slices, each a whole Mach-O file (or static
//! library) at an offset aligned to its page size.

use std::path::{Path, PathBuf};

const FAT_MAGIC: u32 = 0xcafe_babe;
const FAT_MAGIC_64: u32 = 0xcafe_babf;
const MH_MAGIC: u32 = 0xfeed_face;
const MH_MAGIC_64: u32 = 0xfeed_facf;
/// The capability bits of a CPU subtype (like arm64e's pointer
/// authentication ABI), which don't change the architecture.
const SUBTYPE_MASK: u32 = 0x00ff_ffff;

/// Architecture names, CPU types and subtypes.
const ARCHS: &[(&str, u32, u32)] = &[
    ("i386", 7, 3),
    ("x86_64", 0x0100_0007, 3),
    ("x86_64h", 0x0100_0007, 8),
    ("armv7", 12, 9),
    ("armv7s", 12, 11),
    ("armv7k", 12, 12),
    ("arm64", 0x0100_000c, 0),
    ("arm64e", 0x0100_000c, 2),
    ("arm64_32", 0x0200_000c, 1),
];

/// One architecture's file in a fat file, or a thin file whole.
#[derive(Debug, Clone, Copy)]
struct Slice<'a> {
    cputype: u32,
    cpusubtype: u32,
    /// The power of two the slice's offset is a multiple of.
    align: u32,
    data: &'a [u8],
}

impl Slice<'_> {
    fn arch(&self) -> String {
        arch_name(self.cputype, self.cpusubtype)
    }

    fn is(&self, arch: (u32, u32)) -> bool {
        self.cputype == arch.0 && self.cpusubtype & SUBTYPE_MASK == arch.1
    }
}

fn arch_name(cputype: u32, cpusubtype: u32) -> String {
    ARCHS
        .iter()
        .find(|(_, t, s)| *t == cputype && *s == cpusubtype & SUBTYPE_MASK)
        .map_or_else(
            || {
                format!(
                    "cputype ({cputype}) cpusubtype ({})",
                    cpusubtype & SUBTYPE_MASK
                )
            },
            |(name, _, _)| (*name).to_string(),
        )
}

fn arch_named(name: &str) -> Result<(u32, u32), String> {
    ARCHS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, t, s)| (*t, *s))
        .ok_or_else(|| format!("unknown architecture specification flag: {name}"))
}

/// What a command line asks for.
#[derive(Debug, Default)]
struct Args {
    inputs: Vec<PathBuf>,
    output: Option<PathBuf>,
    create: bool,
    info: bool,
    archs: bool,
    thin: Option<String>,
    extract: Vec<String>,
    remove: Vec<String>,
    verify: Vec<String>,
}

fn parse(command_line: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut it = command_line.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-create" => args.create = true,
            "-info" => args.info = true,
            "-archs" => args.archs = true,
            "-output" | "-o" => args.output = Some(PathBuf::from(super::value(arg, &mut it)?)),
            "-thin" => args.thin = Some(super::value(arg, &mut it)?.clone()),
            "-extract" | "-extract_family" => {
                args.extract.push(super::value(arg, &mut it)?.clone());
            }
            "-remove" => args.remove.push(super::value(arg, &mut it)?.clone()),
            "-verify_arch" => {
                args.verify.extend(it.by_ref().cloned());
            }
            // `-arch <arch> <file>` names an input's architecture, which this
            // reads from the file itself.
            "-arch" => {
                super::value(arg, &mut it)?;
                args.inputs.push(PathBuf::from(super::value(arg, &mut it)?));
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag: {flag}")),
            file => args.inputs.push(PathBuf::from(file)),
        }
    }
    Ok(args)
}

pub(super) fn main(command_line: &[String]) -> Result<(), String> {
    let args = parse(command_line)?;
    let files = args
        .inputs
        .iter()
        .map(|path| super::read(path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut inputs = Vec::new();
    for (path, data) in args.inputs.iter().zip(&files) {
        inputs.push((path.as_path(), slices(path, data)?));
    }
    let output = || {
        args.output
            .as_deref()
            .ok_or_else(|| "no output file specified (use -output)".to_string())
    };
    if args.info {
        for (path, (fat, slices)) in &inputs {
            let archs = names(slices);
            if *fat {
                println!(
                    "Architectures in the fat file: {} are: {archs} ",
                    path.display()
                );
            } else {
                println!("Non-fat file: {} is architecture: {archs}", path.display());
            }
        }
        return Ok(());
    }
    let [(path, (fat, slices))] = &inputs[..] else {
        if args.create {
            return super::write(output()?, &create(&inputs)?);
        }
        return Err("only one input file can be specified with this operation".to_string());
    };
    if args.create {
        return super::write(output()?, &create(&inputs)?);
    }
    if args.archs {
        println!("{}", names(slices));
        return Ok(());
    }
    if !args.verify.is_empty() {
        for arch in &args.verify {
            let arch = arch_named(arch)?;
            if !slices.iter().any(|s| s.is(arch)) {
                return Err(format!(
                    "{} lacks {}",
                    path.display(),
                    arch_name(arch.0, arch.1)
                ));
            }
        }
        return Ok(());
    }
    if let Some(arch) = &args.thin {
        let slice = find(path, slices, arch)?;
        return super::write(output()?, slice.data);
    }
    if !args.extract.is_empty() {
        let kept = args
            .extract
            .iter()
            .map(|arch| find(path, slices, arch).copied())
            .collect::<Result<Vec<_>, _>>()?;
        // A thin file of the one architecture asked for stays as it is.
        if !fat && kept.len() == 1 {
            return super::write(output()?, kept[0].data);
        }
        return super::write(output()?, &fat_file(&kept));
    }
    if !args.remove.is_empty() {
        let removed = args
            .remove
            .iter()
            .map(|arch| find(path, slices, arch).map(|s| (s.cputype, s.cpusubtype)))
            .collect::<Result<Vec<_>, _>>()?;
        let kept: Vec<Slice> = slices
            .iter()
            .filter(|s| !removed.contains(&(s.cputype, s.cpusubtype)))
            .copied()
            .collect();
        if kept.is_empty() {
            return Err("-remove's specified would result in an empty fat file".to_string());
        }
        return super::write(output()?, &fat_file(&kept));
    }
    Err(
        "one of -create, -thin, -extract, -remove, -info, -archs or -verify_arch must be specified"
            .to_string(),
    )
}

fn names(slices: &[Slice]) -> String {
    slices.iter().map(Slice::arch).collect::<Vec<_>>().join(" ")
}

fn find<'s, 'a>(path: &Path, slices: &'s [Slice<'a>], arch: &str) -> Result<&'s Slice<'a>, String> {
    let wanted = arch_named(arch)?;
    slices.iter().find(|s| s.is(wanted)).ok_or_else(|| {
        format!(
            "{} does not contain the specified architecture ({arch})",
            path.display()
        )
    })
}

/// The slices of every input in one fat file.
fn create(inputs: &[(&Path, (bool, Vec<Slice>))]) -> Result<Vec<u8>, String> {
    let mut all: Vec<(&Path, Slice)> = Vec::new();
    for (path, (_, slices)) in inputs {
        for slice in slices {
            if let Some((other, _)) = all.iter().find(|(_, s)| {
                s.cputype == slice.cputype
                    && s.cpusubtype & SUBTYPE_MASK == slice.cpusubtype & SUBTYPE_MASK
            }) {
                return Err(format!(
                    "{} and {} have the same architectures ({}) and can't be in the same fat output file",
                    other.display(),
                    path.display(),
                    slice.arch()
                ));
            }
            all.push((path, *slice));
        }
    }
    let slices: Vec<Slice> = all.into_iter().map(|(_, s)| s).collect();
    Ok(fat_file(&slices))
}

/// A file's slices, and whether it's fat.
fn slices<'a>(path: &Path, data: &'a [u8]) -> Result<(bool, Vec<Slice<'a>>), String> {
    let be = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
    };
    let be64 = |at: usize| -> Option<u64> {
        Some(u64::from_be_bytes(data.get(at..at + 8)?.try_into().ok()?))
    };
    let truncated = || format!("truncated or malformed fat file: {}", path.display());
    let magic = be(0).unwrap_or_default();
    // A Java class file starts with the same magic, with a version where a
    // fat file has its (small) slice count.
    if (magic == FAT_MAGIC || magic == FAT_MAGIC_64) && be(4).is_some_and(|n| n < 43) {
        let count = be(4).ok_or_else(truncated)? as usize;
        let mut slices = Vec::with_capacity(count);
        for i in 0..count {
            let (at, offset, size, align) = if magic == FAT_MAGIC {
                let at = 8 + i * 20;
                (
                    at,
                    u64::from(be(at + 8).ok_or_else(truncated)?),
                    u64::from(be(at + 12).ok_or_else(truncated)?),
                    be(at + 16),
                )
            } else {
                let at = 8 + i * 32;
                (
                    at,
                    be64(at + 8).ok_or_else(truncated)?,
                    be64(at + 16).ok_or_else(truncated)?,
                    be(at + 24),
                )
            };
            let start = usize::try_from(offset).map_err(|_| truncated())?;
            let end = start + usize::try_from(size).map_err(|_| truncated())?;
            slices.push(Slice {
                cputype: be(at).ok_or_else(truncated)?,
                cpusubtype: be(at + 4).ok_or_else(truncated)?,
                align: align.ok_or_else(truncated)?,
                data: data.get(start..end).ok_or_else(truncated)?,
            });
        }
        return Ok((true, slices));
    }
    let (cputype, cpusubtype) = thin_arch(data).ok_or_else(|| {
        format!(
            "can't figure out the architecture type of: {}",
            path.display()
        )
    })?;
    Ok((
        false,
        vec![Slice {
            cputype,
            cpusubtype,
            align: page_align(cputype),
            data,
        }],
    ))
}

/// The CPU type and subtype of a Mach-O file, or of the first Mach-O member
/// of a static library.
fn thin_arch(data: &[u8]) -> Option<(u32, u32)> {
    let le = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
    };
    if matches!(le(0)?, MH_MAGIC | MH_MAGIC_64) {
        return Some((le(4)?, le(8)?));
    }
    let mut rest = data.strip_prefix(b"!<arch>\n")?;
    while rest.len() >= 60 {
        let header = &rest[..60];
        let name = std::str::from_utf8(&header[..16]).ok()?.trim_end();
        let size: usize = std::str::from_utf8(&header[48..58])
            .ok()?
            .trim()
            .parse()
            .ok()?;
        let body = rest.get(60..60 + size)?;
        // BSD archives put a long name at the start of the member's data.
        let (name, member) = match name.strip_prefix("#1/") {
            Some(length) => {
                let length: usize = length.parse().ok()?;
                let name = std::str::from_utf8(body.get(..length)?).ok()?;
                (name.trim_end_matches('\0').to_string(), &body[length..])
            }
            None => (name.trim_end_matches('/').to_string(), body),
        };
        if !name.starts_with("__.SYMDEF")
            && let Some(arch) = thin_arch(member)
        {
            return Some(arch);
        }
        rest = rest.get(60 + size + size % 2..)?;
    }
    None
}

/// The page size of a CPU type, as a power of two: 16 KB on ARM, 4 KB on
/// Intel.
fn page_align(cputype: u32) -> u32 {
    if cputype & 0xff == 12 { 14 } else { 12 }
}

/// A fat file of `slices`, by alignment, the way Apple's lipo orders them,
/// which puts arm64 after x86_64.
fn fat_file(slices: &[Slice]) -> Vec<u8> {
    let mut slices = slices.to_vec();
    slices.sort_by_key(|s| s.align);
    let mut offsets = Vec::with_capacity(slices.len());
    // Room for the wider of the two headers; the first slice is aligned
    // past it either way.
    let header_size = 8 + 32 * slices.len() as u64;
    let mut end = header_size;
    for slice in &slices {
        let align = 1u64 << slice.align;
        let offset = end.div_ceil(align) * align;
        offsets.push(offset);
        end = offset + slice.data.len() as u64;
    }
    let wide = end > u64::from(u32::MAX);
    let mut out = Vec::with_capacity(usize::try_from(end).unwrap_or_default());
    out.extend_from_slice(&(if wide { FAT_MAGIC_64 } else { FAT_MAGIC }).to_be_bytes());
    out.extend_from_slice(
        &u32::try_from(slices.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for (slice, offset) in slices.iter().zip(&offsets) {
        out.extend_from_slice(&slice.cputype.to_be_bytes());
        out.extend_from_slice(&slice.cpusubtype.to_be_bytes());
        let size = slice.data.len() as u64;
        if wide {
            out.extend_from_slice(&offset.to_be_bytes());
            out.extend_from_slice(&size.to_be_bytes());
            out.extend_from_slice(&slice.align.to_be_bytes());
            out.extend_from_slice(&0u32.to_be_bytes());
        } else {
            // Both fit, since the whole file does.
            let narrow = |n: u64| u32::try_from(n).unwrap_or(u32::MAX).to_be_bytes();
            out.extend_from_slice(&narrow(*offset));
            out.extend_from_slice(&narrow(size));
            out.extend_from_slice(&slice.align.to_be_bytes());
        }
    }
    for (slice, offset) in slices.iter().zip(&offsets) {
        out.resize(usize::try_from(*offset).unwrap_or_default(), 0);
        out.extend_from_slice(slice.data);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Mach-O header for `arch`, enough to read the architecture from.
    fn macho(arch: &str, filler: u8) -> Vec<u8> {
        let (cputype, cpusubtype) = arch_named(arch).unwrap();
        let mut out = MH_MAGIC_64.to_le_bytes().to_vec();
        out.extend_from_slice(&cputype.to_le_bytes());
        out.extend_from_slice(&cpusubtype.to_le_bytes());
        out.extend_from_slice(&[filler; 20]);
        out
    }

    fn archive(member: &[u8]) -> Vec<u8> {
        let mut out = b"!<arch>\n".to_vec();
        let mut push = |name: &str, data: &[u8]| {
            let padded = format!("{name}\0\0\0\0");
            let header = format!(
                "#1/{:<13}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                padded.len(),
                0,
                0,
                0,
                644,
                padded.len() + data.len()
            );
            out.extend_from_slice(header.as_bytes());
            out.extend_from_slice(padded.as_bytes());
            out.extend_from_slice(data);
        };
        push("__.SYMDEF SORTED", &[0; 8]);
        push("a.o", member);
        out
    }

    #[test]
    fn create_joins_thin_files_and_thin_takes_one_back_out() {
        let intel = macho("x86_64", 1);
        let arm = macho("arm64", 2);
        let a = slices(Path::new("a"), &arm).unwrap();
        let b = slices(Path::new("b"), &intel).unwrap();
        let fat = create(&[(Path::new("a"), a), (Path::new("b"), b)]).unwrap();
        let (is_fat, parsed) = slices(Path::new("fat"), &fat).unwrap();
        assert!(is_fat);
        assert_eq!(names(&parsed), "x86_64 arm64");
        assert_eq!(parsed[1].data, &arm[..]);
        assert_eq!(parsed[0].data, &intel[..]);
        assert_eq!(fat.len() % 4, 0);
        assert_eq!(&fat[..4], &FAT_MAGIC.to_be_bytes());
        // arm64's slice starts on a 16 KB page.
        assert_eq!(
            u32::from_be_bytes(fat[8 + 20 + 8..8 + 20 + 12].try_into().unwrap()),
            0x4000
        );
    }

    #[test]
    fn the_same_architecture_twice_is_refused() {
        let arm = macho("arm64", 0);
        let a = slices(Path::new("a"), &arm).unwrap();
        let b = slices(Path::new("b"), &arm).unwrap();
        let err = create(&[(Path::new("a"), a), (Path::new("b"), b)]).unwrap_err();
        assert!(err.contains("same architectures (arm64)"), "{err}");
    }

    #[test]
    fn a_static_library_has_its_members_architecture() {
        let lib = archive(&macho("arm64", 0));
        let (fat, parsed) = slices(Path::new("lib.a"), &lib).unwrap();
        assert!(!fat);
        assert_eq!(names(&parsed), "arm64");
    }

    #[test]
    fn arm64e_keeps_its_capability_bits_but_reads_as_arm64e() {
        let mut data = macho("arm64e", 0);
        data[8..12].copy_from_slice(&0x8000_0002u32.to_le_bytes());
        let (_, parsed) = slices(Path::new("e"), &data).unwrap();
        assert_eq!(parsed[0].arch(), "arm64e");
        assert_eq!(parsed[0].cpusubtype, 0x8000_0002);
    }

    #[test]
    fn something_else_has_no_architecture() {
        let err = slices(Path::new("x.txt"), b"hello, world").unwrap_err();
        assert!(err.contains("can't figure out the architecture type of: x.txt"));
    }
}
