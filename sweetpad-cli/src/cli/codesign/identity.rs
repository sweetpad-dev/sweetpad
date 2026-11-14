//! Signing identities: certificates and their private keys, from the `.p12`
//! files `SWEETPAD_CODE_SIGN_P12` lists, their passwords in
//! `SWEETPAD_CODE_SIGN_P12_PASSWORD`. Without a keychain, that set is where
//! the provisioning stand-in, the `codesign` one and export find them.
//!
//! The variable holds one path or several, separated like `PATH`, and each
//! file may hold several identities, as Keychain Access exports a selection.
//! The password variable holds one password for every file, or one per file
//! in the same order.

use std::path::{Path, PathBuf};

use rsa::RsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::traits::PublicKeyParts;

use super::crypto::{hex, sha1};
use super::der::{self, Reader, Tlv, oids};
use super::pkcs12;
use super::time;

pub(super) const P12_VARIABLE: &str = "SWEETPAD_CODE_SIGN_P12";
pub(super) const PASSWORD_VARIABLE: &str = "SWEETPAD_CODE_SIGN_P12_PASSWORD";

/// Apple's certificate authorities, which a code signature carries above
/// the signing certificate when the `.p12` file doesn't.
const APPLE_ROOT_CA: &[u8] = include_bytes!("certs/AppleRootCA.cer");
const AUTHORITIES: [&[u8]; 4] = [
    include_bytes!("certs/AppleWWDRCAG3.cer"),
    include_bytes!("certs/DeveloperIDCA.cer"),
    include_bytes!("certs/DeveloperIDG2CA.cer"),
    APPLE_ROOT_CA,
];

/// Apple's certificate policy extensions, `1.2.840.113635.100.6.1.<n>`, by
/// the last arc.
const APPLE_POLICY: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x63, 0x64, 0x06, 0x01];

/// What an Apple certificate signs, from the policy extensions Apple puts
/// in it. Apple Development and Apple Distribution certificates carry two
/// each, one for iOS and one for macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Development,
    /// Apps for the App Store, ad hoc and enterprise on iOS, tvOS, watchOS
    /// and visionOS.
    IosDistribution,
    /// Mac apps for the App Store.
    MacAppDistribution,
    /// The installer package a Mac app goes to the App Store in.
    MacInstaller,
    DeveloperIdApplication,
    DeveloperIdInstaller,
}

impl Kind {
    const POLICIES: [(u8, Self); 7] = [
        (2, Self::Development),
        (12, Self::Development),
        (4, Self::IosDistribution),
        (7, Self::MacAppDistribution),
        (8, Self::MacInstaller),
        (13, Self::DeveloperIdApplication),
        (14, Self::DeveloperIdInstaller),
    ];

    /// The names certificates of each kind go by, and that a
    /// `CODE_SIGN_IDENTITY` uses for any of them; a certificate without
    /// Apple's extensions, like a self-signed test one, is read by them.
    const NAMES: [(&'static str, &'static [Self]); 13] = [
        ("Apple Development", &[Self::Development]),
        ("iPhone Developer", &[Self::Development]),
        ("iOS Developer", &[Self::Development]),
        ("Mac Developer", &[Self::Development]),
        (
            "Apple Distribution",
            &[Self::IosDistribution, Self::MacAppDistribution],
        ),
        ("iPhone Distribution", &[Self::IosDistribution]),
        ("iOS Distribution", &[Self::IosDistribution]),
        (
            "3rd Party Mac Developer Application",
            &[Self::MacAppDistribution],
        ),
        ("Mac App Distribution", &[Self::MacAppDistribution]),
        ("3rd Party Mac Developer Installer", &[Self::MacInstaller]),
        ("Mac Installer Distribution", &[Self::MacInstaller]),
        ("Developer ID Application", &[Self::DeveloperIdApplication]),
        ("Developer ID Installer", &[Self::DeveloperIdInstaller]),
    ];

    /// What it signs, in a list of identities.
    pub fn label(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::IosDistribution => "iOS distribution",
            Self::MacAppDistribution => "Mac App Store",
            Self::MacInstaller => "Mac installer",
            Self::DeveloperIdApplication => "Developer ID app",
            Self::DeveloperIdInstaller => "Developer ID installer",
        }
    }

    /// The name to ask for one in an error.
    pub fn name(self) -> &'static str {
        match self {
            Self::Development => "Apple Development",
            Self::IosDistribution | Self::MacAppDistribution => "Apple Distribution",
            Self::MacInstaller => "Mac Installer Distribution",
            Self::DeveloperIdApplication => "Developer ID Application",
            Self::DeveloperIdInstaller => "Developer ID Installer",
        }
    }
}

/// The names that pick the same certificates, as Xcode takes the older
/// ones for the newer kinds.
const ALIASES: [&[&str]; 4] = [
    &[
        "Apple Development",
        "iPhone Developer",
        "iOS Developer",
        "Mac Developer",
    ],
    &[
        "Apple Distribution",
        "iPhone Distribution",
        "iOS Distribution",
    ],
    &[
        "3rd Party Mac Developer Application",
        "Mac App Distribution",
    ],
    &[
        "3rd Party Mac Developer Installer",
        "Mac Installer Distribution",
    ],
];

/// A certificate and the fields of it a signature uses.
#[derive(Debug, Clone)]
pub(super) struct Certificate {
    pub der: Vec<u8>,
    /// The serial number, as its INTEGER element.
    pub serial: Vec<u8>,
    /// The issuer and subject names, as their elements.
    pub issuer: Vec<u8>,
    pub subject: Vec<u8>,
    pub common_name: String,
    pub organizational_unit: Option<String>,
    /// When it's valid from and to, as ISO 8601 text, which sorts.
    pub not_before: String,
    pub not_after: String,
    /// The RSA public key's modulus, without leading zeros.
    pub modulus: Vec<u8>,
    /// The object identifiers of its extensions.
    pub extensions: Vec<Vec<u8>>,
}

impl Certificate {
    pub fn parse(der: &[u8]) -> Result<Self, String> {
        let bad = |e: String| format!("couldn't read a certificate: {e}");
        Self::read(der).map_err(bad)
    }

    fn read(der: &[u8]) -> Result<Self, String> {
        let certificate = Reader::new(der).read()?.expect(der::SEQUENCE)?;
        let tbs = certificate.children().read()?.expect(der::SEQUENCE)?;
        let mut fields = tbs.children();
        fields.optional(der::context(0))?;
        let serial = fields.read()?.expect(der::INTEGER)?.as_der();
        fields.read()?;
        let issuer = fields.read()?.expect(der::SEQUENCE)?;
        let mut validity = fields.read()?.expect(der::SEQUENCE)?.children();
        let mut time = || -> Result<String, String> {
            let t = validity.read()?;
            time::from_asn1(t.tag, t.content).ok_or_else(|| "a malformed validity time".to_string())
        };
        let (not_before, not_after) = (time()?, time()?);
        let subject = fields.read()?.expect(der::SEQUENCE)?;
        let key_info = fields.read()?;
        let mut extensions = Vec::new();
        for field in fields {
            let field = field?;
            if field.tag == der::context(3) {
                for extension in field.children().read()?.children() {
                    extensions.push(extension?.children().read()?.content.to_vec());
                }
            }
        }
        let mut key_fields = key_info.children();
        key_fields.read()?;
        let bits = key_fields.read()?.expect(der::BIT_STRING)?;
        let modulus = bits
            .content
            .get(1..)
            .map(|key| -> Result<Vec<u8>, String> {
                let n = Reader::new(key)
                    .read()?
                    .children()
                    .read()?
                    .expect(der::INTEGER)?;
                Ok(n.content.iter().copied().skip_while(|b| *b == 0).collect())
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            der: der.to_vec(),
            serial,
            issuer: issuer.as_der(),
            subject: subject.as_der(),
            common_name: attribute(subject, oids::COMMON_NAME)?.unwrap_or_default(),
            organizational_unit: attribute(subject, oids::ORGANIZATIONAL_UNIT)?,
            not_before,
            not_after,
            modulus,
            extensions,
        })
    }

    /// The SHA-1 hash of the certificate, in uppercase hex: how `codesign`
    /// and Xcode name an identity.
    pub fn sha1_hex(&self) -> String {
        hex(&sha1(&self.der)).to_ascii_uppercase()
    }

    pub fn has_extension(&self, oid: &[u8]) -> bool {
        self.extensions.iter().any(|e| e == oid)
    }

    /// What it signs: by Apple's policy extensions, else by its name.
    pub fn kinds(&self) -> Vec<Kind> {
        let mut kinds: Vec<Kind> = self
            .extensions
            .iter()
            .filter_map(|oid| match oid.strip_prefix(APPLE_POLICY)? {
                [arc] => Kind::POLICIES
                    .iter()
                    .find(|(n, _)| n == arc)
                    .map(|(_, kind)| *kind),
                _ => None,
            })
            .collect();
        if kinds.is_empty() {
            let prefix = self.common_name.split(':').next().unwrap_or_default();
            if let Some((_, named)) = Kind::NAMES.iter().find(|(name, _)| *name == prefix) {
                kinds.extend_from_slice(named);
            }
        }
        kinds.sort();
        kinds.dedup();
        kinds
    }

    pub fn is(&self, kind: Kind) -> bool {
        self.kinds().contains(&kind)
    }

    /// Whether it's valid at `now`, an ISO 8601 time.
    pub fn is_valid_at(&self, now: &str) -> bool {
        self.not_before.as_str() <= now && now < self.not_after.as_str()
    }
}

/// The first value of attribute `oid` in a name.
fn attribute(name: Tlv<'_>, oid: &[u8]) -> Result<Option<String>, String> {
    for rdn in name.children() {
        for pair in rdn?.children() {
            let mut fields = pair?.children();
            if fields.read()?.content == oid {
                let value = fields.read()?;
                let text = if value.tag == der::BMP_STRING {
                    let units: Vec<u16> = value
                        .content
                        .chunks_exact(2)
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect();
                    String::from_utf16_lossy(&units)
                } else {
                    String::from_utf8_lossy(value.content).into_owned()
                };
                return Ok(Some(text));
            }
        }
    }
    Ok(None)
}

/// A certificate, the chain above it and, to sign with, its private key.
pub(super) struct Identity {
    pub certificate: Certificate,
    /// The certificates above it, up to the root, which a signature
    /// carries.
    pub chain: Vec<Certificate>,
    pub key: Option<RsaPrivateKey>,
}

impl Identity {
    /// The first identity in a `.p12` file.
    #[cfg(test)]
    pub fn from_p12(data: &[u8], password: &str, with_key: bool) -> Result<Self, String> {
        Self::all_from_p12(data, password, with_key)?
            .into_iter()
            .next()
            .ok_or_else(|| "no certificate in the file matches its private key".to_string())
    }

    /// Every identity in a `.p12` file: each private key with the
    /// certificate that matches it. A key without one is skipped, as a
    /// keychain shows no identity for it.
    pub fn all_from_p12(data: &[u8], password: &str, with_key: bool) -> Result<Vec<Self>, String> {
        let contents = pkcs12::parse(data, password)?;
        let certificates = contents
            .certificates
            .iter()
            .map(|der| Certificate::parse(der))
            .collect::<Result<Vec<_>, _>>()?;
        let mut pool = certificates.clone();
        for der in AUTHORITIES {
            pool.push(Certificate::parse(der)?);
        }
        let mut identities = Vec::new();
        let mut unreadable = None;
        for key in &contents.keys {
            let key = match RsaPrivateKey::from_pkcs8_der(key) {
                Ok(key) => key,
                Err(e) => {
                    unreadable.get_or_insert(format!(
                        "couldn't read the private key, which must be RSA: {e}"
                    ));
                    continue;
                }
            };
            let modulus = key.n().to_bytes_be();
            let Some(certificate) = certificates.iter().find(|c| c.modulus == modulus) else {
                continue;
            };
            identities.push(Self {
                chain: chain(certificate, &pool),
                certificate: certificate.clone(),
                key: with_key.then_some(key),
            });
        }
        if identities.is_empty() {
            return Err(unreadable
                .unwrap_or_else(|| "no certificate in the file matches its private key".into()));
        }
        Ok(identities)
    }

    /// Whether the chain ends at Apple's root certificate authority.
    pub fn is_apple_issued(&self) -> bool {
        self.chain
            .last()
            .is_some_and(|root| root.der == APPLE_ROOT_CA)
    }

    /// The team the certificate belongs to: its organizational unit, for an
    /// Apple-issued certificate.
    pub fn team(&self) -> Option<&str> {
        self.is_apple_issued()
            .then_some(self.certificate.organizational_unit.as_deref())
            .flatten()
    }

    /// The root certificate: the top of the chain, or the certificate itself
    /// when it's self-signed.
    pub fn root(&self) -> &Certificate {
        self.chain.last().unwrap_or(&self.certificate)
    }

    /// Whether `name`, a `CODE_SIGN_IDENTITY`, picks this identity: its
    /// SHA-1, its full name, or the start of it, like `Apple Development`,
    /// with the older names Xcode still takes for the same kinds.
    pub fn matches(&self, name: &str) -> bool {
        let common_name = &self.certificate.common_name;
        if name.eq_ignore_ascii_case(&self.certificate.sha1_hex()) || name == common_name {
            return true;
        }
        let kind = common_name.split(':').next().unwrap_or_default();
        name == kind
            || ALIASES
                .iter()
                .any(|names| names.contains(&kind) && names.contains(&name))
    }

    /// Whether this is a development identity, which signs builds a
    /// debugger can attach to.
    pub fn is_development(&self) -> bool {
        self.certificate.is(Kind::Development)
    }

    /// How the identity shows in a list: its name and SHA-1.
    pub fn describe(&self) -> String {
        format!(
            "'{}' ({})",
            self.certificate.common_name,
            self.certificate.sha1_hex()
        )
    }
}

/// The certificates above `certificate` in `pool`, up to a self-signed one.
fn chain(certificate: &Certificate, pool: &[Certificate]) -> Vec<Certificate> {
    let mut chain: Vec<Certificate> = Vec::new();
    let (mut issuer, mut subject) = (&certificate.issuer, &certificate.subject);
    while issuer != subject && chain.len() < 8 {
        let Some(next) = pool.iter().find(|c| &c.subject == issuer) else {
            break;
        };
        (issuer, subject) = (&next.issuer, &next.subject);
        chain.push(next.clone());
    }
    chain
}

/// The identities a run can sign with, like a keychain's.
pub(super) struct IdentitySet {
    pub identities: Vec<Identity>,
}

impl IdentitySet {
    /// The identities in the files `SWEETPAD_CODE_SIGN_P12` lists, or `None`
    /// when the variable isn't set.
    pub fn from_environment(with_keys: bool) -> Result<Option<Self>, String> {
        let Some(list) = std::env::var_os(P12_VARIABLE).filter(|p| !p.is_empty()) else {
            return Ok(None);
        };
        let files: Vec<PathBuf> = std::env::split_paths(&list)
            .filter(|p| !p.as_os_str().is_empty())
            .collect();
        let passwords = std::env::var(PASSWORD_VARIABLE).unwrap_or_default();
        Self::from_files(&files, &passwords, with_keys).map(Some)
    }

    /// The identities in `files`, opened with `passwords`: one per file,
    /// separated by `:`, when there are as many as files, else the whole
    /// text for each. A file one password doesn't open is tried with the
    /// whole text, which may hold a `:` of its own.
    pub fn from_files(files: &[PathBuf], passwords: &str, with_keys: bool) -> Result<Self, String> {
        let split: Vec<&str> = passwords.split(':').collect();
        let mut identities: Vec<Identity> = Vec::new();
        for (index, path) in files.iter().enumerate() {
            let data = std::fs::read(path)
                .map_err(|e| format!("couldn't read {} ({P12_VARIABLE}): {e}", path.display()))?;
            let own = (files.len() > 1 && split.len() == files.len()).then(|| split[index]);
            let mut opened = Identity::all_from_p12(&data, own.unwrap_or(passwords), with_keys);
            if opened.is_err()
                && own.is_some()
                && let Ok(found) = Identity::all_from_p12(&data, passwords, with_keys)
            {
                opened = Ok(found);
            }
            for identity in opened.map_err(|e| file_error(path, &e))? {
                let sha1 = identity.certificate.sha1_hex();
                if !identities.iter().any(|i| i.certificate.sha1_hex() == sha1) {
                    identities.push(identity);
                }
            }
        }
        Ok(Self { identities })
    }

    pub fn by_sha1(&self, sha1: &str) -> Option<&Identity> {
        self.identities
            .iter()
            .find(|i| i.certificate.sha1_hex().eq_ignore_ascii_case(sha1))
    }

    /// The valid identities `name`, a `CODE_SIGN_IDENTITY`, picks, of
    /// `team` when it's given: the newest first.
    pub fn named(&self, name: &str, team: Option<&str>) -> Vec<&Identity> {
        self.valid(|i| i.matches(name) && of_team(i, team))
    }

    /// The one identity `name` picks, as `codesign` looks one up: by SHA-1,
    /// else by name, which must not pick two.
    pub fn by_name(&self, name: &str) -> Result<&Identity, String> {
        if let Some(identity) = self.by_sha1(name) {
            return Ok(identity);
        }
        match self.named(name, None).as_slice() {
            [] => Err(self.missing(&format!("matches '{name}'"))),
            [one] => Ok(one),
            [one, two, ..] => Err(format!(
                "'{name}' is ambiguous: it matches {} and {} in {P12_VARIABLE}; sign with \
                 the SHA-1 of the one you mean",
                one.describe(),
                two.describe()
            )),
        }
    }

    /// The identity of `kind` an export signs with, of `team` when given;
    /// `hint`, ExportOptions' `signingCertificate`, narrows it by name or
    /// SHA-1. The newest wins, as Xcode picks one.
    pub fn select(
        &self,
        kind: Kind,
        team: Option<&str>,
        hint: Option<&str>,
    ) -> Result<&Identity, String> {
        let hint = hint.filter(|h| !h.is_empty());
        self.valid(|i| {
            i.certificate.is(kind) && of_team(i, team) && hint.is_none_or(|h| i.matches(h))
        })
        .first()
        .copied()
        .ok_or_else(|| {
            let team = team.map(|t| format!(" of team {t}")).unwrap_or_default();
            let hint = hint.map(|h| format!(" matching '{h}'")).unwrap_or_default();
            self.missing(&format!("is a valid {} identity{team}{hint}", kind.name()))
        })
    }

    /// The teams of its Apple-issued identities.
    pub fn teams(&self) -> Vec<&str> {
        let mut teams: Vec<&str> = self.identities.iter().filter_map(Identity::team).collect();
        teams.sort_unstable();
        teams.dedup();
        teams
    }

    fn valid(&self, keep: impl Fn(&Identity) -> bool) -> Vec<&Identity> {
        let now = time::now_iso8601();
        let mut found: Vec<&Identity> = self
            .identities
            .iter()
            .filter(|i| i.certificate.is_valid_at(&now) && keep(i))
            .collect();
        found.sort_by(|a, b| b.certificate.not_before.cmp(&a.certificate.not_before));
        found
    }

    /// That no identity `what`, with the ones there are.
    pub fn missing(&self, what: &str) -> String {
        let now = time::now_iso8601();
        let held: Vec<String> = self
            .identities
            .iter()
            .map(|i| {
                if i.certificate.is_valid_at(&now) {
                    i.describe()
                } else {
                    format!("{} (expired {})", i.describe(), i.certificate.not_after)
                }
            })
            .collect();
        format!(
            "no identity in {P12_VARIABLE} {what}; it holds {}",
            if held.is_empty() {
                "none".to_string()
            } else {
                held.join(", ")
            }
        )
    }
}

/// Whether `identity` belongs to `team`, or can't say: a certificate Apple
/// didn't issue names no team.
fn of_team(identity: &Identity, team: Option<&str>) -> bool {
    match (team, identity.team()) {
        (Some(team), Some(own)) => team == own,
        _ => true,
    }
}

fn file_error(path: &Path, error: &str) -> String {
    format!("{}: {error}", path.display())
}
