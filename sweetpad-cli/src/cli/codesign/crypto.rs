//! The hashing and key derivation a signature and a `.p12` file need:
//! SHA-1 beside the SHA-256 in `sweetpad_lib`, HMAC, PBKDF2 and PKCS #12's
//! own key derivation, and CBC decryption over the `des` and `aes` block
//! ciphers. Signing itself is the `rsa` crate's.

use aes::cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray};
use rsa::{Pkcs1v15Sign, RsaPrivateKey};

pub(super) use sweetpad_lib::xcode_hash::sha256;

/// The hash functions the formats here use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hash {
    Sha1,
    Sha256,
}

impl Hash {
    pub fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha1 => sha1(data).to_vec(),
            Self::Sha256 => sha256(data).to_vec(),
        }
    }

    /// The output size, in bytes.
    pub fn size(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Sha256 => 32,
        }
    }
}

/// Both hashes take their input in 64-byte blocks.
const BLOCK: usize = 64;

#[must_use]
#[allow(clippy::many_single_char_names)]
pub(super) fn sha1(input: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % BLOCK != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((input.len() as u64) * 8).to_be_bytes());
    for chunk in message.chunks_exact(BLOCK) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..20 => ((b & c) | (!b & d), 0x5a82_7999),
                20..40 => (b ^ c ^ d, 0x6ed9_eba1),
                40..60 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (state, value) in h.iter_mut().zip([a, b, c, d, e]) {
            *state = state.wrapping_add(value);
        }
    }
    let mut out = [0u8; 20];
    for (bytes, word) in out.chunks_exact_mut(4).zip(h) {
        bytes.copy_from_slice(&word.to_be_bytes());
    }
    out
}

pub(super) fn hmac(hash: Hash, key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut block = if key.len() > BLOCK {
        hash.digest(key)
    } else {
        key.to_vec()
    };
    block.resize(BLOCK, 0);
    let pad = |byte: u8| block.iter().map(|b| b ^ byte).collect::<Vec<u8>>();
    let mut inner = pad(0x36);
    inner.extend_from_slice(message);
    let mut outer = pad(0x5c);
    outer.extend(hash.digest(&inner));
    hash.digest(&outer)
}

/// PBKDF2 with HMAC over `hash` (RFC 8018).
pub(super) fn pbkdf2(
    hash: Hash,
    password: &[u8],
    salt: &[u8],
    iterations: u64,
    length: usize,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(length);
    let mut index: u32 = 1;
    while out.len() < length {
        let mut message = salt.to_vec();
        message.extend_from_slice(&index.to_be_bytes());
        let mut u = hmac(hash, password, &message);
        let mut block = u.clone();
        for _ in 1..iterations {
            u = hmac(hash, password, &u);
            for (b, x) in block.iter_mut().zip(&u) {
                *b ^= x;
            }
        }
        out.extend(block);
        index += 1;
    }
    out.truncate(length);
    out
}

/// What PKCS #12's key derivation derives (RFC 7292, appendix B.3).
#[derive(Clone, Copy)]
pub(super) enum Purpose {
    Key = 1,
    Iv = 2,
    Mac = 3,
}

/// PKCS #12's own key derivation (RFC 7292, appendix B.2). `password` is
/// already the BMPString the format hashes, with its two-byte terminator.
pub(super) fn pkcs12_kdf(
    hash: Hash,
    password: &[u8],
    salt: &[u8],
    purpose: Purpose,
    iterations: u64,
    length: usize,
) -> Vec<u8> {
    let fill = |source: &[u8]| -> Vec<u8> {
        if source.is_empty() {
            return Vec::new();
        }
        let size = BLOCK * source.len().div_ceil(BLOCK);
        source.iter().copied().cycle().take(size).collect()
    };
    let mut i = fill(salt);
    i.extend(fill(password));
    let d = [purpose as u8; BLOCK];
    let mut out = Vec::with_capacity(length);
    loop {
        let mut a = hash.digest(&[&d[..], &i].concat());
        for _ in 1..iterations {
            a = hash.digest(&a);
        }
        out.extend_from_slice(&a);
        if out.len() >= length {
            out.truncate(length);
            return out;
        }
        let b: Vec<u8> = a.iter().copied().cycle().take(BLOCK).collect();
        for chunk in i.chunks_mut(BLOCK) {
            let mut carry = 1u16;
            for (x, y) in chunk.iter_mut().zip(&b).rev() {
                let sum = u16::from(*x) + u16::from(*y) + carry;
                *x = sum.to_le_bytes()[0];
                carry = sum >> 8;
            }
        }
    }
}

/// The ciphers a `.p12` file encrypts with.
#[derive(Debug, Clone, Copy)]
pub(super) enum Cipher {
    TripleDes,
    Aes,
}

/// One block cipher, decrypting a block in place.
type BlockFn = dyn Fn(&mut [u8]);

/// `data` decrypted in CBC mode, its PKCS #7 padding removed.
pub(super) fn decrypt_cbc(
    cipher: Cipher,
    key: &[u8],
    iv: &[u8],
    data: &[u8],
) -> Result<Vec<u8>, String> {
    let bad = || "a wrong key or corrupt data".to_string();
    let size = match cipher {
        Cipher::TripleDes => 8,
        Cipher::Aes => 16,
    };
    if data.is_empty() || !data.len().is_multiple_of(size) || iv.len() != size {
        return Err(bad());
    }
    let decrypt: Box<BlockFn> = match (cipher, key.len()) {
        (Cipher::TripleDes, 24) => {
            let c = des::TdesEde3::new_from_slice(key).map_err(|_| bad())?;
            Box::new(move |block| c.decrypt_block(GenericArray::from_mut_slice(block)))
        }
        (Cipher::Aes, 16) => {
            let c = aes::Aes128::new_from_slice(key).map_err(|_| bad())?;
            Box::new(move |block| c.decrypt_block(GenericArray::from_mut_slice(block)))
        }
        (Cipher::Aes, 24) => {
            let c = aes::Aes192::new_from_slice(key).map_err(|_| bad())?;
            Box::new(move |block| c.decrypt_block(GenericArray::from_mut_slice(block)))
        }
        (Cipher::Aes, 32) => {
            let c = aes::Aes256::new_from_slice(key).map_err(|_| bad())?;
            Box::new(move |block| c.decrypt_block(GenericArray::from_mut_slice(block)))
        }
        _ => return Err(bad()),
    };
    let mut out = data.to_vec();
    let mut previous = iv.to_vec();
    for block in out.chunks_mut(size) {
        let ciphertext = block.to_vec();
        decrypt(block);
        for (b, p) in block.iter_mut().zip(&previous) {
            *b ^= p;
        }
        previous = ciphertext;
    }
    let pad = usize::from(*out.last().ok_or_else(bad)?);
    if pad == 0 || pad > size || !out.ends_with(&vec![u8::try_from(pad).unwrap_or(0); pad]) {
        return Err(bad());
    }
    out.truncate(out.len() - pad);
    Ok(out)
}

/// `bytes` in lowercase hex.
pub(super) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// An RSA PKCS #1 v1.5 signature of the SHA-256 `digest`.
pub(super) fn sign_sha256(key: &RsaPrivateKey, digest: &[u8; 32]) -> Result<Vec<u8>, String> {
    // The DigestInfo prefix for SHA-256 (RFC 8017, section 9.2).
    const PREFIX: [u8; 19] = [
        0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
        0x05, 0x00, 0x04, 0x20,
    ];
    key.sign(
        Pkcs1v15Sign::new_unprefixed(),
        &[&PREFIX[..], digest].concat(),
    )
    .map_err(|e| format!("couldn't sign: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_known_vectors() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(hex(&sha1(long)), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
    }

    #[test]
    fn hmac_and_pbkdf2_known_vectors() {
        // RFC 4231, test case 2.
        assert_eq!(
            hex(&hmac(
                Hash::Sha256,
                b"Jefe",
                b"what do ya want for nothing?"
            )),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // RFC 6070, iterations 2.
        assert_eq!(
            hex(&pbkdf2(Hash::Sha1, b"password", b"salt", 2, 20)),
            "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"
        );
    }

    #[test]
    fn pkcs12_kdf_known_vector() {
        // The classic vector: password "smeg", salt 0A58CF64530D823F, one
        // iteration, a 24-byte key and an 8-byte IV.
        let password: Vec<u8> = "smeg\0".encode_utf16().flat_map(u16::to_be_bytes).collect();
        let salt = [0x0a, 0x58, 0xcf, 0x64, 0x53, 0x0d, 0x82, 0x3f];
        assert_eq!(
            hex(&pkcs12_kdf(
                Hash::Sha1,
                &password,
                &salt,
                Purpose::Key,
                1,
                24
            )),
            "8aaae6297b6cb04642ab5b077851284eb7128f1a2a7fbca3"
        );
        assert_eq!(
            hex(&pkcs12_kdf(Hash::Sha1, &password, &salt, Purpose::Iv, 1, 8)),
            "79993dfe048d3b76"
        );
    }
}
