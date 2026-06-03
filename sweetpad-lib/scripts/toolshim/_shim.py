#!/usr/bin/env python3
"""Toolchain shim: log invocation as JSONL, then exec the real tool.

Invoked through one of the sibling symlinks (swiftc, clang, ld, ...). The shim
determines which tool to wrap from `basename(argv[0])`, appends one JSONL line
to `$SWEETPAD_SHIM_LOG` describing the invocation, then exec's the real tool.

Environment:
  SWEETPAD_SHIM_LOG  Absolute path to a JSONL file. If unset or empty,
                     logging is silently skipped (the shim still execs the
                     real tool — handy for ad-hoc manual invocations).

Finding the real tool:
  1. PATH search with the shim directory removed.
  2. Fallback: `/usr/bin/xcrun --find <tool>` against the active Xcode.

The env-allowlist is duplicated from scripts/common.py — keep them in sync.
We avoid importing common.py here because this script is invoked many times
per build and we want minimal import overhead.
"""

from __future__ import annotations

import fcntl
import json
import os
import shutil
import subprocess
import sys
import time


EXACT: frozenset[str] = frozenset({
    "SDKROOT", "BUILT_PRODUCTS_DIR", "CONFIGURATION",
    "CONFIGURATION_BUILD_DIR", "DERIVED_FILE_DIR", "PROJECT_DIR",
    "PROJECT_NAME", "SRCROOT", "TARGET_NAME", "EFFECTIVE_PLATFORM_NAME",
    "ARCHS", "CURRENT_ARCH", "SWIFT_VERSION",
})
PREFIXES: tuple[str, ...] = ("OTHER_", "GCC_", "SWIFT_", "WARNING_", "CLANG_", "LD_")
SUFFIXES: tuple[str, ...] = ("_SEARCH_PATHS",)


def filtered_env(env: dict[str, str]) -> dict[str, str]:
    out: dict[str, str] = {}
    for k, v in env.items():
        if k in EXACT:
            out[k] = v
        elif any(k.startswith(p) for p in PREFIXES):
            out[k] = v
        elif any(k.endswith(s) for s in SUFFIXES):
            out[k] = v
    return out


def find_real_tool(name: str, my_dir: str) -> str | None:
    """Locate the real tool by stripping `my_dir` from PATH, then via xcrun."""
    my_real = os.path.realpath(my_dir)
    path = os.environ.get("PATH", "")
    kept = [
        p for p in path.split(os.pathsep)
        if p and os.path.realpath(p) != my_real
    ]
    found = shutil.which(name, path=os.pathsep.join(kept))
    if found:
        return found
    try:
        cp = subprocess.run(
            ["/usr/bin/xcrun", "--find", name],
            capture_output=True, text=True, timeout=10,
        )
        if cp.returncode == 0:
            out = cp.stdout.strip()
            if out:
                return out
    except Exception:
        pass
    return None


def append_log(path: str, record: dict) -> None:
    line = json.dumps(record, ensure_ascii=False) + "\n"
    data = line.encode("utf-8")
    fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        os.write(fd, data)
    finally:
        try:
            fcntl.flock(fd, fcntl.LOCK_UN)
        except OSError:
            pass
        os.close(fd)


def main() -> None:
    argv0 = sys.argv[0]
    name = os.path.basename(argv0)
    my_dir = os.path.dirname(os.path.realpath(argv0))
    if not my_dir:
        my_dir = os.path.dirname(os.path.realpath(__file__))

    log_path = os.environ.get("SWEETPAD_SHIM_LOG", "")
    if log_path:
        try:
            record = {
                "tool": name,
                "argv": sys.argv,
                "cwd": os.getcwd(),
                "env": filtered_env(dict(os.environ)),
                "ts": time.time(),
                "pid": os.getpid(),
            }
            append_log(log_path, record)
        except Exception as e:
            sys.stderr.write(f"sweetpad shim: log failed for {name}: {e}\n")

    real = find_real_tool(name, my_dir)
    if not real:
        sys.stderr.write(
            f"sweetpad shim: cannot locate real {name!r} via PATH or xcrun\n"
        )
        sys.exit(127)

    if os.path.realpath(real) == os.path.realpath(argv0):
        sys.stderr.write(f"sweetpad shim: refusing to exec self for {name!r}\n")
        sys.exit(127)

    os.execv(real, [real, *sys.argv[1:]])


if __name__ == "__main__":
    main()
