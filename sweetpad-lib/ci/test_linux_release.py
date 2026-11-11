import importlib.util
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[2]


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, REPO / "sweetpad-cli/ci" / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


package = load("linux_package", "package-linux.py")
version = load("release_version", "release-version.py")


class LinuxReleaseContracts(unittest.TestCase):
    def test_macro_launcher_works_with_compilers_minimal_environment(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            toolchain = root / "relocated-swift/usr"
            (toolchain / "bin").mkdir(parents=True)
            (toolchain / "lib/swift/linux").mkdir(parents=True)
            (toolchain / "bin/swift").touch()
            (root / "sweetpad-macro-swift").symlink_to(toolchain / "bin/swift")
            binary = root / "sweetpad-macros.bin"
            binary.write_text('#!/bin/bash\nprintf "%s\\n" "$LD_LIBRARY_PATH" "$1"\n')
            binary.chmod(0o755)
            launcher = root / "sweetpad-macros"
            launcher.write_bytes((REPO / "sweetpad-lib/ci/linux-macro-launcher.sh").read_bytes())
            launcher.chmod(0o755)
            result = subprocess.check_output([str(launcher), "plugin-argument"],
                                             env={"PATH": "/usr/bin:/bin"}, text=True).splitlines()
            self.assertEqual(Path(result[0]).resolve(), (toolchain / "lib/swift/linux").resolve())
            self.assertEqual(result[1], "plugin-argument")

    def test_tag_version_must_match_and_dry_runs_do_not_publish(self):
        self.assertEqual(version.resolve("1.2.3", "refs/tags/cli-v1.2.3"), "1.2.3")
        self.assertEqual(version.resolve("1.2.3", "refs/heads/main"), "1.2.3-dryrun")
        with self.assertRaises(ValueError):
            version.resolve("1.2.3", "refs/tags/cli-v1.2.4")

    def test_wrong_platform_binaries_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "sweetpad"
            for data in [b"\xcf\xfa\xed\xfe" + b"\0" * 16, b"\x7fELF\x02\x01" + b"\0" * 12 + b"\x03\x00"]:
                binary.write_bytes(data)
                with self.assertRaises(ValueError):
                    package.validate_elf(binary)

    def test_payload_is_reproducible_and_contains_only_public_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "sweetpad"
            for machine, arch in [(183, "arm64"), (62, "x64")]:
                binary.write_bytes(b"\x7fELF\x02\x01" + b"\0" * 12 + machine.to_bytes(2, "little"))
                with patch.object(package.subprocess, "check_output", return_value="sweetpad 1.2.3\n"):
                    first = package.package(binary, "1.2.3", root / arch / "first")
                    second = package.package(binary, "1.2.3", root / arch / "second")
                self.assertEqual(first["sha256"], second["sha256"])
                self.assertEqual(first["platform"], f"linux-{arch}")
                self.assertTrue(first["archive"].endswith(f"linux-{arch}.tar.gz"))
                with tarfile.open(first["archive"]) as archive:
                    names = set(archive.getnames())
                    self.assertIn("install.sh", names)
                    self.assertIn("setup/sweetpad-cli/macros/Package.resolved", names)
                    self.assertIn("setup/sweetpad-cli/macros/Sources/FoundationMacros/LICENSE.md", names)
                    self.assertIn("examples/LinuxApp/LinuxApp.xcodeproj/project.pbxproj", names)
                    self.assertTrue(archive.getmember("sweetpad-dap").issym())
                    self.assertEqual(archive.getmember("sweetpad-dap").linkname, "sweetpad")
                    self.assertFalse(any(n.endswith((".p12", ".cer", ".mobileprovision", ".omo")) or ".sdk/" in n for n in names))


if __name__ == "__main__":
    unittest.main()
