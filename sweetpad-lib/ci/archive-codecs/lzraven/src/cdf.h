/* SPDX-License-Identifier: 0BSD */
/* lzraven — the CDF kernel: one 16-ary symbol's search and adaptation, and the
 * overlapping match copy.  SPEC.md §4.2, §4.3.
 *
 * THIS HEADER IS INCLUDED MORE THAN ONCE, on purpose.  block.c instantiates the
 * whole grammar once per dispatch path; each instantiation defines LZR_KERNEL
 * and renames these functions (see block_body.h) so the kernel inlines straight
 * into the decode loop rather than being reached through a pointer.  There is
 * therefore deliberately no include guard.
 *
 * Every kernel must be *bit-identical* to the scalar one, which is the
 * reference: `LZRAVEN_SIMD=scalar` is the differential baseline and the thing
 * every other path is diffed against.  Two properties make that easy to hold:
 *
 *   - The search is defined as a population count ("how many of the 16 lanes
 *     are greater than x"), never as "find the first lane".  Those agree only
 *     while the CDF is monotone; counting agrees unconditionally, so no kernel
 *     depends on an invariant the reference does not.
 *   - Every lane value lives in [32, 16384] and x in [0, 16383], so a 16-bit
 *     compare is exact whether the ISA offers a signed one (x86 `pcmpgtw`) or
 *     an unsigned one (NEON `cmhi`), and the adaptation delta
 *     `target - cdf[j]` lies in [-16352, 16479] — inside int16 — so the
 *     arithmetic shift and the wrapping add can both be done at 16 bits.
 */

/* The one include this header makes itself: <assert.h> is defined to be
 * includable any number of times, so it survives the guard-less multiple
 * inclusion, and the copy-contract assert below needs it in every consumer. */
#include <assert.h>

/* ---------------------------------------------------------------------------
 * The scalar reference.
 * ------------------------------------------------------------------------- */
#if LZR_KERNEL == LZR_K_SCALAR

/* Locate x in `cdf`, publish the symbol's (cum, freq) pair, then adapt all 16
 * lanes.  Returns the symbol, 0..15.
 *
 * cdf[15] == 16384 is a hard invariant (rans.h), so at least one lane always
 * compares greater and the result is in range; `cdf[(s + 15) & 15] & 0x3FFF`
 * is the exclusive prefix even at s == 0, where it wraps to cdf[15] == 0. */
LZR_INLINE unsigned lzr_cdf_step(uint16_t *cdf, uint32_t x, unsigned rate,
                                 uint32_t *cum, uint32_t *freq)
{
    const uint16_t *hi_t = lzr_hi_target[rate - 6];
    unsigned j, gt = 0, s;
    uint32_t lo;

    for (j = 0; j < 16; j++)
        gt += (cdf[j] > x);
    s = 16u - gt;

    lo = (uint32_t)(cdf[(s + 15) & 15] & 0x3FFF);
    *cum = lo;
    *freq = (uint32_t)(uint16_t)(cdf[s] - lo);

    for (j = 0; j < 16; j++) {
        int t = (cdf[j] > x) ? (int)hi_t[j] : (int)lzr_lo_target[j];
        int d = t - (int)cdf[j];
        /* d is in [-16352, 16479]; the shift is the arithmetic one (gcc/clang
         * define it, and it is what Apple's `ssra` does). */
        cdf[j] = (uint16_t)((int)cdf[j] + (d >> rate));
    }
    return s;
}

/* The overlapping match copy: `d[i] = s[i]` for i in [0, len) where s is
 * `dist` bytes behind d and dist < len, so each byte may depend on one written
 * earlier in this same call.  The reference does it one byte at a time and
 * nothing else may.
 *
 * `avail` is the caller's promise: that many bytes at `d` are writable.  The
 * scalar copy writes exactly `len` of them; the vector kernels overrun into
 * the rest (see the shared vector version below), so the contract is part of
 * the signature and asserted in every instantiation.
 *
 * `s` is passed in rather than derived as `d - dist` inside the loop: pointer
 * arithmetic has to stay forward, or `d + (n - dist)` is arithmetic on a
 * wrapped size_t and undefined even where the address lands in range. */
LZR_INLINE void lzr_copy_overlap(uint8_t *d, const uint8_t *s, size_t len,
                                 size_t dist, size_t avail)
{
    size_t n;

    (void)dist;
    (void)avail;
    assert(len <= avail);
    for (n = 0; n < len; n++)
        d[n] = s[n];
}

#endif /* LZR_K_SCALAR */

/* ---------------------------------------------------------------------------
 * SSE2 — the x86-64 baseline.  Part of the ABI, so no feature test.
 * 16 u16 lanes need two xmm registers.
 * ------------------------------------------------------------------------- */
#if LZR_KERNEL == LZR_K_SSE2

#define LZR_VEC 16u /* bytes per store in the match copy */

/* How many of the 16 lanes compared greater, counted in the vector unit.
 *
 * SSE2 predates POPCNT by a decade and this path is the one a pre-AVX2 CPU
 * gets, so `movemask` + a scalar population count is not available cheaply.
 * Instead: `packs` saturates each 0xFFFF lane to a 0xFF byte, negating turns
 * those into 1 and leaves 0 alone, and `psadbw` against zero sums the bytes of
 * each half — a horizontal count in three instructions with no table, no
 * multiply and no assumption that the mask is contiguous. */
LZR_INLINE unsigned lzr_count_gt(__m128i g0, __m128i g1)
{
    const __m128i zero = _mm_setzero_si128();
    __m128i b = _mm_sub_epi8(zero, _mm_packs_epi16(g0, g1));
    __m128i n = _mm_sad_epu8(b, zero);

    n = _mm_add_epi32(n, _mm_srli_si128(n, 8));
    return (unsigned)_mm_cvtsi128_si32(n);
}

LZR_INLINE unsigned lzr_cdf_step(uint16_t *cdf, uint32_t x, unsigned rate,
                                 uint32_t *cum, uint32_t *freq)
{
    const uint16_t *hi_t = lzr_hi_target[rate - 6];
    const __m128i xv = _mm_set1_epi16((short)x);
    __m128i c0 = _mm_loadu_si128((const __m128i *)(const void *)cdf);
    __m128i c1 = _mm_loadu_si128((const __m128i *)(const void *)(cdf + 8));
    __m128i g0 = _mm_cmpgt_epi16(c0, xv);
    __m128i g1 = _mm_cmpgt_epi16(c1, xv);
    __m128i l0, l1, h0, h1, t0, t1;
    unsigned s;
    uint32_t lo;

    s = 16u - lzr_count_gt(g0, g1);

    lo = (uint32_t)(cdf[(s + 15) & 15] & 0x3FFF);
    *cum = lo;
    *freq = (uint32_t)(uint16_t)(cdf[s] - lo);

    /* Select each lane's target from the same two tables the reference reads —
     * no closed form, no assumption about their contents. */
    l0 = _mm_loadu_si128((const __m128i *)(const void *)lzr_lo_target);
    l1 = _mm_loadu_si128((const __m128i *)(const void *)(lzr_lo_target + 8));
    h0 = _mm_loadu_si128((const __m128i *)(const void *)hi_t);
    h1 = _mm_loadu_si128((const __m128i *)(const void *)(hi_t + 8));
    t0 = _mm_or_si128(_mm_and_si128(g0, h0), _mm_andnot_si128(g0, l0));
    t1 = _mm_or_si128(_mm_and_si128(g1, h1), _mm_andnot_si128(g1, l1));
    c0 = _mm_add_epi16(c0, _mm_srai_epi16(_mm_sub_epi16(t0, c0), (int)rate));
    c1 = _mm_add_epi16(c1, _mm_srai_epi16(_mm_sub_epi16(t1, c1), (int)rate));
    _mm_storeu_si128((__m128i *)(void *)cdf, c0);
    _mm_storeu_si128((__m128i *)(void *)(cdf + 8), c1);
    return s;
}

LZR_INLINE void lzr_vec_copy(uint8_t *d, const uint8_t *s, size_t len)
{
    size_t n;

    for (n = 0; n < len; n += LZR_VEC)
        _mm_storeu_si128((__m128i *)(void *)(d + n),
                         _mm_loadu_si128((const __m128i *)(const void *)
                                         (s + n)));
}

#endif /* LZR_K_SSE2 */

/* ---------------------------------------------------------------------------
 * AVX2 — all 16 lanes in one ymm register.  Opt-in at run time.
 * ------------------------------------------------------------------------- */
#if LZR_KERNEL == LZR_K_AVX2

#define LZR_VEC 32u

LZR_INLINE unsigned lzr_cdf_step(uint16_t *cdf, uint32_t x, unsigned rate,
                                 uint32_t *cum, uint32_t *freq)
{
    const uint16_t *hi_t = lzr_hi_target[rate - 6];
    __m256i c = _mm256_loadu_si256((const __m256i *)(const void *)cdf);
    __m256i g = _mm256_cmpgt_epi16(c, _mm256_set1_epi16((short)x));
    __m256i t;
    unsigned mask, s;
    uint32_t lo;

    /* movemask gives two bits per u16 lane; both are equal, so halving the
     * population count is the lane count.  Same definition as the reference. */
    mask = (unsigned)_mm256_movemask_epi8(g);
    s = 16u - ((unsigned)__builtin_popcount(mask) >> 1);

    lo = (uint32_t)(cdf[(s + 15) & 15] & 0x3FFF);
    *cum = lo;
    *freq = (uint32_t)(uint16_t)(cdf[s] - lo);

    t = _mm256_blendv_epi8(
        _mm256_loadu_si256((const __m256i *)(const void *)lzr_lo_target),
        _mm256_loadu_si256((const __m256i *)(const void *)hi_t), g);
    c = _mm256_add_epi16(c, _mm256_srai_epi16(_mm256_sub_epi16(t, c),
                                              (int)rate));
    _mm256_storeu_si256((__m256i *)(void *)cdf, c);
    return s;
}

LZR_INLINE void lzr_vec_copy(uint8_t *d, const uint8_t *s, size_t len)
{
    size_t n;

    for (n = 0; n < len; n += LZR_VEC)
        _mm256_storeu_si256((__m256i *)(void *)(d + n),
                            _mm256_loadu_si256((const __m256i *)(const void *)
                                               (s + n)));
}

#endif /* LZR_K_AVX2 */

/* ---------------------------------------------------------------------------
 * NEON — mandatory on aarch64, so it needs no feature test either.
 * ------------------------------------------------------------------------- */
#if LZR_KERNEL == LZR_K_NEON

#define LZR_VEC 16u

LZR_INLINE unsigned lzr_cdf_step(uint16_t *cdf, uint32_t x, unsigned rate,
                                 uint32_t *cum, uint32_t *freq)
{
    const uint16_t *hi_t = lzr_hi_target[rate - 6];
    const uint16x8_t xv = vdupq_n_u16((uint16_t)x);
    uint16x8_t c0 = vld1q_u16(cdf);
    uint16x8_t c1 = vld1q_u16(cdf + 8);
    uint16x8_t g0 = vcgtq_u16(c0, xv);
    uint16x8_t g1 = vcgtq_u16(c1, xv);
    uint16x8_t t0, t1;
    int16x8_t d0, d1;
    unsigned s;
    uint32_t lo;

    /* uzp1 takes the low byte of each u16 lane — 0x00 or 0xFF — into one 16-byte
     * vector, and the signed add-across then sums to -(number of lanes greater).
     * Population count again, not a first-set-lane search. */
    s = (unsigned)(16 + (int)vaddlvq_s8(vreinterpretq_s8_u8(
            vuzp1q_u8(vreinterpretq_u8_u16(g0), vreinterpretq_u8_u16(g1)))));

    lo = (uint32_t)(cdf[(s + 15) & 15] & 0x3FFF);
    *cum = lo;
    *freq = (uint32_t)(uint16_t)(cdf[s] - lo);

    t0 = vbslq_u16(g0, vld1q_u16(hi_t), vld1q_u16(lzr_lo_target));
    t1 = vbslq_u16(g1, vld1q_u16(hi_t + 8), vld1q_u16(lzr_lo_target + 8));
    d0 = vreinterpretq_s16_u16(vsubq_u16(t0, c0));
    d1 = vreinterpretq_s16_u16(vsubq_u16(t1, c1));
    /* `rate` is a literal at every call site, so only one arm survives; ssra
     * needs the shift as an immediate. */
    if (rate == 6) {
        c0 = vreinterpretq_u16_s16(vsraq_n_s16(vreinterpretq_s16_u16(c0), d0, 6));
        c1 = vreinterpretq_u16_s16(vsraq_n_s16(vreinterpretq_s16_u16(c1), d1, 6));
    } else {
        c0 = vreinterpretq_u16_s16(vsraq_n_s16(vreinterpretq_s16_u16(c0), d0, 7));
        c1 = vreinterpretq_u16_s16(vsraq_n_s16(vreinterpretq_s16_u16(c1), d1, 7));
    }
    vst1q_u16(cdf, c0);
    vst1q_u16(cdf + 8, c1);
    return s;
}

LZR_INLINE void lzr_vec_copy(uint8_t *d, const uint8_t *s, size_t len)
{
    size_t n;

    for (n = 0; n < len; n += LZR_VEC)
        vst1q_u8(d + n, vld1q_u8(s + n));
}

#endif /* LZR_K_NEON */

/* ---------------------------------------------------------------------------
 * The vectorised overlapping match copy, shared by every vector kernel.
 *
 * `d[i] = d[i - dist]` with dist < len is a self-referential fill, so a naive
 * vector loop would read bytes it has not written yet.  The standard fix — and
 * the one Apple's decoder uses, which is what its `repeat_perm` (`[d][i] ==
 * i % d`) and `rep_distance` ("least multiple of d that is >= 17") tables are
 * for (docs/notes/tables.md §6) — is to materialise one full vector's worth of
 * the repeating pattern first, after which the *same bytes* can be reached at a
 * distance that is both a multiple of `dist` and at least a vector wide.  From
 * there the rest is a plain non-overlapping vector copy.
 *
 * Apple builds that first vector with a single TBL through `repeat_perm`; SSE2
 * has no byte permute at all, so we seed it with a short scalar run instead —
 * at most LZR_VEC + dist - 1 bytes, against a mean overlapping match of ~324.
 *
 * Overrun: the loop stores in whole vectors, so it may write up to LZR_VEC - 1
 * bytes past `len`.  That is in bounds and harmless — but only because of two
 * checks that live elsewhere, which is what `avail` is for.  `avail` is the
 * caller's promise of how many bytes at `d` are writable: from the copy
 * position to the end of the block, every byte of which is written before it
 * is read (the coded region by the grammar, the last LZR_TAIL bytes by the
 * uncoded tail).  The promise rests on block_body.h rejecting any match with
 * `length > coded_end - pos` and any block with `size > dst_capacity -
 * dst_pos`, which together leave at least LZR_TAIL == 32 > LZR_VEC - 1 bytes
 * of block past `len`.  Asserted on entry, so an edit that weakens either
 * check dies in a debug build here instead of writing out of bounds.  It never
 * reads past what it has written, because `back >= LZR_VEC` on every vector
 * load.
 * ------------------------------------------------------------------------- */
#if LZR_KERNEL != LZR_K_SCALAR

LZR_INLINE void lzr_copy_overlap(uint8_t *d, const uint8_t *s, size_t len,
                                 size_t dist, size_t avail)
{
    size_t n, back;

    (void)avail;
    assert(len <= avail && avail - len >= LZR_VEC - 1);
    if (dist == 1) { /* a run of one byte: 62% of overlapping matches */
        memset(d, s[0], len);
        return;
    }
    if (dist >= LZR_VEC) { /* already wide enough to read behind the writes */
        lzr_vec_copy(d, s, len);
        return;
    }
    back = dist;
    while (back < LZR_VEC) /* least multiple of dist that is >= LZR_VEC */
        back += dist;
    if (back >= len) { /* too short to be worth widening */
        for (n = 0; n < len; n++)
            d[n] = s[n];
        return;
    }
    /* Seed one full period-aligned run, then read it back from `d` itself at a
     * distance that is both a multiple of dist and at least a vector wide. */
    for (n = 0; n < back; n++)
        d[n] = s[n];
    lzr_vec_copy(d + back, d, len - back);
}

#endif /* !LZR_K_SCALAR */
