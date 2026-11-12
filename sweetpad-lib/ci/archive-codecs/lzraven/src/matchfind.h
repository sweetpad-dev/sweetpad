/* SPDX-License-Identifier: 0BSD */
/* lzraven — LZ77 match finder for the encoder.
 *
 * Finds, for a position in the input, the match the symbol grammar of SPEC.md §5
 * can encode most cheaply.  It is a standalone component: it depends on nothing
 * else in the library, allocates nothing, and can be compiled and tested on its
 * own (see tests/c/test_matchfind.c).
 *
 * ---------------------------------------------------------------------------
 * DIFFERENCES FROM THE PINNED SKETCH — read this if you are coding against it
 * ---------------------------------------------------------------------------
 *  1. `struct lzr_mf` is *defined here* rather than opaque, because
 *     `lzr_mf_init(lzr_mf *mf, ...)` requires the caller to own the object.  It
 *     holds only pointers, counters and the resolved tuning — no tables — so
 *     `lzr_mf mf;` as an automatic is the intended use.  Treat every field as
 *     private.
 *  2. `lzr_mf_find` never returns 1.  The grammar *can* encode a length-1 match
 *     (command 7, the short rep at rep0), but a caller that fed a length-1
 *     result to a fresh-distance command would emit an unrepresentable symbol,
 *     so that case is a separate, explicit call: `lzr_mf_short_rep`.
 *  3. Additions, all optional: `lzr_mf_init_params` (tuning), `lzr_mf_valid`
 *     (was the scratch big enough), `lzr_mf_len_at` (raw byte-compare at a given
 *     distance), `lzr_mf_dist_class` (SPEC.md §5.4 class of a distance).
 *
 * ---------------------------------------------------------------------------
 * What the finder reports, and why
 * ---------------------------------------------------------------------------
 * The grammar makes some matches far cheaper than others, so "longest" is not
 * the same as "best":
 *
 *  - A match at one of the four repeat distances (rep0..rep3) needs one command
 *    symbol and *no distance symbols at all*, so a slightly shorter rep match
 *    routinely beats a longer fresh one.  `lzr_mf_find` therefore searches the
 *    rep set as well as the hash chains, reports `*rep_slot` whenever the
 *    distance it chose is a rep distance, and — above level 0 — will prefer a
 *    rep over a longer fresh match when a static price model says that is
 *    cheaper per byte covered.
 *  - Lengths are capped at LZR_MF_LEN_MAX (332), the longest a fresh-distance
 *    match command can carry (2 + the ladder's 330).  Long reps could carry 348
 *    (base 18 + 330); those extra 16 bytes are deliberately left on the table so
 *    that a caller may always re-encode a reported match as a fresh distance
 *    without re-checking the cap.  `lzr_mf_len_at` measures past the cap if you
 *    want them.
 *  - Distances default to a bound of LZR_MF_WINDOW_MAX = 8 MiB, which is Apple's
 *    encoder's hardcoded match-finder window.  This is not an arbitrary choice:
 *    distance classes 5 and 6 of SPEC.md §5.4 are the ones Apple's encoder
 *    cannot emit, and staying inside 8 MiB keeps every distance in classes 0..4,
 *    which are validated byte-exactly against streams it did emit.  (Class 4 in
 *    fact runs to 16 MiB, so the default has 8 MiB of headroom before it would
 *    reach class 5 — the point is to match what Apple's encoder is known to
 *    produce, not to sit on the class edge.)  It is a default and not a limit:
 *    see the warning below.
 *
 * ---------------------------------------------------------------------------
 * WARNING — a window above 8 MiB leaves what Apple's encoder can produce
 * ---------------------------------------------------------------------------
 * `lzr_mf_params.window` accepts anything up to LZR_MF_WINDOW_LIMIT, and a
 * caller that asks for more than LZR_MF_WINDOW_MAX gets it.  Understand what
 * that buys and what it costs:
 *
 *  - Above 16 MiB (2^24, the top of class 4) the finder can report distances in
 *    SPEC.md §5.4's classes 5 and 6.  **Apple's own encoder never emits those**,
 *    its window being fixed at 8 MiB, so a stream carrying one is a shape its
 *    encoder could not have produced — and §5.4 is where this project records
 *    exactly how far those two classes have been validated, which is the one
 *    place that answer belongs.  Asking for such a window is opting into that,
 *    and a consumer written by observing Apple's encoder may never have
 *    exercised its own class 5/6 path.
 *  - Between 8 and 16 MiB every distance is still class 4, so the coding is on
 *    the validated side of that question and only the *window* is larger than
 *    Apple's.  That much is safe by inspection.
 *  - Scratch grows with the window, not with the input: the chain array is one
 *    32-bit word per window position, rounded up to a power of two.  Size it
 *    with lzr_mf_scratch_size_window(), not lzr_mf_scratch_size(), or the
 *    window silently shrinks back to fit (which is safe, just not what was
 *    asked for).
 *
 * The finder itself is exact at any window — the tests assert brute-force
 * equality above 8 MiB as well as below, and reach both class 5 and class 6.
 * The caveat is entirely about what a decoder on the far end will accept, not
 * about whether the match is right.
 *
 * ---------------------------------------------------------------------------
 * Call protocol
 * ---------------------------------------------------------------------------
 *   size_t   n  = lzr_mf_scratch_size(len);
 *   void    *sc = ...;                    // n bytes, any alignment
 *   lzr_mf   mf;
 *   lzr_mf_init(&mf, sc, n, buf, len);
 *   for (pos = 0; pos < len; ) {
 *       unsigned l = lzr_mf_find(&mf, pos, reps, &d, &slot);
 *       if (l) { emit_match(l, d, slot); lzr_mf_skip(&mf, pos, l); pos += l; }
 *       else   { emit_literal(buf[pos]); pos += 1; }
 *   }
 *
 * `pos` must not move backwards between calls; the finder keeps a single
 * insertion cursor, and it is only ever advanced.  Within that rule the answer
 * is a pure function of (pos, reps): asking twice gives the same result, and the
 * finder holds no decision state between calls.
 *
 * `lzr_mf_find` indexes every position below `pos` that is not indexed yet, so
 * the `lzr_mf_skip` call above is not load-bearing — the positions a taken match
 * consumed get indexed by the next `lzr_mf_find` regardless.  Call it anyway when
 * you want the work done at a predictable point, or to advance the index with no
 * search at all.
 *
 * A zero return means "encode a literal here".  At levels that enable lazy
 * matching that includes the lazy decision: the finder looked at pos+1, found
 * something enough better to pay for a literal, and said so.  The next call at
 * pos+1 searches again rather than trusting a cached lookahead — the lookahead
 * runs against an index one position shorter, so caching it would make the
 * answer depend on how it was reached.
 */
#ifndef LZRAVEN_MATCHFIND_H
#define LZRAVEN_MATCHFIND_H

#include <stddef.h>
#include <stdint.h>
#include <string.h>

/* Apple's encoder window, and this finder's default.  See the note above: this
 * is a compatibility bound, not a memory bound, and a caller may ask for more. */
#define LZR_MF_WINDOW_MAX ((uint32_t)8u << 20)

/* Largest input this finder indexes; positions are held as uint32_t. */
#define LZR_MF_INPUT_MAX ((size_t)0x7FFFFFFFu)

/* Largest window it will honour.  No distance can exceed the input length, so
 * this is that same bound; a larger request is clamped here rather than allowed
 * to overflow anything.  Every uint32_t distance is in some class 0..6, so this
 * is also the point past which SPEC.md §5.4 has nothing left to say. */
#define LZR_MF_WINDOW_LIMIT ((uint32_t)0x7FFFFFFFu)

/* Match lengths the grammar can encode with a fresh distance (SPEC.md §5.3). */
#define LZR_MF_LEN_MIN 2u
#define LZR_MF_LEN_MAX 332u

/* Tuning presets.  0 is fastest, 9 is most thorough, 6 is the default.  The
 * dial is monotone: every level searches a superset of the candidates the level
 * below it did, so its match is never shorter — asserted at every position of
 * three corpora in tests/c/test_matchfind.c, not merely intended.
 *
 *   level      0     1     2     3     4     5     6     7     8     9
 *   depth      2     4     8    16    24    32    64   128   256  1024
 *   nice       8    12    16    24    32    48    96   160   240   332
 *   lazy       -     -     -     -     x     x     x     x     x     x
 *   exact%  61.3  65.7  68.4  71.9  74.5  78.5  84.4  88.4  92.2  97.1
 *   steps/B 0.02  0.04  0.08  0.15  0.24  0.39  0.87  1.73  2.98  7.08
 *   MB/s     244   243   240   210   195   166   101    57    34    15
 *
 *   depth    chain entries per search
 *   nice     length that ends a search
 *   lazy     one-position lookahead
 *   exact%   share of positions where the match equals brute force's
 *   steps/B  chain entries examined per input byte
 *   MB/s     bytes per second through lzr_mf_find
 *
 * Measured on 8 MiB of real decoded OS 27 OTA plaintext, one x86-64 machine,
 * driven the way an encoder core drives it (LZR_MF_F_NO_HEUR | LZR_MF_F_NO_LAZY,
 * so the figures isolate depth and nice_len; the finder's own heuristics and
 * lazy lookahead roughly halve the throughput above level 5).  `exact%` is over
 * a 2,000-position sample at a 1 MiB window, brute force being quadratic.  The
 * numbers are for comparing levels, not for quoting as throughput; the ranking
 * held on every real payload tried, the absolute values did not.  Reproduce with
 *   test_matchfind --levels <plaintext>
 *
 * Diminishing returns are steep and the knee is level 6: 0..6 buys 23 points of
 * exactness for 2.4x the time, and 6..9 buys 13 more for another 7x.
 *
 * LZR_MF_LEVEL_EXACT is not a speed preset: unbounded chain depth, no early exit,
 * no price heuristics and no lazy matching, so the finder returns the true
 * longest match (ties broken by the smallest distance) and is exactly equivalent
 * to brute force.  The tests use it as the oracle, and it is far too slow for
 * production use.  Combining it with an explicit `depth` measures what a
 * depth-limited chain gives up, which is how the quality figures were produced. */
#define LZR_MF_LEVEL_DEFAULT 6u
#define LZR_MF_LEVEL_EXACT 10u

/* The presets' nice-lengths, by level — named, because two tables are spelled
 * from these figures: the preset table in matchfind.c and the encoder's rep
 * early-exit table (src/encode.c), which must stop at the same length or a
 * level that searches deeper is undercut by an exit calibrated for a lower
 * one.  Both are written from these names so they cannot drift.  Level 10 has
 * no name here on purpose: its nice_len is the unreachable LZR_MF_LEN_MAX + 1
 * (see the note under the preset table) and nothing outside the finder may
 * mirror it. */
#define LZR_MF_NICE_0 8u
#define LZR_MF_NICE_1 12u
#define LZR_MF_NICE_2 16u
#define LZR_MF_NICE_3 24u
#define LZR_MF_NICE_4 32u
#define LZR_MF_NICE_5 48u
#define LZR_MF_NICE_6 96u
#define LZR_MF_NICE_7 160u
#define LZR_MF_NICE_8 240u
#define LZR_MF_NICE_9 LZR_MF_LEN_MAX

/* lzr_mf_params.depth: search this many chain entries and no more. */
#define LZR_MF_DEPTH_UNLIMITED 0xFFFFFFFFu

/* lzr_mf_params.flags — override what the level implies.  All zero = take the
 * level's word for it.  LZR_MF_F_NO_HEUR turns off both price-model decisions
 * (the rep preference and the "not worth taking" prune), leaving the raw longest
 * match; an encoder core with a real adaptive price model wants that. */
#define LZR_MF_F_NO_LAZY 1u
#define LZR_MF_F_LAZY 2u
#define LZR_MF_F_NO_HEUR 4u

typedef struct lzr_mf_params {
    unsigned level; /* 0..10; 0 is a valid preset, see above             */
    /* Max distance in bytes.  0 => LZR_MF_WINDOW_MAX, the default and what
     * Apple's encoder uses.  A larger value is honoured up to
     * LZR_MF_WINDOW_LIMIT — read the warning at the top of this file first, and
     * size the scratch with lzr_mf_scratch_size_window(). */
    uint32_t window;
    unsigned depth;    /* chain entries per search; 0 => from level      */
    unsigned nice_len; /* stop searching at this length; 0 => from level */
    uint32_t flags;    /* LZR_MF_F_*                                     */
} lzr_mf_params;

/* All fields private — use the functions. */
typedef struct lzr_mf {
    const uint8_t *buf;
    size_t len;
    uint32_t *head2; /* [1<<16]           newest position per 2-byte key   */
    uint32_t *head3; /* [1<<hash_bits]    chain heads, 3-byte hash         */
    uint32_t *prev;  /* [prev_mask+1]     chain links, keyed pos&prev_mask */
    uint32_t prev_mask;
    uint32_t window;
    uint32_t ins; /* positions [0, ins) are indexed */
    unsigned hash_bits;
    unsigned depth;
    unsigned nice_len;
    unsigned level;
    int lazy;
    int heur; /* rep preference + "not worth taking" pruning */
    int valid;
    /* Statistics, for tuning.  Never read by the finder itself. */
    uint64_t stat_searches;    /* lzr_mf_find calls that ran a search      */
    uint64_t stat_lookaheads;  /* extra searches the lazy path ran         */
    uint64_t stat_lazy_skips;  /* times the lazy path asked for a literal  */
    uint64_t stat_chain_steps; /* chain entries examined                   */
} lzr_mf;

/* Bytes of scratch needed to index an input of `max_input` bytes at the default
 * window.  Returns 0 if `max_input` exceeds LZR_MF_INPUT_MAX.  Independent of
 * the tuning, so one allocation serves every level.  It is a 256 KiB two-byte
 * table plus one 32-bit word per hash bucket and per window position: 4.25 MiB
 * for a 512 KiB block, 48.25 MiB for an 8 MiB payload chunk. */
size_t lzr_mf_scratch_size(size_t max_input);

/* The same for an explicit window (0 => the default, i.e. identical to
 * lzr_mf_scratch_size).  The chain array covers min(max_input, window) positions
 * rounded up to a power of two, so a window above LZR_MF_WINDOW_MAX costs
 * proportionally more: 4 bytes per window position.  Returns 0 if `max_input`
 * exceeds LZR_MF_INPUT_MAX or if the size would not fit in a size_t. */
size_t lzr_mf_scratch_size_window(size_t max_input, uint32_t window);

/* Initialise at the default level.  `scratch` needs no particular alignment.
 * Less scratch than lzr_mf_scratch_size(len) is not an error: the tables shrink
 * to fit, which costs match quality (and, at the extreme, shortens the effective
 * window) but never correctness.  If not even the floor fits, the finder becomes
 * inert — lzr_mf_valid() returns 0 and lzr_mf_find() always returns 0, which
 * still encodes correctly, as all literals. */
void lzr_mf_init(lzr_mf *mf, void *scratch, size_t scratch_size,
                 const uint8_t *buf, size_t len);

/* As lzr_mf_init, with explicit tuning.  `params` may be NULL (= defaults). */
void lzr_mf_init_params(lzr_mf *mf, void *scratch, size_t scratch_size,
                        const uint8_t *buf, size_t len,
                        const lzr_mf_params *params);

/* 0 if init could not build usable tables (see lzr_mf_init). */
int lzr_mf_valid(const lzr_mf *mf);

/* Best match at `pos`, or 0 to encode a literal there.
 *
 * `reps` is the caller's current rep0..rep3 (a 0 entry means "unused"); it may be
 * NULL.  On a non-zero return, *dist is the distance — always in [1, min(pos,
 * window)] — and *rep_slot is the index in `reps` whose distance was chosen, or
 * -1 for a fresh distance.  The returned length is in [2, LZR_MF_LEN_MAX] and
 * the bytes at `pos` and `pos - *dist` genuinely agree over it.
 *
 * Never returns 1: see the header notes and lzr_mf_short_rep. */
unsigned lzr_mf_find(lzr_mf *mf, size_t pos, const uint32_t reps[4],
                     uint32_t *dist, int *rep_slot);

/* Advance the index over [pos, pos + n) without searching it — for the positions
 * a taken match consumed.  Optional: lzr_mf_find indexes any gap below its own
 * position itself (see the call protocol above).  `n` is clamped to the end of
 * the input. */
void lzr_mf_skip(lzr_mf *mf, size_t pos, unsigned n);

/* 1 if command 7 (short rep: rep0, length 1) applies at `pos`, i.e. rep0 is
 * usable there and the single byte matches.  Split out of lzr_mf_find so that a
 * length-1 result can never be mistaken for something a fresh-distance command
 * could encode. */
int lzr_mf_short_rep(const lzr_mf *mf, size_t pos, const uint32_t reps[4]);

/* Raw byte-compare: how far the match at `pos` for distance `dist` runs, capped
 * at `cap` and at the bytes remaining.  0 if `dist` is unusable at `pos` (zero,
 * past `pos`, or past the window).  `cap` is not clamped to LZR_MF_LEN_MAX — pass
 * 348 to measure a long rep in full. */
unsigned lzr_mf_len_at(const lzr_mf *mf, size_t pos, uint32_t dist,
                       unsigned cap);

/* Distance class of `dist` — the command byte a fresh-distance match would use
 * (SPEC.md §5.4).  -1 for dist == 0.  At any window up to 16 MiB — which
 * includes the default — every distance lzr_mf_find reports has class <= 4.
 * Classes 5 and 6 are the ones Apple's encoder cannot emit, and only a window
 * above 16 MiB can reach them; see the warning at the top of this file. */
int lzr_mf_dist_class(uint32_t dist);

/* The length of the common prefix of `a` and `b`, capped at `cap`.  Shared by
 * the finder's chain walk and the encoder's rep measurement — the hottest loop
 * in any hash-chain encoder — so it runs eight bytes per step: load both sides
 * (memcpy, which the compilers turn into one unaligned load), XOR, and the
 * lowest differing byte of the XOR word names the mismatch.  The sub-word tail
 * goes one byte at a time.
 *
 * Bounds: every read stays inside [a, a + cap) and [b, b + cap).  The wide
 * loop runs only while cap - i >= 8, so its last load ends at a + cap at the
 * furthest.  The callers' side of the contract is that `cap` never exceeds the
 * input that remains past the later cursor — the finder clamps it at
 * mf->len - pos, the encoder at coded_end - pos — and that `b` trails `a` in
 * the same buffer, so bounding `a` bounds both. */
static inline unsigned lzr_mf_match_run(const uint8_t *a, const uint8_t *b,
                                        unsigned cap)
{
    unsigned i = 0;

#if defined(__GNUC__) || defined(__clang__)
    while (cap - i >= 8u) {
        uint64_t wa, wb, x;

        memcpy(&wa, a + i, 8);
        memcpy(&wb, b + i, 8);
        x = wa ^ wb;
        if (x != 0u) {
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
            return i + ((unsigned)__builtin_clzll(x) >> 3);
#else
            return i + ((unsigned)__builtin_ctzll(x) >> 3);
#endif
        }
        i += 8u;
    }
#endif
    while (i < cap && a[i] == b[i])
        i++;
    return i;
}

#endif /* LZRAVEN_MATCHFIND_H */
