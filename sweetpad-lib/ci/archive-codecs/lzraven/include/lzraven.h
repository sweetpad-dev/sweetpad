/* SPDX-License-Identifier: 0BSD */
/* lzraven — a decoder for Apple's LZRAVEN codec (COMPRESSION_LZRAVEN, 0xD05).
 *
 * Clean-room reimplementation from `SPEC.md` in this repository. Nothing here
 * derives from Apple source; the format was recovered by interoperability
 * reverse-engineering and validated byte-exactly against Apple's own codec.
 *
 * Buffer API only. LZRAVEN has no streaming interface — the block model, the
 * repeat distances and the CDFs all persist across blocks and matches reference
 * the output buffer directly, so a partial decode has no meaningful state to
 * hand back. Apple ships the same shape.
 *
 * Thread-safety: the decoder is a pure function of its arguments. It does not
 * allocate and may be called concurrently on distinct buffers. Its one global
 * is the cached SIMD dispatch choice below, which is idempotent and published
 * atomically; `lzraven_simd_set` is the only way to change it and is not itself
 * thread-safe — call it before decoding starts, if at all.
 */
#ifndef LZRAVEN_H
#define LZRAVEN_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Stream magic; on disk (little-endian): 2B 00 CA EC. */
#define LZRAVEN_MAGIC 0xECCA002Bu

/* Every block decodes to exactly this many bytes except the last. */
#define LZRAVEN_BLOCK_SIZE 0x80000u

/* Upper bound on blocks in one stream, hence on output: 8192 * 512 KiB = 4 GiB.
 * This is Apple's limit too — its per-block flag array is 8192 entries wide —
 * and it lets the decoder run without allocating. */
#define LZRAVEN_MAX_BLOCKS 8192u

/* Why an error enum and not a return code: `lzraven_decode_buffer` has to keep
 * Apple's shape, where the return is "bytes written, 0 on failure" — and 0 is
 * ambiguous, because a well-formed empty stream legitimately decodes to zero
 * bytes. Callers that need to tell those apart, or that want to log *why* a CDN
 * blob was rejected, pass a status out-parameter to the _ex form. An out-param
 * rather than a `lzraven_errno()` accessor: a global (or TLS) error slot would
 * make an otherwise pure, reentrant function stateful, which is a poor trade
 * for a decoder that will be called from a consumer's worker threads. */
typedef enum lzraven_status {
    LZRAVEN_OK = 0,
    LZRAVEN_E_ARGS,        /* NULL buffer with nonzero size, etc. */
    LZRAVEN_E_MAGIC,       /* stream magic is not 0xECCA002B */
    LZRAVEN_E_PARAMS,      /* reserved params bits [25:0] are set */
    LZRAVEN_E_TRUNCATED,   /* stream, block payload or terminator is missing */
    LZRAVEN_E_FRAMING,     /* block size / 512 KiB lockstep violation */
    LZRAVEN_E_OUTPUT_FULL, /* dst_capacity too small for the stream */
    LZRAVEN_E_TOO_MANY_BLOCKS, /* stream exceeds LZRAVEN_MAX_BLOCKS */
    LZRAVEN_E_DISTANCE,    /* match distance exceeds the bytes written so far */
    LZRAVEN_E_CORRUPT      /* entropy/grammar layer: exhausted, undrained,
                            * unconsumed, or a symbol overrunning the block */
} lzraven_status;

/* Decode a complete LZRAVEN stream.
 *
 * Returns the number of bytes written to `dst`, or 0 on failure (Apple's
 * convention; note that a valid empty stream also returns 0). Nothing is read
 * outside [src, src + src_size) and nothing is written outside
 * [dst, dst + dst_capacity). `dst` may be NULL only when dst_capacity is 0.
 *
 * The input is assumed to be attacker-controlled: every size, flag and offset
 * in the stream is bounds-checked, and a malformed stream is rejected rather
 * than truncated. There is no checksum anywhere in a raven stream, so a
 * successful return means "the framing and the entropy stream were internally
 * consistent", never "this is the data that was encoded" — integrity has to
 * come from the container above (pbzm / the OTA manifest).
 *
 * Decoding allocates nothing, but keeps the whole model as locals: about 35 KB
 * of stack (the adaptive model plus the per-block flags), which an embedded
 * caller should budget for. */
size_t lzraven_decode_buffer(uint8_t *dst, size_t dst_capacity,
                             const uint8_t *src, size_t src_size);

/* As above; `status`, if non-NULL, receives LZRAVEN_OK or the rejection cause. */
size_t lzraven_decode_buffer_ex(uint8_t *dst, size_t dst_capacity,
                                const uint8_t *src, size_t src_size,
                                lzraven_status *status);

/* A short, static, human-readable form of `status`. Never NULL. */
const char *lzraven_status_string(lzraven_status status);

/* ---------------------------------------------------------------------------
 * Encoder.
 *
 * Same shape as Apple's: buffer in, buffer out, caller-provided scratch, and
 * "bytes written, 0 on failure" as the return. Like the decoder it never calls
 * the allocator — all working state is the scratch block, whose size is fixed
 * before the encode and depends only on the settings, never on the input.
 *
 * What it emits: stored blocks, and entropy-coded blocks using the whole symbol
 * grammar — literals, matched literals, matches at a fresh distance and matches
 * at any of the four repeat distances. The parse is greedy with a one-position
 * lazy lookahead, priced against the live adaptive model rather than a static
 * table. On real OS 27 OTA plaintext that lands within a few per cent of Apple's
 * own encoder; an optimal parse is the remaining gap and is not implemented.
 *
 * Distances stay inside 8 MiB by default, which is Apple's encoder's window.
 * `lzraven_encode_params` can raise that; read the warning on its `window` field
 * before doing so.
 *
 * The scratch is large — a few thousand times the decoder's ~35 KB — in two
 * parts, neither an artefact of this implementation:
 *
 *   - rANS codes symbols backwards while the model adapts forwards, so an
 *     encoder cannot adapt as it writes: it has to walk a whole block's symbols
 *     forwards, recording what each must be coded with, and only then code them
 *     in reverse. Apple's own encoder scratch is 10,907,568 bytes against its
 *     decoder's 35,424 for that reason alone.
 *   - the match finder indexes a window of the input at four bytes per position
 *     plus a hash table, which is the larger half and is the same cost any LZ77
 *     encoder pays for the same dictionary size (xz at an 8 MiB dictionary asks
 *     for rather more).
 * ------------------------------------------------------------------------- */

/* Bytes of scratch `lzraven_encode_buffer` requires. Constant for a given build
 * — it is the default-parameter case of `lzraven_encode_scratch_size_params`
 * below — and the returned size already includes the slack the encoder needs to
 * align the pointer it is given up to 64 bytes, so any address will do. */
size_t lzraven_encode_scratch_size(void);

/* An upper bound on the encoded size of `src_size` bytes. The encoder never
 * exceeds it: a block whose coded form would not be smaller than the plaintext
 * is stored verbatim instead, so the worst case is the whole input stored, which
 * is the header, one u32 per 512 KiB block, the input, and the terminator.
 *
 * Returns 0 when that bound does not fit in a size_t — possible only when
 * size_t is 32 bits and src_size is within pages of its maximum, an input no
 * addressable output buffer could hold anyway. Refused rather than wrapped, so
 * a caller allocating `lzraven_encode_bound(n)` bytes can never be handed a
 * bound smaller than n. */
size_t lzraven_encode_bound(size_t src_size);

/* Encode `src_size` bytes into `dst`.
 *
 * Returns the number of bytes written, or 0 on failure — the decoder's
 * convention, and note that a zero-length input still produces a 12-byte stream
 * rather than nothing. `scratch` must point to at least
 * `lzraven_encode_scratch_size()` bytes and may be uninitialised; nothing is
 * read outside [src, src + src_size) or written outside
 * [dst, dst + dst_capacity). `src` may be NULL only when src_size is 0.
 *
 * Thread-safe on distinct buffers and distinct scratch: like the decoder this is
 * a pure function of its arguments, with no global state at all. */
size_t lzraven_encode_buffer(uint8_t *dst, size_t dst_capacity,
                             const uint8_t *src, size_t src_size,
                             void *scratch);

/* As above; `status`, if non-NULL, receives LZRAVEN_OK or the failure cause. */
size_t lzraven_encode_buffer_ex(uint8_t *dst, size_t dst_capacity,
                                const uint8_t *src, size_t src_size,
                                void *scratch, lzraven_status *status);

/* ---------------------------------------------------------------------------
 * Encoder tuning.
 *
 * The four functions above encode at the default settings.  The ones below take
 * an explicit parameter block instead, so that ratio can be traded against speed
 * and against working set.  What comes out is the same kind of stream either
 * way — same framing, same grammar, same invariants — and `level` in particular
 * changes only which parse is chosen, never what a decoder has to implement.
 * `window` is the one parameter that can change that; see its warning below.
 *
 * Fill the block with `lzraven_encode_params_default` and then change the fields
 * you care about.  Do not brace-initialise it: it has reserved fields that must
 * be zero, and it may grow.
 *
 *     lzraven_encode_params p;
 *     lzraven_encode_params_default(&p);
 *     p.level = 8;
 *     void *scratch = malloc(lzraven_encode_scratch_size_params(&p));
 *     size_t n = lzraven_encode_buffer_params(dst, cap, src, len, scratch, &p);
 * ------------------------------------------------------------------------- */

/* `level` — how hard the match finder looks, 0 (fastest) to 9 (best ratio).
 *
 * It sets the hash-chain search depth and the match length that ends a search;
 * the parse itself, including its lazy lookahead, is the same at every level.
 * Measured on 94 MB of real OS 27 OTA plaintext (`tools/enc_real.py --levels`),
 * against Apple's own encoder's 0.0334 on the same bytes:
 *
 *     level     0      1      2      3      4      5      6      7      8      9
 *     ratio  .0389  .0378  .0370  .0364  .0360  .0353  .0347  .0344  .0341  .0340
 *     MB/s   109.9  116.5  111.7  104.9   98.6   90.5   70.7   51.2   32.1   13.4
 *
 * Diminishing returns are steep in both directions.  Below 3 the search is no
 * longer what costs the time — the parse's own pricing is — so those levels give
 * up ratio and buy almost no speed; above 6 each step costs roughly a third of
 * the throughput for a few tenths of a per cent.  The whole range is 13% of
 * ratio for 8x the time, and 6 is the default because it is the knee.  Figures
 * are one x86-64 machine's, and are for comparing levels rather than quoting. */
#define LZRAVEN_ENCODE_LEVEL_MIN 0
#define LZRAVEN_ENCODE_LEVEL_MAX 9
#define LZRAVEN_ENCODE_LEVEL_DEFAULT 6

/* `window` — the largest match distance the encoder will emit, in bytes.
 *
 * The default is Apple's encoder's hardcoded match-finder window.  Scratch grows
 * with the window (4 bytes per window position, rounded up to a power of two),
 * so lowering it is the way to encode with a small working set, and raising it
 * is what lets a long-range repeat in a large input be matched at all. */
#define LZRAVEN_ENCODE_WINDOW_DEFAULT ((size_t)8u << 20)
#define LZRAVEN_ENCODE_WINDOW_MAX ((size_t)1u << 30)

/* Distances above 16 MiB — the top of distance class 4 — are in classes 5 and 6.
 * **Apple's own encoder never emits those**, because its window is fixed at
 * 8 MiB, so a stream carrying one is a shape its encoder cannot produce.
 *
 * Apple's *decoder* implements all seven classes, and this library's encoding of
 * 5 and 6 is verified byte-exactly against it (SPEC.md §5.4) — so a window above
 * 16 MiB is correct, not merely believed to be.  What it is not is *familiar*: a
 * third-party consumer written by observing Apple's encoder may never have
 * exercised its own class 5/6 path, and a stream from here would be the first
 * thing to try it.  At or below this bound every distance is in classes 0..4 and
 * every stream is a shape Apple's encoder could itself have emitted. */
#define LZRAVEN_ENCODE_WINDOW_APPLE_SHAPED ((size_t)16u << 20)

typedef struct lzraven_encode_params {
    /* [LZRAVEN_ENCODE_LEVEL_MIN, LZRAVEN_ENCODE_LEVEL_MAX]; out of range is an
     * error rather than a clamp, so that a caller passing a level from some
     * other codec's 1..22 scale finds out. */
    int level;
    /* 0 = LZRAVEN_ENCODE_WINDOW_DEFAULT.  Any other figure is used as given —
     * it is a distance bound, not a power-of-two log — except that it is clamped
     * to LZRAVEN_ENCODE_WINDOW_MAX, and a window larger than the input never
     * binds.  Scratch is allocated for it whether it binds or not, so asking for
     * more than the input's length is only a waste.  Read
     * LZRAVEN_ENCODE_WINDOW_APPLE_SHAPED before going past 16 MiB. */
    size_t window;
    /* Must be zero; `lzraven_encode_params_default` zeroes them.  Room for the
     * knobs an optimal parse will want, without another struct. */
    uint64_t reserved[6];
} lzraven_encode_params;

/* The defaults — the settings the non-`_params` entry points above use. */
void lzraven_encode_params_default(lzraven_encode_params *p);

/* Bytes of scratch `lzraven_encode_buffer_params` requires for `p`, or 0 if `p`
 * is not a valid parameter block (which is also how to test one before
 * committing to an encode).  `p` may be NULL, meaning the defaults, in which
 * case this returns exactly `lzraven_encode_scratch_size()`.
 *
 * Derivable before encoding and independent of the input, because the caller has
 * to allocate it first; it is dominated by the match finder's chain array, hence
 * by `window`. */
size_t lzraven_encode_scratch_size_params(const lzraven_encode_params *p);

/* Encode with explicit parameters.  `p` may be NULL, meaning the defaults.
 * Everything else is as `lzraven_encode_buffer`, except that `scratch` must be
 * at least `lzraven_encode_scratch_size_params(p)` bytes — the size for *these*
 * parameters, not the default ones.  Invalid parameters fail with
 * LZRAVEN_E_ARGS rather than silently encoding at some other setting. */
size_t lzraven_encode_buffer_params(uint8_t *dst, size_t dst_capacity,
                                    const uint8_t *src, size_t src_size,
                                    void *scratch,
                                    const lzraven_encode_params *p);

/* As above; `status`, if non-NULL, receives LZRAVEN_OK or the failure cause. */
size_t lzraven_encode_buffer_params_ex(uint8_t *dst, size_t dst_capacity,
                                       const uint8_t *src, size_t src_size,
                                       void *scratch,
                                       const lzraven_encode_params *p,
                                       lzraven_status *status);

/* ---------------------------------------------------------------------------
 * SIMD dispatch.
 *
 * The CDF search and update are two thirds of decode time and are
 * hand-vectorised per instruction set; the path is chosen at run time from
 * CPUID (x86) or the architecture baseline (arm64), so no build ever *requires*
 * a wider instruction set than the machine has. The portable C path is always
 * compiled in and is the reference implementation: every vector path is
 * required to be byte-identical to it, and is tested that way.
 *
 * Named paths: "scalar", "sse2", "avx2", "neon". These exist so the choice can
 * be pinned — for benchmarking, for differential testing, and for anyone who
 * would rather not run vector code at all.
 * ------------------------------------------------------------------------- */

/* The path in force. Never NULL. On the first call the choice is resolved from
 * the environment variable LZRAVEN_SIMD (one of the names above, or "auto")
 * and then from what the CPU supports. */
const char *lzraven_simd_path(void);

/* Pin the path to `name`, or to "auto" to re-resolve it. Returns 1 if the path
 * was selected, 0 if this build or this CPU cannot run it (in which case the
 * current path is unchanged). Not thread-safe; call before decoding. */
int lzraven_simd_set(const char *name);

#ifdef __cplusplus
}
#endif

#endif /* LZRAVEN_H */
