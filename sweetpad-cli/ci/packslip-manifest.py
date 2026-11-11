#!/usr/bin/env python3
"""Describe the CLI separately from extension releases in this repository."""
import argparse
import json
from pathlib import Path
import re


def manifest(directory, version):
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-dryrun)?", version):
        raise ValueError("invalid CLI version")
    lines = ['project = "github.com/sweetpad-dev/sweetpad/tools/cli"',
             f'version = {json.dumps(version)}', 'bin = ["sweetpad", "sweetpad-dap"]']
    for platform, os_name, arch in (("linux-x64", "linux", "x86_64"), ("linux-arm64", "linux", "aarch64"),
                                    ("macos-universal", "darwin", None)):
        path = directory / f"sweetpad-cli-{version}-{platform}.tar.gz"
        if not path.is_file():
            raise FileNotFoundError(path)
        lines += ["", "[[artifact]]", f"path = {json.dumps(str(path))}", f'os = "{os_name}"']
        if arch:
            lines += [f'arch = "{arch}"', 'libc = "any"']
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.write_text(manifest(args.directory, args.version))
