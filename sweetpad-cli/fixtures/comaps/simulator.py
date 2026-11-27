"""Type-check the original CoMaps calls and load the Linux-built nib and catalog in UIKit."""
import argparse
import json
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET

from compile import CATALOGS, NAVIGATION, WIDGET, compile_image_bundle, compile_variant_bundle, write_bundle_info

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


def compare_symbol_catalog(linux, native_rows, catalog, count):
    def glyphs(rows):
        fields = ["Idiom", "Scale", "Reference Point Size", "Template Version", "Interpolatable", "Glyph RenderingMode",
                  "Contains Hierarchical Layers", "Contains Multicolor Layers"]
        return {(row["Name"], row["Glyph Weight"], row["Glyph Size"]): tuple(row.get(key) for key in fields)
                for row in rows if row.get("AssetType") == "Vector Glyph"}

    expected = glyphs(native_rows)
    actual = glyphs(json.loads(xcrun("assetutil", "--info", linux / "Assets.car")))
    sets = sorted(catalog.rglob("*.symbolset"))
    names = sorted(path.stem for path in sets)
    if len(sets) != count or any({key[0] for key in compiled} != set(names) for compiled in (expected, actual)):
        raise SystemExit(f"Expected {count} CoMaps symbols in both catalogs for {catalog}")
    # Xcode versions can also materialize interpolated masters and cache different point sizes.
    # Compare the authored masters here. UIKit exercises all 27 weight/size combinations below.
    sizes = {"S": "Small", "M": "Medium", "L": "Large"}
    authored = set()
    for path in sets:
        slot, = json.loads((path / "Contents.json").read_text())["symbols"]
        masters = ET.parse(path / slot["filename"]).find('.//{*}g[@id="Symbols"]')
        if masters is None or not len(masters):
            raise SystemExit(f"No symbol masters in {path}")
        for master in masters.findall("{*}g"):
            if len(master):
                weight, size = master.attrib["id"].rsplit("-", 1)
                authored.add((path.stem, weight, sizes[size]))
    for key in sorted(authored):
        if key not in expected or key not in actual or expected[key] != actual[key]:
            raise SystemExit(f"Symbol master {key} differs in {catalog}: Xcode {expected.get(key)}, Linux {actual.get(key)}")
    print(f"{catalog.name}: matched {len(authored)} authored masters. Additional interpolated masters: "
          f"Xcode {len(expected.keys() - authored)}, Linux {len(actual.keys() - authored)}.", flush=True)
    cache_keys = sorted({(row["Name"], row["Glyph Weight"], row["Glyph Size"], row["Scale"])
                         for row in native_rows if row.get("AssetType") == "Image" and "Glyph Cached Index" in row})
    return {"names": names, "cached": [dict(zip(("name", "weight", "size", "display"), key)) for key in cache_keys]}


def compile_comaps_reference(resources, app, reference_output):
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
    shutil.unpack_archive(resources / "CoMapsWidget.zip", app.parent)
    widget_catalog = app.parent / WIDGET
    native_widget = app / "XcodeWidget.bundle"
    linux_widget = app / "LinuxWidget.bundle"
    compile_image_bundle(native_widget, [widget_catalog])
    if reference_output:
        reference_output.mkdir(parents=True, exist_ok=True)
        for native in [bundle, native_widget]:
            shutil.copyfile(native / "Assets.car", reference_output / (native.stem + ".car"))
    shutil.copytree(resources / linux_widget.name, linux_widget)
    widget_rows = json.loads(xcrun("assetutil", "--info", native_widget / "Assets.car"))
    app_symbols = compare_symbol_catalog(app, rows, catalog, 55)
    widget_symbols = compare_symbol_catalog(linux_widget, widget_rows, widget_catalog, 1)
    (app / "CoMapsSymbols.json").write_text(json.dumps({"app": app_symbols, "widget": widget_symbols}))
    return len(names)


def check(resources, simctl, simulator, reference_output):
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
        pdf_count = compile_comaps_reference(resources, app, reference_output)
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
                            HERE / "PDFMasks.swift", HERE / "CustomSymbols.swift",
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
                                  simulator, bundle_id], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=240)
            lines = log.stdout.splitlines()
            symbol_result = next((line for line in lines if line.startswith("Compared CoMaps symbols")), None)
            if log.returncode or not symbol_result or "CoMaps resources and variant bundles passed" not in lines:
                raise SystemExit(f"CoMaps iOS {target} probe failed\n{log.stdout}")
            print(symbol_result, flush=True)
            print(f"iOS {target} build passed the CoMaps nib, size-class and layout-direction checks "
                  f"and compared {pdf_count} PDF images with Xcode at 1x, 2x and 3x", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("resources", type=Path, help="output of compile.py")
    parser.add_argument("--simulator", help="reuse a simulator by UDID (otherwise create and delete a test iPhone)")
    parser.add_argument("--device-set", type=Path)
    parser.add_argument("--reference-output", type=Path, help="save the Xcode catalogs to reproduce a comparison")
    args = parser.parse_args()
    simctl = ["simctl", "--set", str(args.device_set.resolve())] if args.device_set else ["simctl"]
    simulator = args.simulator or create_iphone(simctl)
    try:
        subprocess.run(["xcrun", *simctl, "bootstatus", simulator, "-b"], check=True, timeout=600)
        check(args.resources.resolve(), simctl, simulator, args.reference_output)
    finally:
        if not args.simulator:
            subprocess.run(["xcrun", *simctl, "shutdown", simulator], check=False, timeout=60)
            subprocess.run(["xcrun", *simctl, "delete", simulator], check=True, timeout=60)
