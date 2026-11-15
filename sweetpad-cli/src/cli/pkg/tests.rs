//! Packages against what `productbuild` made of the same app, through the
//! files `fixtures/pkg/regen.sh` keeps: the fixture Mac app with every time
//! set to one moment, so the bill of materials and payload come out the
//! same.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use super::*;
use crate::cli::testdir::TempDir;
use crate::cli::xar::Xar;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/pkg")
}

/// The moment `regen.sh` sets every time of the app to.
const MTIME: u64 = 1_800_000_000;

/// A copy of the fixture Mac app as `regen.sh` makes it: its Info.plist
/// replaced and every time set to [`MTIME`].
fn fixture_app(dir: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/codesign/expected/Mac.app");
    let app = dir.join("Mac.app");
    crate::cli::commands::archive::copy_tree(&source, &app).unwrap();
    std::fs::copy(
        fixtures().join("Info.plist"),
        app.join("Contents/Info.plist"),
    )
    .unwrap();
    set_times(&app);
    app
}

fn set_times(path: &Path) {
    let metadata = std::fs::symlink_metadata(path).unwrap();
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).unwrap() {
            set_times(&entry.unwrap().path());
        }
    }
    let time = libc::timespec {
        tv_sec: libc::time_t::try_from(MTIME).unwrap(),
        tv_nsec: 0,
    };
    let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: a NUL-terminated path and two timespecs, as utimensat takes.
    let status = unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            name.as_ptr(),
            [time, time].as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    assert_eq!(status, 0, "{}", path.display());
}

/// Each path of a bill of materials with its record, keyed by its path.
fn bom_paths(data: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let word = |at: usize| u32::from_be_bytes(data[at..at + 4].try_into().unwrap());
    let index = word(16) as usize;
    let block = |n: u32| {
        let at = index + 4 + 8 * n as usize;
        let (start, length) = (word(at) as usize, word(at + 4) as usize);
        &data[start..start + length]
    };
    let variables = word(24) as usize;
    let mut at = variables + 4;
    let mut paths_tree = 0;
    for _ in 0..word(variables) {
        let length = data[at + 4] as usize;
        if &data[at + 5..at + 5 + length] == b"Paths" {
            paths_tree = word(at);
        }
        at += 5 + length;
    }
    let be = |bytes: &[u8], at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    let tree = block(paths_tree);
    let mut node = be(tree, 8);
    // Down the leftmost branch to the first leaf, then along the leaves.
    while u16::from_be_bytes(block(node)[..2].try_into().unwrap()) == 0 {
        node = be(block(node), 12);
    }
    let mut names = BTreeMap::new();
    let mut records = Vec::new();
    while node != 0 {
        let leaf = block(node);
        let count = u16::from_be_bytes(leaf[2..4].try_into().unwrap()) as usize;
        for entry in 0..count {
            let path = block(be(leaf, 12 + 8 * entry));
            let file = block(be(leaf, 16 + 8 * entry));
            let parent = be(file, 0);
            let name = String::from_utf8(file[4..file.len() - 1].to_vec()).unwrap();
            names.insert(be(path, 0), (parent, name));
            records.push((be(path, 0), block(be(path, 4)).to_vec()));
        }
        node = be(leaf, 4);
    }
    let full = |mut id: u32| {
        let mut parts = Vec::new();
        while let Some((parent, name)) = names.get(&id) {
            parts.push(name.clone());
            id = *parent;
        }
        parts.reverse();
        parts.join("/")
    };
    records
        .into_iter()
        .map(|(id, record)| (full(id), record))
        .collect()
}

/// The variable `name` of a bill of materials, as its block's bytes.
fn bom_variable(data: &[u8], name: &str) -> Vec<u8> {
    let word = |at: usize| u32::from_be_bytes(data[at..at + 4].try_into().unwrap());
    let index = word(16) as usize;
    let variables = word(24) as usize;
    let mut at = variables + 4;
    for _ in 0..word(variables) {
        let length = data[at + 4] as usize;
        if &data[at + 5..at + 5 + length] == name.as_bytes() {
            let entry = index + 4 + 8 * word(at) as usize;
            let (start, size) = (word(entry) as usize, word(entry + 4) as usize);
            return data[start..start + size].to_vec();
        }
        at += 5 + length;
    }
    panic!("no {name}");
}

/// Each entry of a gzip cpio payload: its header fields but the inode, and
/// its data.
fn payload_entries(gzip: &[u8]) -> BTreeMap<String, (String, Vec<u8>)> {
    let mut cpio = Vec::new();
    flate2::read::GzDecoder::new(gzip)
        .read_to_end(&mut cpio)
        .unwrap();
    assert_eq!(cpio.len() % 512, 0);
    let mut entries = BTreeMap::new();
    let mut at = 0;
    loop {
        let header = std::str::from_utf8(&cpio[at..at + 76]).unwrap();
        assert_eq!(&header[..6], "070707");
        let octal =
            |range: std::ops::Range<usize>| usize::from_str_radix(&header[range], 8).unwrap();
        let (name_size, file_size) = (octal(59..65), octal(65..76));
        let name = String::from_utf8(cpio[at + 76..at + 75 + name_size].to_vec()).unwrap();
        let data = cpio[at + 76 + name_size..at + 76 + name_size + file_size].to_vec();
        at += 76 + name_size + file_size;
        if name == "TRAILER!!!" {
            break;
        }
        // Everything but the inode, which follows the order paths are
        // written in.
        let fields = format!("{}{}", &header[..12], &header[18..]);
        entries.insert(name, (fields, data));
    }
    entries
}

fn expanded(package: &Path) -> BTreeMap<String, Vec<u8>> {
    let xar = Xar::open(package).unwrap();
    xar.files()
        .into_iter()
        .filter_map(|(path, entry)| Some((path, xar.read(entry).ok()?)))
        .collect()
}

#[test]
fn a_package_holds_what_productbuild_writes() {
    let dir = TempDir::new("pkg-product");
    let app = fixture_app(&dir);
    let package = dir.join("Mac.pkg");
    product(&app, None, &package).unwrap();
    let files = expanded(&package);
    let component = "dev.sweetpad.macfixture.pkg";
    assert_eq!(
        files.keys().collect::<Vec<_>>(),
        [
            "Distribution".to_string(),
            format!("{component}/Bom"),
            format!("{component}/PackageInfo"),
            format!("{component}/Payload"),
        ]
        .iter()
        .collect::<Vec<_>>()
    );
    let expected = |name: &str| std::fs::read(fixtures().join(name)).unwrap();

    let bom = &files[&format!("{component}/Bom")];
    let ours = bom_paths(bom);
    let theirs = bom_paths(&expected("Bom"));
    assert_eq!(
        ours.keys().collect::<Vec<_>>(),
        theirs.keys().collect::<Vec<_>>()
    );
    for (path, record) in &theirs {
        assert_eq!(&ours[path], record, "the record of {path}");
    }
    for variable in ["BomInfo", "VIndex"] {
        assert_eq!(
            bom_variable(bom, variable).len(),
            bom_variable(&expected("Bom"), variable).len(),
            "{variable}"
        );
    }
    assert_eq!(
        bom_variable(bom, "BomInfo"),
        bom_variable(&expected("Bom"), "BomInfo")
    );

    let ours = payload_entries(&files[&format!("{component}/Payload")]);
    let theirs = payload_entries(&expected("Payload"));
    assert_eq!(
        ours.keys().collect::<Vec<_>>(),
        theirs.keys().collect::<Vec<_>>()
    );
    for (path, entry) in &theirs {
        if path == "." {
            // productbuild's root is a temporary folder: its mode and time
            // are that folder's. The owner and link count are not.
            assert_eq!(ours[path].0[18..36], entry.0[18..36]);
            continue;
        }
        assert_eq!(&ours[path], entry, "the payload entry of {path}");
    }

    let generator = format!(
        "generator-version=\"sweetpad-{}\"",
        env!("CARGO_PKG_VERSION")
    );
    let package_info = String::from_utf8(files[&format!("{component}/PackageInfo")].clone())
        .unwrap()
        .replace(&generator, "generator-version=\"\"");
    let reference = String::from_utf8(expected("PackageInfo")).unwrap();
    let start = reference.find("generator-version=\"").unwrap() + "generator-version=\"".len();
    let end = start + reference[start..].find('"').unwrap();
    let reference = format!("{}{}", &reference[..start], &reference[end..]);
    assert_eq!(package_info, reference);
    assert_eq!(
        String::from_utf8(files["Distribution"].clone()).unwrap(),
        String::from_utf8(expected("Distribution")).unwrap()
    );
}

#[test]
fn a_signed_package_carries_both_signatures_over_its_table_of_contents() {
    let dir = TempDir::new("pkg-signed");
    let app = fixture_app(&dir);
    let package = dir.join("Mac.pkg");
    let installer = Installer::fixture();
    product(&app, Some(&installer), &package).unwrap();
    let data = std::fs::read(&package).unwrap();
    assert_eq!(&data[..4], b"xar!");
    let compressed = usize::try_from(u64::from_be_bytes(data[8..16].try_into().unwrap())).unwrap();
    let toc = &data[28..28 + compressed];
    assert_eq!(&toc[..2], [0x78, 0xda]);
    let heap = &data[28 + compressed..];
    let checksum = crate::cli::codesign::sha1(toc);
    assert_eq!(heap[..20], checksum);
    let size = installer.signature_size();
    assert_eq!(heap[20..20 + size], installer.sign(&checksum).unwrap());
    assert_eq!(heap[20 + size], 0x30, "a CMS signature follows");
    let mut xml = String::new();
    flate2::read::ZlibDecoder::new(toc)
        .read_to_string(&mut xml)
        .unwrap();
    for needle in [
        "<signature style=\"RSA\">",
        "<x-signature style=\"CMS\">",
        "<offset>6420</offset>",
    ] {
        assert!(xml.contains(needle), "{needle}");
    }
    // The files read back the same as unsigned.
    let files = expanded(&package);
    assert!(files.contains_key("Distribution"));
}

#[test]
fn the_package_version_has_three_parts() {
    let app = |version: &str| App {
        name: String::new(),
        id: String::new(),
        title: None,
        short_version: Some(version.to_string()),
        version: None,
        minimum_system: None,
        architectures: Vec::new(),
    };
    for (version, expected) in [
        ("1", "1.0.0"),
        ("1.0", "1.0.0"),
        ("2.3.4", "2.3.4"),
        ("1.2.3.4", "1.2.3.4"),
    ] {
        assert_eq!(app(version).package_version(), expected);
    }
    let mut unversioned = app("");
    unversioned.short_version = None;
    assert_eq!(unversioned.package_version(), "0");
}

#[test]
fn cksum_matches_posix() {
    assert_eq!(payload::cksum(b""), 4_294_967_295);
    assert_eq!(payload::cksum(b"hi\n"), 0x5835_334a);
}

/// Write a package of the app in `SWEETPAD_PKG_APP` to `SWEETPAD_PKG_OUT`,
/// to compare with `productbuild` by hand.
#[test]
#[ignore = "writes where SWEETPAD_PKG_OUT says"]
fn write_a_package() {
    let app = std::env::var_os("SWEETPAD_PKG_APP").unwrap();
    let out = std::env::var_os("SWEETPAD_PKG_OUT").unwrap();
    product(Path::new(&app), None, Path::new(&out)).unwrap();
}
