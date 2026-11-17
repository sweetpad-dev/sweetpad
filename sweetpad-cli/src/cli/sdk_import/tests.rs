use std::path::{Path, PathBuf};

use super::*;
use crate::cli::apple_archive::tests::{VERSION, cpio, xip};

const TOOLCHAIN: Toolchain<'static> = Toolchain {
    swift: "6.3",
    swift_build: "swift-6.3-RELEASE",
};
const REQUIRED: [&str; 3] = [
    "swift-sdk.json",
    "version.plist",
    "Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/clang/include/stddef.h",
];

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("sweetpad-sdk-import-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

/// An export as the Mac writes one, from `files` and `links`, with
/// `omitted` members left out and `changed` ones holding other bytes.
#[derive(Default)]
struct Export<'a> {
    files: Vec<(&'a str, &'a [u8])>,
    links: Vec<(&'a str, &'a str)>,
    omitted: Vec<&'a str>,
    changed: Vec<&'a str>,
    version: u64,
}

impl Export<'_> {
    fn write(&self, path: &Path) -> PathBuf {
        let default: Vec<(&str, &[u8])> = vec![
            ("Xcode.app/Contents/version.plist", b"version"),
            ("appintents-phrases.json", b"{}"),
        ];
        let files = if self.files.is_empty() {
            &default
        } else {
            &self.files
        };
        let mut entries = serde_json::Map::new();
        for (name, data) in files {
            entries.insert(
                (*name).into(),
                json!({"type": "file", "size": data.len(), "mode": 0o644, "sha256": hex(&Sha256::digest(data))}),
            );
        }
        for (name, target) in &self.links {
            entries.insert((*name).into(), json!({"type": "symlink", "target": target}));
        }
        let manifest = json!({
            "format": EXPORT_FORMAT,
            "version": if self.version == 0 { 1 } else { self.version },
            "toolchain": TOOLCHAIN.json(),
            "entries": entries,
            "xcodeBuild": "test",
            "xcodeVersion": "27.2",
        });
        let file = File::create(path).unwrap();
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::fast(),
        ));
        let append = |tar: &mut tar::Builder<_>, name: &str, data: &[u8]| {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            // The name goes in as it is, `..` and all, as a hostile archive's would.
            header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
            header.set_cksum();
            tar.append(&header, data).unwrap();
        };
        append(&mut tar, "manifest.json", manifest.to_string().as_bytes());
        for (name, data) in files {
            if self.omitted.contains(name) {
                continue;
            }
            let changed = vec![b'x'; data.len()];
            append(
                &mut tar,
                name,
                if self.changed.contains(name) {
                    &changed
                } else {
                    data
                },
            );
        }
        for (name, target) in &self.links {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            tar.append_link(&mut header, name, target).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        fs::write(
            path.with_file_name(format!("{name}.sha256")),
            format!("{}  {name}\n", sha256_file(path).unwrap()),
        )
        .unwrap();
        path.to_path_buf()
    }
}

fn unpack(scratch: &Scratch, export: &Export<'_>) -> Result<Value, String> {
    let archive = export.write(&scratch.0.join("sdk.tar.gz"));
    let out = scratch.0.join("unpacked");
    fs::create_dir(&out).unwrap();
    unpack_export(&archive, &out, &TOOLCHAIN)
}

#[test]
fn an_export_unpacks_with_its_links_and_bytes() {
    let scratch = Scratch::new("round-trip");
    let manifest = unpack(
        &scratch,
        &Export {
            links: vec![("Xcode.app/Contents/current.plist", "version.plist")],
            ..Export::default()
        },
    )
    .unwrap();
    assert_eq!(manifest["xcodeBuild"], "test");
    let link = scratch.0.join("unpacked/Xcode.app/Contents/current.plist");
    assert_eq!(fs::read(&link).unwrap(), b"version");
    assert!(link.symlink_metadata().unwrap().is_symlink());
}

#[test]
fn an_export_that_is_unsafe_damaged_or_incomplete_is_refused() {
    let cases: Vec<(Export<'_>, &str)> = vec![
        (
            Export {
                files: vec![("Xcode.app/Contents/../../../escape", b"bad")],
                ..Export::default()
            },
            "Unsafe",
        ),
        (
            Export {
                links: vec![("Xcode.app/Contents/link", "../../../../escape")],
                ..Export::default()
            },
            "",
        ),
        (
            Export {
                links: vec![("Xcode.app/Contents/link", "/etc/passwd")],
                ..Export::default()
            },
            "",
        ),
        (
            Export {
                changed: vec!["appintents-phrases.json"],
                ..Export::default()
            },
            "checksum mismatch",
        ),
        (
            Export {
                omitted: vec!["appintents-phrases.json"],
                ..Export::default()
            },
            "missing manifest",
        ),
        (
            Export {
                version: 99,
                ..Export::default()
            },
            "Unsupported",
        ),
    ];
    for (index, (export, error)) in cases.iter().enumerate() {
        let scratch = Scratch::new(&format!("refused-{index}"));
        let err = unpack(&scratch, export).unwrap_err();
        assert!(err.contains(error), "{index}: {err}");
        assert!(!scratch.0.join("escape").exists());
    }
}

#[test]
fn the_checksum_file_names_the_archive_even_with_spaces() {
    let scratch = Scratch::new("checksum");
    let archive = Export::default().write(&scratch.0.join("Apple SDK.tar.gz"));
    let inspected = inspect(&archive, &TOOLCHAIN).unwrap();
    assert_eq!(inspected.checksum, sha256_file(&archive).unwrap());
    assert_eq!(inspected.format, "sweetpad-export");
    let mut file = fs::OpenOptions::new().append(true).open(&archive).unwrap();
    file.write_all(b"changed").unwrap();
    let err = inspect(&archive, &TOOLCHAIN).err().unwrap();
    assert!(err.contains("checksum"), "{err}");
}

fn options<'a>(
    source: Source<'a>,
    destination: &'a Path,
    replace: bool,
    keys: &'a Keys,
) -> Import<'a> {
    Import {
        source,
        destination,
        replace,
        keys,
        phrases: None,
        build_machine_os_build: None,
        toolchain: TOOLCHAIN,
        clang_headers: Path::new("/nonexistent"),
    }
}

/// A bundle writer that writes only what an import checks for.
#[allow(clippy::unnecessary_wraps)] // the bundle writer's signature
fn stub_bundle(inputs: &super::super::sdk_bundle::Inputs<'_>, out: &Path) -> Result<(), String> {
    assert!(inputs.xcode.join("Contents").is_dir());
    for name in REQUIRED {
        write(&out.join(name), b"{}");
    }
    Ok(())
}

#[test]
fn a_folder_that_isnt_an_sdk_is_never_replaced() {
    let scratch = Scratch::new("personal");
    let archive = Export::default().write(&scratch.0.join("sdk.tar.gz"));
    let destination = scratch.0.join("personal");
    write(&destination.join("keep"), b"keep");
    let keys = Keys::default();
    let err = import_locked(
        &options(Source::Archive(&archive), &destination, true, &keys),
        &destination,
        "aarch64",
        &mut |_| {},
        &mut stub_bundle,
    )
    .unwrap_err();
    assert!(err.contains("not a Darwin SDK"), "{err}");
    assert_eq!(fs::read(destination.join("keep")).unwrap(), b"keep");
}

#[test]
fn a_failed_import_keeps_the_sdk_it_would_replace() {
    let scratch = Scratch::new("rollback");
    let archive = Export::default().write(&scratch.0.join("sdk.tar.gz"));
    let destination = scratch.0.join("existing-sdk");
    write(&destination.join("important"), b"previous SDK");
    write(&destination.join("swift-sdk.json"), b"{}");
    let keys = Keys::default();
    let err = import_locked(
        &options(Source::Archive(&archive), &destination, true, &keys),
        &destination,
        "aarch64",
        &mut |_| {},
        &mut |_, _| Err("builder failed".into()),
    )
    .unwrap_err();
    assert_eq!(err, "builder failed");
    assert_eq!(
        fs::read(destination.join("important")).unwrap(),
        b"previous SDK"
    );
    let leftovers: Vec<_> = fs::read_dir(&scratch.0)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".sweetpad-sdk-")
        })
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn an_export_imports_and_replaces_only_when_asked() {
    let scratch = Scratch::new("replace");
    let archive = Export::default().write(&scratch.0.join("sdk.tar.gz"));
    let destination = scratch.0.join("sdk");
    let keys = Keys::default();
    let mut written = 0;
    let mut writer = |inputs: &super::super::sdk_bundle::Inputs<'_>, out: &Path| {
        written += 1;
        assert!(inputs.phrases.is_some_and(Path::is_file));
        stub_bundle(inputs, out)?;
        write(&out.join(PHRASES), b"{}");
        Ok(())
    };
    let first = import_locked(
        &options(Source::Archive(&archive), &destination, false, &keys),
        &destination,
        "aarch64",
        &mut |_| {},
        &mut writer,
    )
    .unwrap();
    assert_eq!(first["reused"], false);
    assert_eq!(first["sourceFormat"], "sweetpad-export");
    let again = import_locked(
        &options(Source::Archive(&archive), &destination, false, &keys),
        &destination,
        "aarch64",
        &mut |_| {},
        &mut writer,
    )
    .unwrap();
    assert_eq!(again["reused"], true);
    let replaced = import_locked(
        &options(Source::Archive(&archive), &destination, true, &keys),
        &destination,
        "aarch64",
        &mut |_| {},
        &mut writer,
    )
    .unwrap();
    assert_eq!(replaced["reused"], false);
    assert_eq!(written, 2);
}

#[test]
fn an_old_recipe_or_toolchain_needs_an_explicit_replace() {
    let scratch = Scratch::new("recipe");
    let archive = Export::default().write(&scratch.0.join("sdk.tar.gz"));
    let destination = scratch.0.join("cached-sdk");
    for name in REQUIRED.iter().chain(&[PHRASES, "keep"]) {
        write(&destination.join(name), b"cached data");
    }
    let current = json!({
        "archiveSha256": sha256_file(&archive).unwrap(),
        "hostArchitecture": "aarch64",
        "importerVersion": IMPORTER_VERSION,
        "toolchain": TOOLCHAIN.json(),
    });
    let keys = Keys::default();
    let receipt_path = destination.join("sweetpad-sdk.json");
    fs::write(&receipt_path, current.to_string()).unwrap();
    let reused = import_locked(
        &options(Source::Archive(&archive), &destination, false, &keys),
        &destination,
        "aarch64",
        &mut |_| {},
        &mut |_, _| panic!("a current SDK isn't rebuilt"),
    )
    .unwrap();
    assert_eq!(reused["reused"], true);
    assert_eq!(reused["importerVersion"], IMPORTER_VERSION);

    let mut legacy = current.clone();
    legacy.as_object_mut().unwrap().remove("importerVersion");
    let mut old_tools = current.clone();
    old_tools["toolchain"]["swift"] = json!("old Swift");
    for receipt in [legacy, old_tools] {
        fs::write(&receipt_path, receipt.to_string()).unwrap();
        let err = import_locked(
            &options(Source::Archive(&archive), &destination, false, &keys),
            &destination,
            "aarch64",
            &mut |_| {},
            &mut |_, _| panic!("an old SDK isn't replaced unasked"),
        )
        .unwrap_err();
        assert!(
            err.contains("recipe or toolchain changed;") && err.contains("--replace-sdk"),
            "{err}"
        );
        assert_eq!(fs::read(destination.join("keep")).unwrap(), b"cached data");
        let kept: Value = serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
        assert_eq!(kept, receipt);
    }
}

#[test]
fn an_export_leaves_out_signing_files_and_host_executables() {
    let scratch = Scratch::new("export-entries");
    let xcode = scratch.0.join("Xcode.app");
    for platform in BASE_PLATFORMS {
        fs::create_dir_all(xcode.join(format!(
            "Contents/Developer/Platforms/{platform}.platform/Developer/SDKs"
        )))
        .unwrap();
    }
    for relative in [
        "Contents/version.plist",
        "Contents/development.key",
        "Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/swift",
        "Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/iphoneos/prebuilt-modules/x",
    ] {
        write(&xcode.join(relative), b"test");
    }
    let names: Vec<String> = export_entries(&xcode)
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names, ["Xcode.app/Contents/version.plist"]);
}

#[test]
fn an_xcode_archive_import_takes_the_build_machine_from_xcode() {
    let scratch = Scratch::new("xip-build-machine");
    let info = property_list::to_xml(&property_list::Plist::Dict(vec![
        (
            "CFBundleIdentifier".into(),
            property_list::Plist::String("com.apple.dt.Xcode".into()),
        ),
        (
            "BuildMachineOSBuild".into(),
            property_list::Plist::String("25C56".into()),
        ),
    ]));
    let sdks: Vec<String> = BASE_PLATFORMS
        .iter()
        .map(|name| {
            format!("Xcode.app/Contents/Developer/Platforms/{name}.platform/Developer/SDKs/{name}.sdk/SDKSettings.json")
        })
        .collect();
    let keys = Keys::default();
    for (name, with_info, expected) in [
        ("with-info", true, Some("25C56")),
        ("without-info", false, None),
    ] {
        let mut items: Vec<(&str, &[u8], u32, u32, u32)> = Vec::new();
        if with_info {
            items.push((
                "Xcode.app/Contents/Info.plist",
                info.as_bytes(),
                0o100_644,
                1,
                1,
            ));
        }
        items.push((
            "Xcode.app/Contents/version.plist",
            VERSION.as_bytes(),
            0o100_644,
            2,
            1,
        ));
        for (index, sdk) in sdks.iter().enumerate() {
            items.push((sdk, b"{}", 0o100_644, 10 + index as u32, 1));
        }
        let archive = scratch.0.join(format!("{name}.xip"));
        fs::write(&archive, xip(&cpio(&items))).unwrap();
        let destination = scratch.0.join(format!("{name}-sdk"));
        let mut seen = None;
        let report = import_locked(
            &options(Source::Archive(&archive), &destination, false, &keys),
            &destination,
            "x86_64",
            &mut |_| {},
            &mut |inputs, out| {
                seen = Some(inputs.build_machine_os_build.map(str::to_string));
                stub_bundle(inputs, out)
            },
        )
        .unwrap();
        assert_eq!(seen.unwrap().as_deref(), expected, "{name}");
        assert_eq!(report["xcodeVersion"], "27.2");
        assert_eq!(report["xcodeBuild"], "fixture");
        assert_eq!(report["sourceFormat"], "xip");
    }
}

#[test]
fn an_xcode_app_import_is_reused_until_xcode_changes() {
    let scratch = Scratch::new("xcode-app");
    let xcode = scratch.0.join("Xcode.app");
    let version = |build: &str| {
        write(
            &xcode.join("Contents/version.plist"),
            property_list::to_xml(&property_list::Plist::Dict(vec![
                (
                    "CFBundleShortVersionString".into(),
                    property_list::Plist::String("27.2".into()),
                ),
                (
                    "ProductBuildVersion".into(),
                    property_list::Plist::String(build.into()),
                ),
            ]))
            .as_bytes(),
        );
    };
    version("27B52");
    let destination = scratch.0.join("xcode-sdk");
    let keys = Keys::default();
    let mut written = 0;
    let mut writer = |inputs: &super::super::sdk_bundle::Inputs<'_>, out: &Path| {
        written += 1;
        stub_bundle(inputs, out)
    };
    let run = |writer: &mut WriteBundle<'_>| {
        import_locked(
            &options(Source::Xcode(&xcode), &destination, false, &keys),
            &destination,
            "aarch64",
            &mut |_| {},
            writer,
        )
    };
    let first = run(&mut writer).unwrap();
    let again = run(&mut writer).unwrap();
    version("27C10");
    let err = run(&mut writer).unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(written, 1);
    assert_eq!(first["reused"], false);
    assert_eq!(again["reused"], true);
    let receipt: Value =
        serde_json::from_slice(&fs::read(destination.join("sweetpad-sdk.json")).unwrap()).unwrap();
    assert_eq!(receipt["sourceFormat"], "xcode-app");
    assert_eq!(receipt["xcodeBuild"], "27B52");
    assert!(receipt.get("archiveSha256").is_none());
}

#[test]
fn an_export_keeps_link_targets_as_readlink_gives_them() {
    let scratch = Scratch::new("export-links");
    let data = scratch.0.join("data");
    write(&data, b"x");
    let deep = format!("{}/x", ["Versions"; 14].join("/"));
    let prefix = "Xcode.app/Contents/F";
    let entries = vec![
        (
            format!("{prefix}/{deep}"),
            Entry::File {
                size: 1,
                mode: 0o644,
                sha256: hex(&Sha256::digest(b"x")),
            },
        ),
        (
            format!("{prefix}/A/x"),
            Entry::File {
                size: 1,
                mode: 0o755,
                sha256: hex(&Sha256::digest(b"x")),
            },
        ),
        (
            format!("{prefix}/slashes"),
            Entry::Symlink {
                target: "A//x".into(),
            },
        ),
        (
            format!("{prefix}/long"),
            Entry::Symlink {
                target: deep.clone(),
            },
        ),
    ];
    let manifest = json!({
        "format": EXPORT_FORMAT,
        "version": 1,
        "toolchain": TOOLCHAIN.json(),
        "entries": entries.iter().map(|(n, e)| (n.clone(), e.json())).collect::<serde_json::Map<_, _>>(),
    });
    let archive = scratch.0.join("sdk.tar.gz");
    write_export(&archive, &manifest, &entries, &|_| data.clone()).unwrap();
    let out = scratch.0.join("out");
    fs::create_dir(&out).unwrap();
    unpack_export(&archive, &out, &TOOLCHAIN).unwrap();
    assert_eq!(
        fs::read_link(out.join(prefix).join("slashes")).unwrap(),
        Path::new("A//x")
    );
    assert_eq!(
        fs::read_link(out.join(prefix).join("long")).unwrap(),
        Path::new(&deep)
    );
    let mode = fs::metadata(out.join(prefix).join("A/x"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755);
}
