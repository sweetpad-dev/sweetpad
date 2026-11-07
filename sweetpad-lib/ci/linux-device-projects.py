#!/usr/bin/env python3
"""Prepare isolated device fixtures and a reviewable provisioning plan.

Sources remain pinned to linux-projects.json. Dependency directories are shared
with the prepared corpus. App identifiers change only in the fixture copies.
No Apple Developer account changes or signing operations occur here.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import plistlib

CI = Path(__file__).resolve().parent
REPO = CI.parent.parent
CORPUS = CI.parent / "corpus"
SHARED = {"node_modules", "Pods", ".dart_tool", "vendor"}
SKIP = {".git", ".build", "build", "DerivedData", "__pycache__"}


def copy_sources(source, target):
    target.mkdir()
    for entry in source.iterdir():
        dest = target / entry.name
        if entry.name in SKIP:
            continue
        if entry.is_symlink():
            dest.symlink_to(entry.resolve(), target_is_directory=entry.is_dir())
        elif entry.is_dir() and entry.name in SHARED:
            dest.symlink_to(entry.resolve(), target_is_directory=True)
        elif entry.is_dir():
            copy_sources(entry, dest)
        else:
            shutil.copy2(entry, dest)


def replace(path, old, new):
    text = path.read_text()
    if old in text:
        path.write_text(text.replace(old, new))


def set_identifier(sweetpad, project, target, identifier):
    subprocess.run([str(sweetpad), "pbxproj", "settings", "set", "--project", str(project),
                    "--target", target, "--force", "--json", "--non-interactive",
                    "PRODUCT_BUNDLE_IDENTIFIER=" + identifier], check=True, capture_output=True)


def configure_flutter(project):
    if project["slug"] != "flutter-samples":
        return
    root = Path(project["source"])
    # Dependency resolution is reusable. Host-specific build outputs are not.
    for folder in [root / ".dart_tool", root / "platform_design/.dart_tool"]:
        if folder.is_symlink():
            original = folder.resolve()
            folder.unlink()
            shutil.copytree(original, folder, ignore=shutil.ignore_patterns("flutter_build"))
    for name in ["Generated.xcconfig", "flutter_export_environment.sh"]:
        replace(root / "platform_design/ios/Flutter" / name,
                str(CORPUS / "flutter-samples"), str(root))
    ios = root / "platform_design/ios"
    shutil.copyfile(CI / "linux-flutter-xcode-backend.sh", ios / "SweetpadFlutterBackend.sh")
    replace(ios / "Runner.xcodeproj/project.pbxproj",
            '/bin/sh \\"$FLUTTER_ROOT/packages/flutter_tools/bin/xcode_backend.sh\\"',
            '/bin/bash \\"$SRCROOT/SweetpadFlutterBackend.sh\\"')
    project["deviceValidationScope"] = "Offline Release startup using an explicit Linux-hosted iOS AOT compiler and the prebuilt Flutter engine."


def configure_shared_js(project):
    slug = project["slug"]
    if slug not in ["react-native-template", "react-native-mmkv"]:
        return
    root = Path(project["source"])
    app = root / ("template" if slug == "react-native-template" else "example")
    original = CORPUS / slug / ("template" if slug == "react-native-template" else "")
    config = app / "metro.config.js"
    marker = "// Sweetpad shared dependency paths"
    if marker not in config.read_text():
        with config.open("a") as output:
            output.write("\n" + marker + "\nconst sweetpadDependencyRoot = " + json.dumps(str(original)) + ";\n"
                         "module.exports.watchFolders = [...new Set([...(module.exports.watchFolders || []), sweetpadDependencyRoot])];\n"
                         "module.exports.resolver.nodeModulesPaths = [require('path').join(sweetpadDependencyRoot, 'node_modules'), ...(module.exports.resolver.nodeModulesPaths || [])];\n")
    compiler = original / "node_modules/hermes-compiler/hermesc/linux64-bin/hermesc"
    settings = [s for s in project.get("linuxSettings", []) if not s.startswith("HERMES_CLI_PATH=")]
    project["linuxSettings"] = settings + ["HERMES_CLI_PATH=" + str(compiler)]
    pods = app / "ios/Pods"
    if pods.is_symlink():
        original_pods = pods.resolve()
        pods.unlink()
        pods.mkdir()
        for child in original_pods.iterdir():
            dest = pods / child.name
            if child.name == "Target Support Files":
                shutil.copytree(child, dest)
            else:
                dest.symlink_to(child, target_is_directory=child.is_dir())
    # CocoaPods generates an absolute Darwin path. PATH selects Apple's
    # tool on macOS and the managed signing adapter on Linux.
    for script in (pods / "Target Support Files").rglob("*.sh"):
        replace(script, "/usr/bin/codesign", "codesign")
    if slug == "react-native-mmkv":
        shutil.copyfile(CI / "mmkv-device-probe.js", app / "index.js")
        project["deviceProbe"] = "MMKV seven storage checks, including persistence across launches"


def use_existing_profile(plan, sweetpad):
    """Apply the explicitly limited, app-local startup configuration."""
    for project in plan["projects"]:
        configure_shared_js(project)
        configure_flutter(project)
        slug = project["slug"]
        if slug not in ["wikipedia-ios", "mastodon-ios", "swiftfin"]:
            continue
        root = Path(project["source"])
        pbx = Path(project["container"])
        data = json.loads(subprocess.check_output(["plutil", "-convert", "json", "-o", "-", str(pbx / "project.pbxproj")]))["objects"]
        targets = [v["name"] for v in data.values() if v.get("isa") == "PBXNativeTarget"
                   and (v.get("productType") == "com.apple.product-type.application"
                        or v.get("productType", "").startswith("com.apple.product-type.app-extension"))]
        (root / "SweetpadDevice.entitlements").write_bytes(plistlib.dumps({}))
        command = [str(sweetpad), "pbxproj", "settings", "set", "--project", str(pbx),
                   "--force", "--json", "--non-interactive", "CODE_SIGN_ENTITLEMENTS=SweetpadDevice.entitlements"]
        for target in targets:
            command.extend(["--target", target])
        subprocess.run(command,check=True,capture_output=True)
        if slug == "mastodon-ios":
            common = root / "MastodonSDK/Sources/MastodonCommon/AppName.swift"
            replace(common, '"group.org.joinmastodon.app.' + plan["team"] + '"',
                    '"org.joinmastodon.app.' + plan["team"] + '.validation"')
            for name in ["AppSecret.swift", "AuthenticationServiceProvider.swift"]:
                replace(root / "MastodonSDK/Sources/MastodonCore" / name, ", accessGroup: AppName.groupID", "")
            local = "try? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)"
            replace(root / "MastodonSDK/Sources/MastodonCore/LegacyPersistence/FileManager+Shared.swift",
                    "containerURL(forSecurityApplicationGroupIdentifier: AppName.groupID)", local)
            replace(root / "MastodonSDK/Sources/CoreDataStack/Extension/URL.swift",
                    "FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup)", local)
        elif slug == "wikipedia-ios":
            path = root / "Wikipedia/Code/NSFileManager+WMFGroup.m"
            replace(path, '@QUOTE(WMF_APP_GROUP_IDENTIFIER)', '@"dev.sweetpad.validation.wikipedia.' + plan["team"] + '"')
            replace(path, "[self containerURLForSecurityApplicationGroupIdentifier:WMFApplicationGroupIdentifier]",
                    "[self URLForDirectory:NSApplicationSupportDirectory inDomain:NSUserDomainMask appropriateForURL:nil create:YES error:nil]")
            replace(root / "Wikipedia/Code/Session.swift",
                    "HTTPCookieStorage.sharedCookieStorage(forGroupContainerIdentifier: WMFApplicationGroupIdentifier)",
                    "HTTPCookieStorage.shared")
            for relative in ["Environment/WMFDataEnvironment.swift", "Data Controllers/Reading Challenge/WMFReadingChallengeCompletionDataController.swift"]:
                replace(root / "WMFData/Sources/WMFData" / relative, '"group.org.wikimedia.wikipedia"',
                        '"dev.sweetpad.validation.wikipedia.' + plan["team"] + '"')
        if slug in ["wikipedia-ios", "mastodon-ios"]:
            target_name = "Wikipedia" if slug == "wikipedia-ios" else "Mastodon"
            target = next(v for v in data.values() if v.get("isa") == "PBXNativeTarget" and v["name"] == target_name)
            extensions = []
            for phase_id in target["buildPhases"]:
                phase = data[phase_id]
                if phase.get("isa") != "PBXCopyFilesBuildPhase": continue
                for build_file in phase["files"]:
                    ref = data[build_file].get("fileRef")
                    path = data.get(ref, {}).get("path", "")
                    if path.endswith(".appex"): extensions.append(path)
            if extensions:
                subprocess.run([str(sweetpad), "pbxproj", "membership", "remove", "--project", str(pbx),
                                "--target", target_name, "--force", "--json", "--non-interactive", *extensions],
                               check=True,capture_output=True)
            project["omittedExtensions"] = sorted(set(project.get("omittedExtensions", []) + extensions))
        project["deviceValidationScope"] = "Main-app startup only; app-local storage; no App Groups, shared keychain, push, associated domains, payments, or WiFi-information capability."
    plan["accountChanges"] = "none; reuse existing certificate and wildcard development profile"
    plan["scope"] = "Existing wildcard profile with isolated app-local startup fixtures. No account resources are created or changed."
    plan.pop("appleDeveloperResources", None)


def set_flutter_compiler(plan, compiler):
    if compiler:
        project = next(p for p in plan["projects"] if p["slug"] == "flutter-samples")
        key = "SWEETPAD_FLUTTER_IOS_GEN_SNAPSHOT="
        project["linuxSettings"] = [s for s in project.get("linuxSettings", []) if not s.startswith(key)] + [key + compiler]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--team", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--sweetpad", type=Path, default=REPO / "target/debug/sweetpad")
    parser.add_argument("--existing-profile", action="store_true", required=True, help="Use limited app-local fixtures without new account resources")
    parser.add_argument("--flutter-ios-snapshot", help="Linux path to the iOS gen_snapshot built with linux-flutter-snapshot.py")
    args = parser.parse_args()
    args.out = args.out.resolve()
    if args.existing_profile and (args.out / "plan.json").is_file():
        plan = json.loads((args.out / "plan.json").read_text())
        use_existing_profile(plan, args.sweetpad)
        set_flutter_compiler(plan, args.flutter_ios_snapshot)
        (args.out / "plan.json").write_text(json.dumps(plan, indent=2) + "\n")
        return
    args.out.mkdir(parents=True, exist_ok=False)
    source = args.out / "sources"
    source.mkdir()
    projects = json.loads((CI / "linux-projects.json").read_text())
    prefix = "dev.sweetpad.validation." + args.team.lower()
    hybrid = {
        "flutter-samples": ("Runner", prefix + ".flutter"),
        "react-native-template": ("HelloWorld", prefix + ".reactnative"),
        "react-native-mmkv": ("MmkvExample", prefix + ".mmkv"),
        "ionic-conference-app": ("App", prefix + ".capacitor"),
    }
    for project in projects:
        slug = project["slug"]
        root = source / slug
        actual = subprocess.check_output(["git", "-C", str(CORPUS / slug), "rev-parse", "HEAD"], text=True).strip()
        if actual != project["commit"]:
            raise RuntimeError(f"{slug}: source revision differs from the manifest")
        copy_sources(CORPUS / slug, root)
        container = root / project.get("project", project.get("workspace"))
        pbx = container if container.suffix == ".xcodeproj" else container.with_suffix(".xcodeproj")
        project["source"] = str(root)
        project["container"] = str(container)
        project["configuration"] = "Release" if slug in hybrid else "Debug"
        configure_shared_js(project)
        configure_flutter(project)
        if slug == "wikipedia-ios":
            config = root / "Configurations/OpenSourceDebug.xcconfig"
            config.write_text(f"DEVELOPMENT_TEAM={args.team}\nSIGNING_DISAMBIGUATOR={args.team}\n")
            project["deviceBundleIdentifier"] = "org.wikimedia.wikipedia" + args.team
        elif slug == "mastodon-ios":
            # AppName.groupID and all extension entitlements must stay equal.
            for path in root.rglob("*"):
                if path.is_file() and not path.is_symlink() and path.suffix in [".swift", ".entitlements", ".pbxproj"]:
                    replace(path, "org.joinmastodon.app", "org.joinmastodon.app." + args.team)
            project["deviceBundleIdentifier"] = "org.joinmastodon.app." + args.team
        elif slug == "swiftfin":
            identifier = "org.jellyfin.swiftfin." + args.team
            set_identifier(args.sweetpad, pbx, "Swiftfin iOS", identifier)
            project["deviceBundleIdentifier"] = identifier
        else:
            target, identifier = hybrid[slug]
            set_identifier(args.sweetpad, pbx, target, identifier)
            project["deviceBundleIdentifier"] = identifier
        print("Prepared", slug, project["deviceBundleIdentifier"], flush=True)
    plan = {"team": args.team, "projects": projects}
    use_existing_profile(plan, args.sweetpad)
    set_flutter_compiler(plan, args.flutter_ios_snapshot)
    (args.out / "plan.json").write_text(json.dumps(plan, indent=2) + "\n")


if __name__ == "__main__":
    main()
