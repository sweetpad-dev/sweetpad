"""Semantic comparisons for native and Linux Apple app bundles."""
import hashlib
import datetime
import json
from pathlib import Path
import plistlib
import struct

import importlib.util
_spec = importlib.util.spec_from_file_location("appintents_model", Path(__file__).with_name("appintents_model.py"))
appintents_model = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(appintents_model)


def differences(a, b, path=""):
    if type(a) is not type(b):
        return [{"path": path, "native": a, "linux": b}]
    if isinstance(a, dict):
        result = []
        for key in sorted(a.keys() | b.keys()):
            if key not in a or key not in b:
                result.append({"path": path + "/" + key, "native": a.get(key), "linux": b.get(key)})
            else:
                result.extend(differences(a[key], b[key], path + "/" + key))
        return result
    if isinstance(a, list):
        if len(a) != len(b):
            return [{"path": path + "/length", "native": len(a), "linux": len(b)}]
        return [d for i, (x, y) in enumerate(zip(a, b)) for d in differences(x, y, path + "/" + str(i))]
    return [] if a == b else [{"path": path, "native": a, "linux": b}]


def normalize_plist(value):
    if isinstance(value, dict):
        return {k: normalize_plist(v) for k, v in value.items() if k != "BuildMachineOSBuild"}
    if isinstance(value, list):
        return [normalize_plist(v) for v in value]
    if isinstance(value, bytes):
        return {"dataSha256": hashlib.sha256(value).hexdigest()}
    if isinstance(value, plistlib.UID):
        return {"uid": value.data}
    if isinstance(value, datetime.datetime):
        return value.isoformat()
    return value


def normalize_registration(value):
    if isinstance(value, dict):
        result = {k: normalize_registration(v) for k, v in value.items() if k != "generator"}
        if "resolvableInputTypes" in result:
            result["resolvableInputTypes"].sort(key=lambda v: json.dumps(v, sort_keys=True))
        return result
    if isinstance(value, list):
        return [normalize_registration(v) for v in value]
    return value


def ignored_layout(path):
    p = Path(path)
    return "_CodeSignature" in p.parts or p.name == "__preview.dylib" or p.name.endswith(".debug.dylib")


def version(value):
    return f"{value >> 16}.{(value >> 8) & 255}.{value & 255}"


def macho(path):
    """Read load commands, without comparing addresses, UUIDs, or code signatures."""
    with path.open("rb") as source:
        magic = source.read(4)
        slices = [0]
        if magic in [b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"]:
            count = struct.unpack(">I", source.read(4))[0]
            wide = magic[-1] == 0xbf
            slices = []
            for _ in range(count):
                arch = source.read(32 if wide else 20)
                slices.append(struct.unpack_from(">Q" if wide else ">I", arch, 8)[0])
        result = {}
        for offset in slices:
            source.seek(offset)
            header = source.read(32)
            if header[:4] != b"\xcf\xfa\xed\xfe":
                continue
            _, cpu, subtype, _, commands, size, _, _ = struct.unpack("<IiiIIIII", header)
            arch = {0x100000c: "arm64", 0x1000007: "x86_64"}.get(cpu, str(cpu))
            if arch == "arm64" and subtype & 0xffffff == 2:
                arch = "arm64e"
            data = source.read(size)
            cursor = 0
            entry = {"buildVersions": [], "rpaths": [], "libraries": []}
            for _ in range(commands):
                command, length = struct.unpack_from("<II", data, cursor)
                if length < 8 or cursor + length > len(data):
                    raise ValueError(f"invalid Mach-O load command in {path}")
                record = data[cursor:cursor + length]
                if command == 0x32:
                    platform, minimum, sdk = struct.unpack_from("<III", record, 8)
                    entry["buildVersions"].append({"platform": platform, "minimum": version(minimum), "sdk": version(sdk)})
                elif command == 0x8000001c:
                    start = struct.unpack_from("<I", record, 8)[0]
                    entry["rpaths"].append(record[start:].split(b"\0", 1)[0].decode())
                elif command in [0xc, 0x80000018, 0x8000001f, 0x80000023]:
                    start, _, current, compatible = struct.unpack_from("<IIII", record, 8)
                    entry["libraries"].append({"path": record[start:].split(b"\0", 1)[0].decode(),
                                               "kind": command, "current": version(current), "compatible": version(compatible)})
                cursor += length
            entry["libraries"].sort(key=lambda v: (v["path"], v["kind"]))
            result[arch] = entry
        return result


def executable(root, info_path, info):
    name = info.get("CFBundleExecutable")
    if not isinstance(name, str):
        return None
    base = (root / info_path).parent
    for path in [base / name, base / "MacOS" / name, base.parent / name]:
        if path.is_file():
            debug = path.with_name(path.name + ".debug.dylib")
            return debug if debug.is_file() else path
    return None


def compare(native, linux, read_plist, build_roots=None):
    def normalized_binary(value, host):
        root = str((build_roots or {}).get(host, "")).rstrip("/")
        result = json.loads(json.dumps(value))
        for entry in result.values():
            if root:
                entry["rpaths"] = [
                    "__PRODUCTS__" + p[len(root):] if p == root or p.startswith(root + "/") else p
                    for p in entry["rpaths"]]
            entry["libraries"] = {v["path"]: v for v in entry["libraries"]}
        return result
    def files(root):
        return {str(p.relative_to(root)) for p in root.rglob("*") if p.is_file()}
    a, b = files(native), files(linux)
    findings = []
    shortcut_models = []
    # Fingerprints identify generated content, not files with independent app
    # resources. Validate each marker against its model before normalizing its
    # name; compare every decoded locale, intent phrase and negative utterance.
    for path in sorted(p for p in a & b if Path(p).name == "nlu.lzfse"):
        try:
            av, amarker = appintents_model.read(native / path)
            bv, bmarker = appintents_model.read(linux / path)
            delta = differences(av, bv)
            shortcut_models.append({"file": path, "differences": delta})
            if delta:
                findings.append({"kind": "appShortcutModel", "file": path, "differences": delta})
            else:
                a.remove(str(amarker.relative_to(native)))
                b.remove(str(bmarker.relative_to(linux)))
                marker = str(Path(path).parent / "__content_fingerprint__.version")
                a.add(marker)
                b.add(marker)
        except (OSError, ValueError, UnicodeError, struct.error) as error:
            findings.append({"kind": "appShortcutModel", "file": path, "error": str(error)})
    common = a & b
    for path in sorted(a ^ b):
        if not ignored_layout(path):
            findings.append({"kind": "missingFile" if path in a else "extraFile", "file": path})
    strings = sorted(p for p in common if Path(p).suffix in [".strings", ".stringsdict"])
    changed = []
    for path in strings:
        if read_plist(native / path) != read_plist(linux / path):
            changed.append(path)
            findings.append({"kind": "localization", "file": path})
    info_paths = sorted(p for p in common if Path(p).name == "Info.plist")
    plist_differences = []
    binaries = {}
    for path in info_paths:
        ai, bi = read_plist(native / path), read_plist(linux / path)
        delta = differences(normalize_plist(ai), normalize_plist(bi))
        if delta:
            plist_differences.append({"file": path, "differences": delta})
            findings.append({"kind": "bundleInfo", "file": path, "differences": delta})
        an, bn = executable(native, path, ai), executable(linux, path, bi)
        if an and bn:
            am, bm = macho(an), macho(bn)
            delta = differences(normalized_binary({k: am[k] for k in am.keys() & bm.keys()}, "xcode"),
                                normalized_binary({k: bm[k] for k in am.keys() & bm.keys()}, "linux"))
            binaries[path] = {"native": am, "linux": bm, "differences": delta,
                              "onlyNativeArchitectures": sorted(am.keys() - bm.keys()),
                              "onlyLinuxArchitectures": sorted(bm.keys() - am.keys())}
            if delta or not am.keys() & bm.keys():
                findings.append({"kind": "machO", "file": path, "differences": delta})
    metadata = []
    for path in sorted(p for p in common if p.endswith("Metadata.appintents/extract.actionsdata")):
        av = normalize_registration(json.loads((native / path).read_text()))
        bv = normalize_registration(json.loads((linux / path).read_text()))
        delta = differences(av, bv)
        metadata.append({"file": path, "differences": delta})
        if delta:
            findings.append({"kind": "appIntents", "file": path, "differences": delta})
    text_differences = []
    for path in sorted(p for p in common if Path(p).suffix in [".txt", ".html", ".css"]):
        if (native / path).read_bytes() != (linux / path).read_bytes():
            text_differences.append(path)
            findings.append({"kind": "resourceContent", "file": path})
    ai, bi = read_plist(native / "Info.plist"), read_plist(linux / "Info.plist")
    fields = ["CFBundleIdentifier", "CFBundleExecutable", "CFBundleVersion", "CFBundleShortVersionString"]
    return {"nativeFiles": len(a), "linuxFiles": len(b), "onlyNative": sorted(a - b), "onlyLinux": sorted(b - a),
            "localizationFilesCompared": len(strings), "differentLocalizations": changed,
            "applicationPropertiesMatch": all(ai.get(k) == bi.get(k) for k in fields),
            "bundleInfoDifferences": plist_differences, "machO": binaries, "appIntents": metadata,
            "appShortcutModels": shortcut_models, "differentTextResources": text_differences, "findings": findings, "passed": not findings,
            "scope": "File inventory, bundle Info.plists, localization values, load commands, App Intents registration, decoded shortcut phrase models with validated content-fingerprint markers, and loose text resources. Asset archives require runtime loading checks; code signatures and UUIDs are not compared."}
