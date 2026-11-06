#!/usr/bin/env python3
"""Record what Xcode's ibtool makes of each storyboard here, into expected/,
with the options Swift Build passes it. Needs Xcode (macOS)."""
import os
import shutil
import subprocess
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
EXPECTED = os.path.join(HERE, "expected")

os.makedirs(EXPECTED, exist_ok=True)
with open(os.path.join(EXPECTED, "version.plist"), "wb") as out:
    subprocess.run([
        "xcrun", "ibtool", "--version", "--output-format", "xml1",
    ], stdout=out, check=True)

with open(os.path.join(EXPECTED, "version.binary"), "wb") as out:
    subprocess.run(["xcrun", "ibtool", "--version", "--output-format", "binary1"], stdout=out, check=True)

sources = [os.path.join(HERE, name) for name in os.listdir(HERE) if name.endswith((".storyboard", ".xib"))]
for directory in os.listdir(HERE):
    if directory.endswith(".lproj"):
        sources.extend(os.path.join(HERE, directory, name) for name in os.listdir(os.path.join(HERE, directory)) if name.endswith((".storyboard", ".xib")))
for source in sorted(sources):
    name = os.path.basename(source)
    out = os.path.join(EXPECTED, name + "c" if name.endswith(".storyboard") else name[:-4] + ".nib")
    if os.path.isdir(out):
        shutil.rmtree(out)
    elif os.path.exists(out):
        os.unlink(out)
    os.makedirs(EXPECTED, exist_ok=True)
    runtime = ET.parse(source).getroot().get("targetRuntime")
    flags = {"watchKit": ["--target-device", "watch", "--minimum-deployment-target", "10.0"],
             "AppleTV": ["--target-device", "tv", "--minimum-deployment-target", "18.0"],
             "MacOSX.Cocoa": ["--minimum-deployment-target", "12.0"]}.get(runtime,
             ["--target-device", "iphone", "--target-device", "ipad", "--minimum-deployment-target", "18.0"])
    subprocess.run([
        "xcrun", "ibtool", "--errors", "--warnings", "--notices",
        "--module", "App", *flags, "--output-format", "human-readable-text",
        "--auto-activate-custom-fonts", "--compile", out,
        source,
    ], check=True)
    print(name, sorted(os.listdir(out)) if os.path.isdir(out) else os.path.getsize(out))
