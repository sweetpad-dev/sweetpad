#!/usr/bin/env python3
"""Expose matching cached artifacts through Flutter's local-engine interface.

The engine framework stays prebuilt. Only the iOS AOT compiler is replaced with
the explicitly supplied Linux ELF build. No SDK files or global mounts change.
"""
import argparse
import json
from pathlib import Path


def prepare(flutter, compiler, out):
    with compiler.open("rb") as source:
        header = source.read(20)
    if header[:6] != b"\x7fELF\x02\x01" or int.from_bytes(header[18:20], "little") != 183:
        raise ValueError("needs the Linux ARM64 iOS gen_snapshot")
    out.mkdir(parents=True, exist_ok=False)
    target = out / "out/ios_release"
    host = out / "out/host_release"
    target.mkdir(parents=True)
    host.mkdir()
    cache = flutter / "bin/cache"
    for child in (cache / "artifacts/engine/ios-release").iterdir():
        if not child.name.startswith("gen_snapshot"):
            (target / child.name).symlink_to(child.resolve(), target_is_directory=child.is_dir())
    (target / "gen_snapshot_arm64").symlink_to(compiler.resolve())
    for name in ["flutter_patched_sdk", "flutter_patched_sdk_product"]:
        (target / name).symlink_to((cache / "artifacts/engine/common" / name).resolve(), target_is_directory=True)
    (host / "dart-sdk").symlink_to((cache / "dart-sdk").resolve(), target_is_directory=True)
    for child in (cache / "artifacts/engine/linux-arm64").iterdir():
        if not (host / child.name).exists():
            (host / child.name).symlink_to(child.resolve(), target_is_directory=child.is_dir())
    (host / "gen").mkdir(exist_ok=True)
    (host / "gen/const_finder.dart.snapshot").symlink_to((cache / "artifacts/engine/common/const_finder.dart.snapshot").resolve())
    settings = {"FLUTTER_ROOT": str(flutter.resolve()), "FLUTTER_ENGINE": str(out.resolve()),
                "LOCAL_ENGINE": "ios_release", "LOCAL_ENGINE_HOST": "host_release"}
    (out / "settings.json").write_text(json.dumps(settings, indent=2) + "\n")
    return settings


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--flutter-root", type=Path, required=True)
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.flutter_root, args.compiler, args.out), indent=2))
