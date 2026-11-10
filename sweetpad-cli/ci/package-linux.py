#!/usr/bin/env python3
"""Package the Linux ARM64 CLI and public toolchain bootstrap sources only."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile

REPO = Path(__file__).resolve().parents[2]
BOOTSTRAP = [
    "sweetpad-lib/ci/linux-setup.sh",
    "sweetpad-lib/ci/linux-sdk.py",
    "sweetpad-lib/ci/linux-macro-launcher.sh",
    "sweetpad-lib/ci/linux-sdk-platforms.py",
    "sweetpad-lib/ci/linux-appintents-resources.py",
    "sweetpad-lib/ci/appintents_model.py",
    "sweetpad-lib/ci/swift-build-linux.patch",
    "sweetpad-lib/ci/swift-build-generic-debug-bridges.patch",
    "sweetpad-lib/ci/swift-llbuild-directory-race.patch",
    "sweetpad-cli/macros/Package.swift",
    "sweetpad-cli/macros/Package.resolved",
    "sweetpad-cli/fixtures/appintents/extract.actionsdata",
    "sweetpad-cli/fixtures/appintents/version.json",
]


def validate_elf(path):
    with Path(path).open("rb") as source:
        header = source.read(20)
    if header[:6] != b"\x7fELF\x02\x01" or int.from_bytes(header[18:20], "little") != 183:
        raise ValueError("the release binary must be a little-endian ARM64 ELF executable")


def source_files(repo):
    files = {"README.md": repo / "sweetpad-cli/ci/linux-release-README.md",
             "install.sh": repo / "sweetpad-cli/ci/install-linux.sh",
             "LINUX_DEVICES.md": repo / "sweetpad-cli/LINUX_DEVICES.md",
             "LICENSE.md": repo / "LICENSE.md",
             "licenses/LICENSE-MIT": repo / "sweetpad-lib/LICENSE-MIT",
             "licenses/LICENSE-APACHE": repo / "sweetpad-lib/LICENSE-APACHE"}
    for name in BOOTSTRAP:
        files["setup/" + name] = repo / name
    for path in sorted((repo / "sweetpad-cli/macros/Sources").rglob("*")):
        if path.is_file() and path.suffix in (".swift", ".md"):
            files["setup/" + str(path.relative_to(repo))] = path
    example = repo / "sweetpad-lib/ci/fixture-linux"
    for path in sorted(example.rglob("*")):
        if path.is_file() and path.suffix in (".swift", ".pbxproj", ".xcscheme", ".xcworkspacedata"):
            files["examples/LinuxApp/" + str(path.relative_to(example))] = path
    return files


def package(binary, version, out, repo=REPO):
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?", version):
        raise ValueError("invalid package version")
    validate_elf(binary)
    binary_version = subprocess.check_output([str(binary.resolve()), "--version"], text=True).strip()
    expected = version.removesuffix("-dryrun")
    if not (binary_version == f"sweetpad {expected}" or binary_version.startswith(f"sweetpad {expected}-dev+")):
        raise ValueError(f"binary version {binary_version!r} differs from package version {version!r}")
    files = {"sweetpad": binary, **source_files(repo)}
    receipt = {"packageVersion": version, "binaryVersion": binary_version,
               "platform": "linux-arm64", "baseline": "Ubuntu 24.04 / glibc 2.39",
               "experimental": True, "appleSdkIncluded": False,
               "files": {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in sorted(files.items())}}
    out.mkdir(parents=True, exist_ok=True)
    archive = out / f"sweetpad-cli-{version}-linux-arm64.tar.gz"
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
    with archive.open("xb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as tar:
            for name, path in sorted(files.items()):
                data = path.read_bytes()
                entry = tarfile.TarInfo(name)
                entry.size = len(data)
                entry.mode = 0o755 if name == "sweetpad" or name.endswith(".sh") else 0o644
                entry.mtime = epoch
                tar.addfile(entry, io.BytesIO(data))
            link = tarfile.TarInfo("sweetpad-dap")
            link.type, link.linkname, link.mode, link.mtime = tarfile.SYMTYPE, "sweetpad", 0o777, epoch
            tar.addfile(link)
            data = (json.dumps(receipt, indent=2) + "\n").encode()
            entry = tarfile.TarInfo("manifest.json")
            entry.size, entry.mode, entry.mtime = len(data), 0o644, epoch
            tar.addfile(entry, io.BytesIO(data))
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(archive.suffix + ".sha256").write_text(f"{checksum}  {archive.name}\n")
    return {"archive": str(archive), "sha256": checksum, **receipt}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(package(args.binary, args.version, args.out), indent=2))
