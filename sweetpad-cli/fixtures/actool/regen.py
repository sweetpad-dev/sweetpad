"""Regenerate the actool stand-in's expected output from Xcode's own actool.

Builds the catalog that probe.json describes, the same way the golden test in
sweetpad-cli/src/cli/actool builds it, and runs `xcrun actool` on it once per
symbol mode. Run it on a Mac after an Xcode update and review the diff:

    python3 -I sweetpad-cli/fixtures/actool/regen.py
"""

import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
INFO = {"info": {"author": "xcode", "version": 1}}
RED = {"color-space": "srgb", "components": {"red": "1.000", "green": "0.000", "blue": "0.000", "alpha": "1.000"}}

# The modes Swift Build asks for: the default, ASSETCATALOG_COMPILER_GENERATE_SWIFT_ASSET_SYMBOL_EXTENSIONS=NO,
# and ASSETCATALOG_COMPILER_GENERATE_ASSET_SYMBOL_FRAMEWORKS=SwiftUI.
MODES = {
    "extensions": [],
    "no-extensions": ["--generate-swift-asset-symbol-extensions", "NO"],
    "swiftui": ["--generate-asset-symbol-framework-support", "SwiftUI"],
}


def write_json(path, value):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(value, f, indent=2)


def png(path):
    rows = b"".join(b"\x00" + b"\xff\x00\x00\xff" * 4 for _ in range(4))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    header = struct.pack(">IIBBBBB", 4, 4, 8, 6, 0, 0, 0)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def build(spec, catalog):
    write_json(os.path.join(catalog, "Contents.json"), INFO)
    for asset in spec["assets"]:
        parts = asset["path"].split("/")
        for depth in range(1, len(parts)):
            folder = "/".join(parts[:depth])
            props = {"properties": {"provides-namespace": True}} if folder in spec["namespaces"] else {}
            write_json(os.path.join(catalog, folder, "Contents.json"), {**props, **INFO})
        idioms = asset.get("idioms", ["universal"])
        empty = asset.get("empty", False)
        kind = asset["kind"]
        if kind == "color":
            colors = [{"idiom": i} if empty else {"color": RED, "idiom": i} for i in idioms]
            write_json(os.path.join(catalog, asset["path"] + ".colorset", "Contents.json"), {"colors": colors, **INFO})
        elif kind == "image":
            folder = os.path.join(catalog, asset["path"] + ".imageset")
            os.makedirs(folder, exist_ok=True)
            if not empty:
                png(os.path.join(folder, "img.png"))
            images = [{"idiom": i, "scale": "1x", **({} if empty else {"filename": "img.png"})} for i in idioms]
            write_json(os.path.join(folder, "Contents.json"), {"images": images, **INFO})
        elif kind == "data":
            folder = os.path.join(catalog, asset["path"] + ".dataset")
            os.makedirs(folder, exist_ok=True)
            with open(os.path.join(folder, "blob.bin"), "wb") as f:
                f.write(b"x")
            write_json(os.path.join(folder, "Contents.json"), {"data": [{"filename": "blob.bin", "idiom": "universal"}], **INFO})
        elif kind == "appicon":
            folder = os.path.join(catalog, asset["path"] + ".appiconset")
            os.makedirs(folder, exist_ok=True)
            png(os.path.join(folder, "icon.png"))
            images = [{"filename": "icon.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"}]
            write_json(os.path.join(folder, "Contents.json"), {"images": images, **INFO})
        else:
            sys.exit(f"unknown asset kind {kind}")


def main():
    with open(os.path.join(HERE, "probe.json")) as f:
        spec = json.load(f)
    expected = os.path.join(HERE, "expected")
    os.makedirs(expected, exist_ok=True)
    work = tempfile.mkdtemp(prefix="sweetpad-actool-")
    try:
        catalog = os.path.join(work, "Probe.xcassets")
        build(spec, catalog)
        for mode, flags in MODES.items():
            out = os.path.join(work, mode)
            os.makedirs(out)
            subprocess.run(
                ["xcrun", "actool", catalog, "--compile", out, "--output-format", "human-readable-text",
                 "--notices", "--warnings", "--output-partial-info-plist", os.path.join(out, "partial.plist"),
                 "--app-icon", "AppIcon", "--accent-color", "AccentColor", "--platform", "iphonesimulator",
                 "--minimum-deployment-target", "18.2", "--target-device", "iphone", "--target-device", "ipad",
                 "--bundle-identifier", "dev.sweetpad.Probe", *flags,
                 "--generate-swift-asset-symbols", os.path.join(out, "Symbols.swift"),
                 "--generate-objc-asset-symbols", os.path.join(out, "Symbols.h"),
                 "--generate-asset-symbol-index", os.path.join(out, "Symbols-Index.plist")],
                check=True, stdout=subprocess.DEVNULL)
            shutil.copy(os.path.join(out, "Symbols.swift"), os.path.join(expected, f"{mode}.swift"))
            if mode != "extensions":
                # The header and the index don't change with the mode.
                continue
            shutil.copy(os.path.join(out, "Symbols.h"), os.path.join(expected, "symbols.h"))
            index = json.loads(subprocess.run(
                ["plutil", "-convert", "json", "-o", "-", os.path.join(out, "Symbols-Index.plist")],
                check=True, capture_output=True, text=True).stdout)
            # The test's catalog lives somewhere else.
            for entries in index.values():
                for entry in entries:
                    entry["catalogPath"] = "$(CATALOG)"
            with open(os.path.join(expected, "index.json"), "w") as f:
                json.dump(index, f, indent=1, ensure_ascii=False)
                f.write("\n")
    finally:
        shutil.rmtree(work)


main()
