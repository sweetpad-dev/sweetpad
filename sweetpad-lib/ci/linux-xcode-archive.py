"""Select SDK inputs from XIP/Apple Archives without installing Xcode.

Two bounded passes retain hard links whose data lies outside the SDK selection.
No archive entry is executed. Files are staged before symlinks are created.
"""
import hashlib
import importlib.util
import functools
import os
from pathlib import Path, PurePosixPath
import posixpath
import shutil
import stat
import struct
import xml.etree.ElementTree as ET
import zlib


@functools.lru_cache(maxsize=8)
def module(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


CODECS = module("linux-archive-codecs")
MAX_OUTPUT = 64 * 1024**3
MAX_ENTRIES = 1_000_000


class Reader:
    def __init__(self, chunks):
        self.chunks = iter(chunks)
        self.pending = memoryview(b"")
        self.prefix = b""
        self.pos = 0

    def read(self, size):
        parts = []
        if self.prefix and size:
            count = min(size, len(self.prefix))
            parts.append(self.prefix[:count]); self.prefix = self.prefix[count:]; size -= count
        while size:
            if not self.pending:
                try:
                    self.pending = memoryview(next(self.chunks))
                except StopIteration:
                    break
                if not self.pending:
                    continue
            count = min(size, len(self.pending))
            parts.append(self.pending[:count]); self.pending = self.pending[count:]; size -= count
        result = b"".join(parts)
        self.pos += len(result)
        if self.pos > MAX_OUTPUT:
            raise ValueError("Xcode archive exceeds the 64 GiB decoded-size limit")
        return result

    def exact(self, size):
        value = self.read(size)
        if len(value) != size:
            raise ValueError("Truncated Xcode archive")
        return value

    def skip(self, size):
        if size < 0:
            raise ValueError("Invalid archive offset")
        while size:
            count = min(size, 1024 * 1024)
            self.exact(count); size -= count

    def unread(self, data):
        self.prefix = data + self.prefix
        self.pos -= len(data)

    def drain(self, zero_padding=False):
        while True:
            value = self.read(1024 * 1024)
            if not value:
                break
            if zero_padding and value.strip(b"\0"):
                raise ValueError("Unexpected data after the CPIO trailer")


def pbz_chunks(source):
    header = source.exact(12)
    magic = header[:4]
    if magic not in (b"pbzx", b"pbzm", b"pbze", b"pbzz", b"pbz4"):
        raise ValueError("Unsupported Apple archive compression container")
    block_size = int.from_bytes(header[4:12], "big")
    if not 0 < block_size <= CODECS.MAX_BLOCK:
        raise ValueError("Invalid Apple archive compression block size")
    algorithm = chr(magic[3])
    total = 0
    while True:
        data = source.read(16)
        if not data:
            break
        if len(data) != 16:
            raise ValueError("Truncated Apple archive compression header")
        original, packed = struct.unpack(">QQ", data)
        if not 0 < original <= block_size or not 0 < packed <= CODECS.MAX_BLOCK:
            raise ValueError("Invalid Apple archive compression range")
        block = source.exact(packed)
        block = block if packed == original else CODECS.decompress(algorithm, block, original)
        total += len(block)
        if total > MAX_OUTPUT:
            raise ValueError("Xcode archive exceeds the 64 GiB decoded-size limit")
        yield block


def file_chunks(source):
    while True:
        value = source.read(1024 * 1024)
        if not value:
            break
        yield value


def xar_content(path):
    """Read the XAR table and verify the stored Content stream's checksum."""
    with Path(path).open("rb") as source:
        header = source.read(28)
        if len(header) != 28:
            raise ValueError("Truncated XIP header")
        magic, header_size, version, packed_size, toc_size, _ = struct.unpack(">4sHHQQI", header)
        if magic != b"xar!" or version != 1 or not 28 <= header_size <= 4096:
            raise ValueError("Unsupported XIP/XAR header")
        if not 0 < packed_size <= 16 * 1024**2 or not 0 < toc_size <= 16 * 1024**2:
            raise ValueError("XIP table exceeds the memory limit")
        source.seek(header_size)
        packed_toc = source.read(packed_size)
        if len(packed_toc) != packed_size:
            raise ValueError("Truncated XIP table")
        decoder = zlib.decompressobj()
        try:
            toc = decoder.decompress(packed_toc, toc_size + 1)
        except zlib.error:
            raise ValueError("Invalid XIP table compression") from None
        if len(toc) != toc_size or not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
            raise ValueError("Invalid XIP table compression")
        try:
            root = ET.fromstring(toc)
        except ET.ParseError:
            raise ValueError("Invalid XIP table XML") from None
        contents = [entry for entry in root.findall("./toc/file") if entry.findtext("name") == "Content"]
        if len(contents) != 1:
            raise ValueError("XIP must contain exactly one Content payload")
        data = contents[0].find("data")
        if data is None:
            raise ValueError("Invalid XIP Content metadata")
        encoding = data.find("encoding")
        if encoding is not None and encoding.get("style") != "application/octet-stream":
            raise ValueError("XIP Content uses unsupported XAR-level compression")
        offset, length = int(data.findtext("offset", "-1")), int(data.findtext("length", "-1"))
        heap = header_size + packed_size
        if offset < 0 or length < 12 or heap + offset + length > Path(path).stat().st_size:
            raise ValueError("XIP Content range is outside the archive")
        checksum = data.find("archived-checksum")
        if checksum is None or checksum.get("style") not in ("sha1", "sha256", "sha512"):
            raise ValueError("XIP Content must have a supported checksum")
        digest = hashlib.new(checksum.get("style"))
        expected = (checksum.text or "").strip().lower()
        source.seek(heap + offset)
        remaining = length
        while remaining:
            block = source.read(min(remaining, 1024 * 1024))
            if not block:
                raise ValueError("Truncated XIP Content")
            digest.update(block); remaining -= len(block)
            yield block
        if digest.hexdigest() != expected:
            raise ValueError("XIP Content checksum failed")


def stream(path, key_file=None, public_key_file=None):
    path = Path(path)
    with path.open("rb") as source:
        magic = source.read(4); source.seek(0)
        if magic == b"xar!":
            raw = Reader(xar_content(path))
        elif magic == b"AEA1":
            raw = Reader(module("linux-aea").chunks(path, key_file, public_key_file))
        else:
            raw = Reader(file_chunks(source))
        for _ in range(3):
            magic = raw.read(4); raw.unread(magic)
            if magic.startswith(b"pbz"):
                raw = Reader(pbz_chunks(raw))
            else:
                break
        # Keep the underlying file/context alive until the returned iterator ends.
        while True:
            data = raw.read(1024 * 1024)
            if not data:
                break
            yield data


def clean_name(name):
    if not isinstance(name,str) or "\0" in name or name.startswith("/") or ".." in PurePosixPath(name).parts:
        raise ValueError("Unsafe Xcode archive path: " + repr(name))
    return "/".join(part for part in PurePosixPath(name).parts if part != ".")


def cpio_entries(source):
    while True:
        prefix = source.exact(6)
        if prefix == b"070707":
            header = prefix + source.exact(70)
            device, inode, mode, _, _, links, _ = [int(header[a:b], 8) for a, b in [(6,12),(12,18),(18,24),(24,30),(30,36),(36,42),(42,48)]]
            name_size, size = int(header[59:65], 8), int(header[65:76], 8)
            alignment = 1; key = (device, inode)
        elif prefix in (b"070701", b"070702"):
            header = prefix + source.exact(104)
            fields = [int(header[pos:pos + 8], 16) for pos in range(6, 110, 8)]
            inode, mode, _, _, links, _, size, major, minor, _, _, name_size, _ = fields
            alignment = 4; key = (major, minor, inode)
            if prefix == b"070702":
                raise ValueError("CRC CPIO is not an Xcode XIP payload format")
        else:
            raise ValueError("Unsupported XIP CPIO header")
        if not 0 < name_size <= 65536 or size > MAX_OUTPUT:
            raise ValueError("Invalid XIP CPIO entry size")
        name = source.exact(name_size)
        if not name.endswith(b"\0"):
            raise ValueError("XIP entry name is not terminated")
        name = clean_name(name[:-1].decode("utf-8"))
        source.skip((-source.pos) % alignment)
        if name == "TRAILER!!!":
            if size:
                raise ValueError("Invalid CPIO trailer")
            source.drain(zero_padding=True)
            return
        offset = source.pos
        kind = {stat.S_IFREG: "file", stat.S_IFDIR: "dir", stat.S_IFLNK: "symlink"}.get(stat.S_IFMT(mode), "special")
        target = None
        if kind == "symlink":
            if size > 65536:
                raise ValueError("XIP symlink target is too large")
            target = source.exact(size).decode("utf-8")
        else:
            source.skip(size)
        source.skip((-source.pos) % alignment)
        yield {"name": name, "kind": kind, "size": size, "offset": offset, "mode": mode & 0o777,
               "link": target, "group": key if kind == "file" and links > 1 else None}


def aa_entries(source):
    while True:
        prefix = source.read(6)
        if not prefix:
            return
        if len(prefix) != 6 or prefix[:4] not in (b"AA01", b"YAA1"):
            raise ValueError("Unsupported Apple Archive entry header")
        size = int.from_bytes(prefix[4:6], "little")
        if size < 6:
            raise ValueError("Invalid Apple Archive header length")
        header = source.exact(size - 6)
        fields = {}; blobs = []; pos = 0
        while pos < len(header):
            if pos + 4 > len(header):
                raise ValueError("Truncated Apple Archive field")
            key = header[pos:pos + 3].decode("ascii"); subtype = chr(header[pos + 3]); pos += 4
            if key in fields:
                raise ValueError("Duplicate Apple Archive field")
            if subtype == "*":
                fields[key] = True; continue
            length = {"1":1,"2":2,"4":4,"8":8,"F":4,"G":20,"H":32,"I":48,"J":64,"S":8,"T":12,"A":2,"B":4,"C":8,"P":2}.get(subtype)
            if length is None or pos + length > len(header):
                raise ValueError("Invalid Apple Archive field encoding")
            data = header[pos:pos + length]; pos += length
            if subtype == "P":
                length = int.from_bytes(data, "little")
                if pos + length > len(header):
                    raise ValueError("Truncated Apple Archive string")
                fields[key] = header[pos:pos + length].rstrip(b"\0").decode("utf-8"); pos += length
            elif subtype in "ABC":
                length = int.from_bytes(data, "little")
                if length > MAX_OUTPUT:
                    raise ValueError("Apple Archive blob exceeds the size limit")
                fields[key] = length; blobs.append((key, length))
            elif subtype in "1248":
                fields[key] = int.from_bytes(data, "little")
            else:
                fields[key] = data
        offsets = {}
        for key, length in blobs:
            offsets[key] = source.pos
            source.skip(length)
        kind = {ord("F"):"file",ord("D"):"dir",ord("L"):"symlink",ord("H"):"hardlink",ord("M"):"metadata"}.get(fields.get("TYP"), "special")
        if kind == "metadata":
            continue
        name = clean_name(fields.get("PAT", ""))
        if not isinstance(fields.get("MOD",0),int) or ("DAT" in fields and "DAT" not in offsets):
            raise ValueError("Invalid Apple Archive file attributes")
        size = fields.get("DAT", 0)
        if kind == "file" and ("SLC" in fields or fields.get("SIZ", size) != size):
            raise ValueError("Sparse or sliced Apple Archive files are unsupported")
        yield {"name":name,"kind":kind,"size":size,"offset":offsets.get("DAT",source.pos),
               "mode":fields.get("MOD",0o755 if kind=="dir" else 0o644) & 0o777,
               "link":fields.get("LNK"),"group":None}


def selected(name):
    parts = PurePosixPath(name).parts
    if not parts or not parts[0].endswith(".app") or "prebuilt-modules" in parts:
        return None
    relative = "/".join(parts[1:])
    if relative == "Contents/version.plist":
        return "Xcode.app/" + relative
    base = "Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/"
    if relative.startswith(base):
        suffix = relative[len(base):]
        if suffix == "include/swift" or suffix.startswith("include/swift/") or any(suffix == "lib/" + p or suffix.startswith("lib/" + p + "/") for p in ["swift","swift_static","clang"]) or suffix.startswith("lib/swift-"):
            return "Xcode.app/" + relative
    platform = "Contents/Developer/Platforms/"
    if relative.startswith(platform):
        remainder = relative[len(platform):].split("/",1)
        if len(remainder) == 2 and remainder[0].endswith(".platform"):
            prefix = remainder[1]
            roots = ["Info.plist","version.plist","PlatformBasicInfo.plist","Developer/SDKs","Developer/Library/Frameworks",
                     "Developer/Library/PrivateFrameworks","Developer/Library/Xcode/Specifications",
                     "Developer/Library/Xcode/Agents/XCTRunner.app","Developer/usr/lib","Library/Application Support"]
            if any(prefix == root or prefix.startswith(root + "/") for root in roots):
                return "Xcode.app/" + relative
    return None


def extract(path, destination, key_file=None, public_key_file=None):
    destination = Path(destination)
    source = Reader(stream(path, key_file, public_key_file))
    magic = source.read(6); source.unread(magic)
    parser = cpio_entries if magic.startswith(b"0707") else aa_entries
    entries = {}; groups = {}; app_names = set()
    for entry in parser(source):
        if len(entries) >= MAX_ENTRIES:
            raise ValueError("Xcode archive entry count exceeds the limit")
        name = entry["name"]
        if name in entries and not (entry["kind"] == entries[name]["kind"] == "dir"):
            raise ValueError("Duplicate Xcode archive path: " + name)
        entries[name] = entry
        if selected(name):
            app_names.add(PurePosixPath(name).parts[0])
        if entry["group"] is not None and entry["size"]:
            previous = groups.setdefault(entry["group"], entry)
            if previous["size"] != entry["size"]:
                raise ValueError("Inconsistent XIP hard link sizes")
    source.drain()
    if len(app_names) != 1:
        raise ValueError("Xcode archive must contain one app with SDK inputs")
    wanted = [entry for entry in entries.values() if selected(entry["name"])]
    regular = []; links = []; directories = []

    def data_entry(entry):
        visited = set()
        while entry["kind"] == "hardlink":
            if entry["name"] in visited:
                raise ValueError("Cyclic Apple Archive hard link")
            visited.add(entry["name"])
            target = clean_name(entry["link"] or "")
            if target not in entries:
                raise ValueError("Missing Apple Archive hard link target")
            entry = entries[target]
        if entry["kind"] != "file":
            raise ValueError("Archive hard link target is not a regular file")
        if entry["group"] is not None:
            entry = groups.get(entry["group"], entry)
        return entry

    for entry in wanted:
        name = selected(entry["name"])
        target = destination / name
        if entry["kind"] in ("file","hardlink"):
            data = data_entry(entry)
            regular.append((data["offset"],data["size"],target,entry["mode"]))
        elif entry["kind"] == "dir":
            directories.append((target,entry["mode"]))
        elif entry["kind"] == "symlink":
            link = entry["link"]
            if not isinstance(link,str) or "\0" in link or link.startswith("/"):
                raise ValueError("Unsafe Xcode symbolic link")
            resolved = posixpath.normpath(posixpath.join(posixpath.dirname(name), link))
            if not resolved.startswith("Xcode.app/"):
                raise ValueError("Xcode symbolic link escapes the app")
            links.append((target,link))
        else:
            raise ValueError("Unsupported special file in selected Xcode inputs")
    # File-vs-directory and symlink-parent collisions must fail before writing.
    leaves = {str(target) for _,_,target,_ in regular} | {str(target) for target,_ in links}
    for target in [p for _,_,p,_ in regular] + [p for p,_ in links] + [p for p,_ in directories]:
        if any(str(parent) in leaves for parent in target.parents):
            raise ValueError("Conflicting Xcode archive parent path")
    total = sum(size for _,size,_,_ in regular)
    destination.mkdir(parents=True,exist_ok=True)
    app = destination / "Xcode.app"
    if app.exists() or app.is_symlink():
        raise ValueError("Extract Xcode inputs into an empty staging directory")
    if shutil.disk_usage(destination).free < 2 * total + 512 * 1024**2:
        raise ValueError("Not enough disk space for the extracted Xcode inputs and staged SDK")
    for target,_ in directories:
        target.mkdir(parents=True,exist_ok=True)
    # Group identical data ranges; hard-linked files can appear before their data.
    ranges = {}
    for offset,size,target,mode in regular:
        ranges.setdefault((offset,size),[]).append((target,mode))
    source = Reader(stream(path,key_file,public_key_file))
    for (offset,size),targets in sorted(ranges.items()):
        source.skip(offset - source.pos)
        target,mode = targets[0]
        target.parent.mkdir(parents=True,exist_ok=True)
        with target.open("xb") as output:
            remaining = size
            while remaining:
                block = source.exact(min(remaining,1024*1024)); output.write(block); remaining -= len(block)
        target.chmod(mode)
        for alias,alias_mode in targets[1:]:
            alias.parent.mkdir(parents=True,exist_ok=True)
            # Copies preserve distinct modes without changing the shared inode.
            shutil.copyfile(target,alias); alias.chmod(alias_mode)
    source.drain()
    for target,link in links:
        target.parent.mkdir(parents=True,exist_ok=True)
        target.symlink_to(link)
    for target,_ in links:
        try:
            target.resolve().relative_to(app.resolve())
        except (ValueError, RuntimeError):
            raise ValueError("Xcode symbolic link resolves outside the app or contains a cycle") from None
    for target,mode in reversed(directories):
        target.chmod(mode | 0o700)
    return destination / "Xcode.app"
