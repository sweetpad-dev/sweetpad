//! The proofs Apple's sign-in asks for: SRP-6a over the RFC 5054 2048-bit
//! group with Apple's `s2k` and `s2k_fo` password derivations, and the
//! hashcash stamp that rate-limits sign-in.

use std::time::{Duration, Instant};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use rsa::BigUint;
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest, Sha256};

const N_HEX: &str = "AC6BDB41324A9A9BF166DE5E1389582FAF72B6651987EE07FC3192943DB56050A\
37329CBB4A099ED8193E0757767A13DD52312AB4B03310DCD7F48A9DA04FD50E8083969\
EDB767B0CF6095179A163AB3661A05FBD5FAAAE82918A9962F0B93B855F97993EC975EE\
AA80D740ADBF4FF747359D041D5C33EA71D281E446B14773BCA97B43A23FB801676BD207\
A436C6481F1D2B9078717461A5B9D32E688F87748544523B524B0D57D5EA77A2775D2EC\
FA032CFBDBF52FB3786160279004E57AE6AF874E7303CE53299CCC041C7BC308D82A5698\
F3A8D0C38271AE35F8E9DBFBB694B5C803D89F7AE435DE236D525F54759B65E372FCD68\
EF20FA7111F9E4AFF73";

pub(crate) fn n() -> BigUint {
    BigUint::parse_bytes(N_HEX.as_bytes(), 16).expect("the group prime is hex")
}

fn generator() -> BigUint {
    BigUint::from(2u32)
}

pub(crate) fn sha(parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part);
    }
    digest.finalize().into()
}

/// `value` left-padded with zeros to the group's 256 bytes.
pub(crate) fn pad(value: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 256usize.saturating_sub(value.len())];
    out.extend_from_slice(value);
    out
}

/// Fill `out` from the system's random source.
pub(crate) fn random_bytes(out: &mut [u8]) -> Result<(), String> {
    use std::io::Read as _;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(out))
        .map_err(|error| format!("Couldn't read random bytes: {error}."))
}

/// PBKDF2-HMAC-SHA256 for one 32-byte block.
pub(crate) fn pbkdf2(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let mac = Hmac::<Sha256>::new_from_slice(password).expect("HMAC takes any key length");
    let mut round = mac.clone();
    round.update(salt);
    round.update(&1u32.to_be_bytes());
    let mut block: [u8; 32] = round.finalize().into_bytes().into();
    let mut out = block;
    for _ in 1..rounds {
        let mut round = mac.clone();
        round.update(&block);
        block = round.finalize().into_bytes().into();
        for (o, b) in out.iter_mut().zip(block) {
            *o ^= b;
        }
    }
    out
}

/// The client half of one SRP exchange.
pub(crate) struct Srp {
    private: BigUint,
    /// `A`, as Apple's `a` field sends it.
    pub(crate) public: Vec<u8>,
}

impl Srp {
    pub(crate) fn new(private: BigUint) -> Self {
        let public = generator().modpow(&private, &n()).to_bytes_be();
        Self { private, public }
    }

    pub(crate) fn random() -> Result<Self, String> {
        let mut bytes = [0u8; 32];
        random_bytes(&mut bytes)?;
        bytes[31] |= 1;
        Ok(Self::new(BigUint::from_bytes_be(&bytes)))
    }

    /// `M1` and `M2`, base64, for the challenge `/signin/init` returned.
    #[allow(clippy::many_single_char_names)] // SRP's own names: N, b, x, k, u
    pub(crate) fn proofs(
        &self,
        account: &str,
        password: &str,
        challenge: &Value,
    ) -> Result<(String, String), String> {
        let invalid = || "Apple returned invalid SRP parameters.".to_string();
        let b64 = base64::engine::general_purpose::STANDARD;
        let salt = challenge["salt"]
            .as_str()
            .and_then(|s| b64.decode(s).ok())
            .ok_or_else(invalid)?;
        let raw_b = challenge["b"]
            .as_str()
            .and_then(|s| b64.decode(s).ok())
            .ok_or_else(invalid)?;
        let rounds = challenge["iteration"]
            .as_u64()
            .filter(|r| (1..=10_000_000).contains(r))
            .and_then(|r| u32::try_from(r).ok())
            .ok_or_else(invalid)?;
        let protocol = challenge["protocol"].as_str().ok_or_else(invalid)?;
        if !(1..=1024).contains(&salt.len())
            || !(1..=256).contains(&raw_b.len())
            || !matches!(protocol, "s2k" | "s2k_fo")
        {
            return Err(invalid());
        }
        let n = n();
        let zero = BigUint::from(0u32);
        let b = BigUint::from_bytes_be(&raw_b);
        if b == zero || b >= n {
            return Err(invalid());
        }
        let mut password_hash = sha(&[password.as_bytes()]).to_vec();
        if protocol == "s2k_fo" {
            password_hash = crate::cli::apple_archive::hex(&password_hash).into_bytes();
        }
        let derived = pbkdf2(&password_hash, &salt, rounds);
        let x = BigUint::from_bytes_be(&sha(&[&salt, &sha(&[b":", &derived])]));
        let n_bytes = n.to_bytes_be();
        let k = BigUint::from_bytes_be(&sha(&[&n_bytes, &pad(&[2])]));
        let u = BigUint::from_bytes_be(&sha(&[&pad(&self.public), &pad(&raw_b)]));
        if u == zero {
            return Err(invalid());
        }
        let kv = (&k * generator().modpow(&x, &n)) % &n;
        let base = (&b + &n - kv) % &n;
        let secret = base.modpow(&(&self.private + &u * &x), &n);
        if secret == zero {
            return Err(invalid());
        }
        let shared = sha(&[&secret.to_bytes_be()]);
        let xor: Vec<u8> = sha(&[&n_bytes])
            .iter()
            .zip(sha(&[&pad(&[2])]))
            .map(|(a, b)| a ^ b)
            .collect();
        let m1 = sha(&[
            &xor,
            &sha(&[account.as_bytes()]),
            &salt,
            &self.public,
            &b.to_bytes_be(),
            &shared,
        ]);
        let m2 = sha(&[&self.public, &m1, &shared]);
        Ok((b64.encode(m1), b64.encode(m2)))
    }
}

/// A hashcash stamp, `1:bits:yymmddHHMMSS:challenge::counter`, whose SHA-1
/// starts with `bits` zero bits. `date` defaults to now, in UTC.
pub(crate) fn hashcash(
    bits: Option<&str>,
    challenge: Option<&str>,
    date: Option<&str>,
) -> Result<String, String> {
    let unsupported = || "Apple returned a hashcash challenge sweetpad can't answer.".to_string();
    let bits: u32 = bits
        .and_then(|b| b.trim().parse().ok())
        .filter(|b| *b <= 24)
        .ok_or_else(unsupported)?;
    let challenge = challenge
        .filter(|c| !c.is_empty() && c.len() <= 1024)
        .ok_or_else(unsupported)?;
    let stamp = date.map_or_else(
        || {
            let time = crate::cli::codesign::time::utc_time(std::time::SystemTime::now());
            time.trim_end_matches('Z').to_string()
        },
        str::to_string,
    );
    let prefix = format!("1:{bits}:{stamp}:{challenge}::");
    let started = Instant::now();
    for counter in 0u64.. {
        let candidate = format!("{prefix}{counter}");
        if leading_zero_bits(&Sha1::digest(candidate.as_bytes())) >= bits {
            return Ok(candidate);
        }
        if counter % 4096 == 0 && started.elapsed() > Duration::from_secs(60) {
            break;
        }
    }
    Err("Apple's hashcash challenge took too long. Try signing in again later.".into())
}

fn leading_zero_bits(digest: &[u8]) -> u32 {
    let mut count = 0;
    for byte in digest {
        count += byte.leading_zeros();
        if *byte != 0 {
            break;
        }
    }
    count
}
