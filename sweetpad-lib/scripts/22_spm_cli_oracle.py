#!/usr/bin/env python3
"""SwiftPM CLI oracle capture.

sweetpad reads a Swift package's structure from `swift package dump-package`
(see `src/cli/swiftpm.rs`) and drives build/test/run with the `swift` toolchain
— it never shells out to xcodebuild to figure out a package's structure. This
script grounds that path by capturing, for the committed sample package at
`fixtures/_synthetic-spm-cli/project`, what xcodebuild and swift actually do:

  fixtures/_synthetic-spm-cli/xcode-<ver>/captures/
      dump-package.json   `swift package dump-package`       (our structure source)
      list.json           `xcodebuild -list -json`           (xcodebuild's synthesized schemes)
      build.json          `swift build --configuration debug` exit status
      test.json           `swift test`                        exit status
      meta.json           description + the toolchain used

`tests/spm_oracle.rs` then compares our `Manifest::scheme_names()` (parsed from
`dump-package.json`) against xcodebuild's `-list` schemes, proving the
no-xcodebuild path matches reality. The captures are JSON so they diff cleanly.

Idempotent: existing captures are kept unless --force.

Flags:
  --xcode <ver|slot>    pick a specific Xcode (default: current)
  --force               re-capture even if outputs exist
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


SYNTH_DIR = common.FIXTURES_DIR / "_synthetic-spm-cli"
PROJECT_DIR = SYNTH_DIR / "project"


def _capture_json(cmd: list[str], out_path: Path) -> tuple[bool, str]:
    """Run a command that prints JSON; store its parsed stdout. xcodebuild and
    swift may print a non-JSON preamble, so slice from the first brace."""
    cp = subprocess.run(cmd, cwd=PROJECT_DIR, capture_output=True, text=True, timeout=600)
    if cp.returncode != 0 or not cp.stdout.strip():
        return False, (cp.stderr or cp.stdout)[-400:]
    text = cp.stdout
    start = text.find("{")
    if start < 0:
        return False, f"no JSON in output: {text[:200]}"
    try:
        parsed = json.loads(text[start:])
    except json.JSONDecodeError as e:
        return False, f"non-JSON: {e}"
    with out_path.open("w") as f:
        json.dump(parsed, f, indent=2, sort_keys=True)
        f.write("\n")
    return True, ""


def _capture_status(cmd: list[str], out_path: Path) -> bool:
    """Run an action command and record only its exit status (build/test produce
    artifacts, not a structured result we oracle against — success is the signal)."""
    cp = subprocess.run(cmd, cwd=PROJECT_DIR, capture_output=True, text=True, timeout=1200)
    with out_path.open("w") as f:
        json.dump(
            {"command": cmd, "exitCode": cp.returncode, "ok": cp.returncode == 0},
            f,
            indent=2,
            sort_keys=True,
        )
        f.write("\n")
    return cp.returncode == 0


def process(xcode: common.XcodeInstall, *, force: bool) -> None:
    captures = SYNTH_DIR / f"xcode-{xcode.version}" / "captures"
    captures.mkdir(parents=True, exist_ok=True)

    def want(name: str) -> bool:
        exists = (captures / name).exists()
        if exists and not force:
            common.log(f"  keep {name} (exists)")
        return force or not exists

    if want("dump-package.json"):
        ok, info = _capture_json(["swift", "package", "dump-package"], captures / "dump-package.json")
        common.log(f"  dump-package: {'ok' if ok else 'FAIL ' + info}")

    if want("list.json"):
        ok, info = _capture_json(["xcodebuild", "-list", "-json"], captures / "list.json")
        common.log(f"  xcodebuild -list: {'ok' if ok else 'FAIL ' + info}")

    if want("build.json"):
        ok = _capture_status(["swift", "build", "--configuration", "debug"], captures / "build.json")
        common.log(f"  swift build: {'ok' if ok else 'FAIL'}")

    if want("test.json"):
        ok = _capture_status(["swift", "test"], captures / "test.json")
        common.log(f"  swift test: {'ok' if ok else 'FAIL'}")

    with (captures / "meta.json").open("w") as f:
        json.dump(
            {
                "description": "SwiftPM CLI oracle: dump-package vs xcodebuild -list, plus swift build/test status",
                "package": str(PROJECT_DIR.relative_to(common.REPO_ROOT)),
                "xcode": xcode.version,
            },
            f,
            indent=2,
            sort_keys=True,
        )
        f.write("\n")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--xcode", help="restrict to one Xcode (version or slot)")
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()

    if not (PROJECT_DIR / "Package.swift").exists():
        common.log(f"ERROR: sample package missing at {PROJECT_DIR}")
        return 1

    installs = common.discover_installed_xcodes()
    xcodes = common.selected_xcodes(installs, args.xcode)

    for x in xcodes:
        common.log(f"\n========= _synthetic-spm-cli :: xcode {x.version} =========")
        try:
            with common.with_xcode(x):
                process(x, force=args.force)
        except Exception as e:
            common.log(f"ERROR: {e}")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
