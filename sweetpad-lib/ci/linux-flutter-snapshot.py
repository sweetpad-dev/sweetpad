#!/usr/bin/env python3
"""Build the pinned Flutter fixture's iOS AOT compiler on ARM64 Linux.

Builds only gen_snapshot, with Linux host code and iOS target code generation.
It does not build or replace Flutter.framework, or change the installed SDK.
Needs Python 3.12+, Ninja, and a Clang installation with Linux resource headers.
"""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import re
import subprocess
import tarfile
import urllib.request
import zipfile

DART = "416e33b0ffaef062a60ec966bb8c82ed2b83f128"
GN = "b8becbd4f7979f63ab3542524b2b4d8db2ce3b52"
DEPENDENCIES = {
    "third_party/zlib": "https://chromium.googlesource.com/chromium/src/third_party/zlib/+archive/3008c4b3a06bd65392c31db8846000a21e3d03c5.tar.gz",
    "third_party/icu": "https://chromium.googlesource.com/chromium/deps/icu/+archive/a86a32e67b8d1384b33f8fa48c83a6079b86f8cd.tar.gz",
    "third_party/boringssl/src": "https://boringssl.googlesource.com/boringssl/+archive/c7eb007fd10627f1abb41a3e368b85e24cd20af9.tar.gz",
}
GN_ARGS = ('target_os="ios" target_cpu="arm64" host_cpu="arm64" '
           'dart_target_arch="arm64" is_debug=false is_release=false '
           'is_product=true dart_debug=false dart_runtime_mode="release" '
           'is_clang=true dart_use_compressed_pointers=false dart_sysroot="" '
           'dart_use_crashpad=false')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--flutter-root", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True, help="Fresh source and compiler directory")
    parser.add_argument("--clang-root", type=Path, required=True, help="Clang prefix containing bin/ and lib/clang/")
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() not in ["aarch64", "arm64"]:
        raise RuntimeError("This recipe supports ARM64 Linux hosts")
    revision = re.search(r"'dart_revision':\s*'([a-f0-9]+)'", (args.flutter_root / "DEPS").read_text())
    if not revision or revision[1] != DART:
        raise RuntimeError("Flutter's Dart revision differs from this pinned recipe")
    root = args.out.resolve()
    root.mkdir(parents=True, exist_ok=False)
    receipt = {"dartRevision": DART, "gnRevision": GN, "downloads": [], "commands": [],
               "scope": "Linux-hosted iOS ARM64 gen_snapshot only; uncompressed pointers; product mode"}

    def download(url, name):
        target = root / name
        urllib.request.urlretrieve(url, target)
        receipt["downloads"].append({"url": url, "sha256": hashlib.sha256(target.read_bytes()).hexdigest(), "bytes": target.stat().st_size})
        return target

    archive = download("https://codeload.github.com/dart-lang/sdk/tar.gz/" + DART, "dart.tar.gz")
    with tarfile.open(archive) as source:
        source.extractall(root, filter="data")
    source_root = root / ("sdk-" + DART)
    for child in source_root.iterdir():
        child.rename(root / child.name)
    source_root.rmdir()
    archive.unlink()
    with (root / ".gn").open("a") as config:
        config.write('\nroot = "//runtime/bin:gen_snapshot_product"\nroot_patterns = [ "//runtime/bin:gen_snapshot_product" ]\n')
    for destination, url in DEPENDENCIES.items():
        archive = download(url, "dependency.tar.gz")
        with tarfile.open(archive) as source:
            source.extractall(root / destination, filter="data")
        archive.unlink()
    archive = download("https://chrome-infra-packages.appspot.com/dl/gn/gn/linux-arm64/+/git_revision:" + GN, "gn.zip")
    with zipfile.ZipFile(archive) as source:
        source.extract("gn", root / "buildtools")
    archive.unlink()
    (root / "buildtools/gn").chmod(0o755)

    # The standard iOS GN configuration selects a Mac host. For this one host
    # tool, keep target_os=ios (DART_TARGET_OS_MACOS_IOS) and current_os=linux.
    path = root / "build/config/BUILDCONFIG.gn"
    text = path.read_text()
    marker = 'if (current_os == "") {'
    assert text.count(marker) == 1
    path.write_text(text.replace(marker, 'if (target_os == "ios" && host_os == "linux") {\n  current_os = "linux"\n}\n\n' + marker))
    # GN evaluates every target in a loaded file. Exclude unrelated SDK,
    # runtime, and test targets, while retaining gen_snapshot's dart:io subset.
    path = root / "runtime/bin/BUILD.gn"
    text = path.read_text()
    marker = "# A source set for the implementation of 'dart:io' library."
    assert marker in text
    path.write_text(text.split(marker)[0])
    (root / "build/config/gclient_args.gni").write_text("build_devtools_from_sources = false\n")
    clang = root / "buildtools/linux-arm64/clang"
    clang.mkdir(parents=True)
    for name in ["bin", "lib"]:
        (clang / name).symlink_to(args.clang_root.resolve() / name)
    # These identities affect rebuild invalidation, not compiler trust. Record
    # actual local versions instead of claiming a downloaded CIPD toolchain.
    for path, value in [(clang / ".versions/clang.cipd_version", "local-clang"),
                        (root / "buildtools/sysroot/linux/.versions/sysroot.cipd_version", "local-linux-sysroot")]:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"instance_id": value}))

    def run(command):
        receipt["commands"].append(command)
        with (root / "build.log").open("a") as log:
            subprocess.run(command, cwd=root, stdout=log, stderr=log, check=True)

    gn = str(root / "buildtools/gn")
    target = "//runtime/bin:gen_snapshot_product"
    run([gn, "gen", "out/SweetpadIOS", "--root-target=" + target, "--root-pattern=" + target, "--args=" + GN_ARGS])
    run(["ninja", "-C", "out/SweetpadIOS", "-j" + str(args.jobs), "gen_snapshot_product"])
    compiler = root / "out/SweetpadIOS/gen_snapshot_product"
    data = compiler.read_bytes()
    if data[:6] != b"\x7fELF\x02\x01" or int.from_bytes(data[18:20], "little") != 183:
        raise RuntimeError("The result is not a Linux ARM64 ELF compiler")
    defines = subprocess.check_output([gn, "desc", "out/SweetpadIOS", target, "defines"], cwd=root, text=True)
    assert "DART_TARGET_OS_MACOS_IOS" in defines
    receipt.update({"compiler": str(compiler), "sha256": hashlib.sha256(data).hexdigest(),
                    "version": subprocess.check_output([str(compiler), "--version"], text=True, stderr=subprocess.STDOUT).strip(),
                    "clang": subprocess.check_output([str(args.clang_root / "bin/clang"), "--version"], text=True),
                    "defines": defines.splitlines()})
    (root / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(compiler)


if __name__ == "__main__":
    main()
