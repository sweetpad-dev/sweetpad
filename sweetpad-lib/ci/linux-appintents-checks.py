#!/usr/bin/env python3
"""Compare localized shortcut training with Xcode, including incremental edits."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess

from appintents_model import read

CI = Path(__file__).resolve().parent


def digest(root):
    return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in root.rglob("*") if p.is_file() and (p.name == "nlu.lzfse" or p.suffix == ".version")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=CI.parent / "corpus/_linux-followups/validation/localized-shortcuts")
    parser.add_argument("--vm", default="sweetpad-linux")
    parser.add_argument("--xcode", type=Path, default=Path("/Applications/Xcode.app"))
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    fixtures = CI.parents[1] / "sweetpad-cli/fixtures/appintents"
    roots = {host: args.out / host / "LocaleProbe.app" for host in ["native", "linux"]}
    for root in roots.values():
        if root.exists():
            raise RuntimeError("choose a fresh --out directory")
        metadata = root / "Metadata.appintents"
        metadata.mkdir(parents=True)
        for name in ["extract.actionsdata", "version.json"]:
            shutil.copyfile(fixtures / name, metadata / name)
        (root / "Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": "dev.sweetpad.LocaleProbe",
            "CFBundleDisplayName": "LocaleProbe", "CFBundleDevelopmentRegion": "en"}))
        for locale in ["en", "fr", "de", "pl", "ja"]:
            folder = root / (locale + ".lproj")
            folder.mkdir()
            (folder / "InfoPlist.strings").write_bytes(plistlib.dumps({"CFBundleDisplayName": "Sonde Locale" if locale == "fr" else "LocaleProbe"}))
        (root / "fr.lproj/AppShortcuts.strings").write_bytes(plistlib.dumps({"Greet in ${applicationName}": "Saluer dans ${applicationName}"}))
    wrapper = args.out / "appintentsnltrainingprocessor"
    wrapper.symlink_to("/home/hyzyla_home/sweetpad-target/debug/sweetpad")
    records = {}
    validator = args.out / "appshortcutstringsprocessor"
    validator.symlink_to(wrapper.readlink())
    controls = []
    for name, key, text, extra in [
        ("valid", "Greet in ${applicationName}", "Saluer dans ${applicationName}", []),
        ("missing-app", "Greet in ${applicationName}", "Saluer", []),
        ("unknown-parameter", "Greet in ${applicationName}", "Saluer ${name} ${applicationName}", []),
        ("unused", "Other ${applicationName}", "Autre ${applicationName}", []),
        ("empty", "Greet in ${applicationName}", "", []),
        ("app-name-override", "Greet in ${applicationName}", "Saluer", ["--app-name-override"]),
        ("disabled", "Greet in ${applicationName}", "Saluer", ["--disable"]),
        ("string-table", "Greet in ${applicationName}", "Saluer dans ${applicationName}", []),
    ]:
        folder = args.out / "validation" / name
        folder.mkdir(parents=True)
        source = folder / ("AppShortcuts.strings" if name == "string-table" else "AppShortcuts.xcstrings")
        if name == "string-table":
            source.write_text(json.dumps(key) + " = " + json.dumps(text) + ";\n")
        else:
            source.write_text(json.dumps({"sourceLanguage": "en", "strings": {key: {"localizations": {
                "fr": {"stringUnit": {"state": "translated", "value": text}}}}}, "version": "1.0"}))
        common = ["--source-file", str(source), "--input-data-path", str(roots["native"] / "Metadata.appintents"),
                  "--platform-family", "iOS", "--deployment-target", "18.0", *extra]
        results = {}
        for host in ["native", "linux"]:
            command = ([str(args.xcode / "Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/appshortcutstringsprocessor")]
                       if host == "native" else ["orb", "-m", args.vm, str(validator)])
            result = subprocess.run([*command, *common], capture_output=True, text=True, timeout=90)
            results[host] = {"exitCode": result.returncode, "diagnostics": json.loads(result.stdout or "[]")}
        controls.append({"case": name, **results, "passed": results["native"] == results["linux"]})
    records["catalog-validation"] = {"cases": controls, "passed": all(r["passed"] for r in controls)}
    for phase in ["baseline", "no-op", "translation-and-name", "remove-shortcuts"]:
        before = digest(roots["linux"])
        for host, root in roots.items():
            if phase == "translation-and-name":
                (root / "fr.lproj/AppShortcuts.strings").write_bytes(plistlib.dumps({"Greet in ${applicationName}": "Dire bonjour via ${applicationName}"}))
                (root / "fr.lproj/InfoPlist.strings").write_bytes(plistlib.dumps({"CFBundleDisplayName": "Autre Sonde"}))
            elif phase == "remove-shortcuts":
                path = root / "Metadata.appintents/extract.actionsdata"
                value = json.loads(path.read_text())
                value["autoShortcuts"] = []
                path.write_text(json.dumps(value))
            command = ["--product-path", str(root), "--infoplist-path", str(root / "Info.plist"),
                       "--bundle-id", "dev.sweetpad.LocaleProbe", "--extracted-metadata-path", str(root / "Metadata.appintents"),
                       "--temp-dir-path", str(root.parent / "ssu"), "--archive-ssu-assets"]
            if host == "native":
                command = [str(args.xcode / "Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/appintentsnltrainingprocessor"), *command]
            else:
                command = ["orb", "-m", args.vm, str(wrapper), *command]
            result = subprocess.run(command, capture_output=True, text=True, timeout=120)
            (args.out / f"{phase}-{host}.stdout").write_text(result.stdout)
            (args.out / f"{phase}-{host}.stderr").write_text(result.stderr)
            result.check_returncode()
        models = {}
        stale = {}
        for host, root in roots.items():
            models[host] = {str(p.parent.relative_to(root)): read(p, allow_stale=host == "native")[0] for p in root.rglob("nlu.lzfse")}
            stale[host] = len(list(root.rglob("*.version"))) - len(models[host])
        native_retained = None
        if phase == "remove-shortcuts":
            # The native command retains its previous models after all shortcuts
            # disappear. Keep that evidence and compare with a fresh control too.
            native_retained = models["native"]
            clean = args.out / "native-clean/LocaleProbe.app"
            shutil.copytree(roots["native"], clean, ignore=shutil.ignore_patterns("nlu.appintents", "nlu", "root.ssu.yaml"))
            command = [str(args.xcode / "Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/appintentsnltrainingprocessor"),
                       "--product-path", str(clean), "--infoplist-path", str(clean / "Info.plist"),
                       "--bundle-id", "dev.sweetpad.LocaleProbe", "--extracted-metadata-path", str(clean / "Metadata.appintents"),
                       "--temp-dir-path", str(clean.parent / "ssu"), "--archive-ssu-assets"]
            subprocess.run(command, check=True, capture_output=True, text=True, timeout=120)
            models["native"] = {str(p.parent.relative_to(clean)): read(p)[0] for p in clean.rglob("nlu.lzfse")}
        record = {"native": models["native"], "linux": models["linux"],
                  "modelsMatch": models["native"] == models["linux"],
                  "declaredOutputExists": (roots["linux"].parent / "ssu/root.ssu.yaml").is_file()}
        record["staleVersionMarkers"] = stale
        if native_retained is not None:
            record["nativeIncrementalModelsRetained"] = len(native_retained)
            record["comparedWithFreshNativeControl"] = True
        if phase == "no-op":
            record["modelHashesUnchanged"] = before == digest(roots["linux"])
        if phase == "remove-shortcuts":
            record["staleModelsRemoved"] = not models["linux"]
        record["passed"] = record["modelsMatch"] and record["declaredOutputExists"] and record.get("modelHashesUnchanged", True) and record.get("staleModelsRemoved", True)
        records[phase] = record
        (args.out / "summary.json").write_text(json.dumps(records, indent=2, ensure_ascii=False) + "\n")
        print(phase, "pass" if record["passed"] else "FAIL", flush=True)
    if not all(r["passed"] for r in records.values()):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
