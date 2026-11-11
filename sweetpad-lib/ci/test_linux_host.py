"""Distribution selection must precede privileged package installation."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("linux-host-tools.sh")


class LinuxHostTests(unittest.TestCase):
    def plan(self, release, arch="x86_64", skip=False):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "os-release"
            source.write_text(release)
            return subprocess.run([
                "bash", "-euc",
                'source "$1"; sweetpad_linux_host "$2" "$3"; '
                'printf "%s\\n" "$SWEETPAD_PACKAGE_MANAGER" "$SWEETPAD_SWIFT_PLATFORM" "${SWEETPAD_HOST_PACKAGES[*]-}"',
                "host-test", str(SCRIPT), str(source), arch,
            ], env={**os.environ, "SWEETPAD_SKIP_SYSTEM_PACKAGES": "1" if skip else "0"},
                capture_output=True, text=True)

    def test_debian_uses_native_signed_toolchain_and_default_gcc(self):
        for version in ["12", "13"]:
            for arch, suffix in [("x86_64", ""), ("aarch64", "-aarch64")]:
                result = self.plan(f'ID=debian\nVERSION_ID="{version}"', arch)
                self.assertEqual(result.returncode, 0, result.stderr)
                manager, platform, packages = result.stdout.splitlines()
                self.assertEqual(manager, "apt")
                self.assertEqual(platform, f"debian{version}{suffix}")
                self.assertIn("build-essential", packages.split())
                self.assertIn("llvm", packages.split())
                self.assertNotIn("libgcc-13-dev", packages.split())

    def test_ubuntu_retains_its_toolchain_and_llvm(self):
        result = self.plan('ID=ubuntu\nVERSION_ID="24.04"', "aarch64")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("ubuntu24.04-aarch64", result.stdout)
        self.assertIn("llvm-18", result.stdout)

    def test_arch_uses_pacman_and_libxml_compatibility_package(self):
        result = self.plan('ID=arch\nPRETTY_NAME="Arch Linux"')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.startswith("pacman\nubuntu24.04\n"))
        self.assertIn("libxml2-legacy", result.stdout.split())
        self.assertNotEqual(self.plan("ID=arch", "aarch64").returncode, 0)

    def test_unknown_distribution_needs_explicit_manual_packages(self):
        for release in ["ID=fedora\nVERSION_ID=45", "", "ID=ubuntu\nVERSION_ID=22.04"]:
            result = self.plan(release)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("--skip-system-packages", result.stderr)
            manual = self.plan(release, skip=True)
            self.assertEqual(manual.returncode, 0, manual.stderr)
            self.assertEqual(manual.stdout.splitlines()[:2], ["", "ubuntu24.04"])
            self.assertIn("untested", manual.stderr)

    def test_alpine_and_unsupported_cpu_fail_before_package_installation(self):
        for skip in [False, True]:
            result = self.plan("ID=alpine\nVERSION_ID=3.24", skip=skip)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("container", result.stderr)
        self.assertNotEqual(self.plan("ID=ubuntu\nVERSION_ID=24.04", "riscv64").returncode, 0)


if __name__ == "__main__":
    unittest.main()
