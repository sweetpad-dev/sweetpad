/* SPDX-License-Identifier: 0BSD */
/* lzraven — interfaces shared between the block decoder and the stream driver. */
#ifndef LZRAVEN_INTERNAL_H
#define LZRAVEN_INTERNAL_H

#include <stddef.h>
#include <stdint.h>

#include "lzraven.h"
#include "model.h"
#include "simd.h"

/* Decode one entropy-coded block.
 *
 * `dst_pos` is where this block's output starts in `dst`; the LZ window is the
 * whole output so far, not the block, so matches may reach into earlier blocks.
 * On success writes the block's length to *out_len and returns 1; on failure
 * sets *status and returns 0. Never writes outside [dst, dst + dst_capacity),
 * and never reads output it has not itself written. */
int lzr_decode_block(lzr_model *m, const uint8_t *payload, size_t csize,
                     uint8_t *dst, size_t dst_capacity, size_t dst_pos,
                     size_t *out_len, lzraven_status *status);

/* The per-dispatch-path instantiations of the above (src/block_body.h).  They
 * are byte-for-byte equivalent; `_scalar` is the reference.  Declared here so
 * the definitions satisfy -Wmissing-prototypes. */
#define LZR_DECL_BLOCK(name)                                                  \
    int name(lzr_model *m, const uint8_t *payload, size_t csize,              \
             uint8_t *dst, size_t dst_capacity, size_t dst_pos,               \
             size_t *out_len, lzraven_status *status)

LZR_DECL_BLOCK(lzr_decode_block_scalar);
#if LZR_HAVE_SSE2
LZR_DECL_BLOCK(lzr_decode_block_sse2);
#endif
#if LZR_HAVE_AVX2
LZR_DECL_BLOCK(lzr_decode_block_avx2);
#endif
#if LZR_HAVE_NEON
LZR_DECL_BLOCK(lzr_decode_block_neon);
#endif

/* The dispatch path in force, resolved once (src/stream.c). */
lzr_simd_kind lzr_simd_active(void);

#endif /* LZRAVEN_INTERNAL_H */
