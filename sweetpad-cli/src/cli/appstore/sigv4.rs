//! AWS Signature Version 4, for the S3 upload notarization makes with the
//! temporary credentials the notary service hands out: the canonical
//! request over the query, the signed headers and the payload's SHA-256,
//! and the signing key derived from the secret for the day, region and
//! service.

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
    /// The canonical query: each `name=value` URI-encoded, sorted by name
    /// and joined with `&`; empty for none.
    pub(crate) query: &'a str,
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
        "{}\n{}\n{}\n{canonical_headers}\n{signed}\n{}",
        request.method, request.path, request.query, request.payload_sha256
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
    encode(key, true)
}

/// A query value: each byte but the unreserved ones percent-encoded.
pub(crate) fn encode_value(value: &str) -> String {
    encode(value, false)
}

fn encode(text: &str, keep_slashes: bool) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            b'/' if keep_slashes => "/".to_string(),
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
            query: "",
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

    /// AWS's example GET Bucket Lifecycle and GET Bucket requests, from
    /// the same page: a query of a bare name, then of two values.
    #[test]
    fn signs_awss_example_queries() {
        let credentials = Credentials {
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let empty = hex(&sha256(b""));
        for (query, signature) in [
            (
                "lifecycle=",
                "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543",
            ),
            (
                "max-keys=2&prefix=J",
                "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7",
            ),
        ] {
            let request = Signing {
                method: "GET",
                path: "/",
                query,
                headers: vec![
                    ("host".into(), "examplebucket.s3.amazonaws.com".into()),
                    ("x-amz-content-sha256".into(), empty.clone()),
                    ("x-amz-date".into(), "20130524T000000Z".into()),
                ],
                payload_sha256: &empty,
                region: "us-east-1",
                service: "s3",
                amz_date: "20130524T000000Z",
            };
            assert!(
                authorization(&request, &credentials).ends_with(&format!("Signature={signature}")),
                "{query}"
            );
        }
    }

    #[test]
    fn object_keys_keep_their_slashes() {
        assert_eq!(encode_path("/a b/c+d.zip"), "/a%20b/c%2Bd.zip");
        assert_eq!(encode_value("a/b+c=d"), "a%2Fb%2Bc%3Dd");
        assert_eq!(
            amz_date(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_369_353_600)),
            "20130524T000000Z"
        );
    }
}
