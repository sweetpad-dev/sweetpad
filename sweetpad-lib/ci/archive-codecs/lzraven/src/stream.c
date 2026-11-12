/* SPDX-License-Identifier: 0BSD */
/* lzraven — stream framing, the block loop and the content-filter pass
 * (SPEC.md §2, §9).
 *
 * 8-byte stream header, then a chain of blocks each introduced by one u32
 * (payload size in bits [31:12], flags in [11:0]), terminated by an all-zero
 * word.  Every block decodes to exactly 512 KiB except the last.
 *
 * All cursors are size_t.  Apple's driver does this arithmetic in 32 bits,
 * where `payload + csize + 4` can wrap just below 4 GiB and permit a small
 * overread; with size_t that class of bug cannot occur.
 */
#include <stdlib.h>
#include <string.h>

#include "internal.h"
#include "rans.h"

/* ---------------------------------------------------------------------------
 * SIMD dispatch (src/simd.h).
 *
 * Resolved once and cached, because a decode of a 12-byte stream should not
 * pay for a CPUID and a getenv.  The cache is the decoder's only global, it is
 * idempotent — every writer computes the same value from the same inputs — and
 * it is published atomically where C11 atomics exist, so concurrent first calls
 * race benignly rather than formally.
 * ------------------------------------------------------------------------- */
#if defined(__STDC_NO_ATOMICS__) || defined(__STDC_NO_ATOMICS)
#define LZR_CHOICE_T int
#define LZR_CHOICE_LOAD(p) (*(p))
#define LZR_CHOICE_STORE(p, v) (*(p) = (v))
#else
#include <stdatomic.h>
#define LZR_CHOICE_T _Atomic int
#define LZR_CHOICE_LOAD(p) atomic_load_explicit((p), memory_order_relaxed)
#define LZR_CHOICE_STORE(p, v)                                                \
    atomic_store_explicit((p), (v), memory_order_relaxed)
#endif

/* -1 = not yet resolved. */
static LZR_CHOICE_T lzr_choice = -1;

/* Map a name to a kind.  Returns -1 for "auto" and for anything unrecognised,
 * which both mean "work it out from the CPU". */
static int lzr_simd_by_name(const char *name)
{
    static const lzr_simd_kind all[] = { LZR_SIMD_SCALAR, LZR_SIMD_SSE2,
                                         LZR_SIMD_AVX2, LZR_SIMD_NEON };
    size_t i;

    if (!name || !*name)
        return -1;
    for (i = 0; i < sizeof all / sizeof *all; i++)
        if (!strcmp(name, lzr_simd_kind_name(all[i])))
            return (int)all[i];
    return -1;
}

lzr_simd_kind lzr_simd_active(void)
{
    int c = LZR_CHOICE_LOAD(&lzr_choice);

    if (c < 0) {
        /* LZRAVEN_SIMD forces a path, for benchmarking and for running the
         * whole test suite once per path on one machine.  An unavailable or
         * unrecognised value falls back to the best one this CPU can run. */
        c = lzr_simd_by_name(getenv("LZRAVEN_SIMD"));
        if (c < 0 || !lzr_simd_usable((lzr_simd_kind)c))
            c = (int)lzr_simd_best();
        LZR_CHOICE_STORE(&lzr_choice, c);
    }
    return (lzr_simd_kind)c;
}

const char *lzraven_simd_path(void)
{
    return lzr_simd_kind_name(lzr_simd_active());
}

int lzraven_simd_set(const char *name)
{
    int c;

    if (name && !strcmp(name, "auto")) {
        LZR_CHOICE_STORE(&lzr_choice, -1);
        (void)lzr_simd_active();
        return 1;
    }
    c = lzr_simd_by_name(name);
    if (c < 0 || !lzr_simd_usable((lzr_simd_kind)c))
        return 0;
    LZR_CHOICE_STORE(&lzr_choice, c);
    return 1;
}

/* ---------------------------------------------------------------------------
 * BCJ filters.  src/bcj.c owns these; this file only calls them.
 *
 * Both transforms are position-dependent on the *absolute* output offset
 * (ARM64: `w - (off >> 2)` and `U - (off >> 12)`; x86: `a - q`), where the
 * offset is into the whole output buffer and not into the 512 KiB chunk.  So
 * the chunk base has to be passed in: the `_at` entry points take it, and the
 * plain ones are the base == 0 case.  Measured, not assumed — a 1 MiB
 * plaintext whose second chunk is ARM64 code gets flags 0x002 on chunk 1, and
 * only the absolute-offset inverse reproduces it.
 * ------------------------------------------------------------------------- */
#include <lzraven_bcj.h>

/* Undo the per-block executable prefilters, in 512 KiB chunks.  The encoder
 * applies ARM64 first then x86, so the inverse runs x86 first.  A chunk with
 * fewer than 64 bytes left in it is skipped entirely.
 *
 * Block k starts at k * 512 KiB because of the lockstep rule, so the block
 * ordinal is also the chunk index.  The one corner where they part company is a
 * final *stored* block longer than 512 KiB — the header's 20-bit size field
 * allows it, no encoder emits it, and its second chunk simply goes unfiltered
 * here rather than reading a flag word nobody wrote. */
static void lzr_apply_filters(uint8_t *dst, size_t total,
                              const uint16_t *flags, size_t nblocks)
{
    size_t k;

    for (k = 0; k < nblocks; k++) {
        size_t base = (size_t)k * LZRAVEN_BLOCK_SIZE;
        size_t end;
        unsigned fl = flags[k];

        if (!(fl & 6))
            continue;
        if (base > total || total - base < 64)
            continue;
        end = (total - base < LZRAVEN_BLOCK_SIZE) ? total
                                                  : base + LZRAVEN_BLOCK_SIZE;
        if (fl & 4)
            lzraven_bcj_x86_decode_at(dst + base, end - base, base);
        if (fl & 2)
            lzraven_bcj_arm64_decode_at(dst + base, end - base,
                                        (fl >> 10) & 3, base);
    }
}

size_t lzraven_decode_buffer_ex(uint8_t *dst, size_t dst_capacity,
                                const uint8_t *src, size_t src_size,
                                lzraven_status *status)
{
    /* ~35 KB of state, matching Apple's scratch: the persistent model plus one
     * flag word per 512 KiB of output.  Keeping it on the stack is what lets
     * the decoder run without allocating. */
    lzr_model model;
    uint16_t block_flags[LZRAVEN_MAX_BLOCKS];
    lzraven_status st = LZRAVEN_OK;
    size_t out = 0, hdr = 8, nblocks = 0;

    if (status)
        *status = LZRAVEN_OK;
    if (!src || (!dst && dst_capacity)) {
        st = LZRAVEN_E_ARGS;
        goto fail;
    }
    /* The shortest legal stream is the 8-byte header plus a terminator. */
    if (src_size < 12) {
        st = LZRAVEN_E_TRUNCATED;
        goto fail;
    }
    if (lzr_rd32(src) != LZRAVEN_MAGIC) {
        st = LZRAVEN_E_MAGIC;
        goto fail;
    }
    /* params: bits [25:0] must be zero.  Bits [31:26] are a quantized
     * dictionary size that the decoder must *not* use — there is no
     * decoder-side window, matches reference the output buffer directly. */
    if (lzr_rd32(src + 4) & 0x03FFFFFFu) {
        st = LZRAVEN_E_PARAMS;
        goto fail;
    }

    lzr_model_init(&model);
    memset(block_flags, 0, sizeof block_flags);

    for (;;) {
        uint32_t h;
        size_t csize, pay, n;
        unsigned flags;

        /* In bounds by induction: src_size >= 12 covers the first header, and
         * the tail of the loop checks room for every subsequent one. */
        h = lzr_rd32(src + hdr);
        if (h < 0x1000)
            break; /* all-zero terminator (any header with size 0) */

        csize = (size_t)(h >> 12);
        flags = (unsigned)(h & 0xFFF);
        pay = hdr + 4;

        if (out >= dst_capacity) {
            st = LZRAVEN_E_OUTPUT_FULL;
            goto fail;
        }
        /* 512 KiB lockstep: only the final block may be short. */
        if (out % LZRAVEN_BLOCK_SIZE != 0) {
            st = LZRAVEN_E_FRAMING;
            goto fail;
        }
        if (csize > src_size - pay) {
            st = LZRAVEN_E_TRUNCATED;
            goto fail;
        }
        if (nblocks >= LZRAVEN_MAX_BLOCKS) {
            st = LZRAVEN_E_TOO_MANY_BLOCKS;
            goto fail;
        }
        block_flags[nblocks++] = (uint16_t)flags;

        if (flags & 1) { /* stored: bypasses the entropy decoder entirely */
            if (csize > dst_capacity - out) {
                st = LZRAVEN_E_OUTPUT_FULL;
                goto fail;
            }
            if (csize)
                memcpy(dst + out, src + pay, csize);
            n = csize;
        } else {
            if (!lzr_decode_block(&model, src + pay, csize, dst, dst_capacity,
                                  out, &n, &st))
                goto fail;
        }

        out += n;
        hdr = pay + csize;
        /* The terminator is mandatory: a stream that ends after the last
         * payload is rejected. */
        if (hdr > src_size || src_size - hdr < 4) {
            st = LZRAVEN_E_TRUNCATED;
            goto fail;
        }
    }

    /* Bytes after the terminator are never examined, matching Apple. */
    if (out)
        lzr_apply_filters(dst, out, block_flags, nblocks);
    return out;

fail:
    if (status)
        *status = st;
    return 0;
}

size_t lzraven_decode_buffer(uint8_t *dst, size_t dst_capacity,
                             const uint8_t *src, size_t src_size)
{
    return lzraven_decode_buffer_ex(dst, dst_capacity, src, src_size, NULL);
}

const char *lzraven_status_string(lzraven_status status)
{
    switch (status) {
    case LZRAVEN_OK:                return "ok";
    case LZRAVEN_E_ARGS:            return "invalid arguments";
    case LZRAVEN_E_MAGIC:           return "bad stream magic";
    case LZRAVEN_E_PARAMS:          return "reserved params bits set";
    case LZRAVEN_E_TRUNCATED:       return "truncated stream";
    case LZRAVEN_E_FRAMING:         return "bad block framing";
    case LZRAVEN_E_OUTPUT_FULL:     return "output buffer too small";
    case LZRAVEN_E_TOO_MANY_BLOCKS: return "stream exceeds the block limit";
    case LZRAVEN_E_DISTANCE:        return "match distance out of range";
    case LZRAVEN_E_CORRUPT:         return "corrupt entropy-coded block";
    }
    return "unknown status";
}
