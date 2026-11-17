//! Apple's FCS key envelopes, which some AEA archives carry instead of their
//! key: the key sealed with RFC 9180 HPKE in base mode (P-256, HKDF-SHA256,
//! AES-256-GCM) to a recipient whose private key Apple publishes at a URL
//! in the archive's metadata. That URL is only fetched from Apple's own
//! HTTPS distribution hosts, and no account or cookie goes with it.

use std::collections::HashMap;
use std::sync::Mutex;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hkdf::Hkdf;
use p256::pkcs8::DecodePrivateKey as _;
use sha2::Sha256;

/// The hosts a key may be fetched from.
const HOSTS: [&str; 4] = [
    "swdist.apple.com",
    "wkms-public.apple.com",
    "appldnld.apple.com",
    "updates.cdn-apple.com",
];
/// The most a key response may be.
const KEY_LIMIT: usize = 128 * 1024;

/// An archive's `key\0value` metadata entries, each with a little-endian
/// length first; empty when the data isn't laid out that way.
pub(super) fn metadata(data: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut result = HashMap::new();
    let mut pos = 0;
    while pos < data.len() {
        let Some(length) = data.get(pos..pos + 4) else {
            return Ok(HashMap::new());
        };
        let length = u32::from_le_bytes(length.try_into().unwrap()) as usize;
        if length < 5 || length > data.len() - pos {
            return Ok(HashMap::new());
        }
        let entry = &data[pos + 4..pos + length];
        let Some(split) = entry.iter().position(|b| *b == 0) else {
            return Ok(HashMap::new());
        };
        let Ok(key) = std::str::from_utf8(&entry[..split]) else {
            return Ok(HashMap::new());
        };
        if key.starts_with("com.apple.wkms.") && result.contains_key(key) {
            return Err("duplicate Apple key metadata".into());
        }
        result.insert(key.to_string(), entry[split + 1..].to_vec());
        pos += length;
        if result.len() > 128 {
            return Err("too many Apple key metadata entries".into());
        }
    }
    Ok(result)
}

/// Refuse a key URL that isn't plain HTTPS to one of [`HOSTS`].
pub(super) fn validate_url(url: &str) -> Result<(), String> {
    let refused = || {
        "AEA key URL must use an approved Apple HTTPS distribution host; supply \
         --aea-key-file for other sources"
            .to_string()
    };
    let rest = url.strip_prefix("https://").ok_or_else(refused)?;
    if url.contains('#') {
        return Err(refused());
    }
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    if authority.contains('@') {
        return Err(refused());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    if !HOSTS.contains(&host.to_ascii_lowercase().as_str()) || port.is_some_and(|p| p != "443") {
        return Err(refused());
    }
    Ok(())
}

/// The key at `url`, following at most three redirects, each to an
/// approved host. Keys are cached for the life of the process.
pub(super) fn fetch_key(url: &str) -> Result<Vec<u8>, String> {
    static CACHE: Mutex<Option<HashMap<String, Vec<u8>>>> = Mutex::new(None);
    validate_url(url)?;
    if let Some(key) = CACHE
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .get(url)
    {
        return Ok(key.clone());
    }
    let failed = || {
        "Apple archive key download failed; supply --aea-key-file or use a Mac SDK export"
            .to_string()
    };
    let session = crate::cli::net::Session::from_env();
    let mut current = url.to_string();
    let mut redirects = 0;
    let body = loop {
        let request = crate::cli::net::Request::new(minreq::Method::Get, current.as_str())
            .header("User-Agent", "Sweetpad SDK import")
            .header("Accept", "application/x-pem-file, application/octet-stream")
            .timeout(20)
            .without_redirects();
        let response = session.send(&request).map_err(|_| failed())?;
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            redirects += 1;
            let location = response.header("location").ok_or_else(failed)?;
            if redirects > 3 {
                return Err(failed());
            }
            current = if location.starts_with('/') {
                format!("https://{}{location}", crate::cli::net::host(&current))
            } else {
                location.to_string()
            };
            validate_url(&current)?;
            continue;
        }
        if !response.ok() {
            return Err(failed());
        }
        break response.body;
    };
    if body.len() > KEY_LIMIT {
        return Err("Apple archive key response exceeds the size limit".into());
    }
    let mut cache = CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    if cache.len() >= 16 {
        cache.clear();
    }
    cache.insert(url.to_string(), body.clone());
    Ok(body)
}

/// The archive key in the FCS envelope in `auth`, an archive's metadata,
/// or `None` when it has none. `private_key` is the recipient's key when
/// the caller has it (PEM or DER); otherwise `fetch` gets it from the URL
/// the metadata names.
pub(super) fn unwrap(
    auth: &[u8],
    private_key: Option<&[u8]>,
    fetch: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Option<[u8; 32]>, String> {
    let fields = metadata(auth)?;
    let Some(response) = fields.get("com.apple.wkms.fcs-response") else {
        return Ok(None);
    };
    let invalid = || "invalid Apple FCS key envelope".to_string();
    if response.len() > 16384 {
        return Err("Apple FCS response exceeds the size limit".into());
    }
    let response: serde_json::Value = serde_json::from_slice(response).map_err(|_| invalid())?;
    let field = |name: &str| -> Result<Vec<u8>, String> {
        let text = response
            .get(name)
            .and_then(|v| v.as_str())
            .ok_or_else(invalid)?;
        STANDARD.decode(text).map_err(|_| invalid())
    };
    let encapsulated = field("enc-request")?;
    let ciphertext = field("wrapped-key")?;
    let fetched;
    let private_key = if let Some(key) = private_key {
        key
    } else {
        let url = fields
            .get("com.apple.wkms.fcs-key-url")
            .ok_or_else(invalid)?;
        let url = std::str::from_utf8(url).map_err(|_| invalid())?;
        fetched = fetch(url)?;
        &fetched
    };
    let key = hpke_open(
        &private_key_from(private_key)?,
        &encapsulated,
        &ciphertext,
        b"",
        b"",
    )?;
    <[u8; 32]>::try_from(key)
        .map(Some)
        .map_err(|_| "Apple FCS envelope did not contain a 32-byte archive key".to_string())
}

/// A P-256 private key in PEM or DER, PKCS #8 or SEC1.
fn private_key_from(data: &[u8]) -> Result<p256::SecretKey, String> {
    let key = if data.starts_with(b"-----") {
        let text = std::str::from_utf8(data).map_err(|e| e.to_string());
        text.and_then(|text| {
            p256::SecretKey::from_pkcs8_pem(text)
                .or_else(|_| p256::SecretKey::from_sec1_pem(text))
                .map_err(|e| e.to_string())
        })
    } else {
        p256::SecretKey::from_pkcs8_der(data)
            .or_else(|_| p256::SecretKey::from_sec1_der(data))
            .map_err(|e| e.to_string())
    };
    key.map_err(|_| "Apple FCS key must be a P-256 private key".to_string())
}

/// Open an HPKE base-mode message to `private_key`.
fn hpke_open(
    private_key: &p256::SecretKey,
    encapsulated: &[u8],
    ciphertext: &[u8],
    info: &[u8],
    associated: &[u8],
) -> Result<Vec<u8>, String> {
    if encapsulated.len() != 65 || ciphertext.len() > 4096 {
        return Err("invalid Apple HPKE envelope length".into());
    }
    let sender = p256::PublicKey::from_sec1_bytes(encapsulated)
        .map_err(|_| "invalid Apple FCS key envelope")?;
    let shared = p256::ecdh::diffie_hellman(private_key.to_nonzero_scalar(), sender.as_affine());
    let recipient = p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(
        &private_key.public_key(),
        false,
    );

    let extract = |suite: &[u8], salt: &[u8], label: &[u8], ikm: &[u8]| {
        let ikm = [b"HPKE-v1", suite, label, ikm].concat();
        Hkdf::<Sha256>::extract(Some(salt), &ikm).0
    };
    let expand = |suite: &[u8], prk: &[u8], label: &[u8], context: &[u8], out: &mut [u8]| {
        let size = u16::try_from(out.len()).unwrap().to_be_bytes();
        let info = [&size[..], b"HPKE-v1", suite, label, context].concat();
        Hkdf::<Sha256>::from_prk(prk)
            .expect("an extracted key is long enough")
            .expand(&info, out)
            .expect("HPKE keys are short");
    };
    let kem = b"KEM\x00\x10";
    let eae_prk = extract(kem, b"", b"eae_prk", shared.raw_secret_bytes());
    let mut secret = [0u8; 32];
    let context = [encapsulated, recipient.as_bytes()].concat();
    expand(kem, &eae_prk, b"shared_secret", &context, &mut secret);
    let suite = b"HPKE\x00\x10\x00\x01\x00\x02";
    let psk_hash = extract(suite, b"", b"psk_id_hash", b"");
    let info_hash = extract(suite, b"", b"info_hash", info);
    let context = [&[0u8][..], &psk_hash, &info_hash].concat();
    let prk = extract(suite, &secret, b"secret", b"");
    let (mut key, mut nonce) = ([0u8; 32], [0u8; 12]);
    expand(suite, &prk, b"key", &context, &mut key);
    expand(suite, &prk, b"base_nonce", &context, &mut nonce);
    aes_gcm::Aes256Gcm::new(&key.into())
        .decrypt(
            &nonce.into(),
            Payload {
                msg: ciphertext,
                aad: associated,
            },
        )
        .map_err(|_| "invalid Apple FCS key envelope".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector() -> serde_json::Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sweetpad-lib/ci/fixtures/xcode-archives/rfc9180-p256.json"
        );
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn hex(value: &serde_json::Value) -> Vec<u8> {
        let text = value.as_str().unwrap();
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn hpke_opens_the_rfc_9180_vector() {
        let vector = vector();
        let key = p256::SecretKey::from_slice(&hex(&vector["skRm"])).unwrap();
        let message = &vector["encryption"];
        let opened = hpke_open(
            &key,
            &hex(&vector["enc"]),
            &hex(&message["ct"]),
            &hex(&vector["info"]),
            &hex(&message["aad"]),
        )
        .unwrap();
        assert_eq!(opened, hex(&message["pt"]));
    }

    #[test]
    fn keys_come_only_from_apples_https_hosts() {
        validate_url("https://wkms-public.apple.com/key").unwrap();
        validate_url("https://SWDIST.apple.com:443/a?b").unwrap();
        for url in [
            "http://swdist.apple.com/key",
            "https://swdist.apple.com.evil.example/key",
            "https://swdist.apple.com@evil.example/key",
            "https://127.0.0.1/key",
            "file:///tmp/key",
            "https://user:password@swdist.apple.com/key",
            "https://swdist.apple.com:444/key",
            "https://swdist.apple.com/key#x",
        ] {
            assert!(validate_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn metadata_that_isnt_key_value_entries_reads_as_none() {
        let entry = |key: &str, value: &[u8]| {
            let body = [key.as_bytes(), b"\0", value].concat();
            [&(body.len() as u32 + 4).to_le_bytes()[..], &body].concat()
        };
        let data = [entry("a", b"1"), entry("b", b"2\x003")].concat();
        let fields = metadata(&data).unwrap();
        assert_eq!(fields["b"], b"2\x003");
        assert!(metadata(b"bplist00").unwrap().is_empty());
        let twice = [
            entry("com.apple.wkms.x", b""),
            entry("com.apple.wkms.x", b""),
        ]
        .concat();
        assert!(metadata(&twice).is_err());
    }
}
