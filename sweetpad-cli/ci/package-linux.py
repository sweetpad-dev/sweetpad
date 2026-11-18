#!/usr/bin/env python3
"""Package the static CLI and its Swift helpers as an archive and a Debian package."""
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

# .parent, unlike .parents[2], also works where the CI folder is mounted alone.
REPO = Path(__file__).resolve().parent.parent.parent


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


HELPERS = ("swbuild", "SWBBuildServiceBundle", "sweetpad-macros")
# What the helpers may load from the host. Everything else comes from
# libexec/sweetpad, through a RUNPATH of $ORIGIN alone.
SYSTEM_LIBRARIES = {"libc.so.6", "libm.so.6", "libdl.so.2", "libpthread.so.0", "librt.so.1", "libutil.so.1",
                    "ld-linux-aarch64.so.1", "ld-linux-x86-64.so.2", "libstdc++.so.6", "libgcc_s.so.1",
                    "libsqlite3.so.0"}


# Debian 12, the oldest host the package supports: glibc 2.36 and GCC 12's
# libstdc++. Helpers built on a newer host ask for newer symbol versions.
GLIBC_FLOOR = (2, 36)
GLIBCXX_FLOOR = (3, 4, 30)


def symbol_versions(path, prefix):
    """The highest PREFIX_x.y[.z] version a file names, as a tuple."""
    found = re.findall(rb"\b" + prefix + rb"_(\d+(?:\.\d+)+)\b", Path(path).read_bytes())
    return max((tuple(int(part) for part in version.split(b".")) for version in found), default=())


def check_file_floor(path):
    for prefix, floor in ((b"GLIBC", GLIBC_FLOOR), (b"GLIBCXX", GLIBCXX_FLOOR)):
        version = symbol_versions(path, prefix)
        if version > floor:
            wanted = ".".join(map(str, version))
            raise ValueError(f"libexec/sweetpad/{path.name} needs {prefix.decode()}_{wanted}, newer than Debian 12's; build it on Debian 12")


def check_host_floor(libexec):
    for path in sorted(libexec.iterdir()):
        if path.name in HELPERS or ".so" in path.name:
            check_file_floor(path)


def elf_dynamic(path):
    """The DT_NEEDED names and the DT_RUNPATH/DT_RPATH of a 64-bit ELF file."""
    data = Path(path).read_bytes()
    offset = struct.unpack_from("<Q", data, 32)[0]
    size, count = struct.unpack_from("<HH", data, 54)
    loads, dynamic = [], None
    for i in range(count):
        kind, _, start, vaddr, _, length, _, _ = struct.unpack_from("<IIQQQQQQ", data, offset + i * size)
        if kind == 1:
            loads.append((vaddr, start, length))
        elif kind == 2:
            dynamic = (start, length)
    if dynamic is None:
        return [], None
    entries = [struct.unpack_from("<qQ", data, entry) for entry in range(dynamic[0], dynamic[0] + dynamic[1], 16)]
    strtab = next(value for tag, value in entries if tag == 5)
    base = next(start + strtab - vaddr for vaddr, start, length in loads if vaddr <= strtab < vaddr + length)

    def string(at):
        return data[base + at:data.index(b"\0", base + at)].decode()
    needed = [string(value) for tag, value in entries if tag == 1]
    paths = [string(value) for tag, value in entries if tag in (15, 29)]
    return needed, paths[0] if paths else None


def check_helper_linkage(libexec):
    """Every helper and bundled library finds what it loads beside it or on the host."""
    bundled = {path.name for path in libexec.glob("*.so*")}
    for path in [*(libexec / name for name in HELPERS), *(libexec / name for name in sorted(bundled))]:
        needed, runpath = elf_dynamic(path)
        # A library that loads only host libraries needs no RUNPATH.
        if runpath != "$ORIGIN" and (path.name in HELPERS or runpath is not None or set(needed) & bundled):
            raise ValueError(f"libexec/sweetpad/{path.name} must have a RUNPATH of $ORIGIN alone, not {runpath!r}")
        missing = sorted(set(needed) - bundled - SYSTEM_LIBRARIES)
        if missing:
            raise ValueError(f"libexec/sweetpad/{path.name} needs {', '.join(missing)}, which is neither bundled nor a host library")


def helper_files(helpers, arch):
    """The tree build-swift-helpers.sh stages: libexec/sweetpad and licenses."""
    files = {}
    for top in ("libexec", "licenses"):
        for path in sorted((helpers / top).rglob("*")):
            if path.is_symlink() or not (path.is_file() or path.is_dir()):
                raise ValueError(f"helpers must be regular files: {path}")
            if path.is_file():
                files[str(path.relative_to(helpers))] = path
    for name in HELPERS:
        path = helpers / "libexec/sweetpad" / name
        if not path.is_file():
            raise ValueError(f"helpers lack libexec/sweetpad/{name}")
        if validate_elf(path) != arch:
            raise ValueError(f"libexec/sweetpad/{name} is not built for linux-{arch}")
    if not any(name.startswith("libexec/sweetpad/SwiftBuild_") for name in files):
        raise ValueError("helpers lack Swift Build's resource bundles")
    record = json.loads((helpers / "libexec/sweetpad/sweetpad-swift-build.json").read_text())
    pins = json.loads((REPO / "sweetpad-lib/ci/linux-toolchain.json").read_text())
    for key in ("swiftBuild", "swift"):
        if record.get(key) != pins[key]:
            raise ValueError(f"helpers record {key} {record.get(key)!r}; the CLI pins {pins[key]}")
    check_helper_linkage(helpers / "libexec/sweetpad")
    check_host_floor(helpers / "libexec/sweetpad")
    return files


LINKER = "libexec/sweetpad/llvm"


def linker_files(linker, arch):
    """The tree build-darwin-linker.sh stages: LLVM's multicall LLD and
    dsymutil, its record, and LLVM's licenses."""
    files = {}
    for top in ("libexec", "licenses"):
        for path in sorted((linker / top).rglob("*")):
            if path.is_symlink() or not (path.is_file() or path.is_dir()):
                raise ValueError(f"the linker tree must hold regular files: {path}")
            if path.is_file():
                files[str(path.relative_to(linker))] = path
    expected = {LINKER, "libexec/sweetpad/sweetpad-llvm.json"}
    if not expected <= set(files) or not any(name.startswith("licenses/llvm/") for name in files):
        raise ValueError(f"the linker tree must hold {', '.join(sorted(expected))} and licenses/llvm")
    extra = sorted(name for name in files if name.startswith("libexec/") and name not in expected)
    if extra:
        raise ValueError(f"the linker tree holds unexpected files: {', '.join(extra)}")
    binary = linker / LINKER
    if validate_elf(binary) != arch:
        raise ValueError(f"{LINKER} is not built for linux-{arch}")
    record = json.loads((linker / "libexec/sweetpad/sweetpad-llvm.json").read_text())
    pins = json.loads((REPO / "sweetpad-lib/ci/linux-toolchain.json").read_text())
    if (record.get("llvm"), record.get("sourceSha256")) != (pins["llvm"], pins["llvmSourceSha256"]):
        raise ValueError(f"the linker records LLVM {record.get('llvm')!r}; the CLI pins {pins['llvm']}")
    # It loads only host libraries, so it has no RUNPATH at all.
    needed, runpath = elf_dynamic(binary)
    if runpath is not None:
        raise ValueError(f"{LINKER} must have no RUNPATH, not {runpath!r}")
    missing = sorted(set(needed) - SYSTEM_LIBRARIES)
    if missing:
        raise ValueError(f"{LINKER} needs {', '.join(missing)}, which isn't a host library")
    check_file_floor(binary)
    return files


def executable(name):
    return name in ("sweetpad", LINKER, *(f"libexec/sweetpad/{helper}" for helper in HELPERS)) or name.endswith(".sh")


def source_files(repo):
    files = {"README.md": repo / "sweetpad-cli/ci/linux-release-README.md",
             "LINUX_DEVICES.md": repo / "sweetpad-cli/LINUX_DEVICES.md",
             "LICENSE.md": repo / "LICENSE.md",
             "licenses/LICENSE-MIT": repo / "sweetpad-lib/LICENSE-MIT",
             "licenses/LICENSE-APACHE": repo / "sweetpad-lib/LICENSE-APACHE"}
    for path in sorted((repo / "sweetpad-cli/macros/Sources").rglob("LICENSE*")):
        if path.is_file():
            files["licenses/macros/" + str(path.relative_to(repo / "sweetpad-cli/macros/Sources"))] = path
    # The LZRAVEN decoder is a port of liblzraven's.
    files["licenses/lzraven/LICENSE"] = repo / "sweetpad-cli/src/cli/apple_archive/LICENSE-lzraven"
    return files


def package(binary, helpers, linker, version, out, repo=REPO):
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?", version):
        raise ValueError("invalid package version")
    arch = validate_elf(binary)
    validate_static(binary)
    binary_version = subprocess.check_output([str(binary.resolve()), "--version"], text=True).strip()
    if not valid_binary_version(binary_version, version):
        raise ValueError(f"binary version {binary_version!r} differs from package version {version!r}")
    files = {"sweetpad": binary, **source_files(repo), **helper_files(helpers, arch), **linker_files(linker, arch)}
    receipt = {"packageVersion": version, "binaryVersion": binary_version,
               "platform": "linux-" + arch, "linkage": "static", "embeddedSetup": True,
               "baseline": "Static Linux CLI; Apple build toolchain setup supports Ubuntu 24.04, Debian 12/13 and Arch x64",
               "experimental": True, "appleSdkIncluded": False,
               "swiftHelpers": json.loads((helpers / "libexec/sweetpad/sweetpad-swift-build.json").read_text()),
               "darwinLinker": json.loads((linker / "libexec/sweetpad/sweetpad-llvm.json").read_text()),
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
                entry.mode = 0o755 if executable(name) else 0o644
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


def debian_depends(repo=REPO):
    """The libraries the Swift helpers and LLVM's linker load, then the host
    packages `sweetpad setup` would install, so that apt installs them and
    setup finds them ready. LLVM is versioned on Ubuntu and not on Debian."""
    hosts = json.loads((repo / "sweetpad-lib/ci/linux-toolchain.json").read_text())["hostPackages"]
    depends = ["ca-certificates", "libc6 (>= 2.36)", "libgcc-s1", "libstdc++6", "libsqlite3-0"]
    depends += [name for name in hosts["apt"] if name not in depends]
    llvm = hosts["aptLlvm"]
    depends.append(f"{llvm['ubuntu']} | {llvm['debian']}")
    return depends


def debian_package(binary, helpers, linker, version, out, revision=1, repo=REPO):
    arch = {"x64": "amd64", "arm64": "arm64"}[validate_elf(binary)]
    validate_static(binary)
    shipped = {**helper_files(helpers, validate_elf(binary)), **linker_files(linker, validate_elf(binary))}
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
        # sweetpad finds its helpers in /usr/libexec/sweetpad, beside /usr/bin.
        for name, source in shipped.items():
            path = stage / "usr" / name if name.startswith("libexec/") else stage / "usr/share/doc/sweetpad" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(source.read_bytes())
            path.chmod(0o755 if executable(name) else 0o644)
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
            f"Depends: {', '.join(debian_depends(repo))}\n"
            "Homepage: https://sweetpad.hyzyla.dev\n"
            "Description: Build Apple projects with the Sweetpad CLI\n"
            " Static CLI with its sample app, and the Swift Build,\n"
            " macro plugin and LLVM linker binaries it runs on Linux. Apple SDKs\n"
            " and signing material are supplied separately.\n"
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
    parser.add_argument("--helpers", type=Path, required=True, help="output of build-swift-helpers.sh")
    parser.add_argument("--linker", type=Path, required=True, help="output of build-darwin-linker.sh")
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--deb", action="store_true", help="also build a Debian package")
    parser.add_argument("--deb-revision", type=int, default=1)
    args = parser.parse_args()
    report = package(args.binary, args.helpers, args.linker, args.version, args.out)
    if args.deb:
        report["debian"] = debian_package(args.binary, args.helpers, args.linker, args.version, args.out, args.deb_revision)
    print(json.dumps(report, indent=2))
