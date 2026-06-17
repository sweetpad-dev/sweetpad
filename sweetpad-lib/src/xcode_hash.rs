//! Reproduce Xcode's 28-character DerivedData project hash.
//!
//! Xcode names each DerivedData folder `<ProjectName>-<HASH>`. The 28-char
//! `<HASH>` is derived from the absolute filesystem path of the project's
//! `.xcodeproj` or `.xcworkspace`:
//!
//! 1. MD5(path-as-utf8) → 16 bytes
//! 2. First 8 bytes → big-endian u64 "head"; last 8 → "tail"
//! 3. Encode each u64 in base-26 over 14 lowercase letters, low digit at the
//!    high index. Concatenate (head first) to get the 28-char output.
//!
//! No external dependencies for the hash itself — both MD5 (RFC 1321) and the
//! base-26 encoder are spelled out below. Unicode NFD (the one thing worth a
//! crate — see [`derived_data_hash`]) is the lone exception. The unit tests pin
//! down the algorithm against four real path → hash mappings drawn from a
//! captured Xcode DerivedData tree on the author's machine.

// RFC 1321 uses single-letter variable names by long-standing convention
// (`a`, `b`, `c`, `d` for the running state words). Keeping them improves
// readability against the spec; the same goes for the manual range loop
// that mirrors the spec's block decoding.
#![allow(
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::manual_slice_fill,
    clippy::too_many_lines
)]

/// Compute the DerivedData hash for `path` (the absolute path to a
/// `.xcodeproj` or `.xcworkspace`).
///
/// The path is canonically decomposed (Unicode NFD) before hashing. Xcode's
/// `hashStringForPath` MD5s the path NSString's UTF-8 bytes; macOS hands that
/// string back in decomposed form, so a path containing precomposed (NFC)
/// characters — e.g. `é` typed as U+00E9 rather than `e` + U+0301 — would hash
/// to a folder Xcode never created unless we decompose first. Verified against
/// real `xcodebuild -showBuildSettings` on a macOS runner: a project under a
/// precomposed `Café/` directory resolves byte-identically only with NFD, and
/// it holds on APFS (which preserves bytes on disk) too, not just HFS+. ASCII
/// paths are unaffected (NFD is a no-op), so the pinned ASCII vectors hold.
#[must_use]
pub fn derived_data_hash(path: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let normalized: String = path.nfd().collect();
    let digest = md5(normalized.as_bytes());
    let head = u64::from_be_bytes(digest[..8].try_into().expect("16-byte digest"));
    let tail = u64::from_be_bytes(digest[8..].try_into().expect("16-byte digest"));
    let mut out = [b'_'; 28];
    base26_into(head, &mut out[..14]);
    base26_into(tail, &mut out[14..]);
    String::from_utf8(out.to_vec()).expect("ASCII letters")
}

/// Fill `dst` (14 bytes) with the base-26 encoding of `n`, lowest digit
/// at the rightmost index. Empty digits become `'a'` — this matches
/// Xcode's behaviour for short values.
fn base26_into(mut n: u64, dst: &mut [u8]) {
    debug_assert_eq!(dst.len(), 14);
    for slot in dst.iter_mut().rev() {
        *slot = b'a' + u8::try_from(n % 26).expect("0..26");
        n /= 26;
    }
}

// =====================================================================
// MD5 (RFC 1321)
// =====================================================================

/// RFC 1321 MD5. Pure-Rust, no allocation beyond the working state.
fn md5(input: &[u8]) -> [u8; 16] {
    // Per RFC 1321 — these are the initial state constants.
    let mut a: u32 = 0x6745_2301;
    let mut b: u32 = 0xefcd_ab89;
    let mut c: u32 = 0x98ba_dcfe;
    let mut d: u32 = 0x1032_5476;

    let bit_len = (input.len() as u64).wrapping_mul(8);

    // Pad to (length % 64 == 56): append 0x80, then zeros, then the
    // 64-bit little-endian bit length. The padded length is always
    // a multiple of 64 — handled below by iterating block by block,
    // synthesising the tail block(s) once `input` runs out.
    let mut total_len = input.len() + 1; // for the 0x80 byte
    while total_len % 64 != 56 {
        total_len += 1;
    }
    total_len += 8; // 64-bit length
    let mut offset = 0;
    let mut block = [0u8; 64];
    while offset < total_len {
        // Fill `block` with the next 64 bytes of (input ++ padding ++ length).
        for slot in &mut block {
            *slot = 0;
        }
        for i in 0..64 {
            let pos = offset + i;
            if pos < input.len() {
                block[i] = input[pos];
            } else if pos == input.len() {
                block[i] = 0x80;
            } else if pos >= total_len - 8 {
                let shift = (pos - (total_len - 8)) * 8;
                block[i] = ((bit_len >> shift) & 0xff) as u8;
            } // else zero (already cleared above)
        }
        process_block(&block, &mut a, &mut b, &mut c, &mut d);
        offset += 64;
    }

    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&a.to_le_bytes());
    out[4..8].copy_from_slice(&b.to_le_bytes());
    out[8..12].copy_from_slice(&c.to_le_bytes());
    out[12..].copy_from_slice(&d.to_le_bytes());
    out
}

fn process_block(block: &[u8; 64], a0: &mut u32, b0: &mut u32, c0: &mut u32, d0: &mut u32) {
    // Decode the block into 16 little-endian u32 words.
    let mut x = [0u32; 16];
    for (i, word) in x.iter_mut().enumerate() {
        let base = i * 4;
        *word = u32::from_le_bytes([
            block[base],
            block[base + 1],
            block[base + 2],
            block[base + 3],
        ]);
    }

    let (mut a, mut b, mut c, mut d) = (*a0, *b0, *c0, *d0);

    // Round 1: F(b,c,d) = (b & c) | (!b & d)
    macro_rules! ff {
        ($a:ident, $b:ident, $c:ident, $d:ident, $k:expr, $s:expr, $i:expr) => {
            $a = $a
                .wrapping_add(($b & $c) | (!$b & $d))
                .wrapping_add(x[$k])
                .wrapping_add($i);
            $a = $a.rotate_left($s).wrapping_add($b);
        };
    }
    // Round 2: G(b,c,d) = (b & d) | (c & !d)
    macro_rules! gg {
        ($a:ident, $b:ident, $c:ident, $d:ident, $k:expr, $s:expr, $i:expr) => {
            $a = $a
                .wrapping_add(($b & $d) | ($c & !$d))
                .wrapping_add(x[$k])
                .wrapping_add($i);
            $a = $a.rotate_left($s).wrapping_add($b);
        };
    }
    // Round 3: H(b,c,d) = b ^ c ^ d
    macro_rules! hh {
        ($a:ident, $b:ident, $c:ident, $d:ident, $k:expr, $s:expr, $i:expr) => {
            $a = $a
                .wrapping_add($b ^ $c ^ $d)
                .wrapping_add(x[$k])
                .wrapping_add($i);
            $a = $a.rotate_left($s).wrapping_add($b);
        };
    }
    // Round 4: I(b,c,d) = c ^ (b | !d)
    macro_rules! ii {
        ($a:ident, $b:ident, $c:ident, $d:ident, $k:expr, $s:expr, $i:expr) => {
            $a = $a
                .wrapping_add($c ^ ($b | !$d))
                .wrapping_add(x[$k])
                .wrapping_add($i);
            $a = $a.rotate_left($s).wrapping_add($b);
        };
    }

    // Round 1
    ff!(a, b, c, d, 0, 7, 0xd76a_a478);
    ff!(d, a, b, c, 1, 12, 0xe8c7_b756);
    ff!(c, d, a, b, 2, 17, 0x2420_70db);
    ff!(b, c, d, a, 3, 22, 0xc1bd_ceee);
    ff!(a, b, c, d, 4, 7, 0xf57c_0faf);
    ff!(d, a, b, c, 5, 12, 0x4787_c62a);
    ff!(c, d, a, b, 6, 17, 0xa830_4613);
    ff!(b, c, d, a, 7, 22, 0xfd46_9501);
    ff!(a, b, c, d, 8, 7, 0x6980_98d8);
    ff!(d, a, b, c, 9, 12, 0x8b44_f7af);
    ff!(c, d, a, b, 10, 17, 0xffff_5bb1);
    ff!(b, c, d, a, 11, 22, 0x895c_d7be);
    ff!(a, b, c, d, 12, 7, 0x6b90_1122);
    ff!(d, a, b, c, 13, 12, 0xfd98_7193);
    ff!(c, d, a, b, 14, 17, 0xa679_438e);
    ff!(b, c, d, a, 15, 22, 0x49b4_0821);

    // Round 2
    gg!(a, b, c, d, 1, 5, 0xf61e_2562);
    gg!(d, a, b, c, 6, 9, 0xc040_b340);
    gg!(c, d, a, b, 11, 14, 0x265e_5a51);
    gg!(b, c, d, a, 0, 20, 0xe9b6_c7aa);
    gg!(a, b, c, d, 5, 5, 0xd62f_105d);
    gg!(d, a, b, c, 10, 9, 0x0244_1453);
    gg!(c, d, a, b, 15, 14, 0xd8a1_e681);
    gg!(b, c, d, a, 4, 20, 0xe7d3_fbc8);
    gg!(a, b, c, d, 9, 5, 0x21e1_cde6);
    gg!(d, a, b, c, 14, 9, 0xc337_07d6);
    gg!(c, d, a, b, 3, 14, 0xf4d5_0d87);
    gg!(b, c, d, a, 8, 20, 0x455a_14ed);
    gg!(a, b, c, d, 13, 5, 0xa9e3_e905);
    gg!(d, a, b, c, 2, 9, 0xfcef_a3f8);
    gg!(c, d, a, b, 7, 14, 0x676f_02d9);
    gg!(b, c, d, a, 12, 20, 0x8d2a_4c8a);

    // Round 3
    hh!(a, b, c, d, 5, 4, 0xfffa_3942);
    hh!(d, a, b, c, 8, 11, 0x8771_f681);
    hh!(c, d, a, b, 11, 16, 0x6d9d_6122);
    hh!(b, c, d, a, 14, 23, 0xfde5_380c);
    hh!(a, b, c, d, 1, 4, 0xa4be_ea44);
    hh!(d, a, b, c, 4, 11, 0x4bde_cfa9);
    hh!(c, d, a, b, 7, 16, 0xf6bb_4b60);
    hh!(b, c, d, a, 10, 23, 0xbebf_bc70);
    hh!(a, b, c, d, 13, 4, 0x289b_7ec6);
    hh!(d, a, b, c, 0, 11, 0xeaa1_27fa);
    hh!(c, d, a, b, 3, 16, 0xd4ef_3085);
    hh!(b, c, d, a, 6, 23, 0x0488_1d05);
    hh!(a, b, c, d, 9, 4, 0xd9d4_d039);
    hh!(d, a, b, c, 12, 11, 0xe6db_99e5);
    hh!(c, d, a, b, 15, 16, 0x1fa2_7cf8);
    hh!(b, c, d, a, 2, 23, 0xc4ac_5665);

    // Round 4
    ii!(a, b, c, d, 0, 6, 0xf429_2244);
    ii!(d, a, b, c, 7, 10, 0x432a_ff97);
    ii!(c, d, a, b, 14, 15, 0xab94_23a7);
    ii!(b, c, d, a, 5, 21, 0xfc93_a039);
    ii!(a, b, c, d, 12, 6, 0x655b_59c3);
    ii!(d, a, b, c, 3, 10, 0x8f0c_cc92);
    ii!(c, d, a, b, 10, 15, 0xffef_f47d);
    ii!(b, c, d, a, 1, 21, 0x8584_5dd1);
    ii!(a, b, c, d, 8, 6, 0x6fa8_7e4f);
    ii!(d, a, b, c, 15, 10, 0xfe2c_e6e0);
    ii!(c, d, a, b, 6, 15, 0xa301_4314);
    ii!(b, c, d, a, 13, 21, 0x4e08_11a1);
    ii!(a, b, c, d, 4, 6, 0xf753_7e82);
    ii!(d, a, b, c, 11, 10, 0xbd3a_f235);
    ii!(c, d, a, b, 2, 15, 0x2ad7_d2bb);
    ii!(b, c, d, a, 9, 21, 0xeb86_d391);

    *a0 = a0.wrapping_add(a);
    *b0 = b0.wrapping_add(b);
    *c0 = c0.wrapping_add(c);
    *d0 = d0.wrapping_add(d);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        let mut out = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            let _ = write!(out, "{b:02x}");
        }
        out
    }

    // RFC 1321 test vectors. These pin the MD5 implementation independent of
    // the base-26 encoder above.
    #[test]
    fn md5_rfc1321_empty() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
    }
    #[test]
    fn md5_rfc1321_a() {
        assert_eq!(hex(&md5(b"a")), "0cc175b9c0f1b6a831c399e269772661");
    }
    #[test]
    fn md5_rfc1321_abc() {
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
    }
    #[test]
    fn md5_rfc1321_message_digest() {
        assert_eq!(
            hex(&md5(b"message digest")),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
    }
    #[test]
    fn md5_rfc1321_alphabet() {
        assert_eq!(
            hex(&md5(b"abcdefghijklmnopqrstuvwxyz")),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
    }
    #[test]
    fn md5_rfc1321_long() {
        assert_eq!(
            hex(&md5(
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
            )),
            "d174ab98d277d9f5a5611c2c9f419d9f"
        );
    }
    #[test]
    fn md5_long_input_spans_multiple_blocks() {
        // A 1000-byte input forces the padding + length to spill into a
        // second tail block (input is 1000 bytes, last block boundary is
        // at 1024; padding ends at 1024).
        let input = vec![b'a'; 1000];
        assert_eq!(hex(&md5(&input)), "cabe45dcc9ae5b66ba86600cca6b8ba8");
    }

    // Real DerivedData → workspace-path mappings captured from the author's
    // local `~/Library/Developer/Xcode/DerivedData/` tree. Each tuple is
    // `(absolute_path_to_workspace_or_project, expected_28_char_hash)`.
    #[test]
    fn xcode_hash_against_known_kingfisher_workspace() {
        let path =
            "/Users/hyzyla_home/Developer/sweetpad-lib/corpus/kingfisher/Kingfisher.xcworkspace";
        assert_eq!(derived_data_hash(path), "bsyqrgpmdpgiztchzytjjxsrwpeh");
    }
    #[test]
    fn xcode_hash_against_known_alamofire_workspace() {
        let path =
            "/Users/hyzyla_home/Developer/sweetpad-lib/corpus/alamofire/Alamofire.xcworkspace";
        assert_eq!(derived_data_hash(path), "feaqvuepuyjdicgjemrbxfrsxmtp");
    }
    #[test]
    fn xcode_hash_against_known_icecubes_project() {
        let path =
            "/Users/hyzyla_home/Developer/sweetpad-lib/corpus/ice-cubes/IceCubesApp.xcodeproj";
        assert_eq!(derived_data_hash(path), "ghsrshhtrtkrwmapcbfqgxikfpkc");
    }
    #[test]
    fn xcode_hash_against_known_netnewswire_project() {
        let path =
            "/Users/hyzyla_home/Developer/sweetpad-lib/corpus/netnewswire/NetNewsWire.xcodeproj";
        assert_eq!(derived_data_hash(path), "bpuyxkpvfxwatagjdqrifpslpucc");
    }

    #[test]
    fn hash_output_is_28_lowercase_ascii() {
        let h = derived_data_hash("/some/path");
        assert_eq!(h.len(), 28);
        assert!(h.bytes().all(|b| b.is_ascii_lowercase()));
    }

    /// A non-ASCII path hashes the same whether the caller spells it
    /// precomposed (NFC) or decomposed (NFD): Xcode keys off the filesystem's
    /// decomposed form, so we normalize to NFD before MD5. Here `é` is written
    /// both as U+00E9 and as `e` + combining acute (U+0301); both must agree,
    /// and both must equal hashing the explicit NFD spelling.
    #[test]
    fn hash_is_invariant_to_unicode_composition() {
        let nfc = "/Users/me/Café.xcodeproj"; // 'é' = U+00E9
        let nfd = "/Users/me/Cafe\u{0301}.xcodeproj"; // 'e' + U+0301
        assert_ne!(nfc, nfd, "the two spellings differ byte-for-byte");
        assert_eq!(derived_data_hash(nfc), derived_data_hash(nfd));
    }

    /// NFD normalization must not perturb the pinned ASCII vectors — it is a
    /// no-op on ASCII, so the hash is exactly the MD5 of the raw bytes.
    #[test]
    fn ascii_paths_are_unchanged_by_normalization() {
        let path =
            "/Users/hyzyla_home/Developer/sweetpad-lib/corpus/alamofire/Alamofire.xcworkspace";
        assert_eq!(derived_data_hash(path), "feaqvuepuyjdicgjemrbxfrsxmtp");
    }
}
