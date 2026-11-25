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
FIXTURES = Path(__file__).resolve().parents[1]


def compile_variant_bundle(bundle, binary=None):
    """Build the same inputs with the Linux CLI or the installed Xcode tools."""
    bundle.mkdir(parents=True, exist_ok=True)
    target = ["--target-device", "iphone", "--target-device", "ipad", "--minimum-deployment-target", "15.6"]
    commands = []
    for source in ["Base.lproj/AttributedLabels.xib", "ButtonStateColors.xib"]:
        xib = FIXTURES / "ibtool" / source
        commands.append(["ibtool", "--module", "App", *target, "--compile", bundle / (xib.stem + ".nib"), xib])
    commands.append(["actool", FIXTURES / "actool/resources/SizeClasses.xcassets",
                     "--platform", "iphonesimulator", *target, "--compile", bundle])
    for command in commands:
        if binary is None:
            command = ["xcrun", *command]
        subprocess.run(command, executable=binary, check=True)
    with (bundle / "Info.plist").open("wb") as f:
        plistlib.dump({"CFBundleIdentifier": f"dev.sweetpad.{bundle.stem}", "CFBundleName": bundle.stem,
                       "CFBundlePackageType": "BNDL", "CFBundleVersion": "1"}, f)


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
    print(f"Compiled {len(xibs)} XIBs and {len(storyboards)} storyboards", flush=True)

    common = ["actool", maps / "Images.xcassets", "--platform", "iphonesimulator",
              "--target-device", "iphone", "--target-device", "ipad", "--bundle-identifier", "dev.sweetpad.CoMapsProbe"]
    for target in ["15.6", "17.0"]:
        subprocess.run([*common, "--minimum-deployment-target", target, "--generate-swift-asset-symbols",
                        out / f"Symbols-{target}.swift"], executable=binary, check=True)
    subprocess.run([*common, "--minimum-deployment-target", "15.6", "--compile", out],
                   executable=binary, check=True)
    if not (out / "Assets.car").stat().st_size:
        raise SystemExit("Empty compiled CoMaps catalog")
    compile_variant_bundle(out / "LinuxVariants.bundle", binary)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("checkout", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    compile_resources(args.binary.resolve(), args.checkout.resolve(), args.output.resolve())
