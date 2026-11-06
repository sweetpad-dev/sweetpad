#!/usr/bin/env python3
"""Capture Xcode's intent class declarations; run on macOS with Xcode."""
from pathlib import Path
import subprocess
here=Path(__file__).resolve().parent
for source in here.glob('*.intentdefinition'):
    subprocess.run(['xcrun','intentbuilderc','generate','-input',str(source),'-output',str(here/'expected'),'-language','Swift','-swiftVersion','6.2','-visibility','public'],check=True)
