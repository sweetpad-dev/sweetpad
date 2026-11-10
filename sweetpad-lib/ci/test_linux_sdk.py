"""Archive integrity, extraction safety, and replacement rollback checks."""
import hashlib
import importlib.util
import io
import json
import shutil
from pathlib import Path
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("linux_sdk", Path(__file__).with_name("linux-sdk.py"))
sdk = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(sdk)


class SDKTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.root = Path(self.work.name)

    def archive(self, files=None, links=None, omitted=(), changed=(), version=1):
        files = files or {"Xcode.app/Contents/version.plist": b"version", "appintents-phrases.json": b"{}"}
        links = links or {}
        entries = {name: {"type": "file", "size": len(data), "mode": 0o644,
                          "sha256": hashlib.sha256(data).hexdigest()} for name, data in files.items()}
        entries.update({name: {"type": "symlink", "target": target} for name, target in links.items()})
        path = self.root / "sdk.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            manifest = {"format": "sweetpad-apple-sdk", "version": version, "toolchain": sdk.VERSIONS,
                        "entries": entries, "xcodeBuild": "test", "xcodeVersion": "27.2"}
            data = json.dumps(manifest).encode()
            info = tarfile.TarInfo("manifest.json"); info.size = len(data)
            archive.addfile(info, io.BytesIO(data))
            for name, data in files.items():
                if name in omitted:
                    continue
                if name in changed:
                    data = b"x" * len(data)
                info = tarfile.TarInfo(name); info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
            for name, target in links.items():
                info = tarfile.TarInfo(name); info.type = tarfile.SYMTYPE; info.linkname = target
                archive.addfile(info)
        path.with_name(path.name + ".sha256").write_text(f"{sdk.digest(path)}  {path.name}\n")
        return path

    def unpack(self, archive):
        out = self.root / "unpacked"; out.mkdir()
        return sdk.unpack_verified(archive, out)

    def test_round_trip_preserves_relative_link_and_bytes(self):
        archive = self.archive(links={"Xcode.app/Contents/current.plist": "version.plist"})
        manifest = self.unpack(archive)
        self.assertEqual(manifest["xcodeBuild"], "test")
        self.assertEqual((self.root / "unpacked/Xcode.app/Contents/current.plist").read_bytes(), b"version")
        self.assertTrue((self.root / "unpacked/Xcode.app/Contents/current.plist").is_symlink())

    def test_rejects_traversal(self):
        with self.assertRaisesRegex(ValueError, "Unsafe"):
            self.unpack(self.archive(files={"Xcode.app/Contents/../../../escape": b"bad"}))
        self.assertFalse((self.root / "escape").exists())

    def test_rejects_escaping_link(self):
        with self.assertRaises(ValueError):
            self.unpack(self.archive(links={"Xcode.app/Contents/link": "../../../../escape"}))

    def test_rejects_absolute_link(self):
        with self.assertRaises(ValueError):
            self.unpack(self.archive(links={"Xcode.app/Contents/link": "/etc/passwd"}))

    def test_rejects_corrupt_file(self):
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            self.unpack(self.archive(changed=("appintents-phrases.json",)))

    def test_rejects_missing_file(self):
        with self.assertRaisesRegex(ValueError, "missing manifest"):
            self.unpack(self.archive(omitted=("appintents-phrases.json",)))

    def test_rejects_unknown_format(self):
        with self.assertRaisesRegex(ValueError, "Unsupported"):
            self.unpack(self.archive(version=99))

    def test_checksum_file_supports_spaces_in_archive_name(self):
        archive = self.archive().rename(self.root / "Apple SDK.tar.gz")
        archive.with_name(archive.name + ".sha256").write_text(f"{sdk.digest(archive)}  {archive.name}\n")
        self.assertEqual(sdk.inspect_archive(archive)["archiveSha256"], sdk.digest(archive))

    def test_rejects_changed_archive_checksum(self):
        archive = self.archive()
        with archive.open("ab") as target:
            target.write(b"changed")
        with self.assertRaisesRegex(ValueError, "checksum"):
            sdk.inspect_archive(archive)

    def test_rejects_non_sdk_destination_without_removing_files(self):
        archive = self.archive()
        destination = self.root / "personal"; destination.mkdir()
        (destination / "keep").write_text("keep")
        args = SimpleNamespace(archive=archive, destination=destination, replace=True, xtool="xtool")
        with patch.object(sdk.platform, "system", return_value="Linux"), patch.object(sdk.platform, "machine", return_value="aarch64"):
            with self.assertRaisesRegex(ValueError, "not a Darwin SDK"):
                sdk.import_sdk(args)
        self.assertEqual((destination / "keep").read_text(), "keep")

    def test_failed_builder_preserves_existing_sdk(self):
        archive = self.archive()
        destination = self.root / "existing-sdk"; destination.mkdir()
        (destination / "important").write_text("previous SDK")
        (destination / "swift-sdk.json").write_text("{}")
        args = SimpleNamespace(archive=archive, destination=destination, replace=True, xtool="xtool")
        with patch.object(sdk.platform, "system", return_value="Linux"), patch.object(sdk.platform, "machine", return_value="aarch64"), \
             patch.object(sdk.subprocess, "check_output", return_value="xtool 1.21.0\n"), \
             patch.object(sdk.subprocess, "run", side_effect=OSError("builder failed")):
            with self.assertRaisesRegex(OSError, "builder failed"):
                sdk.import_sdk(args)
        self.assertEqual((destination / "important").read_text(), "previous SDK")
        self.assertEqual(list(self.root.glob(".sweetpad-sdk-*")), [])

    def test_export_omits_signing_files_and_host_executables(self):
        xcode = self.root / "Xcode.app"
        for platform in sdk.BASE_PLATFORMS:
            (xcode / f"Contents/Developer/Platforms/{platform}.platform/Developer/SDKs").mkdir(parents=True)
        for relative in ["Contents/version.plist", "Contents/development.key", "Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/swift"]:
            path = xcode / relative; path.parent.mkdir(parents=True, exist_ok=True); path.write_text("test")
        self.assertEqual(set(sdk.export_entries(xcode)), {"Xcode.app/Contents/version.plist"})

    def test_import_supplies_linux_compiler_resource_headers(self):
        resource = self.root / "linux-swift"
        include = resource / "clang/include"; include.mkdir(parents=True)
        (include / "stddef.h").write_text("Linux compiler header")
        bundle = self.root / "sdk"
        with patch.object(sdk.subprocess, "check_output", return_value=json.dumps({"compilerVersion": "Swift version 6.4.2", "paths": {"runtimeResourcePath": str(resource)}})):
            sdk.copy_host_clang_headers(bundle, lambda a, b: shutil.copytree(a, b, dirs_exist_ok=True))
        target = bundle / "Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/clang/include/stddef.h"
        self.assertEqual(target.read_text(), "Linux compiler header")

    def test_import_rejects_an_incompatible_host_compiler(self):
        with patch.object(sdk.subprocess, "check_output", return_value=json.dumps({"compilerVersion": "Swift version 6.3.4"})):
            with self.assertRaisesRegex(ValueError, "requires Swift 6.4.2"):
                sdk.copy_host_clang_headers(self.root / "sdk", shutil.copytree)


if __name__ == "__main__":
    unittest.main()
