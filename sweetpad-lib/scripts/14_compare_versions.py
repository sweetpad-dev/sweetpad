#!/usr/bin/env python3
"""Compare two captured Xcode versions' per-target build settings.

Reports the build settings that *genuinely* differ between version A and B after
canonicalizing the volatile Xcode-app / SDK-version path drift and dropping the
keys that merely echo the toolchain version. This is the `compare_versions(a, b)`
core of the delta/dedup design in DOCS.md, used here as a standalone analysis
tool: it answers "what actually changes in build settings across an Xcode major"
and surfaces version-conditional behaviour the resolver may need to model (the
way the 16.4 capture exposed `XCODE_VERSION_MAJOR` nested expansion).

Both captures are real `xcodebuild -showBuildSettings` output. Cross-version
differences come from (1) the Xcode-app path (`Xcode-16.4.0.app` vs
`Xcode-26.0.1.app`), (2) the SDK version embedded in paths (`iPhoneOS17.5.sdk`
vs `iPhoneOS18.5.sdk`), (3) keys whose value is literally the version/SDK number,
(4) the build-output root (a capture taken with default DerivedData vs a
project-local `build/` dir), and (5) genuine behavioural changes. (1)+(2) are
normalized away, (3) is dropped via `ECHO_KEYS`/`skip_key`, (4) is bucketed into
`path_diffs` (geometry, shown collapsed), and (5) — value/flag/list changes and
keys added/removed between versions — is the behavioural signal this tool prints.

The `--delta` auto-capture-and-commit mode from the original design (stage a
fresh per-target capture, diff vs the last kept version, commit only on change)
is intentionally NOT built: the version-selection policy captures the latest
non-beta minor per major with no minor sweeps (see DOCS.md), so there is no dedup
workflow to drive it. `compare_versions` here is the reusable core if that
workflow is ever revived.

Usage:
  scripts/14_compare_versions.py 16.4.0 26.0.1            # all shared captures
  scripts/14_compare_versions.py 15.4.0 26.0.1 --slug kingfisher
  scripts/14_compare_versions.py 16.4.0 26.0.1 --max-samples 3 --top 40
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

# Keys whose value is just the toolchain/SDK version — trivially different across
# versions and not a behavioural change, so they're excluded from the diff.
ECHO_KEYS: frozenset[str] = frozenset({
    "XCODE_VERSION_ACTUAL", "XCODE_VERSION_MAJOR", "XCODE_VERSION_MINOR",
    "XCODE_PRODUCT_BUILD_VERSION",
    "SDK_NAME", "SDK_NAMES", "SDK_VERSION", "SDK_VERSION_ACTUAL",
    "SDK_VERSION_MAJOR", "SDK_VERSION_MINOR", "SDK_PRODUCT_BUILD_VERSION",
    "CORRESPONDING_SIMULATOR_SDK_NAME",
    "DTPlatformVersion", "DTSDKName", "DTSDKBuild", "DTXcode", "DTXcodeBuild",
    "DTPlatformBuild", "DTPlatformName", "DTCompiler",
    "PLATFORM_PRODUCT_BUILD_VERSION",
    # Per-version derived-data cache roots (the version/build/hash is the only
    # thing that changes) — not Xcode behaviour.
    "CACHE_ROOT", "CCHROOT", "COMPILATION_CACHE_CAS_PATH",
    "SDK_STAT_CACHE_DIR", "SDK_STAT_CACHE_PATH",
    # Pure host-environment noise (differs by what was on $PATH at capture time,
    # not by Xcode behaviour).
    "PATH",
})


def skip_key(key: str) -> bool:
    # `SDK_DIR_<platform><version>` keys (e.g. `SDK_DIR_iphoneos18_5`) encode the
    # SDK version in the key name itself — pure version echo, skip.
    return key in ECHO_KEYS or key.startswith("SDK_DIR_")


# Normalize the volatile fragments that legitimately differ by version so they
# don't masquerade as behavioural diffs: the Xcode app, SDK dirs, the per-version
# DeveloperTools cache root, and any bare `<platform><version>` token.
_XCODE_APP = re.compile(r"Xcode-[0-9][0-9.]*\.app")
_SDK_DIR = re.compile(r"([A-Za-z]+)[0-9]+(?:\.[0-9]+)*\.sdk")
_DEVTOOLS_CACHE = re.compile(r"(com\.apple\.DeveloperTools/)[0-9][0-9.]*-[0-9A-Za-z]+")
_PLATFORM_VER = re.compile(
    r"\b(iphoneos|iphonesimulator|macos|macosx|appletvos|appletvsimulator"
    r"|watchos|watchsimulator|xros|xrsimulator|driverkit)[0-9]+(?:\.[0-9]+)*"
)
_STATCACHE_HASH = re.compile(r"-[0-9a-f]{6,}\.sdkstatcache")


def canon(value: str) -> str:
    value = _XCODE_APP.sub("Xcode-§.app", value)
    value = _SDK_DIR.sub(r"\1§.sdk", value)
    value = _DEVTOOLS_CACHE.sub(r"\1§", value)
    value = _STATCACHE_HASH.sub("-§.sdkstatcache", value)
    value = _PLATFORM_VER.sub(r"\1§", value)
    return value


def load_settings(path: Path) -> dict[str, str]:
    """Return the `buildSettings` of a per-target capture (1-element list)."""
    try:
        doc = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return {}
    entry = doc[0] if isinstance(doc, list) and doc else doc
    return entry.get("buildSettings", {}) if isinstance(entry, dict) else {}


def per_target_files(version: str, slug: str | None) -> dict[str, Path]:
    """Map each per-target capture's `<slug>/<rel>` key to its path."""
    out: dict[str, Path] = {}
    slugs = [slug] if slug else [p.slug for p in common.CORPUS]
    for s in slugs:
        root = common.metadata_dir(s, version) / "_per_target"
        if not root.is_dir():
            continue
        for f in root.rglob("*.json"):
            out[f"{s}/{f.relative_to(root)}"] = f
    return out


def _is_path_diff(ca: str, cb: str) -> bool:
    """Both values are absolute paths (or a single such path) — a geometry
    difference (different build-output root / capture methodology), not Xcode
    behaviour. Kept out of the default behavioural view, like the oracle's
    structural tier separates path drift from real value misses."""
    return ca.startswith("/") and cb.startswith("/") and " " not in ca and " " not in cb


def compare_versions(a: str, b: str, slug: str | None) -> dict:
    """Diff per-target captures shared by versions `a` and `b`.

    Splits the differing keys into `diffs` (behavioural — value/flag/list
    changes) and `path_diffs` (both sides a single absolute path — geometry, e.g.
    one capture used default DerivedData and the other a project-local `build/`).
    `only_in_a`/`only_in_b` aggregate keys present in just one version.
    """
    files_a = per_target_files(a, slug)
    files_b = per_target_files(b, slug)
    shared = sorted(set(files_a) & set(files_b))

    diffs: dict[str, dict] = defaultdict(lambda: {"count": 0, "samples": []})
    path_diffs: dict[str, int] = defaultdict(int)
    only_a: dict[str, int] = defaultdict(int)
    only_b: dict[str, int] = defaultdict(int)
    captures_compared = 0

    for rel in shared:
        sa = load_settings(files_a[rel])
        sb = load_settings(files_b[rel])
        if not sa or not sb:
            continue
        captures_compared += 1
        for key in (sa.keys() | sb.keys()):
            if skip_key(key):
                continue
            if key not in sa:
                only_b[key] += 1
                continue
            if key not in sb:
                only_a[key] += 1
                continue
            ca, cb = canon(sa[key]), canon(sb[key])
            if ca == cb:
                continue
            if _is_path_diff(ca, cb):
                path_diffs[key] += 1
                continue
            d = diffs[key]
            d["count"] += 1
            if len(d["samples"]) < 8 and (ca, cb) not in d["samples"]:
                d["samples"].append((ca, cb))

    return {
        "version_a": a,
        "version_b": b,
        "captures_compared": captures_compared,
        "shared_captures": len(shared),
        "diffs": dict(diffs),
        "path_diffs": dict(path_diffs),
        "only_in_a": dict(only_a),
        "only_in_b": dict(only_b),
    }


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("version_a")
    ap.add_argument("version_b")
    ap.add_argument("--slug", help="restrict to one corpus slug")
    ap.add_argument("--top", type=int, default=60, help="show the N most frequent diff keys")
    ap.add_argument("--max-samples", type=int, default=2, help="value samples per key")
    ap.add_argument("--json", action="store_true", help="emit the full result as JSON")
    args = ap.parse_args()

    result = compare_versions(args.version_a, args.version_b, args.slug)

    if args.json:
        print(json.dumps(result, indent=2, sort_keys=True))
        return 0

    a, b = result["version_a"], result["version_b"]
    print(f"=== compare per-target build settings: {a} -> {b} ===")
    print(
        f"shared captures: {result['shared_captures']}, "
        f"compared (both non-empty): {result['captures_compared']}"
    )
    diffs = result["diffs"]
    print(f"\n{len(diffs)} keys differ (after canonicalizing Xcode/SDK path drift, "
          f"dropping {len(ECHO_KEYS)} version-echo keys):\n")
    for key, d in sorted(diffs.items(), key=lambda kv: (-kv[1]["count"], kv[0]))[: args.top]:
        print(f"  {d['count']:<4} {key}")
        for ca, cb in d["samples"][: args.max_samples]:
            print(f"         {a}: {ca!r}")
            print(f"         {b}: {cb!r}")
    if result["only_in_a"]:
        print(f"\nkeys only in {a} ({len(result['only_in_a'])}): "
              f"{', '.join(sorted(result['only_in_a'])[:20])}")
    if result["only_in_b"]:
        print(f"\nkeys only in {b} ({len(result['only_in_b'])}): "
              f"{', '.join(sorted(result['only_in_b'])[:20])}")
    if result["path_diffs"]:
        print(f"\n{len(result['path_diffs'])} path-only (geometry) keys differ "
              f"(different build-output root / capture methodology, not behaviour): "
              f"{', '.join(sorted(result['path_diffs'])[:12])}…")
    return 0


if __name__ == "__main__":
    sys.exit(main())
