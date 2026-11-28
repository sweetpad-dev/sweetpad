"""Link typed LLVM AIR modules into a Metal library."""

import hashlib
import re
import struct
import subprocess
from dataclasses import dataclass


KINDS = {
    "vertex": 0,
    "fragment": 1,
    "kernel": 2,
    "visible": 4,
    "intersection": 6,
    "mesh": 7,
    "object": 8,
}
HEADER = struct.Struct("<4sHHHBBHHQ8Q")
TARGET = re.compile(r"air64(?:_v\d+)?-apple-(macosx|ios)(\d+)(?:\.(\d+))?(?:\.\d+)?(-simulator)?")


def wrapped(bitcode):
    if bitcode.startswith(b"\xde\xc0\x17\x0b"):
        _, version, offset, size, _ = struct.unpack_from("<5I", bitcode)
        if version or offset + size > len(bitcode):
            raise ValueError("invalid AIR bitcode wrapper")
        bitcode = bitcode[offset : offset + size]
    if not bitcode.startswith(b"BC\xc0\xde"):
        raise ValueError("expected LLVM bitcode")
    blob = struct.pack("<5I", 0x0B17C0DE, 0, 20, len(bitcode), 0xFFFFFFFF) + bitcode
    return blob + bytes(-len(blob) % 16)


@dataclass(frozen=True)
class Module:
    target: str
    versions: tuple
    entries: list
    bitcode: bytes

    @classmethod
    def read(cls, path, disassembler):
        ir = subprocess.check_output([disassembler, str(path), "-o", "-"], text=True)
        nodes = dict(re.findall(r"^!(\d+) = !\{(.*)\}$", ir, re.M))

        def named(key):
            line = re.search(r"^!air\." + key + r" = !\{([^}]*)\}", ir, re.M)
            return [nodes[n] for n in re.findall(r"!(\d+)", line[1])] if line else []

        versions = []
        for key in ("version", "language_version"):
            data = named(key)
            if len(data) != 1:
                raise ValueError(f"{path}: missing AIR {key}")
            versions.extend(int(n) for n in re.findall(r"i32 (\d+)", data[0])[:2])
        entries = []
        for kind, code in KINDS.items():
            for node in named(kind):
                symbol = re.search(r"@([A-Za-z_$][\w.$]*),", node)
                if not symbol:
                    raise ValueError(f"{path}: unsupported AIR entry name")
                public = b""
                if kind == "vertex":
                    inputs = re.search(r"@[^,]+, !\d+, !(\d+)", node)
                    if not inputs:
                        raise ValueError(f"{path}: missing vertex inputs")
                    arguments = [nodes[n] for n in re.findall(r"!(\d+)", nodes[inputs[1]])]
                    public = vertex_attributes(arguments)
                entries.append((symbol[1], code, public))
        triple = re.search(r'^target triple = "([^"]+)"', ir, re.M)
        if not triple or not TARGET.fullmatch(triple[1]) or not entries or len(versions) != 4:
            raise ValueError(f"{path}: expected an AIR module with entry points")
        return cls(triple[1], tuple(versions), entries, wrapped(path.read_bytes()))


def tag(name, data):
    return name + struct.pack("<H", len(data)) + data


def record(data):
    return struct.pack("<I", len(data) + 8) + data + b"ENDT"


def vertex_attributes(arguments):
    # Metal reflection uses the scalar MTLDataType followed by vector width.
    bases = {
        "float": 3,
        "half": 16,
        "int": 29,
        "uint": 33,
        "short": 37,
        "ushort": 41,
        "char": 45,
        "uchar": 49,
    }
    names, types, slots = bytearray(), bytearray(), set()
    for arg in arguments:
        if '!"air.vertex_input"' not in arg:
            continue
        location = re.search(r'!"air.location_index", i32 (\d+), i32 1', arg)
        typ = re.search(r'!"air.arg_type_name", !"([a-z]+)([234]?)"', arg)
        name = re.search(r'!"air.arg_name", !"(\w+)"', arg)
        if not location or not typ or not name or typ[1] not in bases:
            raise ValueError("unsupported vertex attribute descriptor")
        slot = int(location[1])
        if slot in slots or not 0 <= slot < 31:
            raise ValueError("duplicate or invalid vertex attribute location")
        slots.add(slot)
        names += name[1].encode() + b"\0" + struct.pack("<H", slot | 0x8000)
        types.append(bases[typ[1]] + int(typ[2] or 1) - 1)
    count = struct.pack("<H", len(slots))
    return tag(b"VATT", count + names) + tag(b"VATY", count + types) if slots else b""


def library(modules):
    first = modules[0]
    target = TARGET.fullmatch(first.target)
    mac = target[1] == "macosx"
    platform = 0x8001 if mac else 1
    os = 0x81 if mac else 0x87 if target[4] else 0x82
    major, minor = int(target[2]), int(target[3] or 0)
    names = set()
    info, public, private, code = (bytearray() for _ in range(4))
    for module in modules:
        if module.target != first.target or module.versions[:2] != first.versions[:2]:
            raise ValueError("cannot link AIR modules for different deployment targets")
        code_offset = len(code)
        code.extend(module.bitcode)
        digest = hashlib.sha256(module.bitcode).digest()
        for name, kind, attributes in module.entries:
            if name in names:
                raise ValueError(f"duplicate Metal function '{name}'")
            names.add(name)
            tags = (
                tag(b"NAME", name.encode() + b"\0")
                + tag(b"TYPE", bytes([kind]))
                + tag(b"HASH", digest)
                + tag(b"OFFT", struct.pack("<3Q", len(public), len(private), code_offset))
                + tag(b"VERS", struct.pack("<4H", *module.versions))
                + tag(b"MDSZ", struct.pack("<Q", len(module.bitcode)))
            )
            info.extend(record(tags))
            public.extend(record(attributes))
            private.extend(record(b""))
    table = struct.pack("<I", len(names)) + info + b"ENDT"
    a = HEADER.size
    b = a + len(table)
    c = b + len(public)
    d = c + len(private)
    header = HEADER.pack(
        b"MTLB",
        platform,
        2,
        6,
        0,
        os,
        major,
        minor,
        d + len(code),
        a,
        len(info),
        b,
        len(public),
        c,
        len(private),
        d,
        len(code),
    )
    return header + table + public + private + code
