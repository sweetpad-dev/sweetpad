#!/usr/bin/env python3
"""Install and exercise an ARM64 tarball with an empty HOME and no Apple SDK."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile


def check(archive, out):
    if platform.system() != "Linux" or platform.machine() != "aarch64":
        raise ValueError("run this check in Ubuntu 24.04 ARM64")
    out = out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    expected = archive.with_suffix(archive.suffix + ".sha256").read_text().split()
    if len(expected) != 2 or expected[1] != archive.name or hashlib.sha256(archive.read_bytes()).hexdigest() != expected[0]:
        raise ValueError("archive checksum does not match its sidecar")
    extracted, home = out / "unpacked", out / "home"
    extracted.mkdir()
    home.mkdir()
    with tarfile.open(archive) as source:
        source.extractall(extracted, filter="data")
    manifest = json.loads((extracted / "manifest.json").read_text())
    assert manifest["platform"] == "linux-arm64" and manifest["appleSdkIncluded"] is False
    for name, digest in manifest["files"].items():
        assert hashlib.sha256((extracted / name).read_bytes()).hexdigest() == digest, name
    assert (extracted / "sweetpad-dap").is_symlink() and os.readlink(extracted / "sweetpad-dap") == "sweetpad"
    env = {"HOME": str(home), "PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "NO_COLOR": "1",
           "XDG_CONFIG_HOME": str(home / ".config"), "XDG_CACHE_HOME": str(home / ".cache"),
           "XDG_STATE_HOME": str(home / ".local/state"), "SWEETPAD_NONINTERACTIVE": "1"}

    def invoke(name, args, success=True):
        result = subprocess.run(list(map(str, args)), cwd=out, env=env, capture_output=True, text=True, timeout=60)
        (out / (name + ".stdout")).write_text(result.stdout)
        (out / (name + ".stderr")).write_text(result.stderr)
        assert (result.returncode == 0) == success, (name, result.stdout, result.stderr)
        return result

    prefix = home / ".local"
    invoke("install", ["/bin/bash", extracted / "install.sh", "--prefix", prefix])
    resources = prefix / "share/sweetpad"
    for directory in ("setup", "licenses", "examples"):
        (resources / directory / "obsolete-file").write_text("from an older installation")
    (resources / "unrelated-file").write_text("preserve")
    invoke("reinstall", ["/bin/bash", extracted / "install.sh", "--prefix", prefix])
    assert not list(resources.rglob("obsolete-file"))
    assert (resources / "unrelated-file").read_text() == "preserve"
    binary = prefix / "bin/sweetpad"
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == manifest["files"]["sweetpad"]
    assert invoke("version", [binary, "--version"]).stdout.strip() == manifest["binaryVersion"]
    assert "sweetpad dap" in invoke("dap-alias", [prefix / "bin/sweetpad-dap", "--help"]).stdout
    invoke("help", [binary, "--help"])
    invoke("sdk-export-help", [binary, "sdk", "export", "--help"])
    invoke("sdk-import-help", [binary, "sdk", "import", "--help"])
    invoke("setup-help", [binary, "setup", "--help"])
    invalid_sdk=out / "invalid-sdk.tar.gz"
    invalid_sdk.write_bytes(b"not an SDK archive")
    invalid_sdk.with_name(invalid_sdk.name + ".sha256").write_text(f"{hashlib.sha256(invalid_sdk.read_bytes()).hexdigest()}  {invalid_sdk.name}\n")
    raw_setup=invoke("raw-setup-source-discovery", [extracted / "sweetpad", "setup", "--sdk", invalid_sdk, "--json"], success=False)
    raw_error=json.loads(raw_setup.stderr)["error"]
    assert "gzip" in raw_error["message"] and raw_error["diagnostics"][0]["stage"] == "bootstrap"
    doctor = json.loads(invoke("linux-doctor-missing-tools", [binary, "doctor", "--target", "ios", "--json"], success=False).stdout)
    assert doctor["ok"] and doctor["data"]["summary"]["problems"] > 0
    assert not any(c["name"] in ("Xcode", "xcodebuild", "Simulator runtimes") for c in doctor["data"]["checks"])
    value = json.loads(invoke("device-guide", [binary, "help", "linux-devices", "--json"]).stdout)
    assert value["ok"]
    project = prefix / "share/sweetpad/examples/LinuxApp/LinuxApp.xcodeproj"
    value = json.loads(invoke("schemes", [binary, "scheme", "list", "--project", project, "--json"]).stdout)
    assert value["ok"] and "LinuxApp" in json.dumps(value["data"])
    result = invoke("missing-toolchain", [binary, "build", "--project", project, "--scheme", "LinuxApp",
                    "--destination", "generic/platform=iOS Simulator", "--json"], success=False)
    error = json.loads(result.stderr)
    assert error["ok"] is False and any(s in error["error"]["message"].lower() for s in ["sdk", "swift", "swbuild"])
    setup = prefix / "share/sweetpad/setup/sweetpad-lib/ci/linux-setup.sh"
    invoke("setup-syntax", ["/bin/bash", "-n", setup])
    assert (setup.parents[2] / "sweetpad-cli/macros/Package.resolved").is_file()
    assert shutil.which("swift", path=env["PATH"]) is None
    report = {"passed": True, "archiveSha256": expected[0], "binarySha256": manifest["files"]["sweetpad"],
              "binaryVersion": manifest["binaryVersion"], "emptyHome": True, "developerEnvironmentInherited": False,
              "sdkAvailable": False, "sourceCheckoutRequired": False, "installedPrefix": str(prefix),
              "checks": ["checksum", "manifest hashes", "install", "version", "DAP alias", "help", "device guide",
                         "project schemes", "missing-toolchain diagnostic", "bundled setup syntax"]}
    (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    check(args.archive.resolve(), args.out)
