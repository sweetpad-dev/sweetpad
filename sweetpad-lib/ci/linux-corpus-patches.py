#!/usr/bin/env python3
"""Apply explicit, pinned compatibility patches to coverage fixtures.

Run on the Mac, then in the Linux VM with --resolved-only: corpus sources
are shared, but each host has its own resolved RevenueCat checkout.
This is a fixture preparation step, never part of a user's app build.
"""

import argparse
import json
from pathlib import Path
import subprocess

CI = Path(__file__).resolve().parent
CORPUS = CI.parent / "corpus"


def prepare(entry, derived_data, check):
    root = (derived_data / entry["resolvedCheckout"] if "resolvedCheckout" in entry
            else CORPUS / entry["checkout"])
    actual = subprocess.check_output(
        ["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
    if actual != entry["commit"]:
        raise RuntimeError(f"{root}: expected {entry['commit']}, found {actual}")
    pending = []
    for name in entry["patches"]:
        patch = CI / "patches" / name
        applied = subprocess.run(
            ["git", "-C", str(root), "apply", "--reverse", "--check", str(patch)],
            capture_output=True).returncode == 0
        if not applied:
            subprocess.run(["git", "-C", str(root), "apply", "--check", str(patch)], check=True)
            pending.append(patch)
        print(f"{root.name}: {name}: {'applied' if applied else 'ready'}", flush=True)
    if not check:
        for patch in pending:
            subprocess.run(["git", "-C", str(root), "apply", str(patch)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--derived-data", type=Path, default=Path.home() /
                        "Library/Developer/Xcode/DerivedData/IceCubesApp-amgqitujpnvhzvclmndqodwauekw")
    parser.add_argument("--check", action="store_true", help="check pins and patches without writing")
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--resolved-only", action="store_true")
    group.add_argument("--skip-resolved", action="store_true")
    args = parser.parse_args()
    entries = json.loads((CI / "linux-corpus-patches.json").read_text())
    for entry in entries:
        resolved = "resolvedCheckout" in entry
        if (args.resolved_only and not resolved) or (args.skip_resolved and resolved):
            continue
        prepare(entry, args.derived_data, args.check)


if __name__ == "__main__":
    main()
