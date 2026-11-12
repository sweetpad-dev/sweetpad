#!/usr/bin/env python3
"""Package the standalone static CLI as an archive and a Debian package."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import struct
import shutil
import tarfile
import tempfile

REPO = Path(__file__).resolve().parents[2]


def validate_elf(path):
    with Path(path).open("rb") as source:
        header = source.read(20)
    if header[:6] != b"\x7fELF\x02\x01" or int.from_bytes(header[18:20], "little") not in (62, 183):
        raise ValueError("the release binary must be a little-endian ARM64 or x86_64 ELF executable")
    return "arm64" if int.from_bytes(header[18:20], "little") == 183 else "x64"


def validate_static(path):
    """Reject loader or shared-library dependencies, including static-PIE cases."""
    validate_elf(path)
    data = Path(path).read_bytes()
    if len(data) < 64:
        raise ValueError("truncated ELF header")
    offset = struct.unpack_from("<Q", data, 32)[0]
    size, count = struct.unpack_from("<HH", data, 54)
    if not count or size < 56 or offset + size * count > len(data):
        raise ValueError("invalid ELF program headers")
    for i in range(count):
        kind, _, start, _, _, length, _, _ = struct.unpack_from("<IIQQQQQQ", data, offset + i * size)
        if kind == 3:
            raise ValueError("release CLI must be static: ELF interpreter is present")
        if kind == 2:
            if length % 16 or start + length > len(data):
                raise ValueError("invalid ELF dynamic section")
            for entry in range(start, start + length, 16):
                tag, _ = struct.unpack_from("<qQ", data, entry)
                if tag == 0:
                    break
                if tag == 1:
                    raise ValueError("release CLI must be static: DT_NEEDED is present")


def source_files(repo):
    files = {"README.md": repo / "sweetpad-cli/ci/linux-release-README.md",
             "LINUX_DEVICES.md": repo / "sweetpad-cli/LINUX_DEVICES.md",
             "LICENSE.md": repo / "LICENSE.md",
             "licenses/LICENSE-MIT": repo / "sweetpad-lib/LICENSE-MIT",
             "licenses/LICENSE-APACHE": repo / "sweetpad-lib/LICENSE-APACHE"}
    for path in sorted((repo / "sweetpad-cli/macros/Sources").rglob("LICENSE*")):
        if path.is_file():
            files["licenses/macros/" + str(path.relative_to(repo / "sweetpad-cli/macros/Sources"))] = path
    codecs = repo / "sweetpad-lib/ci/archive-codecs"
    files["licenses/archive-codecs/README.md"] = codecs / "README.md"
    for path in sorted(codecs.rglob("LICENSE*")):
        if path.is_file():
            files["licenses/archive-codecs/" + str(path.relative_to(codecs))] = path
    return files


def package(binary, version, out, repo=REPO):
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?", version):
        raise ValueError("invalid package version")
    arch = validate_elf(binary)
    validate_static(binary)
    binary_version = subprocess.check_output([str(binary.resolve()), "--version"], text=True).strip()
    if not valid_binary_version(binary_version, version):
        raise ValueError(f"binary version {binary_version!r} differs from package version {version!r}")
    files = {"sweetpad": binary, **source_files(repo)}
    receipt = {"packageVersion": version, "binaryVersion": binary_version,
               "platform": "linux-" + arch, "linkage": "static", "embeddedSetup": True,
               "baseline": "Static Linux CLI; Apple build toolchain setup supports Ubuntu 24.04, Debian 12/13 and Arch x64",
               "experimental": True, "appleSdkIncluded": False,
               "files": {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in sorted(files.items())}}
    out.mkdir(parents=True, exist_ok=True)
    archive = out / f"sweetpad-cli-{version}-linux-{arch}.tar.gz"
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


def debian_package(binary, version, out, revision=1, repo=REPO):
    arch = {"x64": "amd64", "arm64": "arm64"}[validate_elf(binary)]
    validate_static(binary)
    # Debian sorts ~ prereleases before their stable release.
    upstream = version.replace("-", "~", 1)
    if revision < 1:
        raise ValueError("Debian revision must be positive")
    deb_version = f"{upstream}-{revision}"
    output = out.resolve() / f"sweetpad_{deb_version}_{arch}.deb"
    if output.exists():
        raise FileExistsError(output)
    out.mkdir(parents=True, exist_ok=True)
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
    with tempfile.TemporaryDirectory(prefix="sweetpad-deb-") as directory:
        stage = Path(directory)
        bin_dir = stage / "usr/bin"
        bin_dir.mkdir(parents=True)
        shutil.copyfile(binary, bin_dir / "sweetpad")
        (bin_dir / "sweetpad").chmod(0o755)
        (bin_dir / "sweetpad-dap").symlink_to("sweetpad")
        for name, source in source_files(repo).items():
            path = stage / "usr/share/doc/sweetpad" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(source.read_bytes())
            path.chmod(0o644)
        control = stage / "DEBIAN"
        control.mkdir()
        installed_kib = sum(
            max(1, (path.lstat().st_size + 1023) // 1024) if path.is_file() else 1
            for path in stage.rglob("*") if control not in path.parents and path != control
        )
        (control / "control").write_text(
            f"Package: sweetpad\nVersion: {deb_version}\nArchitecture: {arch}\n"
            f"Installed-Size: {installed_kib}\n"
            "Maintainer: Yevhenii Hyzyla <hyzyla@gmail.com>\nSection: devel\nPriority: optional\n"
            "Depends: ca-certificates, python3, bash\n"
            "Homepage: https://sweetpad.hyzyla.dev\n"
            "Description: Build Apple projects with the Sweetpad CLI\n"
            " Static CLI with embedded public setup files. Apple SDKs and signing\n"
            " material are supplied separately.\n"
        )
        (control / "control").chmod(0o644)
        for path in sorted(stage.rglob("*"), reverse=True):
            if path.is_dir():
                path.chmod(0o755)
            os.utime(path, (epoch, epoch), follow_symlinks=False)
        os.utime(stage, (epoch, epoch))
        subprocess.run(["dpkg-deb", "--build", "--root-owner-group", "--uniform-compression", "-Zxz", "-z6", str(stage), str(output)], check=True)
    checksum = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_name(output.name + ".sha256").write_text(f"{checksum}  {output.name}\n")
    return {"package": str(output), "version": deb_version, "architecture": arch, "sha256": checksum}


def valid_binary_version(binary_version, version):
    expected = version.removesuffix("-dryrun")
    return binary_version == f"sweetpad {expected}" or (version.endswith("-dryrun") and binary_version.startswith(f"sweetpad {expected}-dev+"))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--deb", action="store_true", help="also build a Debian package")
    parser.add_argument("--deb-revision", type=int, default=1)
    args = parser.parse_args()
    report = package(args.binary, args.version, args.out)
    if args.deb:
        report["debian"] = debian_package(args.binary, args.version, args.out, args.deb_revision)
    print(json.dumps(report, indent=2))
