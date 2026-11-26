"""Type-check the original CoMaps calls and load the Linux-built nib and catalog in UIKit."""
import argparse
import json
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import tempfile

from compile import CATALOGS, NAVIGATION, compile_image_bundle, compile_variant_bundle, write_bundle_info

HERE = Path(__file__).resolve().parent
BUNDLE_ID = "dev.sweetpad.CoMapsProbe"


def xcrun(*args):
    return subprocess.check_output(["xcrun", *args], text=True).strip()


def create_iphone(simctl):
    devices = json.loads(xcrun(*simctl, "list", "devices", "available", "-j"))["devices"]
    for runtime, entries in reversed(list(devices.items())):
        _, _, version = runtime.partition(".iOS-")
        if not version or int(version.split("-")[0]) < 17:
            continue
        for device in entries:
            model = device["deviceTypeIdentifier"]
            if ".iPhone-" in model:
                return xcrun(*simctl, "create", "SweetPad CoMaps", model, runtime)
    raise SystemExit("The selected Xcode needs an iPhone simulator with iOS 17 or later")


def rendition_manifest(bundle):
    rows = json.loads(xcrun("assetutil", "--info", bundle / "Assets.car"))
    fields = ["Name", "AssetType", "RenditionName", "Idiom", "Scale", "Appearance",
              "SizeClass Horizontal", "SizeClass Vertical", "LayoutDirection", "Flippable", "PixelWidth", "PixelHeight"]
    # Xcode can pack the same public renditions into internal texture atlases.
    return sorted(tuple(row.get(key, "") for key in fields)
                  for row in rows if row.get("AssetType") in {"Image", "Vector"})


def compile_comaps_reference(resources, app):
    shutil.unpack_archive(resources / "CoMapsImages.zip", app.parent)
    catalog = app.parent / "Images.xcassets"
    bundle = app / "XcodeCoMaps.bundle"
    bundle.mkdir()
    write_bundle_info(bundle)
    xcrun("actool", catalog, "--platform", "iphonesimulator", "--target-device", "iphone",
          "--target-device", "ipad", "--minimum-deployment-target", "15.6", "--bundle-identifier", BUNDLE_ID,
          "--compile", bundle)
    rows = json.loads(xcrun("assetutil", "--info", bundle / "Assets.car"))
    names = sorted({r["Name"] for r in rows if r.get("AssetType") == "Vector"
                    and r.get("RenditionName", "").lower().endswith(".pdf")})
    sets = {pdf.parent for pdf in catalog.rglob("*.pdf")}
    if len(names) != len(sets) or not {"ic_booking_plus", "ic_booking_minus"}.issubset(names):
        raise SystemExit(f"Expected all {len(sets)} CoMaps PDF sets including both booking controls, got {names}")
    (app / "CoMapsPDFs.json").write_text(json.dumps(names))
    return len(names)


def check(resources, simctl, simulator):
    device_sdk = xcrun("--sdk", "iphoneos", "--show-sdk-path")
    sim_sdk = xcrun("--sdk", "iphonesimulator", "--show-sdk-path")
    with tempfile.TemporaryDirectory() as tmp:
        app = Path(tmp) / "CoMapsProbe.app"
        app.mkdir()
        for filename in ["TimeSelector.nib", "LinuxNavigation.nib", "Assets.car"]:
            shutil.copyfile(resources / filename, app / filename)
        linux = app / "LinuxVariants.bundle"
        native = app / "XcodeVariants.bundle"
        shutil.copytree(resources / linux.name, linux)
        compile_variant_bundle(native)
        linux_masks = app / "LinuxPDFMasks.bundle"
        native_masks = app / "XcodePDFMasks.bundle"
        shutil.copytree(resources / linux_masks.name, linux_masks)
        compile_image_bundle(native_masks, [CATALOGS / "PdfMasks.xcassets"])
        pdf_count = compile_comaps_reference(resources, app)
        xcrun("ibtool", "--module", "CoMaps", "--target-device", "iphone", "--minimum-deployment-target", "15.6",
              "--compile", app / "XcodeNavigation.nib", resources / NAVIGATION.name)
        expected = rendition_manifest(native) + rendition_manifest(native_masks)
        actual = rendition_manifest(linux) + rendition_manifest(linux_masks)
        if actual != expected:
            raise SystemExit(f"Image renditions differ\nXcode: {expected}\nLinux: {actual}")
        print(f"Matched {len(expected)} Xcode image and vector renditions", flush=True)
        for target in ["15.6", "17.0"]:
            bundle_id = f"{BUNDLE_ID}.ios{target.replace('.', '-')}"
            symbols = resources / f"Symbols-{target}.swift"
            swift = ["swiftc", "-module-name", "CoMapsProbe", symbols, HERE / "Images.swift"]
            subprocess.run(["xcrun", "--sdk", "iphoneos", *swift, "-typecheck", "-sdk", device_sdk, "-target", f"arm64-apple-ios{target}"], check=True)
            print(f"CoMaps calls type-check for iOS {target}", flush=True)
            subprocess.run(["xcrun", "--sdk", "iphonesimulator", *swift, "-parse-as-library", HERE / "PickerApp.swift",
                            HERE / "ResourceVariants.swift", HERE / "ImageDirection.swift", HERE / "NavigationControl.swift",
                            HERE / "PDFMasks.swift",
                            "-sdk", sim_sdk,
                            "-target", f"{platform.machine()}-apple-ios{target}-simulator", "-o", app / "CoMapsProbe"], check=True)
            with (app / "Info.plist").open("wb") as f:
                plistlib.dump({"CFBundleIdentifier": bundle_id, "CFBundleExecutable": "CoMapsProbe",
                               "CFBundleName": "CoMapsProbe", "CFBundlePackageType": "APPL",
                               "CFBundleVersion": "1", "CFBundleShortVersionString": "1.0",
                               "UIApplicationSceneManifest": {"UIApplicationSupportsMultipleScenes": False},
                               "CFBundleSupportedPlatforms": ["iPhoneSimulator"], "MinimumOSVersion": target}, f)
            subprocess.run(["codesign", "--force", "--sign", "-", app], check=True)
            subprocess.run(["xcrun", *simctl, "install", simulator, app], check=True, timeout=180)
            log = subprocess.run(["xcrun", *simctl, "launch", "--console-pty", "--terminate-running-process",
                                  simulator, bundle_id], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=90)
            if log.returncode or "CoMaps resources and variant bundles passed" not in log.stdout.splitlines():
                raise SystemExit(f"CoMaps iOS {target} probe failed\n{log.stdout}")
            print(f"iOS {target} build passed the CoMaps nib, size-class and layout-direction checks "
                  f"and compared {pdf_count} PDF images with Xcode at 1x, 2x and 3x", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("resources", type=Path, help="output of compile.py")
    parser.add_argument("--simulator", help="reuse a simulator by UDID (otherwise create and delete a test iPhone)")
    parser.add_argument("--device-set", type=Path)
    args = parser.parse_args()
    simctl = ["simctl", "--set", str(args.device_set.resolve())] if args.device_set else ["simctl"]
    simulator = args.simulator or create_iphone(simctl)
    try:
        subprocess.run(["xcrun", *simctl, "bootstatus", simulator, "-b"], check=True, timeout=600)
        check(args.resources.resolve(), simctl, simulator)
    finally:
        if not args.simulator:
            subprocess.run(["xcrun", *simctl, "shutdown", simulator], check=False, timeout=60)
            subprocess.run(["xcrun", *simctl, "delete", simulator], check=True, timeout=60)
