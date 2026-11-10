"""Release packages require release-stamped binaries; dry runs can use WIP."""
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("package_linux", Path(__file__).with_name("package-linux.py"))
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


class ReleaseVersionTests(unittest.TestCase):
    def test_tag_packages_reject_development_binaries(self):
        self.assertFalse(package.valid_binary_version("sweetpad 0.1.18-dev+abcdef", "0.1.18"))
        self.assertTrue(package.valid_binary_version("sweetpad 0.1.18", "0.1.18"))

    def test_dry_run_keeps_development_validation_available(self):
        self.assertTrue(package.valid_binary_version("sweetpad 0.1.18-dev+abcdef", "0.1.18-dryrun"))
        self.assertFalse(package.valid_binary_version("sweetpad 0.1.17-dev+abcdef", "0.1.18-dryrun"))


if __name__ == "__main__":
    unittest.main()
