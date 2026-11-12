#!/usr/bin/env python3
"""Install and exercise a Linux tarball with an empty HOME and no Apple SDK."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile


def check(archive, out):
    if platform.system() != "Linux" or platform.machine() not in ("aarch64", "x86_64"):
        raise ValueError("run this check in Ubuntu 24.04 ARM64 or x86_64")
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
    assert manifest["platform"] == {"aarch64":"linux-arm64", "x86_64":"linux-x64"}[platform.machine()] and manifest["appleSdkIncluded"] is False
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
    assert manifest["linkage"] == "static" and manifest["embeddedSetup"]
    assert not (extracted / "install.sh").exists()
    assert not (extracted / "setup").exists()
    (prefix / "bin").mkdir(parents=True)
    private = home / ".swiftpm/swift-sdks/private-marker"
    private.parent.mkdir(parents=True)
    private.write_text("preserve SDK state")
    # A single copied executable must work without its archive or a checkout.
    shutil.copyfile(extracted / "sweetpad", prefix / "bin/sweetpad")
    (prefix / "bin/sweetpad").chmod(0o755)
    (prefix / "bin/sweetpad-dap").symlink_to("sweetpad")
    shutil.copyfile(extracted / "sweetpad", prefix / "bin/sweetpad.new")
    (prefix / "bin/sweetpad.new").chmod(0o755)
    os.replace(prefix / "bin/sweetpad.new", prefix / "bin/sweetpad")
    assert private.read_text() == "preserve SDK state"
    binary = prefix / "bin/sweetpad"
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == manifest["files"]["sweetpad"]
    header = binary.read_bytes()[:20]
    assert int.from_bytes(header[18:20], "little") == {"aarch64": 183, "x86_64": 62}[platform.machine()]
    spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package-linux.py"))
    package = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(package)
    package.validate_static(binary)
    assert invoke("version", [binary, "--version"]).stdout.strip() == manifest["binaryVersion"]
    assert "sweetpad dap" in invoke("dap-alias", [prefix / "bin/sweetpad-dap", "--help"]).stdout
    invoke("help", [binary, "--help"])
    invoke("sdk-export-help", [binary, "sdk", "export", "--help"])
    invoke("sdk-import-help", [binary, "sdk", "import", "--help"])
    invoke("sdk-download-help", [binary, "sdk", "download", "--help"])
    invoke("auth-help", [binary, "auth", "--help"])
    invoke("setup-help", [binary, "setup", "--help"])
    # Exercise both embedded Python helpers without credentials or network access.
    missing_session = out / "missing-apple-session.json"
    auth_login = invoke("auth-no-session", [binary, "auth", "login", "--apple-id", "synthetic@example.com",
                                            "--session-file", missing_session, "--json"], success=False)
    assert "non-interactive" in json.loads(auth_login.stderr)["error"]["message"]
    for name, arguments in [("download-no-session", ["sdk", "download"]), ("setup-xcode-no-session", ["setup"])]:
        response = invoke(name, [binary, *arguments, "--xcode", "27.3 beta", "--session-file", missing_session, "--json"], success=False)
        assert "No Apple session file exists" in json.loads(response.stderr)["error"]["message"]
    logged_out = invoke("auth-logout-empty", [binary, "auth", "logout", "--session-file", missing_session, "--json"])
    assert json.loads(logged_out.stdout)["data"]["loggedOut"] is True
    assert not missing_session.exists()
    invalid_sdk=out / "invalid-sdk.tar.gz"
    invalid_sdk.write_bytes(b"not an SDK archive")
    invalid_sdk.with_name(invalid_sdk.name + ".sha256").write_text(f"{hashlib.sha256(invalid_sdk.read_bytes()).hexdigest()}  {invalid_sdk.name}\n")
    raw_setup=invoke("raw-setup-source-discovery", [extracted / "sweetpad", "setup", "--sdk", invalid_sdk, "--json"], success=False)
    raw_error=json.loads(raw_setup.stderr)["error"]
    assert "Expected a Sweetpad SDK export" in raw_error["message"] and raw_error["diagnostics"][0]["stage"] == "bootstrap"
    installed_setup=invoke("standalone-setup-source-discovery", [binary, "setup", "--sdk", invalid_sdk, "--json"], success=False)
    assert "Expected a Sweetpad SDK export" in json.loads(installed_setup.stderr)["error"]["message"]
    doctor = json.loads(invoke("linux-doctor-missing-tools", [binary, "doctor", "--target", "ios", "--json"], success=False).stdout)
    assert doctor["ok"] and doctor["data"]["summary"]["problems"] > 0
    assert not any(c["name"] in ("Xcode", "xcodebuild", "Simulator runtimes") for c in doctor["data"]["checks"])
    value = json.loads(invoke("device-guide", [binary, "help", "linux-devices", "--json"]).stdout)
    assert value["ok"]
    invoke("new-project", [binary, "project", "new", "LinuxApp", "--bundle-id", "dev.sweetpad.distribution", "--no-git", "--json"])
    project = out / "LinuxApp/LinuxApp.xcodeproj"
    value = json.loads(invoke("schemes", [binary, "scheme", "list", "--project", project, "--json"]).stdout)
    assert value["ok"] and "LinuxApp" in json.dumps(value["data"])
    result = invoke("missing-toolchain", [binary, "build", "--project", project, "--scheme", "LinuxApp",
                    "--destination", "generic/platform=iOS Simulator", "--json"], success=False)
    error = json.loads(result.stderr)
    assert error["ok"] is False and any(s in error["error"]["message"].lower() for s in ["sdk", "swift", "swbuild"])
    setup = next((home / ".cache/sweetpad/distribution").glob("*/setup/sweetpad-lib/ci/linux-setup.sh"))
    invoke("setup-syntax", ["/bin/bash", "-n", setup])
    assert (setup.parents[2] / "sweetpad-cli/macros/Package.resolved").is_file()
    assert shutil.which("swift", path=env["PATH"]) is None
    report = {"passed": True, "archiveSha256": expected[0], "binarySha256": manifest["files"]["sweetpad"],
              "binaryVersion": manifest["binaryVersion"], "emptyHome": True, "developerEnvironmentInherited": False,
              "sdkAvailable": False, "sourceCheckoutRequired": False, "installedPrefix": str(prefix),
              "checks": ["checksum", "manifest hashes", "single-binary install", "static linkage", "version", "DAP alias", "help", "device guide",
                         "embedded setup files", "embedded authentication and download helpers", "non-interactive Apple sessions",
                         "project schemes", "missing-toolchain diagnostic", "bundled setup syntax", "SDK state preserved"]}
    (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    check(args.archive.resolve(), args.out)
