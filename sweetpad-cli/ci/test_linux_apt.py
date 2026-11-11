"""APT metadata must authenticate all package indexes and their payloads."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("apt_repository", Path(__file__).with_name("build-apt-repository.py"))
apt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(apt)


class RepositoryTests(unittest.TestCase):
    def test_index_hashes_and_immutable_lookup(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            package, key, out = root / "sweetpad_1.0.0-1_arm64.deb", root / "key.asc", root / "apt"
            package.write_bytes(b"package data")
            key.write_text("public key")
            def run(*args, **kwargs):
                if args[0] == "dpkg-deb":
                    return "Package: sweetpad\nVersion: 1.0.0-1\nArchitecture: arm64\n"
                return b"Package: sweetpad\nArchitecture: arm64\nSHA256: package-digest\n" if args[3] == "arm64" else b""
            with patch.object(apt, "run", side_effect=run), patch.object(apt.subprocess, "run") as sign:
                apt.build([package], out, "fingerprint", key)
            release = (out / "dists/stable/Release").read_text()
            for path in (out / "dists/stable/main").glob("binary-*/Packages*"):
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                self.assertIn(digest, release)
                self.assertEqual((path.parent / "by-hash/SHA256" / digest).read_bytes(), path.read_bytes())
            self.assertEqual(sign.call_count, 2)
            self.assertEqual((out / "pool/main/s/sweetpad" / package.name).read_bytes(), package.read_bytes())
            self.assertEqual((out / "sweetpad-archive-keyring.asc").read_text(), "public key")

    def test_rejects_unrelated_package(self):
        with tempfile.TemporaryDirectory() as root:
            with patch.object(apt, "run", return_value="Package: other\nVersion: 1\nArchitecture: arm64\n"):
                with self.assertRaisesRegex(ValueError, "unexpected package"):
                    apt.build([Path(root) / "other.deb"], Path(root) / "apt", "key", Path(root) / "key.asc")


if __name__ == "__main__":
    unittest.main()
