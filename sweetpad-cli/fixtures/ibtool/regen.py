#!/usr/bin/env python3
"""Record what Xcode's ibtool makes of each storyboard here, into expected/,
with the options Swift Build passes it. Needs Xcode (macOS)."""
import os
import shutil
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
EXPECTED = os.path.join(HERE, "expected")

for name in sorted(os.listdir(HERE)):
    if not name.endswith(".storyboard"):
        continue
    out = os.path.join(EXPECTED, name + "c")
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(EXPECTED, exist_ok=True)
    subprocess.run([
        "xcrun", "ibtool", "--errors", "--warnings", "--notices",
        "--module", "App", "--target-device", "iphone", "--target-device", "ipad",
        "--minimum-deployment-target", "18.0", "--output-format", "human-readable-text",
        "--auto-activate-custom-fonts", "--compilation-directory", EXPECTED,
        os.path.join(HERE, name),
    ], check=True)
    print(name, sorted(os.listdir(out)))
