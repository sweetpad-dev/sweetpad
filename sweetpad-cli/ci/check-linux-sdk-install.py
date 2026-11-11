#!/usr/bin/env python3
"""Build the packaged example with a fresh home and explicit read-only tools.

Bubblewrap exposes no developer home, source checkout, signing files, or app
build cache. The private SDK and public compiler runtimes are prerequisites.
"""
import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import shutil
import subprocess
import tarfile


def check(args):
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    unpacked, work = out / "unpacked", out / "work"
    unpacked.mkdir()
    work.mkdir()
    with tarfile.open(args.archive) as source:
        source.extractall(unpacked, filter="data")
    manifest = json.loads((unpacked / "manifest.json").read_text())
    assert hashlib.sha256((unpacked / "sweetpad").read_bytes()).hexdigest() == manifest["files"]["sweetpad"]
    command = ["bwrap", "--die-with-parent", "--unshare-pid", "--unshare-ipc", "--unshare-uts",
               "--unshare-net", "--tmpfs", "/", "--ro-bind", "/usr", "/usr",
               "--ro-bind", "/etc", "/etc", "--symlink", "usr/bin", "/bin",
               "--symlink", "usr/sbin", "/sbin", "--symlink", "usr/lib", "/lib"]
    if Path("/usr/lib64").exists():
        command += ["--symlink", "usr/lib64", "/lib64"]
    command += ["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp",
                "--bind", str(work), "/work", "--ro-bind", str(unpacked), "/package",
                "--ro-bind", str(args.swift.resolve()), "/opt/swift",
                "--ro-bind", str(args.swbuild_products.resolve()), "/opt/swbuild",
                "--ro-bind", str(args.macros.resolve()), "/opt/bin/sweetpad-macros.bin",
                "--ro-bind", str(Path(__file__).resolve().parents[2] / "sweetpad-lib/ci/linux-macro-launcher.sh"),
                "/opt/bin/sweetpad-macros",
                "--symlink", "../swift/usr/bin/swift", "/opt/bin/sweetpad-macro-swift",
                "--ro-bind", str(args.sdk.resolve()), "/opt/darwin-sdk",
                "--chdir", "/work", "--clearenv", "--setenv", "HOME", "/work/home",
                "--setenv", "PATH", "/work/home/.local/bin:/opt/bin:/opt/swbuild:/opt/swift/usr/bin:/usr/bin:/bin",
                "--setenv", "LD_LIBRARY_PATH", "/opt/swift/usr/lib/swift/linux",
                "--setenv", "SWEETPAD_DARWIN_SDK", "/opt/darwin-sdk",
                "--setenv", "LANG", "C.UTF-8", "--setenv", "NO_COLOR", "1"]
    script = '''set -eu
mkdir -p "$HOME"
mkdir -p "$HOME/.local/bin"
cp /package/sweetpad "$HOME/.local/bin/sweetpad"
ln -s sweetpad "$HOME/.local/bin/sweetpad-dap"
test ! -e /Users
test ! -e /home
test ! -e /root
test ! -e /work/home/.cache
sweetpad project new LinuxApp --bundle-id dev.sweetpad.distribution --no-git --json > /work/project.json
mv /work/LinuxApp /work/Example
sweetpad build --project /work/Example/LinuxApp.xcodeproj --scheme LinuxApp \
  --destination 'generic/platform=iOS' --json --non-interactive -- \
  -derivedDataPath /work/DerivedData CODE_SIGNING_ALLOWED=NO > /work/build.json
'''
    result = subprocess.run([*command, "/bin/bash", "-c", script], capture_output=True, text=True, timeout=600)
    (out / "sandbox.stdout").write_text(result.stdout)
    (out / "sandbox.stderr").write_text(result.stderr)
    result.check_returncode()
    build = json.loads((work / "build.json").read_text())
    assert build["ok"] and build["data"]["built"]
    product = work / Path(build["data"]["productPath"]).relative_to("/work")
    executable = product / "LinuxApp"
    assert executable.is_file()
    description = subprocess.check_output(["file", str(executable)], text=True).strip()
    assert "Mach-O" in description and "arm64" in description
    installed = work / "home/.local/bin/sweetpad"
    assert hashlib.sha256(installed.read_bytes()).hexdigest() == manifest["files"]["sweetpad"]
    report = {"passed": True, "hostArchitecture": platform.machine(), "targetArchitecture": "arm64", "targetPlatform": "iOS device", "freshHome": True, "sourceCheckoutVisible": False,
              "developerHomeVisible": False, "networkAvailable": False, "appBuildCacheReused": False,
              "toolchainsReadOnly": True, "sdkReadOnly": True, "signingMaterialAvailable": False,
              "installedBinarySha256": manifest["files"]["sweetpad"],
              "product": str(product), "executableDescription": description,
              "scope": "Fresh app build from the installed release artifact; compiler and SDK installation are explicit prerequisites."}
    (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    if not args.keep_build_cache:
        for relative in ["Build/Intermediates.noindex", "ModuleCache.noindex", "SDKStatCaches.noindex", "Index.noindex"]:
            shutil.rmtree(work / "DerivedData" / relative, ignore_errors=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--swift", type=Path, required=True)
    parser.add_argument("--swbuild-products", type=Path, required=True)
    parser.add_argument("--macros", type=Path, required=True)
    parser.add_argument("--sdk", type=Path, required=True)
    parser.add_argument("--keep-build-cache", action="store_true")
    check(parser.parse_args())
