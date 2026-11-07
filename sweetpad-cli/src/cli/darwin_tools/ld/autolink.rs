//! LLD resolves an autolinked reexport from the first framework it opens.
//! Xcode prefers SwiftUI's umbrella even when SwiftUICore appears first.
//! Keep those references through the public umbrella at older deployment targets.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Frameworks {
    ui: bool,
    core: bool,
    arch: Option<(u32, u32)>,
}

pub(super) fn prefer_swiftui_umbrella(args: Vec<String>) -> Result<Vec<String>, String> {
    let cache = sweetpad_core::paths::sweetpad_state_dir()
        .map(|p| p.join("linker-inputs"))
        .unwrap_or_default();
    prefer(args, &cache)
}

fn prefer(mut args: Vec<String>, cache: &Path) -> Result<Vec<String>, String> {
    // An explicit weak framework must keep its requested loading policy.
    if args.windows(2).any(|a| {
        a[1] == "SwiftUI"
            && matches!(
                a[0].as_str(),
                "-framework"
                    | "-weak_framework"
                    | "-lazy_framework"
                    | "-reexport_framework"
                    | "-upward_framework"
            )
    }) {
        return Ok(args);
    }
    let mut found = Frameworks {
        arch: args
            .windows(2)
            .find(|a| a[0] == "-arch")
            .and_then(|a| super::super::lipo::arch_named(&a[1]).ok()),
        ..Frameworks::default()
    };
    let all_load = args.iter().any(|a| a == "-all_load");
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "-o" | "-object_path_lto" | "-dependency_info" | "-map" => {
                rest.next();
            }
            "-force_load" => {
                let path = super::super::value(arg, &mut rest)?;
                inspect(Path::new(path), true, &mut found)?;
            }
            "-filelist" => {
                let value = super::super::value(arg, &mut rest)?;
                let (path, folder) = match value.rsplit_once(',') {
                    Some((path, folder)) if !Path::new(value).is_file() => (path, Some(folder)),
                    _ => (value.as_str(), None),
                };
                let list = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
                for item in list.lines() {
                    let path =
                        folder.map_or_else(|| PathBuf::from(item), |f| Path::new(f).join(item));
                    inspect(&path, all_load, &mut found)?;
                }
            }
            path if !path.starts_with('-') => inspect(Path::new(path), all_load, &mut found)?,
            _ => {}
        }
        if found.ui && found.core {
            let arch = found
                .arch
                .ok_or("missing architecture for SwiftUI autolink input")?;
            args.insert(0, umbrella_input(cache, arch)?.display().to_string());
            break;
        }
    }
    Ok(args)
}

// Keep the framework implicit: an explicit -framework retains it even if no
// live input needs SwiftUI. A leading object changes only autolink ordering.
// Persist it so the linker's dependency file remains valid for no-op builds.
fn umbrella_input(cache: &Path, (cpu, subtype): (u32, u32)) -> Result<PathBuf, String> {
    if cache.as_os_str().is_empty() {
        return Err("SwiftUI autolink inputs require HOME or XDG_STATE_HOME".into());
    }
    let wide = cpu & 0x0300_0000 != 0;
    let header_size = if wide { 32 } else { 28 };
    let options = b"-framework\0SwiftUI\0";
    let command_size = (12 + u32::try_from(options.len()).unwrap() + 7) & !7;
    let size = header_size + command_size + 24;
    let mut bytes = Vec::new();
    let mut word = |value: u32| bytes.extend_from_slice(&value.to_le_bytes());
    for value in [
        if wide { 0xfeed_facf } else { 0xfeed_face },
        cpu,
        subtype,
        1,
        2,
        command_size + 24,
        0,
    ] {
        word(value);
    }
    if wide {
        word(0);
    }
    for value in [0x2d, command_size, 2] {
        word(value);
    }
    bytes.extend_from_slice(options);
    bytes.resize((header_size + command_size) as usize, 0);
    for value in [2u32, 24, size, 0, size, 1] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.push(0);
    let name = format!("swiftui-{cpu:x}-{subtype:x}-v1.o");
    let path = cache.join(&name);
    if std::fs::read(&path).is_ok_and(|existing| existing == bytes) {
        return Ok(path);
    }
    std::fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let temporary = cache.join(format!(".{name}-{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes)
        .and_then(|()| std::fs::rename(&temporary, &path))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

fn inspect(path: &Path, archives: bool, found: &mut Frameworks) -> Result<(), String> {
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|e| e == "o" || (archives && e == "a"))
    {
        return Ok(());
    }
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    scan(&mut file, 0, size, archives, found, 0)
        .map_err(|e| format!("reading autolink options in {}: {e}", path.display()))
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "malformed Mach-O or archive header",
    )
}

fn scan(
    file: &mut File,
    offset: u64,
    size: u64,
    archives: bool,
    found: &mut Frameworks,
    depth: u8,
) -> io::Result<()> {
    if size < 8 || depth > 8 {
        return Ok(());
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut magic = [0; 8];
    file.read_exact(&mut magic)?;
    if &magic == b"!<arch>\n" && archives {
        return archive(file, offset, size, found, depth + 1);
    }
    if matches!(&magic[..4], b"\xca\xfe\xba\xbe" | b"\xca\xfe\xba\xbf") {
        return fat(file, offset, size, magic, archives, found, depth + 1);
    }
    let header_size = match &magic[..4] {
        b"\xcf\xfa\xed\xfe" => 32,
        b"\xce\xfa\xed\xfe" => 28,
        _ => return Ok(()),
    };
    if size < header_size {
        return Err(invalid());
    }
    let cpu = u32::from_le_bytes(magic[4..].try_into().unwrap());
    let mut subtype = [0; 4];
    file.seek(SeekFrom::Start(offset + 8))?;
    file.read_exact(&mut subtype)?;
    if found
        .arch
        .is_some_and(|arch| arch != (cpu, u32::from_le_bytes(subtype) & 0x00ff_ffff))
    {
        return Ok(());
    }
    let mut header = [0; 12];
    file.seek(SeekFrom::Start(offset + 12))?;
    file.read_exact(&mut header)?;
    // Only relocatable object inputs contribute automatic framework options.
    if u32::from_le_bytes(header[..4].try_into().unwrap()) != 1 {
        return Ok(());
    }
    found
        .arch
        .get_or_insert((cpu, u32::from_le_bytes(subtype) & 0x00ff_ffff));
    let count = u32::from_le_bytes(header[4..8].try_into().unwrap());
    let length = u32::from_le_bytes(header[8..12].try_into().unwrap());
    if u64::from(length) > size - header_size || length > 16 * 1024 * 1024 {
        return Err(invalid());
    }
    let mut commands = vec![0; length as usize];
    file.seek(SeekFrom::Start(offset + header_size))?;
    file.read_exact(&mut commands)?;
    commands_frameworks(&commands, count, found)
}

fn commands_frameworks(data: &[u8], count: u32, found: &mut Frameworks) -> io::Result<()> {
    let mut offset = 0usize;
    for _ in 0..count {
        let header = data.get(offset..offset + 8).ok_or_else(invalid)?;
        let kind = u32::from_le_bytes(header[..4].try_into().unwrap());
        let size = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
        if size < 8 {
            return Err(invalid());
        }
        let command = data
            .get(offset..offset.checked_add(size).ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
        if kind == 0x2d {
            let count =
                u32::from_le_bytes(command.get(8..12).ok_or_else(invalid)?.try_into().unwrap())
                    as usize;
            let options: Vec<_> = command[12..].split(|b| *b == 0).take(count).collect();
            if options.len() != count {
                return Err(invalid());
            }
            for pair in options.windows(2) {
                if pair[0] == b"-framework" {
                    found.ui |= pair[1] == b"SwiftUI";
                    found.core |= pair[1] == b"SwiftUICore";
                }
            }
        }
        offset += size;
    }
    Ok(())
}

fn archive(
    file: &mut File,
    offset: u64,
    size: u64,
    found: &mut Frameworks,
    depth: u8,
) -> io::Result<()> {
    let mut cursor = 8;
    while cursor + 60 <= size {
        let mut header = [0; 60];
        file.seek(SeekFrom::Start(offset + cursor))?;
        file.read_exact(&mut header)?;
        if header[58..] != [0x60, b'\n'] {
            return Err(invalid());
        }
        let decimal = |bytes: &[u8]| {
            std::str::from_utf8(bytes)
                .map_err(|_| invalid())?
                .trim()
                .parse::<u64>()
                .map_err(|_| invalid())
        };
        let length = decimal(&header[48..58])?;
        let name_size = if header.starts_with(b"#1/") {
            decimal(&header[3..16])?
        } else {
            0
        };
        if name_size > length || length > size - cursor - 60 {
            return Err(invalid());
        }
        scan(
            file,
            offset + cursor + 60 + name_size,
            length - name_size,
            true,
            found,
            depth,
        )?;
        cursor += 60 + length + length % 2;
    }
    Ok(())
}

fn fat(
    file: &mut File,
    offset: u64,
    size: u64,
    header: [u8; 8],
    archives: bool,
    found: &mut Frameworks,
    depth: u8,
) -> io::Result<()> {
    let count = u32::from_be_bytes(header[4..].try_into().unwrap());
    let wide = header[3] == 0xbf;
    let stride = if wide { 32 } else { 20 };
    if count > 42 || 8 + u64::from(count) * stride > size {
        return Err(invalid());
    }
    for index in 0..count {
        let mut record = [0; 32];
        file.seek(SeekFrom::Start(offset + 8 + u64::from(index) * stride))?;
        file.read_exact(&mut record[..usize::try_from(stride).map_err(|_| invalid())?])?;
        let arch = (
            u32::from_be_bytes(record[..4].try_into().unwrap()),
            u32::from_be_bytes(record[4..8].try_into().unwrap()) & 0x00ff_ffff,
        );
        if found.arch.is_some_and(|wanted| wanted != arch) {
            continue;
        }
        let read = |range: std::ops::Range<usize>| {
            if wide {
                u64::from_be_bytes(record[range].try_into().unwrap())
            } else {
                u64::from(u32::from_be_bytes(record[range].try_into().unwrap()))
            }
        };
        let (start, length) = if wide {
            (read(8..16), read(16..24))
        } else {
            (read(8..12), read(12..16))
        };
        if start > size || length > size - start {
            return Err(invalid());
        }
        scan(file, offset + start, length, archives, found, depth)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    const OBJECT: &[u8] = include_bytes!("../../../../fixtures/linker/swiftui-autolink.bin");

    #[test]
    fn native_object_and_filelist_prefer_the_public_umbrella() {
        let dir = TempDir::new("sweetpad-autolink");
        let object = dir.join("file with spaces.o");
        std::fs::write(&object, OBJECT).unwrap();
        let path = object.display().to_string();
        let actual = prefer(vec![path.clone()], &dir.join("cache")).unwrap();
        assert_eq!(&actual[1..], std::slice::from_ref(&path));
        let mut found = Frameworks::default();
        inspect(Path::new(&actual[0]), false, &mut found).unwrap();
        assert!(found.ui && !found.core);
        let list = dir.join("inputs");
        std::fs::write(&list, "file with spaces.o\n").unwrap();
        let args = vec![
            "-filelist".into(),
            format!("{},{}", list.display(), dir.display()),
        ];
        assert_eq!(prefer(args, &dir.join("cache")).unwrap()[0], actual[0]);
        let weak = vec!["-weak_framework".into(), "SwiftUI".into(), path];
        assert_eq!(prefer(weak.clone(), &dir.join("cache")).unwrap(), weak);
    }

    #[test]
    fn only_loaded_archive_members_contribute_frameworks() {
        let dir = TempDir::new("sweetpad-autolink-archive");
        let archive = dir.join("Kit.a");
        let mut bytes = b"!<arch>\n".to_vec();
        bytes.extend_from_slice(
            format!(
                "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                "file.o/",
                0,
                0,
                0,
                "100644",
                OBJECT.len()
            )
            .as_bytes(),
        );
        bytes.extend_from_slice(OBJECT);
        if !OBJECT.len().is_multiple_of(2) {
            bytes.push(b'\n');
        }
        std::fs::write(&archive, bytes).unwrap();
        let path = archive.display().to_string();
        assert_eq!(
            prefer(vec![path.clone()], &dir.join("cache")).unwrap(),
            std::slice::from_ref(&path)
        );
        let forced = prefer(vec!["-force_load".into(), path.clone()], &dir.join("cache")).unwrap();
        assert_eq!(&forced[1..], ["-force_load", &path]);
        assert!(Path::new(&forced[0]).is_file());
    }

    #[test]
    fn a_different_architecture_and_output_paths_do_not_add_dependencies() {
        let dir = TempDir::new("sweetpad-autolink-arch");
        let object = dir.join("input.o");
        std::fs::write(&object, OBJECT).unwrap();
        let args = vec![
            "-arch".into(),
            "x86_64".into(),
            object.display().to_string(),
        ];
        assert_eq!(prefer(args.clone(), &dir.join("cache")).unwrap(), args);
        let args = vec!["-object_path_lto".into(), object.display().to_string()];
        assert_eq!(prefer(args.clone(), &dir.join("cache")).unwrap(), args);
    }
}
