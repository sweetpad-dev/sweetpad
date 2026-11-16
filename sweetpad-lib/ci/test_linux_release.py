import importlib.util
import json
from pathlib import Path
import subprocess
import struct
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


def elf(path, machine, needed=(), runpath="$ORIGIN"):
    """A 64-bit ELF file with one load segment and a dynamic section."""
    strings = bytearray(b"\0")
    def name(text):
        strings.extend(text.encode() + b"\0")
        return len(strings) - len(text) - 1
    entries = [(1, name(library)) for library in needed]
    if runpath is not None:
        entries.append((29, name(runpath)))
    dynamic_at = 64 + 2 * 56
    strtab_at = dynamic_at + 16 * (len(entries) + 2)
    entries += [(5, strtab_at), (0, 0)]
    data = bytearray(strtab_at + len(strings))
    data[:6] = b"\x7fELF\x02\x01"
    data[18:20] = machine.to_bytes(2, "little")
    struct.pack_into("<Q", data, 32, 64)
    struct.pack_into("<HH", data, 54, 56, 2)
    struct.pack_into("<IIQQQQQQ", data, 64, 1, 5, 0, 0, 0, len(data), len(data), 8)
    struct.pack_into("<IIQQQQQQ", data, 120, 2, 6, dynamic_at, dynamic_at, 0, 16 * len(entries), 16 * len(entries), 8)
    for index, (tag, value) in enumerate(entries):
        struct.pack_into("<qQ", data, dynamic_at + 16 * index, tag, value)
    data[strtab_at:] = strings
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def helpers(root, machine):
    """The tree build-swift-helpers.sh stages, with stand-in binaries."""
    libexec = root / "libexec/sweetpad"
    for name in package.HELPERS:
        elf(libexec / name, machine, needed=["libswiftCore.so", "libc.so.6"])
    elf(libexec / "libswiftCore.so", machine, needed=["libstdc++.so.6"])
    (libexec / "SwiftBuild_SWBCore.resources").mkdir()
    (libexec / "SwiftBuild_SWBCore.resources/Core.xcspec").write_text("(\n)\n")
    pins = json.loads((REPO / "sweetpad-lib/ci/linux-toolchain.json").read_text())
    (libexec / "sweetpad-swift-build.json").write_text(
        '{"swiftBuild":"%s","patches":"0123","swift":"%s"}\n' % (pins["swiftBuild"], pins["swift"]))
    (root / "licenses/swift-build").mkdir(parents=True)
    (root / "licenses/swift-build/LICENSE.txt").write_text("Apache License\n")
    return root


def linker(root, machine):
    """The tree build-darwin-linker.sh stages, with a stand-in binary."""
    elf(root / package.LINKER, machine, needed=["libstdc++.so.6", "libm.so.6", "libgcc_s.so.1", "libc.so.6"], runpath=None)
    pins = json.loads((REPO / "sweetpad-lib/ci/linux-toolchain.json").read_text())
    (root / "libexec/sweetpad/sweetpad-llvm.json").write_text(
        '{"llvm": "%s", "sourceSha256": "%s"}\n' % (pins["llvm"], pins["llvmSourceSha256"]))
    (root / "licenses/llvm").mkdir(parents=True)
    (root / "licenses/llvm/LICENSE.TXT").write_text("Apache License with LLVM Exceptions\n")
    return root


class LinuxReleaseContracts(unittest.TestCase):
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
                data = bytearray(120)
                data[:6] = b"\x7fELF\x02\x01"
                data[18:20] = machine.to_bytes(2, "little")
                struct.pack_into("<Q", data, 32, 64)
                struct.pack_into("<HH", data, 54, 56, 1)
                struct.pack_into("<I", data, 64, 1)
                binary.write_bytes(data)
                tree = helpers(root / arch / "helpers", machine)
                tools = linker(root / arch / "linker", machine)
                with patch.object(package.subprocess, "check_output", return_value="sweetpad 1.2.3\n"):
                    first = package.package(binary, tree, tools, "1.2.3", root / arch / "first")
                    second = package.package(binary, tree, tools, "1.2.3", root / arch / "second")
                self.assertEqual(first["sha256"], second["sha256"])
                self.assertEqual(first["platform"], f"linux-{arch}")
                self.assertTrue(first["archive"].endswith(f"linux-{arch}.tar.gz"))
                with tarfile.open(first["archive"]) as archive:
                    names = set(archive.getnames())
                    self.assertNotIn("install.sh", names)
                    self.assertFalse(any(n.startswith(("setup/", "examples/")) for n in names))
                    self.assertIn("licenses/macros/FoundationMacros/LICENSE.md", names)
                    self.assertIn("licenses/archive-codecs/lzfse/LICENSE", names)
                    self.assertIn("licenses/archive-codecs/lzraven/LICENSE", names)
                    self.assertEqual(first["linkage"], "static")
                    self.assertTrue(first["embeddedSetup"])
                    self.assertTrue(archive.getmember("sweetpad-dap").issym())
                    for helper in package.HELPERS:
                        self.assertEqual(archive.getmember(f"libexec/sweetpad/{helper}").mode, 0o755)
                    self.assertEqual(archive.getmember("libexec/sweetpad/SwiftBuild_SWBCore.resources/Core.xcspec").mode, 0o644)
                    self.assertEqual(archive.getmember("libexec/sweetpad/libswiftCore.so").mode, 0o644)
                    self.assertIn("libexec/sweetpad/sweetpad-swift-build.json", names)
                    self.assertIn("licenses/swift-build/LICENSE.txt", names)
                    self.assertEqual(first["swiftHelpers"]["patches"], "0123")
                    self.assertEqual(archive.getmember(package.LINKER).mode, 0o755)
                    self.assertIn("licenses/llvm/LICENSE.TXT", names)
                    self.assertEqual(first["darwinLinker"]["llvm"], json.loads(
                        (REPO / "sweetpad-lib/ci/linux-toolchain.json").read_text())["llvm"])
                    self.assertEqual(archive.getmember("sweetpad-dap").linkname, "sweetpad")
                    self.assertFalse(any(n.endswith((".p12", ".cer", ".mobileprovision", ".omo")) or ".sdk/" in n for n in names))

    def test_helpers_must_match_the_cli_architecture_and_be_complete(self):
        with tempfile.TemporaryDirectory() as directory:
            tree = helpers(Path(directory), 62)
            self.assertIn("libexec/sweetpad/swbuild", package.helper_files(tree, "x64"))
            with self.assertRaisesRegex(ValueError, "linux-arm64"):
                package.helper_files(tree, "arm64")
            (tree / "libexec/sweetpad/sweetpad-macros").unlink()
            with self.assertRaisesRegex(ValueError, "sweetpad-macros"):
                package.helper_files(tree, "x64")

    def test_helpers_load_only_bundled_and_host_libraries_through_origin(self):
        with tempfile.TemporaryDirectory() as directory:
            tree = helpers(Path(directory), 183)
            libexec = tree / "libexec/sweetpad"
            self.assertEqual(package.elf_dynamic(libexec / "swbuild"), (["libswiftCore.so", "libc.so.6"], "$ORIGIN"))
            package.check_helper_linkage(libexec)
            elf(libexec / "libBlocksRuntime.so", 183, needed=["libc.so.6"], runpath=None)
            package.check_helper_linkage(libexec)
            elf(libexec / "libBlocksRuntime.so", 183, needed=["libswiftCore.so"], runpath=None)
            with self.assertRaisesRegex(ValueError, "libBlocksRuntime.so must have a RUNPATH"):
                package.check_helper_linkage(libexec)
            elf(libexec / "libBlocksRuntime.so", 183, needed=["libc.so.6"], runpath=None)
            elf(libexec / "swbuild", 183, needed=["libswiftCore.so"], runpath="/home/ci/swift/usr/lib/swift/linux:$ORIGIN")
            with self.assertRaisesRegex(ValueError, "RUNPATH"):
                package.check_helper_linkage(libexec)
            elf(libexec / "swbuild", 183, needed=["libFoundation.so"])
            with self.assertRaisesRegex(ValueError, "libFoundation.so"):
                package.check_helper_linkage(libexec)

    def test_helpers_built_on_a_newer_host_than_debian_12_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            tree = helpers(Path(directory), 183)
            core = tree / "libexec/sweetpad/libswiftCore.so"
            core.write_bytes(core.read_bytes() + b"\0GLIBC_2.17\0GLIBC_2.36\0GLIBCXX_3.4.30\0")
            package.helper_files(tree, "arm64")
            core.write_bytes(core.read_bytes() + b"GLIBC_2.38\0")
            with self.assertRaisesRegex(ValueError, "GLIBC_2.38"):
                package.helper_files(tree, "arm64")
            elf(core, 183, needed=["libstdc++.so.6"])
            core.write_bytes(core.read_bytes() + b"\0GLIBCXX_3.4.32\0")
            with self.assertRaisesRegex(ValueError, "GLIBCXX_3.4.32"):
                package.helper_files(tree, "arm64")

    def test_helpers_must_come_from_the_pinned_swift(self):
        with tempfile.TemporaryDirectory() as directory:
            tree = helpers(Path(directory), 183)
            record = tree / "libexec/sweetpad/sweetpad-swift-build.json"
            record.write_text(record.read_text().replace('"swift":"', '"swift":"0.'))
            with self.assertRaisesRegex(ValueError, "swift '0."):
                package.helper_files(tree, "arm64")

    def test_the_linker_is_the_pinned_llvm_for_the_cli_architecture(self):
        with tempfile.TemporaryDirectory() as directory:
            tree = linker(Path(directory), 183)
            self.assertIn(package.LINKER, package.linker_files(tree, "arm64"))
            with self.assertRaisesRegex(ValueError, "linux-x64"):
                package.linker_files(tree, "x64")
            record = tree / "libexec/sweetpad/sweetpad-llvm.json"
            record.write_text(record.read_text().replace('"llvm": "', '"llvm": "0.'))
            with self.assertRaisesRegex(ValueError, "LLVM '0."):
                package.linker_files(tree, "arm64")

    def test_the_linker_loads_only_host_libraries_without_a_runpath(self):
        with tempfile.TemporaryDirectory() as directory:
            tree = linker(Path(directory), 183)
            binary = tree / package.LINKER
            elf(binary, 183, needed=["libc.so.6"], runpath="$ORIGIN/../lib")
            with self.assertRaisesRegex(ValueError, "no RUNPATH"):
                package.linker_files(tree, "arm64")
            elf(binary, 183, needed=["libLLVM.so.23.1", "libc.so.6"], runpath=None)
            with self.assertRaisesRegex(ValueError, "libLLVM.so.23.1"):
                package.linker_files(tree, "arm64")
            elf(binary, 183, needed=["libc.so.6"], runpath=None)
            binary.write_bytes(binary.read_bytes() + b"\0GLIBC_2.38\0")
            with self.assertRaisesRegex(ValueError, "GLIBC_2.38"):
                package.linker_files(tree, "arm64")

    def test_the_linker_tree_holds_only_the_linker_and_its_record(self):
        with tempfile.TemporaryDirectory() as directory:
            tree = linker(Path(directory), 183)
            (tree / "libexec/sweetpad/ld64.lld").symlink_to("llvm")
            with self.assertRaisesRegex(ValueError, "regular files"):
                package.linker_files(tree, "arm64")
            (tree / "libexec/sweetpad/ld64.lld").unlink()
            elf(tree / "libexec/sweetpad/dsymutil", 183, needed=["libc.so.6"], runpath=None)
            with self.assertRaisesRegex(ValueError, "unexpected files"):
                package.linker_files(tree, "arm64")


if __name__ == "__main__":
    unittest.main()
