/* SPDX-License-Identifier: 0BSD */
/* lzraven — the entropy coder: 8-way interleaved adaptive rANS driven by
 * AV1/Daala-style adaptive 16-symbol CDFs.  SPEC.md §3.
 *
 * M = 2^14, 64-bit states, 32-bit renormalisation, one forward u32 stream per
 * block.  Decode step: x' = f * (x >> 14) + (x & 0x3FFF) - cum.
 *
 * The primitives live in the header so the grammar in block.c can inline them;
 * they are the hot loop of the whole decoder.
 *
 * Two halves, two include guards.  Everything up to `lzr_rans_init` is shared
 * and included once.  `lzr_sym` — the 16-ary symbol, which is where the decoder
 * spends two thirds of its time — is templated on the CDF kernel (src/cdf.h,
 * scalar or one of the vector ISAs) and is compiled once per dispatch path, so
 * the bottom of this file is deliberately outside the guard.
 */
#ifndef LZRAVEN_RANS_H
#define LZRAVEN_RANS_H

#include <stddef.h>
#include <stdint.h>
#include <string.h>

#define LZR_M 16384u /* probability total, 2^14 */

typedef struct lzr_rans {
    uint64_t st[8];      /* the eight interleaved states */
    const uint8_t *w;    /* payload, read as little-endian u32 words */
    size_t wpos, wcount; /* word cursor / count (payload truncated to words) */
    uint32_t next;       /* round-robin counter; slot = next & 7 */
    int err;             /* sticky: the word stream ran out */
} lzr_rans;

/* Adaptation targets (SPEC.md §3.1): a symbol's lanes are nudged toward
 * 32*(j+1) below the symbol and toward 16384 - 32*(15-j) above it, the latter
 * biased by 2^rate - 1 so a truncating arithmetic shift converges exactly.
 * Index [rate - 6]. */
static const uint16_t lzr_lo_target[16] = {
    32,  64,  96,  128, 160, 192, 224, 256,
    288, 320, 352, 384, 416, 448, 480, 512
};
static const uint16_t lzr_hi_target[2][16] = {
    { 15967, 15999, 16031, 16063, 16095, 16127, 16159, 16191,
      16223, 16255, 16287, 16319, 16351, 16383, 16415, 16447 }, /* rate 6 */
    { 16031, 16063, 16095, 16127, 16159, 16191, 16223, 16255,
      16287, 16319, 16351, 16383, 16415, 16447, 16479, 16511 }  /* rate 7 */
};

static inline uint32_t lzr_rd32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static inline void lzr_wr32(uint8_t *p, uint32_t v)
{
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
    p[2] = (uint8_t)(v >> 16);
    p[3] = (uint8_t)(v >> 24);
}

/* Lazy refill, at the top of every read.  One conditional refill is provably
 * enough: a modelled symbol removes at most log2(M/f_min) = 9 bits and a bypass
 * field at most 32, so x never reaches 0 and (x << 32) is then >= 2^32. */
static inline void lzr_renorm(lzr_rans *r, unsigned i)
{
    if ((r->st[i] >> 32) == 0) {
        if (r->wpos >= r->wcount) {
            r->err = 1;
            return; /* state left as-is; all downstream reads stay in range */
        }
        r->st[i] = (r->st[i] << 32) | lzr_rd32(r->w + 4 * r->wpos);
        r->wpos++;
    }
}

/* n uniform bits out of a named state (used only by the block prologue, which
 * addresses states 0 and 1 directly rather than through the round-robin). */
static inline uint64_t lzr_bypass_at(lzr_rans *r, unsigned i, unsigned n)
{
    uint64_t v;
    lzr_renorm(r, i);
    v = r->st[i] & (((uint64_t)1 << n) - 1); /* n <= 32 */
    r->st[i] >>= n;
    return v;
}

/* n uniform bits from the next state in the round-robin. */
static inline uint32_t lzr_bypass(lzr_rans *r, unsigned n)
{
    unsigned i = r->next++ & 7;
    return (uint32_t)lzr_bypass_at(r, i, n);
}

/* A binary symbol's (cum, freq) pair and its adaptation, given the symbol.
 * probs[k] is p(symbol 0), scaled to M; the update is LZMA's, with
 * kNumMoveBits = 5, at 14-bit precision.
 *
 * Split out from lzr_bit because the encoder (src/encode.c) needs the identical
 * model evolution but supplies the symbol instead of reading it — and a second
 * copy of the update that differed in one shift would desynchronise the two on
 * the first adaptive symbol. */
static inline void lzr_bit_apply(uint16_t *probs, unsigned k, unsigned b,
                                 uint32_t *cum, uint32_t *freq)
{
    uint32_t p = probs[k];

    if (b == 0) {
        *cum = 0;
        *freq = p;
        probs[k] = (uint16_t)(p + ((LZR_M - p) >> 5));
    } else {
        *cum = p;
        *freq = LZR_M - p;
        probs[k] = (uint16_t)(p - (p >> 5));
    }
}

/* One adaptive binary symbol. */
static inline unsigned lzr_bit(lzr_rans *r, uint16_t *probs, unsigned k)
{
    unsigned i = r->next++ & 7;
    uint64_t st;
    uint32_t x, f, cum;
    unsigned b;

    lzr_renorm(r, i);
    st = r->st[i];
    x = (uint32_t)(st & 0x3FFF);
    b = (x < probs[k]) ? 0u : 1u;
    lzr_bit_apply(probs, k, b, &cum, &f);
    r->st[i] = (uint64_t)f * (st >> 14) + x - cum;
    return b;
}

/* Seed the eight states from the head of a block payload (SPEC.md §3.3 /
 * entropy-coder.md §7).  Returns 0 if the payload is too short to seed. */
static inline int lzr_rans_init(lzr_rans *r, const uint8_t *payload, size_t len)
{
    unsigned i;

    r->wcount = len / 4; /* a trailing partial word is never read */
    if (r->wcount < 2)
        return 0;
    r->w = payload;
    r->wpos = 2;
    r->next = 0;
    r->err = 0;
    memset(r->st, 0, sizeof r->st);
    r->st[0] = ((uint64_t)lzr_rd32(payload) << 32) | lzr_rd32(payload + 4);

    /* States 1..7 are flushed as a raw 32-bit low half plus a (5-bit exponent,
     * mantissa) code for the high half, all read back out of state 0. */
    for (i = 1; i < 8; i++) {
        uint64_t lo = lzr_bypass_at(r, 0, 32);
        unsigned k = (unsigned)lzr_bypass_at(r, 0, 5);
        uint64_t hi = ((uint64_t)1 << k) + (k ? lzr_bypass_at(r, 0, k) : 0);
        r->st[i] = lo | (hi << 32);
    }
    return !r->err;
}

#endif /* LZRAVEN_RANS_H — end of the shared half */

/* ---------------------------------------------------------------------------
 * One adaptive 16-ary symbol from an ascending 16-entry CDF — the hot loop.
 *
 * Outside the guard: block.c includes this file again for each dispatch path,
 * with LZR_KERNEL set and the names below redirected (block_body.h), so the
 * chosen CDF kernel inlines into the grammar instead of being called through a
 * pointer.  Only the CDF search and update differ between paths; the rANS
 * arithmetic around them is written once, here.
 *
 * cdf[15] == 16384 is a hard invariant: it holds at init and is a fixed point
 * of the update (the rate bias is exactly 2^rate - 1, so the last lane never
 * moves), which is what guarantees the search returns 0..15 and that
 * cdf[(s-1) & 15] & 0x3FFF is the exclusive prefix even at s == 0.
 * ------------------------------------------------------------------------- */
#ifdef LZR_KERNEL

#include "cdf.h"

LZR_INLINE unsigned lzr_sym(lzr_rans *r, uint16_t *cdf, unsigned rate)
{
    unsigned i = r->next++ & 7;
    uint64_t st;
    uint32_t x, lo = 0, f = 0;
    unsigned s;

    lzr_renorm(r, i);
    st = r->st[i];
    x = (uint32_t)(st & 0x3FFF);

    s = lzr_cdf_step(cdf, x, rate, &lo, &f);

    r->st[i] = (uint64_t)f * (st >> 14) + x - lo;
    return s;
}

#endif /* LZR_KERNEL */
