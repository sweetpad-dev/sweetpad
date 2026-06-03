#!/usr/bin/env python3
"""Synthetic build-settings captures for high-value Xcode flags that no
corpus project happens to enable.

For each (override_name, command-line settings) tuple, runs
`xcodebuild -showBuildSettings -json` against an existing scheme with the
overrides appended as `KEY=VALUE` arguments. The resulting JSON is saved
under `fixtures/<base>/xcode-<ver>/metadata/_synthetic/<override>/build-settings/`
so that `06_audit_coverage.py` (which globs `metadata/**/build-settings/*.json`)
picks them up automatically as additional snapshots.

These are NOT real-world resolutions — they reflect what xcodebuild emits
when forced via command-line overrides. They're still useful as snapshot
oracles for resolver behavior on those settings.

Idempotent: skip overrides whose output JSON already exists unless
`--force`. Safe to re-run.

Flags:
  --base <slug>     project to override against (default: alamofire)
  --scheme <name>   scheme to use (default: 'Alamofire iOS' for alamofire)
  --force           re-capture even if output exists
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


# (override_label, list of "K=V" args, description, optional destination override)
# Destination override is a sentinel like "generic-ios-device" (resolved in
# main()) — None means use the default iOS Simulator picked from the fixture.
OVERRIDES: list[tuple[str, list[str], str, str | None]] = [
    ("library-evolution",
        ["BUILD_LIBRARY_FOR_DISTRIBUTION=YES"],
        "Force Swift library evolution (binary-stable interface)", None),
    ("llvm-lto",
        ["LLVM_LTO=YES"],
        "Force link-time optimization", None),
    ("mergeable-library",
        ["MERGEABLE_LIBRARY=YES"],
        "Xcode 15+ mergeable library at link time", None),
    ("strict-concurrency-upcoming",
        ["SWIFT_UPCOMING_FEATURE_STRICT_CONCURRENCY=YES"],
        "Swift upcoming feature: strict concurrency", None),
    ("library-evolution+lto",
        ["BUILD_LIBRARY_FOR_DISTRIBUTION=YES", "LLVM_LTO=YES"],
        "Library evolution + LTO together", None),
    # AUDIT gap fills:
    ("archs-arm64e",
        ["ARCHS=arm64e"],
        "Force arm64e (Pointer Authentication) — requires generic/platform=iOS",
        "generic/platform=iOS"),
    ("ldflags-quoted-whitespace",
        ['OTHER_LDFLAGS=-framework "My Framework" -Wl,-segalign,0x4000'],
        "Quoted-whitespace flag in OTHER_LDFLAGS", None),
    ("swift-version-6",
        ["SWIFT_VERSION=6.0"],
        "Swift language mode 6.0", None),
    ("ios-deployment-15",
        ["IPHONEOS_DEPLOYMENT_TARGET=15.0"],
        "Override iOS deployment target back to 15.0", None),
    ("dead-code-stripping-off",
        ["DEAD_CODE_STRIPPING=NO"],
        "Disable dead-code stripping", None),
    ("swift-onone",
        ["SWIFT_OPTIMIZATION_LEVEL=-Onone"],
        "Force Swift -Onone even in Release", None),
    ("gcc-optimization-s",
        ["GCC_OPTIMIZATION_LEVEL=s"],
        "Force clang -Os", None),
    ("enable-bitcode-no",
        ["ENABLE_BITCODE=NO"],
        "Explicit bitcode disable (Xcode default since 14, still worth snapshotting)", None),
]

DEFAULT_BASE = "alamofire"
DEFAULT_SCHEME_PER_BASE = {
    "alamofire": "Alamofire iOS",
    "kingfisher": "Kingfisher",
    "ice-cubes": "IceCubesApp",
}


def project_args_for(slug: str) -> list[str]:
    """Return -workspace or -project args for the slug's primary subproject."""
    root = common.CORPUS_DIR / slug
    workspaces = sorted(p for p in root.iterdir()
                        if p.is_dir() and p.suffix == ".xcworkspace"
                        and p.name != "project.xcworkspace")
    if workspaces:
        return ["-workspace", str(workspaces[0])]
    projects = sorted(p for p in root.iterdir() if p.is_dir() and p.suffix == ".xcodeproj")
    if not projects:
        raise FileNotFoundError(f"no .xcodeproj or .xcworkspace in {root}")
    return ["-project", str(projects[0])]


def capture(base: str, scheme: str, override_label: str,
            kvs: list[str], config: str, dest: str, xcode_version: str,
            *, force: bool) -> tuple[bool, str]:
    out_dir = (common.FIXTURES_DIR / base / f"xcode-{xcode_version}"
               / "metadata" / "_synthetic" / override_label / "build-settings")
    out_dir.mkdir(parents=True, exist_ok=True)
    dest_slug = common.destination_slug(dest)
    out_path = out_dir / f"{common.slug(config)}__{dest_slug}.json"
    if out_path.exists() and not force:
        common.log(f"  skip (exists): {out_path.name}")
        return True, ""

    cmd = [
        "xcodebuild", "-showBuildSettings", "-json",
        "-scheme", scheme,
        "-configuration", config,
        "-destination", dest,
        *project_args_for(base),
        *kvs,
    ]
    cp = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
    if cp.returncode != 0 or not cp.stdout.strip():
        return False, (cp.stderr or cp.stdout)[-500:]
    try:
        parsed = json.loads(cp.stdout)
    except json.JSONDecodeError as e:
        return False, f"non-JSON output: {e}"
    with out_path.open("w") as f:
        json.dump(parsed, f, indent=2, sort_keys=True)
        f.write("\n")
    return True, str(out_path)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--base", default=DEFAULT_BASE,
                    help=f"corpus slug to override against (default: {DEFAULT_BASE})")
    ap.add_argument("--scheme", default=None,
                    help="scheme name (default: per-slug default)")
    ap.add_argument("--xcode", default=None,
                    help="restrict to one Xcode (version or slot); default: current")
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()

    base = args.base
    scheme = args.scheme or DEFAULT_SCHEME_PER_BASE.get(base)
    if not scheme:
        common.log(f"no default scheme for {base}; pass --scheme")
        return 1

    installs = common.discover_installed_xcodes()
    if not installs:
        common.log("no Xcode installs found")
        return 1
    # The version only selects the output-path / destinations.json bucket; the
    # capture itself runs against whatever `xcode-select` is currently pointed
    # at (the multi-version orchestrator switches it before calling us).
    xcode_version = common.selected_xcodes(installs, args.xcode)[0].version

    # Read alamofire's existing destinations.json to grab an iOS Simulator dest
    dest_path = (common.FIXTURES_DIR / base / f"xcode-{xcode_version}"
                 / "metadata" / "schemes" / scheme / "destinations.json")
    if not dest_path.exists():
        common.log(f"missing {dest_path}; run 02_capture_metadata first")
        return 1
    dests = json.loads(dest_path.read_text())
    sim = next((d for d in dests
                if d.get("platform") == "iOS Simulator"
                and "id" in d and "OS" in d
                and "placeholder" not in d.get("id", "").lower()
                and not d.get("name", "").startswith("Any ")), None)
    if not sim:
        common.log(f"no usable iOS Simulator destination in {dest_path}")
        return 1
    dest = f"platform=iOS Simulator,id={sim['id']}"
    common.log(f"using destination {dest} (id maps to {sim.get('name')})")

    had_failure = False
    for override_label, kvs, _notes, dest_override in OVERRIDES:
        use_dest = dest_override or dest
        common.log(f"capture override={override_label} kvs={kvs} dest={use_dest}")
        for config in ("Debug", "Release"):
            ok, info = capture(base, scheme, override_label, kvs,
                               config, use_dest, xcode_version, force=args.force)
            if ok:
                common.log(f"  {config}: {info or 'already captured'}")
            else:
                had_failure = True
                common.log(f"  {config}: FAILED — {info}")

    return 1 if had_failure else 0


if __name__ == "__main__":
    sys.exit(main())
