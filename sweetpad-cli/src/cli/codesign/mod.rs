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
//! Only iOS-style bundles, without `Contents`, are signed.

// What this prints is the tool's output, which Swift Build parses, so it
// doesn't go through `Output`.
#![allow(clippy::print_stderr)]

mod blobs;
mod cms;
mod crypto;
mod der;
mod identity;
mod macho;
mod pkcs12;
mod plist;
pub mod provisioning;
mod resources;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use blobs::Directory;
use identity::Identity;
use plist::Plist;

/// Whether the environment names an identity to sign with.
#[must_use]
pub fn identity_configured() -> bool {
    std::env::var_os(identity::P12_VARIABLE).is_some_and(|path| !path.is_empty())
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
    identity: String,
    identifier: Option<String>,
    entitlements: Option<PathBuf>,
    der_entitlements: bool,
    preserve: Vec<String>,
    flags: u32,
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
                if inline.as_deref() != Some("none") {
                    return Err(
                        "secure timestamps aren't supported; sign with '--timestamp=none'"
                            .to_string(),
                    );
                }
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
    if options.identity.is_empty() {
        return Err("no identity given with '--sign'".to_string());
    }
    Ok(options)
}

fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let path = options.path.clone().ok_or("no path to sign")?;
    let identity = if options.identity == "-" {
        None
    } else {
        let identity = Identity::from_environment(true)?.ok_or_else(|| {
            format!(
                "no signing identity: point {} at a .p12 file holding '{}' and its key, its \
                 password in {}",
                identity::P12_VARIABLE,
                options.identity,
                identity::PASSWORD_VARIABLE
            )
        })?;
        if !identity.matches(&options.identity) {
            return Err(format!(
                "the identity in {} is '{}' ({}), not '{}'",
                identity::P12_VARIABLE,
                identity.certificate.common_name,
                identity.certificate.sha1_hex(),
                options.identity
            ));
        }
        Some(identity)
    };
    let signer = Signer {
        options: &options,
        identity: identity.as_ref(),
        time: SystemTime::now(),
    };
    if path.is_dir() {
        signer.sign_bundle(&path)
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
    fn sign_bundle(&self, bundle: &Path) -> Result<(), String> {
        if bundle.join("Contents").is_dir() {
            return Err(format!(
                "{} is a macOS bundle, which this codesign doesn't sign yet",
                bundle.display()
            ));
        }
        let info_path = bundle.join("Info.plist");
        let info_bytes = std::fs::read(&info_path)
            .map_err(|e| format!("couldn't read {}: {e}", info_path.display()))?;
        let info = plist::read_info(&info_bytes)?;
        let executable = plist::string(&info, "CFBundleExecutable")
            .ok_or_else(|| format!("{} names no CFBundleExecutable", info_path.display()))?
            .to_string();
        let identifier = match (
            &self.options.identifier,
            plist::string(&info, "CFBundleIdentifier"),
        ) {
            (Some(identifier), _) => identifier.clone(),
            (None, Some(identifier)) => identifier.to_string(),
            (None, None) => {
                return Err(format!(
                    "{} names no CFBundleIdentifier",
                    info_path.display()
                ));
            }
        };
        let resources = resources::code_resources(bundle, &executable)?;
        let folder = bundle.join("_CodeSignature");
        std::fs::create_dir_all(&folder)
            .and_then(|()| std::fs::write(folder.join("CodeResources"), &resources))
            .map_err(|e| format!("couldn't write {}: {e}", folder.display()))?;
        let binding = Bundle {
            identifier,
            info: crypto::sha256(&info_bytes),
            resources: crypto::sha256(&resources),
        };
        self.sign_file(&bundle.join(executable), Some(&binding))
    }

    fn sign_file(&self, path: &Path, bundle: Option<&Bundle>) -> Result<(), String> {
        let data =
            std::fs::read(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
        let mut slices = macho::split(&data).map_err(|e| format!("{}: {e}", path.display()))?;
        for slice in &mut slices {
            slice.data = self
                .sign_slice(&slice.data, path, bundle)
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
            default_identifier(path, self.identity.is_none(), layout.uuid)
        };
        let entitlements = match &self.options.entitlements {
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
        };

        // codesign reserves room for the signature before it signs: the
        // blobs, the CMS wrapper's header and 18,000 bytes for the CMS,
        // or 8 bytes ad hoc, rounded to 16 bytes.
        let unsigned = directory.build(&layout.prepare(data, 0)?);
        let mut sized = extra.clone();
        sized.push((blobs::SLOT_CODE_DIRECTORY, unsigned));
        sized.push((blobs::SLOT_CMS, blobs::blob(blobs::CMS_WRAPPER, &[])));
        let reserve = if self.identity.is_some() { 18_000 } else { 8 };
        let size = (blobs::superblob(sized).len() + reserve).next_multiple_of(16);

        let mut out = layout.prepare(data, size)?;
        let code_directory = directory.build(&out);
        let cms = match self.identity {
            Some(identity) => cms::sign(identity, &code_directory, self.time)?,
            None => Vec::new(),
        };
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
