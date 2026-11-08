#!/usr/bin/env python3
"""Exercise Linux test products, incremental builds, and native bundle comparisons.

Runs on the Mac with an OrbStack Linux machine and a booted iOS simulator.
The working fixture is copied under --out; checked-in sources are never mutated.
"""
import argparse
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import time
import xml.etree.ElementTree as ET

CI = Path(__file__).resolve().parent
REPO = CI.parent.parent
spec = importlib.util.spec_from_file_location("checks", CI / "linux-project-checks.py")
checks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checks)
spec = importlib.util.spec_from_file_location("prebuilt_tests", CI / "prebuilt_tests.py")
prebuilt_tests = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prebuilt_tests)


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def invoke(command, stem, timeout=1200):
    started = time.monotonic()
    result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    stem.parent.mkdir(parents=True, exist_ok=True)
    stem.with_suffix(".stdout").write_text(result.stdout)
    stem.with_suffix(".stderr").write_text(result.stderr)
    payload = None
    for stream in [result.stdout, result.stderr]:
        try:
            payload = json.loads(stream)
            break
        except ValueError:
            pass
    record = {"command": command, "exitCode": result.returncode,
              "seconds": round(time.monotonic() - started, 2), "result": payload}
    save(stem.with_suffix(".json"), record)
    return record


def build(args, root, phase, host, clean=False):
    common = ["test", "build", "--project", str(root / "ParityProbe.xcodeproj"),
              "--scheme", "ParityProbe", "--configuration", "Debug",
              "--destination", "generic/platform=iOS Simulator", "--json", "--non-interactive"]
    flags = ["ARCHS=arm64", "ONLY_ACTIVE_ARCH=YES", "CODE_SIGNING_ALLOWED=NO", "ENABLE_DEBUG_DYLIB=NO"]
    if host == "xcode":
        command = [str(REPO / "target/debug/sweetpad"), *common,
                   "--", "-derivedDataPath", str(args.out / ("native-dd-clean" if clean else "native-dd")), *flags]
    else:
        script = '''export PATH="$HOME/bin:$HOME/swift/swift-6.4.2-RELEASE/usr/bin:$PATH"
exec "$HOME/sweetpad-target/debug/sweetpad" "$@" -- -derivedDataPath "$HOME/SWEETPAD_PARITY_DD" ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO ENABLE_DEBUG_DYLIB=NO 'SWIFT_ACTIVE_COMPILATION_CONDITIONS=$(inherited) LINUX_PRODUCT'
'''
        script = script.replace("SWEETPAD_PARITY_DD", args.linux_dd + ("-clean" if clean else ""))
        command = ["orb", "-m", args.vm, "bash", "-lc", script, "--", *common]
    result = invoke(command, args.out / phase / (host + "-build"))
    print(phase, host, "build", result["exitCode"], flush=True)
    return result


def products(args, phase, host, clean=False):
    if host == "xcode":
        return args.out / ("native-dd-clean" if clean else "native-dd") / "Build/Products"
    source = Path("/home/hyzyla_home") / (args.linux_dd + ("-clean" if clean else "")) / "Build/Products"
    return checks.copy_product(source, args.out / phase / "linux-export", args.vm)


def test(args, root, phase, host, product_root, expected):
    environment = dict(expected, EXPECTED_ORIGIN="linux" if host == "linux" else "xcode")
    if host == "linux":
        build = json.loads((args.out / phase / "linux-build.json").read_text())["result"]["data"]
        if not build.get("xctestrunPath"):
            raise RuntimeError(build.get("xctestrunNote", "Linux test build emitted no manifest"))
        manifest = product_root / Path(build["xctestrunPath"]).name
        generated = checks.plist(manifest)
        targets = generated["TestConfigurations"][0]["TestTargets"]
        assert len(targets) == 1 and targets[0]["EnvironmentVariables"] == environment
    else:
        manifest = prebuilt_tests.write(product_root,
            [Path("Debug-iphonesimulator/ParityProbe.app/PlugIns/ParityProbeTests.xctest")],
            "ParityProbe", environment, name="ParityProbe-parity.xctestrun")
    binaries = [p for p in product_root.rglob("*") if p.is_file() and p.name in ["ParityProbe", "ParityProbeTests"]]
    before = {str(p.relative_to(product_root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in binaries}
    command = [str(REPO / "target/debug/sweetpad"), "test", "run", "--project", str(root / "ParityProbe.xcodeproj"),
               "--scheme", "ParityProbe", "--xctestrun", str(manifest), "--on", args.simulator,
               "--result-bundle", str(args.out / phase / (host + ".xcresult")), "--json", "--non-interactive"]
    result = invoke(command, args.out / phase / (host + "-tests"), timeout=600)
    after = {str(p.relative_to(product_root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in binaries}
    result["executableHashes"] = before
    result["executablesUnchanged"] = before == after and bool(before)
    save(args.out / phase / (host + "-tests.json"), result)
    print(phase, host, "tests", result["exitCode"], flush=True)
    return result


def generate(root):
    subprocess.run(["xcodegen", "generate", "--spec", str(root / "project.yml"), "--project", str(root)], check=True)


def edit(root, relative, old, new):
    path = root / relative
    text = path.read_text()
    if old in text:
        path.write_text(text.replace(old, new))
    elif new not in text:
        raise ValueError(f"neither expected version found in {path}")


def run_phase(args, root, phase, expected, clean=False):
    record = {"builds": {}, "tests": {}, "expected": dict(expected)}
    if clean and (args.out / "native-dd-clean").exists():
        shutil.rmtree(args.out / "native-dd-clean")
    if clean:
        # test build has no --clean flag. Remove this fixture's generated
        # control products and intermediates before rebuilding them.
        subprocess.run(["orb", "-m", args.vm, "python3", "-c",
            "from pathlib import Path; import shutil,sys; p=Path.home()/sys.argv[1]/'Build'; "
            "assert not p.is_symlink(); shutil.rmtree(p, ignore_errors=True)", args.linux_dd + "-clean"], check=True)
    for host in ["xcode", "linux"]:
        if host == "linux":
            # Test expectations belong in the fixture's scheme. The CLI must
            # preserve them in its own manifest; never patch that manifest.
            path = root / "ParityProbe.xcodeproj/xcshareddata/xcschemes/ParityProbe.xcscheme"
            tree = ET.parse(path)
            launch = tree.getroot().find("LaunchAction")
            existing = launch.find("EnvironmentVariables")
            if existing is not None: launch.remove(existing)
            environment = ET.SubElement(launch, "EnvironmentVariables")
            for key, value in sorted(dict(expected, EXPECTED_ORIGIN="linux").items()):
                ET.SubElement(environment, "EnvironmentVariable", key=key, value=value, isEnabled="YES")
            tree.write(path, encoding="unicode", xml_declaration=True)
        record["builds"][host] = build(args, root, phase, host, clean)
    if all(r["exitCode"] == 0 for r in record["builds"].values()):
        roots = {host: products(args, phase, host, clean) for host in ["xcode", "linux"]}
        build_roots = {"xcode": args.out / ("native-dd-clean" if clean else "native-dd") / "Build/Products",
                       "linux": "/home/hyzyla_home/" + args.linux_dd + ("-clean" if clean else "") + "/Build/Products"}
        record["comparison"] = checks.compare(roots["xcode"] / "Debug-iphonesimulator/ParityProbe.app",
                                               roots["linux"] / "Debug-iphonesimulator/ParityProbe.app", build_roots)
        findings = record["comparison"]["findings"]
        if not clean and findings == [{"kind": "extraFile", "file": "Metadata.appintents/root.ssu.yaml"}]:
            # Xcode can remove its archived training source during package-only
            # metadata extraction without retraining. Keep the incremental
            # discrepancy visible, then require a fresh native control to prove
            # that the Linux archive and runtime models are correct.
            control_args = copy.copy(args)
            control_args.out = args.out / (phase + "-native-control")
            fresh_dir = control_args.out / "native-dd-clean"
            if fresh_dir.exists():
                shutil.rmtree(fresh_dir)
            control = build(control_args, root, "build", "xcode", clean=True)
            record["nativeFreshControlBuild"] = control
            if control["exitCode"] == 0:
                fresh_root = products(control_args, phase, "xcode", clean=True)
                fresh_app = fresh_root / "Debug-iphonesimulator/ParityProbe.app"
                record["nativeIncrementalVsFresh"] = checks.compare(
                    roots["xcode"] / "Debug-iphonesimulator/ParityProbe.app", fresh_app,
                    {"xcode": build_roots["xcode"], "linux": fresh_root})
                fresh_comparison = checks.compare(fresh_app,
                    roots["linux"] / "Debug-iphonesimulator/ParityProbe.app",
                    dict(build_roots, xcode=fresh_root))
                native_drift = record["nativeIncrementalVsFresh"]["findings"]
                if fresh_comparison["passed"] and native_drift == [{"kind": "extraFile", "file": "Metadata.appintents/root.ssu.yaml"}]:
                    record["nativeIncrementalComparison"] = record["comparison"]
                    record["comparison"] = fresh_comparison
                    record["usedFreshNativeControl"] = True
                    roots["xcode"] = fresh_root
        for host in ["xcode", "linux"]:
            record["tests"][host] = test(args, root, phase, host, roots[host], expected)
    record["passed"] = (bool(record["tests"]) and all(
        r["exitCode"] == 0 and r["executablesUnchanged"] and r["result"]["data"]["total"] > 0
        for r in record["tests"].values()) and record["comparison"]["passed"])
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=CI.parent / "corpus/_linux-followups/validation/parity")
    parser.add_argument("--vm", default="sweetpad-linux")
    parser.add_argument("--linux-dd", help="DerivedData directory name under the VM home (default: unique to --out)")
    parser.add_argument("--simulator", required=True)
    parser.add_argument("--baseline-only", action="store_true")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--resume", action="store_true", help="Continue completed phases recorded in summary.json")
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.linux_dd = args.linux_dd or "sweetpad-parity-" + hashlib.sha256(str(args.out).encode()).hexdigest()[:12]
    if not args.linux_dd or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-" for c in args.linux_dd):
        parser.error("--linux-dd must be a simple directory name")
    if (args.out / "summary.json").exists() and not (args.resume or args.prepare_only):
        parser.error("--out already has results; use --resume or choose a fresh output directory")
    root = args.out / "work/Parity Probe ü"
    if not root.exists():
        shutil.copytree(CI / "fixture-linux-parity", root)
    generate(root)
    if args.prepare_only:
        return
    expected = {"EXPECTED_GREETING": "source-v1|package-v1", "EXPECTED_RESOURCE": "resource-v1",
                "EXPECTED_PACKAGE_RESOURCE": "package-resource-v1", "EXPECT_OBSOLETE": "true"}
    summary_path = args.out / "summary.json"
    summary = json.loads(summary_path.read_text()) if args.resume and summary_path.exists() else {}
    phases = ["baseline", "no-op", "app-source", "package-source", "app-resource", "package-resource", "rename-resource", "delete-resource", "asset-color", "localization", "delete-asset", "delete-generated-output", "clean-control"]
    for phase in phases:
        previous = summary.get(phase)
        if previous and previous.get("passed", phase == "baseline"):
            expected = previous.get("expected", expected)
            if args.baseline_only:
                break
            continue
        if phase == "app-source":
            edit(root, "App/ParityProbe.swift", "source-v1", "source-v2")
            expected["EXPECTED_GREETING"] = "source-v2|package-v1"
        elif phase == "package-source":
            edit(root, "Packages/ValueKit/Sources/ValueKit/ValueKit.swift", "package-v1", "package-v2")
            expected["EXPECTED_GREETING"] = "source-v2|package-v2"
        elif phase == "app-resource":
            edit(root, "Resources/message.txt", "resource-v1", "resource-v2")
            expected["EXPECTED_RESOURCE"] = "resource-v2"
        elif phase == "package-resource":
            edit(root, "Packages/ValueKit/Sources/ValueKit/package.txt", "package-resource-v1", "package-resource-v2")
            expected["EXPECTED_PACKAGE_RESOURCE"] = "package-resource-v2"
        elif phase == "rename-resource":
            old = root / "Resources/message.txt"
            if old.exists():
                old.rename(root / "Resources/renamed.txt")
            edit(root, "project.yml", "PROBE_RESOURCE_NAME: message", "PROBE_RESOURCE_NAME: renamed")
            expected["ABSENT_RESOURCE"] = "message"
            generate(root)
        elif phase == "delete-resource":
            (root / "Resources/obsolete.txt").unlink(missing_ok=True)
            expected["EXPECT_OBSOLETE"] = "false"
            generate(root)
        elif phase == "asset-color":
            edit(root, "Resources/Assets.xcassets/Accent.colorset/Contents.json", '"0.250"', '"0.400"')
            expected["EXPECTED_RED"] = "0.4"
        elif phase == "localization":
            edit(root, "Resources/Localizable.xcstrings", "Bonjour", "Salut")
            expected["EXPECTED_FRENCH"] = "Salut"
        elif phase == "delete-asset":
            badge = root / "Resources/Assets.xcassets/Badge.imageset"
            if badge.exists():
                shutil.rmtree(badge)
            expected["EXPECT_BADGE"] = "false"
        elif phase == "delete-generated-output":
            (args.out / "native-dd/Build/Products/Debug-iphonesimulator/ParityProbe.app/Assets.car").unlink(missing_ok=True)
            command = ["orb", "-m", args.vm, "rm", "-f", "/home/hyzyla_home/" + args.linux_dd + "/Build/Products/Debug-iphonesimulator/ParityProbe.app/Assets.car"]
            subprocess.run(command, check=True)
        if phase not in summary or not summary[phase].get("passed", phase == "baseline"):
            summary[phase] = run_phase(args, root, phase, expected, clean=phase == "clean-control")
            if phase == "no-op" and summary[phase]["passed"]:
                unchanged = {host: summary[phase]["tests"][host]["executableHashes"] == summary["baseline"]["tests"][host]["executableHashes"] for host in ["xcode", "linux"]}
                summary[phase]["noOpExecutablesUnchanged"] = unchanged
                summary[phase]["passed"] = all(unchanged.values())
            if phase == "clean-control" and summary[phase]["passed"]:
                controls = {}
                for host in ["xcode", "linux"]:
                    incremental = products(args, "final-incremental", host)
                    clean = products(args, phase, host, True)
                    suffix = "Debug-iphonesimulator/ParityProbe.app"
                    original = (str(args.out / "native-dd") if host == "xcode" else "/home/hyzyla_home/" + args.linux_dd) + "/Build/Products"
                    clean_original = (str(args.out / "native-dd-clean") if host == "xcode" else "/home/hyzyla_home/" + args.linux_dd + "-clean") + "/Build/Products"
                    controls[host] = checks.compare(incremental / suffix, clean / suffix,
                        {"xcode": original, "linux": clean_original})
                summary[phase]["incrementalVsClean"] = controls
                native_archive_drift = controls["xcode"]["findings"] == [
                    {"kind": "extraFile", "file": "Metadata.appintents/root.ssu.yaml"}]
                summary[phase]["nativeIncrementalArchiveDrift"] = native_archive_drift
                # Linux must match its clean build and the fresh native build.
                # Preserve the native-only archived-source discrepancy in the
                # strict self-comparison; do not discard it from the report.
                summary[phase]["passed"] = controls["linux"]["passed"] and (
                    controls["xcode"]["passed"] or native_archive_drift)
            save(summary_path, summary)
            if not summary[phase]["passed"]:
                raise SystemExit(1)
        if args.baseline_only:
            break


if __name__ == "__main__":
    main()
