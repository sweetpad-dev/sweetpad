"""Compile the CoMaps resources on Linux and export inputs for simulator.py on macOS.

Use the CoMaps revision pinned in .github/workflows/sweetpad-lib.yaml.
"""
import argparse
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile

CELL = Path("UI/Editor/OpeningHours/Cells/MWMOpeningHoursTimeSelectorTableViewCell.xib")
NAVIGATION = Path("Classes/CustomViews/NavigationDashboard/Views/NavigationControlView.xib")
WIDGET = Path("CoMapsWidgetExtension/Assets.xcassets")
FIXTURES = Path(__file__).resolve().parents[1]
CATALOGS = FIXTURES / "actool/resources"
IOS_TARGET = ["--target-device", "iphone", "--target-device", "ipad", "--minimum-deployment-target", "15.6"]
COMAPS_ICONS = ["--app-icon", "Icon", "--alternate-app-icon", "Debug Icon"]


def write_bundle_info(bundle):
    with (bundle / "Info.plist").open("wb") as f:
        plistlib.dump({"CFBundleIdentifier": f"dev.sweetpad.{bundle.stem}", "CFBundleName": bundle.stem,
                       "CFBundlePackageType": "BNDL", "CFBundleVersion": "1"}, f)


def run_tool(command, binary=None):
    subprocess.run(command if binary else ["xcrun", *command], executable=binary, check=True)


def compile_image_bundle(bundle, catalogs, binary=None):
    bundle.mkdir(parents=True, exist_ok=True)
    run_tool(["actool", *catalogs, "--platform", "iphonesimulator", *IOS_TARGET, "--compile", bundle], binary)
    write_bundle_info(bundle)


def compile_variant_bundle(bundle, binary=None):
    """Build the same inputs with the Linux CLI or the installed Xcode tools."""
    compile_image_bundle(bundle, [CATALOGS / "SizeClasses.xcassets", CATALOGS / "Direction.xcassets"], binary)
    for source in ["Base.lproj/AttributedLabels.xib", "ButtonStateColors.xib"]:
        xib = FIXTURES / "ibtool" / source
        run_tool(["ibtool", "--module", "App", *IOS_TARGET, "--compile", bundle / (xib.stem + ".nib"), xib], binary)


def compile_gap_bundle(bundle, binary=None):
    bundle.mkdir(parents=True, exist_ok=True)
    run_tool(["actool", CATALOGS / "CoMapsSlots.xcassets", "--platform", "iphonesimulator", *IOS_TARGET,
              "--app-icon", "MapIcon", "--output-partial-info-plist", bundle / "IconInfo.plist",
              "--compile", bundle], binary)
    write_bundle_info(bundle)
    interfaces = FIXTURES / "ibtool/runtime/comaps"
    for source in sorted(interfaces.iterdir()):
        suffix = {".xib": ".nib", ".storyboard": ".storyboardc"}.get(source.suffix)
        if suffix:
            run_tool(["ibtool", "--module", "App", *IOS_TARGET, "--compile",
                      bundle / (source.stem + suffix), source], binary)
    run_tool(["ibtool", "--module", "App", "--target-device", "iphone", "--target-device", "ipad",
              "--minimum-deployment-target", "17.0", "--compile", bundle / "CoMapsAdaptiveModern.nib",
              interfaces / "CoMapsAdaptiveViews.xib"], binary)


def compile_resources(binary, checkout, out):
    maps = checkout / "iphone/Maps"
    xibs = sorted(maps.rglob("*.xib"))
    storyboards = sorted(maps.rglob("*.storyboard"))
    if (len(xibs), len(storyboards)) != (78, 5):
        raise SystemExit(f"Expected 78 XIBs and 5 storyboards in {maps}, found {len(xibs)} and {len(storyboards)}")
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        for source in [*xibs, *storyboards]:
            product = (tmp / source.relative_to(maps)).with_suffix(".nib" if source.suffix == ".xib" else ".storyboardc")
            product.parent.mkdir(parents=True, exist_ok=True)
            command = ["ibtool", "--module", "CoMaps", "--target-device", "iphone",
                       "--minimum-deployment-target", "15.6", "--compile", product, source]
            result = subprocess.run(command, executable=binary, capture_output=True, text=True)
            if result.returncode:
                raise SystemExit(f"Failed to compile {source}\n{result.stdout}{result.stderr}")
            nibs = list(product.rglob("*.nib")) if source.suffix == ".storyboard" else [product]
            if not nibs or not all(nib.is_file() and nib.stat().st_size for nib in nibs):
                raise SystemExit(f"Missing compiled nib data for {source}")
        shutil.copyfile((tmp / CELL).with_suffix(".nib"), out / "TimeSelector.nib")
        shutil.copyfile((tmp / NAVIGATION).with_suffix(".nib"), out / "LinuxNavigation.nib")
        shutil.copyfile(maps / NAVIGATION, out / NAVIGATION.name)
    print(f"Compiled {len(xibs)} XIBs and {len(storyboards)} storyboards", flush=True)

    common = ["actool", maps / "Images.xcassets", "--platform", "iphonesimulator",
              "--target-device", "iphone", "--target-device", "ipad", "--bundle-identifier", "dev.sweetpad.CoMapsProbe"]
    for target in ["15.6", "17.0"]:
        subprocess.run([*common, "--minimum-deployment-target", target, "--generate-swift-asset-symbols",
                        out / f"Symbols-{target}.swift"], executable=binary, check=True)
    subprocess.run([*common, "--minimum-deployment-target", "15.6", *COMAPS_ICONS,
                    "--output-partial-info-plist", out / "CoMapsIconInfo.plist", "--compile", out],
                   executable=binary, check=True)
    if not (out / "Assets.car").stat().st_size:
        raise SystemExit("Empty compiled CoMaps catalog")
    # Pass the original catalog to Xcode so UIKit can compare the same named images.
    shutil.make_archive(str(out / "CoMapsImages"), "zip", maps, "Images.xcassets")
    compile_image_bundle(out / "LinuxWidget.bundle", [maps / WIDGET], binary)
    shutil.make_archive(str(out / "CoMapsWidget"), "zip", maps, WIDGET)
    compile_variant_bundle(out / "LinuxVariants.bundle", binary)
    compile_gap_bundle(out / "LinuxSlots.bundle", binary)
    compile_image_bundle(out / "LinuxPDFMasks.bundle", [CATALOGS / "PdfMasks.xcassets"], binary)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("checkout", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    compile_resources(args.binary.resolve(), args.checkout.resolve(), args.output.resolve())
