#!/usr/bin/env python3
"""Build and run the IB feature checker on an explicitly selected simulator.

Run on a Mac. --linux-vm builds in OrbStack, then installs that Linux-built
product on the same simulator. Both languages must report every check true.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shlex
import subprocess

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]


def run(command, log, timeout=120):
    result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    log.with_suffix(".stdout").write_text(result.stdout)
    log.with_suffix(".stderr").write_text(result.stderr)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--simulator", required=True)
    parser.add_argument("--linux-vm")
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    subprocess.run(["xcodegen", "generate", "--spec", str(HERE / "project.yml")], check=True)
    binary = args.binary or (Path("/home/hyzyla_home/sweetpad-target/debug/sweetpad")
                             if args.linux_vm else REPO / "target/debug/sweetpad")
    command = [str(binary), "-C", str(HERE), "build", "--scheme", "IBAudit",
               "--destination", "generic/platform=iOS Simulator", "--non-interactive", "-o", "json", "--"]
    if not args.linux_vm:
        command += ["-derivedDataPath", str(out / "derived-data")]
    command += ["CODE_SIGNING_ALLOWED=NO", "ARCHS=arm64", "ONLY_ACTIVE_ARCH=YES"]
    if args.linux_vm:
        shell = ('SWIFT_DIR=$(ls -d "$HOME"/swift/swift-*-RELEASE | sort | tail -1); '
                 'export PATH="$HOME/bin:$HOME/.cargo/bin:$SWIFT_DIR/usr/bin:$PATH"; exec ' + shlex.join(command))
        command = ["orb", "-m", args.linux_vm, "bash", "-lc", shell]
    build = run(command, out / "build", timeout=1200)
    (out / "command.json").write_text(json.dumps(command, indent=2) + "\n")
    if build.returncode:
        print(build.stderr)
        return build.returncode
    envelope = json.loads(build.stdout)
    assert envelope["ok"] and envelope["data"]["built"], envelope
    product = Path(envelope["data"]["productPath"])
    assert product.is_absolute() and product.suffix == ".app" and product.name == "IBAudit.app", product
    if args.linux_vm:
        assert "/DerivedData/" in str(product), product
        staged = out / product.name
        subprocess.run(["orb", "-m", args.linux_vm, "cp", "-a", str(product), str(staged)], check=True)
        product = staged
    assert product.is_dir(), product
    subprocess.run(["xcrun", "simctl", "install", args.simulator, str(product)], check=True)
    rows = []
    for language in ["en", "fr"]:
        command = ["xcrun", "simctl", "launch", "--console-pty", "--terminate-running-process",
                   args.simulator, "dev.sweetpad.ibaudit", "-AppleLanguages", f"({language})"]
        if language == "fr":
            command += ["--expect-french"]
        result = run(command, out / language, timeout=60)
        lines = [line.strip() for line in result.stdout.splitlines() if line.startswith("IBAUDIT ")]
        passed = result.returncode == 0 and "IBAUDIT done" in lines and not any(line.endswith("=false") for line in lines)
        rows.append({"language": language, "passed": passed, "checks": lines, "command": command})
        print(language, passed, len(lines) - 1, flush=True)
    report = {"recorded_at": datetime.now(timezone.utc).isoformat(), "linux_vm": args.linux_vm,
              "simulator": args.simulator, "product": str(product), "runs": rows,
              "passed": all(row["passed"] for row in rows)}
    (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
