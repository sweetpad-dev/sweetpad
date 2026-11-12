/* SPDX-License-Identifier: 0BSD */
/* lzraven — SIMD dispatch: which CDF kernel this build can offer, and which one
 * this CPU should run.
 *
 * The decoder's hot loop is `lzr_sym` (SPEC.md §4.3): search a 16-entry u16 CDF
 * for the symbol, then nudge all 16 lanes toward a target.  Both halves are
 * pure data-parallel work over exactly one vector register's worth of lanes, so
 * they are written once as portable C (the reference) and once per vector ISA.
 *
 * The grammar in block.c is instantiated once per kernel so the kernel inlines
 * into it; the choice is then made once per block, not once per symbol.  The
 * scalar instantiation always exists and is the differential baseline —
 * `LZRAVEN_SIMD=scalar` selects it on any machine.
 */
#ifndef LZRAVEN_SIMD_H
#define LZRAVEN_SIMD_H

/* The kernels exist to be inlined into the decode loop; a call per symbol would
 * give most of the win back, and the compilers' cost models do not always agree
 * with that (GCC 14 inlines the AVX2 kernel and outlines the wider SSE2 one).
 * So say it rather than hope. */
#if defined(__GNUC__) || defined(__clang__)
#define LZR_INLINE static inline __attribute__((always_inline))
#else
#define LZR_INLINE static inline
#endif

/* ---------------------------------------------------------------------------
 * Which kernels this build contains.
 *
 * SSE2 is part of the x86-64 ABI, so that path needs no feature test and no
 * special compiler flags.  AVX2 does: it is compiled into a target-attributed
 * region and reached only after CPUID says the CPU (and the OS's XSAVE state)
 * supports it.  NEON is mandatory on aarch64, so it needs no test either.
 * ------------------------------------------------------------------------- */
#define LZR_K_SCALAR 0
#define LZR_K_SSE2   1
#define LZR_K_AVX2   2
#define LZR_K_NEON   3

#if defined(__x86_64__) || defined(_M_X64)
#define LZR_HAVE_SSE2 1
/* GCC's `#pragma GCC target` and clang's `#pragma clang attribute` are the two
 * spellings of "compile the functions in this region for another ISA level".
 * Neither compiler understands the other's, hence the pair. */
/* `_Pragma` takes one parenthesised string literal, so these cannot be split
 * across lines however long they get. */
#if defined(__clang__)
#define LZR_TARGET_AVX2_PUSH _Pragma("clang attribute push (__attribute__((target(\"avx2,popcnt\"))), apply_to=function)")
#define LZR_TARGET_POP _Pragma("clang attribute pop")
#define LZR_HAVE_AVX2 1
#elif defined(__GNUC__)
#define LZR_TARGET_AVX2_PUSH                                                  \
    _Pragma("GCC push_options") _Pragma("GCC target(\"avx2,popcnt\")")
#define LZR_TARGET_POP _Pragma("GCC pop_options")
#define LZR_HAVE_AVX2 1
#endif
#endif /* x86-64 */

#if defined(__aarch64__) || defined(_M_ARM64)
#define LZR_HAVE_NEON 1
#endif

#ifndef LZR_HAVE_SSE2
#define LZR_HAVE_SSE2 0
#endif
#ifndef LZR_HAVE_AVX2
#define LZR_HAVE_AVX2 0
#endif
#ifndef LZR_HAVE_NEON
#define LZR_HAVE_NEON 0
#endif
#ifndef LZR_TARGET_AVX2_PUSH
#define LZR_TARGET_AVX2_PUSH
#define LZR_TARGET_POP
#endif

/* The runtime choice.  Values are stable: tools print the name, not the enum. */
typedef enum lzr_simd_kind {
    LZR_SIMD_SCALAR = 0,
    LZR_SIMD_SSE2 = 1,
    LZR_SIMD_AVX2 = 2,
    LZR_SIMD_NEON = 3
} lzr_simd_kind;

/* ---------------------------------------------------------------------------
 * Feature detection.
 * ------------------------------------------------------------------------- */
#if LZR_HAVE_AVX2
#include <cpuid.h>

/* xgetbv(0), spelled as bytes so no -mxsave is needed to reach the intrinsic. */
static inline unsigned lzr_xgetbv0(void)
{
    unsigned lo, hi;

    __asm__ volatile(".byte 0x0f, 0x01, 0xd0" : "=a"(lo), "=d"(hi) : "c"(0));
    (void)hi;
    return lo;
}

/* AVX2 needs three things, and skipping any of them faults at run time: the
 * CPU has it, the OS has enabled XSAVE, and the OS actually saves the SSE and
 * AVX state on a context switch (XCR0 bits 1 and 2). */
static inline int lzr_cpu_has_avx2(void)
{
    unsigned a, b, c, d;

    if (__get_cpuid_max(0, 0) < 7)
        return 0;
    if (!__get_cpuid(1, &a, &b, &c, &d))
        return 0;
    if (!(c & bit_OSXSAVE) || !(c & bit_AVX))
        return 0;
    if ((lzr_xgetbv0() & 0x6u) != 0x6u)
        return 0;
    __cpuid_count(7, 0, a, b, c, d);
    return (b & bit_AVX2) != 0;
}
#endif /* LZR_HAVE_AVX2 */

/* The best kernel this CPU can run, ignoring any override. */
static inline lzr_simd_kind lzr_simd_best(void)
{
#if LZR_HAVE_AVX2
    if (lzr_cpu_has_avx2())
        return LZR_SIMD_AVX2;
#endif
#if LZR_HAVE_NEON
    return LZR_SIMD_NEON;
#elif LZR_HAVE_SSE2
    return LZR_SIMD_SSE2;
#else
    return LZR_SIMD_SCALAR;
#endif
}

/* Is `k` compiled in *and* runnable here? */
static inline int lzr_simd_usable(lzr_simd_kind k)
{
    switch (k) {
    case LZR_SIMD_SCALAR:
        return 1;
    case LZR_SIMD_SSE2:
        return LZR_HAVE_SSE2;
    case LZR_SIMD_AVX2:
#if LZR_HAVE_AVX2
        return lzr_cpu_has_avx2();
#else
        return 0;
#endif
    case LZR_SIMD_NEON:
        return LZR_HAVE_NEON;
    }
    return 0;
}

static inline const char *lzr_simd_kind_name(lzr_simd_kind k)
{
    switch (k) {
    case LZR_SIMD_SCALAR: return "scalar";
    case LZR_SIMD_SSE2:   return "sse2";
    case LZR_SIMD_AVX2:   return "avx2";
    case LZR_SIMD_NEON:   return "neon";
    }
    return "scalar";
}

#endif /* LZRAVEN_SIMD_H */
