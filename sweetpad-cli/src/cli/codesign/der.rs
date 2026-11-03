//! ASN.1 in DER, read and written: certificates, PKCS #12 files, the CMS
//! signature and DER entitlements. Reading also takes BER's indefinite
//! lengths and constructed strings, which Apple's own CMS and Keychain
//! Access exports use.

pub(super) const BOOLEAN: u8 = 0x01;
pub(super) const INTEGER: u8 = 0x02;
pub(super) const BIT_STRING: u8 = 0x03;
pub(super) const OCTET_STRING: u8 = 0x04;
pub(super) const NULL: u8 = 0x05;
pub(super) const OID: u8 = 0x06;
pub(super) const UTF8_STRING: u8 = 0x0c;
pub(super) const UTC_TIME: u8 = 0x17;
pub(super) const BMP_STRING: u8 = 0x1e;
pub(super) const SEQUENCE: u8 = 0x30;
pub(super) const SET: u8 = 0x31;

/// `[n]`, constructed: an explicit tag, or an implicit one over a
/// constructed type.
pub(super) const fn context(n: u8) -> u8 {
    0xa0 | n
}

/// One element: its tag, its contents and the bytes it was read from.
#[derive(Debug, Clone, Copy)]
pub(super) struct Tlv<'a> {
    pub tag: u8,
    pub content: &'a [u8],
    pub raw: &'a [u8],
    /// Whether the length was BER's indefinite one, so `content` ends
    /// before the end-of-contents marker.
    indefinite: bool,
}

impl<'a> Tlv<'a> {
    /// The elements inside a constructed one.
    pub fn children(&self) -> Reader<'a> {
        Reader::new(self.content)
    }

    /// An OCTET STRING's bytes, joined from its pieces when BER split it.
    pub fn octets(&self) -> Result<Vec<u8>, String> {
        match self.tag {
            OCTET_STRING => Ok(self.content.to_vec()),
            0x24 => {
                let mut out = Vec::new();
                for piece in self.children() {
                    out.extend(piece?.octets()?);
                }
                Ok(out)
            }
            tag => Err(format!("expected an OCTET STRING, found tag {tag:#04x}")),
        }
    }

    /// The bytes of an OCTET STRING under an implicit tag, primitive or
    /// constructed from pieces.
    pub fn implicit_octets(&self) -> Result<Vec<u8>, String> {
        if self.tag & 0x20 == 0 {
            return Ok(self.content.to_vec());
        }
        let mut out = Vec::new();
        for piece in self.children() {
            out.extend(piece?.octets()?);
        }
        Ok(out)
    }

    /// The bytes of `raw` with a definite length, as DER has it: the same
    /// as `raw` unless BER wrote it indefinite.
    pub fn as_der(&self) -> Vec<u8> {
        if self.indefinite {
            tlv(self.tag, self.content)
        } else {
            self.raw.to_vec()
        }
    }

    pub fn expect(self, tag: u8) -> Result<Self, String> {
        if self.tag == tag {
            Ok(self)
        } else {
            Err(format!("expected tag {tag:#04x}, found {:#04x}", self.tag))
        }
    }

    /// A non-negative INTEGER that fits in a `u64`.
    pub fn uint(&self) -> Result<u64, String> {
        self.expect(INTEGER)?;
        let bytes = self.content;
        if bytes.is_empty() || bytes.len() > 9 || bytes[0] & 0x80 != 0 {
            return Err("an INTEGER out of range".to_string());
        }
        Ok(bytes.iter().fold(0, |n, b| (n << 8) | u64::from(*b)))
    }
}

/// The elements of a run of DER, one at a time.
#[derive(Debug, Clone)]
pub(super) struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// The next element, which must be there.
    pub fn read(&mut self) -> Result<Tlv<'a>, String> {
        self.next()
            .unwrap_or_else(|| Err("an ASN.1 element is missing".to_string()))
    }

    /// The next element if it has `tag`, else `None` and nothing is read.
    pub fn optional(&mut self, tag: u8) -> Result<Option<Tlv<'a>>, String> {
        if self.data.first() == Some(&tag) {
            self.read().map(Some)
        } else {
            Ok(None)
        }
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = Result<Tlv<'a>, String>;

    fn next(&mut self) -> Option<Self::Item> {
        // An end-of-contents marker closes an indefinite length; inside the
        // contents it read, it's where the reading stops.
        if self.data.is_empty() || self.data.starts_with(&[0, 0]) {
            return None;
        }
        Some(parse(self.data).map(|(element, rest)| {
            self.data = rest;
            element
        }))
    }
}

fn parse(data: &[u8]) -> Result<(Tlv<'_>, &[u8]), String> {
    let truncated = || "truncated ASN.1".to_string();
    let tag = *data.first().ok_or_else(truncated)?;
    if tag & 0x1f == 0x1f {
        return Err(format!("unsupported multi-byte ASN.1 tag {tag:#04x}"));
    }
    let first = *data.get(1).ok_or_else(truncated)?;
    let (start, length) = match first {
        0x80 => {
            if tag & 0x20 == 0 {
                return Err("an indefinite length on a primitive ASN.1 element".to_string());
            }
            let body = &data[2..];
            let length = indefinite_length(body)?;
            let end = 2 + length + 2;
            return Ok((
                Tlv {
                    tag,
                    content: &body[..length],
                    raw: &data[..end],
                    indefinite: true,
                },
                &data[end..],
            ));
        }
        n if n < 0x80 => (2, usize::from(n)),
        n => {
            let count = usize::from(n & 0x7f);
            if count > 4 {
                return Err("an ASN.1 length too large".to_string());
            }
            let bytes = data.get(2..2 + count).ok_or_else(truncated)?;
            (
                2 + count,
                bytes.iter().fold(0usize, |n, b| (n << 8) | usize::from(*b)),
            )
        }
    };
    let end = start.checked_add(length).ok_or_else(truncated)?;
    let content = data.get(start..end).ok_or_else(truncated)?;
    Ok((
        Tlv {
            tag,
            content,
            raw: &data[..end],
            indefinite: false,
        },
        &data[end..],
    ))
}

/// How many bytes of `body` come before its end-of-contents marker, past
/// the nested elements.
fn indefinite_length(body: &[u8]) -> Result<usize, String> {
    let mut rest = body;
    while !rest.starts_with(&[0, 0]) {
        if rest.is_empty() {
            return Err("an indefinite ASN.1 length without its end".to_string());
        }
        rest = parse(rest)?.1;
    }
    Ok(body.len() - rest.len())
}

/// An element with a definite length.
pub(super) fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let length = content.len();
    if length < 0x80 {
        out.push(u8::try_from(length).unwrap_or(0));
    } else {
        let bytes = length.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | u8::try_from(bytes.len() - skip).unwrap_or(0));
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(content);
    out
}

/// A constructed element of the elements in `parts`.
pub(super) fn constructed(tag: u8, parts: &[&[u8]]) -> Vec<u8> {
    tlv(tag, &parts.concat())
}

pub(super) fn sequence(parts: &[&[u8]]) -> Vec<u8> {
    constructed(SEQUENCE, parts)
}

/// A SET OF, its elements in DER's order: sorted by their encodings.
pub(super) fn set_of(mut parts: Vec<Vec<u8>>) -> Vec<u8> {
    parts.sort();
    tlv(SET, &parts.concat())
}

pub(super) fn oid(encoded: &[u8]) -> Vec<u8> {
    tlv(OID, encoded)
}

pub(super) fn integer(n: u64) -> Vec<u8> {
    let bytes = n.to_be_bytes();
    let skip = bytes.iter().take_while(|b| **b == 0).count().min(7);
    let mut content = bytes[skip..].to_vec();
    if content[0] & 0x80 != 0 {
        content.insert(0, 0);
    }
    tlv(INTEGER, &content)
}

/// An algorithm identifier with NULL parameters, as the RSA and SHA ones
/// are written.
pub(super) fn algorithm(encoded_oid: &[u8]) -> Vec<u8> {
    sequence(&[&oid(encoded_oid), &[NULL, 0]])
}

/// The object identifiers this module reads and writes, encoded.
pub(super) mod oids {
    pub const DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x01];
    pub const SIGNED_DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02];
    pub const ENCRYPTED_DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x06];
    pub const CONTENT_TYPE: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x03];
    pub const MESSAGE_DIGEST: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x04];
    pub const SIGNING_TIME: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x05];
    pub const X509_CERTIFICATE: &[u8] =
        &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x16, 0x01];
    pub const SHA256_WITH_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];
    pub const SHA1: &[u8] = &[0x2b, 0x0e, 0x03, 0x02, 0x1a];
    pub const SHA256: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
    pub const HMAC_SHA1: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x07];
    pub const HMAC_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x09];
    pub const PBES2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0d];
    pub const PBKDF2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0c];
    pub const AES128_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x02];
    pub const AES192_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x16];
    pub const AES256_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x2a];
    pub const PBE_SHA1_3DES: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x01, 0x03];
    pub const PBE_SHA1_RC2_40: &[u8] =
        &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x01, 0x06];
    pub const KEY_BAG: &[u8] = &[
        0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x0a, 0x01, 0x01,
    ];
    pub const SHROUDED_KEY_BAG: &[u8] = &[
        0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x0a, 0x01, 0x02,
    ];
    pub const CERT_BAG: &[u8] = &[
        0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x0a, 0x01, 0x03,
    ];
    pub const COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03];
    pub const ORGANIZATIONAL_UNIT: &[u8] = &[0x55, 0x04, 0x0b];
    /// Apple's CDHashes property list, an attribute of a code signature.
    pub const APPLE_CDHASHES: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x63, 0x64, 0x09, 0x01];
    /// Apple's CDHashes2: each code directory's hash type and full hash.
    pub const APPLE_CDHASHES2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x63, 0x64, 0x09, 0x02];
    /// The extension of Apple's Worldwide Developer Relations intermediate
    /// certificates.
    pub const APPLE_WWDR_INTERMEDIATE: &[u8] =
        &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x63, 0x64, 0x06, 0x02, 0x01];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_definite_and_indefinite_lengths_alike() {
        let definite = sequence(&[&integer(1), &tlv(OCTET_STRING, b"ab")]);
        let mut indefinite = vec![SEQUENCE, 0x80];
        indefinite.extend(integer(1));
        indefinite.extend([
            0x24,
            0x80,
            OCTET_STRING,
            1,
            b'a',
            OCTET_STRING,
            1,
            b'b',
            0,
            0,
        ]);
        indefinite.extend([0, 0]);
        for bytes in [&definite, &indefinite] {
            let element = Reader::new(bytes).read().unwrap();
            let mut children = element.children();
            assert_eq!(children.read().unwrap().uint().unwrap(), 1);
            assert_eq!(children.read().unwrap().octets().unwrap(), b"ab");
            assert!(children.next().is_none());
            assert_eq!(element.raw.len(), bytes.len());
        }
    }

    #[test]
    fn writes_long_lengths_and_positive_integers() {
        let long = tlv(OCTET_STRING, &[0; 300]);
        assert_eq!(&long[..4], &[OCTET_STRING, 0x82, 0x01, 0x2c]);
        assert_eq!(integer(0), [INTEGER, 1, 0]);
        assert_eq!(integer(128), [INTEGER, 2, 0, 0x80]);
        assert_eq!(integer(65_537), [INTEGER, 3, 1, 0, 1]);
    }
}
