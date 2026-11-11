"""Release packages require release-stamped binaries; dry runs can use WIP."""
import importlib.util
from pathlib import Path
import unittest
import tempfile

SPEC = importlib.util.spec_from_file_location("package_linux", Path(__file__).with_name("package-linux.py"))
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


class ReleaseVersionTests(unittest.TestCase):
    def test_architecture_comes_from_the_binary_not_the_packaging_host(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "sweetpad"
            for machine, expected in [(183, "arm64"), (62, "x64")]:
                header = bytearray(20)
                header[:6] = b"\x7fELF\x02\x01"
                header[18:20] = machine.to_bytes(2, "little")
                binary.write_bytes(header)
                self.assertEqual(package.validate_elf(binary), expected)
            header[18:20] = (3).to_bytes(2, "little")
            binary.write_bytes(header)
            with self.assertRaises(ValueError):
                package.validate_elf(binary)

    def test_tag_packages_reject_development_binaries(self):
        self.assertFalse(package.valid_binary_version("sweetpad 0.1.18-dev+abcdef", "0.1.18"))
        self.assertTrue(package.valid_binary_version("sweetpad 0.1.18", "0.1.18"))

    def test_dry_run_keeps_development_validation_available(self):
        self.assertTrue(package.valid_binary_version("sweetpad 0.1.18-dev+abcdef", "0.1.18-dryrun"))
        self.assertFalse(package.valid_binary_version("sweetpad 0.1.17-dev+abcdef", "0.1.18-dryrun"))


if __name__ == "__main__":
    unittest.main()
