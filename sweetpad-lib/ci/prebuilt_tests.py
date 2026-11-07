"""Write simulator XCTest manifests from explicitly selected built bundles.

No Xcode build or native manifest is required. This intentionally covers logic
and app-hosted unit tests; UI tests and test-plan options need a supplied manifest.
"""
import argparse
from pathlib import Path
import plistlib


def manifest(products, bundles, scheme, environment=None):
    products = products.resolve()
    targets = []
    for bundle in bundles:
        bundle = (products / bundle).resolve()
        relative = bundle.relative_to(products)
        info = plistlib.loads((bundle / "Info.plist").read_bytes())
        if bundle.suffix != ".xctest" or not (bundle / info["CFBundleExecutable"]).is_file():
            raise ValueError(f"not a built test bundle: {bundle}")
        if info.get("CFBundleName", "").endswith("UITests"):
            raise ValueError("UI tests require a manifest specifying the target application")
        host = next((p for p in bundle.parents if p.suffix == ".app" and p.is_relative_to(products)), None)
        configuration = relative.parts[0]
        if "iphonesimulator" not in configuration:
            raise ValueError("only iOS simulator test products are supported")
        base = "__TESTROOT__/" + configuration
        developer = "__PLATFORMS__/iPhoneSimulator.platform/Developer"
        testing = {
            "DYLD_FRAMEWORK_PATH": f"{base}:{base}/PackageFrameworks:{developer}/Library/Frameworks",
            "DYLD_LIBRARY_PATH": f"{base}:{developer}/usr/lib",
            "DYLD_INSERT_LIBRARIES": "/usr/lib/libRPAC.dylib",
            "__XCODE_BUILT_PRODUCTS_DIR_PATHS": base,
            "__XPC_DYLD_FRAMEWORK_PATH": base,
            "__XPC_DYLD_LIBRARY_PATH": base,
            "XCODE_SCHEME_NAME": scheme,
        }
        target = {
            "BlueprintName": bundle.stem,
            "ProductModuleName": info["CFBundleExecutable"],
            "TestBundlePath": "__TESTROOT__/" + relative.as_posix(),
            "TestHostPath": developer + "/Library/Xcode/Agents/xctest",
            "DependentProductPaths": ["__TESTROOT__/" + relative.as_posix()],
            "EnvironmentVariables": dict(environment or {}),
            "TestingEnvironmentVariables": testing,
            "SystemAttachmentLifetime": "deleteOnSuccess",
            "UserAttachmentLifetime": "deleteOnSuccess",
        }
        if host:
            host_path = "__TESTROOT__/" + host.relative_to(products).as_posix()
            host_info = plistlib.loads((host / "Info.plist").read_bytes())
            target.update(IsAppHostedTestBundle=True, TestHostPath=host_path,
                          TestHostBundleIdentifier=host_info["CFBundleIdentifier"],
                          TestBundlePath="__TESTHOST__/" + bundle.relative_to(host).as_posix())
            target["DependentProductPaths"].insert(0, host_path)
            testing["DYLD_INSERT_LIBRARIES"] = "__TESTHOST__/Frameworks/libXCTestBundleInject.dylib:/usr/lib/libRPAC.dylib"
            testing["XCInjectBundleInto"] = "unused"
        targets.append(target)
    if not targets:
        raise ValueError("select at least one test bundle")
    return {"__xctestrun_metadata__": {"FormatVersion": 2},
            "ContainerInfo": {"ContainerName": scheme, "SchemeName": scheme},
            "TestConfigurations": [{"Name": "Prebuilt unit tests", "IsEnabled": True, "TestTargets": targets}]}


def write(products, bundles, scheme, environment=None, name="prebuilt.xctestrun"):
    path = products / name
    path.write_bytes(plistlib.dumps(manifest(products, bundles, scheme, environment)))
    return path


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--products", type=Path, required=True)
    parser.add_argument("--scheme", required=True)
    parser.add_argument("bundles", nargs="+", type=Path, help="Explicit paths relative to Products")
    args = parser.parse_args()
    print(write(args.products, args.bundles, args.scheme))
