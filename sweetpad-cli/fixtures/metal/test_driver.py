#!/usr/bin/env python3
"""Linux driver regressions. Set SWEETPAD_METAL_TOOLS to exercise the Rust multicall binary."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from verify import DRIVER, HERE, exports


class DriverTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.work = Path(tmp.name)

    def invoke(self, tool, *args, success=True):
        tools = os.environ.get("SWEETPAD_METAL_TOOLS")
        command = [str(Path(tools).resolve() / tool)] if tools else [sys.executable, str(DRIVER), tool]
        result = subprocess.run(
            [*command, *map(str, args)], cwd=self.work, text=True, capture_output=True
        )
        self.assertEqual(result.returncode == 0, success, result.stderr)
        return result

    def compile(self, source, *opts, target="air64-apple-macosx27.0", out="test.air", success=True):
        return self.invoke("metal", "-c", "-target", target, *opts, source, "-o", out, success=success)

    def test_swift_build_flags_dependencies_and_link_target(self):
        header = self.work / "value.h"
        header.write_text("#define STEP 3u\n")
        (self.work / "shader.metal").write_text(
            '#include <metal_stdlib>\n#include "value.h"\n'
            "typedef struct { uint value; } Word;\n"
            "kernel void update(device Word *a [[buffer(2)]], uint i [[thread_position_in_grid]]) "
            "{ a[i].value += STEP; }\n"
        )
        target = "air64-apple-ios15.6-simulator"
        flags = (
            "-MMD",
            "-MT",
            "dependencies",
            "-MF",
            "shader.dat",
            "-serialize-diagnostics",
            "shader.dia",
            "-fmetal-math-mode=fast",
            "-fmetal-math-fp32-functions=fast",
            "-frecord-sources",
            "-gline-tables-only",
        )
        self.compile("shader.metal", *flags, target=target)
        self.assertTrue((self.work / "shader.dia").is_file())
        deps = (self.work / "shader.dat").read_text()
        self.assertIn("value.h", deps)
        self.assertIn("shader.metal", deps)
        self.assertNotIn("sweetpad-metal-", deps)
        self.invoke("metal", "-target", target, "-frecord-sources", "test.air", "-o", "default.metallib")
        self.assertEqual(
            exports(self.work / "default.metallib"), {"update": {"type": 2, "version": [2, 4, 2, 4]}}
        )
        original = (self.work / "test.air").read_bytes()
        self.compile("shader.metal", *flags, target=target)
        self.assertEqual((self.work / "test.air").read_bytes(), original)
        header.write_text("#define STEP 17u\n")
        self.compile("shader.metal", *flags, target=target)
        self.assertNotEqual((self.work / "test.air").read_bytes(), original)
        result = self.invoke(
            "metal", "-target", "air64-apple-ios15.6", "test.air", "-o", "wrong", success=False
        )
        self.assertIn("target", result.stderr)
        self.assertFalse((self.work / "wrong").exists())

    def test_multi_entry_multiple_modules_and_duplicate_exports(self):
        self.compile(HERE / "Kinds.metal", "-DVERTEX", "-DFRAGMENT", out="render.air")
        self.compile(HERE / "Kinds.metal", "-DCOMPUTE", out="compute.air")
        self.invoke("metallib", "compute.air", "render.air", "-o", "joined.metallib")
        lib = self.work / "joined.metallib"
        self.assertEqual(sorted(exports(lib)), ["increment", "origin", "shade"])
        saved = lib.read_bytes()
        repeated = self.invoke("metallib", "compute.air", "compute.air", "-o", lib, success=False)
        self.assertIn("duplicate Metal function", repeated.stderr)
        self.assertEqual(lib.read_bytes(), saved)

    def test_unsupported_inputs_fail_without_replacing_output(self):
        out = self.work / "saved.air"
        out.write_bytes(b"previous successful build")
        shader = self.work / "invalid.metal"
        for body, diagnostic in (
            ("fragment uint f(uint x [[thread_position_in_grid]]) { return x; }", "fragment input"),
            ("kernel void f(uint x [[nonexistent]]) {}", "kernel input"),
            ("kernel void f(device uint *p [[buffer(0)]]) { *p = missing(); }", "missing"),
        ):
            with self.subTest(body=body):
                shader.write_text("#include <metal_stdlib>\n" + body)
                self.assertIn(diagnostic, self.compile(shader, out=out, success=False).stderr)
                self.assertEqual(out.read_bytes(), b"previous successful build")

    def test_language_feature_availability(self):
        for kind in ("MESH", "OBJECT"):
            result = self.compile(
                HERE / "Kinds.metal", "-std=macos-metal2.4", "-D" + kind, success=False
            )
            self.assertIn("require MSL 3.0", result.stderr)
        result = self.compile(
            HERE / "Language.metal", "-std=metal4.0", "-DREQUIRE_PLACEMENT", success=False
        )
        self.assertIn("placement new requires MSL 4.1", result.stderr)
        self.compile(HERE / "Language.metal", "-std=metal4.1", "-DREQUIRE_PLACEMENT")


if __name__ == "__main__":
    unittest.main()
