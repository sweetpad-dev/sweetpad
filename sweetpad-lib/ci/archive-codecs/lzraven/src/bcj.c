/* SPDX-License-Identifier: 0BSD */
/*
 * bcj.c — LZRAVEN executable prefilters (ARM64 branch, x86 CALL/JMP rel32).
 *
 * Written from the format description in docs/notes/block-format.md 7.
 *
 * Both filters replace a PC-relative field with an absolute one, truncated to
 * a narrow signed field and sign-extended back into its instruction slot; the
 * decode direction subtracts the position term the encoder added.  "Position"
 * is the byte offset of the instruction within the *whole* output buffer, so
 * every worker takes a base_off for the chunk it is filtering.
 *
 * Constraints this file honours, because it runs on attacker-controlled data:
 *   - no access outside [buf, buf + len);
 *   - no unaligned loads (every 32-bit access is assembled byte by byte);
 *   - no signed arithmetic anywhere, so no signed overflow;
 *   - no size_t expression that can wrap.
 */

#include "lzraven_bcj.h"

/* --------------------------------------------------------------------------
 * little-endian 32-bit access, byte at a time (no alignment assumptions)
 * -------------------------------------------------------------------------- */

static uint32_t bcj_rd32(const uint8_t *p)
{
    return (uint32_t)p[0]
         | ((uint32_t)p[1] << 8)
         | ((uint32_t)p[2] << 16)
         | ((uint32_t)p[3] << 24);
}

static void bcj_wr32(uint8_t *p, uint32_t v)
{
    p[0] = (uint8_t)(v & 0xFFu);
    p[1] = (uint8_t)((v >> 8) & 0xFFu);
    p[2] = (uint8_t)((v >> 16) & 0xFFu);
    p[3] = (uint8_t)((v >> 24) & 0xFFu);
}

/* --------------------------------------------------------------------------
 * ARM64 branch filter
 *
 * Walks 4-byte-aligned words from buf + phase, stepping 4.
 *
 * BL   (w & 0xFC000000) == 0x94000000.  Plain B (0x14000000) is not filtered.
 *      Guard: bits 25, 24 and 23 of w must be equal, i.e. the 26-bit immediate
 *      is the sign-extension of its low 24 bits.
 *      decode: t = (w - (pos >> 2)) & 0xFFFFFF
 *      encode: t = (w + (pos >> 2)) & 0xFFFFFF
 *      then rebuild the 26-bit slot as the sign-extension of t.
 *
 * ADRP (w & 0x9F000000) == 0x90000000.  The 21-bit page delta lives in
 *      immlo = bits [30:29] and immhi = bits [23:5], natural value
 *      (immhi << 2) | immlo.  Guard: bits 23 and 22 of w must be equal.
 *      The filtered word uses a different packing of the same 20 bits:
 *      U = (immlo << 18) | (immhi & 0x3FFFF), with immhi bit 18 carrying the
 *      sign extension so the guard holds in both directions.
 *      decode: natural = (U - (pos >> 12)) & 0xFFFFF
 *      encode: U = (natural + (pos >> 12)) & 0xFFFFF
 * -------------------------------------------------------------------------- */

#define BCJ_A64_BL_MASK   0xFC000000u
#define BCJ_A64_BL_OP     0x94000000u
#define BCJ_A64_ADRP_MASK 0x9F000000u
#define BCJ_A64_ADRP_OP   0x90000000u
/* opcode bit 31, bit 28 and Rd — everything an ADRP rewrite must preserve */
#define BCJ_A64_ADRP_KEEP 0x9000001Fu

static void bcj_arm64(uint8_t *buf, size_t len, unsigned phase,
                      uint64_t base_off, int encode)
{
    size_t off;
    size_t last;

    if (buf == NULL || len < LZRAVEN_BCJ_MIN_CHUNK || len < 4u) {
        return;
    }
    phase &= 3u;
    last = len - 4u;                 /* len >= 4, so this cannot wrap */

    for (off = (size_t)phase; off <= last; off += 4u) {
        uint32_t w = bcj_rd32(buf + off);
        uint64_t pos = base_off + (uint64_t)off;

        if ((w & BCJ_A64_BL_MASK) == BCJ_A64_BL_OP) {
            uint32_t delta;
            uint32_t t;
            uint32_t s;

            /* skip unless bits 25, 24, 23 agree */
            if (((w + 0x00800000u) & 0x03000000u) != 0u) {
                continue;
            }
            delta = (uint32_t)(pos >> 2);
            t = (encode ? (w + delta) : (w - delta)) & 0x00FFFFFFu;
            s = (t >> 23) & 1u;
            bcj_wr32(buf + off, BCJ_A64_BL_OP | (s << 25) | (s << 24) | t);
        } else if ((w & BCJ_A64_ADRP_MASK) == BCJ_A64_ADRP_OP) {
            uint32_t immlo;
            uint32_t immhi;
            uint32_t delta;
            uint32_t v;
            uint32_t nlo;
            uint32_t nhi;

            /* skip unless bits 23 and 22 agree */
            if (((w + 0x00400000u) & 0x00800000u) != 0u) {
                continue;
            }
            immlo = (w >> 29) & 3u;
            immhi = (w >> 5) & 0x7FFFFu;
            delta = (uint32_t)(pos >> 12);

            if (encode) {
                /* natural packing in, filtered packing out */
                uint32_t nat = ((immhi & 0x3FFFFu) << 2) | immlo;
                v = (nat + delta) & 0x000FFFFFu;
                nlo = (v >> 18) & 3u;
                nhi = (v & 0x3FFFFu) | (((v >> 17) & 1u) << 18);
            } else {
                /* filtered packing in, natural packing out */
                uint32_t u = (immlo << 18) | (immhi & 0x3FFFFu);
                v = (u - delta) & 0x000FFFFFu;
                nlo = v & 3u;
                nhi = ((v >> 2) & 0x3FFFFu) | (((v >> 19) & 1u) << 18);
            }
            bcj_wr32(buf + off,
                     (w & BCJ_A64_ADRP_KEEP) | (nlo << 29) | (nhi << 5));
        }
    }
}

/* --------------------------------------------------------------------------
 * x86 CALL/JMP rel32 filter
 *
 * Byte scan for E8 (CALL rel32) / E9 (JMP rel32).  With p the opcode position
 * and q = p + 1 the rel32 field, a site is filtered only when buf[q + 3] is
 * 0x00 or 0xFF; otherwise the scan resumes at p + 4.  The filtered form is the
 * classic LZMA/BCJ layout: the low 24 bits of the absolute target stored
 * big-endian, followed by a sign byte.
 * -------------------------------------------------------------------------- */

static void bcj_x86(uint8_t *buf, size_t len, uint64_t base_off, int encode)
{
    size_t p;
    size_t last;

    if (buf == NULL || len < LZRAVEN_BCJ_MIN_CHUNK || len < 5u) {
        return;
    }
    last = len - 5u;                 /* len >= 5, so this cannot wrap */

    p = 0u;
    while (p <= last) {
        size_t q;
        uint32_t pos;

        if ((buf[p] & 0xFEu) != 0xE8u) {
            p += 1u;
            continue;
        }
        q = p + 1u;                          /* q + 3 <= len - 1 */
        if (buf[q + 3u] != 0x00u && buf[q + 3u] != 0xFFu) {
            p += 4u;
            continue;
        }
        pos = (uint32_t)(base_off + (uint64_t)q);

        if (encode) {
            /* rel32 (25-bit signed, stored little-endian) -> absolute */
            uint32_t a = (bcj_rd32(buf + q) + pos) & 0x01FFFFFFu;
            buf[q]      = (uint8_t)((a >> 16) & 0xFFu);
            buf[q + 1u] = (uint8_t)((a >> 8) & 0xFFu);
            buf[q + 2u] = (uint8_t)(a & 0xFFu);
            buf[q + 3u] = (uint8_t)(((a >> 24) & 1u) ? 0xFFu : 0x00u);
        } else {
            uint32_t a = ((uint32_t)buf[q] << 16)
                       | ((uint32_t)buf[q + 1u] << 8)
                       | (uint32_t)buf[q + 2u];
            uint32_t r;
            a |= (uint32_t)(buf[q + 3u] & 1u) << 24;
            r = (a - pos) & 0x01FFFFFFu;
            if ((r & 0x01000000u) != 0u) {
                r |= 0xFE000000u;            /* sign-extend to 32 bits */
            }
            bcj_wr32(buf + q, r);
        }
        p += 5u;
    }
}

/* --------------------------------------------------------------------------
 * public entry points
 * -------------------------------------------------------------------------- */

void lzraven_bcj_arm64_decode_at(uint8_t *buf, size_t len, unsigned phase,
                                 uint64_t base_off)
{
    bcj_arm64(buf, len, phase, base_off, 0);
}

void lzraven_bcj_arm64_encode_at(uint8_t *buf, size_t len, unsigned phase,
                                 uint64_t base_off)
{
    bcj_arm64(buf, len, phase, base_off, 1);
}

void lzraven_bcj_x86_decode_at(uint8_t *buf, size_t len, uint64_t base_off)
{
    bcj_x86(buf, len, base_off, 0);
}

void lzraven_bcj_x86_encode_at(uint8_t *buf, size_t len, uint64_t base_off)
{
    bcj_x86(buf, len, base_off, 1);
}

void lzraven_bcj_arm64_decode(uint8_t *buf, size_t len, unsigned phase)
{
    bcj_arm64(buf, len, phase, 0u, 0);
}

void lzraven_bcj_arm64_encode(uint8_t *buf, size_t len, unsigned phase)
{
    bcj_arm64(buf, len, phase, 0u, 1);
}

void lzraven_bcj_x86_decode(uint8_t *buf, size_t len)
{
    bcj_x86(buf, len, 0u, 0);
}

void lzraven_bcj_x86_encode(uint8_t *buf, size_t len)
{
    bcj_x86(buf, len, 0u, 1);
}
