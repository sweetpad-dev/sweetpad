#!/usr/bin/env python3
"""Compare native, raw LLD, and Sweetpad links across deployment targets.

The tiny assembly fixture isolates framework autolinking from Swift codegen.
All generated objects and libraries stay in the ignored evidence directory.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess

CI = Path(__file__).resolve().parent
REPO = CI.parent.parent


def run(command, env=None):
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=90)
    if result.returncode:
        raise RuntimeError(f"{command}: {result.stderr}")
    return result


def worker(args):
    bundle = Path.home() / ".swiftpm/swift-sdks/darwin.artifactbundle"
    sdk = bundle / "Developer/Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator.sdk"
    compiler = Path.home() / "swift/swift-6.4.2-RELEASE/usr/bin/clang"
    linker = bundle / "toolset/bin/ld64.lld"
    wrapper = args.out / "ld64.lld"
    for path, target in [(wrapper, Path.home() / "sweetpad-target/debug/sweetpad"),
                         (args.out / "sdk-ld64.lld", linker)]:
        path.unlink(missing_ok=True)
        path.symlink_to(target)
    return compiler, sdk, [("raw", linker), ("wrapped", wrapper)]


def build(args):
    env = dict(os.environ, DEVELOPER_DIR=args.developer_dir)
    if args.worker:
        compiler, sdk, linkers = worker(args)
        compile_prefix = [str(compiler)]
    else:
        sdk = Path(args.developer_dir) / "Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator.sdk"
        compile_prefix = ["xcrun", "clang"]
        linkers = [("native", "ld")]
    version = json.loads((sdk / "SDKSettings.json").read_text())["Version"]
    template = (REPO / "sweetpad-cli/fixtures/linker/swiftui-autolink.s").read_text()
    for minimum in ["15.0", "17.6", "18.0", "27.0"]:
        for order in ["core-first", "ui-first", "unused"]:
            stem = f"{minimum}-{order}"
            source = args.out / (stem + ".s")
            text = template
            if order == "ui-first":
                text = text.replace('"SwiftUICore"', '"TEMP"').replace('"SwiftUI"', '"SwiftUICore"').replace('"TEMP"', '"SwiftUI"')
            elif order == "unused":
                text = text[:text.index(".section")] + text[text.index(".linker_option"):]
            source.write_text(text)
            object_path = args.out / (stem + ("-linux.o" if args.worker else "-native.o"))
            run([*compile_prefix, "-target", f"arm64-apple-ios{minimum}-simulator",
                 "-isysroot", str(sdk), "-c", str(source), "-o", str(object_path)], env)
            for host, linker in linkers:
                command = ([str(linker)] if args.worker else ["xcrun", str(linker)])
                common = ["-dylib", "-arch", "arm64", "-platform_version", "ios-simulator",
                          minimum, version, "-syslibroot", str(sdk)]
                for weak in [False, True]:
                    output = args.out / (stem + ("-weak" if weak else "") + f"-{host}.dylib")
                    extra = ["-weak_framework", "SwiftUI"] if weak else []
                    run([*command, *common, *extra, str(object_path), "-lSystem", "-o", str(output)], env)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=CI.parent / "corpus/_linux-followups/validation/linking-matrix")
    parser.add_argument("--vm", default="sweetpad-linux")
    parser.add_argument("--developer-dir", default="/Applications/Xcode.app/Contents/Developer")
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    build(args)
    if args.worker:
        return
    run(["orb", "-m", args.vm, "python3", str(Path(__file__).resolve()), "--worker", "--out", str(args.out)])
    spec = importlib.util.spec_from_file_location("comparison", CI / "bundle_comparison.py")
    comparison = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(comparison)
    records = []
    for path in sorted(args.out.glob("*-native.dylib")):
        stem = path.name.removesuffix("-native.dylib")
        native = comparison.macho(path)
        raw = comparison.macho(args.out / (stem + "-raw.dylib"))
        wrapped = comparison.macho(args.out / (stem + "-wrapped.dylib"))
        records.append({"case": stem, "native": native, "raw": raw, "wrapped": wrapped,
                        "rawMatchesNative": raw == native, "passed": wrapped == native})
        print(stem, "pass" if wrapped == native else "FAIL", flush=True)
    (args.out / "summary.json").write_text(json.dumps(records, indent=2) + "\n")
    if not records or not all(r["passed"] for r in records):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
