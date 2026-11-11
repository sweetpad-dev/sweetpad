#!/usr/bin/env python3
"""Validate signed APT installation and updates in a disposable Ubuntu container."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

CI = Path(__file__).resolve().parent


def inside(args):
    if not Path("/.dockerenv").is_file():
        raise RuntimeError("Run APT lifecycle checks in a disposable Docker container")
    args.out.mkdir(parents=True, exist_ok=False)
    commands = []
    def run(*command, success=True, **kwargs):
        value = subprocess.run(list(map(str, command)), capture_output=True, text=True, timeout=180, **kwargs)
        commands.append({"command": list(map(str, command)), "code": value.returncode,
                         "stdout": value.stdout, "stderr": value.stderr})
        (args.out / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        if success:
            assert value.returncode == 0, commands[-1]
        return value
    with tempfile.TemporaryDirectory(prefix="apt-validation-") as directory:
        directory = Path(directory)
        directory.chmod(0o755)
        key_home = directory / "keys"
        key_home.mkdir(mode=0o700)
        os.environ["GNUPGHOME"] = str(key_home)
        run("gpg", "--batch", "--passphrase", "", "--quick-generate-key", "Sweetpad APT test", "ed25519", "sign", "1d")
        listing = run("gpg", "--batch", "--with-colons", "--list-secret-keys").stdout
        fingerprint = next(line.split(":")[9] for line in listing.splitlines() if line.startswith("fpr:"))
        public = directory / "key.asc"
        public.write_text(run("gpg", "--armor", "--export", fingerprint).stdout)
        spec = importlib.util.spec_from_file_location("apt_repository", CI / "build-apt-repository.py")
        builder = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(builder)
        site = directory / "repository"
        builder.build(list(args.package_dir.glob("*.deb")), site, fingerprint, public)
        Path("/etc/apt/sources.list.d/sweetpad-test.sources").write_text(
            f"Types: deb\nURIs: file:{site}\nSuites: stable\nComponents: main\nSigned-By: {public}\n")
        sentinel = Path.home() / ".swiftpm/swift-sdks/sweetpad-distribution-marker"
        sentinel.parent.mkdir(parents=True, exist_ok=True)
        sentinel.write_text("private state")
        run("apt-get", "update", "-o", "APT::Update::Error-Mode=any")
        run("apt-get", "install", "-y", "sweetpad")
        first = run("dpkg-query", "-W", "-f=${Version}", "sweetpad").stdout
        run("sweetpad", "--version")
        updated = directory / "updated"
        builder.build([*args.package_dir.glob("*.deb"), *args.upgrade_dir.glob("*.deb")], updated, fingerprint, public)
        shutil.rmtree(site)
        updated.rename(site)
        run("apt-get", "update", "-o", "APT::Update::Error-Mode=any")
        run("apt-get", "install", "-y", "sweetpad")
        second = run("dpkg-query", "-W", "-f=${Version}", "sweetpad").stdout
        run("dpkg", "--compare-versions", second, "gt", first)
        signed = site / "dists/stable/InRelease"
        signed.write_text(signed.read_text().replace("Origin: Sweetpad", "Origin: Modified"))
        failure = run("apt-get", "update", "-o", "APT::Update::Error-Mode=any", success=False)
        assert failure.returncode != 0 and "signature" in (failure.stdout + failure.stderr).lower(), commands[-1]
        assert sentinel.read_text() == "private state"
        report = {"passed": True, "installedVersion": first, "upgradedVersion": second,
                  "alteredMetadataRejected": True, "privateStatePreserved": True}
        (args.out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package-dir", type=Path, required=True)
    parser.add_argument("--upgrade-dir", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.inside:
        inside(args)
    else:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["docker", "run", "--rm", "-v", f"{CI}:/validation/sweetpad-cli/ci:ro",
                        "-v", f"{args.package_dir.resolve()}:/packages:ro",
                        "-v", f"{args.upgrade_dir.resolve()}:/upgrade:ro",
                        "-v", f"{args.out.parent.resolve()}:/reports", "ubuntu:24.04", "bash", "-euc",
                        "apt-get update -qq\nDEBIAN_FRONTEND=noninteractive apt-get install -yqq python3 ca-certificates gnupg dpkg-dev\n"
                        f"python3 /validation/sweetpad-cli/ci/check-linux-apt.py --inside --package-dir /packages --upgrade-dir /upgrade --out /reports/{args.out.name}"], check=True)
