#!/usr/bin/env python3
"""Check that product export preserves an exact bundle and keeps failure evidence."""
import importlib.util
import json
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import subprocess
import plistlib
import struct
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("checks", Path(__file__).with_name("linux-project-checks.py"))
checks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checks)


class ExportTests(unittest.TestCase):
    def test_concurrent_reports_preserve_each_project(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "checks.json"
            with ThreadPoolExecutor(max_workers=8) as pool:
                list(pool.map(lambda i: checks.save_result(output, str(i), {"value": i}), range(32)))
            self.assertEqual(json.loads(output.read_text()), {str(i): {"value": i} for i in range(32)})

    def local_transport(self, command, **kwargs):
        if command[0] == "orb":
            self.assertEqual(command[:4], ["orb", "-m", "test-vm", "tar"])
            return self.popen(command[3:], **kwargs)
        return self.popen(command, **kwargs)

    def setUp(self):
        self.popen = subprocess.Popen

    def test_export_removes_files_deleted_since_previous_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source/Example.app"
            source.mkdir(parents=True)
            (source / "current.txt").write_text("first")
            (source / "obsolete.txt").write_text("remove me")
            with patch.object(checks.subprocess, "Popen", self.local_transport):
                product = checks.copy_product(source, root / "export", "test-vm")
                (source / "current.txt").write_text("second")
                (source / "obsolete.txt").unlink()
                checks.copy_product(source, root / "export", "test-vm")
            self.assertEqual((product / "current.txt").read_text(), "second")
            self.assertFalse((product / "obsolete.txt").exists())

    def test_failed_export_keeps_previous_product(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            product = root / "export/Example.app"
            product.mkdir(parents=True)
            (product / "evidence.txt").write_text("retain")
            source = root / "missing/Example.app"
            with patch.object(checks.subprocess, "Popen", self.local_transport):
                with self.assertRaises((RuntimeError, subprocess.CalledProcessError)):
                    checks.copy_product(source, root / "export", "test-vm")
            self.assertEqual((product / "evidence.txt").read_text(), "retain")


class ComparisonTests(unittest.TestCase):
    def shortcut_models(self, native, linux):
        source = checks.REPO / "sweetpad-cli/fixtures/appintents/DataApp.nlu"
        data = source.read_bytes()
        _, fingerprint = checks.bundle_comparison.appintents_model.Model(data).content()
        for app, marker in [(native, fingerprint), (linux, "a" * len(fingerprint))]:
            folder = app / "en.lproj/nlu.appintents"
            folder.mkdir(parents=True)
            content = data.replace(fingerprint.encode(), marker.encode())
            (folder / "nlu.lzfse").write_bytes(b"bvx-" + struct.pack("<I", len(content)) + content + b"bvx$")
            (folder / (marker + ".version")).write_bytes(b"\0")

    def test_shortcut_fingerprints_normalize_only_after_full_phrase_comparison(self):
        with tempfile.TemporaryDirectory() as tmp:
            native, linux = self.bundles(Path(tmp), {"CFBundleIdentifier": "example"})
            self.shortcut_models(native, linux)
            self.assertTrue(checks.compare(native, linux)["passed"])
            model = linux / "en.lproj/nlu.appintents/nlu.lzfse"
            model.write_bytes(model.read_bytes().replace(b"Greet in DataApp", b"Gr33t in DataApp"))
            result = checks.compare(native, linux)
            self.assertFalse(result["passed"])
            self.assertIn("appShortcutModel", {f["kind"] for f in result["findings"]})

    def test_shortcut_marker_must_match_the_model_header(self):
        with tempfile.TemporaryDirectory() as tmp:
            native, linux = self.bundles(Path(tmp), {"CFBundleIdentifier": "example"})
            self.shortcut_models(native, linux)
            marker, = linux.glob("en.lproj/nlu.appintents/*.version")
            marker.rename(marker.with_name("wrong.version"))
            result = checks.compare(native, linux)
            self.assertFalse(result["passed"])
            self.assertIn("appShortcutModel", {f["kind"] for f in result["findings"]})

    def bundles(self, root, info):
        native, linux = root / "native.app", root / "linux.app"
        for path in [native, linux]:
            path.mkdir()
            (path / "Info.plist").write_bytes(plistlib.dumps(info))
        return native, linux

    def test_complete_info_plist_and_missing_localization_are_failures(self):
        with tempfile.TemporaryDirectory() as tmp:
            native, linux = self.bundles(Path(tmp), {"CFBundleIdentifier": "example", "CFBundleURLTypes": []})
            (linux / "Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": "example", "CFBundleURLTypes": [{"CFBundleURLSchemes": ["unexpected"]}]}))
            (native / "fr.lproj").mkdir()
            (native / "fr.lproj/Localizable.strings").write_bytes(plistlib.dumps({"hello": "Bonjour"}))
            result = checks.compare(native, linux)
            self.assertTrue(result["applicationPropertiesMatch"])
            self.assertFalse(result["passed"])
            self.assertEqual({f["kind"] for f in result["findings"]}, {"bundleInfo", "missingFile"})

    def test_macho_platform_mismatch_is_detected(self):
        with tempfile.TemporaryDirectory() as tmp:
            native, linux = self.bundles(Path(tmp), {"CFBundleIdentifier": "example", "CFBundleExecutable": "App"})
            for root, platform in [(native, 7), (linux, 2)]:
                command = struct.pack("<IIIIII", 0x32, 24, platform, 18 << 16, 27 << 16, 0)
                header = struct.pack("<IiiIIIII", 0xfeedfacf, 0x100000c, 0, 2, 1, len(command), 0, 0)
                (root / "App").write_bytes(header + command)
            result = checks.compare(native, linux)
            self.assertFalse(result["passed"])
            self.assertEqual(result["findings"][0]["kind"], "machO")

    def test_only_known_build_roots_are_normalized_and_rpath_order_matters(self):
        def binary(paths):
            commands = []
            for path in paths:
                text = path.encode() + b"\0"
                size = (12 + len(text) + 7) // 8 * 8
                commands.append(struct.pack("<III", 0x8000001c, size, 12) + text + b"\0" * (size - 12 - len(text)))
            data = b"".join(commands)
            return struct.pack("<IiiIIIII", 0xfeedfacf, 0x100000c, 0, 2, len(commands), len(data), 0, 0) + data
        with tempfile.TemporaryDirectory() as tmp:
            native, linux = self.bundles(Path(tmp), {"CFBundleExecutable": "App"})
            (native / "App").write_bytes(binary(["/native/Products/Frameworks", "@loader_path"]))
            (linux / "App").write_bytes(binary(["/linux/Products/Frameworks", "@loader_path"]))
            roots = {"xcode": "/native/Products", "linux": "/linux/Products"}
            self.assertTrue(checks.compare(native, linux, roots)["passed"])
            (linux / "App").write_bytes(binary(["/linux/Products-unrelated/Frameworks", "@loader_path"]))
            self.assertFalse(checks.compare(native, linux, roots)["passed"])
            (linux / "App").write_bytes(binary(["@loader_path", "/linux/Products/Frameworks"]))
            self.assertFalse(checks.compare(native, linux, roots)["passed"])


if __name__ == "__main__":
    unittest.main()
