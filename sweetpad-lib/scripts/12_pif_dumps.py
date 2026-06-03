#!/usr/bin/env python3
"""Copy PIF (Project Interchange Format) dumps from DerivedData into fixtures/.

Xcode's modern build system writes a normalized JSON representation of every
project, target, and workspace it sees into `XCBuildData/PIFCache/{project,
target,workspace}/`. These files are the canonical machine-readable
intermediate between `.xcodeproj/project.pbxproj` and the build engine — far
cleaner to parse than pbxproj, and a great oracle for resolver behavior.

Source:
  fixtures/<slug>/xcode-<ver>/.derived/Build/Intermediates.noindex/XCBuildData/PIFCache/

Destination:
  fixtures/<slug>/xcode-<ver>/pif/{project,target,workspace}/<filename>

PIF filenames embed content hashes — e.g.
`PROJECT@v11_mod=...hash=...plugins=...-json` — so multiple variations of the
same project may be cached. We copy them as-is, preserving the hashing in
filenames so the resolver can correlate cache entries to inputs.

Idempotent: skips existing destination files unless --force.

Flags:
  --slug <name>     restrict to one corpus slug
  --xcode <ver>     restrict to one Xcode version
  --force           re-copy even if destination exists
"""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


PIF_REL_PATH = Path(".derived/Build/Intermediates.noindex/XCBuildData/PIFCache")
PIF_SUBDIRS = ("workspace", "project", "target")


def collect_for_fixture(fixture_root: Path, *, force: bool) -> tuple[int, int]:
    """Returns (copied, skipped)."""
    src = fixture_root / PIF_REL_PATH
    if not src.exists():
        return 0, 0
    dst_root = fixture_root / "pif"
    copied = 0
    skipped = 0
    for sub in PIF_SUBDIRS:
        src_sub = src / sub
        if not src_sub.exists():
            continue
        dst_sub = dst_root / sub
        dst_sub.mkdir(parents=True, exist_ok=True)
        for f in src_sub.iterdir():
            if not f.is_file():
                continue
            dst = dst_sub / f.name
            if dst.exists() and not force:
                skipped += 1
                continue
            shutil.copy2(f, dst)
            copied += 1
    return copied, skipped


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--slug", help="restrict to one corpus slug")
    ap.add_argument("--xcode", help="restrict to one Xcode version (e.g. 26.0.1)")
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()

    if not common.FIXTURES_DIR.exists():
        common.log("no fixtures/ dir")
        return 1

    had_any = False
    for project_dir in sorted(common.FIXTURES_DIR.iterdir()):
        if not project_dir.is_dir() or project_dir.name.startswith("_"):
            continue
        if args.slug and project_dir.name != args.slug:
            continue
        for xcode_dir in sorted(project_dir.iterdir()):
            if not xcode_dir.is_dir() or not xcode_dir.name.startswith("xcode-"):
                continue
            if args.xcode and xcode_dir.name != f"xcode-{args.xcode}":
                continue
            copied, skipped = collect_for_fixture(xcode_dir, force=args.force)
            if copied or skipped:
                had_any = True
                common.log(f"{project_dir.name}/{xcode_dir.name}: "
                           f"copied {copied}, skipped {skipped}")
            else:
                common.log(f"{project_dir.name}/{xcode_dir.name}: no PIFCache found")

    if not had_any:
        common.log("no PIF data found anywhere — was a build run?")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
