/* SPDX-License-Identifier: 0BSD */
/* lzraven — model initialisation (SPEC.md §5.1).
 *
 * Binary probabilities start at p = 1/2 (8192 over a 2^14 denominator).  All
 * 16-ary CDFs start uniform, cdf[k] = 1024*(k+1), except the per-state command
 * CDF, which starts from the two count tables scaled by 256 — their inclusive
 * prefix sums are themselves valid CDFs ending at exactly 16384.
 */
#include "model.h"

#include <string.h>

#define NSLOT(field) (sizeof(field) / (16 * sizeof(uint16_t)))

/* RavModelInitCommands.lit_state_count / .mat_state_count. */
static const uint8_t lzr_lit_state_count[16] = {
    17, 1, 1, 1, 1, 1, 1, 17, 17, 1, 1, 1, 1, 1, 1, 1
};
static const uint8_t lzr_mat_state_count[16] = {
    17, 17, 17, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1
};

static void fill_uniform(uint16_t *p, size_t slots)
{
    size_t i;
    unsigned k;

    for (i = 0; i < slots; i++)
        for (k = 0; k < 16; k++)
            p[16 * i + k] = (uint16_t)(1024 * (k + 1));
}

static void fill_counts(uint16_t *cdf, const uint8_t *counts)
{
    unsigned k, acc = 0;

    for (k = 0; k < 16; k++) {
        acc += counts[k];
        cdf[k] = (uint16_t)(acc * 256); /* 64 * 256 == 16384 */
    }
}

void lzr_model_init(lzr_model *m)
{
    unsigned s, i;

    for (s = 0; s < 14; s++)
        for (i = 0; i < 4; i++)
            m->is_match[s][i] = 8192;

    /* States 0..8 are literal states, 9..13 match states. */
    for (s = 0; s < 14; s++)
        fill_counts(m->cmd[s], s <= 8 ? lzr_lit_state_count : lzr_mat_state_count);

    fill_uniform(&m->lam_hi[0][0][0][0], NSLOT(m->lam_hi));
    fill_uniform(&m->lam_lo[0][0][0], NSLOT(m->lam_lo));
    fill_uniform(&m->lit_hi[0][0][0], NSLOT(m->lit_hi));
    fill_uniform(&m->lit_lo[0][0][0][0], NSLOT(m->lit_lo));
    fill_uniform(&m->rep_short_len[0][0], NSLOT(m->rep_short_len));
    fill_uniform(&m->rep_len1[0][0], NSLOT(m->rep_len1));
    fill_uniform(&m->rep_len2[0][0], NSLOT(m->rep_len2));
    fill_uniform(m->rep_len_esc10, NSLOT(m->rep_len_esc10));
    fill_uniform(m->rep_len_esc14, NSLOT(m->rep_len_esc14));
    fill_uniform(&m->mat_len1[0][0][0], NSLOT(m->mat_len1));
    fill_uniform(&m->mat_len2[0][0], NSLOT(m->mat_len2));
    fill_uniform(m->mat_len_esc10, NSLOT(m->mat_len_esc10));
    fill_uniform(m->mat_len_esc14, NSLOT(m->mat_len_esc14));
    fill_uniform(&m->dist[0][0][0], NSLOT(m->dist));

    for (i = 0; i < 4; i++)
        m->reps[i] = 1;
    m->prev_len = 0;
    m->state = 0;
}
