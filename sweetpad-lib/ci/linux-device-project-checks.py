#!/usr/bin/env python3
"""Install and check an existing Linux-built app on a physical iPhone.

No compilation, re-signing, private-key access, or account changes occur here.
Records Apple signature verification, two launches, process checks, hashes,
and optional screenshots. Online login and service behavior require other tests.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import time
import re

CI = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("checks", CI / "linux-project-checks.py")
checks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checks)


def invoke(command, stem, timeout=180):
    result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    stem.with_suffix(".stdout").write_text(result.stdout)
    stem.with_suffix(".stderr").write_text(result.stderr)
    payload = None
    for stream in [result.stdout, result.stderr]:
        try:
            payload = json.loads(stream)
            break
        except ValueError:
            pass
    stem.with_suffix(".json").write_text(json.dumps({"command": command, "exitCode": result.returncode, "result": payload}, indent=2) + "\n")
    result.check_returncode()
    return payload


def hashes(app):
    files = set(app.rglob("*.dylib"))
    for info in app.rglob("Info.plist"):
        executable = checks.plist(info).get("CFBundleExecutable")
        if executable and (info.parent / executable).is_file():
            files.add(info.parent / executable)
    return {str(p.relative_to(app)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(files)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--build-result", type=Path, required=True)
    parser.add_argument("--only", required=True)
    parser.add_argument("--device", required=True)
    parser.add_argument("--vm", default="sweetpad-linux")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--screenshots", action="store_true")
    parser.add_argument("--execution-audit", type=Path, help="Require a successful Linux executable-provenance audit")
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    project = next(p for p in json.loads(args.plan.read_text())["projects"] if p["slug"] == args.only)
    result = json.loads(args.build_result.read_text())
    if not result.get("ok") or not result.get("data", {}).get("built"):
        raise ValueError("a successful signed Linux build is required")
    if args.execution_audit:
        audit = json.loads(args.execution_audit.read_text())
        if not audit.get("passed") or audit.get("violations"):
            raise ValueError("the supplied Linux execution audit did not pass")
    app = checks.copy_product(Path(result["data"]["productPath"]), args.out / "product", args.vm)
    info = checks.plist(app / "Info.plist")
    bundle = info["CFBundleIdentifier"]
    assert bundle == project["deviceBundleIdentifier"]
    assert (app / "embedded.mobileprovision").is_file()
    if project.get("omittedExtensions") and list(app.rglob("*.appex")):
        raise ValueError("limited startup products must not contain stale extensions")
    before = hashes(app)
    invoke(["/usr/bin/codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app)], args.out / "signature")
    installation = invoke(["xcrun", "devicectl", "device", "install", "app", "--device", args.device,
                           "--timeout", "120", "--json-output", "-", str(app)], args.out / "install")
    installed = next(p for p in installation["result"]["installedApplications"] if p["bundleID"] == bundle)
    executable = installed["installationURL"].rstrip("/") + "/" + info["CFBundleExecutable"]
    launches = []
    previous_phase = None
    for number in [1, 2]:
        invoke(["xcrun", "devicectl", "device", "process", "launch", "--device", args.device,
                "--terminate-existing", "--timeout", "60", "--json-output", "-", bundle], args.out / f"launch-{number}")
        time.sleep(5)
        processes = invoke(["xcrun", "devicectl", "device", "info", "processes", "--device", args.device,
                            "--timeout", "30", "--json-output", "-"], args.out / f"processes-{number}")
        running = [p["processIdentifier"] for p in processes["result"]["runningProcesses"] if p.get("executable") == executable]
        launches.append({"launch": number, "runningProcessIdentifiers": running})
        if not running:
            raise RuntimeError(f"{bundle} exited after launch {number}")
        if args.only == "react-native-mmkv":
            report_file = args.out / f"mmkv-store-{number}.bin"
            invoke(["xcrun", "devicectl", "device", "copy", "from", "--device", args.device,
                    "--domain-type", "appDataContainer", "--domain-identifier", bundle,
                    "--source", "Documents/mmkv/dev.sweetpad.parity", "--destination", str(report_file),
                    "--timeout", "30", "--json-output", "-"], args.out / f"mmkv-copy-{number}")
            matches = re.findall(rb"SWEETPAD_REPORT_BEGIN(.*?)SWEETPAD_REPORT_END", report_file.read_bytes())
            if not matches:
                raise RuntimeError("MMKV did not persist a device probe report")
            reports = [json.loads(m) for m in matches]
            probe = max(reports, key=lambda r: r["phase"])
            if probe["error"] or len(probe["checks"]) != 7 or not all(probe["checks"].values()):
                raise RuntimeError(f"MMKV checks failed: {probe}")
            if previous_phase is not None and probe["phase"] != previous_phase + 1:
                raise RuntimeError(f"MMKV launch counter did not persist: {probe}")
            previous_phase = probe["phase"]
            launches[-1]["storageProbe"] = probe
        if args.screenshots:
            invoke(["uv", "tool", "run", "--from", "pymobiledevice3==11.50.0", "pymobiledevice3", "developer", "dvt", "screenshot",
                    "--udid", args.device, "--userspace", str(args.out / f"launch-{number}.png")],
                   args.out / f"screenshot-{number}", timeout=120)
    unchanged = hashes(app) == before
    report = {"project": args.only, "bundleIdentifier": bundle, "passed": unchanged,
              "scope": project.get("deviceValidationScope", "Two startup checks; no online login or backend service tests."),
              "configuration": project["configuration"], "launches": launches,
              "executionAudit": str(args.execution_audit) if args.execution_audit else None,
              "executablesUnchanged": unchanged, "executableHashes": before,
              "screenshots": args.screenshots, "visualReview": "pending" if args.screenshots else "not captured"}
    (args.out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    if not unchanged:
        raise RuntimeError("exported executable hashes changed during device checks")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
