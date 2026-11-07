#!/usr/bin/env python3
"""Build pinned real projects against a native baseline.

Runs on the Mac with the same source paths mounted in OrbStack. Use --prepare
to clone missing sources. Existing checkouts must already match the pin.
The runner records setup errors separately and preserves both CLI streams.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import time

CI = Path(__file__).resolve().parent
REPO = CI.parent.parent
CORPUS = CI.parent / "corpus"
DESTINATION = "generic/platform=iOS Simulator"


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def prepare(project, clone):
    root = CORPUS / project["slug"]
    if not root.exists() and clone:
        subprocess.run(["git", "clone", "--no-checkout", project["url"], str(root)], check=True)
        subprocess.run(["git", "-C", str(root), "checkout", project["commit"]], check=True)
    actual = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
    if actual != project["commit"]:
        raise RuntimeError(f"{root}: expected {project['commit']}, found {actual}")
    for name in project.get("patches", []):
        patch = CI / "patches" / name
        applied = subprocess.run(["git", "-C", str(root), "apply", "--reverse", "--check", str(patch)], capture_output=True)
        if applied.returncode:
            subprocess.run(["git", "-C", str(root), "apply", "--check", str(patch)], check=True)
            subprocess.run(["git", "-C", str(root), "apply", str(patch)], check=True)
    if project.get("setup") and not (root / project["setupOutput"]).exists():
        subprocess.run(project["setup"], cwd=root, check=True)
    return root


def build(args, project, root, host):
    slug = project["slug"]
    stem = args.out / f"{slug}-{host}"
    container = "workspace" if "workspace" in project else "project"
    common = ["build", f"--{container}", str(root / project[container]),
              "--scheme", project["scheme"], "--configuration", "Debug",
              "--destination", DESTINATION, "--json", "--non-interactive"]
    if args.clean:
        common.append("--clean")
    settings = ["ARCHS=arm64", "ONLY_ACTIVE_ARCH=YES", "CODE_SIGNING_ALLOWED=NO"]
    if host == "xcode":
        command = [str(args.sweetpad), "--developer-dir", str(args.developer_dir), *common,
                   "--", "-derivedDataPath", str(args.out / f"{slug}-xcode"),
                   *settings, *project.get("xcodeFlags", [])]
    else:
        # Positional arguments preserve spaces without inserting them into shell code.
        script = '''export PATH="$HOME/bin:$HOME/swift/swift-${SWEETPAD_SWIFT_VERSION:-6.4.2}-RELEASE/usr/bin:$PATH"
count="$1"; shift
common=("${@:1:count}"); shift "$count"
extra=()
for value in "$@"; do extra+=("${value//@HOME@/$HOME}"); done
exec "$HOME/sweetpad-target/debug/sweetpad" "${common[@]}" -- -derivedDataPath "$HOME/$SWEETPAD_PROJECT_DD" ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO "${extra[@]}"
'''
        command = ["orb", "-m", args.vm, "env", f"SWEETPAD_PROJECT_DD={project['linuxDerivedData']}",
                   "bash", "-lc", script, "--", str(len(common)), *common,
                   *project.get("linuxSettings", [])]
    env = dict(os.environ)
    # The projects treat SwiftLint as optional; a global copy can modify sources.
    env["PATH"] = ":".join(p for p in env.get("PATH", "").split(":") if p != "/opt/homebrew/bin")
    env["PATH"] = str(args.out / "tools/bin") + ":" + str(args.out / "tools/js/node_modules/.bin") + ":" + env["PATH"]
    started = time.time()
    with stem.with_suffix(".json").open("w") as stdout, stem.with_suffix(".stderr").open("w") as stderr:
        try:
            status = subprocess.run(command, cwd=REPO, env=env, stdout=stdout, stderr=stderr,
                                    timeout=args.timeout).returncode
        except subprocess.TimeoutExpired:
            status = -1
    record = {"commit": project["commit"], "host": host, "command": command,
              "exitCode": status, "seconds": round(time.time() - started, 2)}
    save(stem.with_suffix(".command.json"), record)
    return record


def outcome(stem):
    for suffix in (".json", ".stderr"):
        try:
            result = json.loads(stem.with_suffix(suffix).read_text())
            if result.get("ok") and result.get("data", {}).get("built"):
                return {"built": True, "productPath": result["data"].get("productPath")}
            error = result.get("error", {})
            diagnostics = [d["message"] for d in error.get("diagnostics", []) if d["severity"] == "error"]
            return {"built": False, "errors": list(dict.fromkeys(diagnostics)) or [error.get("message", "unknown error")]}
        except (OSError, json.JSONDecodeError):
            continue
    return {"built": False, "errors": ["No CLI result; inspect the command record and stderr."]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true")
    parser.add_argument("--prepare-only", action="store_true", help="Apply source patches without building; combine with --prepare to clone missing sources")
    parser.add_argument("--summarize", action="store_true", help="Read existing results without building")
    parser.add_argument("--clean", action="store_true")
    parser.add_argument("--host", choices=["both", "xcode", "linux"], default="both")
    parser.add_argument("--only", default="")
    parser.add_argument("--vm", default=os.environ.get("SWEETPAD_VM", "sweetpad-linux"))
    parser.add_argument("--sweetpad", type=Path, default=REPO / "target/debug/sweetpad")
    parser.add_argument("--developer-dir", type=Path, default=Path(os.environ.get("DEVELOPER_DIR", "/Applications/Xcode-27.0.0.app/Contents/Developer")))
    parser.add_argument("--out", type=Path, default=CORPUS / "_linux-followups")
    parser.add_argument("--timeout", type=int, default=3600)
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    projects = json.loads((CI / "linux-projects.json").read_text())
    checks_path = args.out / "project-checks.json"
    checks = json.loads(checks_path.read_text()) if checks_path.exists() else {}
    results = []
    for project in projects:
        if args.only not in project["slug"]:
            continue
        result = {"project": project["slug"], "commit": project["commit"]}
        try:
            if args.prepare_only:
                prepare(project, args.prepare)
                print(f"{project['slug']}: sources prepared; install framework dependencies before building", flush=True)
                continue
            if not args.summarize:
                root = prepare(project, args.prepare)
                for host in (["xcode", "linux"] if args.host == "both" else [args.host]):
                    record = build(args, project, root, host)
                    print(f"{project['slug']} {host}: exit {record['exitCode']}", flush=True)
            for host in ["xcode", "linux"]:
                result[host] = outcome(args.out / f"{project['slug']}-{host}")
            check = checks.get(project["slug"], {})
            if check.get("commit") != project["commit"]:
                check = {}
            result["bundleComparison"] = check.get("bundleComparison", "pending" if all(result[h]["built"] for h in ["xcode", "linux"]) else "blocked by build errors")
            result["runtimeComparison"] = check.get("runtime", "not run")
        except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
            result["setupError"] = str(error)
        results.append(result)
    if args.prepare_only:
        for result in results:
            print(f"{result['project']}: {result['setupError']}")
        return int(bool(results))
    save(args.out / "real-projects.json", results)
    return int(any(r.get("setupError") or not all(r[h]["built"] for h in ["xcode", "linux"]) for r in results))


if __name__ == "__main__":
    raise SystemExit(main())
