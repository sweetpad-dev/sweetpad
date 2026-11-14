//! A stand-in for `codesign`, which signs what the Swift Build engine builds
//! for a device on Linux, and for the provisioning Xcode does before it
//! (see [`provisioning`]). Swift Build runs it as Xcode runs `codesign`:
//! `--force --sign <SHA-1> [--entitlements <xcent>] --timestamp=none
//! --generate-entitlement-der <path>`, once for each library and nested
//! bundle, then for the bundle around them.
//!
//! The signature is `codesign`'s, blob for blob: a code directory hashing
//! each 16 KiB page with SHA-256, the designated requirement, the
//! entitlements in XML and DER, and a CMS signature with Apple's CDHashes
//! attributes and the certificates up to Apple's root. It reserves the
//! space `codesign` reserves, so the headers, and with them every hash,
//! come out the same. The identity is a `.p12` file's (see [`identity`]).
//! iOS-style bundles, macOS ones and versioned frameworks are signed (see
//! [`bundle`]), with a secure timestamp when one is asked for or the
//! identity is a Developer ID one (see [`timestamp`]).

// What this prints is the tool's output, which Swift Build parses, so it
// doesn't go through `Output`.
#![allow(clippy::print_stderr)]

mod blobs;
mod bundle;
mod cms;
mod crypto;
pub(crate) use crypto::{Sha1, hex, sha1};
mod der;
mod identity;
mod inspect;
pub(crate) mod installer;
mod macho;
mod pkcs12;
use crate::cli::property_list as plist;
pub mod provisioning;
pub(crate) mod resign;
mod resources;
#[cfg(test)]
mod tests;
pub(crate) mod time;
mod timestamp;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use blobs::Directory;
use bundle::Layout;
use identity::{Identity, IdentitySet, Kind};
use plist::Plist;
use timestamp::Timestamp;

/// Whether the environment names an identity to sign with.
#[must_use]
pub fn identity_configured() -> bool {
    std::env::var_os(identity::P12_VARIABLE).is_some_and(|path| !path.is_empty())
}

/// The identities the environment names, each with what it signs, for
/// `doctor`; `None` when none is configured.
#[must_use]
pub fn identity_summary() -> Option<Result<Vec<String>, String>> {
    let set = match IdentitySet::from_environment(false) {
        Ok(None) => return None,
        Ok(Some(set)) => set,
        Err(e) => return Some(Err(e)),
    };
    let now = time::now_iso8601();
    Some(Ok(set
        .identities
        .iter()
        .map(|i| {
            let kinds: Vec<&str> = i.certificate.kinds().iter().map(|k| k.label()).collect();
            let expired = if i.certificate.is_valid_at(&now) {
                ""
            } else {
                ", expired"
            };
            format!(
                "{} ({}{expired})",
                i.certificate.common_name,
                kinds.join(", ")
            )
        })
        .collect()))
}

/// How many profiles of each kind the provisioning stand-in finds, for
/// `doctor`.
#[must_use]
pub fn profile_summary() -> Vec<(&'static str, usize)> {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for profile in provisioning::load_profiles(&provisioning::profile_folders()) {
        let name = profile.kind().name();
        match counts.iter_mut().find(|(n, _)| *n == name) {
            Some((_, count)) => *count += 1,
            None => counts.push((name, 1)),
        }
    }
    counts
}

#[must_use]
pub fn main(args: &[String]) -> ExitCode {
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: codesign: {message}");
            ExitCode::FAILURE
        }
    }
}

/// A stand-in for `derq`, which Swift Build runs to write a target's
/// entitlements in DER beside the XML ones: `derq query -f xml -i <xcent>
/// -o <der> --raw`.
#[must_use]
pub fn derq_main(args: &[String]) -> ExitCode {
    let run = || -> Result<(), String> {
        let (mut input, mut output) = (None, None);
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "query" | "--raw" => {}
                "-f" => {
                    if args.next().map(String::as_str) != Some("xml") {
                        return Err("only '-f xml' input is supported".to_string());
                    }
                }
                "-i" => input = args.next(),
                "-o" => output = args.next(),
                other => return Err(format!("unsupported option '{other}'")),
            }
        }
        let input = input.ok_or("no input given with '-i'")?;
        let output = output.ok_or("no output given with '-o'")?;
        let data = std::fs::read(input).map_err(|e| format!("couldn't read {input}: {e}"))?;
        let der = blobs::der_entitlements_payload(&plist::parse(&data)?)?;
        std::fs::write(output, der).map_err(|e| format!("couldn't write {output}: {e}"))
    };
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: derq: {message}");
            ExitCode::FAILURE
        }
    }
}

/// What a `codesign` command line asks for.
#[derive(Default)]
struct Options {
    display: bool,
    requirements: Option<String>,
    identity: String,
    identifier: Option<String>,
    entitlements: Option<PathBuf>,
    /// Entitlements to sign with, as a property list's XML, in place of a
    /// file: what an export signs with.
    entitlements_data: Option<Vec<u8>>,
    der_entitlements: bool,
    preserve: Vec<String>,
    flags: u32,
    timestamp: Timestamp,
    path: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args.iter();
    let value = |flag: &str, args: &mut std::slice::Iter<'_, String>| {
        args.next().cloned().ok_or(format!("{flag} needs a value"))
    };
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, inline)) if flag.starts_with("--") => (flag, Some(inline.to_string())),
            _ => (arg.as_str(), None),
        };
        match flag {
            "--" => {
                let path = args.next().ok_or("no path after '--'")?;
                if options.path.replace(PathBuf::from(path)).is_some() || args.next().is_some() {
                    return Err("only one path at a time is signed".to_string());
                }
                break;
            }
            "-d" | "--display" => options.display = true,
            "-r-" => options.requirements = Some("-".to_string()),
            "-r" | "--requirements" => {
                options.requirements = Some(inline.map_or_else(|| value(flag, &mut args), Ok)?);
            }
            "-f" | "--force" | "-v" | "--verbose" => {}
            "-s" | "--sign" => {
                options.identity = inline.map_or_else(|| value(flag, &mut args), Ok)?;
            }
            "-i" | "--identifier" => {
                options.identifier = Some(inline.map_or_else(|| value(flag, &mut args), Ok)?);
            }
            "--entitlements" => {
                options.entitlements = Some(PathBuf::from(
                    inline.map_or_else(|| value(flag, &mut args), Ok)?,
                ));
            }
            "--generate-entitlement-der" => options.der_entitlements = true,
            "--preserve-metadata" => {
                let list = inline.map_or_else(|| value(flag, &mut args), Ok)?;
                options.preserve = list.split(',').map(str::to_string).collect();
            }
            "-o" | "--options" => {
                let list = inline.map_or_else(|| value(flag, &mut args), Ok)?;
                for option in list.split(',') {
                    options.flags |= match option {
                        "kill" => 0x200,
                        "hard" => 0x100,
                        "restrict" => 0x800,
                        "library" => 0x2000,
                        "runtime" => 0x1_0000,
                        other => return Err(format!("unsupported option '{other}'")),
                    };
                }
            }
            "--timestamp" => {
                options.timestamp = match inline.as_deref() {
                    Some("none") => Timestamp::None,
                    Some(url) => Timestamp::Server(url.to_string()),
                    None => Timestamp::Server(timestamp::APPLE.to_string()),
                };
            }
            // There is no keychain here; the identity comes from the
            // environment.
            "--keychain" => {
                inline.map_or_else(|| value(flag, &mut args), Ok)?;
            }
            _ if !flag.starts_with('-') => {
                if options.path.replace(PathBuf::from(arg)).is_some() {
                    return Err("only one path at a time is signed".to_string());
                }
            }
            _ => return Err(format!("unsupported option '{arg}'")),
        }
    }
    if options.display {
        if !options.identity.is_empty() || options.requirements.as_deref() != Some("-") {
            return Err("only '--display --requirements -' inspection is supported".to_string());
        }
    } else if options.requirements.is_some() {
        return Err("custom signing requirements aren't supported".to_string());
    } else if options.identity.is_empty() {
        return Err("no identity given with '--sign'".to_string());
    }
    Ok(options)
}

fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let path = options.path.clone().ok_or("no path to sign")?;
    if options.display {
        return inspect::display(&path);
    }
    let set = if options.identity == "-" {
        None
    } else {
        Some(IdentitySet::from_environment(true)?.ok_or_else(|| {
            format!(
                "no signing identity: point {} at a .p12 file holding '{}' and its key, its \
                 password in {}",
                identity::P12_VARIABLE,
                options.identity,
                identity::PASSWORD_VARIABLE
            )
        })?)
    };
    let identity = set
        .as_ref()
        .map(|set| set.by_name(&options.identity))
        .transpose()?;
    let signer = Signer {
        options: &options,
        identity,
        time: SystemTime::now(),
    };
    if path.is_dir() {
        signer.sign_bundle(&path)
    } else if let Some(bundle) = bundle::of_executable(&path) {
        // Flutter signs Framework.framework/Framework directly. Apple's tool
        // still recognizes its owning bundle, identifier and resource seal.
        signer.sign_bundle(&bundle)
    } else {
        signer.sign_file(&path, None)
    }
}

/// What an executable's signature binds besides its code: the bundle's
/// identifier, `Info.plist` and `CodeResources`.
struct Bundle {
    identifier: String,
    info: [u8; 32],
    resources: [u8; 32],
}

struct Signer<'a> {
    options: &'a Options,
    identity: Option<&'a Identity>,
    time: SystemTime,
}

impl Signer<'_> {
    /// The timestamp server to stamp the signature with, if any.
    fn timestamp(&self) -> Option<&str> {
        let identity = self.identity?;
        match &self.options.timestamp {
            Timestamp::Default => identity
                .certificate
                .is(Kind::DeveloperIdApplication)
                .then_some(timestamp::APPLE),
            Timestamp::None => None,
            Timestamp::Server(url) => Some(url),
        }
    }

    fn cms(&self, code_directory: &[u8]) -> Result<Vec<u8>, String> {
        self.identity.map_or_else(
            || Ok(Vec::new()),
            |identity| cms::sign(identity, code_directory, self.time, self.timestamp()),
        )
    }

    fn sign_bundle(&self, bundle: &Path) -> Result<(), String> {
        let layout = Layout::of(bundle)?;
        let identifier = match (
            &self.options.identifier,
            plist::string(&layout.info, "CFBundleIdentifier"),
        ) {
            (Some(identifier), _) => identifier.clone(),
            (None, Some(identifier)) => identifier.to_string(),
            (None, None) => {
                return Err(format!(
                    "{} names no CFBundleIdentifier",
                    layout.info_path.display()
                ));
            }
        };
        let Some(executable) = &layout.executable else {
            if bundle.extension().is_some_and(|s| s == "bundle")
                && plist::string(&layout.info, "CFBundlePackageType") == Some("BNDL")
            {
                return self.sign_resource_bundle(&layout, &identifier);
            }
            return Err(format!(
                "{} names no CFBundleExecutable",
                layout.info_path.display()
            ));
        };
        let resources = resources::code_resources(&layout.root, executable, layout.flavor)?;
        let folder = layout.root.join("_CodeSignature");
        std::fs::create_dir_all(&folder)
            .and_then(|()| std::fs::write(folder.join("CodeResources"), &resources))
            .map_err(|e| format!("couldn't write {}: {e}", folder.display()))?;
        let binding = Bundle {
            identifier,
            info: crypto::sha256(&layout.info_bytes),
            resources: crypto::sha256(&resources),
        };
        self.sign_file(&layout.root.join(executable), Some(&binding))
    }

    /// Resource-only bundles keep detached signature files and hash their
    /// Info.plist as a single unpaged code slot, as Apple's codesign does.
    fn sign_resource_bundle(&self, layout: &Layout, identifier: &str) -> Result<(), String> {
        // Apple's codesign ignores executable entitlements for a bundle
        // without executable code, even when Swift Build supplies an xcent.
        let info = layout.info_bytes.as_slice();
        let resources = resources::code_resources(&layout.root, "Info.plist", layout.flavor)?;
        let requirements = blobs::requirements(identifier, self.identity)?;
        let directory = Directory {
            identifier,
            team: self.identity.and_then(Identity::team),
            flags: self.options.flags
                | if self.identity.is_none() {
                    blobs::FLAG_ADHOC
                } else {
                    0
                },
            exec_segment: (0, 0, 0),
            page_log2: 0,
            runtime: None,
            special: vec![
                (blobs::SPECIAL_INFO, crypto::sha256(info)),
                (blobs::SPECIAL_REQUIREMENTS, crypto::sha256(&requirements)),
                (blobs::SPECIAL_RESOURCES, crypto::sha256(&resources)),
            ],
        }
        .build_unpaged(info);
        let signature = self.cms(&directory)?;
        let folder = layout.root.join("_CodeSignature");
        std::fs::create_dir_all(&folder)
            .map_err(|e| format!("couldn't create {}: {e}", folder.display()))?;
        for (name, bytes) in [
            ("CodeResources", resources),
            ("CodeRequirements", requirements),
            ("CodeDirectory", directory),
            ("CodeSignature", signature),
        ] {
            std::fs::write(folder.join(name), bytes)
                .map_err(|e| format!("couldn't write {}: {e}", folder.join(name).display()))?;
        }
        Ok(())
    }

    fn sign_file(&self, path: &Path, bundle: Option<&Bundle>) -> Result<(), String> {
        let data =
            std::fs::read(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
        let mut slices = macho::split(&data).map_err(|e| format!("{}: {e}", path.display()))?;
        // An ad hoc identifier carries one UUID for every slice: the arm64
        // slice's, else the first's.
        let uuid = slices
            .iter()
            .find(|s| s.arch.is_some_and(|a| a.0 == macho::CPU_TYPE_ARM64))
            .or_else(|| slices.first())
            .and_then(|s| macho::Layout::parse(&s.data).ok())
            .and_then(|layout| layout.uuid);
        for slice in &mut slices {
            slice.data = self
                .sign_slice(&slice.data, path, bundle, uuid)
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let signed = macho::join(slices);
        // Write beside it and rename, so the file is never half signed.
        let temporary = path.with_extension("sweetpad-codesign");
        std::fs::write(&temporary, signed)
            .and_then(|()| {
                let permissions = std::fs::metadata(path)?.permissions();
                std::fs::set_permissions(&temporary, permissions)?;
                std::fs::rename(&temporary, path)
            })
            .map_err(|e| {
                let _ = std::fs::remove_file(&temporary);
                format!("couldn't write {}: {e}", path.display())
            })
    }

    fn sign_slice(
        &self,
        data: &[u8],
        path: &Path,
        bundle: Option<&Bundle>,
        uuid: Option<[u8; 16]>,
    ) -> Result<Vec<u8>, String> {
        let layout = macho::Layout::parse(data)?;
        let existing = layout
            .signature(data)
            .and_then(blobs::read_existing)
            .filter(|e| e.flags & blobs::FLAG_LINKER_SIGNED == 0);
        let preserve = |what: &str| self.options.preserve.iter().any(|p| p == what);
        let preserved = |what: &str| existing.as_ref().filter(|_| preserve(what));
        let identifier = if let Some(identifier) = &self.options.identifier {
            identifier.clone()
        } else if let Some(existing) = preserved("identifier") {
            existing.identifier.clone()
        } else if let Some(bundle) = bundle {
            bundle.identifier.clone()
        } else {
            default_identifier(path, self.identity.is_none(), uuid)
        };
        let entitlements = match &self.options.entitlements {
            _ if self.options.entitlements_data.is_some() => self.options.entitlements_data.clone(),
            Some(file) => Some(
                std::fs::read(file)
                    .map_err(|e| format!("couldn't read {}: {e}", file.display()))?,
            ),
            None => preserved("entitlements").and_then(|e| e.entitlements.clone()),
        };
        let parsed = entitlements.as_deref().map(plist::parse).transpose()?;
        let mut flags = self.options.flags;
        if let Some(existing) = preserved("flags") {
            flags |= existing.flags & !blobs::FLAG_ADHOC;
        }
        if self.identity.is_none() {
            flags |= blobs::FLAG_ADHOC;
        }
        let mut exec_flags = 0;
        if layout.filetype == macho::MH_EXECUTE {
            exec_flags |= blobs::EXECSEG_MAIN_BINARY;
        }
        if parsed
            .as_ref()
            .and_then(|e| plist::get(e, "get-task-allow"))
            .is_some_and(|v| *v == Plist::Bool(true))
        {
            exec_flags |= blobs::EXECSEG_ALLOW_UNSIGNED;
        }

        let requirements = blobs::requirements(&identifier, self.identity)?;
        let mut special = vec![(blobs::SPECIAL_REQUIREMENTS, crypto::sha256(&requirements))];
        let mut extra = vec![(blobs::SLOT_REQUIREMENTS, requirements)];
        if let Some(bundle) = bundle {
            special.push((blobs::SPECIAL_INFO, bundle.info));
            special.push((blobs::SPECIAL_RESOURCES, bundle.resources));
        }
        if let Some(entitlements) = &entitlements {
            let entitlements = blobs::blob(blobs::ENTITLEMENTS, entitlements);
            special.push((blobs::SPECIAL_ENTITLEMENTS, crypto::sha256(&entitlements)));
            extra.push((blobs::SLOT_ENTITLEMENTS, entitlements));
            if self.options.der_entitlements {
                let der =
                    blobs::der_entitlements(parsed.as_ref().unwrap_or(&Plist::Dict(Vec::new())))?;
                special.push((blobs::SPECIAL_DER_ENTITLEMENTS, crypto::sha256(&der)));
                extra.push((blobs::SLOT_DER_ENTITLEMENTS, der));
            }
        }
        let team = self.identity.and_then(Identity::team);
        let directory = Directory {
            identifier: &identifier,
            team,
            flags,
            exec_segment: (layout.text.0, layout.text.1, exec_flags),
            special,
            page_log2: layout.page_log2(),
            runtime: (flags & blobs::FLAG_RUNTIME != 0).then(|| layout.sdk.unwrap_or(0)),
        };

        // codesign reserves room for the signature before it signs: the
        // blobs, the CMS wrapper's header and 18,000 bytes for the CMS,
        // or 8 bytes ad hoc, rounded to 16 bytes.
        let mut unsigned = directory.build(&layout.prepare(data, 0)?);
        // codesign sizes the code directory as one without the runtime
        // version and the field after it.
        if directory.runtime.is_some() {
            unsigned.truncate(unsigned.len() - 8);
        }
        let mut sized = extra.clone();
        sized.push((blobs::SLOT_CODE_DIRECTORY, unsigned));
        sized.push((blobs::SLOT_CMS, blobs::blob(blobs::CMS_WRAPPER, &[])));
        let reserve = if self.identity.is_some() { 18_000 } else { 8 };
        let size = (blobs::superblob(sized).len() + reserve).next_multiple_of(16);

        let mut out = layout.prepare(data, size)?;
        let code_directory = directory.build(&out);
        let cms = self.cms(&code_directory)?;
        let mut blobs = extra;
        blobs.push((blobs::SLOT_CODE_DIRECTORY, code_directory));
        blobs.push((blobs::SLOT_CMS, blobs::blob(blobs::CMS_WRAPPER, &cms)));
        let mut signature = blobs::superblob(blobs);
        if signature.len() > size {
            return Err("the signature outgrew the room reserved for it".to_string());
        }
        signature.resize(size, 0);
        out.extend(signature);
        Ok(out)
    }
}

/// The identifier `codesign` gives a file outside a bundle: its name
/// without the extension, and for an ad hoc signature of a name without a
/// dot, its UUID after it, to tell it from others of that name.
fn default_identifier(path: &Path, adhoc: bool, uuid: Option<[u8; 16]>) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(stem, _)| stem)
        .to_string();
    match uuid.filter(|_| adhoc && !stem.contains('.')) {
        Some(uuid) => {
            format!("{stem}-{}", crypto::hex(&[&b"UUID"[..], &uuid].concat()))
        }
        None => stem,
    }
}
