"""Apple-tool interoperability, selective hard links, authentication and safe staging."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).parent
FIXTURES = ROOT / "fixtures/xcode-archives"


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


archive = load("linux-xcode-archive")
generator = load("generate_xcode_archive_fixtures")
aea = load("linux-aea")
try:
    import cryptography
    HAS_CRYPTO = True
except ImportError:
    HAS_CRYPTO = False


class ArchiveTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory()
        self.addCleanup(self.work.cleanup)
        self.root = Path(self.work.name)
        self.environment = patch.dict(os.environ, {"XDG_CACHE_HOME": str(self.root / "cache")})
        self.environment.start(); self.addCleanup(self.environment.stop)

    def assert_extract(self, source, **options):
        destination = self.root / source.name
        app = archive.extract(source, destination, **options)
        header = app / generator.HEADER.removeprefix("Xcode.app/")
        self.assertEqual(header.read_bytes(), generator.DATA)
        self.assertTrue((header.parent / "link").is_symlink())
        self.assertEqual((header.parent / "link").read_bytes(), generator.DATA)
        self.assertEqual((header.parent / "empty").read_bytes(), b"")
        self.assertFalse((app / "Contents/Resources").exists())
        self.assertEqual((app / "Contents/version.plist").read_bytes(), generator.VERSION)

    def test_fixture_integrity(self):
        for name, digest in json.loads((FIXTURES / "checksums.json").read_text()).items():
            self.assertEqual(hashlib.sha256((FIXTURES / name).read_bytes()).hexdigest(), digest, name)

    def test_apple_compression_and_filtered_hard_links(self):
        for source in sorted(FIXTURES.glob("*.aar")) + sorted(FIXTURES.glob("*.xip")):
            with self.subTest(source=source.name):
                self.assert_extract(source)

    @unittest.skipUnless(HAS_CRYPTO, "Python cryptography is required for AEA")
    def test_apple_signed_and_encrypted_archives(self):
        for profile in [0, 1, 2]:
            with self.subTest(profile=profile):
                self.assert_extract(FIXTURES / f"profile{profile}.aea")

    @unittest.skipUnless(HAS_CRYPTO, "Python cryptography is required for AEA")
    def test_external_keys_padded_archives_and_murmur_checksums(self):
        key = self.root / "key"; key.write_bytes(bytes(range(32)))
        with self.assertRaisesRegex(ValueError, "public key is not embedded"):
            b"".join(aea.chunks(FIXTURES / "profile0-external.aea"))
        self.assert_extract(FIXTURES / "profile0-external.aea", public_key_file=FIXTURES / "signing-public.pem")
        with self.assertRaisesRegex(ValueError, "key is not embedded"):
            b"".join(aea.chunks(FIXTURES / "profile1-external.aea"))
        self.assert_extract(FIXTURES / "profile1-external.aea", key_file=key)
        key.write_bytes(bytes(32))
        with self.assertRaisesRegex(ValueError, "authentication failed"):
            b"".join(aea.chunks(FIXTURES / "profile1-external.aea", key_file=key))

    @unittest.skipUnless(HAS_CRYPTO, "Python cryptography is required for AEA")
    def test_authentication_rejects_modified_ciphertext_and_padding(self):
        for name in ["profile0.aea", "profile1.aea", "profile2.aea", "profile1-external.aea"]:
            original = (FIXTURES / name).read_bytes()
            key = self.root / "key"; key.write_bytes(bytes(range(32)))
            for offset in [len(original)//2, len(original)-1]:
                data = bytearray(original); data[offset] ^= 0x80
                source = self.root / (name + str(offset)); source.write_bytes(data)
                with self.subTest(name=name,offset=offset), self.assertRaisesRegex(ValueError, "authentication|signature|checksum"):
                    b"".join(aea.chunks(source, key_file=key if "external" in name else None))

    @unittest.skipUnless(HAS_CRYPTO, "Python cryptography is required for AEA")
    def test_hpke_matches_rfc9180(self):
        from cryptography.hazmat.primitives.asymmetric import ec
        vector=json.loads((FIXTURES/"rfc9180-p256.json").read_text())
        private=ec.derive_private_key(int(vector["skRm"],16),ec.SECP256R1())
        message=vector["encryption"]
        actual=aea.APPLE_KEYS.hpke_open(private,bytes.fromhex(vector["enc"]),bytes.fromhex(message["ct"]),
                                      bytes.fromhex(vector["info"]),bytes.fromhex(message["aad"]))
        self.assertEqual(actual,bytes.fromhex(message["pt"]))

    @unittest.skipUnless(HAS_CRYPTO, "Python cryptography is required for AEA")
    def test_apple_fcs_envelope_from_independent_go_sender(self):
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric import ec
        vector=json.loads((FIXTURES/"rfc9180-p256.json").read_text())
        private=ec.derive_private_key(int(vector["skRm"],16),ec.SECP256R1())
        pem=private.private_bytes(serialization.Encoding.PEM,serialization.PrivateFormat.PKCS8,serialization.NoEncryption())
        keys=archive.module("linux-aea").APPLE_KEYS
        keys._cache.clear()
        with patch.object(keys.urllib.request,"build_opener") as build:
            build.return_value.open.return_value.__enter__.return_value.read.return_value=pem
            self.assert_extract(FIXTURES/"profile1-fcs.aea")
            self.assertEqual(build.return_value.open.call_count,1)
        key_file=self.root/"fcs.p8";key_file.write_bytes(pem)
        result=b"".join(aea.chunks(FIXTURES/"profile1-fcs.aea",key_file=key_file))
        self.assertEqual(result,(FIXTURES/"raw.aar").read_bytes())

    def test_archive_metadata_cannot_fetch_keys_from_unapproved_hosts(self):
        for url in ["http://swdist.apple.com/key", "https://swdist.apple.com.evil.example/key",
                    "https://swdist.apple.com@evil.example/key", "https://127.0.0.1/key",
                    "file:///tmp/key", "https://user:password@swdist.apple.com/key",
                    "https://swdist.apple.com:444/key"]:
            with self.subTest(url=url),self.assertRaises(ValueError):
                aea.APPLE_KEYS.validate_url(url)

    def test_unsafe_cpio_names_links_and_parent_conflicts_never_escape_staging(self):
        cases = [
            [("../escape",b"bad",0o100644,1,1)],
            [(generator.HEADER,b"../../../../../../../../escape",0o120777,1,1)],
            [(str(Path(generator.HEADER).parent),b"folder",0o120777,1,1),
             (generator.HEADER,b"bad",0o100644,2,1)],
            [(generator.HEADER,b"a",0o100644,1,1),(generator.HEADER,b"b",0o100644,2,1)],
        ]
        for index, entries in enumerate(cases):
            source=self.root/f"unsafe{index}.xip"; source.write_bytes(generator.xip(generator.cpio(entries)))
            with self.subTest(index=index), self.assertRaises(ValueError):
                archive.extract(source,self.root/f"stage{index}")
            self.assertFalse((self.root/"escape").exists())

    def test_xip_payload_corruption_and_truncation_are_rejected(self):
        original=(FIXTURES/"odc-pbzx.xip").read_bytes()
        bad_toc=bytearray(original);bad_toc[35]^=0x80
        for index,data in enumerate([original[:-1],original[:28],original[:-20]+bytes(20),bytes(bad_toc)]):
            source=self.root/f"bad{index}.xip";source.write_bytes(data)
            with self.subTest(index=index), self.assertRaises(ValueError):
                archive.extract(source,self.root/f"out{index}")

    def test_codec_bounds_reject_untrusted_lengths(self):
        for algorithm in ["e","m","x","z","4"]:
            with self.subTest(algorithm=algorithm), self.assertRaises(ValueError):
                archive.CODECS.decompress(algorithm,b"bad",archive.CODECS.MAX_BLOCK+1)

    def test_existing_sdk_is_preserved_when_native_archive_preparation_fails(self):
        sdk=load("linux-sdk")
        installed=self.root/"sdk";installed.mkdir();(installed/"swift-sdk.json").write_text("{}");(installed/"sentinel").write_text("keep")
        from types import SimpleNamespace
        args=SimpleNamespace(archive=FIXTURES/"odc-pbzx.xip",destination=installed,replace=True,xtool="fixture-xtool")
        with patch.object(sdk.platform,"system",return_value="Linux"), patch.object(sdk.platform,"machine",return_value="aarch64"), patch.object(sdk.subprocess,"check_output",return_value="xtool "+sdk.VERSIONS["xtool"]):
            with self.assertRaisesRegex(ValueError,"Missing required platform"):
                sdk.import_sdk(args)
        self.assertEqual((installed/"sentinel").read_text(),"keep")
        self.assertEqual(list(self.root.glob(".sweetpad-sdk-*")),[])


if __name__ == "__main__":
    unittest.main()
