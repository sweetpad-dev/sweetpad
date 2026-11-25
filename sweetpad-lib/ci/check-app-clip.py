#!/usr/bin/env python3
"""Compare the delivery fixture's prebuilt App Clip products on a Mac with Xcode."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile


FIXTURE = Path(__file__).resolve().parent / "fixture-delivery"
PARENT = "com.apple.developer.parent-application-identifiers"
CLIPS = "com.apple.developer.associated-appclip-app-identifiers"
ON_DEMAND = "com.apple.developer.on-demand-install-capable"


def run(*command, timeout=240):
    result = subprocess.run(command, capture_output=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"{command[0]} failed: {(result.stderr + result.stdout).decode(errors='replace')}")
    return result.stdout


def equal(what, actual, expected):
    if actual != expected:
        raise ValueError(f"{what}: expected {expected!r}, got {actual!r}")


def inspect(app, simulator):
    clip = app / "AppClips/DeliveryClip.app"
    equal("embedded Clips", list(clip.parent.glob("*.app")), [clip])
    metadata, rights = {}, {}
    for role, bundle in [("host", app), ("clip", clip)]:
        run("codesign", "--verify", "--deep", "--strict", bundle)
        info = plistlib.loads((bundle / "Info.plist").read_bytes())
        metadata[role] = {key: info[key] for key in (
            "CFBundleIdentifier", "CFBundleExecutable", "CFBundleVersion",
            "CFBundleShortVersionString", "MinimumOSVersion", "UIDeviceFamily", "DTPlatformName")}
        equal(f"{role} platform", info["DTPlatformName"], "iphonesimulator" if simulator else "iphoneos")
        if role == "clip":
            metadata[role]["NSAppClip"] = info["NSAppClip"]
        if simulator:
            # Simulator entitlements live in the Mach-O, outside the ad hoc signature.
            with tempfile.TemporaryDirectory() as temp:
                output = Path(temp) / "simulated.plist"
                run("xcrun", "segedit", bundle / info["CFBundleExecutable"],
                    "-extract", "__TEXT", "__entitlements", output)
                rights[role] = plistlib.loads(output.read_bytes().rstrip(b"\0"))
        else:
            rights[role] = plistlib.loads(run("codesign", "-d", "--entitlements", "-", "--xml", bundle))
            profile = plistlib.loads(run("security", "cms", "-D", "-i", bundle / "embedded.mobileprovision"))
            with tempfile.TemporaryDirectory() as temp:
                run("codesign", "-d", f"--extract-certificates={temp}/signer", bundle)
                signer = (Path(temp) / "signer0").read_bytes()
                if signer not in profile["DeveloperCertificates"]:
                    raise ValueError(f"{role} signing certificate is missing from its profile")
            association = (CLIPS,) if role == "host" else (PARENT, ON_DEMAND)
            for key in ("application-identifier", "get-task-allow", *association):
                equal(f"{role} signed/profile {key}", rights[role][key], profile["Entitlements"][key])
            equal(f"{role} development signature", rights[role]["get-task-allow"], True)

    equal("Clip bundle ID", metadata["clip"]["CFBundleIdentifier"], metadata["host"]["CFBundleIdentifier"] + ".Clip")
    equal("Clip parent", rights["clip"][PARENT], [rights["host"]["application-identifier"]])
    equal("on-demand capability", rights["clip"][ON_DEMAND], True)
    if not simulator:
        equal("host association", rights["host"][CLIPS], [rights["clip"]["application-identifier"]])
    return {"info": metadata, "entitlements": rights}


def binary_hashes(app):
    info = plistlib.loads((app / "Info.plist").read_bytes())
    executable = app / info["CFBundleExecutable"]
    binaries = [executable]
    # When Xcode emits a debug launcher, the Swift code lives in this dylib.
    debug_dylib = app / (executable.name + ".debug.dylib")
    if debug_dylib.is_file():
        binaries.append(debug_dylib)
    return {binary.name: hashlib.sha256(binary.read_bytes()).hexdigest() for binary in binaries}


def install_on_simulator(udid, bundle):
    # CoreSimulator's install service cannot read some Linux VM mounts.
    with tempfile.TemporaryDirectory() as temp:
        local = Path(temp) / bundle.name
        run("ditto", bundle, local)
        run("xcrun", "simctl", "install", udid, local)


def simulator_check(args):
    reference = inspect(args.xcode, True)
    equal("Linux simulator product", inspect(args.linux, True), reference)
    host_id = reference["info"]["host"]["CFBundleIdentifier"]
    equal("UI test fixture identifier", host_id, "dev.example.delivery")
    clip_id = reference["info"]["clip"]["CFBundleIdentifier"]
    simctl = ("xcrun", "simctl")
    report = {}
    for label, app in [("xcode", args.xcode), ("linux", args.linux)]:
        install_on_simulator(args.simulator, app)
        try:
            run(*simctl, "launch", args.simulator, host_id)
        finally:
            run(*simctl, "uninstall", args.simulator, host_id)
        clip = app / "AppClips/DeliveryClip.app"
        install_on_simulator(args.simulator, clip)
        try:
            result = args.out / f"{label}.xcresult"
            test = json.loads(run(args.sweetpad, "test", "--project", FIXTURE / "Delivery.xcodeproj",
                                  "--scheme", "DeliveryClipInvocation", "--destination",
                                  f"platform=iOS Simulator,id={args.simulator}", "--result-bundle", result,
                                  "--non-interactive", "-o", "json", "--", "-parallel-testing-enabled", "NO",
                                  "-derivedDataPath", args.out / "runner", timeout=600))
            equal(f"{label} UI test", (test["ok"], test["data"]["passed"], test["data"]["passedTests"]),
                  (True, True, 1))
            installed = Path(os.fsdecode(run(*simctl, "get_app_container", args.simulator, clip_id, "app")).strip())
            digests = binary_hashes(installed)
            equal(f"{label} installed Clip binaries", digests, binary_hashes(clip))
            report[label] = {"host": str(app.resolve()), "clipSHA256": digests,
                             "passedTests": test["data"]["passedTests"], "invocationTest": str(result)}
        finally:
            run(*simctl, "uninstall", args.simulator, clip_id)
    return report


def archive_app(archive):
    info = plistlib.loads((archive / "Info.plist").read_bytes())
    return archive / "Products" / info["ApplicationProperties"]["ApplicationPath"]


def uuids(binary):
    output = run("xcrun", "dwarfdump", "--uuid", binary)
    result = {tuple(line.split()[1:3]) for line in output.splitlines() if line.startswith(b"UUID: ")}
    if not result:
        raise ValueError(f"{binary} has no UUID")
    return result


def distribution_check(args):
    native = archive_app(args.xcode)
    linux = archive_app(args.linux)
    reference = inspect(native, False)
    equal("Linux archive", inspect(linux, False), reference)
    for archive, app in [(args.xcode, native), (args.linux, linux)]:
        for bundle in (app, app / "AppClips/DeliveryClip.app"):
            binary = plistlib.loads((bundle / "Info.plist").read_bytes())["CFBundleExecutable"]
            dsym = archive / "dSYMs" / (bundle.name + ".dSYM") / "Contents/Resources/DWARF" / binary
            equal(f"{dsym} UUIDs", uuids(dsym), uuids(bundle / binary))
    with tempfile.TemporaryDirectory() as temp:
        run("ditto", "-x", "-k", args.ipa, temp)
        exported = inspect(Path(temp) / "Payload" / linux.name, False)
        equal("Linux development IPA", exported, reference)
    return {"xcodeArchive": str(args.xcode.resolve()), "linuxArchive": str(args.linux.resolve()),
            "ipa": str(args.ipa.resolve()), **reference}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    sim = modes.add_parser("simulator", help="install each product on a dedicated booted iPhone and exercise its Clip")
    sim.add_argument("--simulator", required=True, help="dedicated iPhone simulator UDID")
    sim.add_argument("--sweetpad", default="sweetpad", help="macOS SweetPad CLI")
    dist = modes.add_parser("distribution", help="verify development-signed device archives and a debugging IPA")
    dist.add_argument("--ipa", type=Path, required=True)
    for sub in (sim, dist):
        sub.add_argument("--xcode", type=Path, required=True, help="Xcode-built host .app or .xcarchive")
        sub.add_argument("--linux", type=Path, required=True, help="Linux-built host .app or .xcarchive")
        sub.add_argument("--out", type=Path, required=True, help="new directory for results")
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    report = simulator_check(args) if args.mode == "simulator" else distribution_check(args)
    (args.out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"{args.mode} passed: {args.out / 'report.json'}")


if __name__ == "__main__":
    main()
