//! Getting Apple's SDKs onto Linux: `sdk export` packs them from a Mac's
//! Xcode into a private archive, and `sdk import` turns that archive, an
//! Xcode `.xip`, `.aea` or Apple Archive, or an Xcode.app the host can
//! read, into the Darwin SDK bundle (see [`super::sdk_bundle`]).
//!
//! An import is transactional: the bundle is written beside the
//! destination and moved into place only once it's whole, so a failed
//! import, or one replacing an SDK, leaves the SDK that was there.

// Sizes are 64-bit, narrowed to buffer lengths of a few MiB, and shown in GiB.
#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::apple_archive::{self, Keys, select};
use super::property_list;

/// Bumped when the bundle an import writes changes, so an SDK imported by
/// an older sweetpad gets rebuilt. The export format is versioned apart.
pub(crate) const IMPORTER_VERSION: u64 = 3;
/// The platforms an SDK must have, as Xcode names their folders.
const BASE_PLATFORMS: [&str; 3] = ["iPhoneOS", "iPhoneSimulator", "MacOSX"];
/// The platforms an SDK has when Xcode does.
const EXTRA_PLATFORMS: [&str; 6] = [
    "AppleTVOS",
    "AppleTVSimulator",
    "WatchOS",
    "WatchSimulator",
    "XROS",
    "XRSimulator",
];
/// The phrase resources an export carries beside `Xcode.app`.
const PHRASES: &str = "appintents-phrases.json";
const EXPORT_FORMAT: &str = "sweetpad-apple-sdk";
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const MIB: u64 = 1024 * 1024;

/// The Swift and Swift Build an SDK is made for, as reports and receipts
/// name them.
pub(crate) struct Toolchain<'a> {
    pub swift: &'a str,
    pub swift_build: &'a str,
}

impl Toolchain<'_> {
    fn json(&self) -> Value {
        json!({"swift": self.swift, "swiftBuild": self.swift_build})
    }
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buf = vec![0u8; MIB as usize];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(hex(&digest.finalize()))
}

use super::apple_archive::hex;

fn free_space(path: &Path) -> Result<u64, String> {
    use std::os::unix::ffi::OsStrExt as _;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: `c` is a valid C string and `stat` a struct statvfs fills.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &raw mut stat) } != 0 {
        return Err(format!(
            "couldn't read the free space at {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    #[allow(
        clippy::useless_conversion,
        reason = "the field types differ by platform"
    )]
    Ok(u64::from(stat.f_bavail) * u64::from(stat.f_frsize))
}

fn require_space(path: &Path, required: u64) -> Result<(), String> {
    let free = free_space(path)?;
    if free < required {
        return Err(format!(
            "Need {:.1} GiB free at {}; available: {:.1} GiB",
            required as f64 / GIB,
            path.display(),
            free as f64 / GIB
        ));
    }
    Ok(())
}

/// A folder beside the destination, removed when dropped.
struct Work(PathBuf);

impl Work {
    fn new(parent: &Path, prefix: &str) -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let path = parent.join(format!("{prefix}{}-{nanos:09}", std::process::id()));
        fs::create_dir(&path).map_err(|e| format!("couldn't create {}: {e}", path.display()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A plist's string at `key`; the file may be XML or binary.
fn plist_string(path: &Path, key: &str) -> Result<Option<String>, String> {
    let data = fs::read(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    let plist = property_list::read_info(&data)
        .map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    Ok(property_list::string(&plist, key).map(str::to_string))
}

/// An Xcode.app's version and build, and the macOS build it was built on.
struct XcodeVersion {
    version: String,
    build: String,
    build_machine: Option<String>,
}

fn xcode_version(xcode: &Path) -> Result<XcodeVersion, String> {
    let contents = xcode.join("Contents");
    let plist = contents.join("version.plist");
    let need = |key: &str| -> Result<String, String> {
        plist_string(&plist, key)?.ok_or_else(|| format!("{} has no {key}", plist.display()))
    };
    let info = contents.join("Info.plist");
    Ok(XcodeVersion {
        version: need("CFBundleShortVersionString")?,
        build: need("ProductBuildVersion")?,
        build_machine: if info.is_file() {
            plist_string(&info, "BuildMachineOSBuild")?
        } else {
            None
        },
    })
}

/// The folders and files of Xcode an SDK is made from, inside `Contents`,
/// that this Xcode has. Fails when a base platform is missing.
fn export_roots(xcode: &Path) -> Result<Vec<String>, String> {
    let contents = xcode.join("Contents");
    let toolchain = format!("{}/usr", select::TOOLCHAIN);
    let mut roots = vec!["version.plist".to_string()];
    roots.extend(
        select::TOOLCHAIN_LIBS
            .iter()
            .map(|lib| format!("{toolchain}/lib/{lib}")),
    );
    roots.push(format!("{toolchain}/include/swift"));
    let mut runtimes: Vec<String> = fs::read_dir(contents.join(&toolchain).join("lib"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("swift-"))
        .map(|name| format!("{toolchain}/lib/{name}"))
        .collect();
    runtimes.sort();
    roots.extend(runtimes);
    for name in BASE_PLATFORMS.iter().chain(&EXTRA_PLATFORMS) {
        let root = format!("Developer/Platforms/{name}.platform");
        if BASE_PLATFORMS.contains(name) && !contents.join(&root).join("Developer/SDKs").is_dir() {
            return Err(format!("Missing required platform: {name}"));
        }
        roots.extend(
            select::PLATFORM_PARTS
                .iter()
                .map(|part| format!("{root}/{part}")),
        );
    }
    roots.retain(|root| contents.join(root).symlink_metadata().is_ok());
    Ok(roots)
}

/// An entry of an export's manifest.
#[derive(Clone)]
enum Entry {
    File {
        size: u64,
        mode: u32,
        sha256: String,
    },
    Symlink {
        target: String,
    },
}

impl Entry {
    fn json(&self) -> Value {
        match self {
            Self::File { size, mode, sha256 } => {
                json!({"type": "file", "size": size, "mode": mode, "sha256": sha256})
            }
            Self::Symlink { target } => json!({"type": "symlink", "target": target}),
        }
    }
}

/// Every file and link under the export roots, as `Xcode.app/...` paths,
/// in a stable order. File checksums are left empty.
fn export_entries(xcode: &Path) -> Result<Vec<(String, Entry)>, String> {
    fn visit(xcode: &Path, path: &Path, entries: &mut Vec<(String, Entry)>) -> Result<(), String> {
        if path.file_name().is_some_and(|name| name == select::SKIPPED) {
            return Ok(());
        }
        let relative = path.strip_prefix(xcode).map_err(|e| e.to_string())?;
        let name = format!("Xcode.app/{}", relative.to_string_lossy());
        let meta = path
            .symlink_metadata()
            .map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            let target = fs::read_link(path).map_err(|e| e.to_string())?;
            let resolved = apple_archive::resolve_links(path)
                .ok_or_else(|| format!("{} is a link cycle", path.display()))?;
            let inside = resolved
                .strip_prefix(xcode)
                .map_err(|_| format!("{} links outside {}", path.display(), xcode.display()))?;
            let target = if target.is_absolute() {
                relative_path(inside, relative.parent().unwrap_or(Path::new("")))
            } else {
                target.to_string_lossy().into_owned()
            };
            entries.push((name, Entry::Symlink { target }));
        } else if meta.is_dir() {
            let mut children: Vec<PathBuf> = fs::read_dir(path)
                .map_err(|e| format!("couldn't read {}: {e}", path.display()))?
                .map(|entry| entry.map(|e| e.path()))
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?;
            children.sort();
            for child in children {
                visit(xcode, &child, entries)?;
            }
        } else if meta.is_file() {
            entries.push((
                name,
                Entry::File {
                    size: meta.len(),
                    mode: meta.permissions().mode() & 0o777,
                    sha256: String::new(),
                },
            ));
        }
        Ok(())
    }

    let xcode = apple_archive::resolve_links(xcode)
        .ok_or_else(|| format!("{} is a link cycle", xcode.display()))?;
    let mut entries = Vec::new();
    for root in export_roots(&xcode)? {
        visit(&xcode, &xcode.join("Contents").join(root), &mut entries)?;
    }
    // Every link must resolve inside the export, not only inside Xcode.
    let names: std::collections::HashSet<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    for (name, entry) in &entries {
        if let Entry::Symlink { .. } = entry {
            let path = xcode.join(name.strip_prefix("Xcode.app/").unwrap());
            let resolved = apple_archive::resolve_links(&path).unwrap_or_default();
            let inside = resolved.strip_prefix(&xcode).unwrap_or(Path::new(""));
            let resolved = format!("Xcode.app/{}", inside.to_string_lossy());
            let prefix = format!("{resolved}/");
            if !names.contains(resolved.as_str()) && !names.iter().any(|n| n.starts_with(&prefix)) {
                return Err(format!("Export would contain a dangling link: {name}"));
            }
        }
    }
    Ok(entries)
}

/// `path` relative to `base`, both relative to the same folder.
fn relative_path(path: &Path, base: &Path) -> String {
    let path: Vec<_> = path.components().collect();
    let base: Vec<_> = base.components().collect();
    let common = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".into(); base.len() - common];
    parts.extend(
        path[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

/// Pack the SDK inputs of `xcode`, on a Mac, into `output`, a `.tar.gz`
/// with its manifest first and a `.sha256` file beside it.
#[allow(clippy::too_many_lines)] // one linear run: entries, phrases, checksums, the archive
pub(crate) fn export(
    xcode: &Path,
    output: &Path,
    dry_run: bool,
    toolchain: &Toolchain<'_>,
    progress: &mut dyn FnMut(&str),
) -> Result<Value, String> {
    if !cfg!(target_os = "macos") {
        return Err("Export Apple SDK files on a Mac with Xcode".into());
    }
    let xcode = std::path::absolute(xcode).map_err(|e| e.to_string())?;
    let version = xcode_version(&xcode)?;
    let mut entries = export_entries(&xcode)?;
    let size: u64 = entries
        .iter()
        .map(|(_, e)| match e {
            Entry::File { size, .. } => *size,
            Entry::Symlink { .. } => 0,
        })
        .sum();
    let output = std::path::absolute(output).map_err(|e| e.to_string())?;
    let file_name = output
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Archive filename must be UTF-8")?
        .to_string();
    if file_name.contains(['\n', '\r']) {
        return Err("Archive filename must not contain a newline".into());
    }
    // An app's Info.plist names this Mac's build as BuildMachineOSBuild.
    let build_machine = std::process::Command::new("sw_vers")
        .arg("-buildVersion")
        .output()
        .map_err(|e| format!("couldn't run sw_vers: {e}"))?;
    let build_machine = String::from_utf8_lossy(&build_machine.stdout)
        .trim()
        .to_string();
    let mut report = json!({
        "archive": output.to_string_lossy(),
        "xcodeBuild": version.build,
        "xcodeVersion": version.version,
        "toolchain": toolchain.json(),
        "buildMachineOSBuild": build_machine,
        "fileCount": entries.len(),
        "unpackedBytes": size,
        "requiredFreeBytes": size + 256 * MIB,
        "dryRun": dry_run,
    });
    if dry_run {
        return Ok(report);
    }
    let sidecar = output.with_file_name(format!("{file_name}.sha256"));
    if output.symlink_metadata().is_ok() || sidecar.symlink_metadata().is_ok() {
        return Err("Export output already exists; select a new output path".into());
    }
    let parent = output.parent().unwrap_or(Path::new("/"));
    fs::create_dir_all(parent).map_err(|e| format!("couldn't create {}: {e}", parent.display()))?;
    require_space(parent, size + 256 * MIB)?;
    progress(&format!(
        "Exporting {} files ({:.1} GiB before compression)",
        entries.len(),
        size as f64 / GIB
    ));
    let work = Work::new(parent, ".sweetpad-sdk-export-")?;
    let phrases = work.0.join(PHRASES);
    write_phrases(&xcode, &version.build, &phrases, progress)?;
    let phrases_size = fs::metadata(&phrases).map_err(|e| e.to_string())?.len();
    entries.push((
        PHRASES.to_string(),
        Entry::File {
            size: phrases_size,
            mode: 0o644,
            sha256: String::new(),
        },
    ));
    report["fileCount"] = json!(entries.len());
    report["unpackedBytes"] = json!(size + phrases_size);
    let source = |name: &str| -> PathBuf {
        if name == PHRASES {
            phrases.clone()
        } else {
            xcode.join(name.strip_prefix("Xcode.app/").unwrap())
        }
    };
    for (name, entry) in &mut entries {
        if let Entry::File { sha256, .. } = entry {
            *sha256 = sha256_file(&source(name))?;
        }
    }
    let mut manifest = report.clone();
    manifest["format"] = json!(EXPORT_FORMAT);
    manifest["version"] = json!(1);
    manifest["entries"] =
        Value::Object(entries.iter().map(|(n, e)| (n.clone(), e.json())).collect());

    let temporary = work.0.join("sdk.tar.gz");
    write_export(&temporary, &manifest, &entries, &source)?;
    let failed = |e: std::io::Error| format!("couldn't write {}: {e}", temporary.display());
    let checksum = sha256_file(&temporary)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).map_err(failed)?;
    // A hard link won't replace another export.
    fs::hard_link(&temporary, &output)
        .map_err(|e| format!("couldn't write {}: {e}", output.display()))?;
    let mut sidecar_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&sidecar)
        .map_err(|e| format!("couldn't write {}: {e}", sidecar.display()))?;
    writeln!(sidecar_file, "{checksum}  {file_name}").map_err(|e| e.to_string())?;
    report["sha256"] = json!(checksum);
    report["archiveBytes"] = json!(fs::metadata(&output).map_err(|e| e.to_string())?.len());
    Ok(report)
}

/// Write an export: gzip around tar, the manifest first, then each entry
/// in manifest order, every member with mode and nothing else of the Mac's.
fn write_export(
    temporary: &Path,
    manifest: &Value,
    entries: &[(String, Entry)],
    source: &dyn Fn(&str) -> PathBuf,
) -> Result<(), String> {
    let file = File::create(temporary).map_err(|e| e.to_string())?;
    let gzip =
        flate2::write::GzEncoder::new(std::io::BufWriter::new(file), flate2::Compression::new(1));
    let mut tar = tar::Builder::new(gzip);
    let failed = |e: std::io::Error| format!("couldn't write {}: {e}", temporary.display());
    let header = |kind: tar::EntryType, size: u64, mode: u32| {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(size);
        header.set_mode(mode);
        header.set_mtime(0);
        header
    };
    let content = format!(
        "{}\n",
        serde_json::to_string(manifest).map_err(|e| e.to_string())?
    );
    let mut head = header(tar::EntryType::Regular, content.len() as u64, 0o600);
    tar.append_data(&mut head, "manifest.json", content.as_bytes())
        .map_err(failed)?;
    for (name, entry) in entries {
        match entry {
            Entry::Symlink { target } => {
                // As `readlink` gives it: the import compares it with the
                // manifest byte for byte, and tar's own setter would fold
                // a `//` that Apple's SDKs have.
                let bytes = target.as_bytes();
                if bytes.len() >= 100 {
                    let mut long =
                        header(tar::EntryType::GNULongLink, bytes.len() as u64 + 1, 0o644);
                    long.set_path("././@LongLink").map_err(failed)?;
                    long.set_cksum();
                    tar.append(&long, [bytes, b"\0"].concat().as_slice())
                        .map_err(failed)?;
                }
                let mut head = header(tar::EntryType::Symlink, 0, 0o777);
                head.set_link_name_literal(&bytes[..bytes.len().min(99)])
                    .map_err(failed)?;
                tar.append_data(&mut head, name, std::io::empty())
                    .map_err(failed)?;
            }
            Entry::File { size, mode, .. } => {
                let mut head = header(tar::EntryType::Regular, *size, *mode);
                let data =
                    File::open(source(name)).map_err(|e| format!("couldn't read {name}: {e}"))?;
                tar.append_data(
                    &mut head,
                    name,
                    BufReader::with_capacity(MIB as usize, data),
                )
                .map_err(failed)?;
            }
        }
    }
    tar.into_inner()
        .and_then(flate2::write::GzEncoder::finish)
        .and_then(|mut out| out.flush())
        .map_err(failed)?;
    Ok(())
}

/// The languages whose phrase expansions an export carries.
const LOCALES: &str = "af am ar az be bg bn ca cs da de el en en-AU en-GB en-US es es-MX et eu fa \
    fi fil fr fr-CA gl gu he hi hr hu hy id is it ja ka kk km kmr kn ko ku-TR lo lt lv mk ml mn mr \
    ms my nb ne nl nn no pa pl pt pt-BR pt-PT ro ru sk sl sq sr sv sw ta te th tr uk ur uz vi \
    zh-Hans zh-Hant zh-HK";

/// Write how Xcode expands App Shortcut phrases in each language, which
/// the Linux build's App Intents training needs: train a probe shortcut
/// with Xcode's processor and read back what it made of the phrase, with
/// the probe's name taken out. No app's model is copied.
fn write_phrases(
    xcode: &Path,
    build: &str,
    out: &Path,
    progress: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let tool = xcode.join(format!(
        "Contents/{}/usr/bin/appintentsnltrainingprocessor",
        select::TOOLCHAIN
    ));
    let probe = "SweetpadPhraseResourceProbe";
    let bundle_id = "dev.sweetpad.PhraseResourceProbe";
    let phrase = format!("Greet in {probe}");
    let work = Work::new(&std::env::temp_dir(), "sweetpad-phrase-export-")?;
    let app = work.0.join("Probe.app");
    let metadata = app.join("Metadata.appintents");
    let write = |path: &Path, data: &[u8]| {
        fs::write(path, data).map_err(|e| format!("couldn't write {}: {e}", path.display()))
    };
    fs::create_dir_all(&metadata).map_err(|e| e.to_string())?;
    write(
        &metadata.join("extract.actionsdata"),
        include_bytes!("../../fixtures/appintents/extract.actionsdata"),
    )?;
    write(
        &metadata.join("version.json"),
        include_bytes!("../../fixtures/appintents/version.json"),
    )?;
    let locales: Vec<&str> = LOCALES.split_whitespace().collect();
    for locale in &locales {
        fs::create_dir(app.join(format!("{locale}.lproj"))).map_err(|e| e.to_string())?;
    }
    let info = app.join("Info.plist");
    write(
        &info,
        property_list::to_xml(&property_list::Plist::Dict(vec![
            (
                "CFBundleIdentifier".into(),
                property_list::Plist::String(bundle_id.into()),
            ),
            (
                "CFBundleDisplayName".into(),
                property_list::Plist::String(probe.into()),
            ),
            (
                "CFBundleDevelopmentRegion".into(),
                property_list::Plist::String("en".into()),
            ),
        ]))
        .as_bytes(),
    )?;
    progress(&format!(
        "Exporting App Shortcut phrase expansions from Xcode {build}"
    ));
    let result = std::process::Command::new(&tool)
        .arg("--product-path")
        .arg(&app)
        .arg("--infoplist-path")
        .arg(&info)
        .arg("--extracted-metadata-path")
        .arg(&metadata)
        .arg("--temp-dir-path")
        .arg(work.0.join("ssu"))
        .args(["--bundle-id", bundle_id, "--archive-ssu-assets"])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("couldn't run {}: {e}", tool.display()))?;
    if !result.status.success() {
        return Err(format!(
            "{} failed: {}",
            tool.display(),
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    let mut expansions = serde_json::Map::new();
    for locale in &locales {
        let path = app.join(format!("{locale}.lproj/nlu.appintents/nlu.lzfse"));
        let packed =
            fs::read(&path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
        let model = super::appintents::nlu::read(&packed).map_err(|e| format!("{locale}: {e}"))?;
        let [utterances] = model.intents.values().collect::<Vec<_>>()[..] else {
            return Err(format!(
                "{locale}: the probe model has more than one intent"
            ));
        };
        let mut prefixes = Vec::new();
        for utterance in utterances {
            let prefix = utterance
                .strip_suffix(&phrase)
                .ok_or_else(|| format!("{locale}: unexpected probe expansion"))?;
            prefixes.push(prefix.to_string());
        }
        prefixes.sort();
        let negatives: Vec<String> = model
            .negatives
            .iter()
            .map(|n| n.replace(probe, "${applicationName}"))
            .collect();
        expansions.insert(
            (*locale).to_string(),
            json!({"prefixes": prefixes, "negatives": negatives}),
        );
    }
    let document = json!({"version": 1, "xcodeBuild": build, "locales": expansions});
    let text = serde_json::to_string_pretty(&document).map_err(|e| e.to_string())?;
    write(out, format!("{text}\n").as_bytes())
}

/// Refuse a name in an export that isn't a plain relative path inside
/// `Xcode.app/Contents`, or the phrase resources.
fn safe_name(name: &str) -> Result<(), String> {
    let plain = !name.is_empty()
        && !name.starts_with('/')
        && name.split('/').all(|part| !matches!(part, "" | "." | ".."));
    if !plain {
        return Err(format!("Unsafe archive path: {name}"));
    }
    if name != PHRASES && !name.starts_with("Xcode.app/Contents/") {
        return Err(format!("Unexpected SDK archive path: {name}"));
    }
    Ok(())
}

/// An export's manifest, which its first member must be.
fn read_manifest<R: Read>(
    entries: &mut tar::Entries<'_, R>,
    toolchain: &Toolchain<'_>,
) -> Result<Value, String> {
    let bad = || "SDK archive must start with its manifest".to_string();
    let mut first = entries.next().ok_or_else(bad)?.map_err(|e| e.to_string())?;
    let is_manifest = first.path_bytes().as_ref() == b"manifest.json"
        && first.header().entry_type().is_file()
        && first.size() <= 64 * MIB;
    if !is_manifest {
        return Err(bad());
    }
    let mut data = Vec::new();
    first.read_to_end(&mut data).map_err(|e| e.to_string())?;
    let manifest: Value =
        serde_json::from_slice(&data).map_err(|e| format!("invalid SDK manifest: {e}"))?;
    if manifest["format"] != EXPORT_FORMAT || manifest["version"] != 1 {
        return Err("Unsupported SDK archive format".into());
    }
    if manifest["toolchain"] != toolchain.json() {
        return Err(format!(
            "Incompatible SDK toolchain requirements; expected {}",
            toolchain.json()
        ));
    }
    let entries = manifest["entries"]
        .as_object()
        .ok_or("invalid SDK manifest entries")?;
    for (name, entry) in entries {
        safe_name(name)?;
        match entry["type"].as_str() {
            Some("file") if entry["size"].as_u64().is_some() => {}
            Some("symlink") if entry["target"].is_string() => {}
            _ => return Err(format!("Unsupported manifest entry: {name}")),
        }
    }
    Ok(manifest)
}

fn manifest_size(manifest: &Value) -> u64 {
    manifest["entries"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, e)| e["size"].as_u64())
        .sum()
}

fn gzip_entries(
    archive: &Path,
) -> Result<tar::Archive<flate2::read::GzDecoder<BufReader<File>>>, String> {
    let file =
        File::open(archive).map_err(|e| format!("couldn't open {}: {e}", archive.display()))?;
    Ok(tar::Archive::new(flate2::read::GzDecoder::new(
        BufReader::with_capacity(MIB as usize, file),
    )))
}

/// What an archive is, checked against the `.sha256` beside it.
struct Inspected {
    checksum: String,
    /// `sweetpad-export`, `xip`, `aea` or `apple-archive`.
    format: &'static str,
    required_free: u64,
}

fn inspect(archive: &Path, toolchain: &Toolchain<'_>) -> Result<Inspected, String> {
    let mut magic = [0u8; 4];
    let n = File::open(archive)
        .and_then(|mut f| f.read(&mut magic))
        .map_err(|e| format!("couldn't read {}: {e}", archive.display()))?;
    let magic = &magic[..n];
    let checksum = sha256_file(archive)?;
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let sidecar = archive.with_file_name(format!("{name}.sha256"));
    let matches_sidecar = || -> Result<bool, String> {
        let text = fs::read_to_string(&sidecar)
            .map_err(|e| format!("couldn't read {}: {e}", sidecar.display()))?;
        Ok(text
            .trim()
            .split_once(char::is_whitespace)
            .is_some_and(|(sum, file)| sum == checksum && file.trim_start() == name))
    };
    if magic.starts_with(&[0x1f, 0x8b]) {
        if !matches_sidecar()? {
            return Err("SDK archive checksum does not match its .sha256 file".into());
        }
        let mut tar = gzip_entries(archive)?;
        let mut entries = tar.entries().map_err(|e| e.to_string())?;
        let size = manifest_size(&read_manifest(&mut entries, toolchain)?);
        return Ok(Inspected {
            checksum,
            format: "sweetpad-export",
            required_free: 2 * size + 512 * MIB,
        });
    }
    let size = fs::metadata(archive).map_err(|e| e.to_string())?.len();
    let format = apple_archive::kind(magic)
        .filter(|_| size >= 12)
        .ok_or("Expected a Sweetpad SDK export, Xcode .xip, .aea, or Apple Archive")?;
    if sidecar.exists() && !matches_sidecar()? {
        return Err("Xcode archive checksum does not match its .sha256 file".into());
    }
    Ok(Inspected {
        checksum,
        format,
        required_free: 512 * MIB,
    })
}

/// Check `archive` is an export, `.xip`, `.aea` or Apple Archive this
/// sweetpad can import, and matches the `.sha256` beside it, before setup
/// spends time on anything else.
pub(crate) fn check(archive: &Path, toolchain: &Toolchain<'_>) -> Result<(), String> {
    inspect(archive, toolchain).map(|_| ())
}

/// Unpack an export into `work`, checking every member against the
/// manifest; its manifest.
fn unpack_export(archive: &Path, work: &Path, toolchain: &Toolchain<'_>) -> Result<Value, String> {
    let mut tar = gzip_entries(archive)?;
    let mut members = tar.entries().map_err(|e| e.to_string())?;
    let manifest = read_manifest(&mut members, toolchain)?;
    let entries = manifest["entries"].as_object().unwrap();
    require_space(work, 2 * manifest_size(&manifest) + 512 * MIB)?;
    let mut seen = std::collections::HashSet::new();
    // Links come last, so no member can be written through one.
    let mut links = Vec::new();
    for member in members {
        let mut member = member.map_err(|e| e.to_string())?;
        let name = String::from_utf8(member.path_bytes().into_owned())
            .map_err(|_| "SDK archive path isn't UTF-8")?;
        let name = name.trim_end_matches('/').to_string();
        safe_name(&name)?;
        let Some(entry) = entries.get(&name).filter(|_| seen.insert(name.clone())) else {
            return Err(format!("Duplicate or unlisted archive entry: {name}"));
        };
        let destination = work.join(&name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("couldn't create {}: {e}", parent.display()))?;
        }
        let kind = member.header().entry_type();
        let link = member
            .link_name_bytes()
            .map(|l| String::from_utf8_lossy(&l).into_owned());
        if kind.is_symlink()
            && entry["type"] == "symlink"
            && link.as_deref() == entry["target"].as_str()
        {
            let link = link.unwrap();
            if link.starts_with('/') {
                return Err(format!("Absolute archive link: {name}"));
            }
            let parent = name.rsplit_once('/').map_or("", |(p, _)| p);
            safe_name(&normalize(&format!("{parent}/{link}")))?;
            links.push((destination, link));
        } else if kind.is_file()
            && entry["type"] == "file"
            && Some(member.size()) == entry["size"].as_u64()
        {
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
                .map_err(|e| format!("couldn't write {}: {e}", destination.display()))?;
            let mut digest = Sha256::new();
            let mut buf = vec![0u8; MIB as usize];
            loop {
                let n = member.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                digest.update(&buf[..n]);
                output.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            }
            if entry["sha256"].as_str() != Some(hex(&digest.finalize()).as_str()) {
                return Err(format!("SDK checksum mismatch: {name}"));
            }
            let mode = entry["mode"].as_u64().unwrap_or(0o644) as u32 & 0o777;
            fs::set_permissions(&destination, fs::Permissions::from_mode(mode))
                .map_err(|e| e.to_string())?;
        } else {
            return Err(format!("Unsupported or changed archive entry: {name}"));
        }
    }
    if seen.len() != entries.len() {
        return Err("SDK archive is missing manifest entries".into());
    }
    for (path, target) in &links {
        std::os::unix::fs::symlink(target, path)
            .map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    }
    let root = apple_archive::resolve_links(work).ok_or("SDK staging folder is a link cycle")?;
    for (path, _) in &links {
        let inside = apple_archive::resolve_links(path).is_some_and(|r| r.starts_with(&root));
        if !inside || !path.exists() {
            return Err(format!("Dangling SDK link: {}", path.display()));
        }
    }
    Ok(manifest)
}

/// `path` with `.` and `..` worked out lexically, keeping the `..` that
/// climb above where it starts.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// Where an import's Xcode comes from.
pub(crate) enum Source<'a> {
    /// An export, `.xip`, `.aea` or Apple Archive.
    Archive(&'a Path),
    /// An Xcode.app this host can read, as on a Mac's Linux machine.
    Xcode(&'a Path),
}

pub(crate) struct Import<'a> {
    pub source: Source<'a>,
    pub destination: &'a Path,
    /// Replace an SDK that's there, once the new one is whole.
    pub replace: bool,
    pub keys: &'a Keys,
    /// With an Xcode.app: the phrase resources an export from it made.
    pub phrases: Option<&'a Path>,
    /// With an Xcode.app: the macOS build to name instead of Xcode's own.
    pub build_machine_os_build: Option<&'a str>,
    pub toolchain: Toolchain<'a>,
    /// The host Swift's Clang resource headers.
    pub clang_headers: &'a Path,
}

/// The architecture an SDK is imported on, as the receipt names it.
fn host_architecture() -> Result<&'static str, String> {
    match std::env::consts::ARCH {
        arch @ ("aarch64" | "x86_64") if cfg!(target_os = "linux") => Ok(arch),
        _ => Err("SDK import requires ARM64 or x86_64 Linux".into()),
    }
}

/// Import an SDK into `options.destination`, one import at a time per
/// destination; the report, with the receipt.
pub(crate) fn import(
    options: &Import<'_>,
    progress: &mut dyn FnMut(&str),
) -> Result<Value, String> {
    use std::os::fd::AsRawFd as _;
    host_architecture()?;
    let destination = std::path::absolute(options.destination).map_err(|e| e.to_string())?;
    let parent = destination
        .parent()
        .ok_or("SDK destination has no parent folder")?;
    fs::create_dir_all(parent).map_err(|e| format!("couldn't create {}: {e}", parent.display()))?;
    let name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let lock_path = parent.join(format!(".{name}.import.lock"));
    let lock = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&lock_path)
        .map_err(|e| format!("couldn't open {}: {e}", lock_path.display()))?;
    // SAFETY: the descriptor is open for the life of `lock`.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(format!(
            "Another SDK import is active for {}",
            destination.display()
        ));
    }
    let arch = host_architecture()?;
    import_locked(options, &destination, arch, progress, &mut |inputs, out| {
        super::sdk_bundle::write(inputs, out).map(|_| ())
    })
}

/// The bundle writer an import runs; tests stand in for it.
type WriteBundle<'w> = dyn FnMut(&super::sdk_bundle::Inputs<'_>, &Path) -> Result<(), String> + 'w;

#[allow(clippy::too_many_lines)] // one linear run: reuse checks, unpacking, the bundle, the swap
fn import_locked(
    options: &Import<'_>,
    destination: &Path,
    arch: &str,
    progress: &mut dyn FnMut(&str),
    write_bundle: &mut WriteBundle<'_>,
) -> Result<Value, String> {
    let (source_format, checksum, inspected, xcode_source) = match options.source {
        Source::Xcode(xcode) => {
            let xcode = std::path::absolute(xcode).map_err(|e| e.to_string())?;
            let version = xcode_version(&xcode)?;
            ("xcode-app", None, None, Some((xcode, version)))
        }
        Source::Archive(archive) => {
            let archive = fs::canonicalize(archive)
                .map_err(|e| format!("couldn't read {}: {e}", archive.display()))?;
            let inspected = inspect(&archive, &options.toolchain)?;
            (
                inspected.format,
                Some(inspected.checksum.clone()),
                Some((archive, inspected)),
                None,
            )
        }
    };
    let meta = destination.symlink_metadata();
    if meta.as_ref().is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("SDK destination must not be a symbolic link".into());
    }
    let exists = meta.is_ok();
    if exists && !destination.join("swift-sdk.json").is_file() {
        return Err("Existing destination is not a Darwin SDK; select an empty destination".into());
    }
    let receipt: Value = fs::read(destination.join("sweetpad-sdk.json"))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or(Value::Null);
    let mut required = vec![
        "swift-sdk.json",
        "version.plist",
        "Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/clang/include/stddef.h",
    ];
    if source_format == "sweetpad-export" || receipt["localizedAppIntents"] == true {
        required.push(PHRASES);
    }
    let same_source = match (&xcode_source, &checksum) {
        (Some((_, version)), _) => {
            receipt["sourceFormat"] == source_format
                && receipt["xcodeBuild"] == version.build.as_str()
        }
        (None, Some(checksum)) => receipt["archiveSha256"] == checksum.as_str(),
        (None, None) => false,
    };
    let recipe_current = receipt["importerVersion"] == IMPORTER_VERSION
        && receipt["toolchain"] == options.toolchain.json();
    let complete = required.iter().all(|p| destination.join(p).is_file());
    if exists
        && same_source
        && receipt["hostArchitecture"] == arch
        && recipe_current
        && !options.replace
        && complete
    {
        let mut report = json!({"sdkPath": destination.to_string_lossy()});
        match (&checksum, &xcode_source) {
            (Some(checksum), _) => report["archiveSha256"] = json!(checksum),
            (None, Some((_, version))) => report["xcodeBuild"] = json!(version.build),
            _ => {}
        }
        report["importerVersion"] = json!(IMPORTER_VERSION);
        report["reused"] = json!(true);
        return Ok(report);
    }
    if exists && !options.replace {
        if same_source && !recipe_current {
            return Err(
                "SDK import recipe or toolchain changed; use --replace with sdk import, or \
                        --replace-sdk with setup, to rebuild the existing SDK"
                    .into(),
            );
        }
        return Err(
            "SDK destination already exists; use --replace with sdk import, or \
                    --replace-sdk with setup, to replace it after a successful import"
                .into(),
        );
    }
    let parent = destination.parent().unwrap();
    let work = Work::new(parent, ".sweetpad-sdk-")?;
    let (xcode, phrases, build_machine, xcode_version_text, xcode_build) =
        match (xcode_source, inspected) {
            (Some((xcode, version)), _) => (
                xcode,
                options.phrases.map(Path::to_path_buf),
                options
                    .build_machine_os_build
                    .map(str::to_string)
                    .or(version.build_machine),
                version.version,
                version.build,
            ),
            (None, Some((archive, inspected))) => {
                require_space(&work.0, inspected.required_free)?;
                if inspected.format == "sweetpad-export" {
                    let manifest = unpack_export(&archive, &work.0, &options.toolchain)?;
                    let text = |key: &str| manifest[key].as_str().map(str::to_string);
                    (
                        work.0.join("Xcode.app"),
                        Some(work.0.join(PHRASES)),
                        text("buildMachineOSBuild"),
                        text("xcodeVersion").ok_or("SDK manifest has no xcodeVersion")?,
                        text("xcodeBuild").ok_or("SDK manifest has no xcodeBuild")?,
                    )
                } else {
                    progress("Reading Xcode archive and selecting private SDK inputs");
                    let xcode = apple_archive::extract(
                        &archive,
                        &work.0,
                        options.keys,
                        Some(&mut *progress),
                    )?;
                    let version = xcode_version(&xcode)?;
                    // Check the base platforms are there before writing.
                    export_roots(&xcode)?;
                    // No Mac takes part, so Xcode's own build machine stands in.
                    (
                        xcode,
                        None,
                        version.build_machine,
                        version.version,
                        version.build,
                    )
                }
            }
            (None, None) => unreachable!("an import has an archive or an Xcode"),
        };
    let bundle = work.0.join("darwin.artifactbundle");
    write_bundle(
        &super::sdk_bundle::Inputs {
            xcode: &xcode,
            clang_headers: options.clang_headers,
            phrases: phrases.as_deref(),
            build_machine_os_build: build_machine.as_deref(),
        },
        &bundle,
    )?;
    let mut receipt = serde_json::Map::new();
    if let Some(checksum) = &checksum {
        receipt.insert("archiveSha256".into(), json!(checksum));
    }
    for (key, value) in [
        ("importerVersion", json!(IMPORTER_VERSION)),
        ("xcodeBuild", json!(xcode_build)),
        ("xcodeVersion", json!(xcode_version_text)),
        ("toolchain", options.toolchain.json()),
        ("hostArchitecture", json!(arch)),
        ("sourceFormat", json!(source_format)),
        ("localizedAppIntents", json!(bundle.join(PHRASES).is_file())),
    ] {
        receipt.insert(key.into(), value);
    }
    let text = serde_json::to_string_pretty(&receipt).map_err(|e| e.to_string())?;
    fs::write(bundle.join("sweetpad-sdk.json"), format!("{text}\n")).map_err(|e| e.to_string())?;
    let backup = work.0.join("previous-sdk");
    if destination.exists() {
        fs::rename(destination, &backup)
            .map_err(|e| format!("couldn't move the old SDK aside: {e}"))?;
    }
    if let Err(e) = fs::rename(&bundle, destination) {
        if backup.exists() {
            let _ = fs::rename(&backup, destination);
        }
        return Err(format!(
            "couldn't move the SDK into {}: {e}",
            destination.display()
        ));
    }
    let mut report = json!({"sdkPath": destination.to_string_lossy()});
    for (key, value) in receipt {
        report[key] = value;
    }
    report["reused"] = json!(false);
    Ok(report)
}

#[cfg(test)]
mod tests;
