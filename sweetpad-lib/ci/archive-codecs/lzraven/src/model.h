/* SPDX-License-Identifier: 0BSD */
/* lzraven — the persistent probability model (SPEC.md §5, entropy-coder.md §8.1).
 *
 * Apple keeps this as one 0x4A20-byte scratch region; we keep the same contents
 * as typed fields.  The comment on each field is its offset in APPLE'S region,
 * not in this struct, so the derivation can be checked against the notes.  The
 * two layouts agree up to and including `dist`; Apple leaves an 8-byte gap at
 * 0x49F0, so from `reps` onward the numbers are Apple's alone — do not diff
 * them against offsetof() here.  588 sixteen-entry CDFs, 56 binary
 * probabilities, four repeat distances, the previous block length and the
 * state byte — all of it persists across blocks.
 */
#ifndef LZRAVEN_MODEL_H
#define LZRAVEN_MODEL_H

#include <stdint.h>

typedef struct lzr_model {
    uint16_t is_match[14][4];          /* 0x0000  binary, [state][posState]   */
    uint16_t cmd[14][16];              /* 0x0070  command CDF per state       */
    uint16_t lam_hi[3][4][16][16];     /* 0x0230  matched-literal high nibble  */
    uint16_t lam_lo[3][16][16];        /* 0x1A30  matched-literal low nibble   */
    uint16_t lit_hi[4][4][16];         /* 0x2030  plain-literal high nibble    */
    uint16_t lit_lo[4][4][16][16];     /* 0x2230  plain-literal low nibble     */
    uint16_t rep_short_len[4][16];     /* 0x4230  commands 8..11              */
    uint16_t rep_len1[4][16];          /* 0x42B0  rep ladder rung 1           */
    uint16_t rep_len2[4][16];          /* 0x4330  rep ladder rung 2           */
    uint16_t rep_len_esc10[16];        /* 0x43B0                              */
    uint16_t rep_len_esc14[16];        /* 0x43D0                              */
    uint16_t mat_len1[7][4][16];       /* 0x43F0  match ladder rung 1         */
    uint16_t mat_len2[4][16];          /* 0x4770  match ladder rung 2         */
    uint16_t mat_len_esc10[16];        /* 0x47F0                              */
    uint16_t mat_len_esc14[16];        /* 0x4810                              */
    uint16_t dist[7][2][16];           /* 0x4830  two CDFs per distance class */
    /* -- Apple's layout has a gap at 0x49F0; offsets below are Apple's only -- */
    uint32_t reps[4];                  /* 0x49F8  repeat distances, init 1    */
    uint32_t prev_len;                 /* 0x4A18  previous block's length     */
    uint8_t state;                     /* 0x4A1C  the 14-state machine        */
} lzr_model;

void lzr_model_init(lzr_model *m);

#endif /* LZRAVEN_MODEL_H */
