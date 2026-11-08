#!/usr/bin/env python3
"""Run unchanged AccountTests products in app hosts with an isolated keychain.

Build AccountTests on both hosts first. Supply the native Products directory,
the VM Products directory, and a completed linux-parity.py baseline. This uses
app-group entitlements supported by ad-hoc simulator signing. It does not need
a developer identity or a physical device.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys

CI = Path(__file__).resolve().parent
sys.path.insert(0, str(CI))
import prebuilt_tests
import simulator_signing
spec = importlib.util.spec_from_file_location("checks", CI / "linux-project-checks.py")
checks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checks)


def prepare(products, host, output, developer):
    configuration = "Debug-iphonesimulator"
    source = products / configuration / "AccountTests.xctest"
    executable = source / "AccountTests"
    before = hashlib.sha256(executable.read_bytes()).hexdigest()
    root = output / "Products"
    app = root / configuration / host.name
    app.parent.mkdir(parents=True, exist_ok=True)
    if app.exists():
        shutil.rmtree(app)
    shutil.copytree(host, app, symlinks=True)
    shutil.rmtree(app / "PlugIns", ignore_errors=True)
    (app / "PlugIns").mkdir()
    shutil.copytree(source, app / "PlugIns/AccountTests.xctest", symlinks=True)
    for resource in (products / configuration).glob("*.bundle"):
        shutil.copytree(resource, app / resource.name, symlinks=True)
    frameworks = app / "Frameworks"
    frameworks.mkdir(exist_ok=True)
    for framework in (products / configuration / "PackageFrameworks").glob("*.framework"):
        shutil.copytree(framework, frameworks / framework.name, dirs_exist_ok=True, symlinks=True)
    shutil.copy2(developer / "Platforms/iPhoneSimulator.platform/Developer/usr/lib/libXCTestBundleInject.dylib", frameworks)
    simulator_signing.sign(app, {"com.apple.security.application-groups": ["group.dev.sweetpad.AccountTests"]})
    manifest = prebuilt_tests.write(root, [app.relative_to(root) / "PlugIns/AccountTests.xctest"], "AccountTests")
    copied = app / "PlugIns/AccountTests.xctest/AccountTests"
    return manifest, executable, copied, before


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-products", type=Path, required=True)
    parser.add_argument("--linux-products", type=Path, required=True)
    parser.add_argument("--parity-out", type=Path, required=True)
    parser.add_argument("--simulator", required=True)
    parser.add_argument("--vm", default="sweetpad-linux")
    parser.add_argument("--out", type=Path, default=CI.parent / "corpus/_linux-followups/validation/keychain-tests")
    parser.add_argument("--developer-dir", type=Path, default=Path("/Applications/Xcode.app/Contents/Developer"))
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    linux = checks.copy_product(args.linux_products, args.out / "linux-source", args.vm)
    hosts = {"xcode": args.parity_out / "native-dd/Build/Products/Debug-iphonesimulator/ParityProbe.app",
             "linux": args.parity_out / "baseline/linux-export/Products/Debug-iphonesimulator/ParityProbe.app"}
    records = {}
    for host, source in [("xcode", args.native_products), ("linux", linux)]:
        output = args.out / host
        manifest, original, copied, original_hash = prepare(source, hosts[host], output, args.developer_dir)
        signed_hash = hashlib.sha256(copied.read_bytes()).hexdigest()
        command = [str(CI.parents[1] / "target/debug/sweetpad"), "test", "run", "--project",
                   str(CI.parent / "corpus/netnewswire/NetNewsWire.xcodeproj"), "--scheme", "AccountTests",
                   "--xctestrun", str(manifest), "--on", args.simulator,
                   "--result-bundle", str(output / "tests.xcresult"), "--json", "--non-interactive"]
        result = subprocess.run(command, capture_output=True, text=True, timeout=1200)
        (output / "stdout.json").write_text(result.stdout)
        (output / "stderr.json").write_text(result.stderr)
        payload = json.loads(result.stdout or result.stderr)
        records[host] = {"command": command, "exitCode": result.returncode, "result": payload,
                         "sourceExecutableUnchanged": original_hash == hashlib.sha256(original.read_bytes()).hexdigest(),
                         "signedExecutableUnchangedDuringTests": signed_hash == hashlib.sha256(copied.read_bytes()).hexdigest()}
        (args.out / "summary.json").write_text(json.dumps(records, indent=2) + "\n")
        print(host, payload.get("data", {}).get("passedTests"), "passed", flush=True)
    if not all(r["result"].get("data", {}).get("passed") and r["sourceExecutableUnchanged"]
               and r["signedExecutableUnchangedDuringTests"] for r in records.values()):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
