#!/usr/bin/env python3
"""Run the Swift helpers from a Linux archive on a host with no Swift installed.

Checks that swbuild starts its build service, that the macro plugin answers
the compiler's capability request, and that 'sweetpad doctor' finds both in
libexec/sweetpad beside the unpacked binary. Needs only the host libraries
the Debian package depends on.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tarfile

HERE = Path(__file__).resolve().parent


def load_packaging():
    spec = importlib.util.spec_from_file_location("package_linux", HERE / "package-linux.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def plugin_capability(plugin, env):
    """The compiler's first message to a plugin: an 8-byte length, then JSON."""
    request = json.dumps({"getCapability": {"capability": {"protocolVersion": 7}}}).encode()
    process = subprocess.Popen([str(plugin)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env)
    try:
        process.stdin.write(struct.pack("<Q", len(request)) + request)
        process.stdin.flush()
        size = struct.unpack("<Q", process.stdout.read(8))[0]
        reply = json.loads(process.stdout.read(size))
    finally:
        process.stdin.close()
        process.wait(timeout=30)
    return reply["getCapabilityResult"]["capability"]


def check(archive, out):
    out = out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    unpacked, home = out / "unpacked", out / "home"
    unpacked.mkdir()
    home.mkdir()
    with tarfile.open(archive) as source:
        try:
            source.extractall(unpacked, filter="data")
        except TypeError:  # Python before 3.11.4, as in Debian 12
            source.extractall(unpacked)
    libexec = unpacked / "libexec/sweetpad"
    load_packaging().check_helper_linkage(libexec)
    env = {"HOME": str(home), "PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "NO_COLOR": "1"}
    assert shutil.which("swift", path=env["PATH"]) is None, "run this check on a host without Swift"

    def run(name, args, success=True):
        result = subprocess.run(list(map(str, args)), env=env, capture_output=True, text=True, timeout=120)
        (out / f"{name}.stdout").write_text(result.stdout)
        (out / f"{name}.stderr").write_text(result.stderr)
        assert (result.returncode == 0) == success, (name, result.stdout, result.stderr)
        return result

    alive = run("swbuild-is-alive", [libexec / "swbuild", "isAlive"])
    assert "yes" in alive.stdout, alive.stdout
    capability = plugin_capability(libexec / "sweetpad-macros", env)
    assert capability["protocolVersion"] >= 7, capability
    doctor = json.loads(run("doctor", [unpacked / "sweetpad", "doctor", "--target", "ios", "--json"], success=False).stdout)
    checks = {c["name"]: c for c in doctor["data"]["checks"]}
    for name in ("Swift Build", "Swift Build service", "Swift Build patches", "Macro plugin"):
        assert checks[name]["status"] == "ok", checks[name]
    assert str(libexec) in checks["Swift Build"]["detail"], checks["Swift Build"]
    with open("/etc/os-release") as source:
        host = dict(line.rstrip("\n").split("=", 1) for line in source if "=" in line)
    report = {"passed": True, "host": host.get("PRETTY_NAME", "").strip('"'), "machine": os.uname().machine,
              "swiftInstalled": False, "pluginProtocolVersion": capability["protocolVersion"],
              "checks": ["helper linkage", "swbuild starts its service", "macro plugin handshake",
                         "doctor finds packaged helpers"]}
    (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    check(args.archive.resolve(), args.out)
