#!/usr/bin/env python3
"""Download and checksum published CLI packages before distribution."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

REPO = "sweetpad-dev/sweetpad"


def collect(tag, out, history=0):
    if not re.fullmatch(r"cli-v\d+\.\d+\.\d+", tag):
        raise ValueError("distribution requires a stable cli-vX.Y.Z tag")
    out.mkdir(parents=True, exist_ok=True)
    tags = [tag]
    releases = json.loads(subprocess.check_output(["gh", "release", "list", "--repo", REPO, "--limit", "100",
                                                   "--json", "tagName,isDraft,isPrerelease"], text=True))
    candidates = [r["tagName"] for r in releases if not r["isDraft"] and not r["isPrerelease"]
                  and re.fullmatch(r"cli-v\d+\.\d+\.\d+", r["tagName"])]
    candidates.sort(key=lambda t: tuple(map(int, t[5:].split("."))), reverse=True)
    if candidates and candidates[0] != tag:
        raise ValueError("publish distribution channels only for the latest stable CLI release")
    if history:
        tags += [t for t in candidates if t != tag][:history - 1]
    for release_tag in tags:
        release = json.loads(subprocess.check_output(["gh", "release", "view", release_tag, "--repo", REPO,
                                                      "--json", "isDraft,isPrerelease,assets"], text=True))
        if release["isDraft"] or release["isPrerelease"]:
            raise ValueError("only published stable releases may be distributed")
        names = {a["name"] for a in release["assets"]}
        version = release_tag[5:]
        expected = [f"sweetpad_{version}-1_{arch}.deb" for arch in ("amd64", "arm64")]
        if release_tag == tag:
            expected += [f"sweetpad-cli-{version}-{platform}.tar.gz" for platform in ("linux-x64", "linux-arm64", "macos-universal")]
        for name in expected:
            if name not in names:
                if release_tag == tag:
                    raise ValueError(f"missing release artifact: {name}")
                continue  # Releases predating Debian packages have no pool entries.
            if name + ".sha256" not in names:
                raise ValueError(f"missing checksum: {name}")
            subprocess.run(["gh", "release", "download", release_tag, "--repo", REPO, "--dir", str(out),
                            "--pattern", name, "--pattern", name + ".sha256", "--clobber"], check=True)
            sidecar = (out / (name + ".sha256")).read_text().split()
            if len(sidecar) != 2 or sidecar[1] != name or hashlib.sha256((out / name).read_bytes()).hexdigest() != sidecar[0]:
                raise ValueError(f"checksum mismatch: {name}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--history", type=int, default=0)
    args = parser.parse_args()
    collect(args.tag, args.out, args.history)
