/* SPDX-License-Identifier: 0BSD */
/* lzraven — the encoder: stream framing, stored blocks, and entropy-coded blocks
 * with the full symbol grammar of SPEC.md §5 (literals, matches, repeats).
 *
 * What this produces
 * ------------------
 * Two kinds of block, chosen per 512 KiB unit by whichever comes out smaller:
 *
 *   - stored (flag bit 0): the payload is the plaintext, which the decoder
 *     memcpys without entering the entropy coder.  Always available, so every
 *     input has a valid encoding.
 *   - coded: a greedy-plus-lazy LZ77 parse over the match finder in
 *     src/matchfind.h, emitting literals, matched literals, matches at a fresh
 *     distance and matches at any of the four repeat distances.
 *
 * The one thing that must be right: order
 * ---------------------------------------
 * rANS codes symbols **backwards** — the encoder walks the symbol stream in
 * reverse and writes words at a decreasing index, which is exactly what the
 * decoder's forward word stream reads back (SPEC.md §4.1).  But the probability
 * model adapts **forwards**, in decode order (SPEC.md §4.2).  One pass cannot
 * satisfy both: the (cum, freq) pair a symbol must be coded with depends on
 * every symbol *before* it, while the coding itself can only happen after every
 * symbol *after* it has been coded.
 *
 * So a block is encoded in two passes:
 *
 *   1. forward (`lzr_enc_forward`) — parse the block and walk the symbols in
 *      decode order, driving the decoder's own model and CDF kernel, and record
 *      the (cum, freq, slot) triple each symbol must be coded with.
 *   2. backward (`lzr_enc_backward`) — replay those records in reverse, coding
 *      each into its rANS state and emitting words at a decreasing index.
 *
 * Pass 1 shares the decoder's adaptation rather than reimplementing it: the CDF
 * kernel from src/cdf.h and the binary update from src/rans.h.  That is not a
 * tidiness point.  A private copy of the adaptation that differs in one shift
 * desynchronises on the first adaptive symbol, and the failure presents as a
 * coder bug rather than a model bug.  For the same reason the state machine and
 * the literal-context table come from src/grammar.h, which the decoder includes
 * too: an encoder that made one different transition would emit a stream that is
 * framed correctly and decodes to different bytes, with no framing error
 * anywhere to catch it.
 *
 * Interleaving.  The decoder takes each read from the next of eight rANS states,
 * round-robin (`next++ & 7`).  Pass 1 records which slot each read will land on,
 * so pass 2 needs no assumption about the rotation: it feeds the states in
 * exactly the reverse of the order the decoder will drain them.
 *
 * Two further details, each of which silently produces an undecodable stream if
 * missed:
 *
 *   - The last 32 bytes of a coded block are not entropy-coded (SPEC.md §4.4).
 *     The backward pass therefore *starts* by seeding the eight states with
 *     `(1 << 32) | word` from those 32 bytes, so the tail falls out of the states
 *     when the coded region is exhausted rather than being coded at all.  Seeding
 *     that way also satisfies the state-drain invariant — `x >> 32 == 1` on every
 *     state at the end of the block — for free.  Our decoder enforces that
 *     invariant and Apple's does not; every block Apple's encoder has produced
 *     for us satisfies it, so meeting it costs nothing and keeps our streams the
 *     same shape as theirs.
 *   - The block prologue is coded *last*.  The decoder reads the block's length
 *     and seven of the eight initial states out of state 0 before any symbol, so
 *     in reverse those are the final things written; state 0 is then flushed
 *     whole, as the two words at the very front of the payload.
 *
 * Renormalisation.  The decoder refills a state lazily, at the top of every read
 * (`lzr_renorm`).  The encoder mirrors it: before coding a symbol it flushes one
 * word if the state would otherwise overflow 64 bits, which is precisely the
 * point at which the decoder will refill.  Both sides keep every state in
 * [2^32, 2^64), which is what makes the two decisions agree with nothing
 * signalled either way.  A useful consequence: a correct encode leaves the word
 * stream exactly consumed and every state drained, both of which our decoder
 * checks — so a mis-ordered encode is rejected rather than quietly accepted.
 *
 * The parse
 * ---------
 * Greedy, with a one-position lazy lookahead, priced against the *live* model
 * rather than a static table: `lzr_pr_*` read the CDFs the forward pass is
 * adapting and cost a candidate in 256ths of a bit.  The match finder is
 * therefore asked for the raw longest match (LZR_MF_F_NO_HEUR / _NO_LAZY) and
 * every judgement — rep versus fresh, match versus literal, and the lazy skip —
 * is made here, where the real probabilities are.  The four rep distances are
 * tested by direct byte-compare rather than through the finder, so a rep older
 * than the finder's current view is still usable.
 *
 * Pricing doubles as a validity check.  A symbol whose frequency has adapted to
 * zero cannot be coded at all; `lzr_pr_freq` prices that as unaffordable, so a
 * candidate needing one is never chosen and the forward pass's own freq != 0
 * check stays a backstop rather than a block-abandoning event.
 *
 * That the live model beats the finder's static one is measured, not assumed.
 * On the 94 MB of real OTA plaintext, against Apple's own encoder at 0.0334:
 *
 *   finder's heuristics, its own lazy matching        0.0361    76 MB/s
 *   live pricing, no lazy lookahead                   0.0376    94 MB/s
 *   live pricing + lazy lookahead                     0.0347    61 MB/s
 *
 * Chain depth is the other axis, and it is the encoder's compression level:
 * `lzraven_encode_params.level` picks the finder's preset, from depth 2 to depth
 * 1024.  The default stays at 6 because that is the knee — level 8 buys 0.0341
 * for a third of the speed and level 9 buys 0.0340 for a fifteenth of it — but
 * the whole range is a caller's to choose, and the table is in lzraven.h.
 *
 * What is *not* here is an optimal parse.  Greedy-plus-lazy takes each decision
 * on local prices, and the ~4% still separating us from Apple is concentrated in
 * text-like content, where the right choice at one position depends on the whole
 * parse after it.  That needs a dynamic program over a multi-candidate finder,
 * and this one reports a single match per position.
 */
#include "encode.h"

#include <string.h>

#include "grammar.h"
#include "internal.h"
#include "matchfind.h"
#include "rans.h"

/* The forward pass adapts with the decoder's CDF kernel.  Which kernel is
 * immaterial — every dispatch path is required to be bit-identical (src/cdf.h) —
 * so the encoder always takes the scalar one and needs no run-time dispatch. */
#define LZR_KERNEL LZR_K_SCALAR
#include "cdf.h"

/* Longest length the two rep command families can carry: base 18 plus the
 * ladder's 330 (SPEC.md §5.2, §5.3).  Sixteen more than a fresh-distance match,
 * and more than the finder will report, which is why reps are measured here. */
#define LZR_ENC_REP_LEN_MAX (18u + LZR_LADDER_MAX)

/* A rep this long already covers the position well, so the hash-chain search is
 * skipped entirely.  Pure speed/ratio trade, measured rather than guessed: at
 * the default level it costs nothing measurable on the real OTA corpus (0.0347
 * either way) and saves the walk over exactly the runs where the walk is most
 * expensive.
 *
 * The figures are the finder's own nice-lengths, the length at which it stops
 * searching, spelled from the same names (LZR_MF_NICE_*, matchfind.h) as its
 * preset table so that the two decisions are the same decision and raising the
 * level raises both together.  A level that searches deeper should not be
 * undercut by a caller-side early exit calibrated for level 6. */
static const uint16_t lzr_enc_nice[10] = {
    LZR_MF_NICE_0, LZR_MF_NICE_1, LZR_MF_NICE_2, LZR_MF_NICE_3, LZR_MF_NICE_4,
    LZR_MF_NICE_5, LZR_MF_NICE_6, LZR_MF_NICE_7, LZR_MF_NICE_8, LZR_MF_NICE_9
};
_Static_assert(sizeof lzr_enc_nice / sizeof lzr_enc_nice[0] ==
                   LZRAVEN_ENCODE_LEVEL_MAX + 1,
               "one early-exit length per encoder level");

/* ---------------------------------------------------------------------------
 * Records.
 *
 * One u32 per read, in decode order:
 *
 *   bit 31      1 = a raw bypass field, 0 = a modelled symbol
 *   bits 30:28  the rANS slot this read comes from
 *   modelled    bits 27:14 = freq - 1, bits 13:0 = cum
 *   bypass      bits 5:0   = field width in bits; the value is the *preceding*
 *               record, stored raw, which the reverse walk steps back onto
 *
 * cum is in [0, 16353] and freq in [31, 16353] for every symbol this format can
 * produce — the binary update saturates at 31 either side of 2^14 and the CDF
 * update keeps 32 of headroom per lane (SPEC.md §4.2) — so both fit their
 * fields.  Storing freq - 1 also makes freq == 0, and hence a division by zero
 * in the backward pass, unrepresentable.
 * ------------------------------------------------------------------------- */
#define LZR_REC_BYPASS 0x80000000u

typedef struct lzr_fwd {
    lzr_model *m;
    uint32_t *rec;
    size_t nrec;
    unsigned next; /* the decoder's round-robin counter */
    int err;
} lzr_fwd;

static void lzr_rec_push(lzr_fwd *f, uint32_t v)
{
    if (f->nrec >= LZR_ENC_RECS) {
        f->err = 1;
        return;
    }
    f->rec[f->nrec++] = v;
}

/* One adaptive 16-ary symbol.  Both the (cum, freq) pair and the adaptation come
 * out of the decoder's own kernel: feeding it x = cum makes its search return
 * this symbol, since cum <= x < cum + freq, so it publishes the pair the decoder
 * will see and then nudges all 16 lanes exactly as the decoder will. */
static void lzr_fwd_sym(lzr_fwd *f, uint16_t *cdf, unsigned s, unsigned rate)
{
    unsigned slot = f->next++ & 7;
    uint32_t cum = (uint32_t)(cdf[(s + 15) & 15] & 0x3FFF);
    uint32_t got_cum = 0, freq = 0;
    unsigned got = lzr_cdf_step(cdf, cum, rate, &got_cum, &freq);

    /* True for any monotone CDF, which the update provably preserves — checked
     * rather than assumed, because a CDF that had lost monotonicity would code
     * some other symbol and the block would decode to different bytes.  Failing
     * here abandons the block and stores it instead. */
    if (got != s || got_cum != cum || freq == 0 || freq > LZR_M) {
        f->err = 1;
        return;
    }
    lzr_rec_push(f, ((uint32_t)slot << 28) | ((freq - 1) << 14) | cum);
}

/* One adaptive binary symbol.  `lzr_bit_apply` is the decoder's own update
 * (src/rans.h), shared rather than copied. */
static void lzr_fwd_bit(lzr_fwd *f, uint16_t *probs, unsigned k, unsigned b)
{
    unsigned slot = f->next++ & 7;
    uint32_t cum = 0, freq = 0;

    lzr_bit_apply(probs, k, b, &cum, &freq);
    lzr_rec_push(f, ((uint32_t)slot << 28) | ((freq - 1) << 14) | cum);
}

/* One raw bypass field of n uniform bits.  Value first, tag second, so the
 * reverse walk meets the tag and can then step back onto the value. */
static void lzr_fwd_bypass(lzr_fwd *f, unsigned n, uint32_t v)
{
    unsigned slot = f->next++ & 7;

    lzr_rec_push(f, v);
    lzr_rec_push(f, LZR_REC_BYPASS | ((uint32_t)slot << 28) | n);
}

/* ---------------------------------------------------------------------------
 * Shared arithmetic: the length ladder's and the distance field's closed forms,
 * inverted (SPEC.md §5.3, §5.4).
 * ------------------------------------------------------------------------- */

/* The exponent k with 2^k <= v < 2^(k+1), for v >= 1.  The decoder reads state
 * high halves and block lengths as a 5-bit exponent plus a k-bit mantissa, and
 * the distance field's leading term is the same idea. */
static unsigned lzr_exp(uint64_t v)
{
    unsigned k = 0;

    while (v >> (k + 1))
        k++;
    return k;
}

/* The ladder, inverted: `v` is the length minus its base (2 for a match, 18 for
 * a long rep) and is in [0, LZR_LADDER_MAX].  Rungs 0..9 and 15 carry v on their
 * own; the rest split it as 16*hi + s2 - 150 with hi in [10, 29], reached
 * directly for hi <= 13 and through the esc14 rung above that.  hi is therefore
 * (v + 150) / 16, which is what the decoder's `16 * hi + s2 - 150` inverts to. */
typedef struct lzr_ladder_code {
    unsigned rung1; /* the first symbol, 0..15 */
    unsigned hi;    /* 10..29 when rung1 is 10..14, else unused */
    unsigned s2;    /* the trailing symbol, when there is one */
    int split;      /* 0 = rung1 alone */
} lzr_ladder_code;

static void lzr_ladder_encode(unsigned v, lzr_ladder_code *o)
{
    o->hi = 0;
    o->s2 = 0;
    o->split = 0;
    if (v <= 9u) {
        o->rung1 = v;
        return;
    }
    if (v >= LZR_LADDER_MAX) { /* the saturating rung carries nothing else */
        o->rung1 = 15u;
        return;
    }
    o->hi = (v + 150u) >> 4;  /* 10..29 */
    o->s2 = (v + 150u) & 15u;
    o->rung1 = o->hi <= 13u ? o->hi : 14u;
    o->split = 1;
}

/* The distance field, inverted.  Class 0 (distances 1..256) is two nibbles and
 * nothing else; above it the leading term puts dist - 1 in [2^(kk+4), 2^(kk+5))
 * with kk = 4*class + (d1 >> 2), and the low bits split into a (kk-2)-bit bypass
 * field and the final nibble.  `cls` is the command byte itself. */
typedef struct lzr_dist_code {
    unsigned cls;
    unsigned d1, d2;
    unsigned nbits; /* bypass width; 0 for class 0 */
    uint32_t extra;
} lzr_dist_code;

static void lzr_dist_encode(uint32_t dist, lzr_dist_code *o)
{
    uint32_t u = dist - 1u;
    unsigned kk;

    if (u < 256u) {
        o->cls = 0;
        o->d1 = (unsigned)(u >> 4);
        o->d2 = (unsigned)(u & 15u);
        o->nbits = 0;
        o->extra = 0;
        return;
    }
    kk = lzr_exp(u) - 4u; /* >= 4, since u >= 256 */
    o->cls = kk >> 2;
    o->d1 = ((kk & 3u) << 2) | (unsigned)((u >> (kk + 2u)) & 3u);
    o->d2 = (unsigned)(u & 15u);
    o->nbits = kk - 2u;
    o->extra = (u >> 4) & (((uint32_t)1 << (kk - 2u)) - 1u);
}

/* ---------------------------------------------------------------------------
 * Prices, in 256ths of a bit.
 *
 * A symbol coded with frequency f out of 2^14 costs -log2(f / 2^14); the encoder
 * only ever compares candidates, so five fractional bits of log2 is ample and
 * the table below is the whole of it.  Everything is compared as
 * price * other_length against other_price * length, i.e. per byte covered.
 * ------------------------------------------------------------------------- */

/* Unaffordable.  Chosen so that a sum of every symbol a command could need
 * stays inside a u32 while still dominating any real price (the dearest real
 * symbol is 14 bits, or 3584). */
#define LZR_PRICE_INF 0x00100000u

/* round(256 * log2(1 + i / 32)). */
static const uint16_t lzr_log2_frac[32] = {
      0,  11,  22,  33,  44,  54,  63,  73,
     82,  92, 100, 109, 118, 126, 134, 142,
    150, 157, 165, 172, 179, 186, 193, 200,
    207, 213, 220, 226, 232, 238, 244, 250
};

/* -log2(f / 2^14).  f == 0 is a symbol the model has adapted out of existence:
 * it cannot be coded at all, so it prices as unaffordable and every candidate
 * that would need it is discarded before the forward pass ever sees it. */
static uint32_t lzr_pr_freq(uint32_t f)
{
    unsigned e;

    if (f == 0u || f > LZR_M)
        return LZR_PRICE_INF;
    e = lzr_exp(f);
    return (14u - e) * 256u - lzr_log2_frac[((f << 5) >> e) - 32u];
}

/* The (cum, freq) split is the kernel's, so that a symbol priced as affordable is
 * exactly one the kernel will code (src/cdf.h). */
static uint32_t lzr_pr_sym(const uint16_t *cdf, unsigned s)
{
    uint32_t lo = (uint32_t)(cdf[(s + 15) & 15] & 0x3FFF);

    return lzr_pr_freq((uint32_t)(uint16_t)(cdf[s] - lo));
}

static uint32_t lzr_pr_bit(const uint16_t *probs, unsigned k, unsigned b)
{
    uint32_t p = probs[k];

    return lzr_pr_freq(b ? LZR_M - p : p);
}

static uint32_t lzr_pr_ladder(const uint16_t *rung1, const uint16_t *esc10,
                              const uint16_t *esc14, const uint16_t *rung2,
                              unsigned v)
{
    lzr_ladder_code l;
    uint32_t p;

    lzr_ladder_encode(v, &l);
    p = lzr_pr_sym(rung1, l.rung1);
    if (!l.split)
        return p;
    if (l.hi == 10u)
        return p + lzr_pr_sym(esc10, l.s2);
    if (l.hi <= 13u)
        return p + lzr_pr_sym(rung2, l.s2);
    return p + lzr_pr_sym(esc14, l.hi - 14u) + lzr_pr_sym(rung2, l.s2);
}

/* One literal at `pos`, in the contexts `state` selects.  `state` is a parameter
 * rather than m->state so the lazy lookahead can price position pos + 1 against
 * the state a literal at pos would leave behind. */
static uint32_t lzr_pr_lit(const lzr_model *m, const uint8_t *src, size_t pos,
                           unsigned state)
{
    unsigned ps = (unsigned)(pos & 3u);
    unsigned t2 = (unsigned)src[pos - 1] >> 6;
    unsigned b = src[pos], hi = b >> 4, lo = b & 15u;
    unsigned lam = lzr_lit_lam_ctx[state];
    uint32_t p = lzr_pr_bit(m->is_match[state], ps, 1);

    if (lam <= 2u) { /* matched literal: contexted on the byte one rep0 back */
        uint32_t rep0 = m->reps[0];
        unsigned mb;

        if (rep0 == 0u || (size_t)rep0 > pos)
            return LZR_PRICE_INF; /* unreachable; priced out rather than
                                   * trusted, and lzr_fwd_lit re-checks it
                                   * before dereferencing */
        mb = src[pos - rep0];
        p += lzr_pr_sym(m->lam_hi[lam][t2][mb >> 4], hi);
        /* A high-nibble miss falls back to the *plain* literal low CDF, not the
         * matched one (SPEC.md §5.1). */
        if (hi == (unsigned)(mb >> 4))
            p += lzr_pr_sym(m->lam_lo[lam][mb & 15u], lo);
        else
            p += lzr_pr_sym(m->lit_lo[ps][t2][hi], lo);
    } else {
        p += lzr_pr_sym(m->lit_hi[ps][t2], hi);
        p += lzr_pr_sym(m->lit_lo[ps][t2][hi], lo);
    }
    return p;
}

/* A candidate command: a length with either a fresh distance or a rep slot. */
typedef struct lzr_cand {
    unsigned len;
    uint32_t dist;
    int slot; /* -1 = fresh distance, else 0..3 */
} lzr_cand;

static uint32_t lzr_pr_cand(const lzr_model *m, unsigned state, unsigned ps,
                            const lzr_cand *c)
{
    uint32_t p = lzr_pr_bit(m->is_match[state], ps, 0);

    if (c->slot < 0) {
        lzr_dist_code d;

        lzr_dist_encode(c->dist, &d);
        return p + lzr_pr_sym(m->cmd[state], d.cls) +
               lzr_pr_ladder(m->mat_len1[d.cls][ps], m->mat_len_esc10,
                             m->mat_len_esc14, m->mat_len2[ps], c->len - 2u) +
               lzr_pr_sym(m->dist[d.cls][0], d.d1) +
               lzr_pr_sym(m->dist[d.cls][1], d.d2) + 256u * d.nbits;
    }
    if (c->len == 1u) /* command 7: rep0, length 1, no length symbol */
        return p + lzr_pr_sym(m->cmd[state], 7u);
    if (c->len <= 17u)
        return p + lzr_pr_sym(m->cmd[state], 8u + (unsigned)c->slot) +
               lzr_pr_sym(m->rep_short_len[ps], c->len - 2u);
    return p + lzr_pr_sym(m->cmd[state], 12u + (unsigned)c->slot) +
           lzr_pr_ladder(m->rep_len1[ps], m->rep_len_esc10, m->rep_len_esc14,
                         m->rep_len2[ps], c->len - 18u);
}

/* ---------------------------------------------------------------------------
 * The forward pass: emit the symbols of one command, in decode order.
 * ------------------------------------------------------------------------- */

static void lzr_fwd_ladder(lzr_fwd *f, uint16_t *rung1, uint16_t *esc10,
                           uint16_t *esc14, uint16_t *rung2, unsigned v)
{
    lzr_ladder_code l;

    lzr_ladder_encode(v, &l);
    lzr_fwd_sym(f, rung1, l.rung1, 6);
    if (!l.split)
        return;
    if (l.hi == 10u) {
        lzr_fwd_sym(f, esc10, l.s2, 6);
    } else if (l.hi <= 13u) {
        lzr_fwd_sym(f, rung2, l.s2, 6);
    } else {
        lzr_fwd_sym(f, esc14, l.hi - 14u, 6);
        lzr_fwd_sym(f, rung2, l.s2, 6);
    }
}

static void lzr_fwd_lit(lzr_fwd *f, const uint8_t *src, size_t pos)
{
    lzr_model *m = f->m;
    unsigned state = m->state;
    unsigned ps = (unsigned)(pos & 3u);
    unsigned t2 = (unsigned)src[pos - 1] >> 6;
    unsigned b = src[pos], hi = b >> 4, lo = b & 15u;
    unsigned lam = lzr_lit_lam_ctx[state];

    lzr_fwd_bit(f, m->is_match[state], ps, 1);
    if (lam <= 2u) {
        uint32_t rep0 = m->reps[0];
        unsigned mb;

        /* 0 < rep0 <= pos holds by induction — the reps only ever take values
         * of emitted match distances, each checked against its position when
         * chosen — but the pricer refuses to trust that (lzr_pr_lit prices
         * this path unreachable instead), and the pass that dereferences must
         * not trust more than the pass that prices: src[pos - rep0] outside
         * the buffer is the one out-of-bounds read a broken parse could
         * produce, and the planned optimal-parse rewrite is exactly the kind
         * of change that could break the induction.  Fail the block; it is
         * stored instead, like every other f->err. */
        if (rep0 == 0u || (size_t)rep0 > pos) {
            f->err = 1;
            return;
        }
        mb = src[pos - rep0];

        lzr_fwd_sym(f, m->lam_hi[lam][t2][mb >> 4], hi, 6);
        if (hi == (unsigned)(mb >> 4))
            lzr_fwd_sym(f, m->lam_lo[lam][mb & 15u], lo, 6);
        else
            lzr_fwd_sym(f, m->lit_lo[ps][t2][hi], lo, 7);
    } else {
        lzr_fwd_sym(f, m->lit_hi[ps][t2], hi, 7);
        lzr_fwd_sym(f, m->lit_lo[ps][t2][hi], lo, 7);
    }
    m->state = (uint8_t)lzr_next_state(16, state);
}

/* A match at a fresh distance: the command *is* the distance class, and the
 * bypass field comes after *both* modelled distance symbols (SPEC.md §5.4). */
static void lzr_fwd_match(lzr_fwd *f, unsigned ps, unsigned len, uint32_t dist)
{
    lzr_model *m = f->m;
    unsigned state = m->state;
    lzr_dist_code d;

    lzr_dist_encode(dist, &d);
    lzr_fwd_bit(f, m->is_match[state], ps, 0);
    lzr_fwd_sym(f, m->cmd[state], d.cls, 6);
    lzr_fwd_ladder(f, m->mat_len1[d.cls][ps], m->mat_len_esc10,
                   m->mat_len_esc14, m->mat_len2[ps], len - 2u);
    lzr_fwd_sym(f, m->dist[d.cls][0], d.d1, 7);
    lzr_fwd_sym(f, m->dist[d.cls][1], d.d2, 7);
    if (d.cls)
        lzr_fwd_bypass(f, d.nbits, d.extra);

    m->reps[3] = m->reps[2];
    m->reps[2] = m->reps[1];
    m->reps[1] = m->reps[0];
    m->reps[0] = dist;
    m->state = (uint8_t)lzr_next_state(d.cls, state);
}

/* A repeat: command 7 (rep0, length 1), 8..11 (length 2..17 direct) or 12..15
 * (length 18..348 through the ladder).  Commands 8..15 move the slot they used
 * to the front, LZMA-style; command 7 already is the front and moves nothing. */
static void lzr_fwd_rep(lzr_fwd *f, unsigned ps, unsigned len, int slot)
{
    lzr_model *m = f->m;
    unsigned state = m->state;
    uint32_t dist = m->reps[slot];
    unsigned cmd;

    lzr_fwd_bit(f, m->is_match[state], ps, 0);
    if (len == 1u) {
        cmd = 7u;
        lzr_fwd_sym(f, m->cmd[state], cmd, 6);
    } else if (len <= 17u) {
        cmd = 8u + (unsigned)slot;
        lzr_fwd_sym(f, m->cmd[state], cmd, 6);
        lzr_fwd_sym(f, m->rep_short_len[ps], len - 2u, 6);
    } else {
        cmd = 12u + (unsigned)slot;
        lzr_fwd_sym(f, m->cmd[state], cmd, 6);
        lzr_fwd_ladder(f, m->rep_len1[ps], m->rep_len_esc10, m->rep_len_esc14,
                       m->rep_len2[ps], len - 18u);
    }
    if (cmd != 7u && slot) {
        int j;

        for (j = slot; j > 0; j--)
            m->reps[j] = m->reps[j - 1];
        m->reps[0] = dist;
    }
    m->state = (uint8_t)lzr_next_state(cmd, state);
}

/* ---------------------------------------------------------------------------
 * The parse.
 * ------------------------------------------------------------------------- */

typedef struct lzr_parse {
    lzr_fwd *f;
    lzr_mf *mf;
    size_t base; /* absolute position the finder's view starts at */
    const uint8_t *src;
    size_t coded_end; /* no command may write past this; landing exactly on
                       * it is legal and common (SPEC.md §5.5) */
    unsigned nice;    /* lzr_enc_nice[] for the level in force */
} lzr_parse;

/* Fold one candidate into the incumbent, cheapest per byte covered wins. */
static void lzr_enc_fold(const lzr_model *m, unsigned state, unsigned ps,
                         const lzr_cand *c, lzr_cand *best, uint32_t *best_p)
{
    uint32_t p = lzr_pr_cand(m, state, ps, c);

    if (p >= LZR_PRICE_INF)
        return;
    if (best->len == 0u ||
        (uint64_t)p * best->len < (uint64_t)*best_p * c->len) {
        *best = *c;
        *best_p = p;
    }
}

/* The cheapest command at `pos`, or `out->len == 0` if there is none.  Returns
 * its price.  `state` is passed in for the same reason as in lzr_pr_lit.
 *
 * No candidate may run past `coded_end`: Apple's decoder rejects a block whose
 * last match overshoots the uncoded tail rather than truncating it (SPEC.md
 * §5.5), so the caps below are the rule, not a convenience. */
static uint32_t lzr_enc_best(lzr_parse *pp, size_t pos, unsigned state,
                             lzr_cand *out)
{
    const lzr_model *m = pp->f->m;
    const uint8_t *src = pp->src;
    unsigned ps = (unsigned)(pos & 3u);
    size_t room = pp->coded_end - pos;
    unsigned cap_rep = room < LZR_ENC_REP_LEN_MAX ? (unsigned)room
                                                  : LZR_ENC_REP_LEN_MAX;
    unsigned cap_mat = room < LZR_MF_LEN_MAX ? (unsigned)room : LZR_MF_LEN_MAX;
    uint32_t best_p = LZR_PRICE_INF;
    unsigned longest_rep = 0;
    int i;

    out->len = 0;
    out->dist = 0;
    out->slot = -1;

    /* 1. The four rep distances, measured directly against the input.  Going
     *    through the finder would restrict them to its current view, and a rep
     *    costs no distance symbols at all — it is the cheapest thing the grammar
     *    has, and worth keeping reachable however old it is. */
    for (i = 0; i < 4; i++) {
        uint32_t d = m->reps[i];
        lzr_cand c;
        unsigned l;
        int j;

        if (d == 0u || (size_t)d > pos)
            continue;
        for (j = 0; j < i; j++)
            if (m->reps[j] == d)
                break;
        if (j < i)
            continue; /* a lower slot holds the same distance and codes cheaper */
        if (src[pos] != src[pos - d])
            continue;
        l = 1u + lzr_mf_match_run(src + pos + 1, src + pos - d + 1, cap_rep - 1u);
        if (l > longest_rep)
            longest_rep = l;
        c.dist = d;
        c.slot = i;
        if (i == 0) { /* command 7 — the only length-1 command in the grammar */
            c.len = 1u;
            lzr_enc_fold(m, state, ps, &c, out, &best_p);
        }
        if (l >= LZR_MF_LEN_MIN) {
            c.len = l;
            lzr_enc_fold(m, state, ps, &c, out, &best_p);
        }
    }

    /* 2. The longest match at a fresh distance.  Skipped when a rep already
     *    covers this position well — the finder makes the same early exit
     *    internally, and this one also saves the walk. */
    if (longest_rep < pp->nice && cap_mat >= LZR_MF_LEN_MIN) {
        uint32_t d = 0;
        int slot = -1;
        unsigned l = lzr_mf_find(pp->mf, pos - pp->base, m->reps, &d, &slot);

        if (l > cap_mat)
            l = cap_mat;
        /* slot >= 0 means the finder landed on a rep distance, which step 1 has
         * already priced — as a rep, which is strictly cheaper.
         *
         * There is no filter on the distance class here: whatever the finder
         * reports is inside the window it was given, and the window is what
         * decides which classes are reachable.  At the default 8 MiB every
         * distance is in classes 0..4, exactly as Apple's own encoder's are; a
         * caller that raised the window past 16 MiB asked for classes 5 and 6
         * and is told what that means at lzraven_encode_params.window. */
        if (l >= LZR_MF_LEN_MIN && slot < 0) {
            lzr_cand c;

            c.len = l;
            c.dist = d;
            c.slot = -1;
            lzr_enc_fold(m, state, ps, &c, out, &best_p);
        }
    }
    return best_p;
}

/* Walk the block, in decode order, choosing and emitting one command or literal
 * at a time.  `src` is the whole plaintext and `pos` is absolute, because every
 * context in this grammar is: posState is `pos & 3`, the literal context is the
 * previous *output* byte, and a match may reach into any earlier block. */
static int lzr_enc_forward(lzr_fwd *f, lzr_mf *mf, size_t base,
                           const uint8_t *src, size_t off, size_t size,
                           unsigned nice)
{
    lzr_parse pp;
    size_t pos = off;

    pp.f = f;
    pp.mf = mf;
    pp.base = base;
    pp.src = src;
    pp.coded_end = off + size - LZR_TAIL;
    pp.nice = nice;

    /* The stream's very first byte has no previous byte to context on, so the
     * decoder takes it as eight raw bits (src/block_body.h). */
    if (pos == 0) {
        lzr_fwd_bypass(f, 8, src[0]);
        pos = 1;
    }

    while (pos < pp.coded_end) {
        unsigned state = f->m->state;
        unsigned ps = (unsigned)(pos & 3u);
        lzr_cand c;
        uint32_t cost, lit;

        cost = lzr_enc_best(&pp, pos, state, &c);
        lit = lzr_pr_lit(f->m, src, pos, state);

        /* Worth taking at all?  A command has to beat coding the bytes it covers
         * as literals, at the model's current prices. */
        if (c.len != 0u && (uint64_t)cost >= (uint64_t)lit * c.len)
            c.len = 0;

        /* Lazy: a literal here plus the best command at pos + 1 may cover more
         * ground per bit than the command here.  The lookahead is priced against
         * the state a literal would leave behind; reps are unchanged by one. */
        if (c.len != 0u && pos + 1u < pp.coded_end) {
            lzr_cand c1;
            uint32_t cost1 = lzr_enc_best(&pp, pos + 1u,
                                          lzr_next_state(16, state), &c1);

            if (c1.len != 0u &&
                (uint64_t)(lit + cost1) * c.len <
                    (uint64_t)cost * (c1.len + 1u))
                c.len = 0;
        }

        if (c.len == 0u) {
            lzr_fwd_lit(f, src, pos);
            pos++;
        } else if (c.slot < 0) {
            lzr_fwd_match(f, ps, c.len, c.dist);
            pos += c.len;
        } else {
            lzr_fwd_rep(f, ps, c.len, c.slot);
            pos += c.len;
        }
        if (f->err)
            return 0;
    }
    return 1;
}

/* ---------------------------------------------------------------------------
 * The backward pass — the rANS encoder proper.
 * ------------------------------------------------------------------------- */
typedef struct lzr_bwd {
    uint64_t st[8];
    uint8_t *w;
    size_t wi; /* next word goes at --wi; 0 means the block's budget is spent */
    int err;
} lzr_bwd;

static void lzr_bwd_word(lzr_bwd *e, uint32_t v)
{
    if (e->wi == 0) {
        e->err = 1;
        return;
    }
    lzr_wr32(e->w + 4 * --e->wi, v);
}

/* The inverse of `lzr_sym` / `lzr_bit`: q = x / freq, r = x - q * freq,
 * x' = (q << 14) + r + cum (SPEC.md §4.1).  The flush test is written as a shift
 * of x rather than as `x >= freq << 50` so that freq = 2^14 — which this format
 * cannot produce, but which would overflow the shift — stays representable.
 *
 * The hardware division stays.  A reciprocal multiply pays only when one
 * divisor serves many divisions, and here `freq` comes off an adaptive CDF
 * that moves after every symbol, so the reciprocal would be recomputed per
 * division and save nothing.  Measured (gprof, level 6, real OTA payload):
 * the whole backward pass, divisions included, is ~7% of an encode — the
 * finder and the pricing loop are where the time is. */
static void lzr_bwd_sym(lzr_bwd *e, unsigned slot, uint32_t cum, uint32_t freq)
{
    uint64_t x = e->st[slot];

    if ((x >> 50) >= (uint64_t)freq) {
        lzr_bwd_word(e, (uint32_t)x);
        x >>= 32;
    }
    e->st[slot] = ((x / freq) << 14) + (x % freq) + cum;
}

/* The inverse of `lzr_bypass_at`: the decoder takes the low n bits and shifts
 * them out, so the encoder shifts them in.  n is in [1, 32]. */
static void lzr_bwd_bypass(lzr_bwd *e, unsigned slot, unsigned n, uint32_t v)
{
    uint64_t x = e->st[slot];

    if (n == 0 || n > 32) {
        e->err = 1;
        return;
    }
    if ((x >> (64 - n)) != 0) {
        lzr_bwd_word(e, (uint32_t)x);
        x >>= 32;
    }
    e->st[slot] = (x << n) | v;
}

/* Returns the payload length in words, or 0 on failure. */
static size_t lzr_enc_backward(lzr_bwd *e, const uint32_t *rec, size_t nrec,
                               const uint8_t *tail, size_t limit,
                               uint32_t len_raw, unsigned len_k)
{
    size_t j = nrec;
    unsigned i;

    /* Seed from the uncoded tail (SPEC.md §4.4). */
    for (i = 0; i < 8; i++)
        e->st[i] = ((uint64_t)1 << 32) | lzr_rd32(tail + 4 * i);
    e->wi = limit;
    e->err = 0;

    while (j > 0) {
        uint32_t v = rec[--j];
        unsigned slot = (v >> 28) & 7;

        if (v & LZR_REC_BYPASS) {
            if (j == 0) { /* a tag with no value: cannot happen, not trusted */
                e->err = 1;
                break;
            }
            lzr_bwd_bypass(e, slot, v & 0x3Fu, rec[--j]);
        } else {
            lzr_bwd_sym(e, slot, v & 0x3FFFu, ((v >> 14) & 0x3FFFu) + 1u);
        }
        if (e->err)
            return 0;
    }

    /* The prologue in reverse: the block length's mantissa out of state 1, then
     * its exponent out of state 0 (src/block_body.h).  A zero exponent means a
     * zero-length delta, and the decoder then never touches state 1. */
    if (len_k)
        lzr_bwd_bypass(e, 1, len_k, len_raw);
    lzr_bwd_bypass(e, 0, 5, len_k);

    /* Then states 7..1, each serialised into state 0 as a 32-bit low half plus a
     * (5-bit exponent, mantissa) code for the high half — the reverse of the
     * order `lzr_rans_init` reads them.  The high half is >= 1 because every
     * state is kept >= 2^32, which is what makes the exponent code valid. */
    for (i = 8; i-- > 1;) {
        uint64_t x = e->st[i];
        unsigned k;

        /* Every state is kept >= 2^32 by construction, so the high half is at
         * least 1 and the exponent code is valid.  Checked because a zero high
         * half would be encoded as 1 and produce a stream that Apple's decoder
         * accepts and decodes to different bytes — the one failure mode with no
         * outward symptom.  Failing here stores the block instead. */
        if ((x >> 32) == 0) {
            e->err = 1;
            return 0;
        }
        k = lzr_exp(x >> 32);
        if (k)
            lzr_bwd_bypass(e, 0, k,
                           (uint32_t)((x >> 32) - ((uint64_t)1 << k)));
        lzr_bwd_bypass(e, 0, 5, k);
        lzr_bwd_bypass(e, 0, 32, (uint32_t)x);
    }

    /* Finally state 0 itself, whole, as the payload's first two words: the high
     * half first, then the low half. */
    lzr_bwd_word(e, (uint32_t)e->st[0]);
    lzr_bwd_word(e, (uint32_t)(e->st[0] >> 32));
    if (e->err)
        return 0;
    return limit - e->wi;
}

/* ---------------------------------------------------------------------------
 * One block.
 * ------------------------------------------------------------------------- */

/* Try to code one block.  On success returns the payload length in bytes and
 * points `*payload` into the scratch; returns 0 to mean "store this block
 * instead", in which case the caller restores the model from its checkpoint.
 *
 * The word budget is one block's worth, so a payload that would not be smaller
 * than the plaintext runs out of room and is abandoned — which is the decision
 * we would make anyway. */
static size_t lzr_enc_block(lzr_enc_scratch *s, lzr_mf *mf, size_t base,
                            const uint8_t *src, size_t off, size_t size,
                            unsigned nice, const uint8_t **payload)
{
    lzr_fwd f;
    lzr_bwd e;
    uint32_t delta, value;
    unsigned k;
    size_t words, limit = size / 4;

    /* A coded block is at least 32 coded bytes plus the 32-byte uncoded tail,
     * and never more than one block (src/block_body.h). */
    if (size < 2 * LZR_TAIL || size > LZRAVEN_BLOCK_SIZE)
        return 0;

    /* The block length is coded as a zig-zag delta against the previous coded
     * block's length, as a 5-bit exponent plus a mantissa.  Stored blocks do not
     * touch the model, so the delta is against the last *coded* block. */
    delta = (uint32_t)size - s->model.prev_len;
    value = (delta << 1) ^ (uint32_t)((int32_t)delta >> 31);
    k = lzr_exp((uint64_t)value + 1u);
    s->model.prev_len = (uint32_t)size;

    f.m = &s->model;
    f.rec = s->rec;
    f.nrec = 0;
    /* Where the decoder's rotation stands once the prologue has been read: it
     * has taken one read from state 0 and, unless the delta was zero, one from
     * state 1. */
    f.next = k ? 2u : 1u;
    f.err = 0;

    if (!lzr_enc_forward(&f, mf, base, src, off, size, nice) || f.err)
        return 0;

    e.w = s->word;
    words = lzr_enc_backward(&e, s->rec, f.nrec, src + off + size - LZR_TAIL,
                             limit, value - ((1u << k) - 1u), k);
    if (!words || 4 * words >= size)
        return 0;
    *payload = s->word + 4 * e.wi;
    return 4 * words;
}

/* ---------------------------------------------------------------------------
 * The stream.
 * ------------------------------------------------------------------------- */

/* params bits [31:26] hold a quantized dictionary size: index i denotes
 * (4 + (i & 3)) << (14 + (i >> 2)), quarter-octaves from 64 KiB to 3.5 GiB, and
 * the encoder picks the smallest entry >= src_size (SPEC.md §3.1).  A decoder
 * must not use the field — there is no decoder-side window — but writing what
 * Apple writes keeps our streams indistinguishable in shape from theirs, and it
 * is the only part of the header that carries any information at all. */
static uint32_t lzr_enc_params(size_t src_size)
{
    unsigned i;

    for (i = 0; i < 63; i++) {
        uint64_t sz = (uint64_t)(4u + (i & 3u)) << (14u + (i >> 2));

        if (sz >= (uint64_t)src_size)
            break;
    }
    return (uint32_t)i << 26;
}

/* Apple aligns its scratch pointer up to 64 bytes before use; the size reported
 * by `lzraven_encode_scratch_size` includes the slack for it. */
static lzr_enc_scratch *lzr_enc_align(void *p)
{
    uintptr_t a = ((uintptr_t)p + 63u) & ~(uintptr_t)63u;

    return (lzr_enc_scratch *)(void *)a;
}

/* ---------------------------------------------------------------------------
 * Parameters.
 * ------------------------------------------------------------------------- */

void lzraven_encode_params_default(lzraven_encode_params *p)
{
    if (!p)
        return;
    memset(p, 0, sizeof *p);
    p->level = LZRAVEN_ENCODE_LEVEL_DEFAULT;
    p->window = 0; /* = LZRAVEN_ENCODE_WINDOW_DEFAULT */
}

/* The smallest power of two >= v, floored at LZR_ENC_VIEW_MIN.  v is at most
 * LZRAVEN_ENCODE_WINDOW_MAX, so the loop terminates well inside a size_t. */
static size_t lzr_enc_ceilpow2(size_t v)
{
    size_t r = LZR_ENC_VIEW_MIN;

    while (r < v)
        r <<= 1;
    return r;
}

int lzr_enc_resolve(const lzraven_encode_params *p, lzr_enc_tune *t)
{
    size_t window = LZRAVEN_ENCODE_WINDOW_DEFAULT;
    unsigned level = (unsigned)LZRAVEN_ENCODE_LEVEL_DEFAULT;
    size_t i;

    if (p) {
        if (p->level < LZRAVEN_ENCODE_LEVEL_MIN ||
            p->level > LZRAVEN_ENCODE_LEVEL_MAX)
            return 0;
        for (i = 0; i < sizeof p->reserved / sizeof p->reserved[0]; i++)
            if (p->reserved[i])
                return 0;
        level = (unsigned)p->level;
        if (p->window) {
            window = p->window;
            if (window > LZRAVEN_ENCODE_WINDOW_MAX)
                window = LZRAVEN_ENCODE_WINDOW_MAX;
        }
    }

    t->level = level;
    t->nice_len = lzr_enc_nice[level];
    t->window = (uint32_t)window;
    t->view = lzr_enc_ceilpow2(window);
    t->history = t->view / 2u;

    /* What the finder asks for to index a view that long at that window — its
     * own figure, not a copy of its layout, so that the allocation and the
     * finder cannot drift apart.  Handing it less is safe (it shrinks its tables
     * to fit) but it would shrink the window with them, which is precisely the
     * setting the caller asked for.  0 means "no such size", which for a view
     * bounded by LZRAVEN_ENCODE_WINDOW_MAX cannot happen. */
    t->mf_bytes = lzr_mf_scratch_size_window(t->view, t->window);
    if (t->mf_bytes == 0)
        return 0;

    /* Re-anchoring must advance the view by at least a whole block, or the block
     * loop would not make progress.  LZR_ENC_VIEW_MIN is chosen so that it
     * cannot fail; it is checked because a view that violated it would loop
     * rather than misencode, and a hang is the worse failure. */
    if (t->history + (size_t)LZRAVEN_BLOCK_SIZE >= t->view)
        return 0;
    return 1;
}

size_t lzraven_encode_scratch_size_params(const lzraven_encode_params *p)
{
    lzr_enc_tune t;
    size_t n;

    if (!lzr_enc_resolve(p, &t))
        return 0;
    n = sizeof(lzr_enc_scratch) + 64u;
    if (t.mf_bytes > (size_t)-1 - n)
        return 0; /* would not fit in a size_t; refuse rather than wrap */
    return n + t.mf_bytes;
}

size_t lzraven_encode_scratch_size(void)
{
    return lzraven_encode_scratch_size_params(NULL);
}

size_t lzraven_encode_bound(size_t src_size)
{
    /* Round up without the usual `+ BLOCK_SIZE - 1`, which wraps for src_size
     * near SIZE_MAX on a 32-bit size_t — the very wrap the decoder holds up as
     * the bug class to avoid (SPEC.md §9). */
    size_t blocks = src_size / LZRAVEN_BLOCK_SIZE +
                    (src_size % LZRAVEN_BLOCK_SIZE != 0);

    /* header + one u32 per block + the worst payload (a stored block is the
     * plaintext) + the terminator.  When that sum has no representable value —
     * only possible when size_t is 32 bits and src_size is within pages of
     * SIZE_MAX, where no output buffer could hold the stream either — refuse
     * rather than wrap, as matchfind.c does for its scratch sizes. */
    if (src_size > (size_t)-1 - 12 - 4 * blocks)
        return 0;
    return 12 + 4 * blocks + src_size;
}

size_t lzraven_encode_buffer_params_ex(uint8_t *dst, size_t dst_capacity,
                                       const uint8_t *src, size_t src_size,
                                       void *scratch,
                                       const lzraven_encode_params *p,
                                       lzraven_status *status)
{
    lzr_enc_scratch *s;
    lzr_enc_tune t;
    lzr_mf mf;
    lzr_mf_params mp;
    lzraven_status st = LZRAVEN_OK;
    size_t out, off = 0, base = 0, view_end = 0;
    int anchored = 0;

    if (status)
        *status = LZRAVEN_OK;
    if (!dst || !scratch || (!src && src_size) || !lzr_enc_resolve(p, &t)) {
        st = LZRAVEN_E_ARGS;
        goto fail;
    }
    /* Divide before rounding: `src_size + BLOCK_SIZE - 1` wraps for src_size
     * near SIZE_MAX on a 32-bit size_t and would pass an unencodable input. */
    if (src_size / LZRAVEN_BLOCK_SIZE + (src_size % LZRAVEN_BLOCK_SIZE != 0) >
        LZRAVEN_MAX_BLOCKS) {
        st = LZRAVEN_E_TOO_MANY_BLOCKS;
        goto fail;
    }
    /* The shortest legal stream is the 8-byte header plus the terminator. */
    if (dst_capacity < 12) {
        st = LZRAVEN_E_OUTPUT_FULL;
        goto fail;
    }

    s = lzr_enc_align(scratch);
    lzr_model_init(&s->model);

    /* The finder reports the raw longest match and nothing else: every
     * judgement — rep versus fresh, match versus literal, and the lazy skip —
     * is made against the live model in lzr_enc_best, which knows the real
     * probabilities, where the finder's own price model is static and prices a
     * literal at a 9-bit worst case. */
    memset(&mf, 0, sizeof mf);
    mp.level = t.level;
    mp.window = t.window;
    mp.depth = 0;
    mp.nice_len = 0;
    mp.flags = LZR_MF_F_NO_HEUR | LZR_MF_F_NO_LAZY;

    lzr_wr32(dst, LZRAVEN_MAGIC);
    lzr_wr32(dst + 4, lzr_enc_params(src_size));
    out = 8;

    while (off < src_size) {
        size_t size = src_size - off;
        const uint8_t *payload = NULL;
        size_t csize;
        uint32_t flags = 0;

        if (size > LZRAVEN_BLOCK_SIZE)
            size = LZRAVEN_BLOCK_SIZE;

        /* (Re-)anchor the finder's view so that it covers this whole block plus
         * as much history as the view allows.  For any input the view spans this
         * happens exactly once, at off == 0, and the finder then sees the whole
         * input; past that it re-anchors every `view - history` bytes, and the
         * guarantee is that lookback within a block is never shorter than
         * `history`.  Re-indexing is the cost; a finder that indexed 4 GiB at
         * once is not an option. */
        if (!anchored || off + size > view_end) {
            base = off > t.history ? off - t.history : 0;
            view_end = base + t.view;
            if (view_end > src_size)
                view_end = src_size;
            lzr_mf_init_params(&mf, s->mf, t.mf_bytes, src + base,
                               view_end - base, &mp);
            anchored = 1;
        }

        /* Coding a block adapts the model, and a block we end up storing must
         * leave it untouched — the decoder's stored path never enters the
         * entropy coder.  So checkpoint first, exactly as Apple's encoder does
         * (SPEC.md §7.1: the checkpoint memcpy is what pinned the model region's
         * length at 0x4A20). */
        s->save = s->model;
        csize = lzr_enc_block(s, &mf, base, src, off, size, t.nice_len,
                              &payload);
        if (!csize) {
            s->model = s->save;
            payload = src + off;
            csize = size;
            flags = 1; /* stored */
        }

        /* Room for this block's header word, its payload, and the terminator. */
        if (dst_capacity - out < csize + 8) {
            st = LZRAVEN_E_OUTPUT_FULL;
            goto fail;
        }
        lzr_wr32(dst + out, (uint32_t)(csize << 12) | flags);
        out += 4;
        memcpy(dst + out, payload, csize);
        out += csize;
        off += size;
    }

    /* The terminator is mandatory (SPEC.md §3.2). */
    lzr_wr32(dst + out, 0);
    return out + 4;

fail:
    if (status)
        *status = st;
    return 0;
}

size_t lzraven_encode_buffer_params(uint8_t *dst, size_t dst_capacity,
                                    const uint8_t *src, size_t src_size,
                                    void *scratch,
                                    const lzraven_encode_params *p)
{
    return lzraven_encode_buffer_params_ex(dst, dst_capacity, src, src_size,
                                           scratch, p, NULL);
}

size_t lzraven_encode_buffer_ex(uint8_t *dst, size_t dst_capacity,
                                const uint8_t *src, size_t src_size,
                                void *scratch, lzraven_status *status)
{
    return lzraven_encode_buffer_params_ex(dst, dst_capacity, src, src_size,
                                           scratch, NULL, status);
}

size_t lzraven_encode_buffer(uint8_t *dst, size_t dst_capacity,
                             const uint8_t *src, size_t src_size, void *scratch)
{
    return lzraven_encode_buffer_params_ex(dst, dst_capacity, src, src_size,
                                           scratch, NULL, NULL);
}
