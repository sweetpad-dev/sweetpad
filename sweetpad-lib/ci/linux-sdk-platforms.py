#!/usr/bin/env python3
"""Copy additional Apple SDK platforms from the user's Xcode into a Darwin SDK.

Copies SDK data and platform metadata, without copying macOS-only executables.
Uses buffered reads because optimized copies from shared mounts can lose data.
"""
import argparse
import copy
import json
from pathlib import Path
import shutil

PLATFORMS = (
    "AppleTVOS", "AppleTVSimulator", "WatchOS", "WatchSimulator", "XROS", "XRSimulator",
)


def copy_tree(source, destination):
    if source.is_symlink():
        target = source.readlink()
        if destination.is_symlink() and destination.readlink() == target:
            return
        destination.unlink(missing_ok=True)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.symlink_to(target)
    elif source.is_dir():
        destination.mkdir(parents=True, exist_ok=True)
        for child in source.iterdir():
            copy_tree(child, destination / child.name)
    elif source.is_file():
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.is_symlink():
            destination.unlink()
        # Compare bytes, not size/mtime: a failed shared-mount copy can keep both.
        if destination.is_file() and source.stat().st_size == destination.stat().st_size:
            with source.open("rb") as a, destination.open("rb") as b:
                while (chunk := a.read(1024 * 1024)) == b.read(1024 * 1024):
                    if not chunk:
                        return
        temporary = destination.with_name(destination.name + ".sweetpad-copy")
        with source.open("rb") as a, temporary.open("wb") as b:
            shutil.copyfileobj(a, b, length=1024 * 1024)
        shutil.copystat(source, temporary)
        temporary.replace(destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("xcode", type=Path)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--metadata-only", action="store_true", help="Update triple mappings after a completed SDK copy")
    args = parser.parse_args()
    source = args.xcode / "Contents/Developer/Platforms"
    destination = args.bundle / "Developer/Platforms"
    for platform in PLATFORMS:
        root = source / (platform + ".platform")
        if not (root / "Developer/SDKs").is_dir():
            continue
        target = destination / root.name
        for part in (() if args.metadata_only else ("Info.plist", "version.plist", "PlatformBasicInfo.plist", "Developer/SDKs",
                     "Developer/Library/Xcode/Specifications", "Developer/Library/Xcode/Agents/XCTRunner.app", "Developer/Library/Frameworks",
                     "Developer/Library/PrivateFrameworks", "Developer/usr/lib", "Library/Application Support")):
            if (root / part).exists():
                copy_tree(root / part, target / part)
        print(f"Imported {root.name}", flush=True)
    # iOS companion/watch products can also need these stubs in the base SDK.
    for platform in ("iPhoneOS", "iPhoneSimulator"):
        root = source / (platform + ".platform")
        stubs = root / "Library/Application Support"
        if stubs.is_dir() and not args.metadata_only:
            copy_tree(stubs, destination / root.name / "Library/Application Support")
    update_triples(args.bundle)


def update_triples(bundle):
    """Swift Build matches the destination triple to this SDK metadata.

    Without a matching entry it silently uses the Linux Swift resource path,
    even when the Apple platform SDK itself is present.
    """
    path = bundle / "swift-sdk.json"
    metadata = json.loads(path.read_text())
    triples = metadata["targetTriples"]
    template = next(iter(triples.values()))
    for platform in PLATFORMS:
        root = bundle / "Developer/Platforms" / (platform + ".platform")
        seen = set()
        for sdk in sorted((root / "Developer/SDKs").glob("*.sdk")):
            settings = json.loads((sdk / "SDKSettings.json").read_text())
            if settings["CanonicalName"] in seen:
                continue
            seen.add(settings["CanonicalName"])
            for target in settings["SupportedTargets"].values():
                vendor = target["LLVMTargetTripleVendor"]
                system = target["LLVMTargetTripleSys"]
                environment = target.get("LLVMTargetTripleEnvironment", "")
                suffix = "-" + environment if environment else ""
                for arch in target["Archs"]:
                    entry = copy.deepcopy(template)
                    entry["sdkRootPath"] = str(sdk.relative_to(bundle))
                    libraries = str((root / "Developer/usr/lib").relative_to(bundle))
                    entry["includeSearchPaths"] = [libraries]
                    entry["librarySearchPaths"] = [libraries]
                    triples[f"{arch}-{vendor}-{system}{suffix}"] = entry
    content = json.dumps(metadata, indent=2) + "\n"
    if path.read_text() != content:
        temporary = path.with_name(path.name + ".sweetpad-copy")
        temporary.write_text(content)
        temporary.replace(path)


if __name__ == "__main__":
    main()
