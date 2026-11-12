/* SPDX-License-Identifier: 0BSD */
/* lzraven — the parts of the symbol grammar the decoder and the encoder share
 * (SPEC.md §5.1, §5.3, §4.4).
 *
 * The state machine and the two constants below are the *definition* of the
 * grammar, not an implementation detail of either side: the encoder has to make
 * exactly the transitions the decoder will make, and pick contexts from exactly
 * the same table, or it produces a stream that is framed correctly and decodes
 * to different bytes with nothing to catch it.  A second copy of the tables that
 * differed in one entry would do precisely that, so there is one copy and both
 * sides include it.
 */
#ifndef LZRAVEN_GRAMMAR_H
#define LZRAVEN_GRAMMAR_H

#include <stdint.h>

#define LZR_TAIL 32u        /* trailing bytes of a block are not entropy-coded */
#define LZR_LADDER_MAX 330u /* length-ladder saturation value */

/* RavGetLitLamContext.lut — literals since the last match, saturating at 4.
 * Value 0..2 select the matched-literal ("lam") path. */
static const uint8_t lzr_lit_lam_ctx[14] = {
    4, 3, 3, 2, 1, 2, 1, 2, 1, 0, 0, 0, 0, 0
};

/* RavUpdateState.lut row 16 — the literal row. */
static const uint8_t lzr_lit_row[14] = {
    0, 0, 0, 2, 3, 1, 5, 1, 7, 4, 6, 8, 6, 8
};

/* RavUpdateState.lut[cmd][state], in closed form (tables.md §2).  `cmd` is 0..15
 * for a command and 16 for a literal. */
static inline unsigned lzr_next_state(unsigned cmd, unsigned state)
{
    if (cmd == 16)
        return lzr_lit_row[state];
    if (cmd <= 6)
        return state <= 8 ? 11u : 13u;
    if (cmd == 7)
        return state <= 8 ? 9u : 12u;
    return state <= 8 ? 10u : 12u;
}

#endif /* LZRAVEN_GRAMMAR_H */
