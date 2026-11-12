/* SPDX-License-Identifier: 0BSD */
/* lzraven — the symbol grammar and the per-block decode loop (SPEC.md §4).
 *
 * An LZMA-family grammar: a 14-state machine selects contexts, one binary
 * symbol chooses literal vs command, commands 0..6 are matches at a fresh
 * distance (the command *is* the distance class), 7 is a short rep, and 8..15
 * are four repeat slots crossed with two length classes.  Lengths come from a
 * shared ladder with two instances (base 2 for matches, base 18 for reps).
 *
 * Ported case-by-case from tests/validate_symbols.py, the reference decoder
 * that is byte-exact on 33 vectors and 94 MB of real OTA payload.
 *
 * The loop itself lives in src/block_body.h, because it is compiled more than
 * once: the CDF search and update are two thirds of the decoder's time and are
 * hand-vectorised per ISA (src/cdf.h), and a kernel is only worth having if it
 * inlines into the loop.  So this file instantiates the same grammar once per
 * dispatch path and picks between them once per block.  The scalar
 * instantiation is the reference every other path is diffed against, and stays
 * reachable everywhere via LZRAVEN_SIMD=scalar.
 */
#include "grammar.h"
#include "internal.h"
#include "rans.h"
#include "simd.h"

#if LZR_HAVE_SSE2
#include <immintrin.h>
#endif
#if LZR_HAVE_NEON
#include <arm_neon.h>
#endif

/* The state machine, the literal-context table and LZR_TAIL / LZR_LADDER_MAX are
 * in src/grammar.h: the encoder has to make exactly these transitions, so there
 * is one copy of them rather than one per side. */

/* ---------------------------------------------------------------------------
 * The instantiations.  Each one pulls in src/cdf.h for its kernel, then the
 * grammar in src/block_body.h around it.
 * ------------------------------------------------------------------------- */

#define LZR_KERNEL LZR_K_SCALAR
#define LZR_TAG _scalar
#define LZR_ENTRY lzr_decode_block_scalar
#include "block_body.h"

#if LZR_HAVE_SSE2
#define LZR_KERNEL LZR_K_SSE2
#define LZR_TAG _sse2
#define LZR_ENTRY lzr_decode_block_sse2
#include "block_body.h"
#endif

#if LZR_HAVE_AVX2
/* Compiled for AVX2 whether or not this CPU has it; the dispatcher below only
 * enters it after CPUID (and XCR0) say it is safe, so no build ever *requires*
 * AVX2 to run. */
LZR_TARGET_AVX2_PUSH
#define LZR_KERNEL LZR_K_AVX2
#define LZR_TAG _avx2
#define LZR_ENTRY lzr_decode_block_avx2
#include "block_body.h"
LZR_TARGET_POP
#endif

#if LZR_HAVE_NEON
#define LZR_KERNEL LZR_K_NEON
#define LZR_TAG _neon
#define LZR_ENTRY lzr_decode_block_neon
#include "block_body.h"
#endif

/* ---------------------------------------------------------------------------
 * Dispatch.  Once per block — a 512 KiB unit of work — so the branch is free.
 * ------------------------------------------------------------------------- */
int lzr_decode_block(lzr_model *m, const uint8_t *payload, size_t csize,
                     uint8_t *dst, size_t dst_capacity, size_t dst_pos,
                     size_t *out_len, lzraven_status *status)
{
    switch (lzr_simd_active()) {
#if LZR_HAVE_SSE2
    case LZR_SIMD_SSE2:
        return lzr_decode_block_sse2(m, payload, csize, dst, dst_capacity,
                                     dst_pos, out_len, status);
#endif
#if LZR_HAVE_AVX2
    case LZR_SIMD_AVX2:
        return lzr_decode_block_avx2(m, payload, csize, dst, dst_capacity,
                                     dst_pos, out_len, status);
#endif
#if LZR_HAVE_NEON
    case LZR_SIMD_NEON:
        return lzr_decode_block_neon(m, payload, csize, dst, dst_capacity,
                                     dst_pos, out_len, status);
#endif
    default:
        break;
    }
    return lzr_decode_block_scalar(m, payload, csize, dst, dst_capacity,
                                   dst_pos, out_len, status);
}
