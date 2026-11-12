/* SPDX-License-Identifier: 0BSD */
/* lzraven — encoder-internal declarations: the caller-provided scratch layout.
 *
 * The public entry points are in include/lzraven.h; this header exists so the
 * one allocation-free scratch block has a name and a size that the public
 * `lzraven_encode_scratch_size` can report.  Nothing here is part of the ABI.
 *
 * Why the encoder's state dwarfs the decoder's ~35 KB, in two parts:
 *
 *   - rANS codes symbols backwards while the model adapts forwards (SPEC.md
 *     §7.2, §11.1), so an encoder cannot adapt as it writes.  It has to walk the
 *     symbols forwards once, recording the (cum, freq) pair each one must be
 *     coded with, and only then code them in reverse.  That recording scales
 *     with the symbol count of a whole block rather than with the model.
 *   - the match finder indexes a window of the input, at four bytes per
 *     position plus a hash table.  That is the larger half, and it is the same
 *     cost any LZ77 encoder pays for the same dictionary size.
 */
#ifndef LZRAVEN_ENCODE_H
#define LZRAVEN_ENCODE_H

#include <stddef.h>
#include <stdint.h>

#include "lzraven.h"
#include "matchfind.h"
#include "model.h"

/* Records per block, worst case.
 *
 * One record per read, except that a raw bypass field costs two — a tag plus its
 * raw value.  Counting them per command class, against the coded bytes the
 * command advances (SPEC.md §5.2):
 *
 *   literal              flag + 2 nibbles                   3 / 1    = 3
 *   cmd 7   short rep    flag + command                     2 / 1    = 2
 *   cmd 8..11  rep       flag + command + length            3 / >= 2 <= 1.5
 *   cmd 12..15 long rep  flag + command + <= 3 ladder     <= 5 / >= 18 <= 0.28
 *   cmd 0   match        flag + command + ladder + 2 dist   5 / >= 2 <= 2.5
 *   cmd 1..6 match       the same plus a bypass field       7 / >= 2 <= 3.5
 *
 * so seven records per two coded bytes bounds any parse this grammar can
 * express, and the worst case is a length-2 match at a distance over 256 rather
 * than a literal.  The head of block 0 adds one bypass for the stream's very
 * first byte.  The forward pass range-checks the bound at run time as well and
 * abandons the block if it is ever exceeded, so being wrong here costs ratio
 * rather than correctness — which is exactly why the derivation above is checked
 * at compile time instead of being left as a comment. */
#define LZR_ENC_REC_WORST 7u  /* records ... */
#define LZR_ENC_REC_BYTES 2u  /* ... per this many coded bytes */
#define LZR_ENC_RECS                                                          \
    ((LZR_ENC_REC_WORST * (size_t)LZRAVEN_BLOCK_SIZE + LZR_ENC_REC_BYTES - 1u) \
         / LZR_ENC_REC_BYTES +                                                \
     64u)

_Static_assert(LZR_ENC_RECS * LZR_ENC_REC_BYTES >=
                   LZR_ENC_REC_WORST * (size_t)LZRAVEN_BLOCK_SIZE,
               "LZR_ENC_RECS must cover the worst records-per-byte command");

/* Payload words per block.  A coded block is kept only if its payload is
 * shorter than the plaintext it replaces, so one block's worth of words is the
 * cap: the writer stops when it is reached and the block falls back to stored. */
#define LZR_ENC_WORDS ((size_t)LZRAVEN_BLOCK_SIZE / 4u)

/* The match finder's view (src/matchfind.h).
 *
 * The finder holds no tables of its own — it indexes a byte range the caller
 * hands it, and the tables scale with that range: 4 bytes per position plus a
 * hash table.  A whole stream can be 4 GiB, so the encoder indexes a *view* of
 * it at a time and re-anchors when a block reaches the end of the current one.
 * The view is a power of two at least as large as the requested window, and the
 * lookback re-anchoring guarantees is half of it; between anchors the finder
 * sees everything back to the view's origin.
 *
 * The floor exists because re-anchoring has to advance by at least a whole
 * block: with a half-view of history, `view/2 + 512 KiB < view` needs a view
 * above 1 MiB.  So a caller asking for a window smaller than that still pays for
 * a 2 MiB view — it is the smallest one the block loop can make progress in —
 * though the finder still bounds distances by the window it was given. */
#define LZR_ENC_VIEW_MIN ((size_t)2u << 20)

/* The view is this rounded up by doubling, so the floor being a power of two is
 * what makes every view one; and the floor has to satisfy the re-anchoring rule
 * itself, or the smallest window a caller may ask for would not encode at all. */
_Static_assert((LZR_ENC_VIEW_MIN & (LZR_ENC_VIEW_MIN - 1u)) == 0,
               "LZR_ENC_VIEW_MIN must be a power of two");
_Static_assert(LZR_ENC_VIEW_MIN / 2u + (size_t)LZRAVEN_BLOCK_SIZE <
                   LZR_ENC_VIEW_MIN,
               "re-anchoring must advance the smallest view by a whole block");

/* The resolved form of a lzraven_encode_params: everything the encoder needs,
 * derived once, so that lzraven_encode_scratch_size_params() and the encode
 * itself cannot disagree about how big the match finder's tables are. */
typedef struct lzr_enc_tune {
    unsigned level;    /* the finder's level, 0..9                        */
    unsigned nice_len; /* rep length at which the chain search is skipped */
    uint32_t window;   /* largest distance the finder may report          */
    size_t view;       /* bytes the finder indexes at once; a power of two */
    size_t history;    /* lookback re-anchoring guarantees; view / 2      */
    size_t mf_bytes;   /* scratch the finder needs for that view + window */
} lzr_enc_tune;

/* Resolve `p` (NULL = the defaults).  Returns 0 if `p` is not a valid parameter
 * block, in which case `*t` is untouched. */
int lzr_enc_resolve(const lzraven_encode_params *p, lzr_enc_tune *t);

/* The fixed head of the scratch block.  The match finder's tables follow it, and
 * are by far the larger part — they are what makes the encoder's scratch tens of
 * megabytes rather than a few — but their size depends on the parameters, so
 * they are a flexible member rather than a field.  `lzr_enc_tune.mf_bytes` is
 * how many bytes of `mf` there are. */
typedef struct lzr_enc_scratch {
    lzr_model model;           /* live model — persists across blocks */
    lzr_model save;            /* checkpoint, for abandoning a coded block */
    uint32_t rec[LZR_ENC_RECS];
    /* The payload under construction, as bytes: the words go in at a decreasing
     * index and are written little-endian, so the buffer is the payload and
     * needs no byte-order pass on either kind of host. */
    uint8_t word[4u * LZR_ENC_WORDS];
    uint8_t mf[];
} lzr_enc_scratch;

#endif /* LZRAVEN_ENCODE_H */
