#!/usr/bin/env python3
"""Resolve a tag-checked release version or a non-publishing dry-run version."""
import argparse
import os
from pathlib import Path
import tomllib


def resolve(crate, reference):
    if reference.startswith("refs/tags/cli-v"):
        tag = reference.removeprefix("refs/tags/cli-v")
        if tag != crate:
            raise ValueError(f"tag cli-v{tag} differs from workspace version {crate}")
        return crate
    return crate + "-dryrun"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=Path(__file__).resolve().parents[2] / "Cargo.toml")
    parser.add_argument("--ref", default=os.environ.get("GITHUB_REF", ""))
    args = parser.parse_args()
    crate = tomllib.loads(args.manifest.read_text())["workspace"]["package"]["version"]
    print(resolve(crate, args.ref))
