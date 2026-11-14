//! Secure timestamps, as `codesign --timestamp` gets them: an RFC 3161
//! token from a timestamp server over the SHA-256 of the CMS signature's
//! value, which goes in the signer's unsigned attributes. Apple's own
//! server is the default, as it is `codesign`'s.

use super::der::{self, Reader, oids};
use crate::cli::net;

pub(super) const APPLE: &str = "http://timestamp.apple.com/ts01";

/// Which timestamp a signature gets.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) enum Timestamp {
    /// `codesign`'s default, without `--timestamp`: Apple's, for a
    /// Developer ID signature only.
    #[default]
    Default,
    /// `--timestamp=none`.
    None,
    /// `--timestamp`, or `--timestamp=<url>`.
    Server(String),
}

/// The token `url` stamps `imprint`, the hash of a signature, with.
pub(super) fn token(url: &str, imprint: &[u8; 32]) -> Result<Vec<u8>, String> {
    let nonce = nonce();
    let unavailable = |why: String| format!("the timestamp service is not available: {why}");
    let request = net::Request::new(minreq::Method::Post, url)
        .header("Content-Type", "application/timestamp-query")
        .body(request(imprint, &nonce))
        .timeout(30);
    let response = net::Session::from_env()
        .send_retrying(&request, 3)
        .map_err(|e| unavailable(e.to_string()))?;
    if !response.ok() {
        return Err(unavailable(format!(
            "{url} answered {} {}",
            response.status, response.reason
        )));
    }
    check(&response.body, imprint, &nonce)
}

/// A TimeStampReq for `imprint` with `nonce`, asking for the server's
/// certificate in the token.
pub(super) fn request(imprint: &[u8; 32], nonce: &[u8]) -> Vec<u8> {
    der::sequence(&[
        &der::integer(1),
        &der::sequence(&[
            &der::algorithm(oids::SHA256),
            &der::tlv(der::OCTET_STRING, imprint),
        ]),
        &der::tlv(der::INTEGER, &positive(nonce)),
        &der::tlv(der::BOOLEAN, &[0xff]),
    ])
}

/// The token of a TimeStampResp, once it's granted and stamps `imprint`
/// with `nonce`.
pub(super) fn check(response: &[u8], imprint: &[u8; 32], nonce: &[u8]) -> Result<Vec<u8>, String> {
    let malformed = |why: String| format!("the timestamp server's answer is malformed: {why}");
    let mut fields = Reader::new(response)
        .read()
        .and_then(|r| r.expect(der::SEQUENCE))
        .map_err(malformed)?
        .children();
    let mut status = fields.read().map_err(malformed)?.children();
    let granted = status.read().and_then(|s| s.uint()).map_err(malformed)?;
    if granted > 1 {
        let text = status
            .optional(der::SEQUENCE)
            .ok()
            .flatten()
            .map(|texts| {
                texts
                    .children()
                    .filter_map(Result::ok)
                    .map(|t| String::from_utf8_lossy(t.content).into_owned())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .filter(|t| !t.is_empty());
        return Err(format!(
            "the timestamp server refused the request (status {granted}){}",
            text.map(|t| format!(": {t}")).unwrap_or_default()
        ));
    }
    let token = fields
        .read()
        .map_err(|_| malformed("it holds no token".to_string()))?;
    let info = tst_info(&token).map_err(malformed)?;
    let mut parts = Reader::new(&info).read().map_err(malformed)?.children();
    parts.read().map_err(malformed)?; // version
    parts.read().map_err(malformed)?; // policy
    let mut message = parts.read().map_err(malformed)?.children();
    message.read().map_err(malformed)?;
    let stamped = message.read().and_then(|m| m.octets()).map_err(malformed)?;
    if stamped != imprint {
        return Err("the timestamp server stamped another signature".to_string());
    }
    // The serial number, the time, and the optional accuracy and ordering
    // before the nonce.
    let echoed = (|| {
        parts.read()?.expect(der::INTEGER)?;
        parts.read()?.expect(der::GENERALIZED_TIME)?;
        parts.optional(der::SEQUENCE)?;
        parts.optional(der::BOOLEAN)?;
        Ok::<_, String>(parts.optional(der::INTEGER)?.map(|n| n.content))
    })()
    .map_err(malformed)?;
    if echoed != Some(positive(nonce).as_slice()) {
        return Err("the timestamp server's answer is for another request".to_string());
    }
    Ok(token.as_der())
}

/// The TSTInfo a token signs: the content of its SignedData.
fn tst_info(token: &der::Tlv<'_>) -> Result<Vec<u8>, String> {
    let mut info = token.children();
    if info.read()?.expect(der::OID)?.content != oids::SIGNED_DATA {
        return Err("the token isn't signed data".to_string());
    }
    let signed = info.read()?.expect(der::context(0))?.children().read()?;
    let mut fields = signed.children();
    fields.read()?; // version
    fields.read()?; // digest algorithms
    let mut content = fields.read()?.children();
    if content.read()?.expect(der::OID)?.content != oids::TST_INFO {
        return Err("the token doesn't stamp a time".to_string());
    }
    content
        .read()?
        .expect(der::context(0))?
        .children()
        .read()?
        .octets()
}

/// `bytes` as the contents of a non-negative INTEGER.
fn positive(bytes: &[u8]) -> Vec<u8> {
    let start = bytes
        .iter()
        .position(|b| *b != 0)
        .unwrap_or(bytes.len().saturating_sub(1));
    let mut out = bytes[start..].to_vec();
    if out.first().is_none_or(|b| b & 0x80 != 0) {
        out.insert(0, 0);
    }
    out
}

/// Eight bytes no other request has: random, or failing that, the time and
/// process.
fn nonce() -> [u8; 8] {
    use std::io::Read as _;
    let mut out = [0u8; 8];
    let random = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut out));
    if random.is_err() {
        let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
        out.copy_from_slice(&super::crypto::sha256(seed.as_bytes())[..8]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request `openssl ts -query` made, and Apple's server's answer to it.
    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/codesign/timestamp")
                .join(name),
        )
        .unwrap()
    }

    const IMPRINT: [u8; 32] = [
        0x7f, 0x36, 0x46, 0x1f, 0x35, 0xc2, 0x1e, 0x79, 0x7b, 0x25, 0x77, 0x0b, 0x70, 0x78, 0x23,
        0xa8, 0x49, 0x93, 0x0c, 0xce, 0x85, 0x7c, 0xfe, 0x40, 0xc0, 0xe8, 0x38, 0x7a, 0x77, 0x32,
        0x2f, 0x37,
    ];
    const NONCE: [u8; 8] = [0xa0, 0x63, 0x55, 0x29, 0xe4, 0xc9, 0x0a, 0x85];

    #[test]
    fn a_request_is_what_openssl_asks_for() {
        assert_eq!(request(&IMPRINT, &NONCE), fixture("request.tsq"));
    }

    #[test]
    fn a_granted_answer_for_this_request_gives_its_token() {
        let response = fixture("response.tsr");
        let token = check(&response, &IMPRINT, &NONCE).unwrap();
        assert_eq!(token[0], der::SEQUENCE);
        assert!(
            token
                .windows(oids::TST_INFO.len())
                .any(|w| w == oids::TST_INFO)
        );
        let mut other = NONCE;
        other[7] ^= 1;
        assert_eq!(
            check(&response, &IMPRINT, &other).unwrap_err(),
            "the timestamp server's answer is for another request"
        );
        let mut another = IMPRINT;
        another[0] ^= 1;
        assert_eq!(
            check(&response, &another, &NONCE).unwrap_err(),
            "the timestamp server stamped another signature"
        );
    }

    #[test]
    fn a_refusal_says_why() {
        let refusal = der::sequence(&[&der::sequence(&[
            &der::integer(2),
            &der::sequence(&[&der::tlv(der::UTF8_STRING, b"unsupported algorithm")]),
        ])]);
        assert_eq!(
            check(&refusal, &IMPRINT, &NONCE).unwrap_err(),
            "the timestamp server refused the request (status 2): unsupported algorithm"
        );
    }
}
