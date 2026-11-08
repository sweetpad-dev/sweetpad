#!/usr/bin/env python3
"""Capture registration, localization, and phrase models with the user's Xcode.
Requires xcodegen and a built target/debug/sweetpad on macOS.

    DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer python3 -I sweetpad-cli/fixtures/appintents/regen.py

Only source paths and JSON formatting are normalized. Rust tests normalize the
generator and the order of the native resolver sets. The SSU test compares its
semantic contents because native training adds a timestamp and reorders phrases.
"""

import ctypes
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent

def capture(*args):
    return subprocess.check_output(args, text=True).strip()


def save_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def normalize_paths(value):
    if isinstance(value, dict):
        for key, member in value.items():
            if key in ("file", "source") and isinstance(member, str):
                value[key] = Path(member).name
            else:
                normalize_paths(member)
    elif isinstance(value, list):
        for member in value:
            normalize_paths(member)


def decompress(path):
    data = path.read_bytes()
    library = ctypes.CDLL("/usr/lib/libcompression.dylib")
    decode = library.compression_decode_buffer
    decode.restype = ctypes.c_size_t
    decode.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t,
                       ctypes.c_void_p, ctypes.c_int]
    output = ctypes.create_string_buffer(8 * 1024 * 1024)
    count = decode(output, len(output), data, len(data), None, 0x801)  # COMPRESSION_LZFSE
    if not count:
        raise RuntimeError("libcompression could not decode the native phrase model")
    return output.raw[:count]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", choices=["DataApp", "Types", "Files"])
    selected = parser.parse_args().only
    developer = Path(os.environ.get("DEVELOPER_DIR") or capture("xcode-select", "-p"))
    sweetpad = HERE.parents[2] / "target/debug/sweetpad"
    outputs = {}
    with tempfile.TemporaryDirectory(prefix="sweetpad-appintents-oracle-") as tmp:
        work = Path(tmp)
        (work / "Host.swift").write_text('import SwiftUI\n@main struct ProbeApp: App { var body: some Scene { WindowGroup { Text("Metadata") } } }\n')
        spec = {"name": "AppIntentsOracle", "options": {"deploymentTarget": {"iOS": "18.0"}}, "targets": {}}
        for source, module in [("DataApp", "DataApp"), ("Types", "MetadataProbe"), ("Files", "FileProbe")]:
            spec["targets"][module] = {"type": "application", "platform": "iOS",
                "sources": [{"path": str(HERE / f"{source}.swift")}, {"path": "Host.swift"}],
                "settings": {"base": {"GENERATE_INFOPLIST_FILE": "YES", "SWIFT_VERSION": "6.0", "SWIFT_EMIT_LOC_STRINGS": "YES",
                    "PRODUCT_BUNDLE_IDENTIFIER": f"dev.sweetpad.{module}"}}}
        save_json(work / "project.json", spec)
        subprocess.run(["xcodegen", "generate", "--spec", str(work / "project.json"), "--project", str(work)], check=True)
        derived = work / "DerivedData"
        for source, module, expected in [("DataApp", "DataApp", "extract.actionsdata"),
                                         ("Types", "MetadataProbe", "Types.actionsdata"),
                                         ("Files", "FileProbe", "Files.actionsdata")]:
            if selected and source != selected:
                continue
            command = [str(sweetpad), "build", "--developer-dir", str(developer),
                "--project", str(work / "AppIntentsOracle.xcodeproj"), "--scheme", module,
                "--destination", "generic/platform=iOS Simulator", "--json", "--non-interactive",
                "--", "-derivedDataPath", str(derived), "ARCHS=arm64", "ONLY_ACTIVE_ARCH=YES", "CODE_SIGNING_ALLOWED=NO"]
            result = subprocess.run(command, capture_output=True, text=True, check=True)
            report = json.loads(result.stdout)
            if not report.get("data", {}).get("built"):
                raise RuntimeError(result.stdout + result.stderr)
            objects = derived / f"Build/Intermediates.noindex/AppIntentsOracle.build/Debug-iphonesimulator/{module}.build"
            constants = objects / f"Objects-normal/arm64/{source}.swiftconstvalues"
            metadata = Path(report["data"]["productPath"]) / "Metadata.appintents"
            values = json.loads(constants.read_text())
            normalize_paths(values)
            outputs[f"{source}.swiftconstvalues"] = json.dumps(values, indent=2, sort_keys=True).encode() + b"\n"
            outputs[expected] = (metadata / "extract.actionsdata").read_bytes()
            if source == "DataApp":
                strings = json.loads((objects / "Objects-normal/arm64/ExtractedAppShortcutsMetadata.stringsdata").read_text())
                normalize_paths(strings)
                outputs["Shortcuts.stringsdata"] = json.dumps(strings, indent=2, sort_keys=True).encode() + b"\n"
                outputs["DataApp.nlu"] = decompress(metadata / "nlu/nlu.lzfse")
                outputs["root.ssu.yaml"] = (metadata / "root.ssu.yaml").read_bytes()
                outputs["version.json"] = (metadata / "version.json").read_bytes()
    for name, data in outputs.items():
        (HERE / name).write_bytes(data)


if __name__ == "__main__":
    main()
