//! AWS Signature Version 4, for the one S3 upload notarization makes with
//! the temporary credentials the notary service hands out: the canonical
//! request over the signed headers and the payload's SHA-256, and the
//! signing key derived from the secret for the day, region and service.

use std::fmt::Write as _;

use crate::cli::codesign::{Hash, hex, hmac, sha256};

/// Temporary credentials: an access key, its secret and session token.
pub(crate) struct Credentials {
    pub(crate) access_key: String,
    pub(crate) secret_key: String,
    pub(crate) session_token: Option<String>,
}

/// A request to sign. `headers` are lowercase names with their values,
/// `host` and `x-amz-date` among them, and every one is signed.
pub(crate) struct Signing<'a> {
    pub(crate) method: &'a str,
    /// The path, already URI-encoded.
    pub(crate) path: &'a str,
    pub(crate) headers: Vec<(String, String)>,
    /// The hex SHA-256 of the body, as `x-amz-content-sha256` gives it.
    pub(crate) payload_sha256: &'a str,
    pub(crate) region: &'a str,
    pub(crate) service: &'a str,
    /// `YYYYMMDDThhmmssZ`, as `x-amz-date` gives it.
    pub(crate) amz_date: &'a str,
}

/// The `Authorization` header that signs `request` with `credentials`.
pub(crate) fn authorization(request: &Signing<'_>, credentials: &Credentials) -> String {
    let mut headers = request.headers.clone();
    headers.sort();
    let signed: Vec<&str> = headers.iter().map(|(name, _)| name.as_str()).collect();
    let signed = signed.join(";");
    let mut canonical_headers = String::new();
    for (name, value) in &headers {
        let _ = writeln!(canonical_headers, "{name}:{}", value.trim());
    }
    let canonical = format!(
        "{}\n{}\n\n{canonical_headers}\n{signed}\n{}",
        request.method, request.path, request.payload_sha256
    );
    let day = &request.amz_date[..8];
    let scope = format!("{day}/{}/{}/aws4_request", request.region, request.service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        request.amz_date,
        hex(&sha256(canonical.as_bytes()))
    );
    let key = [day, request.region, request.service, "aws4_request"]
        .iter()
        .fold(
            format!("AWS4{}", credentials.secret_key).into_bytes(),
            |key, part| hmac(Hash::Sha256, &key, part.as_bytes()),
        );
    let signature = hex(&hmac(Hash::Sha256, &key, string_to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
        credentials.access_key
    )
}

/// An S3 object key as a path: each byte but the unreserved ones and `/`
/// percent-encoded.
pub(crate) fn encode_path(key: &str) -> String {
    key.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `time` as `x-amz-date` writes it.
pub(crate) fn amz_date(time: std::time::SystemTime) -> String {
    crate::cli::codesign::time::iso8601(time).replace(['-', ':'], "")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS's example GET Object request, from "Signature Calculations for
    /// the Authorization Header: Transferring Payload in a Single Chunk".
    #[test]
    fn signs_awss_example_request() {
        let credentials = Credentials {
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let empty = hex(&sha256(b""));
        let request = Signing {
            method: "GET",
            path: "/test.txt",
            headers: vec![
                ("host".into(), "examplebucket.s3.amazonaws.com".into()),
                ("range".into(), "bytes=0-9".into()),
                ("x-amz-content-sha256".into(), empty.clone()),
                ("x-amz-date".into(), "20130524T000000Z".into()),
            ],
            payload_sha256: &empty,
            region: "us-east-1",
            service: "s3",
            amz_date: "20130524T000000Z",
        };
        assert_eq!(
            authorization(&request, &credentials),
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn object_keys_keep_their_slashes() {
        assert_eq!(encode_path("/a b/c+d.zip"), "/a%20b/c%2Bd.zip");
        assert_eq!(
            amz_date(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_369_353_600)),
            "20130524T000000Z"
        );
    }
}
