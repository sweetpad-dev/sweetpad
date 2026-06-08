#!/usr/bin/env python3
"""Mutation audit — measure the coverage of our coverage.

Each mutation is a one-line edit to the resolver / argument generator that
represents a plausible bug. The audit applies it, runs the fast nets, and records
whether *any* net goes red (red = the bug was caught). A mutation that slips
through every fast net is an unguarded corner — printed so it can be prioritized
with data instead of guessed at.

This is how we'd have known, before shipping, that nothing watched the
`SDKROOT = auto` binding: that mutation is the first row, and it must be caught by
`bsp_arg_invariants` (the multiplatform fixture). The originals are always
restored, even on error or Ctrl-C.

    python3 scripts/21_mutation_audit.py          # fast tier (seconds per row)
    python3 scripts/21_mutation_audit.py --e2e    # also prove the de-exoneration
                                                  # (builds the multiplatform fixture)
"""

import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# The fast nets, by the test-binary name cargo prints after "Running …". `--lib`
# covers the in-crate unit tests (e.g. bsp::tests::editor_sdk_*).
FAST_NETS = ["lib", "bsp_arg_invariants", "compiler_args_oracle"]

# id, file, the unique anchor to replace, its replacement, and a note. `expect` is
# what we believe today — the audit prints reality, and a drift from `expect` is
# itself the signal.
MUTATIONS = [
    {
        "id": "sdkroot-auto-unbound",
        "file": "src/build_context.rs",
        "find": "&& !requested_supported",
        "replace": "&& true",
        "note": "SDKROOT=auto stops binding to a concrete SDK (the IceCubes bug)",
        "expect": "caught",
    },
    {
        "id": "editor-sdk-ignores-platforms",
        "file": "src/bsp/mod.rs",
        "find": (
            "    let platform = if sdkroot.is_empty() || sdkroot == \"auto\" {\n"
            "        supported_platforms.to_lowercase()\n"
            "    } else {\n"
            "        sdkroot\n"
            "    };"
        ),
        "replace": "    let _ = supported_platforms;\n    let platform = sdkroot;",
        "note": "editor ignores SUPPORTED_PLATFORMS, defaults auto -> macOS",
        "expect": "caught",
    },
    {
        "id": "drop-sdk",
        "file": "src/compiler_args.rs",
        "find": "a.pair(\"-sdk\", sdk);",
        "replace": "let _ = sdk;",
        "note": "no -sdk emitted at all",
        "expect": "caught",
    },
    {
        "id": "wrong-target-platform",
        "file": "src/compiler_args.rs",
        "find": "Some(format!(\"{arch}-{vendor}-{os}{suffix}\"))",
        "replace": "Some(format!(\"{arch}-{vendor}-macos{suffix}\"))",
        "note": "-target forced to macos, mismatching -sdk on every non-macOS platform",
        "expect": "caught",
    },
    {
        "id": "drop-module-name",
        "file": "src/compiler_args.rs",
        "find": "a.pair(\"-module-name\", module);",
        "replace": "let _ = module;",
        "note": "no -module-name emitted",
        "expect": "caught",
    },
    {
        "id": "drop-macro-plugin",
        "file": "src/compiler_args.rs",
        "find": "for plugin in macro_plugins {",
        "replace": "for plugin in macro_plugins.iter().take(0) {",
        "note": "third-party macro plugins not loaded (-load-plugin-executable)",
        "expect": "caught",
    },
    {
        "id": "drop-package-frameworks",
        "file": "src/compiler_args.rs",
        "find": "a.pair(\"-F\", &format!(\"{products}/PackageFrameworks\"));",
        "replace": "let _ = products;",
        "note": "dynamic SPM-product -F PackageFrameworks search path dropped",
        "expect": "caught",
    },
]


def run_one_net(net: str) -> str:
    """Run one net and classify by exit status — robust to output formatting.
    'build-error' means the mutation didn't compile (not a real catch)."""
    cmd = ["cargo", "test", "--lib"] if net == "lib" else ["cargo", "test", "--test", net]
    proc = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True)
    out = proc.stdout + proc.stderr
    if "error[" in out or "could not compile" in out:
        return "build-error"
    return "FAILED" if proc.returncode != 0 else "ok"


def run_fast_nets() -> dict:
    return {net: run_one_net(net) for net in FAST_NETS}


def caught_by(result: dict) -> list:
    if any(s == "build-error" for s in result.values()):
        return ["build-error"]
    return [net for net, status in result.items() if status == "FAILED"]


def apply_mutation(m: dict) -> str:
    path = REPO / m["file"]
    original = path.read_text()
    count = original.count(m["find"])
    if count != 1:
        raise SystemExit(
            f"anchor for '{m['id']}' matched {count} time(s) in {m['file']} "
            f"(expected exactly 1) — the source moved; update the anchor."
        )
    path.write_text(original.replace(m["find"], m["replace"]))
    return original


def main() -> int:
    do_e2e = "--e2e" in sys.argv

    print("=" * 78)
    print("MUTATION AUDIT — does a fast net catch each injected bug?")
    print("=" * 78)

    rows = []
    for m in MUTATIONS:
        path = REPO / m["file"]
        original = apply_mutation(m)
        try:
            result = run_fast_nets()
        finally:
            path.write_text(original)
        nets = caught_by(result)
        rows.append((m, nets))
        if nets == ["build-error"]:
            verdict = "DID NOT COMPILE (anchor/replacement invalid)"
        elif nets:
            verdict = "CAUGHT by " + ", ".join(nets)
        else:
            verdict = "UNCAUGHT by fast tier"
        print(f"\n• {m['id']:<28} {verdict}")
        print(f"    {m['note']}")

    uncaught = [m for m, nets in rows if not nets]
    caught = [m for m, nets in rows if nets and nets != ["build-error"]]
    print("\n" + "-" * 78)
    print(f"{len(caught)}/{len(rows)} mutations caught by the fast tier.")
    if uncaught:
        print("Corners only the slow BSP_CORPUS e2e tier guards (candidates for a fast net):")
        for m in uncaught:
            print(f"  - {m['id']}: {m['note']}")
    print("-" * 78)

    if do_e2e:
        print("\n" + "=" * 78)
        print("E2E: revert the SDKROOT=auto fix, prove the de-exoneration reclassifies it")
        print("=" * 78)
        m = MUTATIONS[0]
        path = REPO / m["file"]
        original = apply_mutation(m)
        try:
            proc = subprocess.run(
                ["cargo", "test", "--test", "bsp_corpus_completion", "--", "--nocapture"],
                cwd=REPO, capture_output=True, text=True,
                env={**os.environ,
                     "BSP_CORPUS": "1",
                     "BSP_CORPUS_ONLY": "_synthetic-multiplatform"},
            )
        finally:
            path.write_text(original)
        out = proc.stdout + proc.stderr
        line = next((l for l in out.splitlines() if "_synthetic-multiplatform" in l and "clean" in l), "")
        reclassified = "(incl 0 reclassified)" not in line and "reclassified" in line
        print(f"  measurement: {line.strip() or '(no measurement line — build may have failed)'}")
        if reclassified:
            print("  ✓ de-exoneration fired: the stdlib-load failure was charged to us, not exonerated.")
        else:
            print("  ✗ no reclassification observed (check the build succeeded and the bug reproduced).")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
