#!/usr/bin/env python3
"""Private Apple SDK export and transactional Linux import for SweetPad."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import plistlib
import posixpath
import shutil
import subprocess
import sys
import tarfile
import tempfile

CI = Path(__file__).resolve().parent
PINS = json.loads((CI / "linux-toolchain.json").read_text())
VERSIONS = {key: PINS[key] for key in ("swift", "swiftBuild", "xtool")}
# Bump when SDK construction changes; the export archive format stays separate.
IMPORTER_VERSION = 2
BASE_PLATFORMS = ("iPhoneOS", "iPhoneSimulator", "MacOSX")
EXTRA_PLATFORMS = ("AppleTVOS", "AppleTVSimulator", "WatchOS", "WatchSimulator", "XROS", "XRSimulator")


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def require_space(path, required):
    free = shutil.disk_usage(path).free
    if free < required:
        raise ValueError(f"Need {required / 1024**3:.1f} GiB free at {path}; available: {free / 1024**3:.1f} GiB")


def export_roots(xcode):
    roots = ["version.plist"]
    toolchain = "Developer/Toolchains/XcodeDefault.xctoolchain/usr"
    roots += [f"{toolchain}/lib/{name}" for name in ("swift", "swift_static", "clang")]
    roots += [f"{toolchain}/include/swift"]
    roots += [str(p.relative_to(xcode / "Contents")) for p in
              sorted((xcode / "Contents" / toolchain / "lib").glob("swift-*")) if p.is_dir()]
    for name in BASE_PLATFORMS + EXTRA_PLATFORMS:
        root = f"Developer/Platforms/{name}.platform"
        if name in BASE_PLATFORMS and not (xcode / "Contents" / root / "Developer/SDKs").is_dir():
            raise ValueError(f"Missing required platform: {name}")
        roots += [f"{root}/{part}" for part in ("Info.plist", "version.plist", "PlatformBasicInfo.plist",
                  "Developer/SDKs", "Developer/Library/Frameworks", "Developer/Library/PrivateFrameworks",
                  "Developer/Library/Xcode/Specifications", "Developer/Library/Xcode/Agents/XCTRunner.app",
                  "Developer/usr/lib", "Library/Application Support")]
    return [xcode / "Contents" / name for name in roots if (xcode / "Contents" / name).exists()]


def export_entries(xcode):
    entries = {}

    def visit(path):
        # Compiler caches contain large, machine-specific prebuilt modules.
        if path.name == "prebuilt-modules":
            return
        relative = "Xcode.app/" + path.relative_to(xcode).as_posix()
        if path.is_symlink():
            target = os.readlink(path)
            resolved = path.resolve()
            resolved.relative_to(xcode)  # Reject links outside the supplied Xcode.
            if target.startswith("/"):
                target = os.path.relpath(resolved, path.parent)
            entries[relative] = {"type": "symlink", "target": target}
        elif path.is_dir():
            for child in sorted(path.iterdir()):
                visit(child)
        elif path.is_file():
            entries[relative] = {"type": "file", "size": path.stat().st_size,
                                 "mode": path.stat().st_mode & 0o777}

    for root in export_roots(xcode):
        visit(root)
    # Every link must resolve within the selected export, not just within Xcode.
    for name, entry in entries.items():
        if entry["type"] == "symlink":
            resolved = "Xcode.app/" + (xcode / name.removeprefix("Xcode.app/")).resolve().relative_to(xcode).as_posix()
            if resolved not in entries and not any(p.startswith(resolved + "/") for p in entries):
                raise ValueError(f"Export would contain a dangling link: {name}")
    return entries


def export_sdk(args):
    if platform.system() != "Darwin":
        raise ValueError("Export Apple SDK files on a Mac with Xcode")
    xcode = args.xcode.resolve()
    version = plistlib.loads((xcode / "Contents/version.plist").read_bytes())
    entries = export_entries(xcode)
    size = sum(item.get("size", 0) for item in entries.values())
    output = args.output.absolute()
    if "\n" in output.name or "\r" in output.name:
        raise ValueError("Archive filename must not contain a newline")
    report = {"archive": str(output), "xcodeBuild": version["ProductBuildVersion"],
              "xcodeVersion": version["CFBundleShortVersionString"], "toolchain": VERSIONS,
              "fileCount": len(entries), "unpackedBytes": size,
              "requiredFreeBytes": size + 256 * 1024**2, "dryRun": args.dry_run}
    if args.dry_run:
        return report
    if output.exists() or output.with_name(output.name + ".sha256").exists():
        raise ValueError("Export output already exists; select a new output path")
    output.parent.mkdir(parents=True, exist_ok=True)
    require_space(output.parent, report["requiredFreeBytes"])
    print(f"Exporting {len(entries)} files ({size / 1024**3:.1f} GiB before compression)", file=sys.stderr, flush=True)
    with tempfile.TemporaryDirectory(prefix="sweetpad-sdk-export-", dir=output.parent) as work:
        work = Path(work)
        phrases = work / "appintents-phrases.json"
        subprocess.run([sys.executable, str(CI / "linux-appintents-resources.py"), "--xcode", str(xcode),
                        "--out", str(phrases)], check=True, stdout=sys.stderr)
        entries["appintents-phrases.json"] = {"type": "file", "size": phrases.stat().st_size, "mode": 0o644}
        report["fileCount"] = len(entries)
        report["unpackedBytes"] += phrases.stat().st_size
        for name, entry in entries.items():
            if entry["type"] == "file":
                source = phrases if name == phrases.name else xcode / name.removeprefix("Xcode.app/")
                entry["sha256"] = digest(source)
        manifest = {"format": "sweetpad-apple-sdk", "version": 1, **report, "entries": entries}
        temporary = work / "sdk.tar.gz"
        with tarfile.open(temporary, "w:gz", compresslevel=1) as archive:
            content = (json.dumps(manifest, sort_keys=True) + "\n").encode()
            info = tarfile.TarInfo("manifest.json")
            info.size, info.mode = len(content), 0o600
            archive.addfile(info, io.BytesIO(content))
            for name, entry in entries.items():
                info = tarfile.TarInfo(name)
                if entry["type"] == "symlink":
                    info.type, info.linkname = tarfile.SYMTYPE, entry["target"]
                    archive.addfile(info)
                else:
                    info.size, info.mode = entry["size"], entry["mode"]
                    source = phrases if name == phrases.name else xcode / name.removeprefix("Xcode.app/")
                    with source.open("rb") as data:
                        archive.addfile(info, data)
        checksum = digest(temporary)
        os.chmod(temporary, 0o600)
        # Exclusive creation prevents overwriting another export.
        os.link(temporary, output)
        sidecar = output.with_name(output.name + ".sha256")
        with sidecar.open("x") as target:
            target.write(f"{checksum}  {output.name}\n")
    return {**report, "sha256": checksum, "archiveBytes": output.stat().st_size}


def safe_name(name):
    parts = PurePosixPath(name).parts
    if not parts or name.startswith("/") or ".." in parts or str(PurePosixPath(name)) != name:
        raise ValueError(f"Unsafe archive path: {name}")
    if name != "appintents-phrases.json" and not name.startswith("Xcode.app/Contents/"):
        raise ValueError(f"Unexpected SDK archive path: {name}")


def unpack_verified(archive, work):
    with tarfile.open(archive, "r:gz") as source:
        first = source.next()
        if first is None or first.name != "manifest.json" or not first.isfile() or first.size > 64 * 1024**2:
            raise ValueError("SDK archive must start with its manifest")
        manifest = json.load(source.extractfile(first))
        if manifest.get("format") != "sweetpad-apple-sdk" or manifest.get("version") != 1:
            raise ValueError("Unsupported SDK archive format")
        if manifest.get("toolchain") != VERSIONS:
            raise ValueError(f"Incompatible SDK toolchain requirements; expected {VERSIONS}")
        entries = manifest["entries"]
        for name in entries:
            safe_name(name)
        require_space(work, 2 * sum(e.get("size", 0) for e in entries.values()) + 512 * 1024**2)
        seen = set()
        # Defer links until files exist. No archive member can write through a link.
        links = []
        while (member := source.next()) is not None:
            name = member.name
            safe_name(name)
            if name in seen or name not in entries:
                raise ValueError(f"Duplicate or unlisted archive entry: {name}")
            seen.add(name)
            entry = entries[name]
            destination = work / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            if member.issym() and entry["type"] == "symlink" and member.linkname == entry["target"]:
                target = posixpath.normpath(posixpath.join(posixpath.dirname(name), member.linkname))
                safe_name(target)
                if member.linkname.startswith("/"):
                    raise ValueError(f"Absolute archive link: {name}")
                links.append((destination, member.linkname))
            elif member.isfile() and entry["type"] == "file" and member.size == entry["size"]:
                with source.extractfile(member) as data, destination.open("xb") as output:
                    shutil.copyfileobj(data, output)
                if digest(destination) != entry["sha256"]:
                    raise ValueError(f"SDK checksum mismatch: {name}")
                destination.chmod(entry["mode"] & 0o777)
            else:
                raise ValueError(f"Unsupported or changed archive entry: {name}")
        if seen != set(entries):
            raise ValueError("SDK archive is missing manifest entries")
        for path, target in links:
            path.symlink_to(target)
        for path, _ in links:
            path.resolve().relative_to(work.resolve())
            if not path.exists():
                raise ValueError(f"Dangling SDK link: {path}")
    return manifest


def inspect_archive(archive):
    with archive.open("rb") as source:
        magic = source.read(4)
    if magic != b"\x1f\x8b\x08\x00" and not magic.startswith(b"\x1f\x8b"):
        return inspect_xcode_archive(archive, magic)
    checksum = digest(archive)
    expected = archive.with_name(archive.name + ".sha256").read_text().strip().split(maxsplit=1)
    if len(expected) != 2 or expected[0] != checksum or expected[1] != archive.name:
        raise ValueError("SDK archive checksum does not match its .sha256 file")
    with tarfile.open(archive, "r:gz") as source:
        first = source.next()
        if first is None or first.name != "manifest.json" or not first.isfile() or first.size > 64 * 1024**2:
            raise ValueError("SDK archive must start with its manifest")
        manifest = json.load(source.extractfile(first))
    if manifest.get("format") != "sweetpad-apple-sdk" or manifest.get("version") != 1:
        raise ValueError("Unsupported SDK archive format")
    if manifest.get("toolchain") != VERSIONS:
        raise ValueError(f"Incompatible SDK toolchain requirements; expected {VERSIONS}")
    size = 0
    for name, entry in manifest["entries"].items():
        safe_name(name)
        if entry.get("type") not in ("file", "symlink"):
            raise ValueError(f"Unsupported manifest entry: {name}")
        if entry["type"] == "file":
            if not isinstance(entry.get("size"), int) or entry["size"] < 0:
                raise ValueError(f"Invalid file size: {name}")
            size += entry["size"]
    return {"archiveSha256": checksum, "xcodeVersion": manifest["xcodeVersion"],
            "xcodeBuild": manifest["xcodeBuild"], "toolchain": VERSIONS,
            "unpackedBytes": size, "requiredFreeBytes": 2 * size + 512 * 1024**2}


def inspect_xcode_archive(archive, magic):
    kind = "xip" if magic == b"xar!" else "aea" if magic == b"AEA1" else "apple-archive" if magic in (b"AA01", b"YAA1", b"pbzx", b"pbzm", b"pbze", b"pbzz", b"pbz4") else None
    if kind is None or archive.stat().st_size < 12:
        raise ValueError("Expected a Sweetpad SDK export, Xcode .xip, .aea, or Apple Archive")
    checksum = digest(archive)
    sidecar = archive.with_name(archive.name + ".sha256")
    if sidecar.exists():
        expected = sidecar.read_text().strip().split(maxsplit=1)
        if len(expected) != 2 or expected[0] != checksum or expected[1] != archive.name:
            raise ValueError("Xcode archive checksum does not match its .sha256 file")
    return {"archiveSha256": checksum, "sourceFormat": kind, "archiveBytes": archive.stat().st_size,
            "toolchain": VERSIONS, "requiredFreeBytes": 512 * 1024**2}


def unpack_xcode_archive(archive, work, args):
    import importlib.util
    spec = importlib.util.spec_from_file_location("sweetpad_xcode_archive", CI / "linux-xcode-archive.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    print("Reading Xcode archive and selecting private SDK inputs", file=sys.stderr, flush=True)
    xcode = module.extract(archive, work, getattr(args, "aea_key_file", None), getattr(args, "aea_public_key_file", None),
                          progress=lambda message: print(message, file=sys.stderr, flush=True))
    version = plistlib.loads((xcode / "Contents/version.plist").read_bytes())
    # Check the baseline inputs before invoking any SDK builder.
    export_roots(xcode)
    return {"xcodeVersion": version["CFBundleShortVersionString"], "xcodeBuild": version["ProductBuildVersion"]}


def copy_host_clang_headers(bundle, copy_tree):
    # `xtool sdk build` deliberately removes Xcode's Clang include directory.
    # Its install operation normally supplies the Linux compiler's headers.
    target = json.loads(subprocess.check_output(["swift", "-print-target-info"], text=True))
    if "Swift version " + VERSIONS["swift"] not in target.get("compilerVersion", ""):
        raise ValueError("SDK import requires Swift " + VERSIONS["swift"] + " on PATH")
    host_include = Path(target["paths"]["runtimeResourcePath"]) / "clang/include"
    if not (host_include / "stddef.h").is_file():
        raise ValueError(f"Linux Swift Clang resource headers are missing: {host_include}")
    resource_include = bundle / "Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/clang/include"
    copy_tree(host_include, resource_include)


def preserve_platform_manifest_modes(xcode, bundle):
    # xtool rewrites these manifests with Linux build properties. Keep those
    # contents while restoring the archive's permissions, even under umask 0000.
    for root in (bundle / "Developer/Platforms").iterdir():
        source = xcode / "Contents/Developer/Platforms" / root.name / "Info.plist"
        target = root / "Info.plist"
        if source.is_file() and target.is_file() and not target.is_symlink():
            shutil.copymode(source, target)


def supplement(xcode, bundle, phrases):
    subprocess.run([sys.executable, str(CI / "linux-sdk-platforms.py"), str(xcode), str(bundle)],
                   check=True, stdout=sys.stderr)
    import importlib.util
    spec = importlib.util.spec_from_file_location("sweetpad_platforms", CI / "linux-sdk-platforms.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    copy_host_clang_headers(bundle, module.copy_tree)
    contents = xcode / "Contents"
    module.copy_tree(contents / "version.plist", bundle / "version.plist")
    relative = Path("Developer/Toolchains/XcodeDefault.xctoolchain/usr/include/swift")
    module.copy_tree(contents / relative, bundle / relative)
    for root in (bundle / "Developer/Platforms").iterdir():
        for part in ("version.plist", "Library/Application Support/MessagesApplicationExtensionStub"):
            source = contents / "Developer/Platforms" / root.name / part
            if source.exists():
                module.copy_tree(source, root / part)
    preserve_platform_manifest_modes(xcode, bundle)
    if phrases.is_file():
        shutil.copyfile(phrases, bundle / "appintents-phrases.json")


def import_sdk(args):
    if platform.system() != "Linux" or platform.machine() not in ("aarch64", "x86_64"):
        raise ValueError("SDK import requires ARM64 or x86_64 Linux")
    import fcntl
    destination = args.destination.absolute()
    destination.parent.mkdir(parents=True, exist_ok=True)
    with (destination.parent / ("." + destination.name + ".import.lock")).open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError(f"Another SDK import is active for {destination}") from None
        return import_locked(args)


def sdk_host_architecture(bundle):
    # xtool wraps the linker in a shell script. Older receipts omit the host;
    # inspect the actual ELF behind the wrapper before reusing those SDKs.
    for relative in ("toolset/bin/orig/ld64.lld", "toolset/bin/ld64.lld"):
        try:
            with (bundle / relative).open("rb") as source:
                header = source.read(20)
        except OSError:
            continue
        if header[:6] == b"\x7fELF\x02\x01":
            return {62: "x86_64", 183: "aarch64"}.get(int.from_bytes(header[18:20], "little"))
    return None


def import_locked(args):
    archive = args.archive.resolve()
    inspected = inspect_archive(archive)
    checksum = inspected["archiveSha256"]
    source_format = inspected.get("sourceFormat", "sweetpad-export")
    destination = args.destination.absolute()
    if destination.is_symlink():
        raise ValueError("SDK destination must not be a symbolic link")
    if destination.exists() and not (destination / "swift-sdk.json").is_file():
        raise ValueError("Existing destination is not a Darwin SDK; select an empty destination")
    receipt_path = destination / "sweetpad-sdk.json"
    receipt = json.loads(receipt_path.read_text()) if receipt_path.is_file() else {}
    required = ("swift-sdk.json", "toolset-swb.json", "toolset/bin/ld64.lld", "version.plist",
                "Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/clang/include/stddef.h")
    if source_format == "sweetpad-export" or receipt.get("localizedAppIntents"):
        required += ("appintents-phrases.json",)
    recipe_current = receipt.get("importerVersion") == IMPORTER_VERSION and receipt.get("toolchain") == VERSIONS
    if destination.exists() and receipt.get("archiveSha256") == checksum and receipt.get("hostArchitecture", sdk_host_architecture(destination)) == platform.machine() and recipe_current and not args.replace and all((destination / p).is_file() for p in required):
        return {"sdkPath": str(destination), "archiveSha256": checksum, "importerVersion": IMPORTER_VERSION, "reused": True}
    if destination.exists() and not args.replace:
        if receipt.get("archiveSha256") == checksum and not recipe_current:
            raise ValueError("SDK import recipe or toolchain changed; use --replace with sdk import, or --replace-sdk with setup, to rebuild the existing SDK")
        raise ValueError("SDK destination already exists; use --replace with sdk import, or --replace-sdk with setup, to replace it after a successful import")
    xtool = args.xtool or shutil.which("xtool") or str(Path.home() / "bin/xtool")
    if subprocess.check_output([xtool, "--version"], text=True).strip() != "xtool " + VERSIONS["xtool"]:
        raise ValueError("SDK import requires xtool " + VERSIONS["xtool"] + "; run sweetpad setup --sdk ARCHIVE")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".sweetpad-sdk-", dir=destination.parent) as work:
        work = Path(work)
        require_space(work, inspected["requiredFreeBytes"])
        manifest = unpack_verified(archive, work) if source_format == "sweetpad-export" else unpack_xcode_archive(archive, work, args)
        subprocess.run([xtool, "sdk", "build", str(work / "Xcode.app"), str(work)],
                       check=True, stdout=sys.stderr)
        bundle = work / "darwin.xtoolsdk"
        supplement(work / "Xcode.app", bundle, work / "appintents-phrases.json")
        receipt = {"archiveSha256": checksum, "importerVersion": IMPORTER_VERSION, "xcodeBuild": manifest["xcodeBuild"],
                   "xcodeVersion": manifest["xcodeVersion"], "toolchain": VERSIONS, "hostArchitecture": platform.machine(),
                   "sourceFormat": source_format, "localizedAppIntents": (bundle / "appintents-phrases.json").is_file()}
        (bundle / "sweetpad-sdk.json").write_text(json.dumps(receipt, indent=2) + "\n")
        backup = work / "previous-sdk"
        if destination.exists():
            destination.rename(backup)
        try:
            bundle.rename(destination)
        except BaseException:
            if backup.exists():
                backup.rename(destination)
            raise
    return {"sdkPath": str(destination), **receipt, "reused": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    export = commands.add_parser("export")
    export.add_argument("--xcode", type=Path, default=Path("/Applications/Xcode.app"))
    export.add_argument("--output", type=Path, required=True)
    export.add_argument("--dry-run", action="store_true")
    install = commands.add_parser("import")
    install.add_argument("--archive", type=Path, required=True)
    install.add_argument("--destination", type=Path, default=Path.home() / ".swiftpm/swift-sdks/darwin.artifactbundle")
    install.add_argument("--xtool")
    install.add_argument("--replace", action="store_true")
    install.add_argument("--aea-key-file", type=Path)
    install.add_argument("--aea-public-key-file", type=Path)
    inspect = commands.add_parser("inspect")
    inspect.add_argument("--archive", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.action == "export":
            report = export_sdk(args)
        elif args.action == "inspect":
            report = inspect_archive(args.archive)
        else:
            report = import_sdk(args)
        print(json.dumps(report))
    except (OSError, ValueError, KeyError, subprocess.SubprocessError, tarfile.TarError) as error:
        print(json.dumps({"error": str(error)}))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
