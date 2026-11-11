#!/usr/bin/env python3
"""Exercise native package installation, upgrade, and removal in a container."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile


def check(package, upgrade, out):
    if not Path("/.dockerenv").is_file():
        raise RuntimeError("Run package lifecycle checks in a disposable Docker container")
    out.mkdir(parents=True, exist_ok=False)
    sentinel = Path.home() / ".swiftpm/swift-sdks/sweetpad-distribution-marker"
    sentinel.parent.mkdir(parents=True, exist_ok=True)
    sentinel.write_text("private SDK and signing state")
    results = []

    def run(*args):
        value = subprocess.run(list(map(str, args)), capture_output=True, text=True, timeout=180)
        results.append({"command": list(map(str, args)), "code": value.returncode,
                        "stdout": value.stdout, "stderr": value.stderr})
        (out / "commands.json").write_text(json.dumps(results, indent=2) + "\n")
        assert value.returncode == 0, results[-1]
        return value.stdout.strip()

    def installed(path):
        expected = run("dpkg-deb", "-f", path, "Version")
        assert run("dpkg-query", "-W", "-f=${Version}", "sweetpad") == expected
        assert "sweetpad:" in run("dpkg-query", "-S", "/usr/bin/sweetpad")
        assert run("/usr/bin/sweetpad", "--version").startswith("sweetpad ")
        assert "sweetpad dap" in run("/usr/bin/sweetpad-dap", "--help")
        assert sentinel.read_text() == "private SDK and signing state"
        return expected

    # Inspect the package's control archive: installation has no maintainer hooks.
    for path in [package, upgrade]:
        data = subprocess.check_output(["dpkg-deb", "--ctrl-tarfile", str(path)])
        with tarfile.open(fileobj=io.BytesIO(data)) as archive:
            assert not any(Path(n).name in {"preinst", "postinst", "prerm", "postrm"} for n in archive.getnames())
    run("apt-get", "install", "-y", package)
    first = installed(package)
    run("apt-get", "install", "-y", upgrade)
    second = installed(upgrade)
    assert first != second
    run("dpkg", "--compare-versions", second, "gt", first)
    run("apt-get", "remove", "-y", "sweetpad")
    assert not Path("/usr/bin/sweetpad").exists()
    assert not Path("/usr/bin/sweetpad-dap").exists()
    assert sentinel.read_text() == "private SDK and signing state"
    report = {"passed": True, "installedVersion": first, "upgradedVersion": second,
              "packageSha256": hashlib.sha256(package.read_bytes()).hexdigest(),
              "noMaintainerScripts": True, "privateStatePreserved": True}
    (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--upgrade", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    check(args.package.resolve(), args.upgrade.resolve(), args.out.resolve())
