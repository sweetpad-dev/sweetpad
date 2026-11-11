"""Release packages require release-stamped binaries; dry runs can use WIP."""
import importlib.util
from pathlib import Path
import unittest
import tempfile
import struct

SPEC = importlib.util.spec_from_file_location("package_linux", Path(__file__).with_name("package-linux.py"))
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


class ReleaseVersionTests(unittest.TestCase):
    def test_static_contract_rejects_loader_and_shared_library_dependencies(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "sweetpad"
            def image(kind, payload):
                data = bytearray(120 + len(payload))
                data[:6] = b"\x7fELF\x02\x01"
                data[18:20] = (62).to_bytes(2, "little")
                struct.pack_into("<Q", data, 32, 64)
                struct.pack_into("<HH", data, 54, 56, 1)
                struct.pack_into("<IIQQQQQQ", data, 64, kind, 4, 120, 0, 0, len(payload), len(payload), 8)
                data[120:] = payload
                binary.write_bytes(data)
            image(2, struct.pack("<qQ", 0, 0))
            package.validate_static(binary)  # A static PIE may have a dynamic table.
            image(3, b"/lib64/ld-linux-x86-64.so.2\0")
            with self.assertRaisesRegex(ValueError, "interpreter"):
                package.validate_static(binary)
            image(2, struct.pack("<qQqQ", 1, 42, 0, 0))
            with self.assertRaisesRegex(ValueError, "DT_NEEDED"):
                package.validate_static(binary)

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
