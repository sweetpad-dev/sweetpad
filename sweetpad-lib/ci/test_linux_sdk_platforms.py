#!/usr/bin/env python3
"""Portable SDK-import regressions. No Xcode or Apple SDK files are needed."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("sdk_platforms", Path(__file__).with_name("linux-sdk-platforms.py"))
sdk_platforms = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sdk_platforms)


class SDKImportTests(unittest.TestCase):
    def test_imports_versioned_backdeployment_libraries_without_host_tools(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            relative = Path("Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib")
            source = root / "Xcode.app/Contents" / relative / "swift-6.2/iphonesimulator"
            source.mkdir(parents=True)
            (source / "runtime.data").write_bytes(b"Mach-O runtime fixture")
            (source / "libswiftCompatibilitySpan.dylib").symlink_to("runtime.data")
            (source / "macOS-only-executable").write_bytes(b"do not import")
            bundle = root / "bundle"
            copied = sdk_platforms.copy_swift_runtimes(root / "Xcode.app", bundle)
            target = bundle / relative / "swift-6.2/iphonesimulator/libswiftCompatibilitySpan.dylib"
            self.assertEqual(target.read_bytes(), b"Mach-O runtime fixture")
            self.assertFalse(target.is_symlink())
            self.assertFalse(target.with_name("macOS-only-executable").exists())
            self.assertEqual(copied, ["swift-6.2/iphonesimulator/libswiftCompatibilitySpan.dylib"])

    def test_repairs_same_size_corruption_and_preserves_sdk_aliases(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source, destination = root / "source", root / "destination"
            source.mkdir()
            original = bytes(range(256)) * 300
            (source / "SDK.data").write_bytes(original)
            (source / "SDK.alias").symlink_to("SDK.data")
            sdk_platforms.copy_tree(source, destination)
            (destination / "SDK.data").write_bytes(bytes(len(original)))
            stamp = (source / "SDK.data").stat()
            os.utime(destination / "SDK.data", ns=(stamp.st_atime_ns, stamp.st_mtime_ns))
            sdk_platforms.copy_tree(source, destination)
            self.assertEqual((destination / "SDK.data").read_bytes(), original)
            self.assertEqual((destination / "SDK.alias").readlink(), Path("SDK.data"))

    def test_registers_each_architecture_and_retains_existing_platforms(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            original = {"toolsetPaths": ["toolset.json"], "swiftResourcesPath": "Resources"}
            metadata = root / "swift-sdk.json"
            metadata.write_text(json.dumps({"schemaVersion": "4.0", "targetTriples": {"arm64-apple-ios": original}}))
            sdk = root / "Developer/Platforms/WatchSimulator.platform/Developer/SDKs/WatchSimulator.sdk"
            sdk.mkdir(parents=True)
            (sdk / "SDKSettings.json").write_text(json.dumps({
                "CanonicalName": "watchsimulator27.2", "SupportedTargets": {"watchsimulator": {
                    "LLVMTargetTripleVendor": "apple", "LLVMTargetTripleSys": "watchos",
                    "LLVMTargetTripleEnvironment": "simulator", "Archs": ["arm64", "x86_64"],
                }},
            }))
            sdk.with_name("WatchSimulator27.2.sdk").symlink_to("WatchSimulator.sdk")
            sdk_platforms.update_triples(root)
            content = metadata.read_text()
            triples = json.loads(content)["targetTriples"]
            self.assertEqual(triples["arm64-apple-ios"], original)
            self.assertEqual(set(triples), {"arm64-apple-ios", "arm64-apple-watchos-simulator", "x86_64-apple-watchos-simulator"})
            for arch in ["arm64", "x86_64"]:
                entry = triples[f"{arch}-apple-watchos-simulator"]
                self.assertTrue((root / entry["sdkRootPath"] / "SDKSettings.json").is_file())
                self.assertEqual(entry["swiftResourcesPath"], "Resources")
            sdk_platforms.update_triples(root)
            self.assertEqual(metadata.read_text(), content)


if __name__ == "__main__":
    unittest.main()
