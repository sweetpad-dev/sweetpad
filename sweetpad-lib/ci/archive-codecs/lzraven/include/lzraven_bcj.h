/* SPDX-License-Identifier: 0BSD */
/*
 * lzraven_bcj.h — BCJ-style executable prefilters for the LZRAVEN codec.
 *
 * LZRAVEN block headers carry two filter bits (SPEC.md 2.2).  After the whole
 * stream has been entropy-decoded, the driver walks the output in 512 KiB
 * chunks and, for each chunk, undoes whichever filters that chunk's flags
 * recorded.  Skipping this step returns silently corrupt output on any Mach-O
 * payload, which is the primary real-world workload.
 *
 *   flag bit 1   ARM64 branch filter (BL / ADRP)   -> lzraven_bcj_arm64_decode
 *   flag bit 2   x86 CALL/JMP rel32 filter         -> lzraven_bcj_x86_decode
 *   bits [11:10] ARM64 alignment phase, 0..3       -> the `phase` argument
 *
 * ORDER MATTERS.  The encoder applies ARM64 first and x86 second, so the
 * decoder must undo x86 first and ARM64 second:
 *
 *     if (flags & 4) lzraven_bcj_x86_decode  (chunk, n);
 *     if (flags & 2) lzraven_bcj_arm64_decode(chunk, n, (flags >> 10) & 3);
 *
 * Format reference: docs/notes/block-format.md 7.
 * All functions transform `buf` in place, touch no byte outside
 * [buf, buf + len), never allocate, and are safe on untrusted input.
 */
#ifndef LZRAVEN_BCJ_H
#define LZRAVEN_BCJ_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Output chunk the filters are dispatched over (512 KiB). */
#define LZRAVEN_BCJ_CHUNK 0x80000u

/*
 * A chunk with fewer than this many bytes is not filtered at all — Apple's
 * driver skips it before looking at the flags.  Every entry point below
 * enforces the rule, so a short final chunk is a no-op whatever its flags say.
 */
#define LZRAVEN_BCJ_MIN_CHUNK 64u

/*
 * ---------------------------------------------------------------------------
 * Chunk at output offset 0.
 * ---------------------------------------------------------------------------
 * These are the two entry points the decoder core calls for the first (and,
 * for any output <= 512 KiB, the only) chunk.  `phase` is the alignment phase
 * from flag bits [11:10]; only its low two bits are used.
 */
void lzraven_bcj_arm64_decode(uint8_t *buf, size_t len, unsigned phase);
void lzraven_bcj_x86_decode(uint8_t *buf, size_t len);

/*
 * ---------------------------------------------------------------------------
 * Chunk at a nonzero output offset — REQUIRED for outputs over 512 KiB.
 * ---------------------------------------------------------------------------
 * Both filters subtract a term derived from the position of the instruction
 * *within the whole output buffer*, not within the chunk.  For chunk k the
 * caller must therefore pass base_off = k * LZRAVEN_BCJ_CHUNK:
 *
 *     lzraven_bcj_x86_decode_at  (out + base, n, base);
 *     lzraven_bcj_arm64_decode_at(out + base, n, phase, base);
 *
 * Calling the base-0 forms above on chunk k >= 1 produces wrong bytes (the
 * ARM64 term is off by base >> 2 and the ADRP term by base >> 12).  Verified
 * against Apple's encoder on an 825,104-byte ARM64 payload whose second chunk
 * is also filtered.  The base-0 forms are exactly these with base_off = 0.
 */
void lzraven_bcj_arm64_decode_at(uint8_t *buf, size_t len, unsigned phase,
                                 uint64_t base_off);
void lzraven_bcj_x86_decode_at(uint8_t *buf, size_t len, uint64_t base_off);

/*
 * ---------------------------------------------------------------------------
 * Encode direction (forward transform).
 * ---------------------------------------------------------------------------
 * Not needed to decode a stream; provided because it is the exact inverse and
 * makes round-trip testing possible.  Apply ARM64 first, then x86.
 */
void lzraven_bcj_arm64_encode(uint8_t *buf, size_t len, unsigned phase);
void lzraven_bcj_x86_encode(uint8_t *buf, size_t len);
void lzraven_bcj_arm64_encode_at(uint8_t *buf, size_t len, unsigned phase,
                                 uint64_t base_off);
void lzraven_bcj_x86_encode_at(uint8_t *buf, size_t len, uint64_t base_off);

#ifdef __cplusplus
}
#endif

#endif /* LZRAVEN_BCJ_H */
