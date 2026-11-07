#!/usr/bin/env python3
"""Verify and execute a signed Linux-built parity fixture on a connected iPhone.

Run on the Mac after a Linux `sweetpad test build` for generic/platform=iOS.
The signing identity and profile must already be configured for that build.
This script needs no private key, password, or login-keychain changes.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

CI = Path(__file__).resolve().parent
REPO = CI.parent.parent
sys.path.insert(0, str(CI))
import prebuilt_tests

spec = importlib.util.spec_from_file_location("checks", CI / "linux-project-checks.py")
checks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checks)


def invoke(command, stem, timeout=120):
    result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    stem.with_suffix(".stdout").write_text(result.stdout)
    stem.with_suffix(".stderr").write_text(result.stderr)
    record = {"command": list(map(str, command)), "exitCode": result.returncode}
    for stream in [result.stdout, result.stderr]:
        try:
            record["result"] = json.loads(stream)
            break
        except ValueError:
            pass
    stem.with_suffix(".json").write_text(json.dumps(record, indent=2) + "\n")
    result.check_returncode()
    return record


def executable_hashes(app):
    files = []
    for info in app.rglob("Info.plist"):
        data = checks.plist(info)
        executable = data.get("CFBundleExecutable")
        if executable and (info.parent / executable).is_file():
            files.append(info.parent / executable)
    files.extend(app.rglob("*.dylib"))
    return {str(p.relative_to(app)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(set(files))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vm", default="sweetpad-linux")
    parser.add_argument("--device", required=True, help="Connected iPhone UDID")
    parser.add_argument("--linux-products", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True, help="Fresh evidence directory")
    parser.add_argument("--project", type=Path,
                        default=CI / "fixture-linux-parity/ParityProbe.xcodeproj")
    parser.add_argument("--sweetpad", type=Path, default=REPO / "target/debug/sweetpad")
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    products = args.out / "linux-dd/Build/Products"
    # The host contains its test bundle and runtime frameworks. Module caches
    # and the package's unembedded build products are not device inputs.
    app = checks.copy_product(args.linux_products / "Debug-iphoneos/ParityProbe.app",
                              products / "Debug-iphoneos", args.vm)
    if not (app / "embedded.mobileprovision").is_file():
        raise ValueError("device validation requires a provisioned, signed app")
    before = executable_hashes(app)
    if "ParityProbe" not in before or not any(p.endswith("ParityProbeTests") for p in before):
        raise ValueError("the parity app and app-hosted test executable must both be present")
    invoke(["/usr/bin/codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app)],
           args.out / "apple-signature-verification")
    invoke(["xcrun", "devicectl", "device", "install", "app", "--device", args.device,
            "--timeout", "120", "--json-output", "-", str(app)], args.out / "install")
    common = [str(args.sweetpad), "--developer-dir", "/Applications/Xcode.app/Contents/Developer"]
    selection = ["--project", str(args.project), "--scheme", "ParityProbe", "--on", args.device,
                 "--json", "--non-interactive"]
    manifest = prebuilt_tests.write(products,
        [Path("Debug-iphoneos/ParityProbe.app/PlugIns/ParityProbeTests.xctest")],
        "ParityProbe", {"EXPECTED_ORIGIN": "linux"}, name="ParityProbe-device.xctestrun")
    tests = invoke([*common, "test", "run", *selection, "--xctestrun", str(manifest),
                    "--result-bundle", str(args.out / "device.xcresult")],
                   args.out / "tests", timeout=600)
    after = executable_hashes(app)
    payload = tests.get("result", {}).get("data", {})
    summary = {"device": args.device, "source": str(args.linux_products),
               "executableHashes": before, "executablesUnchanged": before == after,
               "tests": payload}
    (args.out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    if (not payload.get("passed") or payload.get("total") != 7
            or payload.get("passedTests") != 7 or payload.get("skippedTests") != 0
            or before != after):
        raise RuntimeError("expected seven passing Linux-origin tests with unchanged executables")
    invoke([*common, "app", "launch", *selection,
            "--derived-data-path", str(args.out / "linux-dd")], args.out / "launch")
    processes = invoke(["xcrun", "devicectl", "device", "info", "processes", "--device", args.device,
                        "--timeout", "30", "--json-output", "-"], args.out / "processes")
    running = processes.get("result", {}).get("result", {}).get("runningProcesses", [])
    summary["runningProcessIdentifiers"] = [p["processIdentifier"] for p in running
        if p.get("executable", "").endswith("/ParityProbe.app/ParityProbe")]
    summary["passed"] = bool(summary["runningProcessIdentifiers"])
    (args.out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    if not summary["passed"]:
        raise RuntimeError("the Linux-built app is not running after launch")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
