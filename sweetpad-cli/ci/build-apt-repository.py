#!/usr/bin/env python3
"""Build and sign the public Sweetpad APT repository from release packages."""
import argparse
from datetime import datetime, timezone
import gzip
import hashlib
import lzma
from pathlib import Path
import shutil
import subprocess

ARCHES = ("amd64", "arm64")


def run(*args, **kwargs):
    return subprocess.check_output(list(map(str, args)), **kwargs)


def build(packages, out, key, public_key):
    if out.exists():
        raise FileExistsError(out)
    if not packages:
        raise ValueError("no Debian packages supplied")
    out.mkdir(parents=True)
    pool = out / "pool/main/s/sweetpad"
    pool.mkdir(parents=True)
    found = set()
    for package in sorted(packages):
        fields = run("dpkg-deb", "-f", package, "Package", "Version", "Architecture", text=True)
        metadata = dict(line.split(": ", 1) for line in fields.splitlines())
        if metadata["Package"] != "sweetpad" or metadata["Architecture"] not in ARCHES:
            raise ValueError(f"unexpected package: {package}")
        found.add(metadata["Architecture"])
        destination = pool / package.name
        if destination.exists() and destination.read_bytes() != package.read_bytes():
            raise ValueError(f"conflicting package name: {package.name}")
        shutil.copyfile(package, destination)
    release_dir = out / "dists/stable"
    indexes = []
    for arch in ARCHES:
        directory = release_dir / f"main/binary-{arch}"
        directory.mkdir(parents=True)
        data = run("dpkg-scanpackages", "--multiversion", "--arch", arch, "pool", "/dev/null", cwd=out)
        for name, content in (("Packages", data), ("Packages.gz", gzip.compress(data, mtime=0)),
                              ("Packages.xz", lzma.compress(data))):
            path = directory / name
            path.write_bytes(content)
            indexes.append(path)
            # Plain copies: GitHub Pages artifacts cannot contain hard links.
            hashed = directory / "by-hash/SHA256" / hashlib.sha256(content).hexdigest()
            hashed.parent.mkdir(parents=True, exist_ok=True)
            hashed.write_bytes(content)
    date = datetime.now(timezone.utc).strftime("%a, %d %b %Y %H:%M:%S +0000")
    lines = ["Origin: Sweetpad", "Label: Sweetpad", "Suite: stable", "Codename: stable",
             f"Date: {date}", "Architectures: amd64 arm64", "Components: main",
             "Description: Sweetpad CLI for Linux", "Acquire-By-Hash: yes", "SHA256:"]
    for path in sorted(indexes):
        data = path.read_bytes()
        lines.append(f" {hashlib.sha256(data).hexdigest()} {len(data):16} {path.relative_to(release_dir)}")
    release = release_dir / "Release"
    release.write_text("\n".join(lines) + "\n")
    signing = ["gpg", "--batch", "--yes", "--local-user", key, "--digest-algo", "SHA256"]
    subprocess.run([*signing, "--clearsign", "--output", str(release_dir / "InRelease"), str(release)], check=True)
    subprocess.run([*signing, "--armor", "--detach-sign", "--output", str(release_dir / "Release.gpg"), str(release)], check=True)
    shutil.copyfile(public_key, out / "sweetpad-archive-keyring.asc")
    (out / "index.html").write_text('<!doctype html><meta charset="utf-8"><title>Sweetpad APT repository</title>'
                                  '<p>Install instructions: <a href="https://sweetpad.hyzyla.dev/docs/cli/linux/install">Sweetpad Linux installation</a>.</p>\n')
    print(f"Signed APT repository: {out}; package architectures: {', '.join(sorted(found))}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--packages", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--signing-key", required=True, help="dedicated distribution key fingerprint")
    parser.add_argument("--public-key", type=Path, required=True)
    args = parser.parse_args()
    build(list(args.packages.glob("*.deb")), args.out.resolve(), args.signing_key, args.public_key)
