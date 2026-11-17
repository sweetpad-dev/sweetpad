//! Extraction over the archives Apple's own `aa` and `aea` tools wrote
//! (`sweetpad-lib/ci/fixtures/xcode-archives`, see its README), and over
//! small hand-made archives for what Apple's tools never write.

#![allow(clippy::cast_possible_truncation)] // small test archives

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::Digest;

use super::{Keys, extract, source};

pub(crate) const HEADER: &str =
    "Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/include/swift/bridging";
pub(crate) const VERSION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \
     \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
     <plist version=\"1.0\">\n<dict>\n\t<key>CFBundleShortVersionString</key>\n\t\
     <string>27.2</string>\n\t<key>ProductBuildVersion</key>\n\t<string>fixture</string>\n\
     </dict>\n</plist>\n";

fn data() -> Vec<u8> {
    b"Sweetpad archive fixture header\n".repeat(10000)
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sweetpad-lib/ci/fixtures/xcode-archives")
        .join(name)
}

fn fixtures(extension: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(fixture(""))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == extension))
        .collect();
    paths.sort();
    paths
}

/// A folder removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "sweetpad-archive-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn key_file(scratch: &Scratch, bytes: &[u8]) -> Keys {
    let path = scratch.0.join("key");
    std::fs::write(&path, bytes).unwrap();
    Keys::read(Some(&path), None).unwrap()
}

fn extract_to(source: &Path, out: &Path, keys: &Keys) -> Result<PathBuf, String> {
    extract(source, out, keys, None)
}

fn assert_extract(source: &Path, out: &Path, keys: &Keys) -> PathBuf {
    let app = extract_to(source, out, keys).unwrap_or_else(|e| panic!("{}: {e}", source.display()));
    let header = app.join(HEADER.strip_prefix("Xcode.app/").unwrap());
    assert_eq!(std::fs::read(&header).unwrap(), data());
    let link = header.with_file_name("link");
    assert!(link.symlink_metadata().unwrap().is_symlink());
    assert_eq!(std::fs::read(&link).unwrap(), data());
    assert_eq!(std::fs::read(header.with_file_name("empty")).unwrap(), b"");
    assert!(!app.join("Contents/Resources").exists());
    assert_eq!(
        std::fs::read_to_string(app.join("Contents/version.plist")).unwrap(),
        VERSION
    );
    app
}

#[test]
fn the_fixtures_are_the_ones_recorded() {
    let sums: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture("checksums.json")).unwrap()).unwrap();
    for (name, sum) in sums.as_object().unwrap() {
        let digest = sha2::Sha256::digest(std::fs::read(fixture(name)).unwrap());
        let hex = super::hex(&digest);
        assert_eq!(hex, sum.as_str().unwrap(), "{name}");
    }
}

#[test]
fn every_codec_decodes_to_the_same_archive() {
    let raw = std::fs::read(fixture("raw.aar")).unwrap();
    for name in [
        "lz4.aar",
        "lzfse.aar",
        "lzma.aar",
        "lzraven.aar",
        "zlib.aar",
    ] {
        let mut decoded = Vec::new();
        source::open(&fixture(name), &Keys::default())
            .unwrap()
            .read_to_end(&mut decoded)
            .unwrap();
        assert!(decoded == raw, "{name}");
    }
}

#[test]
fn apple_archives_and_xips_extract_with_hard_links_outside_the_sdk() {
    let scratch = Scratch::new("formats");
    for source in fixtures("aar").into_iter().chain(fixtures("xip")) {
        let out = scratch.0.join(source.file_name().unwrap());
        assert_extract(&source, &out, &Keys::default());
    }
}

#[test]
fn shared_clusters_keep_each_files_mode() {
    let scratch = Scratch::new("clusters");
    for name in ["clusters-full.aar", "clusters-deduplicated.aar"] {
        let app = assert_extract(&fixture(name), &scratch.0.join(name), &Keys::default());
        let folder = app
            .join(HEADER.strip_prefix("Xcode.app/").unwrap())
            .parent()
            .unwrap()
            .to_path_buf();
        let mode = |name: &str| {
            std::os::unix::fs::PermissionsExt::mode(
                &std::fs::metadata(folder.join(name)).unwrap().permissions(),
            ) & 0o777
        };
        assert_eq!(mode("bridging"), 0o644);
        assert_eq!(std::fs::read(folder.join("copy")).unwrap(), data());
        assert_eq!(mode("copy"), 0o755);
        assert_eq!(std::fs::read(folder.join("empty-copy")).unwrap(), b"");
        assert!(!app.join("Contents/AAData").exists());
    }
}

#[test]
fn signed_and_encrypted_archives_extract() {
    let scratch = Scratch::new("aea");
    for name in [
        "profile0.aea",
        "profile1.aea",
        "profile2.aea",
        "profile1-clusters.aea",
    ] {
        assert_extract(&fixture(name), &scratch.0.join(name), &Keys::default());
    }
}

#[test]
fn keys_an_archive_doesnt_carry_come_from_files() {
    let scratch = Scratch::new("external");
    let err = extract_to(
        &fixture("profile0-external.aea"),
        &scratch.0.join("a"),
        &Keys::default(),
    )
    .unwrap_err();
    assert!(err.contains("public key is not embedded"), "{err}");
    let public = Keys::read(None, Some(&fixture("signing-public.pem"))).unwrap();
    assert_extract(
        &fixture("profile0-external.aea"),
        &scratch.0.join("b"),
        &public,
    );

    let err = extract_to(
        &fixture("profile1-external.aea"),
        &scratch.0.join("c"),
        &Keys::default(),
    )
    .unwrap_err();
    assert!(err.contains("key is not embedded"), "{err}");
    let key: Vec<u8> = (0..32).collect();
    assert_extract(
        &fixture("profile1-external.aea"),
        &scratch.0.join("d"),
        &key_file(&scratch, &key),
    );
    let err = extract_to(
        &fixture("profile1-external.aea"),
        &scratch.0.join("e"),
        &key_file(&scratch, &[0; 32]),
    )
    .unwrap_err();
    assert!(err.contains("authentication failed"), "{err}");
}

#[test]
fn a_flipped_bit_anywhere_fails_authentication() {
    let scratch = Scratch::new("modified");
    let key: Vec<u8> = (0..32).collect();
    for name in [
        "profile0.aea",
        "profile1.aea",
        "profile2.aea",
        "profile1-external.aea",
    ] {
        let original = std::fs::read(fixture(name)).unwrap();
        for offset in [original.len() / 2, original.len() - 1] {
            let mut data = original.clone();
            data[offset] ^= 0x80;
            let source = scratch.0.join(format!("{name}{offset}"));
            std::fs::write(&source, &data).unwrap();
            let keys = if name.contains("external") {
                key_file(&scratch, &key)
            } else {
                Keys::default()
            };
            let mut decoded = Vec::new();
            let err = source::open(&source, &keys)
                .and_then(|mut s| s.read_to_end(&mut decoded).map_err(|e| e.to_string()))
                .unwrap_err();
            assert!(
                ["authentication", "signature", "checksum"]
                    .iter()
                    .any(|word| err.contains(word)),
                "{name} at {offset}: {err}"
            );
        }
    }
}

/// The RFC 9180 test recipient's private key, as PKCS #8 PEM.
fn fcs_recipient() -> Vec<u8> {
    use p256::pkcs8::EncodePrivateKey as _;
    let vector: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture("rfc9180-p256.json")).unwrap()).unwrap();
    let hex = vector["skRm"].as_str().unwrap();
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    p256::SecretKey::from_slice(&bytes)
        .unwrap()
        .to_pkcs8_pem(p256::pkcs8::LineEnding::default())
        .unwrap()
        .as_bytes()
        .to_vec()
}

#[test]
fn an_fcs_envelope_opens_with_the_recipients_key() {
    let scratch = Scratch::new("fcs");
    let pem = fcs_recipient();
    let mut decoded = Vec::new();
    source::open(&fixture("profile1-fcs.aea"), &key_file(&scratch, &pem))
        .unwrap()
        .read_to_end(&mut decoded)
        .unwrap();
    assert!(decoded == std::fs::read(fixture("raw.aar")).unwrap());

    // Without a key file, the envelope's key comes from the URL it names.
    let archive = std::fs::read(fixture("profile1-fcs.aea")).unwrap();
    let auth_size = u32::from_le_bytes(archive[8..12].try_into().unwrap()) as usize;
    let mut urls = Vec::new();
    let key = super::fcs::unwrap(&archive[12..12 + auth_size], None, &mut |url| {
        urls.push(url.to_string());
        Ok(pem.clone())
    })
    .unwrap();
    assert!(key.is_some());
    assert_eq!(urls.len(), 1);
    super::fcs::validate_url(&urls[0]).unwrap();
}

/// An Apple Archive file record, for archives Apple's encoder won't write.
fn record(
    name: &str,
    data: Option<&[u8]>,
    clusters: &[(&str, u64)],
    size: Option<u64>,
    checksum: Option<[u8; 32]>,
) -> Vec<u8> {
    let mut fields = b"TYP1F".to_vec();
    fields.extend(b"PATP");
    fields.extend((name.len() as u16).to_le_bytes());
    fields.extend(name.as_bytes());
    fields.extend(b"MOD2");
    fields.extend(0o644u16.to_le_bytes());
    for (key, id) in clusters {
        fields.extend(key.as_bytes());
        fields.push(b'8');
        fields.extend(id.to_le_bytes());
    }
    if let Some(size) = size {
        fields.extend(b"SIZ8");
        fields.extend(size.to_le_bytes());
    }
    if let Some(checksum) = checksum {
        fields.extend(b"SH2H");
        fields.extend(checksum);
    }
    if let Some(data) = data {
        fields.extend(b"DATC");
        fields.extend((data.len() as u64).to_le_bytes());
    }
    let mut out = b"AA01".to_vec();
    out.extend((fields.len() as u16 + 6).to_le_bytes());
    out.extend(fields);
    out.extend(data.unwrap_or_default());
    out
}

fn sha256(data: &[u8]) -> [u8; 32] {
    sha2::Sha256::digest(data).into()
}

#[test]
fn clusters_join_across_namespaces_whatever_comes_first() {
    let scratch = Scratch::new("transitive");
    let data = b"shared SDK bytes";
    let provider = record(
        "Xcode.app/Contents/Resources/shared",
        Some(data),
        &[("HLC", 0)],
        None,
        None,
    );
    let records = [
        record(HEADER, None, &[("HLC", 0), ("CLC", 12)], None, None),
        record(
            &format!("{HEADER}.bridge"),
            None,
            &[("CLC", 12), ("SLC", 4)],
            Some(data.len() as u64),
            None,
        ),
        record(
            &format!("{HEADER}.last"),
            None,
            &[("SLC", 4)],
            None,
            Some(sha256(data)),
        ),
        // The same number in another namespace is another cluster.
        record(
            &format!("{HEADER}.other"),
            Some(b"other"),
            &[("SLC", 0)],
            None,
            None,
        ),
    ]
    .concat();
    for (index, archive) in [
        [&provider[..], &records].concat(),
        [&records[..], &provider].concat(),
    ]
    .iter()
    .enumerate()
    {
        let source = scratch.0.join(format!("transitive-{index}.aar"));
        std::fs::write(&source, archive).unwrap();
        let app = extract_to(
            &source,
            &scratch.0.join(format!("out-{index}")),
            &Keys::default(),
        )
        .unwrap();
        let header = app.join(HEADER.strip_prefix("Xcode.app/").unwrap());
        for name in ["bridging", "bridging.bridge", "bridging.last"] {
            assert_eq!(std::fs::read(header.with_file_name(name)).unwrap(), data);
        }
        assert_eq!(
            std::fs::read(header.with_file_name("bridging.other")).unwrap(),
            b"other"
        );
        assert!(!app.join("Contents/Resources").exists());
    }
}

#[test]
fn bad_shared_data_fails_before_anything_is_written() {
    let scratch = Scratch::new("invalid-clusters");
    let provider = record(
        "Xcode.app/Contents/Resources/shared",
        Some(b"abc"),
        &[("SLC", 0)],
        None,
        None,
    );
    let cases = [
        (
            record(HEADER, None, &[("SLC", 0)], Some(3), None),
            "missing Apple Archive shared file data",
        ),
        (
            [
                provider.clone(),
                record(HEADER, Some(b"xyz"), &[("SLC", 0)], None, None),
            ]
            .concat(),
            "conflicting Apple Archive shared file data",
        ),
        (
            [
                provider.clone(),
                record(HEADER, None, &[("SLC", 0)], Some(4), None),
            ]
            .concat(),
            "shared file size",
        ),
        (
            [
                provider.clone(),
                record(HEADER, None, &[("SLC", 0)], None, Some([0; 32])),
            ]
            .concat(),
            "shared file checksum",
        ),
        (
            record(HEADER, Some(b"abc"), &[], None, Some([0; 32])),
            "file checksum failed",
        ),
        (record(HEADER, None, &[], Some(3), None), "missing its data"),
    ];
    for (index, (archive, error)) in cases.iter().enumerate() {
        let source = scratch.0.join(format!("invalid-{index}.aar"));
        std::fs::write(&source, archive).unwrap();
        let out = scratch.0.join(format!("out-{index}"));
        let err = extract_to(&source, &out, &Keys::default()).unwrap_err();
        assert!(err.contains(error), "{index}: {err}");
        assert!(!out.exists(), "{index}");
    }
}

#[test]
fn an_empty_shared_cluster_without_data_stays_empty() {
    let scratch = Scratch::new("empty-cluster");
    let source = scratch.0.join("empty.aar");
    std::fs::write(
        &source,
        record(HEADER, None, &[("SLC", 0)], Some(0), Some(sha256(b""))),
    )
    .unwrap();
    let app = extract_to(&source, &scratch.0.join("out"), &Keys::default()).unwrap();
    let header = app.join(HEADER.strip_prefix("Xcode.app/").unwrap());
    assert_eq!(std::fs::read(header).unwrap(), b"");
}

/// A CPIO entry: name, data, mode, inode and link count.
pub(crate) type CpioItem<'a> = (&'a str, &'a [u8], u32, u32, u32);

/// An odc CPIO archive of `items`.
pub(crate) fn cpio(items: &[CpioItem<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data, mode, inode, links) in items.iter().chain([&("TRAILER!!!", &b""[..], 0, 0, 1)])
    {
        let name = format!("{name}\0");
        out.extend(
            format!(
                "070707{:06o}{inode:06o}{mode:06o}{:06o}{:06o}{links:06o}{:06o}{:011o}{:06o}{:011o}",
                1,
                0,
                0,
                0,
                0,
                name.len(),
                data.len()
            )
            .as_bytes(),
        );
        out.extend(name.as_bytes());
        out.extend(*data);
    }
    out
}

/// A `.xip` around `payload`, stored in a single pbzx block.
pub(crate) fn xip(payload: &[u8]) -> Vec<u8> {
    let mut content = b"pbzx".to_vec();
    for n in [1u64 << 24, payload.len() as u64, payload.len() as u64] {
        content.extend(n.to_be_bytes());
    }
    content.extend(payload);
    let sum = super::hex(&sha256(&content));
    let toc = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><xar><toc><file id=\"1\"><name>Content</name>\
         <data><offset>32</offset><length>{}</length>\
         <encoding style=\"application/octet-stream\"/>\
         <archived-checksum style=\"sha256\">{sum}</archived-checksum></data></file></toc></xar>",
        content.len()
    );
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(toc.as_bytes()).unwrap();
    let packed = encoder.finish().unwrap();
    let mut out = b"xar!".to_vec();
    out.extend(28u16.to_be_bytes());
    out.extend(1u16.to_be_bytes());
    out.extend((packed.len() as u64).to_be_bytes());
    out.extend((toc.len() as u64).to_be_bytes());
    out.extend(3u32.to_be_bytes());
    out.extend(&packed);
    out.extend(sha256(&packed));
    out.extend(content);
    out
}

#[test]
fn a_hand_made_xip_extracts() {
    let scratch = Scratch::new("hand-made");
    let source = scratch.0.join("ok.xip");
    std::fs::write(&source, xip(&cpio(&[(HEADER, b"header", 0o100_644, 1, 1)]))).unwrap();
    let app = extract_to(&source, &scratch.0.join("out"), &Keys::default()).unwrap();
    let header = app.join(HEADER.strip_prefix("Xcode.app/").unwrap());
    assert_eq!(std::fs::read(header).unwrap(), b"header");
}

#[test]
fn names_links_and_parents_never_leave_the_staging_folder() {
    let scratch = Scratch::new("unsafe");
    let parent = Path::new(HEADER).parent().unwrap().to_str().unwrap();
    let cases: [&[CpioItem<'_>]; 4] = [
        &[("../escape", b"bad", 0o100_644, 1, 1)],
        &[(HEADER, b"../../../../../../../../escape", 0o120_777, 1, 1)],
        &[
            (parent, b"folder", 0o120_777, 1, 1),
            (HEADER, b"bad", 0o100_644, 2, 1),
        ],
        &[
            (HEADER, b"a", 0o100_644, 1, 1),
            (HEADER, b"b", 0o100_644, 2, 1),
        ],
    ];
    for (index, entries) in cases.iter().enumerate() {
        let source = scratch.0.join(format!("unsafe{index}.xip"));
        std::fs::write(&source, xip(&cpio(entries))).unwrap();
        assert!(
            extract_to(
                &source,
                &scratch.0.join(format!("stage{index}")),
                &Keys::default()
            )
            .is_err(),
            "{index}"
        );
        assert!(!scratch.0.join("escape").exists());
    }
}

#[test]
fn a_damaged_or_cut_off_xip_fails() {
    let scratch = Scratch::new("damaged");
    let original = std::fs::read(fixture("odc-pbzx.xip")).unwrap();
    let mut bad_toc = original.clone();
    bad_toc[35] ^= 0x80;
    let mut zeroed = original[..original.len() - 20].to_vec();
    zeroed.extend([0; 20]);
    for (index, data) in [
        original[..original.len() - 1].to_vec(),
        original[..28].to_vec(),
        zeroed,
        bad_toc,
    ]
    .iter()
    .enumerate()
    {
        let source = scratch.0.join(format!("bad{index}.xip"));
        std::fs::write(&source, data).unwrap();
        assert!(
            extract_to(
                &source,
                &scratch.0.join(format!("out{index}")),
                &Keys::default()
            )
            .is_err(),
            "{index}"
        );
    }
}
