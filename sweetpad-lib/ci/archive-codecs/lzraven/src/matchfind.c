/* SPDX-License-Identifier: 0BSD */
/* lzraven — LZ77 match finder for the encoder (interface + rationale in
 * src/matchfind.h).
 *
 * Structure: hash chains, in the zlib/liblzma tradition, with two tables.
 *
 *   head2[1<<16]   the newest position sharing the two bytes at pos.  A direct
 *                  table over the exact key, so it has no collisions: it is the
 *                  *nearest* two-byte match, which is the whole answer whenever
 *                  the longest match is only two long.
 *   head3[] + prev[]  a chain per 3-byte hash, newest first.  Every position
 *                  whose match runs to three bytes or more is in the bucket for
 *                  its first three bytes, so the chain cannot miss a candidate —
 *                  a hash collision only wastes a byte-compare.
 *
 * Those two together are why the finder is exactly checkable: walk the chain
 * without a depth limit and the candidate set provably contains the brute-force
 * answer for every length, so LZR_MF_LEVEL_EXACT *is* brute force, and the tests
 * assert equality rather than a bound.  A depth limit then trades quality for
 * speed with no change to what "correct" means.
 *
 * On top of that sit three things the grammar of SPEC.md §5 asks for, all
 * heuristic and all off at level 0:
 *
 *   - the rep set is searched alongside the chains, and a rep match is preferred
 *     over a longer fresh one when it is cheaper per byte covered (a rep costs
 *     one command symbol and no distance symbols at all);
 *   - a match that a static price model says costs more than the literals it
 *     replaces is not reported;
 *   - lazy matching: if the match at pos+1 pays for the literal at pos, report
 *     nothing here and cache pos+1 for the next call.
 *
 * The price model is static and deliberately crude (see lzr_mf_price_* below) —
 * it prices a literal at its worst case, so it errs towards taking matches.  An
 * encoder core with a real adaptive price model should drive these decisions
 * itself and ask this finder for the raw longest match instead (level 0 with a
 * bounded depth, or LZR_MF_LEVEL_EXACT).
 */
#include "matchfind.h"

#include <string.h>

/* ---------------------------------------------------------------------------
 * Tuning presets.  depth / nice_len / lazy / heuristics, by level.
 * ------------------------------------------------------------------------- */
typedef struct lzr_mf_preset {
    unsigned depth;
    unsigned nice_len;
    int lazy;
    int heur;
} lzr_mf_preset;

static const lzr_mf_preset lzr_mf_presets[LZR_MF_LEVEL_EXACT + 1] = {
    /*  0 */ { 2, LZR_MF_NICE_0, 0, 1 },
    /*  1 */ { 4, LZR_MF_NICE_1, 0, 1 },
    /*  2 */ { 8, LZR_MF_NICE_2, 0, 1 },
    /*  3 */ { 16, LZR_MF_NICE_3, 0, 1 },
    /*  4 */ { 24, LZR_MF_NICE_4, 1, 1 },
    /*  5 */ { 32, LZR_MF_NICE_5, 1, 1 },
    /*  6 */ { 64, LZR_MF_NICE_6, 1, 1 },
    /*  7 */ { 128, LZR_MF_NICE_7, 1, 1 },
    /*  8 */ { 256, LZR_MF_NICE_8, 1, 1 },
    /*  9 */ { 1024, LZR_MF_NICE_9, 1, 1 },
    /* 10 */ { LZR_MF_DEPTH_UNLIMITED, LZR_MF_LEN_MAX + 1u, 0, 0 }
};
/* Level 10's nice_len is one past the longest encodable match on purpose: an
 * early exit at the cap would stop the walk before a *nearer* candidate of the
 * same length could displace the incumbent, and brute force breaks ties by
 * distance.  Unreachable nice_len => the walk always completes => exact. */

/* ---------------------------------------------------------------------------
 * The static price model, in bits.  Only ever used to compare two candidates,
 * so absolute accuracy does not matter; the shape does.
 * ------------------------------------------------------------------------- */
/* A literal: the is_match flag plus two nibble symbols, priced incompressible. */
#define LZR_MF_P_LIT 9u
/* is_match flag plus the command symbol, for the two command families. */
#define LZR_MF_P_MATCH_CMD 6u
#define LZR_MF_P_REP_CMD 5u

static unsigned lzr_mf_flog2(uint32_t v) /* v != 0 */
{
    unsigned r = 0;

    while (v > 1) {
        v >>= 1;
        r++;
    }
    return r;
}

/* Distance symbols for `dist` (SPEC.md §5.4): two modelled nibbles for class 0,
 * and for class c the same two plus a (kk-2)-bit bypass field, where the leading
 * term puts dist-1 in [2^(kk+4), 2^(kk+5)).  That works out to
 * floor(log2(dist-1)) + 2 bits, and 8 for class 0. */
static unsigned lzr_mf_price_dist(uint32_t dist)
{
    if (dist <= 256u)
        return 8u;
    return lzr_mf_flog2(dist - 1u) + 2u;
}

/* The shared length ladder (SPEC.md §5.3): `v` is length minus the base (2 for a
 * match, 18 for a long rep).  One symbol for v <= 9 and for the saturating
 * v == 330, two for v <= 73, three above that. */
static unsigned lzr_mf_price_ladder(unsigned v)
{
    if (v <= 9u)
        return 4u;
    if (v == 330u)
        return 4u;
    if (v <= 73u)
        return 8u;
    return 12u;
}

static unsigned lzr_mf_price_fresh(unsigned len, uint32_t dist)
{
    return LZR_MF_P_MATCH_CMD + lzr_mf_price_ladder(len - LZR_MF_LEN_MIN) +
           lzr_mf_price_dist(dist);
}

static unsigned lzr_mf_price_rep(unsigned len, int slot)
{
    unsigned p = LZR_MF_P_REP_CMD + (slot > 0 ? 1u : 0u);

    /* Commands 8..11 carry 2..17 directly; 12..15 use the ladder from base 18. */
    return p + (len <= 17u ? 4u : lzr_mf_price_ladder(len - 18u));
}

/* Cheaper per byte covered?  a is (price, length), likewise b. */
static int lzr_mf_cheaper(unsigned pa, unsigned la, unsigned pb, unsigned lb)
{
    return (uint64_t)pa * (uint64_t)lb < (uint64_t)pb * (uint64_t)la;
}

int lzr_mf_dist_class(uint32_t dist)
{
    /* Class 0 is 1..256; class c >= 1 is 2^(4c+4)+1 .. 2^(4c+8). */
    if (dist == 0u)
        return -1;
    if (dist <= 256u)
        return 0;
    return (int)((lzr_mf_flog2(dist - 1u) - 4u) / 4u);
}

/* ---------------------------------------------------------------------------
 * Hashing and layout
 * ------------------------------------------------------------------------- */
#define LZR_MF_HASH_BITS_MIN 12u
#define LZR_MF_HASH_BITS_MAX 22u
#define LZR_MF_PREV_MIN 4096u

static uint32_t lzr_mf_key2(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8);
}

static uint32_t lzr_mf_hash3(const uint8_t *p, unsigned bits)
{
    uint32_t v = (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16);

    return (v * 2654435761u) >> (32u - bits);
}

/* Saturates at 2^31 rather than wrapping to 0: v is bounded by
 * LZR_MF_WINDOW_LIMIT so the saturation is unreachable, but a shift that walked
 * off the end of a uint32_t would spin forever rather than fail. */
static uint32_t lzr_mf_ceilpow2(uint32_t v)
{
    uint32_t r = LZR_MF_PREV_MIN;

    while (r < v && r < ((uint32_t)1u << 31))
        r <<= 1;
    return r;
}

static unsigned lzr_mf_ideal_hash_bits(size_t input)
{
    unsigned b = LZR_MF_HASH_BITS_MIN;

    /* One bucket per position, within the clamps. */
    while (b < LZR_MF_HASH_BITS_MAX && ((size_t)1 << b) < input)
        b++;
    return b;
}

/* In uint64_t, not size_t, so that a window large enough to overflow a 32-bit
 * size_t is a value this can compare and reject rather than one it wraps.  The
 * bound is (2^16 + 2^22 + 2^31) * 4 < 2^34, so the arithmetic itself cannot
 * overflow whatever size_t is. */
static uint64_t lzr_mf_bytes(unsigned hash_bits, uint32_t nprev)
{
    return (((uint64_t)1 << 16) + ((uint64_t)1 << hash_bits) + (uint64_t)nprev) *
           (uint64_t)sizeof(uint32_t);
}

/* window == 0 means the default.  Anything above the limit is clamped there. */
static uint32_t lzr_mf_resolve_window(uint32_t window)
{
    if (window == 0u)
        return LZR_MF_WINDOW_MAX;
    if (window > LZR_MF_WINDOW_LIMIT)
        return LZR_MF_WINDOW_LIMIT;
    return window;
}

size_t lzr_mf_scratch_size_window(size_t max_input, uint32_t window)
{
    size_t cap = max_input, out;
    unsigned bits;
    uint32_t nprev, win;
    uint64_t bytes;

    if (max_input > LZR_MF_INPUT_MAX)
        return 0;
    win = lzr_mf_resolve_window(window);
    if (cap < LZR_MF_PREV_MIN)
        cap = LZR_MF_PREV_MIN;
    if (cap > (size_t)win)
        cap = (size_t)win;
    bits = lzr_mf_ideal_hash_bits(max_input);
    nprev = lzr_mf_ceilpow2((uint32_t)cap);
    /* Slack so the caller's buffer needs no particular alignment. */
    bytes = lzr_mf_bytes(bits, nprev) + (sizeof(uint32_t) - 1u);
    out = (size_t)bytes;
    if ((uint64_t)out != bytes)
        return 0; /* would not fit in a size_t; refuse rather than wrap */
    return out;
}

size_t lzr_mf_scratch_size(size_t max_input)
{
    return lzr_mf_scratch_size_window(max_input, 0);
}

/* ---------------------------------------------------------------------------
 * Init
 * ------------------------------------------------------------------------- */
void lzr_mf_init_params(lzr_mf *mf, void *scratch, size_t scratch_size,
                        const uint8_t *buf, size_t len,
                        const lzr_mf_params *params)
{
    const lzr_mf_preset *pr;
    unsigned char *base;
    size_t misalign, avail;
    uint64_t need;
    unsigned level, bits;
    uint32_t nprev, cap32, window;

    memset(mf, 0, sizeof *mf);
    mf->buf = buf;
    mf->len = len;

    level = params ? params->level : LZR_MF_LEVEL_DEFAULT;
    if (level > LZR_MF_LEVEL_EXACT)
        level = LZR_MF_LEVEL_EXACT;
    pr = &lzr_mf_presets[level];
    mf->level = level;
    mf->depth = (params && params->depth) ? params->depth : pr->depth;
    mf->nice_len = (params && params->nice_len) ? params->nice_len : pr->nice_len;
    if (mf->nice_len < LZR_MF_LEN_MIN)
        mf->nice_len = LZR_MF_LEN_MIN;
    mf->lazy = pr->lazy;
    mf->heur = pr->heur;
    if (params) {
        if (params->flags & LZR_MF_F_NO_LAZY)
            mf->lazy = 0;
        else if (params->flags & LZR_MF_F_LAZY)
            mf->lazy = 1;
        if (params->flags & LZR_MF_F_NO_HEUR)
            mf->heur = 0;
    }

    /* A window above LZR_MF_WINDOW_MAX is honoured, not clamped: the caller
     * asked for it explicitly, and the warning in matchfind.h says what that
     * means.  Only LZR_MF_WINDOW_LIMIT — the point past which no distance could
     * exist anyway — actually binds. */
    window = lzr_mf_resolve_window(params ? params->window : 0u);

    if (!buf || len == 0 || len > LZR_MF_INPUT_MAX || !scratch)
        return; /* inert; lzr_mf_valid() reports it */

    base = scratch;
    misalign = (size_t)((uintptr_t)base & (sizeof(uint32_t) - 1u));
    if (misalign) {
        size_t adj = sizeof(uint32_t) - misalign;

        if (scratch_size < adj)
            return;
        base += adj;
        scratch_size -= adj;
    }
    avail = scratch_size;

    /* The chain array only has to cover the positions a distance could reach:
     * min(len, window), rounded up.  A bigger window therefore costs scratch in
     * proportion — lzr_mf_scratch_size_window() computes the same figure. */
    cap32 = (uint32_t)((size_t)window < len ? (size_t)window : len);
    nprev = lzr_mf_ceilpow2(cap32);
    bits = lzr_mf_ideal_hash_bits(len);

    /* Shrink to fit rather than fail: first the hash table, then — only if it
     * still will not fit — the chain array, which shortens the window. */
    need = lzr_mf_bytes(bits, nprev);
    while (need > (uint64_t)avail && bits > LZR_MF_HASH_BITS_MIN) {
        bits--;
        need = lzr_mf_bytes(bits, nprev);
    }
    while (need > (uint64_t)avail && nprev > LZR_MF_PREV_MIN) {
        nprev >>= 1;
        need = lzr_mf_bytes(bits, nprev);
    }
    if (need > (uint64_t)avail)
        return; /* inert */

    /* A distance may not outrun the chain array: entries older than `nprev` are
     * aliased, so the window has to stop there.  Given scratch sized for the
     * window this never binds, since nprev >= min(len, window); it is what
     * turns a starved allocation into a shorter window rather than a wrong
     * answer, and what silently undoes a big window that was not paid for. */
    if (window > nprev)
        window = nprev;
    mf->window = window;

    mf->head2 = (uint32_t *)(void *)base;
    mf->head3 = mf->head2 + ((size_t)1 << 16);
    mf->prev = mf->head3 + ((size_t)1 << bits);
    mf->hash_bits = bits;
    mf->prev_mask = nprev - 1u;

    /* head2/head3 must start at 0 ("empty"); prev is only ever read for a
     * position that was inserted, which wrote it, but it is zeroed too so that a
     * mistake in that reasoning would be deterministic rather than not. */
    memset(base, 0, (size_t)need); /* need <= avail, so the cast is exact */
    mf->ins = 0;
    mf->valid = 1;
}

void lzr_mf_init(lzr_mf *mf, void *scratch, size_t scratch_size,
                 const uint8_t *buf, size_t len)
{
    lzr_mf_init_params(mf, scratch, scratch_size, buf, len, NULL);
}

int lzr_mf_valid(const lzr_mf *mf)
{
    return mf->valid;
}

/* ---------------------------------------------------------------------------
 * Indexing
 * ------------------------------------------------------------------------- */
static void lzr_mf_fill(lzr_mf *mf, size_t upto)
{
    const uint8_t *buf = mf->buf;
    size_t len = mf->len;

    if (upto > len)
        upto = len;
    while ((size_t)mf->ins < upto) {
        uint32_t p = mf->ins;

        if ((size_t)p + 2u <= len)
            mf->head2[lzr_mf_key2(buf + p)] = p + 1u;
        if ((size_t)p + 3u <= len) {
            uint32_t h = lzr_mf_hash3(buf + p, mf->hash_bits);

            mf->prev[p & mf->prev_mask] = mf->head3[h];
            mf->head3[h] = p + 1u;
        }
        mf->ins = p + 1u;
    }
}

void lzr_mf_skip(lzr_mf *mf, size_t pos, unsigned n)
{
    size_t upto;

    if (!mf->valid)
        return;
    upto = pos + n;
    if (upto < pos || upto > mf->len) /* wrap, or past the end: clamp */
        upto = mf->len;
    lzr_mf_fill(mf, upto);
}

/* ---------------------------------------------------------------------------
 * Matching.  The byte-run compare itself is lzr_mf_match_run (matchfind.h),
 * shared with the encoder's rep measurement.
 * ------------------------------------------------------------------------- */
unsigned lzr_mf_len_at(const lzr_mf *mf, size_t pos, uint32_t dist, unsigned cap)
{
    size_t avail;

    if (!mf->valid || pos >= mf->len)
        return 0;
    if (dist == 0u || (size_t)dist > pos || dist > mf->window)
        return 0;
    avail = mf->len - pos;
    if ((size_t)cap > avail)
        cap = (unsigned)avail;
    return lzr_mf_match_run(mf->buf + pos, mf->buf + pos - dist, cap);
}

int lzr_mf_short_rep(const lzr_mf *mf, size_t pos, const uint32_t reps[4])
{
    uint32_t d;

    if (!mf->valid || !reps || pos >= mf->len)
        return 0;
    d = reps[0];
    if (d == 0u || (size_t)d > pos || d > mf->window)
        return 0;
    return mf->buf[pos] == mf->buf[pos - d];
}

/* Fold one candidate position into the incumbent.  The rule is brute force's:
 * longer wins, and on a tie the smaller distance wins — which is what makes
 * LZR_MF_LEVEL_EXACT comparable to an exhaustive search regardless of the order
 * candidates arrive in.  The single byte-compare before the loop is the usual
 * filter: it rejects a candidate that cannot reach the length needed to win. */
static void lzr_mf_consider(const uint8_t *buf, size_t pos, uint32_t cp,
                            unsigned cap, unsigned *best, uint32_t *bdist)
{
    uint32_t d = (uint32_t)(pos - cp);
    unsigned need, l;

    if (*best == 0u)
        need = LZR_MF_LEN_MIN;
    else
        need = (d < *bdist) ? *best : *best + 1u;
    if (need > cap)
        return;
    if (buf[pos + need - 1u] != buf[cp + need - 1u])
        return;
    l = lzr_mf_match_run(buf + pos, buf + cp, cap);
    if (l >= LZR_MF_LEN_MIN && (l > *best || (l == *best && d < *bdist))) {
        *best = l;
        *bdist = d;
    }
}

typedef struct lzr_mf_cand {
    unsigned len;
    uint32_t dist;
    int slot;
} lzr_mf_cand;

static unsigned lzr_mf_price(const lzr_mf_cand *c)
{
    if (c->slot >= 0)
        return lzr_mf_price_rep(c->len, c->slot);
    return lzr_mf_price_fresh(c->len, c->dist);
}

/* The whole per-position search, minus the lazy lookahead.  Positions [0, pos)
 * must already be indexed, except that pos-1 may be missing — see step 2.  Takes
 * a mutable `mf` only to keep the statistics counters. */
static lzr_mf_cand lzr_mf_search(lzr_mf *mf, size_t pos,
                                 const uint32_t reps[4])
{
    const uint8_t *buf = mf->buf;
    lzr_mf_cand out;
    size_t avail = mf->len - pos;
    unsigned cap = avail > LZR_MF_LEN_MAX ? LZR_MF_LEN_MAX : (unsigned)avail;
    unsigned best = 0, rep_best = 0;
    uint32_t bdist = 0, win, limit;
    int i, rep_slot = -1, slot = -1;

    out.len = 0;
    out.dist = 0;
    out.slot = -1;
    if (cap < LZR_MF_LEN_MIN)
        return out;
    win = (uint32_t)(mf->window < pos ? mf->window : pos);
    if (win == 0u)
        return out;
    limit = (uint32_t)(pos - win); /* oldest usable position */

    /* 1. The rep set.  Cheapest to test, and seeding the incumbent length first
     *    makes the filter in lzr_mf_consider() fire more often below. */
    for (i = 0; i < 4; i++) {
        uint32_t d = reps[i];
        unsigned l;

        if (d == 0u || d > win)
            continue;
        l = lzr_mf_match_run(buf + pos, buf + pos - d, cap);
        if (l < LZR_MF_LEN_MIN)
            continue;
        if (l > best || (l == best && d < bdist)) {
            best = l;
            bdist = d;
        }
        if (l > rep_best) { /* strict: ties keep the cheaper, lower slot */
            rep_best = l;
            rep_slot = i;
        }
    }

    /* 2. Distance 1, always, without consulting a table.  This is what lets the
     *    lazy lookahead search pos+1 while the index still stops at pos: the one
     *    candidate it would otherwise be missing is position pos, and from pos+1
     *    that is exactly distance 1.  Keeping `pos` out of the tables matters
     *    because head2 has a single slot per key — inserting pos there would
     *    displace the nearest *earlier* two-byte match and lose it for good. */
    lzr_mf_consider(buf, pos, (uint32_t)(pos - 1u), cap, &best, &bdist);

    /* 3. The nearest two-byte match.  Exact key, so this is the complete answer
     *    for length 2 and a free extra candidate for anything longer. */
    if (best < mf->nice_len && (size_t)pos + 2u <= mf->len) {
        uint32_t v = mf->head2[lzr_mf_key2(buf + pos)];

        if (v != 0u) {
            uint32_t cp = v - 1u;

            if (cp < pos && cp >= limit)
                lzr_mf_consider(buf, pos, cp, cap, &best, &bdist);
        }
    }

    /* 4. The 3-byte chain, newest (nearest) first. */
    if (best < mf->nice_len && cap >= 3u && (size_t)pos + 3u <= mf->len) {
        uint32_t cur = mf->head3[lzr_mf_hash3(buf + pos, mf->hash_bits)];
        unsigned n = 0;

        while (cur != 0u && n < mf->depth) {
            uint32_t cp = cur - 1u;
            uint32_t nxt = mf->prev[cp & mf->prev_mask];

            /* A chain is strictly decreasing by construction.  Enforcing it
             * here means an aliased link — possible once the chain array wraps —
             * ends the walk instead of risking a cycle. */
            if (nxt != 0u && nxt - 1u >= cp)
                nxt = 0u;
            /* The index never runs ahead of the position being searched, so this
             * only fires for a caller that searched backwards — documented as
             * giving worse matches, but it must still never give an unsafe one. */
            if (cp < pos) {
                if (cp < limit)
                    break; /* decreasing, so everything below is out of window */
                n++;
                mf->stat_chain_steps++;
                lzr_mf_consider(buf, pos, cp, cap, &best, &bdist);
                if (best >= mf->nice_len)
                    break;
            }
            cur = nxt;
        }
    }

    if (best < LZR_MF_LEN_MIN)
        return out;

    /* Annotate: report the cheaper rep encoding whenever the distance we chose
     * happens to be in the rep set (lowest slot wins — it codes cheapest). */
    for (i = 0; i < 4; i++) {
        if (reps[i] == bdist) {
            slot = i;
            break;
        }
    }
    out.len = best;
    out.dist = bdist;
    out.slot = slot;

    if (!mf->heur)
        return out; /* level 0 / EXACT: the plain longest match, no judgement */

    /* Prefer a rep match that is cheaper per byte covered than the longest one.
     * This is where the format's four rep slots earn their keep: no distance
     * symbols at all, so a shorter rep frequently beats a longer fresh match. */
    if (rep_slot >= 0) {
        lzr_mf_cand r;

        r.len = rep_best;
        r.dist = reps[rep_slot];
        r.slot = rep_slot;
        if (r.slot != out.slot || r.len != out.len || r.dist != out.dist) {
            /* r.len <= out.len always, out being the longest, so a strict
             * price-per-byte win is the whole test. */
            if (lzr_mf_cheaper(lzr_mf_price(&r), r.len, lzr_mf_price(&out),
                               out.len))
                out = r;
        }
    }

    /* Not worth taking?  Literals cost LZR_MF_P_LIT bits each in this model. */
    if (lzr_mf_price(&out) >= LZR_MF_P_LIT * out.len) {
        out.len = 0;
        out.dist = 0;
        out.slot = -1;
    }
    return out;
}

static void lzr_mf_load_reps(uint32_t out[4], const uint32_t *reps)
{
    int i;

    for (i = 0; i < 4; i++)
        out[i] = reps ? reps[i] : 0u;
}

unsigned lzr_mf_find(lzr_mf *mf, size_t pos, const uint32_t reps[4],
                     uint32_t *dist, int *rep_slot)
{
    uint32_t rr[4];
    lzr_mf_cand c0;

    *dist = 0;
    *rep_slot = -1;
    if (!mf->valid || pos >= mf->len)
        return 0;
    lzr_mf_load_reps(rr, reps);

    lzr_mf_fill(mf, pos);
    mf->stat_searches++;
    c0 = lzr_mf_search(mf, pos, rr);
    if (c0.len == 0u)
        return 0;

    /* Lazy matching: would a literal here plus the match at pos+1 code for less
     * per byte covered than the match here?  If so, ask for the literal.
     *
     * The lookahead result is deliberately *not* cached for the next call.  It
     * runs with the index still at `pos` (step 2 of lzr_mf_search explains why
     * it has to), whereas a real search at pos+1 will have indexed pos as well;
     * at bounded depth those two can settle on different candidates.  Caching it
     * would therefore make the answer at pos+1 depend on how the caller arrived
     * there.  Recomputing costs one extra search per literal the lazy path asks
     * for, and keeps lzr_mf_find a pure function of (pos, reps). */
    if (mf->lazy && c0.len < mf->nice_len && pos + 1u < mf->len) {
        lzr_mf_cand c1;

        mf->stat_lookaheads++;
        c1 = lzr_mf_search(mf, pos + 1u, rr);
        if (c1.len >= LZR_MF_LEN_MIN &&
            lzr_mf_cheaper(LZR_MF_P_LIT + lzr_mf_price(&c1), c1.len + 1u,
                           lzr_mf_price(&c0), c0.len)) {
            mf->stat_lazy_skips++;
            return 0;
        }
    }

    *dist = c0.dist;
    *rep_slot = c0.slot;
    return c0.len;
}
