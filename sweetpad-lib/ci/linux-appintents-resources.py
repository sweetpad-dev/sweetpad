#!/usr/bin/env python3
"""Export phrase expansions from the user's Xcode for its private Darwin SDK.

Run on macOS. This trains a synthetic shortcut, reads the generated resources,
and replaces the probe app name with a placeholder. It never copies an app's
models into another product. The exported phrase data stays outside source control.
"""
import argparse
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile

from appintents_model import read

CI = Path(__file__).resolve().parent
LOCALES = "af am ar az be bg bn ca cs da de el en en-AU en-GB en-US es es-MX et eu fa fi fil fr fr-CA gl gu he hi hr hu hy id is it ja ka kk km kmr kn ko ku-TR lo lt lv mk ml mn mr ms my nb ne nl nn no pa pl pt pt-BR pt-PT ro ru sk sl sq sr sv sw ta te th tr uk ur uz vi zh-Hans zh-Hant zh-HK".split()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--xcode", type=Path, default=Path("/Applications/Xcode.app"))
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--locale", action="append", default=[], help="Additional project locale to export")
    args = parser.parse_args()
    developer = args.xcode / "Contents/Developer"
    tool = developer / "Toolchains/XcodeDefault.xctoolchain/usr/bin/appintentsnltrainingprocessor"
    fixtures = CI.parents[1] / "sweetpad-cli/fixtures/appintents"
    probe = "SweetpadPhraseResourceProbe"
    phrase = "Greet in " + probe
    locales_to_export = sorted(set(LOCALES + args.locale))
    if any(not locale or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-" for c in locale) for locale in locales_to_export):
        parser.error("--locale must be a language identifier")
    with tempfile.TemporaryDirectory(prefix="sweetpad-phrase-export-") as work:
        root = Path(work)
        app = root / "Probe.app"
        metadata = app / "Metadata.appintents"
        metadata.mkdir(parents=True)
        for name in ["extract.actionsdata", "version.json"]:
            shutil.copyfile(fixtures / name, metadata / name)
        for locale in locales_to_export:
            (app / (locale + ".lproj")).mkdir()
        info = app / "Info.plist"
        info.write_bytes(plistlib.dumps({"CFBundleIdentifier": "dev.sweetpad.PhraseResourceProbe",
                                        "CFBundleDisplayName": probe, "CFBundleDevelopmentRegion": "en"}))
        command = [str(tool), "--product-path", str(app), "--infoplist-path", str(info),
                   "--extracted-metadata-path", str(metadata), "--temp-dir-path", str(root / "ssu"),
                   "--bundle-id", "dev.sweetpad.PhraseResourceProbe", "--archive-ssu-assets"]
        subprocess.run(command, check=True, capture_output=True, text=True, timeout=120)
        locales = {}
        for locale in locales_to_export:
            model, _ = read(app / (locale + ".lproj/nlu.appintents/nlu.lzfse"))
            utterances, = model["intents"].values()
            if not all(p.endswith(phrase) for p in utterances):
                raise ValueError(f"{locale}: unexpected probe expansion")
            prefixes = sorted(p[:-len(phrase)] for p in utterances)
            negatives = [p.replace(probe, "${applicationName}") for p in model["negatives"]]
            locales[locale] = {"prefixes": prefixes, "negatives": negatives}
        version = plistlib.loads((args.xcode / "Contents/version.plist").read_bytes())
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps({"version": 1, "xcodeBuild": version["ProductBuildVersion"],
                                        "locales": locales}, ensure_ascii=False, indent=2) + "\n")
        print(f"Exported {len(locales)} locale expansions from Xcode {version['ProductBuildVersion']}")


if __name__ == "__main__":
    main()
