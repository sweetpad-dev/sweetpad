#!/usr/bin/env python3
"""Build every scheme of every corpus project on Linux, and with Xcode where
Linux fails, to tell a Linux gap from a project that doesn't build at all.

    ci/linux-sweep.py [--out DIR] [--only SUBSTRING] [--force] [--xcode] [--jobs N]

Runs on the Mac. Each scheme builds its Debug configuration, whatever its
case, in the OrbStack machine
(SWEETPAD_VM, default sweetpad-linux) with the sweetpad that
ci/linux-build.sh builds there, for the first platform the scheme supports
of iOS Simulator, macOS, tvOS, watchOS and visionOS Simulator, and a
package's scheme for iOS Simulator. The corpus
projects come from fixtures/<slug>/xcode-27.0.0/meta.json, and the local
ones (corpus/_*/) from their own schemes.

Xcode 27 refuses a deployment target below iOS 15, macOS 12, tvOS 15 or
watchOS 9, which many corpus pins set. A scheme that fails on that builds
again on both hosts with every deployment target set to iOS 18's
generation, and its result says so. The setting covers every target, so
the minimums themselves would lower the ones a project sets higher.

Each result is a JSON file under OUT (default corpus/_linux-sweep), one per
scheme, and a scheme that already has one is skipped unless --force.
A scheme with no running build targets is retried with `test build`; the
record and summary name the selected action. With
--xcode, each scheme that failed on Linux builds again on the Mac, with
xcodebuild and without signing, as on Linux, and its result goes beside the
Linux one. summary.json and
summary.md sum them up.

--jobs builds that many containers at once (default 1). A container's
schemes build one after another, since they share its DerivedData; the
Mac and the VM share the Mac's cores, so the gain is on small projects,
where planning and package resolution outweigh compiling.
"""

import argparse
import concurrent.futures
import json
import os
import re
import subprocess
import sys
import threading
import time

CI = os.path.dirname(os.path.abspath(__file__))
LIB = os.path.dirname(CI)
REPO = os.path.dirname(LIB)
CORPUS = os.path.join(LIB, "corpus")
VM = os.environ.get("SWEETPAD_VM", "sweetpad-linux")
MAC_SWEETPAD = os.path.join(REPO, "target", "debug", "sweetpad")
TIMEOUT = 60 * 60
with open(os.path.join(CI, "linux-corpus-patches.json")) as patch_manifest:
    CORPUS_PATCHES = json.load(patch_manifest)


def fixture_policy(path, scheme, key):
    """Explicit scheme changes made by the pinned compatibility patches."""
    for entry in CORPUS_PATCHES:
        for policy in entry.get(key, []):
            container = os.path.join(CORPUS, entry["checkout"], policy["container"])
            if path == container and scheme == policy["scheme"]:
                if key == "retiredSchemes" and os.path.exists(
                    os.path.join(CORPUS, entry["checkout"], policy["schemeFile"])
                ):
                    continue
                return policy
    return None

# The destination for each platform a scheme can support, in the order the
# sweep prefers them.
DESTINATIONS = [
    ("iphonesimulator", "generic/platform=iOS Simulator"),
    ("macosx", "generic/platform=macOS"),
    ("appletvsimulator", "generic/platform=tvOS Simulator"),
    ("watchsimulator", "generic/platform=watchOS Simulator"),
    ("xrsimulator", "generic/platform=visionOS Simulator"),
]

# The deployment targets a scheme Xcode 27 refuses builds with.
RAISED_TARGETS = [
    "IPHONEOS_DEPLOYMENT_TARGET=18.0",
    "MACOSX_DEPLOYMENT_TARGET=15.0",
    "TVOS_DEPLOYMENT_TARGET=18.0",
    "WATCHOS_DEPLOYMENT_TARGET=11.0",
]
TOO_LOW = re.compile(r"deployment target '\w+' is set to [\d.]+, but the range")

# Folders a local project keeps that aren't its own: generated sources, build
# output and package checkouts, with their own projects.
SKIPPED = {"Derived", ".build", "DerivedData", "derived-data", "SourcePackages", "checkouts"}

VM_BUILD = r"""
SWIFT_DIR="$(ls -d "$HOME"/swift/swift-*-RELEASE | sort | tail -1)"
export PATH="$HOME/bin:$SWIFT_DIR/usr/bin:$HOME/.cargo/bin:$PATH"
exec "$HOME/sweetpad-target/debug/sweetpad" "$@"
"""


def containers():
    """Every container to sweep and its schemes, the small ones first."""
    found = []
    for name in sorted(os.listdir(CORPUS)):
        if not name.startswith("_") or name.startswith(("_linux-sweep", "_linux-followups")):
            continue
        root = os.path.join(CORPUS, name)
        for dirpath, dirnames, _ in os.walk(root):
            for d in list(dirnames):
                if d.endswith(".xcodeproj"):
                    path = os.path.join(dirpath, d)
                    found.append((name, path, schemes_of(path)))
                if d.endswith((".xcodeproj", ".xcworkspace")) or d in SKIPPED:
                    dirnames.remove(d)
    fixtures = os.path.join(LIB, "fixtures")
    for slug in ["tuist-fixtures", "kingfisher", "alamofire", "netnewswire", "ice-cubes"]:
        meta = os.path.join(fixtures, slug, "xcode-27.0.0", "meta.json")
        with open(meta) as f:
            for sub in json.load(f)["subprojects"]:
                path = os.path.join(CORPUS, sub["project_path"])
                found.append((slug, path, [s["scheme"] for s in sub["schemes"]]))
    return found


def baseline_containers(path):
    """Repeat the exact container/scheme selection from an earlier summary."""
    with open(path) as source:
        rows = json.load(source)
    grouped = {}
    for row in rows:
        schemes = grouped.setdefault((row["slug"], row["container"]), [])
        if row["scheme"] not in schemes:
            schemes.append(row["scheme"])
    return [(slug, path, schemes) for (slug, path), schemes in grouped.items()]


def container_flag(path):
    return ["--workspace" if path.endswith(".xcworkspace") else "--project", path]


def mac_json(args, timeout=300):
    run = subprocess.run([MAC_SWEETPAD, *args, "--json", "--non-interactive"],
                         capture_output=True, text=True, timeout=timeout)
    try:
        return json.loads(run.stdout)
    except json.JSONDecodeError:
        return {"ok": False, "error": {"message": run.stderr.strip() or run.stdout.strip()}}


def schemes_of(path):
    result = mac_json(["scheme", "list", *container_flag(path)])
    return [s["name"] for s in result.get("data", {}).get("schemes", [])]


def destination(path, scheme):
    """The destination to build `scheme` for: the first platform it
    supports, or iOS Simulator for a package's scheme, which names none."""
    policy = fixture_policy(path, scheme, "schemeDestinations")
    if policy:
        return policy["destination"]
    result = mac_json(["settings", "show", *container_flag(path), "--scheme", scheme,
                       "--key", "SUPPORTED_PLATFORMS"])
    platforms = set()
    for target in result.get("data", {}).get("targets", []):
        platforms.update(target.get("settings", {}).get("SUPPORTED_PLATFORMS", "").split())
    for platform, dest in DESTINATIONS:
        if platform in platforms:
            return dest
    return DESTINATIONS[0][1]


def debug_configuration(path):
    """The container's Debug configuration, whatever its case, or its
    first."""
    result = mac_json(["project", "info", *container_flag(path)])
    names = result.get("data", {}).get("configurations", [])
    return next((n for n in names if n.lower() == "debug"), names[0] if names else "Debug")


def build(where, path, scheme, dest, log, settings=(), action="build"):
    args = [*(["test", "build"] if action == "test build" else ["build"]),
            *container_flag(path), "--scheme", scheme,
            "--configuration", debug_configuration(path),
            "--destination", dest, "--non-interactive", "--no-color"]
    if where == "mac":
        # Linux signs only a device build, so the Mac doesn't sign either.
        settings = [*settings, "CODE_SIGNING_ALLOWED=NO"]
    if settings:
        args += ["--", *settings]
    if where == "linux":
        command = ["orb", "-m", VM, "bash", "-c", VM_BUILD, "sweetpad", *args]
    else:
        command = [MAC_SWEETPAD, *args]
    start = time.time()
    try:
        run = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, timeout=TIMEOUT)
        output, status = run.stdout, run.returncode
    except subprocess.TimeoutExpired as e:
        output, status = (e.stdout or b"").decode(errors="replace") + "\n[timed out]\n", -1
    with open(log, "w") as f:
        f.write(output)
    errors = [line.strip() for line in output.splitlines()
              if re.match(r"\s*(error|.*: error):", line)]
    return {
        "ok": status == 0,
        "status": status,
        "seconds": round(time.time() - start, 1),
        "errors": list(dict.fromkeys(errors))[:40],
    }


def slugify(text):
    return re.sub(r"[^A-Za-z0-9._-]+", "_", text)


def summarize(out):
    rows = []
    for dirpath, _, files in os.walk(out):
        for name in files:
            if name.endswith(".json") and name != "summary.json":
                with open(os.path.join(dirpath, name)) as f:
                    rows.append(json.load(f))
    rows.sort(key=lambda r: (r["slug"], r["container"], r["scheme"]))
    with open(os.path.join(out, "summary.json"), "w") as f:
        json.dump(rows, f, indent=2)
    lines = ["| Project | Container | Scheme | Action | Destination | Raised | Linux | Xcode | First error |",
             "|---|---|---|---|---|---|---|---|---|"]
    for r in rows:
        linux = r.get("linux") or {}
        xcode = r.get("xcode") or {}
        mark = lambda b: "" if not b else ("pass" if b.get("ok") else "FAIL")
        error = (linux.get("errors") or [""])[0].replace("|", "\\|")[:140]
        lines.append(f"| {r['slug']} | {os.path.relpath(r['container'], CORPUS)} | {r['scheme']} "
                     f"| {r.get('action', 'build')} | {r['destination'] or 'none'} | {'yes' if r.get('raised') else ''} "
                     f"| {mark(linux)} | {mark(xcode)} | {error} |")
    with open(os.path.join(out, "summary.md"), "w") as f:
        f.write("\n".join(lines) + "\n")
    passed = sum(1 for r in rows if (r.get("linux") or {}).get("ok"))
    print(f"{passed} of {len(rows)} schemes built on Linux; {out}/summary.md")


PRINT = threading.Lock()


def say(text):
    with PRINT:
        print(text, flush=True)


def sweep(args, slug, path, schemes):
    """Build `schemes` of the container at `path`, one after another."""
    for scheme in schemes:
        retired = fixture_policy(path, scheme, "retiredSchemes")
        if retired:
            say(f"retired {scheme}: migrated into {retired['replacement']}")
            continue
        key = f"{slug} {os.path.relpath(path, CORPUS)} {scheme}"
        if args.only not in key:
            continue
        stem = os.path.join(args.out, slug, slugify(os.path.relpath(path, CORPUS)),
                            slugify(scheme))
        os.makedirs(os.path.dirname(stem), exist_ok=True)
        record = None
        if os.path.exists(stem + ".json"):
            with open(stem + ".json") as f:
                record = json.load(f)
        if record is None or args.force:
            dest = destination(path, scheme)
            record = {"slug": slug, "container": path, "scheme": scheme,
                      "destination": dest, "action": "build", "raised": False, "linux": None, "xcode": None}
            if dest:
                record["linux"] = build("linux", path, scheme, dest, stem + ".linux.log")
                if any("builds no targets for this action" in e for e in record["linux"]["errors"]):
                    record["action"] = "test build"
                    os.replace(stem + ".linux.log", stem + ".build-only.linux.log")
                    record["linux"] = build("linux", path, scheme, dest, stem + ".linux.log",
                                            action=record["action"])
                if any(TOO_LOW.search(e) for e in record["linux"]["errors"]):
                    record["raised"] = True
                    record["linux"] = build("linux", path, scheme, dest, stem + ".linux.log",
                                            RAISED_TARGETS, action=record["action"])
                linux = record["linux"]
                say(f"linux  {'pass' if linux['ok'] else 'FAIL'} {linux['seconds']:>6}s  {key}"
                    f"{'  (raised)' if record['raised'] else ''}")
        linux = record.get("linux")
        if (args.xcode and linux and (not linux["ok"] or getattr(args, "xcode_all", False))
                and (record.get("xcode") is None or args.force)):
            record["xcode"] = build("mac", path, scheme, record["destination"],
                                    stem + ".xcode.log",
                                    RAISED_TARGETS if record.get("raised") else (),
                                    action=record.get("action", "build"))
            xcode = record["xcode"]
            say(f"xcode  {'pass' if xcode['ok'] else 'FAIL'} {xcode['seconds']:>6}s  {key}")
        with open(stem + ".json", "w") as f:
            json.dump(record, f, indent=2)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default=os.path.join(CORPUS, "_linux-sweep"))
    parser.add_argument("--only", default="")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--xcode", action="store_true")
    parser.add_argument("--xcode-all", action="store_true", help="Run native controls for passing schemes too")
    parser.add_argument("--baseline", help="Repeat only the container/scheme rows in this summary JSON")
    parser.add_argument("--jobs", type=int, default=1)
    args = parser.parse_args()
    args.xcode = args.xcode or args.xcode_all
    os.makedirs(args.out, exist_ok=True)
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        selected = baseline_containers(args.baseline) if args.baseline else containers()
        futures = [pool.submit(sweep, args, *container) for container in selected]
        for future in futures:
            future.result()
    summarize(args.out)


if __name__ == "__main__":
    sys.exit(main())
