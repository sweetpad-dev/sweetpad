#!/usr/bin/env python3
"""Prepare explicitly pinned dependency compatibility patches on the current host.

Run once on the Mac and once inside the Linux VM after package resolution.
The build engine never edits dependency sources.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys

CI = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("corpus_patches", CI / "linux-corpus-patches.py")
patches = importlib.util.module_from_spec(spec)
spec.loader.exec_module(patches)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", default="")
    parser.add_argument("--derived-data", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    for project in json.loads((CI / "linux-projects.json").read_text()):
        if args.only not in project["slug"]:
            continue
        derived = args.derived_data or (
            CI.parent / "corpus/_linux-followups" / (project["slug"] + "-xcode")
            if sys.platform == "darwin" else Path.home() / project["linuxDerivedData"])
        for entry in project.get("dependencyPatches", []):
            patches.prepare(entry, derived, args.check)


if __name__ == "__main__":
    main()
