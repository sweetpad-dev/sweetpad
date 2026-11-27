#!/usr/bin/env python3
"""Replay fresh Xcode probe registrations and verify the SDK checklist's diagnostics."""

import argparse
from copy import deepcopy
import json
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parents[2] / "sweetpad-lib/ci"))
from bundle_comparison import differences, normalize_registration
from regen import PROBES
from sdk_coverage import BASELINE, checklist


def read(path):
    return json.loads(path.read_text())


def equal(label, expected, actual):
    delta = differences(expected, actual)
    if delta:
        raise ValueError(f"{label}: {len(delta)} differences\n" + json.dumps(delta[:12], indent=2))


def extract(binary, work, constants, module, version, expected_error=None):
    work.mkdir()
    listing = work / "inputs"
    listing.write_text("".join(str(path.resolve()) + "\n" for path in constants))
    result = subprocess.run(["appintentsmetadataprocessor", "--compile-time-extraction", "--module-name", module,
                             "--bundle-identifier", "dev.sweetpad." + module, "--xcode-version", version,
                             "--output", str(work), "--swift-const-vals-list", str(listing)],
                            executable=binary, capture_output=True, text=True)
    metadata = work / "Metadata.appintents"
    if expected_error is None:
        if result.returncode:
            raise ValueError(f"{module}: {result.stderr}")
    elif result.returncode != 1 or expected_error not in result.stderr or metadata.exists():
        raise ValueError(f"{work.name}: expected '{expected_error}' and no registration, "
                         f"got exit {result.returncode}: {result.stderr}")
    return metadata


def compare_capture(capture, work, binary, module):
    version = read(capture / "version.json")
    native = read(capture / "extract.actionsdata")
    output = extract(binary, work, sorted(capture.glob("*.swiftconstvalues")), module, version["toolsVersion"])
    actual = read(output / "extract.actionsdata")
    if native["generator"]["name"] != "xcode-tools" or actual["generator"]["name"] != "sweetpad":
        raise ValueError(f"{module}: unexpected registration generators")
    equal(module + "/extract.actionsdata", normalize_registration(native), normalize_registration(actual))
    equal(module + "/version.json", version, read(output / "version.json"))
    return len(native["actions"])


def verify_no_effect(registration):
    actions = registration["actions"]
    # Only type identity differs between these two native Swift declarations.
    identity = {"identifier", "fullyQualifiedTypeName", "mangledTypeName", "mangledTypeNameV2"}
    pair = [{k: v for k, v in actions[name].items() if k not in identity}
            for name in ("ConfirmProbe", "PromptProbe")]
    equal("native requestValueDialog control", *map(normalize_registration, pair))


def verify_diagnostics(captures, work, binary):
    case_errors = {
        "WhenProbe": "unsupported parameterSummary 'AppIntents.ParameterSummaryWhenCondition",
        "SwitchProbe": "unsupported parameterSummary 'AppIntents.ParameterSummarySwitchCondition",
        "TableProbe": "unsupported parameterSummary argument 'table'",
        "RangeProbe": "unsupported App Intents parameter option 'inclusiveRange'",
        "OptionsProviderProbe": "unsupported App Intents parameter option 'optionsProvider'",
    }
    native = captures / "Unsupported"
    version = read(native / "version.json")["toolsVersion"]
    types = read(native / "Unsupported.swiftconstvalues")
    equal("native diagnostic probes", sorted(case_errors), sorted(read(native / "extract.actionsdata")["actions"]))
    # A failing declaration stops the processor. Isolate them so a prior error cannot mask the next one.
    def run_case(tag, declarations, error):
        path = work / (tag + ".swiftconstvalues")
        path.write_text(json.dumps(declarations))
        extract(binary, work / tag, [path], "Unsupported", version, error)

    for name, message in case_errors.items():
        intent = next(t for t in types if t["typeName"] == "Unsupported." + name)
        run_case(name, [intent], message)

    # Exercise the argument-label guard separately from Swift overload resolution.
    # Each row classified Diagnostic must still reject that exact option.
    options = [k.removeprefix("parameter.") for k, status in checklist().items()
               if k.startswith("parameter.") and status == "Diagnostic"]
    base = next(t for t in read(captures / "ResultProtocols/ResultProtocols.swiftconstvalues")
                if t["typeName"].endswith(".ConfirmProbe"))
    for label in options:
        intent = deepcopy(base)
        param = next(p for p in intent["properties"] if p["label"] == "text")
        param["propertyWrappers"][0]["arguments"].append({"label": label, "valueKind": "RawLiteral", "value": "1"})
        run_case(label, [intent], f"unsupported App Intents parameter option '{label}'")
    return len(case_errors), len(options)


def verify(captures, binary):
    sdk = read(captures / "sdk.json")["version"]
    compare_baseline = tuple(map(int, sdk.split("."))) >= tuple(map(int, BASELINE.split(".")))
    with tempfile.TemporaryDirectory(prefix="sweetpad-intents-replay-") as tmp:
        work = Path(tmp)
        total = 0
        for source, (module, expected) in PROBES.items():
            if expected is None:
                continue
            if compare_baseline:
                native = read(captures / source / "extract.actionsdata")
                baseline = read(HERE / expected)
                if expected.endswith(".actions.json"):
                    native, baseline = {"actions": native["actions"]}, {"actions": baseline}
                equal(source + "/Xcode baseline", normalize_registration(baseline), normalize_registration(native))
            count = compare_capture(captures / source, work / source, binary, module)
            total += count
            print(f"{source}: {count} actions; complete registration and version match")
        verify_no_effect(read(captures / "ResultProtocols/extract.actionsdata"))
        native, labels = verify_diagnostics(captures, work, binary)
        print(f"{total} actions matched; {native} native diagnostic probes and {labels} option diagnostics passed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("captures", type=Path)
    parser.add_argument("--sweetpad", type=Path, default=HERE.parents[2] / "target/debug/sweetpad")
    args = parser.parse_args()
    verify(args.captures, args.sweetpad.resolve())
