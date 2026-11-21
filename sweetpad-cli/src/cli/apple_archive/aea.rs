//! AEA, Apple Encrypted Archive, profiles 0 (signed), 1 (encrypted with a
//! symmetric key) and 2 (both), read as a stream: each segment's MAC is
//! checked, and the segment decrypted and checksummed, before a byte of it
//! is handed on. A valid signature shows the archive is unmodified since it
//! was signed, not that Apple signed it.
//!
//! The layout follows <https://github.com/kinnay/AEA/blob/main/FORMAT.md>
//! and Apple's AppleArchive headers.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use aes::cipher::{KeyIvInit, StreamCipher};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use p256::ecdsa::signature::Verifier as _;
use p256::pkcs8::DecodePublicKey as _;
use sha2::{Digest, Sha256};

use super::source::{other, read_full};
use super::{Keys, MAX_OUTPUT, codecs, fcs};

/// The most authentication metadata a header may carry.
const MAX_AUTH: u64 = 16 * 1024 * 1024;

type Ctr = ctr::Ctr128BE<aes::Aes256>;

fn failed() -> String {
    "AEA authentication failed (wrong key or modified archive)".into()
}

fn hkdf(key: &[u8], info: &[u8], out: &mut [u8], salt: &[u8]) {
    Hkdf::<Sha256>::new(Some(salt), key)
        .expand(info, out)
        .expect("AEA keys are short");
}

fn derived<const N: usize>(key: &[u8], info: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    hkdf(key, info, &mut out, b"");
    out
}

/// `data` once its MAC matches `expected`, decrypted when `encrypted`:
/// `key` is the MAC key, then for encryption the AES-256 key and the
/// counter's start.
fn authenticated(
    key: &[u8],
    mut data: Vec<u8>,
    associated: &[u8],
    expected: &[u8],
    encrypted: bool,
) -> Result<Vec<u8>, String> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key[..32]).unwrap();
    mac.update(associated);
    mac.update(&data);
    mac.update(&(associated.len() as u64).to_le_bytes());
    mac.verify_slice(expected).map_err(|_| failed())?;
    if encrypted {
        Ctr::new(key[32..64].into(), key[64..80].into()).apply_keystream(&mut data);
    }
    Ok(data)
}

/// The values in an archive's authentication metadata: `key\0value`
/// entries, or the strings and data in a property list (certificates).
fn metadata_values(data: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let mut values = Vec::new();
    if data.starts_with(b"bplist") || data.starts_with(b"<?xml") {
        use crate::cli::property_list::{Plist, read_info};
        let Ok(root) = read_info(data) else {
            return Ok(Vec::new());
        };
        let mut pending = vec![(&root, 0)];
        while let Some((item, depth)) = pending.pop() {
            if values.len() >= 128 {
                break;
            }
            if depth > 16 {
                return Ok(Vec::new());
            }
            match item {
                Plist::String(text) => {
                    values.push(text.as_bytes().to_vec());
                    // The binary reader gives data as hex.
                    if let Some(bytes) = from_hex(text) {
                        values.push(bytes);
                    }
                }
                Plist::Data(bytes) => values.push(bytes.clone()),
                Plist::Array(items) => pending.extend(items.iter().map(|v| (v, depth + 1))),
                Plist::Dict(entries) => pending.extend(entries.iter().map(|(_, v)| (v, depth + 1))),
                _ => {}
            }
        }
        return Ok(values);
    }
    let mut pos = 0;
    while pos < data.len() {
        let Some(length) = data.get(pos..pos + 4) else {
            return Ok(Vec::new());
        };
        let length = u32::from_le_bytes(length.try_into().unwrap()) as usize;
        if length < 5 || length > data.len() - pos {
            return Ok(Vec::new());
        }
        let entry = &data[pos + 4..pos + length];
        let Some(split) = entry.iter().position(|b| *b == 0) else {
            return Ok(Vec::new());
        };
        values.push(entry[split + 1..].to_vec());
        if values.len() > 128 {
            return Err("too many AEA authentication entries".into());
        }
        pos += length;
    }
    Ok(values)
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// A 32-byte key as `--aea-key-file` gives it: raw, `hex:`, `base64:`, or
/// bare hex or base64.
fn key_bytes(value: &[u8]) -> Option<[u8; 32]> {
    if let Ok(key) = <[u8; 32]>::try_from(value) {
        return Some(key);
    }
    let text = std::str::from_utf8(value).ok()?.trim();
    let bytes = if let Some(hex) = text.strip_prefix("hex:") {
        from_hex(hex)?
    } else if let Some(base64) = text.strip_prefix("base64:") {
        STANDARD.decode(base64).ok()?
    } else if text.len() == 64 {
        from_hex(text)?
    } else {
        STANDARD.decode(text).ok()?
    };
    <[u8; 32]>::try_from(bytes).ok()
}

/// The P-256 public keys among `values`, each once: PEM or DER keys or
/// certificates, or bare points.
fn public_keys(values: &[Vec<u8>]) -> Vec<([u8; 65], p256::PublicKey)> {
    let mut keys: Vec<([u8; 65], p256::PublicKey)> = Vec::new();
    for value in values {
        let mut candidates = vec![value.as_slice()];
        if value.len() == 67 && value.starts_with(b"\x04\x41\x04") {
            candidates.push(&value[2..]);
        }
        for candidate in candidates {
            let Some(key) = public_key(candidate) else {
                continue;
            };
            let point = p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(&key, false);
            let raw: [u8; 65] = point.as_bytes().try_into().unwrap();
            if !keys.iter().any(|(r, _)| *r == raw) {
                keys.push((raw, key));
            }
            break;
        }
    }
    keys
}

fn public_key(data: &[u8]) -> Option<p256::PublicKey> {
    use x509_cert::der::Decode as _;
    let certificate = |der: &[u8]| {
        let cert = x509_cert::Certificate::from_der(der).ok()?;
        let spki = cert.tbs_certificate.subject_public_key_info;
        p256::PublicKey::from_sec1_bytes(spki.subject_public_key.raw_bytes()).ok()
    };
    if let Ok(text) = std::str::from_utf8(data)
        && text.contains("-----BEGIN")
    {
        if let Ok(key) = p256::PublicKey::from_public_key_pem(text.trim()) {
            return Some(key);
        }
        let body: String = text
            .lines()
            .skip_while(|line| !line.starts_with("-----BEGIN CERTIFICATE-----"))
            .skip(1)
            .take_while(|line| !line.starts_with("-----END"))
            .collect();
        return certificate(&STANDARD.decode(body.trim()).ok()?);
    }
    p256::PublicKey::from_public_key_der(data)
        .ok()
        .or_else(|| certificate(data))
        .or_else(|| p256::PublicKey::from_sec1_bytes(data).ok())
}

/// The 64-bit MurmurHash2 variant AEA checksums segments with.
fn murmur64(data: &[u8]) -> u64 {
    const M: u64 = 0xc6a4_a793_5bd1_e995;
    let mut hash = 0xe223_6fdc_26a5_f6d2 ^ (data.len() as u64).wrapping_mul(M);
    let mut words = data.chunks_exact(8);
    for word in &mut words {
        let mut word = u64::from_le_bytes(word.try_into().unwrap()).wrapping_mul(M);
        word ^= word >> 47;
        word = word.wrapping_mul(M);
        hash = (hash ^ word).wrapping_mul(M);
    }
    let tail = words.remainder();
    if !tail.is_empty() {
        let mut bytes = [0u8; 8];
        bytes[..tail.len()].copy_from_slice(tail);
        hash = (hash ^ u64::from_le_bytes(bytes)).wrapping_mul(M);
    }
    hash ^= hash >> 47;
    hash = hash.wrapping_mul(M);
    hash ^ (hash >> 47)
}

fn exact(file: &mut impl Read, size: usize) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; size];
    if read_full(file, &mut buf).map_err(|e| e.to_string())? != size {
        return Err("truncated AEA archive".into());
    }
    Ok(buf)
}

/// An AEA archive's decrypted and checked payload.
pub(super) struct Reader {
    file: BufReader<File>,
    main: [u8; 32],
    encrypted: bool,
    total: u64,
    produced: u64,
    segment_size: u32,
    per_cluster: usize,
    algorithm: u8,
    checksum: u8,
    cluster_index: u32,
    /// The MAC the next cluster header must have.
    cluster_mac: [u8; 32],
    cluster: Option<Cluster>,
    block: Vec<u8>,
    at: usize,
    finished: bool,
}

struct Cluster {
    key: [u8; 32],
    entries: Vec<u8>,
    segment_macs: Vec<u8>,
    next_mac: [u8; 32],
    index: usize,
}

impl Reader {
    #[allow(clippy::too_many_lines)] // the prologue, field by field in file order
    pub(super) fn open(path: &Path, keys: &Keys) -> Result<Self, String> {
        let file =
            File::open(path).map_err(|e| format!("couldn't open {}: {e}", path.display()))?;
        let file_size = file.metadata().map_err(|e| e.to_string())?.len();
        let mut file = BufReader::with_capacity(1 << 20, file);
        let header = exact(&mut file, 12)?;
        if &header[..4] != b"AEA1" {
            return Err("not an Apple Encrypted Archive".into());
        }
        let profile = u32::from_le_bytes([header[4], header[5], header[6], 0]);
        if profile > 2 {
            return Err(format!(
                "AEA profile {profile} is unsupported; use a Mac SDK export for this archive"
            ));
        }
        let auth_size = u64::from(u32::from_le_bytes(header[8..12].try_into().unwrap()));
        if auth_size > MAX_AUTH {
            return Err("AEA authentication metadata exceeds the memory limit".into());
        }
        let auth = exact(&mut file, auth_size as usize)?;
        let signature_start = 12 + auth.len();
        let signature = exact(&mut file, [128, 0, 160][profile as usize])?;
        let signature_end = signature_start + signature.len();
        let encryption_data = if profile == 0 {
            Some(exact(&mut file, 32)?)
        } else {
            None
        };
        let salt = exact(&mut file, 32)?;
        let root_mac = exact(&mut file, 32)?;
        let encrypted_root = exact(&mut file, 48)?;
        let cluster_mac: [u8; 32] = exact(&mut file, 32)?.try_into().unwrap();
        let prologue_end = signature_end + encryption_data.as_ref().map_or(0, Vec::len) + 144;

        let values = metadata_values(&auth)?;
        let supplied = keys.archive.as_deref();
        let mut candidates: Vec<[u8; 32]> = match (&encryption_data, supplied) {
            (Some(data), _) => vec![data[..].try_into().unwrap()],
            (None, Some(supplied)) => key_bytes(supplied).into_iter().collect(),
            (None, None) => values.iter().filter_map(|v| key_bytes(v)).collect(),
        };
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|key| seen.insert(*key));
        candidates.truncate(16);
        let apple = fcs::metadata(&auth)?;
        if supplied.is_none()
            && profile != 0
            && apple.contains_key("com.apple.wkms.fcs-response")
            && !apple.contains_key("encryption_key")
        {
            candidates.clear();
        }
        if candidates.is_empty()
            && profile != 0
            && let Some(key) = fcs::unwrap(&auth, supplied, &mut fcs::fetch_key)?
        {
            candidates.push(key);
        }
        if candidates.is_empty() {
            return Err(
                "AEA decryption key is not embedded; supply --aea-key-file with a \
                        32-byte key, hex: value, or base64: value"
                    .into(),
            );
        }
        let signing: Vec<([u8; 65], Option<p256::PublicKey>)> = if profile == 1 {
            vec![([0u8; 65], None)]
        } else {
            let sources = match &keys.public {
                Some(public) => vec![public.clone()],
                None => values.clone(),
            };
            let keys = public_keys(&sources);
            if keys.is_empty() {
                return Err(
                    "AEA signing public key is not embedded; supply --aea-public-key-file".into(),
                );
            }
            keys.into_iter()
                .take(16)
                .map(|(raw, key)| (raw, Some(key)))
                .collect()
        };

        let encrypted = profile != 0;
        let mut found = None;
        'search: for material in &candidates {
            for (raw, public) in &signing {
                let raw: &[u8] = if public.is_some() { raw } else { &[] };
                let mut main = [0u8; 32];
                let info = [b"AEA_AMK", &header[4..8], raw].concat();
                hkdf(material, &info, &mut main, &salt);
                let mut root_key = [0u8; 80];
                let root_key = &mut root_key[..if encrypted { 80 } else { 32 }];
                hkdf(&main, b"AEA_RHEK", root_key, b"");
                let associated = [&cluster_mac[..], &auth].concat();
                if let Ok(root) = authenticated(
                    root_key,
                    encrypted_root.clone(),
                    &associated,
                    &root_mac,
                    encrypted,
                ) {
                    found = Some((main, *public, root));
                    break 'search;
                }
            }
        }
        let (main, public, root) = found.ok_or_else(failed)?;

        if let Some(public) = public {
            let mut signature = signature;
            if profile == 2 {
                let sek: [u8; 32] = derived(&main, b"AEA_SEK");
                let key: [u8; 80] = derived(&sek, b"AEA_SEK2");
                let mac = signature[128..].to_vec();
                signature.truncate(128);
                signature = authenticated(&key, signature, b"", &mac, true)?;
            }
            if signature.len() < 2 || signature[0] != 0x30 || usize::from(signature[1]) + 2 > 128 {
                return Err("invalid AEA signature encoding".into());
            }
            signature.truncate(usize::from(signature[1]) + 2);
            let signature = p256::ecdsa::Signature::from_der(&signature)
                .map_err(|_| "invalid AEA signature encoding")?;
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            let mut prologue = exact(&mut file, prologue_end)?;
            prologue[signature_start..signature_end].fill(0);
            p256::ecdsa::VerifyingKey::from(&public)
                .verify(&prologue, &signature)
                .map_err(|_| "AEA signature verification failed")?;
        }

        let u64_at = |at: usize| u64::from_le_bytes(root[at..at + 8].try_into().unwrap());
        let u32_at = |at: usize| u32::from_le_bytes(root[at..at + 4].try_into().unwrap());
        let (total, container_size) = (u64_at(0), u64_at(8));
        let (segment_size, per_cluster) = (u32_at(16), u32_at(20));
        let (algorithm, checksum) = (root[24], root[25]);
        if container_size != file_size || total > MAX_OUTPUT {
            return Err("invalid AEA archive size".into());
        }
        if segment_size == 0
            || segment_size as usize > codecs::MAX_BLOCK
            || per_cluster == 0
            || per_cluster > 65536
            || checksum > 2
        {
            return Err("invalid AEA cluster parameters".into());
        }
        let per_cluster = per_cluster as usize;
        if per_cluster * (8 + checksum_size(checksum) + 32) > codecs::MAX_BLOCK {
            return Err("AEA cluster metadata exceeds the memory limit".into());
        }
        file.seek(SeekFrom::Start(prologue_end as u64))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            file,
            main,
            encrypted,
            total,
            produced: 0,
            segment_size,
            per_cluster,
            algorithm,
            checksum,
            cluster_index: 0,
            cluster_mac,
            cluster: None,
            block: Vec::new(),
            at: 0,
            finished: false,
        })
    }

    fn key_size(&self) -> usize {
        if self.encrypted { 80 } else { 32 }
    }

    /// The next segment, or `None` at the end of the payload.
    fn next_block(&mut self) -> Result<Option<Vec<u8>>, String> {
        let entry_size = 8 + checksum_size(self.checksum);
        let key_size = self.key_size();
        loop {
            if let Some(cluster) = &mut self.cluster
                && cluster.index < self.per_cluster
            {
                let index = cluster.index;
                cluster.index += 1;
                let entry = &cluster.entries[index * entry_size..(index + 1) * entry_size];
                let original = u32::from_le_bytes(entry[..4].try_into().unwrap());
                let packed = u32::from_le_bytes(entry[4..8].try_into().unwrap());
                let expected = entry[8..].to_vec();
                if original == 0 && packed == 0 {
                    if self.produced != self.total {
                        return Err("AEA cluster ended before the declared output size".into());
                    }
                    continue;
                }
                if original > self.segment_size
                    || packed > self.segment_size
                    || original == 0
                    || self.produced + u64::from(original) > self.total
                {
                    return Err("invalid AEA segment range".into());
                }
                let data = exact(&mut self.file, packed as usize)?;
                let mut key = [0u8; 80];
                let key = &mut key[..key_size];
                let info = [&b"AEA_SK"[..], &(index as u32).to_le_bytes()].concat();
                hkdf(&cluster.key, &info, key, b"");
                let mac = &cluster.segment_macs[index * 32..(index + 1) * 32];
                let data = authenticated(key, data, b"", mac, self.encrypted)?;
                let data = if packed == original {
                    data
                } else {
                    codecs::decompress(self.algorithm, &data, original as usize)?
                };
                let actual = match self.checksum {
                    0 => Vec::new(),
                    1 => murmur64(&data).to_le_bytes().to_vec(),
                    _ => Sha256::digest(&data).to_vec(),
                };
                if actual != expected {
                    return Err("AEA segment checksum failed".into());
                }
                self.produced += u64::from(original);
                return Ok(Some(data));
            }
            if let Some(cluster) = self.cluster.take() {
                self.cluster_mac = cluster.next_mac;
                self.cluster_index += 1;
            }
            if self.produced >= self.total {
                self.check_padding()?;
                return Ok(None);
            }
            let info = [&b"AEA_CK"[..], &self.cluster_index.to_le_bytes()].concat();
            let key: [u8; 32] = derived(&self.main, &info);
            let headers = exact(&mut self.file, entry_size * self.per_cluster)?;
            let next_mac: [u8; 32] = exact(&mut self.file, 32)?.try_into().unwrap();
            let segment_macs = exact(&mut self.file, 32 * self.per_cluster)?;
            let mut header_key = [0u8; 80];
            let header_key = &mut header_key[..key_size];
            hkdf(&key, b"AEA_CHEK", header_key, b"");
            let associated = [&next_mac[..], &segment_macs].concat();
            let entries = authenticated(
                header_key,
                headers,
                &associated,
                &self.cluster_mac,
                self.encrypted,
            )?;
            self.cluster = Some(Cluster {
                key,
                entries,
                segment_macs,
                next_mac,
                index: 0,
            });
        }
    }

    /// Bytes after the last cluster must carry the MAC the last cluster
    /// names; with none, Apple leaves that MAC unspecified.
    fn check_padding(&mut self) -> Result<(), String> {
        let key: [u8; 32] = derived(&self.main, b"AEA_PAK");
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key).unwrap();
        let mut size = 0u64;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = self.file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            size += n as u64;
            mac.update(&buf[..n]);
        }
        if size > 0 && mac.verify_slice(&self.cluster_mac).is_err() {
            return Err("AEA padding authentication failed".into());
        }
        Ok(())
    }
}

fn checksum_size(checksum: u8) -> usize {
    [0, 8, 32][usize::from(checksum)]
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.at == self.block.len() {
            if self.finished {
                return Ok(0);
            }
            match self.next_block().map_err(other)? {
                Some(block) => {
                    self.block = block;
                    self.at = 0;
                }
                None => self.finished = true,
            }
        }
        let n = buf.len().min(self.block.len() - self.at);
        buf[..n].copy_from_slice(&self.block[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn murmur_matches_the_c_reference() {
        for (data, hash) in [
            (&b""[..], 0x9ffe_fb51_d66f_1bd9),
            (b"a", 0x87bb_71af_7e6f_83e2),
            (b"sweetpad", 0xf96d_8e1e_f560_f9b3),
            (b"sweetpad murmur!!", 0xad76_325a_88eb_1a51),
        ] {
            assert_eq!(murmur64(data), hash);
        }
    }

    #[test]
    fn key_files_take_raw_hex_and_base64_keys() {
        let key: Vec<u8> = (0..32).collect();
        let hex = super::super::hex(&key);
        assert_eq!(key_bytes(&key).unwrap().to_vec(), key);
        assert_eq!(
            key_bytes(format!("hex:{hex}\n").as_bytes())
                .unwrap()
                .to_vec(),
            key
        );
        assert_eq!(key_bytes(hex.as_bytes()).unwrap().to_vec(), key);
        let base64 = STANDARD.encode(&key);
        assert_eq!(
            key_bytes(format!("base64:{base64}").as_bytes())
                .unwrap()
                .to_vec(),
            key
        );
        assert_eq!(key_bytes(base64.as_bytes()).unwrap().to_vec(), key);
        assert!(key_bytes(b"hex:00").is_none());
    }
}
