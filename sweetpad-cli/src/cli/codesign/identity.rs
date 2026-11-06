//! The signing identity: a certificate and its private key, from the `.p12`
//! file `SWEETPAD_CODE_SIGN_P12` names, its password in
//! `SWEETPAD_CODE_SIGN_P12_PASSWORD`. Without a keychain, that is where
//! both the provisioning stand-in and the `codesign` one find it.

use std::path::PathBuf;

use rsa::RsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::traits::PublicKeyParts;

use super::crypto::{hex, sha1};
use super::der::{self, Reader, Tlv, oids};
use super::pkcs12;

pub(super) const P12_VARIABLE: &str = "SWEETPAD_CODE_SIGN_P12";
pub(super) const PASSWORD_VARIABLE: &str = "SWEETPAD_CODE_SIGN_P12_PASSWORD";

/// Apple's certificate authorities, which a code signature carries above
/// the signing certificate when the `.p12` file doesn't.
const APPLE_ROOT_CA: &[u8] = include_bytes!("certs/AppleRootCA.cer");
const APPLE_WWDR_CA_G3: &[u8] = include_bytes!("certs/AppleWWDRCAG3.cer");

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
        fields.read()?;
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
    /// The identity in the `.p12` file `SWEETPAD_CODE_SIGN_P12` names, or
    /// `None` when the variable isn't set.
    pub fn from_environment(with_key: bool) -> Result<Option<Self>, String> {
        let Some(path) = std::env::var_os(P12_VARIABLE).filter(|p| !p.is_empty()) else {
            return Ok(None);
        };
        let path = PathBuf::from(path);
        let data = std::fs::read(&path)
            .map_err(|e| format!("couldn't read {} ({P12_VARIABLE}): {e}", path.display()))?;
        let password = std::env::var(PASSWORD_VARIABLE).unwrap_or_default();
        Self::from_p12(&data, &password, with_key)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_p12(data: &[u8], password: &str, with_key: bool) -> Result<Self, String> {
        let contents = pkcs12::parse(data, password)?;
        let key = RsaPrivateKey::from_pkcs8_der(&contents.key)
            .map_err(|e| format!("couldn't read the private key, which must be RSA: {e}"))?;
        let modulus = key.n().to_bytes_be();
        let mut certificates = contents
            .certificates
            .iter()
            .map(|der| Certificate::parse(der))
            .collect::<Result<Vec<_>, _>>()?;
        let index = certificates
            .iter()
            .position(|c| c.modulus == modulus)
            .ok_or("no certificate in the file matches its private key")?;
        let certificate = certificates.remove(index);
        let mut pool = certificates;
        for der in [APPLE_WWDR_CA_G3, APPLE_ROOT_CA] {
            pool.push(Certificate::parse(der)?);
        }
        let mut chain: Vec<Certificate> = Vec::new();
        let (mut issuer, mut subject) = (&certificate.issuer, &certificate.subject);
        while issuer != subject && chain.len() < 8 {
            let Some(next) = pool.iter().find(|c| &c.subject == issuer) else {
                break;
            };
            (issuer, subject) = (&next.issuer, &next.subject);
            chain.push(next.clone());
        }
        Ok(Self {
            certificate,
            chain,
            key: with_key.then_some(key),
        })
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
        let aliases: &[&str] = match kind {
            "Apple Development" | "iPhone Developer" | "iOS Developer" | "Mac Developer" => &[
                "Apple Development",
                "iPhone Developer",
                "iOS Developer",
                "Mac Developer",
            ],
            "Apple Distribution" | "iPhone Distribution" | "iOS Distribution" => &[
                "Apple Distribution",
                "iPhone Distribution",
                "iOS Distribution",
            ],
            _ => &[],
        };
        name == kind || aliases.contains(&name)
    }

    /// Whether this is a development identity, which signs builds a
    /// debugger can attach to.
    pub fn is_development(&self) -> bool {
        let common_name = &self.certificate.common_name;
        [
            "Apple Development",
            "iPhone Developer",
            "iOS Developer",
            "Mac Developer",
        ]
        .iter()
        .any(|kind| common_name.starts_with(kind))
    }
}
