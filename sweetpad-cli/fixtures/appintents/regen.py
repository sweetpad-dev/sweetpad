#!/usr/bin/env python3
"""Capture registration, localization, and phrase models with the user's Xcode.
Requires xcodegen and a built target/debug/sweetpad on macOS.

    DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer python3 -I sweetpad-cli/fixtures/appintents/regen.py

The captures normalize source paths and JSON formatting. Rust tests also
normalize the generator identity, resolver sets, and metadata key/value order.
The SSU test compares phrases because native training adds a timestamp and
reorders phrases.
"""

import ctypes
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
PROBES = {
    # Swift source name, module name and checked-in registration.
    "DataApp": ("DataApp", "extract.actionsdata"),
    "Types": ("MetadataProbe", "Types.actionsdata"),
    "Files": ("FileProbe", "Files.actionsdata"),
    "InputOptionsProbe": ("InputOptionsProbe", "InputOptionsProbe.actions.json"),
    "OutputProbe": ("OutputProbe", "OutputProbe.actions.json"),
    "ResultProtocols": ("ResultProtocols", "ResultProtocols.actionsdata"),
    "Unsupported": ("Unsupported", None),
}

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
    parser.add_argument("--only", choices=PROBES)
    parser.add_argument("--export-registration", type=Path,
                        help="export fresh complete registrations and compiler inputs for Linux replay, without updating fixtures")
    args = parser.parse_args()
    developer = Path(os.environ.get("DEVELOPER_DIR") or capture("xcode-select", "-p"))
    sweetpad = HERE.parents[2] / "target/debug/sweetpad"
    if args.export_registration:
        args.export_registration.mkdir(parents=True, exist_ok=True)
        save_json(args.export_registration / "sdk.json", {
            "version": capture("xcrun", "--sdk", "iphoneos", "--show-sdk-version"),
            "xcode": capture("xcodebuild", "-version"),
        })
    outputs = {}
    with tempfile.TemporaryDirectory(prefix="sweetpad-appintents-oracle-") as tmp:
        work = Path(tmp)
        (work / "Host.swift").write_text('import SwiftUI\n@main struct ProbeApp: App { var body: some Scene { WindowGroup { Text("Metadata") } } }\n')
        spec = {"name": "AppIntentsOracle", "options": {"deploymentTarget": {"iOS": "18.0"}}, "targets": {}}
        for source, (module, _) in PROBES.items():
            spec["targets"][module] = {"type": "application", "platform": "iOS",
                "sources": [{"path": str(HERE / f"{source}.swift")}, {"path": "Host.swift"}],
                "settings": {"base": {"GENERATE_INFOPLIST_FILE": "YES", "SWIFT_VERSION": "6.0", "SWIFT_EMIT_LOC_STRINGS": "YES",
                    "PRODUCT_BUNDLE_IDENTIFIER": f"dev.sweetpad.{module}"}}}
        save_json(work / "project.json", spec)
        subprocess.run(["xcodegen", "generate", "--spec", str(work / "project.json"), "--project", str(work)], check=True)
        derived = work / "DerivedData"
        for source, (module, expected) in PROBES.items():
            if args.only and source != args.only:
                continue
            command = [str(sweetpad), "build", "--developer-dir", str(developer),
                "--project", str(work / "AppIntentsOracle.xcodeproj"), "--scheme", module,
                "--destination", "generic/platform=iOS Simulator", "--json", "--non-interactive",
                "--", "-derivedDataPath", str(derived), "ARCHS=arm64", "ONLY_ACTIVE_ARCH=YES", "CODE_SIGNING_ALLOWED=NO"]
            result = subprocess.run(command, capture_output=True, text=True)
            if result.returncode:
                raise RuntimeError(result.stdout + result.stderr)
            report = json.loads(result.stdout)
            if not report.get("data", {}).get("built"):
                raise RuntimeError(result.stdout + result.stderr)
            objects = derived / f"Build/Intermediates.noindex/AppIntentsOracle.build/Debug-iphonesimulator/{module}.build"
            constants = objects / f"Objects-normal/arm64/{source}.swiftconstvalues"
            metadata = Path(report["data"]["productPath"]) / "Metadata.appintents"
            if args.export_registration:
                dest = args.export_registration / source
                dest.mkdir(parents=True, exist_ok=True)
                for path in sorted(constants.parent.glob("*.swiftconstvalues")):
                    values = json.loads(path.read_text())
                    normalize_paths(values)
                    save_json(dest / path.name, values)
                for name in ("extract.actionsdata", "version.json"):
                    save_json(dest / name, json.loads((metadata / name).read_text()))
                continue
            values = json.loads(constants.read_text())
            normalize_paths(values)
            outputs[f"{source}.swiftconstvalues"] = json.dumps(values, indent=2, sort_keys=True).encode() + b"\n"
            if expected is None:
                continue
            if expected.endswith(".actions.json"):
                # The probe's actions only: the metadata under test.
                actions = json.loads((metadata / "extract.actionsdata").read_text())["actions"]
                outputs[expected] = json.dumps(actions, indent=2, sort_keys=True).encode() + b"\n"
            elif source == "ResultProtocols":
                registration = json.loads((metadata / "extract.actionsdata").read_text())
                outputs[expected] = json.dumps(registration, indent=2, sort_keys=True).encode() + b"\n"
            else:
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
