"""Archive integrity, extraction safety, and replacement rollback checks."""
import hashlib
import importlib.util
import io
import json
import os
import plistlib
import shutil
import subprocess
from pathlib import Path
import tarfile
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("linux_sdk", Path(__file__).with_name("linux-sdk.py"))
sdk = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(sdk)
GENERATOR_SPEC = importlib.util.spec_from_file_location(
    "generate_xcode_archive_fixtures", Path(__file__).with_name("generate_xcode_archive_fixtures.py"))
generator = importlib.util.module_from_spec(GENERATOR_SPEC)
GENERATOR_SPEC.loader.exec_module(generator)


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
        args = SimpleNamespace(archive=archive, destination=destination, replace=True)
        with patch.object(sdk.platform, "system", return_value="Linux"), patch.object(sdk.platform, "machine", return_value="aarch64"):
            with self.assertRaisesRegex(ValueError, "not a Darwin SDK"):
                sdk.import_sdk(args)
        self.assertEqual((destination / "keep").read_text(), "keep")

    def test_failed_builder_preserves_existing_sdk(self):
        archive = self.archive()
        destination = self.root / "existing-sdk"; destination.mkdir()
        (destination / "important").write_text("previous SDK")
        (destination / "swift-sdk.json").write_text("{}")
        args = SimpleNamespace(archive=archive, destination=destination, replace=True)
        with patch.object(sdk.platform, "system", return_value="Linux"), patch.object(sdk.platform, "machine", return_value="aarch64"), \
             patch.dict(os.environ, {"SWEETPAD_EXE": "sweetpad"}), \
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

    def test_xcode_archive_import_reads_the_build_machine_from_xcode(self):
        info = plistlib.dumps({"CFBundleIdentifier": "com.apple.dt.Xcode", "BuildMachineOSBuild": "25C56"})
        platforms = [(f"Xcode.app/Contents/Developer/Platforms/{name}.platform/Developer/SDKs/{name}.sdk/SDKSettings.json",
                      b"{}", 0o100644, 10 + index, 1) for index, name in enumerate(sdk.BASE_PLATFORMS)]
        version = ("Xcode.app/Contents/version.plist", generator.VERSION, 0o100644, 2, 1)
        for name, items, build in [
            ("with-info", [("Xcode.app/Contents/Info.plist", info, 0o100644, 1, 1), version] + platforms, "25C56"),
            ("without-info", [version] + platforms, None),
        ]:
            with self.subTest(name):
                archive = self.root / f"{name}.xip"
                archive.write_bytes(generator.xip(generator.cpio(items)))
                with patch("sys.stderr", io.StringIO()):
                    manifest = sdk.unpack_xcode_archive(archive, self.root / name, SimpleNamespace())
                self.assertEqual(manifest, {"xcodeVersion": "27.2", "xcodeBuild": "fixture", "buildMachineOSBuild": build})

    def test_the_bundle_is_written_by_the_sweetpad_that_runs_the_import(self):
        phrases = self.root / "appintents-phrases.json"; phrases.write_text("{}")
        for given, expected in [
            ((phrases, "25C56"), ["--appintents-phrases", str(phrases), "--build-machine-os-build", "25C56"]),
            ((self.root / "absent.json", None), []),
        ]:
            with self.subTest(given=given), patch.dict(os.environ, {"SWEETPAD_EXE": "/opt/sweetpad/bin/sweetpad"}), \
                 patch.object(sdk.subprocess, "run") as run:
                sdk.write_bundle(Path("/x/Xcode.app"), Path("/w/darwin.artifactbundle"), *given)
            self.assertEqual(run.call_args.args[0], [
                "/opt/sweetpad/bin/sweetpad", "sdk", "write-bundle", "--xcode", "/x/Xcode.app",
                "--out", "/w/darwin.artifactbundle", "--json", *expected])
        with patch.dict(os.environ, {"SWEETPAD_EXE": "", "PATH": ""}):
            with self.assertRaisesRegex(ValueError, "needs the sweetpad CLI"):
                sdk.sweetpad_exe()

    def test_an_xcode_app_import_is_reused_until_xcode_changes(self):
        xcode = self.root / "Xcode.app"
        (xcode / "Contents").mkdir(parents=True)
        def version(build):
            (xcode / "Contents/version.plist").write_bytes(plistlib.dumps(
                {"CFBundleShortVersionString": "27.2", "ProductBuildVersion": build}))
        version("27B52")
        destination = self.root / "xcode-sdk"
        args = SimpleNamespace(xcode=xcode, destination=destination, replace=False,
                               appintents_phrases=None, build_machine_os_build=None)

        def write(xcode, bundle, phrases, build_machine):
            for name in ["swift-sdk.json", "version.plist",
                         "Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/clang/include/stddef.h"]:
                path = bundle / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_text("{}")

        with patch.object(sdk.platform, "system", return_value="Linux"), patch.object(sdk.platform, "machine", return_value="aarch64"), \
             patch.object(sdk, "write_bundle", side_effect=write) as written:
            first = sdk.import_sdk(args)
            again = sdk.import_sdk(args)
            version("27C10")
            with self.assertRaisesRegex(ValueError, "already exists"):
                sdk.import_sdk(args)
        self.assertEqual(written.call_count, 1)
        self.assertFalse(first["reused"])
        self.assertTrue(again["reused"])
        receipt = json.loads((destination / "sweetpad-sdk.json").read_text())
        self.assertEqual((receipt["sourceFormat"], receipt["xcodeBuild"]), ("xcode-app", "27B52"))
        self.assertNotIn("archiveSha256", receipt)

    def cached_sdk(self, archive):
        destination = self.root / "cached-sdk"
        for name in ["swift-sdk.json", "version.plist",
                     "Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/clang/include/stddef.h",
                     "appintents-phrases.json", "keep"]:
            path = destination / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b"cached data")
        receipt = {"archiveSha256": sdk.digest(archive), "hostArchitecture": "aarch64",
                   "importerVersion": sdk.IMPORTER_VERSION, "toolchain": dict(sdk.VERSIONS)}
        (destination / "sweetpad-sdk.json").write_text(json.dumps(receipt))
        args = SimpleNamespace(archive=archive, destination=destination, replace=False)
        return args, receipt

    def test_current_recipe_reuses_sdk_without_rebuilding(self):
        args, _ = self.cached_sdk(self.archive())
        with patch.object(sdk.platform, "system", return_value="Linux"), patch.object(sdk.platform, "machine", return_value="aarch64"), \
             patch.object(sdk.subprocess, "check_output", side_effect=AssertionError("cache should not rebuild")):
            result = sdk.import_sdk(args)
        self.assertTrue(result["reused"])
        self.assertEqual(result["importerVersion"], sdk.IMPORTER_VERSION)

    def test_old_recipe_and_toolchain_require_explicit_replacement(self):
        args, current = self.cached_sdk(self.archive())
        legacy = dict(current); legacy.pop("importerVersion")
        old_tools = dict(current, toolchain=dict(sdk.VERSIONS, swift="old Swift"))
        for receipt in [legacy, old_tools]:
            with self.subTest(receipt=receipt):
                path = args.destination / "sweetpad-sdk.json"; path.write_text(json.dumps(receipt))
                with patch.object(sdk.platform, "system", return_value="Linux"), patch.object(sdk.platform, "machine", return_value="aarch64"):
                    with self.assertRaisesRegex(ValueError, "recipe or toolchain changed;.*--replace.*--replace-sdk"):
                        sdk.import_sdk(args)
                self.assertEqual((args.destination / "keep").read_bytes(), b"cached data")
                self.assertEqual(json.loads(path.read_text()), receipt)

    @unittest.skipUnless(os.uname().sysname == 'Linux', 'Host bootstrap uses the Linux shell')
    def test_root_bootstrap_does_not_require_sudo(self):
        helper = Path(__file__).with_name('linux-host-tools.sh')
        tools=self.root/'tools'; tools.mkdir()
        log=self.root/'apt.log'
        for name,body in {
            'id':'echo 0',
            'sudo':'exit 99',
            'apt-get':'test "$DEBIAN_FRONTEND" = noninteractive || exit 2\nprintf "%s\\n" "$1" >> "$APT_FIXTURE_LOG"',
        }.items():
            path=tools/name; path.write_text('#!/bin/sh\n'+body+'\n'); path.chmod(0o755)
        env=dict(os.environ,PATH=f'{tools}:/usr/bin:/bin',APT_FIXTURE_LOG=str(log))
        subprocess.run(['bash', '-euc',
                        'source "$1"; SWEETPAD_PACKAGE_MANAGER=apt; SWEETPAD_HOST_NAME=fixture; '
                        'SWEETPAD_HOST_PACKAGES=(fixture-package); sweetpad_install_host_packages',
                        'bootstrap-test', str(helper)], env=env, check=True, capture_output=True, text=True)
        self.assertEqual(log.read_text().splitlines(),['update','install'])


if __name__ == "__main__":
    unittest.main()
