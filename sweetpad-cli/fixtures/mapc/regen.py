#!/usr/bin/env python3
"""Regenerate the native Core Data mapping archive with the user's Xcode."""
from pathlib import Path
import subprocess

here = Path(__file__).resolve().parent
for model in sorted(here.glob("*.xcmappingmodel")):
    subprocess.run(["xcrun", "mapc", "--module", "Fixture", str(model),
                    str(here / (model.stem + ".cdm"))], check=True)
