//! A stand-in for `codesign`, which signs what the Swift Build engine builds
//! for a device on Linux, and for the provisioning Xcode does before it
//! (see [`provisioning`]). Swift Build runs it as Xcode runs `codesign`:
//! `--force --sign <SHA-1> [--entitlements <xcent>] --timestamp=none
//! --generate-entitlement-der <path>`, once for each library and nested
//! bundle, then for the bundle around them.
//!
//! The signature is `codesign`'s, blob for blob: a code directory hashing
//! each page with SHA-256, and another with SHA-1 for the deployment
//! targets `codesign` gives one (see [`macho`]), the designated requirement, the
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
pub(crate) use crypto::{Hash, Sha1, hex, hmac, sha1, sha256};
mod der;
mod identity;
mod inspect;
pub(crate) mod installer;
mod linkedit;
pub(crate) use linkedit::align_string_tables;
mod macho;
mod pkcs12;
use crate::cli::property_list as plist;
pub mod provisioning;
pub(crate) mod resign;
mod resources;
pub(crate) mod staple;
#[cfg(test)]
mod tests;
pub(crate) mod time;
mod timestamp;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use blobs::{CodeHash, Directory, SlotHash};
use bundle::Layout;
use identity::{Identity, IdentitySet, Kind};
use plist::Plist;
use timestamp::Timestamp;

/// Whether the environment names an identity to sign with.
#[must_use]
pub fn identity_configured() -> bool {
    std::env::var_os(identity::P12_VARIABLE).is_some_and(|path| !path.is_empty())
}

/// The App Store Connect key's text, which uploads and notarization read
/// and no build needs.
const ASC_KEY_VARIABLE: &str = "SWEETPAD_ASC_KEY";

/// Keeps the signing secrets out of the environment a build passes its
/// tasks. Swift Build hands its environment to every script phase and
/// plugin, as Xcode does, and a script that prints its environment would
/// put them in the build log. On a Mac the keychain holds the identity, so
/// nothing there is in the environment to leak. The `.p12` password reaches
/// the codesign and provisioning stand-ins in a file only this user can
/// read, removed when this value drops. The App Store Connect key reaches
/// nothing.
pub(crate) struct HiddenSecrets {
    file: Option<PathBuf>,
}

impl HiddenSecrets {
    /// Removes the secrets from `env`, the changes to a child's environment,
    /// with the password, when `password` is the one set and `signs`, in a
    /// file the child reads instead.
    pub(crate) fn hide(
        env: &mut Vec<(String, Option<String>)>,
        password: Option<&str>,
        signs: bool,
    ) -> Result<Self, String> {
        env.retain(|(name, _)| {
            ![
                ASC_KEY_VARIABLE,
                identity::PASSWORD_VARIABLE,
                identity::PASSWORD_FILE_VARIABLE,
            ]
            .contains(&name.as_str())
        });
        env.push((ASC_KEY_VARIABLE.to_string(), None));
        env.push((identity::PASSWORD_VARIABLE.to_string(), None));
        let Some(password) = password.filter(|_| signs) else {
            return Ok(Self { file: None });
        };
        let file = password_file(password)?;
        env.push((
            identity::PASSWORD_FILE_VARIABLE.to_string(),
            Some(file.display().to_string()),
        ));
        Ok(Self { file: Some(file) })
    }

    /// [`Self::hide`] with the password the environment sets.
    pub(crate) fn hide_environment(
        env: &mut Vec<(String, Option<String>)>,
        signs: bool,
    ) -> Result<Self, String> {
        let password = std::env::var(identity::PASSWORD_VARIABLE).ok();
        Self::hide(env, password.as_deref(), signs)
    }
}

impl Drop for HiddenSecrets {
    fn drop(&mut self) {
        if let Some(file) = &self.file {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// A new file in the temporary folder that only this user can read, holding
/// `password`.
fn password_file(password: &str) -> Result<PathBuf, String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let path = std::env::temp_dir().join(format!(
        "sweetpad-p12-password-{}-{nanos}",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("couldn't create {}: {e}", path.display()))?;
    file.write_all(password.as_bytes())
        .map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    Ok(path)
}

/// An identity as `doctor` shows it: its name, what it signs and what stops
/// it signing, if anything does.
pub struct IdentityStatus {
    pub text: String,
    pub usable: bool,
}

/// The identities the environment names, for `doctor`; `None` when none is
/// configured.
#[must_use]
pub fn identity_summary() -> Option<Result<Vec<IdentityStatus>, String>> {
    let set = match IdentitySet::from_environment(false) {
        Ok(None) => return None,
        Ok(Some(set)) => set,
        Err(e) => return Some(Err(e)),
    };
    Some(Ok(describe_identities(&set, &time::now_iso8601())))
}

/// Each identity with what it signs, and what stops it signing at `now`:
/// an expired certificate, or an issuer neither its `.p12` file nor
/// sweetpad carries.
fn describe_identities(set: &IdentitySet, now: &str) -> Vec<IdentityStatus> {
    set.identities
        .iter()
        .map(|i| {
            let mut notes: Vec<String> = i
                .certificate
                .kinds()
                .iter()
                .map(|k| k.label().to_string())
                .collect();
            let expired = !i.certificate.is_valid_at(now);
            if expired {
                notes.push("expired".to_string());
            }
            let unchained = i.require_chain().is_err();
            if unchained {
                notes.push(format!("issuer {} missing", i.root().issuer_name()));
            }
            IdentityStatus {
                text: format!("{} ({})", i.certificate.common_name, notes.join(", ")),
                usable: !expired && !unchained,
            }
        })
        .collect()
}

/// The provisioning profiles the provisioning stand-in finds, for `doctor`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ProfileSummary {
    /// How many of each kind.
    pub kinds: Vec<(&'static str, usize)>,
    pub expired: usize,
    /// Current profiles that none of the configured identities' certificates
    /// is in, so nothing here can sign with them.
    pub unmatched: usize,
    pub usable: usize,
}

#[must_use]
pub fn profile_summary() -> ProfileSummary {
    let identities = IdentitySet::from_environment(false).ok().flatten();
    summarize_profiles(
        &provisioning::load_profiles(&provisioning::profile_folders()),
        identities.as_ref(),
        &time::now_iso8601(),
    )
}

/// Counts `profiles` by kind and by whether they can sign at `now` with
/// `identities`, or with anything when no identity is configured.
fn summarize_profiles(
    profiles: &[provisioning::Profile],
    identities: Option<&IdentitySet>,
    now: &str,
) -> ProfileSummary {
    let mut summary = ProfileSummary::default();
    for profile in profiles {
        let name = profile.kind().name();
        match summary.kinds.iter_mut().find(|(n, _)| *n == name) {
            Some((_, count)) => *count += 1,
            None => summary.kinds.push((name, 1)),
        }
        let signable = identities.is_none_or(|set| {
            set.identities
                .iter()
                .any(|i| profile.certificates.contains(&i.certificate.der))
        });
        if profile.is_expired(now) {
            summary.expired += 1;
        } else if signable {
            summary.usable += 1;
        } else {
            summary.unmatched += 1;
        }
    }
    summary
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
    info: SlotHash,
    resources: SlotHash,
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

    /// The CMS signature over `directories`, the first of them the one in
    /// the code directory slot.
    fn cms(&self, directories: &[(CodeHash, Vec<u8>)]) -> Result<Vec<u8>, String> {
        self.identity.map_or_else(
            || Ok(Vec::new()),
            |identity| cms::sign(identity, directories, self.time, self.timestamp()),
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
        let executable_path = layout.root.join(executable);
        let code = std::fs::read(&executable_path)
            .map_err(|e| format!("couldn't read {}: {e}", executable_path.display()))?;
        let dual = code_hashes(&code).contains(&CodeHash::Sha1);
        let resources = resources::code_resources(&layout.root, executable, layout.flavor, dual)?;
        let folder = layout.root.join("_CodeSignature");
        std::fs::create_dir_all(&folder)
            .and_then(|()| std::fs::write(folder.join("CodeResources"), &resources))
            .map_err(|e| format!("couldn't write {}: {e}", folder.display()))?;
        let binding = Bundle {
            identifier,
            info: SlotHash::of(&layout.info_bytes),
            resources: SlotHash::of(&resources),
        };
        self.sign_file(&layout.root.join(executable), Some(&binding))
    }

    /// Resource-only bundles keep detached signature files and hash their
    /// Info.plist as a single unpaged code slot, as Apple's codesign does.
    fn sign_resource_bundle(&self, layout: &Layout, identifier: &str) -> Result<(), String> {
        // Apple's codesign ignores executable entitlements for a bundle
        // without executable code, even when Swift Build supplies an xcent.
        let info = layout.info_bytes.as_slice();
        let resources =
            resources::code_resources(&layout.root, "Info.plist", layout.flavor, false)?;
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
                (blobs::SPECIAL_INFO, SlotHash::of(info)),
                (blobs::SPECIAL_REQUIREMENTS, SlotHash::of(&requirements)),
                (blobs::SPECIAL_RESOURCES, SlotHash::of(&resources)),
            ],
        }
        .build_unpaged(info);
        let signature = self.cms(&[(CodeHash::Sha256, directory.clone())])?;
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
        let hashes = code_hashes(&data);
        for slice in &mut slices {
            slice.data = self
                .sign_slice(&slice.data, path, bundle, uuid, hashes)
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

    #[allow(clippy::too_many_lines)] // one slice's signature, its blobs in slot order
    fn sign_slice(
        &self,
        data: &[u8],
        path: &Path,
        bundle: Option<&Bundle>,
        uuid: Option<[u8; 16]>,
        hashes: &[CodeHash],
    ) -> Result<Vec<u8>, String> {
        let layout = macho::Layout::parse(data)?;
        // A file outside a bundle names itself and binds its Info.plist the
        // way a bundle does when it embeds one, as `codesign` reads it.
        let embedded_info = layout
            .info_plist
            .filter(|_| bundle.is_none())
            .and_then(|(offset, size)| data.get(offset..offset + size));
        let embedded_identifier = embedded_info
            .and_then(|info| plist::parse(info).ok())
            .and_then(|info| plist::string(&info, "CFBundleIdentifier").map(str::to_string));
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
        } else if let Some(identifier) = embedded_identifier {
            identifier
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
        let mut special = vec![(blobs::SPECIAL_REQUIREMENTS, SlotHash::of(&requirements))];
        let mut extra = vec![(blobs::SLOT_REQUIREMENTS, requirements)];
        if let Some(bundle) = bundle {
            special.push((blobs::SPECIAL_INFO, bundle.info));
            special.push((blobs::SPECIAL_RESOURCES, bundle.resources));
        } else if let Some(info) = embedded_info {
            special.push((blobs::SPECIAL_INFO, SlotHash::of(info)));
        }
        if let Some(entitlements) = &entitlements {
            let entitlements = blobs::blob(blobs::ENTITLEMENTS, entitlements);
            special.push((blobs::SPECIAL_ENTITLEMENTS, SlotHash::of(&entitlements)));
            extra.push((blobs::SLOT_ENTITLEMENTS, entitlements));
            if self.options.der_entitlements {
                let der =
                    blobs::der_entitlements(parsed.as_ref().unwrap_or(&Plist::Dict(Vec::new())))?;
                special.push((blobs::SPECIAL_DER_ENTITLEMENTS, SlotHash::of(&der)));
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
        // or 8 bytes for each code directory ad hoc, rounded to 16 bytes.
        let unsigned_code = layout.prepare(data, 0)?;
        let mut sized = extra.clone();
        for (&hash, slot) in hashes.iter().zip(directory_slots()) {
            let mut unsigned = directory.build(&unsigned_code, hash);
            // codesign sizes the code directory as one without the runtime
            // version and the field after it.
            if directory.runtime.is_some() {
                unsigned.truncate(unsigned.len() - 8);
            }
            sized.push((slot, unsigned));
        }
        sized.push((blobs::SLOT_CMS, blobs::blob(blobs::CMS_WRAPPER, &[])));
        let reserve = if self.identity.is_some() {
            18_000
        } else {
            8 * hashes.len()
        };
        let size = (blobs::superblob(sized).len() + reserve).next_multiple_of(16);

        let mut out = layout.prepare(data, size)?;
        let directories: Vec<(CodeHash, Vec<u8>)> = hashes
            .iter()
            .map(|&hash| (hash, directory.build(&out, hash)))
            .collect();
        let cms = self.cms(&directories)?;
        let mut blobs = extra;
        for ((_, code_directory), slot) in directories.into_iter().zip(directory_slots()) {
            blobs.push((slot, code_directory));
        }
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

/// The superblob slots of a signature's code directories, in order: the
/// first in the code directory slot, the rest in the alternate ones.
/// The code directories a Mach-O file's signature has: a SHA-1 one beside
/// the SHA-256 one in every slice when one slice needs it.
fn code_hashes(data: &[u8]) -> &'static [CodeHash] {
    let needs_sha1 = macho::split(data).is_ok_and(|slices| {
        slices
            .iter()
            .any(|s| macho::Layout::parse(&s.data).is_ok_and(|layout| layout.needs_sha1()))
    });
    if needs_sha1 {
        &[CodeHash::Sha1, CodeHash::Sha256]
    } else {
        &[CodeHash::Sha256]
    }
}

fn directory_slots() -> impl Iterator<Item = u32> {
    std::iter::once(blobs::SLOT_CODE_DIRECTORY).chain(blobs::SLOT_ALTERNATE_CODE_DIRECTORY..)
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
