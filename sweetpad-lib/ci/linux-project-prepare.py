#!/usr/bin/env python3
"""Prepare pinned hybrid app dependencies on the current build host.

Use a fresh --corpus-root to verify setup without reusing generated Pods or
framework configuration. The receipt records the host and frozen dependency graph.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

CI = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("projects", CI / "linux-projects.py")
projects = importlib.util.module_from_spec(spec)
spec.loader.exec_module(projects)
FLUTTER_PIN = "c197ca5199e9ee5661e865596c9c07294dfab51c"
RELOCATED_PODS = ("hermes-engine", "React-Core-prebuilt", "ReactNativeDependencies", "NitroMmkv")


def flutter_configuration(root, flutter_root):
    """The checked-in iOS project only needs generated build properties.

    Flutter hides `build ios --config-only` on Linux even though its assemble
    backend works there. Resolve Dart dependencies normally, then write these
    path and version inputs explicitly instead of running a macOS CLI.
    """
    app = root / "platform_design"
    # Match the pinned Flutter SDK's IOSDeploymentTargetMigration. The
    # prebuilt engine already requires iOS 15; this is an explicit fixture
    # migration, not a change to arbitrary application deployment targets.
    project = app / "ios/Runner.xcodeproj/project.pbxproj"
    text = project.read_text()
    text = text.replace("IPHONEOS_DEPLOYMENT_TARGET = 11.0;", "IPHONEOS_DEPLOYMENT_TARGET = 15.0;")
    project.write_text(text)
    framework = app / "ios/Flutter/AppFrameworkInfo.plist"
    framework.write_text(framework.read_text().replace('  <key>MinimumOSVersion</key>\n  <string>11.0</string>\n', ''))
    match = re.search(r"(?m)^version:\s*([^\s]+)", (app / "pubspec.yaml").read_text())
    if not match:
        raise RuntimeError("Flutter fixture must declare its version")
    version, _, number = match[1].partition("+")
    package_config = root / ".dart_tool/package_config.json"
    if not package_config.is_file():
        raise RuntimeError("Flutter workspace package configuration is missing")
    settings = {"FLUTTER_ROOT": str(flutter_root), "FLUTTER_APPLICATION_PATH": str(app),
                "FLUTTER_TARGET": "lib/main.dart", "FLUTTER_BUILD_DIR": "build",
                "FLUTTER_BUILD_NAME": version, "FLUTTER_BUILD_NUMBER": number or "1",
                "PACKAGE_CONFIG": str(package_config), "DART_OBFUSCATION": "false",
                "TRACK_WIDGET_CREATION": "false", "TREE_SHAKE_ICONS": "false"}
    generated = app / "ios/Flutter/Generated.xcconfig"
    generated.write_text("// Generated for the pinned Linux fixture.\n" + "".join(f"{k}={v}\n" for k, v in settings.items()))


def portable_pod_lock(text):
    for name in RELOCATED_PODS:
        text = re.sub(rf"(?m)^  {re.escape(name)}: [0-9a-f]{{40}}$", f"  {name}: @PATH_CHECKSUM@", text)
    return text


def run(command, cwd, env):
    print("+", " ".join(map(str, command)), flush=True)
    subprocess.run(list(map(str, command)), cwd=cwd, env=env, check=True)


def frozen_files(slug, root):
    locks = CI / "framework-locks" / slug
    files = []
    if locks.exists():
        for source in sorted(locks.rglob("*")):
            if not source.is_file() or source.parent.name == "podspecs":
                continue
            target = root / source.relative_to(locks)
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists() and target.read_bytes() != source.read_bytes():
                original = subprocess.run(["git", "-C", str(root), "show", "HEAD:" + target.relative_to(root).as_posix()], capture_output=True)
                tracked_original = original.returncode == 0 and original.stdout == target.read_bytes()
                prior = target.read_text()
                if slug == "react-native-mmkv" and target.name == "Podfile.lock":
                    # Retiring the exact debug-thunk Podfile patch changes only
                    # this checksum. Dependency graph/version changes still fail.
                    prior = prior.replace("PODFILE CHECKSUM: 11a564a05207900f861412c2399bb85289308955",
                                          "PODFILE CHECKSUM: ac6e3a3935879b4d187353a98679212e0e3604ce")
                relocated_lock = target.name == "Podfile.lock" and portable_pod_lock(prior) == portable_pod_lock(source.read_text())
                if not tracked_original and not relocated_lock:
                    raise RuntimeError(f"{target}: differs from the captured dependency lock; use a fresh --corpus-root")
            shutil.copyfile(source, target)
            files.append(target)
    # Keep tracked locks in the receipt too.
    for name in ["bun.lock", "package-lock.json"]:
        path = root / name
        if path.is_file() and path not in files:
            files.append(path)
    return {p: hashlib.sha256(p.read_bytes()).hexdigest() for p in files}


def relocate_pod_lock(slug, root, app, env):
    # RN embeds absolute local tarball and hermesc paths in three podspecs.
    # Compute their checksums from captured specs with only the root relocated.
    # CocoaPods deployment mode then validates the complete installed graph.
    with tempfile.TemporaryDirectory(prefix="sweetpad-podspecs-") as work:
        specs = []
        for source in sorted((CI / "framework-locks" / slug / "podspecs").glob("*.json")):
            target = Path(work) / source.name
            target.write_text(source.read_text().replace("@SOURCE_ROOT@", str(root)))
            specs.append(target)
        script = 'puts JSON.generate(ARGV.to_h { |p| s = Pod::Specification.from_file(p); [s.name, s.checksum] })'
        values = json.loads(subprocess.check_output(
            ["bundle", "exec", "ruby", "-rcocoapods", "-rjson", "-e", script, *map(str, specs)],
            cwd=app, env=env, text=True))
    lock = app / "ios/Podfile.lock"
    text = lock.read_text()
    for name, checksum in values.items():
        text, count = re.subn(rf"(?m)^  {re.escape(name)}: [0-9a-f]{{40}}$", f"  {name}: {checksum}", text)
        if count != 1:
            raise RuntimeError(f"{lock}: expected one checksum for {name}")
    lock.write_text(text)
    return lock, hashlib.sha256(text.encode()).hexdigest()


def prepare(project, args, env):
    root = projects.prepare(project, True, args.corpus_root)
    locks = frozen_files(project["slug"], root)
    framework = project["framework"]
    if framework == "Flutter":
        revision = subprocess.check_output(["git", "-C", str(args.flutter_root), "rev-parse", "HEAD"], text=True).strip()
        if revision != FLUTTER_PIN:
            raise RuntimeError(f"Flutter: expected {FLUTTER_PIN}, found {revision}")
        flutter = args.flutter_root / "bin/flutter"
        run([flutter, "precache", "--ios"], root, env)
        run([flutter, "pub", "get", "--enforce-lockfile"], root, env)
        if sys.platform == "linux":
            run([flutter, "pub", "get", "--enforce-lockfile"], root / "platform_design", env)
            flutter_configuration(root, args.flutter_root)
        else:
            run([flutter, "build", "ios", "--simulator", "--config-only", "--no-codesign"], root / "platform_design", env)
    elif framework == "React Native":
        mmkv = project["slug"] == "react-native-mmkv"
        app = root / ("example" if mmkv else "template")
        if mmkv:
            run(["bun", "install", "--frozen-lockfile"], root, env)
        else:
            run(["npm", "ci", "--no-audit", "--no-fund"], app, env)
        # Deployment mode forbids CocoaPods from silently resolving another graph.
        ruby_env = dict(env, BUNDLE_FROZEN="true", BUNDLE_VERSION="4.0.20",
                        BUNDLE_PATH=str(args.gem_home.resolve()))
        run(["bundle", "install"], app, ruby_env)
        lock, digest = relocate_pod_lock(project["slug"], root, app, ruby_env)
        locks[lock] = digest
        if sys.platform == "linux":
            # Retain the small adapters so the execution audit can inspect
            # every executed script after preparation completes.
            metadata = args.gem_home.resolve() / "metadata-tools"
            metadata.mkdir(parents=True, exist_ok=True)
            for tool in ["xcodebuild", "xcode-select", "command"]:
                path = metadata / tool
                path.write_text("#!/bin/sh\nexec " + " ".join(map(shlex.quote,
                    [sys.executable, str(CI / "linux-xcode-metadata.py"), tool])) + ' "$@"\n')
                path.chmod(0o755)
            ruby_env = dict(ruby_env, PATH=str(metadata) + os.pathsep + ruby_env["PATH"])
        run(["bundle", "exec", "pod", "install", "--deployment"], app / "ios", ruby_env)
        if sys.platform == "linux":
            for script in (app / "ios/Pods/Target Support Files").rglob("*.sh"):
                text = script.read_text()
                if "/usr/bin/codesign" in text:
                    script.write_text(text.replace("/usr/bin/codesign", "codesign"))
        (app / "ios/.xcode.env.local").write_text('export NODE_BINARY=$(command -v node)\n')
    elif framework == "Capacitor":
        run(["npm", "ci", "--no-audit", "--no-fund"], root, dict(env, PUPPETEER_SKIP_DOWNLOAD="1"))
        run(["npm", "run", "build"], root, env)
        run(["npx", "--no-install", "cap", "sync", "ios"], root, env)
    for path, digest in locks.items():
        if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise RuntimeError(f"dependency preparation changed frozen lock {path}")
    return {"commit": project["commit"], "framework": framework, "source": str(root),
            "patches": project.get("patches", []),
            "retiredPatches": project.get("retiredPatches", []),
            "locks": {str(p.relative_to(root)): digest for p, digest in locks.items()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", default="")
    parser.add_argument("--corpus-root", type=Path, default=projects.CORPUS)
    parser.add_argument("--out", type=Path, default=projects.CORPUS / "_linux-followups/preparation.json")
    parser.add_argument("--tool-bin", type=Path, default=projects.CORPUS / "_linux-followups/tools/js/node_modules/.bin")
    parser.add_argument("--flutter-root", type=Path, default=projects.CORPUS / "_linux-followups/tools/flutter")
    parser.add_argument("--gem-home", type=Path, default=projects.CORPUS / "_linux-followups/tools/ruby-gems")
    args = parser.parse_args()
    if sys.platform not in ["darwin", "linux"]:
        parser.error("dependency preparation supports macOS and Linux")
    args.corpus_root = args.corpus_root.resolve()
    args.corpus_root.mkdir(parents=True, exist_ok=True)
    args.flutter_root = args.flutter_root.resolve()
    env = dict(os.environ, PATH=str(args.tool_bin.resolve()) + os.pathsep + os.environ["PATH"])
    selected = [p for p in json.loads((CI / "linux-projects.json").read_text())
                if p.get("framework") and args.only in p["slug"]]
    if not selected:
        parser.error("no hybrid projects match --only")
    required = {"node": "v24.22.2"}
    if any(p["framework"] == "React Native" for p in selected): required["bundle"] = "4.0.20"
    if any(p["slug"] == "react-native-mmkv" for p in selected): required["bun"] = "1.3.14"
    versions = {}
    for tool, expected in required.items():
        version = subprocess.check_output([tool, "--version"], text=True, env=env).strip()
        if version.removeprefix("Bundler version ") != expected:
            raise RuntimeError(f"{tool}: expected {expected}, found {version}")
        versions[tool] = version
    args.out.parent.mkdir(parents=True, exist_ok=True)
    records = {"host": sys.platform, "tools": versions, "projects": {}}
    try:
        for project in selected:
            records["projects"][project["slug"]] = prepare(project, args, env)
            args.out.write_text(json.dumps(records, indent=2) + "\n")
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        records["error"] = str(error)
        args.out.write_text(json.dumps(records, indent=2) + "\n")
        raise


if __name__ == "__main__":
    main()
