"""Check frozen-lock protection and explicitly relocated Pod checksums."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("prepare", Path(__file__).with_name("linux-project-prepare.py"))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


class LockTests(unittest.TestCase):
    def test_relocated_checksums_do_not_hide_dependency_version_changes(self):
        first = "PODS:\n  - React-Core (0.85.3)\nSPEC CHECKSUMS:\n  hermes-engine: " + "1" * 40 + "\n"
        moved = first.replace("1" * 40, "2" * 40)
        self.assertEqual(prepare.portable_pod_lock(first), prepare.portable_pod_lock(moved))
        self.assertNotEqual(prepare.portable_pod_lock(first), prepare.portable_pod_lock(moved.replace("0.85.3", "0.86.0")))

    def test_generated_lock_drift_is_rejected(self):
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            fixture = root / "ci/framework-locks/example/template/package-lock.json"
            fixture.parent.mkdir(parents=True)
            fixture.write_text('{"version":1}')
            checkout = root / "checkout"
            (checkout / "template").mkdir(parents=True)
            with patch.object(prepare, "CI", root / "ci"):
                prepare.frozen_files("example", checkout)
                (checkout / "template/package-lock.json").write_text('{"version":2}')
                with self.assertRaisesRegex(RuntimeError, "differs from the captured dependency lock"):
                    prepare.frozen_files("example", checkout)


if __name__ == "__main__":
    unittest.main()
