//! Reading a `.p12` file (PKCS #12, RFC 7292): its certificates and its
//! private key. Keychain Access exports encrypt both with Triple DES under
//! PKCS #12's own key derivation; OpenSSL 3 uses PBES2 with AES. The 40-bit
//! RC2 that older OpenSSL used for certificates isn't read.

use super::crypto::{self, Cipher, Hash, Purpose};
use super::der::{self, Reader, Tlv, oids};

/// What a `.p12` file holds.
pub(super) struct Contents {
    /// Each certificate, in DER.
    pub certificates: Vec<Vec<u8>>,
    /// The private key, as a PKCS #8 PrivateKeyInfo in DER.
    pub key: Vec<u8>,
}

pub(super) fn parse(data: &[u8], password: &str) -> Result<Contents, String> {
    let pfx = Reader::new(data).read()?.expect(der::SEQUENCE)?;
    let mut fields = pfx.children();
    if fields.read()?.uint()? != 3 {
        return Err("not a PKCS #12 version 3 file".to_string());
    }
    let auth_safe = data_content(fields.read()?)?;
    if let Some(mac) = fields.optional(der::SEQUENCE)? {
        verify_mac(mac, &auth_safe, password)?;
    }
    let mut contents = Contents {
        certificates: Vec::new(),
        key: Vec::new(),
    };
    for info in Reader::new(&auth_safe)
        .read()?
        .expect(der::SEQUENCE)?
        .children()
    {
        let info = info?;
        let mut parts = info.children();
        let kind = parts.read()?.expect(der::OID)?;
        let safe_contents = if kind.content == oids::DATA {
            data_content(info)?
        } else if kind.content == oids::ENCRYPTED_DATA {
            let encrypted = parts.read()?.expect(der::context(0))?.children().read()?;
            let mut fields = encrypted.children();
            fields.read()?;
            let mut content = fields.read()?.children();
            content.read()?;
            let algorithm = content.read()?;
            let bytes = content.read()?.implicit_octets()?;
            decrypt(algorithm, &bytes, password)?
        } else {
            continue;
        };
        for bag in Reader::new(&safe_contents).read()?.children() {
            read_bag(bag?, password, &mut contents)?;
        }
    }
    if contents.key.is_empty() {
        return Err("the file holds no private key".to_string());
    }
    if contents.certificates.is_empty() {
        return Err("the file holds no certificate".to_string());
    }
    Ok(contents)
}

/// The bytes of a ContentInfo of type data.
fn data_content(info: Tlv<'_>) -> Result<Vec<u8>, String> {
    let mut parts = info.expect(der::SEQUENCE)?.children();
    if parts.read()?.expect(der::OID)?.content != oids::DATA {
        return Err("expected PKCS #7 data".to_string());
    }
    parts
        .read()?
        .expect(der::context(0))?
        .children()
        .read()?
        .octets()
}

fn read_bag(bag: Tlv<'_>, password: &str, contents: &mut Contents) -> Result<(), String> {
    let mut parts = bag.children();
    let kind = parts.read()?.expect(der::OID)?.content;
    let value = parts.read()?.expect(der::context(0))?.children().read()?;
    if kind == oids::CERT_BAG {
        let mut fields = value.children();
        if fields.read()?.content == oids::X509_CERTIFICATE {
            let certificate = fields.read()?.children().read()?.octets()?;
            contents.certificates.push(certificate);
        }
    } else if kind == oids::KEY_BAG {
        contents.key = value.as_der();
    } else if kind == oids::SHROUDED_KEY_BAG {
        let mut fields = value.children();
        let algorithm = fields.read()?;
        let encrypted = fields.read()?.octets()?;
        contents.key = decrypt(algorithm, &encrypted, password)?;
    }
    Ok(())
}

/// The password as PKCS #12's key derivation takes it: a BMPString with its
/// terminator.
fn bmp(password: &str) -> Vec<u8> {
    password
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_be_bytes)
        .collect()
}

fn verify_mac(mac: Tlv<'_>, content: &[u8], password: &str) -> Result<(), String> {
    let mut fields = mac.children();
    let digest_info = fields.read()?;
    let mut digest_fields = digest_info.children();
    let algorithm = digest_fields.read()?.children().read()?.content;
    let expected = digest_fields.read()?.octets()?;
    let salt = fields.read()?.octets()?;
    let iterations = fields.optional(der::INTEGER)?.map_or(Ok(1), |n| n.uint())?;
    let hash = if algorithm == oids::SHA1 {
        Hash::Sha1
    } else if algorithm == oids::SHA256 {
        Hash::Sha256
    } else {
        return Err("the file's integrity check uses an unsupported hash".to_string());
    };
    let key = crypto::pkcs12_kdf(
        hash,
        &bmp(password),
        &salt,
        Purpose::Mac,
        iterations,
        hash.size(),
    );
    if crypto::hmac(hash, &key, content) == expected {
        Ok(())
    } else {
        Err("wrong password for the .p12 file".to_string())
    }
}

/// `data` decrypted with the algorithm `algorithm` names.
fn decrypt(algorithm: Tlv<'_>, data: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let mut fields = algorithm.children();
    let kind = fields.read()?.expect(der::OID)?.content;
    let parameters = fields.read()?;
    let failed = |e: String| format!("couldn't decrypt the .p12 file: {e}");
    if kind == oids::PBE_SHA1_3DES {
        let mut fields = parameters.children();
        let salt = fields.read()?.octets()?;
        let iterations = fields.read()?.uint()?;
        let password = bmp(password);
        let key = crypto::pkcs12_kdf(Hash::Sha1, &password, &salt, Purpose::Key, iterations, 24);
        let iv = crypto::pkcs12_kdf(Hash::Sha1, &password, &salt, Purpose::Iv, iterations, 8);
        return crypto::decrypt_cbc(Cipher::TripleDes, &key, &iv, data).map_err(failed);
    }
    if kind == oids::PBES2 {
        let mut fields = parameters.children();
        let kdf = fields.read()?;
        let scheme = fields.read()?;
        let mut kdf_fields = kdf.children();
        if kdf_fields.read()?.content != oids::PBKDF2 {
            return Err("the .p12 file uses an unsupported key derivation".to_string());
        }
        let mut pbkdf2 = kdf_fields.read()?.children();
        let salt = pbkdf2.read()?.octets()?;
        let iterations = pbkdf2.read()?.uint()?;
        pbkdf2.optional(der::INTEGER)?;
        let hash = match pbkdf2.optional(der::SEQUENCE)? {
            None => Hash::Sha1,
            Some(prf) if prf.children().read()?.content == oids::HMAC_SHA1 => Hash::Sha1,
            Some(prf) if prf.children().read()?.content == oids::HMAC_SHA256 => Hash::Sha256,
            Some(_) => return Err("the .p12 file uses an unsupported PBKDF2 hash".to_string()),
        };
        let mut scheme_fields = scheme.children();
        let cipher = scheme_fields.read()?.content;
        let iv = scheme_fields.read()?.octets()?;
        let size = if cipher == oids::AES128_CBC {
            16
        } else if cipher == oids::AES192_CBC {
            24
        } else if cipher == oids::AES256_CBC {
            32
        } else {
            return Err("the .p12 file uses an unsupported cipher".to_string());
        };
        let key = crypto::pbkdf2(hash, password.as_bytes(), &salt, iterations, size);
        return crypto::decrypt_cbc(Cipher::Aes, &key, &iv, data).map_err(failed);
    }
    if kind == oids::PBE_SHA1_RC2_40 {
        return Err(
            "the .p12 file encrypts with 40-bit RC2, which isn't supported; export it again \
             from Keychain Access, or with 'openssl pkcs12 -export' from OpenSSL 3"
                .to_string(),
        );
    }
    Err("the .p12 file uses an unsupported encryption".to_string())
}
