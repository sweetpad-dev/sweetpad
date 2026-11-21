//! LZRAVEN (Apple's `COMPRESSION_LZRAVEN`, 0xD05), the codec of recent
//! Xcode `.xip` payloads (`pbzm`) and of AEA archives made with
//! `-a lzraven`. A port of liblzraven's scalar decoder (0BSD, revision
//! `700b88c4b2e1b1d96e499d172dfa0b8e5d9134ab` of
//! <https://github.com/anat0m1a/liblzraven>), whose `SPEC.md` describes the
//! format: an 8-byte header, then blocks of up to 512 KiB, each stored or
//! coded with 8-way interleaved adaptive rANS over an LZMA-like grammar,
//! then the BCJ filters each block's flags ask for.
//!
//! The input is untrusted: every size, distance and offset is checked, and
//! a malformed stream is an error, never a short output. There is no
//! checksum in the stream; the container above carries one.

use std::fmt;

const MAGIC: u32 = 0xECCA_002B;
/// Every block decodes to this many bytes, except the last.
const BLOCK_SIZE: usize = 0x80000;
/// Apple's limit: 8192 blocks of 512 KiB, 4 GiB.
const MAX_BLOCKS: usize = 8192;
/// A coded block's last 32 bytes are not entropy-coded.
const TAIL: usize = 32;
const LADDER_MAX: u32 = 330;
/// The probability total, 2^14.
const M: u32 = 16384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    Magic,
    Params,
    Truncated,
    Framing,
    OutputFull,
    TooManyBlocks,
    Distance,
    Corrupt,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Magic => "bad stream magic",
            Self::Params => "reserved params bits set",
            Self::Truncated => "truncated stream",
            Self::Framing => "bad block framing",
            Self::OutputFull => "output larger than declared",
            Self::TooManyBlocks => "stream exceeds the block limit",
            Self::Distance => "match distance out of range",
            Self::Corrupt => "corrupt entropy-coded block",
        })
    }
}

/// Decode a whole stream whose output is at most `capacity` bytes.
pub(crate) fn decode(src: &[u8], capacity: usize) -> Result<Vec<u8>, Error> {
    // The shortest stream is the header and a terminator.
    if src.len() < 12 {
        return Err(Error::Truncated);
    }
    if rd32(src, 0) != MAGIC {
        return Err(Error::Magic);
    }
    // Bits 31..26 are a dictionary size the decoder doesn't use.
    if rd32(src, 4) & 0x03FF_FFFF != 0 {
        return Err(Error::Params);
    }
    let mut dst = vec![0u8; capacity];
    let mut model = Box::new(Model::new());
    let mut flags = Vec::new();
    let (mut out, mut header) = (0usize, 8usize);
    loop {
        let word = rd32(src, header);
        // A header with no size ends the stream.
        if word < 0x1000 {
            break;
        }
        let size = (word >> 12) as usize;
        let block_flags = (word & 0xFFF) as u16;
        let payload = header + 4;
        if out >= dst.len() {
            return Err(Error::OutputFull);
        }
        // Only the last block may be short of 512 KiB.
        if out % BLOCK_SIZE != 0 {
            return Err(Error::Framing);
        }
        if size > src.len() - payload {
            return Err(Error::Truncated);
        }
        if flags.len() >= MAX_BLOCKS {
            return Err(Error::TooManyBlocks);
        }
        flags.push(block_flags);
        let written = if block_flags & 1 != 0 {
            if size > dst.len() - out {
                return Err(Error::OutputFull);
            }
            dst[out..out + size].copy_from_slice(&src[payload..payload + size]);
            size
        } else {
            decode_block(&mut model, &src[payload..payload + size], &mut dst, out)?
        };
        out += written;
        header = payload + size;
        // The terminator is required.
        if header > src.len() || src.len() - header < 4 {
            return Err(Error::Truncated);
        }
    }
    dst.truncate(out);
    if out > 0 {
        apply_filters(&mut dst, &flags);
    }
    Ok(dst)
}

fn rd32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

/// The adaptive model, which persists across a stream's blocks: 16-entry
/// CDFs, binary probabilities, four repeat distances, the last block's
/// length and the grammar's state.
struct Model {
    is_match: [[u16; 4]; 14],
    cmd: [[u16; 16]; 14],
    lam_hi: [[[[u16; 16]; 16]; 4]; 3],
    lam_lo: [[[u16; 16]; 16]; 3],
    lit_hi: [[[u16; 16]; 4]; 4],
    lit_lo: [[[[u16; 16]; 16]; 4]; 4],
    rep_short_len: [[u16; 16]; 4],
    rep_len1: [[u16; 16]; 4],
    rep_len2: [[u16; 16]; 4],
    rep_len_esc10: [u16; 16],
    rep_len_esc14: [u16; 16],
    mat_len1: [[[u16; 16]; 4]; 7],
    mat_len2: [[u16; 16]; 4],
    mat_len_esc10: [u16; 16],
    mat_len_esc14: [u16; 16],
    dist: [[[u16; 16]; 2]; 7],
    reps: [u32; 4],
    prev_len: u32,
    state: u8,
}

/// The command CDFs start from these counts, times 256; literal states
/// (0..=8) use the first, match states the second.
const LIT_STATE_COUNT: [u8; 16] = [17, 1, 1, 1, 1, 1, 1, 17, 17, 1, 1, 1, 1, 1, 1, 1];
const MAT_STATE_COUNT: [u8; 16] = [17, 17, 17, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1];
const UNIFORM: [u16; 16] = {
    let mut cdf = [0u16; 16];
    let mut k = 0;
    while k < 16 {
        cdf[k] = 1024 * (k as u16 + 1);
        k += 1;
    }
    cdf
};

impl Model {
    fn new() -> Self {
        let counts = |table: &[u8; 16]| {
            let mut cdf = [0u16; 16];
            let mut total = 0u16;
            for (slot, count) in cdf.iter_mut().zip(table) {
                total += u16::from(*count);
                *slot = total * 256;
            }
            cdf
        };
        let mut cmd = [[0u16; 16]; 14];
        for (state, cdf) in cmd.iter_mut().enumerate() {
            *cdf = counts(if state <= 8 {
                &LIT_STATE_COUNT
            } else {
                &MAT_STATE_COUNT
            });
        }
        Self {
            is_match: [[8192; 4]; 14],
            cmd,
            lam_hi: [[[UNIFORM; 16]; 4]; 3],
            lam_lo: [[UNIFORM; 16]; 3],
            lit_hi: [[UNIFORM; 4]; 4],
            lit_lo: [[[UNIFORM; 16]; 4]; 4],
            rep_short_len: [UNIFORM; 4],
            rep_len1: [UNIFORM; 4],
            rep_len2: [UNIFORM; 4],
            rep_len_esc10: UNIFORM,
            rep_len_esc14: UNIFORM,
            mat_len1: [[UNIFORM; 4]; 7],
            mat_len2: [UNIFORM; 4],
            mat_len_esc10: UNIFORM,
            mat_len_esc14: UNIFORM,
            dist: [[UNIFORM; 2]; 7],
            reps: [1; 4],
            prev_len: 0,
            state: 0,
        }
    }
}

/// Literals since the last match, saturating at 4; 0..=2 select the
/// matched-literal path.
const LIT_LAM_CTX: [u8; 14] = [4, 3, 3, 2, 1, 2, 1, 2, 1, 0, 0, 0, 0, 0];
/// The state after a literal.
const LIT_ROW: [u8; 14] = [0, 0, 0, 2, 3, 1, 5, 1, 7, 4, 6, 8, 6, 8];

/// The state after a command 0..=15, or after a literal (16).
fn next_state(cmd: u32, state: u8) -> u8 {
    if cmd == 16 {
        LIT_ROW[usize::from(state)]
    } else if cmd <= 6 {
        if state <= 8 { 11 } else { 13 }
    } else if cmd == 7 {
        if state <= 8 { 9 } else { 12 }
    } else if state <= 8 {
        10
    } else {
        12
    }
}

/// Where a symbol's CDF lanes move: toward these below it and, by rate,
/// toward the high targets above it.
const LO_TARGET: [u16; 16] = [
    32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 480, 512,
];
const HI_TARGET: [[u16; 16]; 2] = [
    [
        15967, 15999, 16031, 16063, 16095, 16127, 16159, 16191, 16223, 16255, 16287, 16319, 16351,
        16383, 16415, 16447,
    ],
    [
        16031, 16063, 16095, 16127, 16159, 16191, 16223, 16255, 16287, 16319, 16351, 16383, 16415,
        16447, 16479, 16511,
    ],
];

/// The entropy decoder: eight interleaved 64-bit rANS states over the
/// block's little-endian 32-bit words, read round-robin.
struct Rans<'a> {
    st: [u64; 8],
    words: &'a [u8],
    pos: usize,
    count: usize,
    next: u32,
    /// Sticky: the words ran out.
    err: bool,
}

impl<'a> Rans<'a> {
    /// Seed the states from the head of a block, or `None` when it's too
    /// short to.
    fn new(payload: &'a [u8]) -> Option<Self> {
        let count = payload.len() / 4;
        if count < 2 {
            return None;
        }
        let mut rans = Self {
            st: [0; 8],
            words: payload,
            pos: 2,
            count,
            next: 0,
            err: false,
        };
        rans.st[0] = (u64::from(rd32(payload, 0)) << 32) | u64::from(rd32(payload, 4));
        // States 1..8 come out of state 0: a raw low half, then the high
        // half as a 5-bit exponent and its mantissa.
        for i in 1..8 {
            let lo = rans.bypass_at(0, 32);
            let k = rans.bypass_at(0, 5) as u32;
            let hi = (1u64 << k) + if k > 0 { rans.bypass_at(0, k) } else { 0 };
            rans.st[i] = lo | (hi << 32);
        }
        (!rans.err).then_some(rans)
    }

    fn renorm(&mut self, i: usize) {
        if self.st[i] >> 32 == 0 {
            if self.pos >= self.count {
                self.err = true;
                return;
            }
            self.st[i] = (self.st[i] << 32) | u64::from(rd32(self.words, 4 * self.pos));
            self.pos += 1;
        }
    }

    fn slot(&mut self) -> usize {
        let i = (self.next & 7) as usize;
        self.next = self.next.wrapping_add(1);
        i
    }

    /// `n` (at most 32) uniform bits from state `i`.
    fn bypass_at(&mut self, i: usize, n: u32) -> u64 {
        self.renorm(i);
        let value = self.st[i] & ((1u64 << n) - 1);
        self.st[i] >>= n;
        value
    }

    fn bypass(&mut self, n: u32) -> u32 {
        let i = self.slot();
        self.bypass_at(i, n) as u32
    }

    /// One adaptive binary symbol; `p` is the probability of 0, out of `M`.
    fn bit(&mut self, p: &mut u16) -> u32 {
        let i = self.slot();
        self.renorm(i);
        let st = self.st[i];
        let x = (st & 0x3FFF) as u32;
        let prob = u32::from(*p);
        let (bit, cum, freq) = if x < prob {
            *p = (prob + ((M - prob) >> 5)) as u16;
            (0, 0, prob)
        } else {
            *p = (prob - (prob >> 5)) as u16;
            (1, prob, M - prob)
        };
        self.st[i] = u64::from(freq)
            .wrapping_mul(st >> 14)
            .wrapping_add(u64::from(x))
            .wrapping_sub(u64::from(cum));
        bit
    }

    /// One adaptive 16-ary symbol from an ascending CDF whose last entry is
    /// `M`, then the CDF adapts at `rate` (6 or 7).
    fn sym(&mut self, cdf: &mut [u16; 16], rate: u32) -> u32 {
        let i = self.slot();
        self.renorm(i);
        let st = self.st[i];
        let x = (st & 0x3FFF) as u32;
        let greater = cdf.iter().filter(|&&c| u32::from(c) > x).count() as u32;
        // cdf[15] == M > x, so at least one lane is greater.
        let symbol = 16 - greater;
        let lo = u32::from(cdf[((symbol + 15) & 15) as usize] & 0x3FFF);
        let freq = u32::from(cdf[symbol as usize].wrapping_sub(lo as u16));
        let high = &HI_TARGET[(rate - 6) as usize];
        for (j, lane) in cdf.iter_mut().enumerate() {
            let target = if u32::from(*lane) > x {
                high[j]
            } else {
                LO_TARGET[j]
            };
            let delta = i32::from(target) - i32::from(*lane);
            *lane = (i32::from(*lane) + (delta >> rate)) as u16;
        }
        self.st[i] = u64::from(freq)
            .wrapping_mul(st >> 14)
            .wrapping_add(u64::from(x))
            .wrapping_sub(u64::from(lo));
        symbol
    }

    /// The length ladder, 0..=330; the caller adds the base.
    fn ladder(
        &mut self,
        rung1: &mut [u16; 16],
        esc10: &mut [u16; 16],
        esc14: &mut [u16; 16],
        rung2: &mut [u16; 16],
    ) -> u32 {
        let first = self.sym(rung1, 6);
        if first <= 9 {
            return first;
        }
        if first == 15 {
            return LADDER_MAX;
        }
        let (high, second) = match first {
            10 => (10, self.sym(esc10, 6)),
            14 => {
                let high = self.sym(esc14, 6) + 14;
                (high, self.sym(rung2, 6))
            }
            _ => (first, self.sym(rung2, 6)),
        };
        16 * high + second - 150
    }
}

/// Decode a coded block into `dst` at `start`, where the whole output so
/// far is the match window. Returns the block's length.
#[allow(clippy::too_many_lines)] // one grammar loop, as liblzraven's block_body.h has it
fn decode_block(
    m: &mut Model,
    payload: &[u8],
    dst: &mut [u8],
    start: usize,
) -> Result<usize, Error> {
    let mut r = Rans::new(payload).ok_or(Error::Corrupt)?;
    // The size: a 5-bit exponent from state 0, its mantissa from state 1,
    // zig-zag decoded and added to the last block's.
    let k = r.bypass_at(0, 5) as u32;
    let value = if k > 0 {
        let value = (r.bypass_at(1, k) + ((1u64 << k) - 1)) as u32;
        r.next = 2;
        value
    } else {
        r.next = 1;
        0
    };
    if r.err {
        return Err(Error::Corrupt);
    }
    let delta = (value >> 1) ^ 0u32.wrapping_sub(value & 1);
    m.prev_len = m.prev_len.wrapping_add(delta);
    let size = m.prev_len as usize;
    if !(2 * TAIL..=BLOCK_SIZE).contains(&size) {
        return Err(Error::Framing);
    }
    if size > dst.len() - start {
        return Err(Error::OutputFull);
    }
    let coded_end = start + size - TAIL;
    let mut pos = start;
    // A stream's first byte has no byte before it to take a context from.
    if pos == 0 {
        dst[0] = r.bypass(8) as u8;
        pos = 1;
    }
    while pos < coded_end {
        if r.err {
            return Err(Error::Corrupt);
        }
        let ps = pos & 3;
        let state = m.state;
        if r.bit(&mut m.is_match[usize::from(state)][ps]) == 1 {
            let t2 = usize::from(dst[pos - 1] >> 6);
            let lam = usize::from(LIT_LAM_CTX[usize::from(state)]);
            let (hi, lo) = if lam <= 2 {
                // A matched literal, in the context of the byte one rep0
                // back.
                let rep0 = m.reps[0] as usize;
                if rep0 == 0 || rep0 > pos {
                    return Err(Error::Distance);
                }
                let matched = usize::from(dst[pos - rep0]);
                let hi = r.sym(&mut m.lam_hi[lam][t2][matched >> 4], 6);
                let lo = if hi as usize == matched >> 4 {
                    r.sym(&mut m.lam_lo[lam][matched & 15], 6)
                } else {
                    r.sym(&mut m.lit_lo[ps][t2][hi as usize], 7)
                };
                (hi, lo)
            } else {
                let hi = r.sym(&mut m.lit_hi[ps][t2], 7);
                (hi, r.sym(&mut m.lit_lo[ps][t2][hi as usize], 7))
            };
            dst[pos] = ((hi << 4) | lo) as u8;
            m.state = next_state(16, state);
            pos += 1;
            continue;
        }
        let cmd = r.sym(&mut m.cmd[usize::from(state)], 6);
        let (length, distance) = if cmd <= 6 {
            // A fresh distance; the command is its class.
            let c = cmd as usize;
            let length = 2 + r.ladder(
                &mut m.mat_len1[c][ps],
                &mut m.mat_len_esc10,
                &mut m.mat_len_esc14,
                &mut m.mat_len2[ps],
            );
            let d1 = u64::from(r.sym(&mut m.dist[c][0], 7));
            let d2 = u64::from(r.sym(&mut m.dist[c][1], 7));
            let distance = if cmd == 0 {
                (d1 << 4) + d2 + 1
            } else {
                let kk = (d1 >> 2) as u32 + 4 * cmd;
                let extra = u64::from(r.bypass(kk - 2));
                ((4 | (d1 & 3)) << (kk + 2)) + (extra << 4) + d2 + 1
            };
            if distance == 0 || distance > pos as u64 {
                return Err(Error::Distance);
            }
            m.reps = [distance as u32, m.reps[0], m.reps[1], m.reps[2]];
            (length as usize, distance as usize)
        } else if cmd == 7 {
            let distance = m.reps[0] as usize;
            if distance == 0 || distance > pos {
                return Err(Error::Distance);
            }
            (1, distance)
        } else {
            let slot = if cmd <= 11 { cmd - 8 } else { cmd - 12 } as usize;
            let distance = m.reps[slot] as usize;
            let length = if cmd <= 11 {
                2 + r.sym(&mut m.rep_short_len[ps], 6)
            } else {
                18 + r.ladder(
                    &mut m.rep_len1[ps],
                    &mut m.rep_len_esc10,
                    &mut m.rep_len_esc14,
                    &mut m.rep_len2[ps],
                )
            };
            if distance == 0 || distance > pos {
                return Err(Error::Distance);
            }
            // The distance used moves to the front.
            m.reps[..=slot].rotate_right(1);
            (length as usize, distance)
        };
        // A match may end on the coded region's end but not past it.
        if length > coded_end - pos {
            return Err(Error::Corrupt);
        }
        let from = pos - distance;
        if distance >= length {
            dst.copy_within(from..from + length, pos);
        } else {
            // Overlapping: each byte may be one this copy wrote.
            for n in 0..length {
                dst[pos + n] = dst[from + n];
            }
        }
        pos += length;
        m.state = next_state(cmd, state);
    }
    // The uncoded tail: the eight states, each seeded with 1 in the high
    // half, end holding the block's last 32 bytes.
    if r.err || r.st.iter().any(|st| st >> 32 != 1) || r.pos != r.count {
        return Err(Error::Corrupt);
    }
    for (i, st) in r.st.iter().enumerate() {
        let at = coded_end + 4 * i;
        dst[at..at + 4].copy_from_slice(&(*st as u32).to_le_bytes());
    }
    Ok(size)
}

/// Undo the executable filters, per 512 KiB block: x86 first, then ARM64,
/// the reverse of the encoder. Offsets are into the whole output.
fn apply_filters(dst: &mut [u8], flags: &[u16]) {
    let total = dst.len();
    for (k, &block_flags) in flags.iter().enumerate() {
        let base = k * BLOCK_SIZE;
        if block_flags & 6 == 0 || base > total || total - base < 64 {
            continue;
        }
        let end = total.min(base + BLOCK_SIZE);
        let chunk = &mut dst[base..end];
        if block_flags & 4 != 0 {
            x86_decode(chunk, base as u64);
        }
        if block_flags & 2 != 0 {
            arm64_decode(chunk, u32::from(block_flags >> 10) & 3, base as u64);
        }
    }
}

/// The filters skip chunks shorter than this.
const BCJ_MIN_CHUNK: usize = 64;

/// Turn the absolute targets of ARM64 `bl` and `adrp` back into relative
/// ones.
fn arm64_decode(buf: &mut [u8], phase: u32, base: u64) {
    if buf.len() < BCJ_MIN_CHUNK {
        return;
    }
    let last = buf.len() - 4;
    let mut off = (phase & 3) as usize;
    while off <= last {
        let w = rd32(buf, off);
        let pos = base + off as u64;
        if w & 0xFC00_0000 == 0x9400_0000 {
            // Only when bits 25, 24 and 23 agree.
            if w.wrapping_add(0x0080_0000) & 0x0300_0000 == 0 {
                let t = w.wrapping_sub((pos >> 2) as u32) & 0x00FF_FFFF;
                let s = (t >> 23) & 1;
                let word = 0x9400_0000 | (s << 25) | (s << 24) | t;
                buf[off..off + 4].copy_from_slice(&word.to_le_bytes());
            }
        } else if w & 0x9F00_0000 == 0x9000_0000 {
            // Only when bits 23 and 22 agree.
            if w.wrapping_add(0x0040_0000) & 0x0080_0000 == 0 {
                let immlo = (w >> 29) & 3;
                let immhi = (w >> 5) & 0x7FFFF;
                let u = (immlo << 18) | (immhi & 0x3FFFF);
                let v = u.wrapping_sub((pos >> 12) as u32) & 0x000F_FFFF;
                let nlo = v & 3;
                let nhi = ((v >> 2) & 0x3FFFF) | (((v >> 19) & 1) << 18);
                let word = (w & 0x9000_001F) | (nlo << 29) | (nhi << 5);
                buf[off..off + 4].copy_from_slice(&word.to_le_bytes());
            }
        }
        off += 4;
    }
}

/// Turn the absolute targets of x86 `call` and `jmp` back into relative
/// ones.
fn x86_decode(buf: &mut [u8], base: u64) {
    if buf.len() < BCJ_MIN_CHUNK {
        return;
    }
    let last = buf.len() - 5;
    let mut p = 0;
    while p <= last {
        if buf[p] & 0xFE != 0xE8 {
            p += 1;
            continue;
        }
        let q = p + 1;
        if buf[q + 3] != 0x00 && buf[q + 3] != 0xFF {
            p += 4;
            continue;
        }
        let pos = (base + q as u64) as u32;
        let a = (u32::from(buf[q]) << 16)
            | (u32::from(buf[q + 1]) << 8)
            | u32::from(buf[q + 2])
            | (u32::from(buf[q + 3] & 1) << 24);
        let mut r = a.wrapping_sub(pos) & 0x01FF_FFFF;
        if r & 0x0100_0000 != 0 {
            r |= 0xFE00_0000;
        }
        buf[q..q + 4].copy_from_slice(&r.to_le_bytes());
        p += 5;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The blocks of a `pbzm` stream, as (declared size, packed data).
    pub(crate) fn pbzm_blocks(data: &[u8]) -> Vec<(usize, &[u8])> {
        let at = data.windows(4).position(|w| w == b"pbzm").unwrap();
        let mut pos = at + 12;
        let mut blocks = Vec::new();
        while pos + 16 <= data.len() {
            let original = u64::from_be_bytes(data[pos..pos + 8].try_into().unwrap()) as usize;
            let packed = u64::from_be_bytes(data[pos + 8..pos + 16].try_into().unwrap()) as usize;
            pos += 16;
            if pos + packed > data.len() {
                break;
            }
            blocks.push((original, &data[pos..pos + packed]));
            pos += packed;
        }
        blocks
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../sweetpad-lib/ci/fixtures/xcode-archives")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn apple_encoded_blocks_decode_to_their_declared_size() {
        for name in ["lzraven.aar", "odc-pbzm.xip", "newc-pbzm.xip"] {
            let data = fixture(name);
            let blocks = pbzm_blocks(&data);
            assert!(!blocks.is_empty(), "{name}");
            for (original, packed) in blocks {
                if packed.len() == original {
                    continue;
                }
                let out = decode(packed, original).unwrap();
                assert_eq!(out.len(), original, "{name}");
            }
        }
    }

    /// Apple's own encoder, in libcompression, or `None` where the host's
    /// libcompression has no LZRAVEN encoder, as on macOS 15.
    #[cfg(target_os = "macos")]
    fn apple_encode(data: &[u8]) -> Option<Vec<u8>> {
        #[link(name = "compression")]
        unsafe extern "C" {
            fn compression_encode_buffer(
                dst: *mut u8,
                dst_size: usize,
                src: *const u8,
                src_size: usize,
                scratch: *mut std::ffi::c_void,
                algorithm: i32,
            ) -> usize;
        }
        let mut out = vec![0u8; data.len() + 65536];
        // SAFETY: both buffers are valid for the sizes given.
        let n = unsafe {
            compression_encode_buffer(
                out.as_mut_ptr(),
                out.len(),
                data.as_ptr(),
                data.len(),
                std::ptr::null_mut(),
                0xD05,
            )
        };
        out.truncate(n);
        (n > 0).then_some(out)
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn decodes_what_apples_encoder_writes() {
        // Nothing to decode against where the host has no encoder.
        if apple_encode(b"a").is_none() {
            return;
        }
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut random = |n: usize| -> Vec<u8> {
            (0..n)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect()
        };
        let text = b"Sweetpad decodes Apple's LZRAVEN without libcompression.\n".repeat(30000);
        let mut mixed = Vec::new();
        for i in 0..2000 {
            mixed.extend(random(i % 97));
            mixed.extend(&text[..(i * 31) % 4096]);
        }
        let mut inputs = vec![
            b"a".to_vec(),
            random(63),
            random(64),
            vec![0u8; 3 << 20],
            text,
            random(700_000),
            mixed,
        ];
        // Executables, which the encoder runs its ARM64 and x86 filters on.
        for binary in ["/bin/ls", "/usr/bin/zip", "/usr/lib/dyld"] {
            if let Ok(data) = std::fs::read(binary) {
                inputs.push(data);
            }
        }
        for data in inputs {
            let packed = apple_encode(&data).expect("Apple's LZRAVEN encoder failed");
            assert!(
                decode(&packed, data.len()).unwrap() == data,
                "{} bytes",
                data.len()
            );
        }
    }

    #[test]
    fn malformed_streams_are_errors() {
        assert_eq!(decode(b"short", 10), Err(Error::Truncated));
        assert_eq!(decode(&[0u8; 12], 10), Err(Error::Magic));
        let mut params = MAGIC.to_le_bytes().to_vec();
        params.extend(1u32.to_le_bytes());
        params.extend(0u32.to_le_bytes());
        assert_eq!(decode(&params, 10), Err(Error::Params));
        // A stored block of 4 bytes, then no terminator.
        let mut stored = MAGIC.to_le_bytes().to_vec();
        stored.extend(0u32.to_le_bytes());
        stored.extend(((4u32 << 12) | 1).to_le_bytes());
        stored.extend(b"abcd");
        assert_eq!(decode(&stored, 10), Err(Error::Truncated));
        stored.extend(0u32.to_le_bytes());
        assert_eq!(decode(&stored, 10).unwrap(), b"abcd");
        assert_eq!(decode(&stored, 3), Err(Error::OutputFull));
        // Every coded block of an Apple stream, cut short, is rejected.
        let data = fixture("lzraven.aar");
        let (original, packed) = pbzm_blocks(&data)
            .into_iter()
            .find(|(original, packed)| packed.len() != *original)
            .unwrap();
        for cut in [12, packed.len() / 2, packed.len() - 5] {
            assert!(decode(&packed[..cut], original).is_err(), "{cut}");
        }
    }

    #[test]
    fn the_filters_undo_what_the_encoder_did() {
        // x86: a call whose absolute target the encoder wrote big-endian.
        let mut x86 = vec![0x90u8; 64];
        x86[10] = 0xE8;
        let target: u32 = 0x0000_0100 + 11; // relative 0x100 from offset 11
        x86[11..14].copy_from_slice(&[(target >> 16) as u8, (target >> 8) as u8, target as u8]);
        x86[14] = 0;
        x86_decode(&mut x86, 0);
        assert_eq!(&x86[11..15], &0x100u32.to_le_bytes());
        // ARM64: `bl` at offset 8 with an absolute target of 0x40 words.
        let mut arm = vec![0u8; 64];
        let absolute = 0x40u32 + (8 >> 2);
        arm[8..12].copy_from_slice(&(0x9400_0000 | absolute).to_le_bytes());
        arm64_decode(&mut arm, 0, 0);
        assert_eq!(rd32(&arm, 8), 0x9400_0040);
    }
}
