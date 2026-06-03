#!/usr/bin/env python3
"""Capture global xcodebuild and per-SDK runtime metadata.

xcodebuild has no true "show global defaults" mode — `-showBuildSettings`
requires a project (or workspace, or SwiftPM package with concrete buildable
targets). Auto-generated SPM schemes return `[]` because they're aggregate
schemes with no own settings. So this script captures the *invariants* that
are global across projects:

  - `xcodebuild -version` output
  - `xcodebuild -showsdks -json` inventory
  - per-SDK xcrun probes (sdk-path, platform-path, version, build-version)
  - `xcodebuild -version -sdk <canonical>` per SDK

The project-level "baseline" layer (what does the resolver get from a real
.xcodeproj with no scheme/target?) is captured by 09_per_project_settings.py.
The documented default values for every build setting (the "spec" layer) is
already captured by 04_snapshot_xcspecs.py.

Outputs (per Xcode version), under `fixtures/_global/xcode-<ver>/`:

  defaults/xcodebuild-version.txt      `xcodebuild -version` full output.
  sdks/showsdks.json                   `xcodebuild -showsdks -json`.
  sdks/<sdk-canonical-name>.json       per-SDK xcrun + xcodebuild metadata.

Idempotent: skips outputs that already exist unless --force.

Flags:
  --xcode <ver|slot>    pick a specific Xcode (default: current via xcode-select)
  --force               re-capture even if outputs exist
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


GLOBAL_DIR = common.FIXTURES_DIR / "_global"


def xb_env(xcode: common.XcodeInstall) -> dict[str, str]:
    e = dict(os.environ)
    e["DEVELOPER_DIR"] = str(xcode.developer_dir)
    return e


def run_xb(args: list[str], *, xcode: common.XcodeInstall, timeout: float = 120,
           ) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["xcodebuild", *args],
        env=xb_env(xcode), capture_output=True, text=True, timeout=timeout,
    )


def run_xcrun(args: list[str], *, xcode: common.XcodeInstall, timeout: float = 30,
              ) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["xcrun", *args],
        env=xb_env(xcode), capture_output=True, text=True, timeout=timeout,
    )


def capture_defaults(out_root: Path, xcode: common.XcodeInstall, *, force: bool) -> None:
    defaults_dir = out_root / "defaults"
    defaults_dir.mkdir(parents=True, exist_ok=True)

    ver_path = defaults_dir / "xcodebuild-version.txt"
    if force or not ver_path.exists():
        cp = run_xb(["-version"], xcode=xcode)
        if cp.returncode == 0:
            ver_path.write_text(cp.stdout)
            common.log(f"  wrote {ver_path}")


def list_sdks(*, xcode: common.XcodeInstall) -> list[dict]:
    cp = run_xb(["-showsdks", "-json"], xcode=xcode)
    if cp.returncode != 0:
        common.log(f"-showsdks failed: {cp.stderr[-300:]}")
        return []
    try:
        return json.loads(cp.stdout)
    except json.JSONDecodeError:
        return []


def capture_sdks(out_root: Path, xcode: common.XcodeInstall, *, force: bool) -> None:
    sdks_dir = out_root / "sdks"
    sdks_dir.mkdir(parents=True, exist_ok=True)

    sdks = list_sdks(xcode=xcode)
    show_path = sdks_dir / "showsdks.json"
    if force or not show_path.exists():
        with show_path.open("w") as f:
            json.dump(sdks, f, indent=2, sort_keys=True)
            f.write("\n")
        common.log(f"  wrote {show_path} ({len(sdks)} sdks)")

    for sdk in sdks:
        canonical = sdk.get("canonicalName") or sdk.get("sdk") or sdk.get("displayName")
        if not canonical:
            continue
        out_path = sdks_dir / f"{common.slug(canonical)}.json"
        if out_path.exists() and not force:
            continue

        record: dict = {"canonicalName": canonical, "showsdks_entry": sdk}

        # xcrun runtime probes — each is best-effort.
        for arg, key in (
            ("--show-sdk-path", "sdk_path"),
            ("--show-sdk-platform-path", "platform_path"),
            ("--show-sdk-version", "sdk_version"),
            ("--show-sdk-build-version", "sdk_build_version"),
            ("--show-sdk-platform-version", "platform_version"),
        ):
            cp = run_xcrun(["--sdk", canonical, arg], xcode=xcode)
            record[key] = {
                "stdout": cp.stdout.rstrip("\n") if cp.stdout else "",
                "rc": cp.returncode,
            }

        # `xcodebuild -version -sdk <name>` emits a different shape (multi-line
        # KEY = VALUE block).
        cp = run_xb(["-version", "-sdk", canonical], xcode=xcode)
        record["xcodebuild_version_sdk"] = {
            "stdout": cp.stdout,
            "rc": cp.returncode,
        }

        with out_path.open("w") as f:
            json.dump(record, f, indent=2, sort_keys=True)
            f.write("\n")
        common.log(f"  wrote {out_path.name}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--xcode", help="restrict to one Xcode (version or slot)")
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()

    installs = common.discover_installed_xcodes()
    if not installs:
        common.log("no Xcode installs found")
        return 1
    xcodes = common.selected_xcodes(installs, args.xcode)

    for x in xcodes:
        common.log(f"\n========= _global :: xcode {x.version} =========")
        out_root = GLOBAL_DIR / f"xcode-{x.version}"
        out_root.mkdir(parents=True, exist_ok=True)
        try:
            with common.with_xcode(x):
                capture_defaults(out_root, x, force=args.force)
                capture_sdks(out_root, x, force=args.force)
        except Exception as e:
            common.log(f"ERROR: {e}")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
