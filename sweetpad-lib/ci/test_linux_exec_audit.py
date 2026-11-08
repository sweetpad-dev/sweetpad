"""Ensure execution provenance catches hidden Darwin tools and inherited cwd."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("audit", Path(__file__).with_name("linux-exec-audit.py"))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class ExecutionTests(unittest.TestCase):
    def test_macos_tool_is_rejected_even_when_the_bridge_exits_zero(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tool = root / "tool"
            tool.write_bytes(bytes.fromhex("cafebabe") + b"data")
            report = audit.inspect_trace(f'10 execve("{tool}", ["tool"], 0x0) = 0\n', root)
            self.assertFalse(report["passed"])
            self.assertEqual(report["violations"][0]["reason"], "macOS executable attempted")

    def test_forked_relative_exec_uses_cwd_at_fork_not_later_parent_cwd(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "child").mkdir()
            (root / "child/tool").write_bytes(b"\x7fELF")
            trace = ('10 chdir("child") = 0\n'
                     '10 clone(flags=CLONE_VM|CLONE_VFORK|SIGCHLD <unfinished ...>\n'
                     '11 execve("./tool", ["./tool"], 0x0 <unfinished ...>\n'
                     '10 <... clone resumed>) = 11\n'
                     '10 chdir("/tmp") = 0\n'
                     '11 <... execve resumed>) = 0\n')
            report = audit.inspect_trace(trace, root)
            self.assertTrue(report["passed"], report)
            self.assertIn(str((root / "child/tool").resolve()), report["tools"])

    def test_deleted_executable_cannot_silently_pass(self):
        report = audit.inspect_trace('1 execve("/missing/tool", [], 0x0) = 0\n', Path("/"))
        self.assertFalse(report["passed"])

    def test_deleted_tool_passes_only_with_a_live_header_capture(self):
        report = audit.inspect_trace('1 execve("/deleted/tool", [], 0x0) = 0\n', Path("/"),
            {(1,"/deleted/tool"): (Path("/deleted/tool"),b"\x7fELF")})
        self.assertTrue(report["passed"],report)

    def test_missing_path_search_candidates_are_not_execution_failures(self):
        with tempfile.TemporaryDirectory() as directory:
            tool = Path(directory) / "tool"
            tool.write_bytes(b"\x7fELF")
            trace = '1 execve("/missing/tool", [], 0x0) = -1 ENOENT\n' + f'1 execve("{tool}", [], 0x0) = 0\n'
            self.assertTrue(audit.inspect_trace(trace, Path(directory))["passed"])


if __name__ == "__main__":
    unittest.main()
