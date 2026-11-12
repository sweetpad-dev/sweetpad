/* SPDX-License-Identifier: 0BSD */
/* lzraven — the per-block decode loop, as a template.
 *
 * THIS HEADER IS INCLUDED MORE THAN ONCE, on purpose, and has no include guard.
 * block.c includes it once per dispatch path (scalar, SSE2, AVX2, NEON) with
 *
 *     LZR_KERNEL  which CDF kernel src/cdf.h should compile
 *     LZR_TAG     a suffix that keeps the instantiations' symbols apart
 *     LZR_ENTRY   the name of the exported entry point
 *
 * so that the kernel inlines into the loop.  The grammar itself is written once
 * and is identical in every instantiation — the paths differ only in src/cdf.h.
 *
 * The renames below are what let the body be written in plain names; they are
 * undone at the bottom of the file so the next instantiation starts clean.
 */
#define LZR_CAT_(a, b) a##b
#define LZR_CAT(a, b) LZR_CAT_(a, b)
#define LZR_ID(n) LZR_CAT(n, LZR_TAG)

#define lzr_cdf_step LZR_ID(lzr_cdf_step)
#define lzr_copy_overlap LZR_ID(lzr_copy_overlap)
#define lzr_vec_copy LZR_ID(lzr_vec_copy)
#define lzr_count_gt LZR_ID(lzr_count_gt)
#define lzr_sym LZR_ID(lzr_sym)
#define lzr_ladder LZR_ID(lzr_ladder)

/* Brings in src/cdf.h for this LZR_KERNEL, then lzr_sym around it. */
#include "rans.h"

/* The shared length ladder (SPEC.md §4.3).  Returns 0..330; the caller adds the
 * base (2 for matches, 18 for reps).  Every rung is at rate 6. */
static unsigned lzr_ladder(lzr_rans *r, uint16_t *rung1, uint16_t *esc10,
                           uint16_t *esc14, uint16_t *rung2)
{
    unsigned s1 = lzr_sym(r, rung1, 6);
    unsigned hi, s2;

    if (s1 <= 9)
        return s1;
    if (s1 == 15)
        return LZR_LADDER_MAX;
    if (s1 == 10) {
        hi = 10;
        s2 = lzr_sym(r, esc10, 6);
    } else if (s1 == 14) {
        hi = lzr_sym(r, esc14, 6) + 14; /* 14..29 */
        s2 = lzr_sym(r, rung2, 6);
    } else { /* 11..13 */
        hi = s1;
        s2 = lzr_sym(r, rung2, 6);
    }
    return 16 * hi + s2 - 150; /* 16*hi >= 160, so this cannot underflow */
}

int LZR_ENTRY(lzr_model *m, const uint8_t *payload, size_t csize,
              uint8_t *dst, size_t dst_capacity, size_t dst_pos,
              size_t *out_len, lzraven_status *status)
{
    lzr_rans r;
    unsigned k, i;
    uint32_t value, mask, delta, size32;
    size_t size, coded_end, pos;

    if (!lzr_rans_init(&r, payload, csize)) {
        *status = LZRAVEN_E_CORRUPT;
        return 0;
    }

    /* The block's uncompressed size: a 5-bit exponent from state 0 and the
     * mantissa from state 1, then zig-zag decoded and delta-added against the
     * previous block's size.  The round-robin starts at 2 — or at 1 in the
     * degenerate k == 0 case, where state 1 was never touched. */
    k = (unsigned)lzr_bypass_at(&r, 0, 5);
    if (k) {
        value = (uint32_t)(lzr_bypass_at(&r, 1, k) + (((uint64_t)1 << k) - 1));
        r.next = 2;
    } else {
        value = 0;
        r.next = 1;
    }
    if (r.err) {
        *status = LZRAVEN_E_CORRUPT;
        return 0;
    }
    mask = (uint32_t)0 - (value & 1);
    delta = (value >> 1) ^ mask;
    size32 = m->prev_len + delta; /* u32 wrap, as Apple's */
    m->prev_len = size32;
    size = size32;

    /* A coded block is 32 coded bytes plus the 32-byte uncoded tail, at least,
     * and never more than one 512 KiB block. */
    if (size < 2 * LZR_TAIL || size > LZRAVEN_BLOCK_SIZE) {
        *status = LZRAVEN_E_FRAMING;
        return 0;
    }
    if (size > dst_capacity - dst_pos) { /* dst_pos < dst_capacity, checked */
        *status = LZRAVEN_E_OUTPUT_FULL;
        return 0;
    }

    coded_end = dst_pos + size - LZR_TAIL;
    pos = dst_pos;

    /* The very first byte of a stream has no previous byte to context on. */
    if (pos == 0) {
        dst[0] = (uint8_t)lzr_bypass(&r, 8);
        pos = 1;
    }

    while (pos < coded_end) {
        unsigned ps = (unsigned)(pos & 3);
        unsigned state = m->state;
        unsigned cmd;
        size_t length, src_off;
        uint64_t dist;

        if (r.err) {
            *status = LZRAVEN_E_CORRUPT;
            return 0;
        }

        if (lzr_bit(&r, m->is_match[state], ps)) {
            /* ---- literal ------------------------------------------------ */
            unsigned prev = dst[pos - 1];
            unsigned t2 = prev >> 6;
            unsigned lam = lzr_lit_lam_ctx[state];
            unsigned hi, lo;

            if (lam <= 2) {
                /* Matched literal: contexted on the byte one rep0 back.  rep0
                 * was bounds-checked when it was decoded, at a position no
                 * greater than this one, so it is in range by induction — the
                 * check below is belt and braces. */
                uint32_t rep0 = m->reps[0];
                unsigned mb;

                if (rep0 == 0 || rep0 > pos) {
                    *status = LZRAVEN_E_DISTANCE;
                    return 0;
                }
                mb = dst[pos - rep0];
                hi = lzr_sym(&r, m->lam_hi[lam][t2][mb >> 4], 6);
                if (hi == (mb >> 4))
                    lo = lzr_sym(&r, m->lam_lo[lam][mb & 15], 6);
                else
                    lo = lzr_sym(&r, m->lit_lo[ps][t2][hi], 7);
            } else {
                hi = lzr_sym(&r, m->lit_hi[ps][t2], 7);
                lo = lzr_sym(&r, m->lit_lo[ps][t2][hi], 7);
            }
            dst[pos] = (uint8_t)((hi << 4) | lo);
            m->state = (uint8_t)lzr_next_state(16, state);
            pos++;
            continue;
        }

        /* ---- a command -------------------------------------------------- */
        cmd = lzr_sym(&r, m->cmd[state], 6);
        if (cmd <= 6) {
            /* Match at a fresh distance; cmd is the distance class.  Two
             * modelled symbols at rate 7, then — after both of them, not
             * between — the raw bypass field. */
            unsigned d1, d2;

            length = 2 + lzr_ladder(&r, m->mat_len1[cmd][ps], m->mat_len_esc10,
                                    m->mat_len_esc14, m->mat_len2[ps]);
            d1 = lzr_sym(&r, m->dist[cmd][0], 7);
            d2 = lzr_sym(&r, m->dist[cmd][1], 7);
            if (cmd == 0) {
                dist = ((uint64_t)d1 << 4) + d2 + 1;
            } else {
                unsigned kk = (d1 >> 2) + 4 * cmd; /* 4..27 */
                uint32_t extra = lzr_bypass(&r, kk - 2);
                dist = ((uint64_t)(4u | (d1 & 3)) << (kk + 2)) +
                       ((uint64_t)extra << 4) + d2 + 1;
            }
            /* The input is untrusted and a distance is 32 bits wide, so this
             * check — not the formula above — is what keeps a match inside the
             * output buffer.  Not optional (SPEC.md §5.4, §9). */
            if (dist == 0 || dist > pos) {
                *status = LZRAVEN_E_DISTANCE;
                return 0;
            }
            m->reps[3] = m->reps[2];
            m->reps[2] = m->reps[1];
            m->reps[1] = m->reps[0];
            m->reps[0] = (uint32_t)dist; /* dist <= pos < 2^32, checked above */
        } else if (cmd == 7) {
            /* Short rep: rep0, length 1. */
            length = 1;
            dist = m->reps[0];
            if (dist == 0 || dist > pos) {
                *status = LZRAVEN_E_DISTANCE;
                return 0;
            }
        } else {
            unsigned slot = (cmd <= 11) ? cmd - 8 : cmd - 12;

            dist = m->reps[slot];
            if (cmd <= 11)
                length = 2 + lzr_sym(&r, m->rep_short_len[ps], 6); /* 2..17 */
            else
                length = 18 + lzr_ladder(&r, m->rep_len1[ps], m->rep_len_esc10,
                                         m->rep_len_esc14, m->rep_len2[ps]);
            if (dist == 0 || dist > pos) {
                *status = LZRAVEN_E_DISTANCE;
                return 0;
            }
            if (slot) { /* move the used distance to the front */
                unsigned j;
                for (j = slot; j > 0; j--)
                    m->reps[j] = m->reps[j - 1];
                m->reps[0] = (uint32_t)dist;
            }
        }

        /* A match may not run past the coded region.  The 32-byte tail is not
         * entropy-coded (SPEC.md §3.3), so the grammar has to land exactly on
         * `coded_end`; a copy that would overshoot it is a malformed block, and
         * Apple's decoder rejects it before copying a single byte rather than
         * truncating.  Landing exactly on `coded_end` is legal and common.
         * `pos < coded_end` here, so the subtraction cannot underflow. */
        if (length > coded_end - pos) {
            *status = LZRAVEN_E_CORRUPT;
            return 0;
        }

        /* The copy therefore fits with the whole 32-byte tail to spare.  That
         * slack is what lets the vector kernels store in whole vectors, so it
         * is handed to lzr_copy_overlap as its writable span and asserted
         * there (src/cdf.h). */
        src_off = pos - (size_t)dist;
        if ((size_t)dist >= length)
            memcpy(dst + pos, dst + src_off, length); /* provably disjoint */
        else
            lzr_copy_overlap(dst + pos, dst + src_off, length, (size_t)dist,
                             (coded_end - pos) + LZR_TAIL);
        pos += length;
        m->state = (uint8_t)lzr_next_state(cmd, state);
    }

    /* ---- the uncoded 32-byte tail --------------------------------------- */
    /* The encoder seeded each state with (1 << 32) | word, where the eight
     * words are the block's last 32 bytes, so they fall out of the states in
     * slot order once the coded region is exhausted.  The high half being
     * exactly 1 on all eight is a free end-of-block consistency check. */
    if (r.err) {
        *status = LZRAVEN_E_CORRUPT;
        return 0;
    }
    for (i = 0; i < 8; i++) {
        if ((r.st[i] >> 32) != 1) {
            *status = LZRAVEN_E_CORRUPT;
            return 0;
        }
    }
    /* Apple requires the payload to be consumed exactly; anything left over is
     * a malformed block.  Verified to hold on every block of every vector. */
    if (r.wpos != r.wcount) {
        *status = LZRAVEN_E_CORRUPT;
        return 0;
    }
    for (i = 0; i < 8; i++)
        lzr_wr32(dst + coded_end + 4 * i, (uint32_t)r.st[i]);

    *out_len = size;
    return 1;
}

#undef lzr_cdf_step
#undef lzr_copy_overlap
#undef lzr_vec_copy
#undef lzr_count_gt
#undef lzr_sym
#undef lzr_ladder
#undef LZR_CAT_
#undef LZR_CAT
#undef LZR_ID
#undef LZR_KERNEL
#undef LZR_TAG
#undef LZR_ENTRY
#ifdef LZR_VEC
#undef LZR_VEC
#endif
