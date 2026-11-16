import importlib.util
import os
from pathlib import Path
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("sdk_tree", HERE / "sdk-tree.py")
sdk_tree = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sdk_tree)


def bundle(root):
    sdk = root / "Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS27.2.sdk"
    (sdk / "usr/lib").mkdir(parents=True)
    (sdk / "usr/lib/libz.tbd").write_text("--- !tapi-tbd\n")
    (root / "toolset/bin").mkdir(parents=True)
    tool = root / "toolset/bin/ld64.lld"
    tool.write_text("#!/bin/sh\n")
    tool.chmod(0o755)
    os.symlink("iPhoneOS27.2.sdk", sdk.parent / "iPhoneOS.sdk")
    (root / "swift-sdk.json").write_text("{}\n")


class SdkTreeTests(unittest.TestCase):
    def test_a_record_reads_back_as_the_tree_it_was_made_from(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "darwin.artifactbundle"
            bundle(root)
            entries = sdk_tree.walk(root)
            out = Path(tmp) / "tree.tsv.gz"
            sdk_tree.write(entries, out)
            self.assertEqual(sdk_tree.read(out), entries)
            self.assertEqual(list(entries), sorted(entries))
            self.assertEqual(entries["toolset/bin/ld64.lld"][:3], ("f", "755", "10"))
            link = "Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk"
            self.assertEqual(entries[link], ("l", entries[link][1], "16", "iPhoneOS27.2.sdk"))
            self.assertEqual(entries["toolset"][0], "d")

    def test_each_kind_of_difference_is_counted_by_area(self):
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a", Path(tmp) / "b"
            bundle(a)
            bundle(b)
            sdk = "Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS27.2.sdk"
            (b / sdk / "usr/lib/libz.tbd").write_text("--- !tapi-tbd-v5\n")
            (b / "toolset/bin/ld64.lld").chmod(0o700)
            (b / "swift-sdk.json").unlink()
            (b / "toolset-swb.json").write_text("{}\n")
            os.unlink(b / "Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk")
            os.symlink(".", b / "Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk")
            report = sdk_tree.compare(sdk_tree.walk(a), sdk_tree.walk(b))
            self.assertEqual(report["byKind"], {"content": 1, "link": 1, "mode": 1, "onlyInA": 1, "onlyInB": 1})
            self.assertEqual(report["byArea"][sdk]["count"], {"content": 1})
            self.assertEqual(report["byArea"]["toolset"]["count"], {"mode": 1})
            link = "Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk"
            self.assertEqual(report["byArea"][link]["count"], {"link": 1})
            ignored = sdk_tree.compare(sdk_tree.walk(a), sdk_tree.walk(b), ignore=["toolset/*", "*.json"])
            self.assertEqual(ignored["byKind"], {"content": 1, "link": 1})

    def test_a_path_a_record_cant_hold_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a\tb").write_text("")
            with self.assertRaises(ValueError):
                sdk_tree.walk(root)


if __name__ == "__main__":
    unittest.main()
