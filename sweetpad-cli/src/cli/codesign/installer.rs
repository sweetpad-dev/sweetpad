//! Signing a flat package as `productbuild --sign` does: an installer
//! identity's RSA signature over the SHA-1 checksum of the package's table
//! of contents, and a detached CMS signature over the same checksum, each
//! with the certificates up to Apple's root.

use std::time::SystemTime;

use super::crypto::{self, sha1};
use super::der::{self, oids};
use super::identity::{Identity, IdentitySet, Kind};
use super::time::utc_time;

/// An identity that signs installer packages.
pub(crate) struct Installer {
    identity: Identity,
}

impl Installer {
    /// The installer identity of `team` in `SWEETPAD_CODE_SIGN_P12`: a Mac
    /// Installer Distribution one, or with `developer_id`, a Developer ID
    /// Installer one, `hint` naming it among several.
    pub(crate) fn find(
        developer_id: bool,
        team: Option<&str>,
        hint: Option<&str>,
    ) -> Result<Self, String> {
        let kind = if developer_id {
            Kind::DeveloperIdInstaller
        } else {
            Kind::MacInstaller
        };
        let set = IdentitySet::from_environment(true)?.ok_or_else(|| {
            format!(
                "no {} identity to sign the package with: add its .p12 file to {}",
                kind.name(),
                super::identity::P12_VARIABLE
            )
        })?;
        Self::select(&set, kind, team, hint)
    }

    pub(super) fn select(
        set: &IdentitySet,
        kind: Kind,
        team: Option<&str>,
        hint: Option<&str>,
    ) -> Result<Self, String> {
        let identity = set.select(kind, team, hint)?.clone();
        Ok(Self { identity })
    }

    pub(crate) fn name(&self) -> &str {
        &self.identity.certificate.common_name
    }

    /// The certificates a package lists: the identity's, then those above
    /// it.
    pub(crate) fn certificates(&self) -> Vec<&[u8]> {
        std::iter::once(self.identity.certificate.der.as_slice())
            .chain(self.identity.chain.iter().map(|c| c.der.as_slice()))
            .collect()
    }

    /// How many bytes its RSA signature takes.
    pub(crate) fn signature_size(&self) -> usize {
        self.identity.certificate.modulus.len()
    }

    /// The RSA signature over `checksum`, a SHA-1 digest.
    pub(crate) fn sign(&self, checksum: &[u8; 20]) -> Result<Vec<u8>, String> {
        let key = self.key()?;
        crypto::sign_sha1(key, checksum)
    }

    /// The detached CMS signature over `content` at `time`.
    pub(crate) fn cms(&self, content: &[u8], time: SystemTime) -> Result<Vec<u8>, String> {
        let key = self.key()?;
        let attribute = |oid: &[u8], value: Vec<u8>| {
            der::sequence(&[&der::oid(oid), &der::set_of(vec![value])])
        };
        let attributes = der::set_of(vec![
            attribute(oids::CONTENT_TYPE, der::oid(oids::DATA)),
            attribute(
                oids::SIGNING_TIME,
                der::tlv(der::UTC_TIME, utc_time(time).as_bytes()),
            ),
            attribute(
                oids::MESSAGE_DIGEST,
                der::tlv(der::OCTET_STRING, &sha1(content)),
            ),
        ]);
        let signature = crypto::sign_sha1(key, &sha1(&attributes))?;
        let certificate = &self.identity.certificate;
        let header = attributes.len() - attribute_content_length(&attributes);
        let signer = der::sequence(&[
            &der::integer(1),
            &der::sequence(&[&certificate.issuer, &certificate.serial]),
            &der::algorithm(oids::SHA1),
            &der::tlv(der::context(0), &attributes[header..]),
            &der::algorithm(oids::SHA1_WITH_RSA),
            &der::tlv(der::OCTET_STRING, &signature),
        ]);
        let certificates: Vec<&[u8]> = self
            .identity
            .chain
            .iter()
            .map(|c| c.der.as_slice())
            .chain([certificate.der.as_slice()])
            .collect();
        let signed_data = der::sequence(&[
            &der::integer(1),
            &der::constructed(der::SET, &[&der::algorithm(oids::SHA1)]),
            &der::sequence(&[&der::oid(oids::DATA)]),
            &der::constructed(der::context(0), &certificates),
            &der::constructed(der::SET, &[&signer]),
        ]);
        Ok(der::sequence(&[
            &der::oid(oids::SIGNED_DATA),
            &der::constructed(der::context(0), &[&signed_data]),
        ]))
    }

    /// The test identity the codesign fixtures are signed with.
    #[cfg(test)]
    pub(crate) fn fixture() -> Self {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/codesign/identity.p12");
        let data = std::fs::read(path).unwrap();
        Self {
            identity: Identity::from_p12(&data, "sweetpad-test", true).unwrap(),
        }
    }

    fn key(&self) -> Result<&rsa::RsaPrivateKey, String> {
        self.identity
            .key
            .as_ref()
            .ok_or_else(|| "the installer identity has no private key".to_string())
    }
}

/// How many bytes of a DER element its contents take.
fn attribute_content_length(element: &[u8]) -> usize {
    let header = match element.get(1) {
        Some(n) if *n >= 0x80 => 2 + usize::from(n & 0x7f),
        _ => 2,
    };
    element.len() - header
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::Pkcs1v15Sign;

    #[test]
    fn the_signature_verifies_as_sha1_over_the_checksum() {
        let installer = Installer::fixture();
        let checksum = sha1(b"table of contents");
        let signature = installer.sign(&checksum).unwrap();
        assert_eq!(signature.len(), installer.signature_size());
        let public = rsa::RsaPublicKey::from(installer.key().unwrap());
        // SHA-1's DigestInfo, then the checksum as its digest.
        let prefix = [
            0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04,
            0x14,
        ];
        public
            .verify(
                Pkcs1v15Sign::new_unprefixed(),
                &[&prefix[..], &checksum].concat(),
                &signature,
            )
            .unwrap();
    }

    #[test]
    fn the_cms_signature_is_detached_sha1_signed_data() {
        let installer = Installer::fixture();
        let checksum = sha1(b"table of contents");
        let cms = installer.cms(&checksum, SystemTime::UNIX_EPOCH).unwrap();
        let mut outer = der::Reader::new(&cms).read().unwrap().children();
        assert_eq!(outer.read().unwrap().content, oids::SIGNED_DATA);
        let contains = |needle: &[u8]| cms.windows(needle.len()).any(|w| w == needle);
        assert!(contains(oids::SHA1_WITH_RSA));
        assert!(contains(&sha1(&checksum)));
        assert!(contains(&installer.identity.certificate.der));
        // Detached: the checksum itself isn't in it.
        assert!(!contains(&checksum));
    }
}
