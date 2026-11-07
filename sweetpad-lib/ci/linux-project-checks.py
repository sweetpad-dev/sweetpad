#!/usr/bin/env python3
"""Compare built bundles and optionally launch both products in an iOS simulator.

Uses existing linux-projects.py results. No builds and no physical-device access.
Screenshots and detailed results stay in the ignored corpus output directory.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import time

CI = Path(__file__).resolve().parent
REPO = CI.parent.parent
spec = importlib.util.spec_from_file_location("bundle_comparison", CI / "bundle_comparison.py")
bundle_comparison = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle_comparison)


def run(command, **kwargs):
    return subprocess.run(command, capture_output=True, text=True, timeout=90, **kwargs)


def product(out, slug, host):
    result = None
    for suffix in ["json", "stderr"]:
        path = out / f"{slug}-{host}.{suffix}"
        if path.exists():
            try:
                result = json.loads(path.read_text())
                break
            except ValueError:
                pass
    if not result or not result.get("ok") or not result.get("data", {}).get("built"):
        message = (result or {}).get("error", {}).get("message", "no successful product")
        raise RuntimeError(f"{host} build: {message}")
    return Path(result["data"]["productPath"])


def copy_product(source, destination, vm):
    destination.mkdir(parents=True, exist_ok=True)
    product = destination / source.name
    with tempfile.TemporaryDirectory(prefix=".sweetpad-copy-", dir=destination) as work:
        staging = Path(work)
        with subprocess.Popen(["orb", "-m", vm, "tar", "-C", str(source.parent), "-cf", "-", source.name], stdout=subprocess.PIPE) as remote:
            subprocess.run(["tar", "-C", str(staging), "-xf", "-"], stdin=remote.stdout, check=True)
            remote.stdout.close()
            if remote.wait() != 0:
                raise RuntimeError("copying the Linux product failed")
        copied = staging / source.name
        if not copied.is_dir():
            raise RuntimeError("copying the Linux product produced no bundle")
        backup = staging / ".previous-product"
        existed = product.exists() or product.is_symlink()
        if existed:
            product.rename(backup)
        try:
            copied.rename(product)
        except OSError:
            if existed:
                backup.rename(product)
            raise
    return product


def plist(path):
    try:
        return plistlib.loads(path.read_bytes())
    except plistlib.InvalidFileException:
        result = run(["plutil", "-convert", "json", "-o", "-", str(path)])
        result.check_returncode()
        return json.loads(result.stdout)


def compare(native, linux, build_roots=None):
    return bundle_comparison.compare(native, linux, plist, build_roots)


def products_root(product):
    return next((p for p in product.parents if p.name == "Products" and p.parent.name == "Build"), None)


def runtime(app, slug, host, args):
    env = dict(os.environ, DEVELOPER_DIR=str(args.developer_dir))
    def sim(*words):
        result = run(["xcrun", "simctl", *words], env=env)
        return {"exitCode": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
    bundle = plist(app / "Info.plist")["CFBundleIdentifier"]
    sim("uninstall", args.simulator, bundle)
    result = {"bundle": bundle, "install": sim("install", args.simulator, str(app))}
    if result["install"]["exitCode"] != 0:
        return result
    result["launch"] = sim("launch", "--terminate-running-process", args.simulator, bundle)
    time.sleep(args.settle)
    jobs = sim("spawn", args.simulator, "launchctl", "list")
    result["running"] = any((parts := line.split()) and parts[0].isdigit() and bundle in parts[-1] for line in jobs["stdout"].splitlines())
    result["screenshot"] = sim("io", args.simulator, "screenshot", str(args.out / f"{slug}-{host}.png"))
    sim("terminate", args.simulator, bundle)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=CI.parent / "corpus/_linux-followups")
    parser.add_argument("--only", default="")
    parser.add_argument("--vm", default="sweetpad-linux")
    parser.add_argument("--simulator", help="UDID of a booted simulator; omit for bundle checks only")
    parser.add_argument("--settle", type=int, default=8)
    parser.add_argument("--developer-dir", type=Path, default=Path("/Applications/Xcode.app/Contents/Developer"))
    args = parser.parse_args()
    args.out = args.out.resolve()
    output = args.out / "project-checks.json"
    records = json.loads(output.read_text()) if output.exists() else {}
    for project in json.loads((CI / "linux-projects.json").read_text()):
        slug = project["slug"]
        if args.only not in slug:
            continue
        result = {"commit": project["commit"]}
        try:
            native = product(args.out, slug, "xcode")
            source = product(args.out, slug, "linux")
            linux = copy_product(source, args.out / "linux-products" / slug, args.vm)
            result["bundleComparison"] = compare(native, linux,
                {k: v for k, v in {"xcode": products_root(native), "linux": products_root(source)}.items() if v})
            if args.simulator:
                result["runtime"] = {host: runtime(app, slug, host, args) for host, app in [("xcode", native), ("linux", linux)]}
        except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
            result["error"] = str(error)
        records[slug] = result
        output.write_text(json.dumps(records, indent=2) + "\n")
        print(slug, "error: " + result["error"] if "error" in result else
              f"{len(result['bundleComparison']['findings'])} differences", flush=True)
    selected = [r for slug, r in records.items() if args.only in slug]
    if any("error" in r or not r["bundleComparison"]["passed"] for r in selected):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
