"""Decode the SSUv1 FlatBuffer used by App Shortcut models for parity checks."""
import ctypes
from pathlib import Path
import struct


def decompress(path):
    data = Path(path).read_bytes()
    if data.startswith(b"bvx-"):
        length = struct.unpack_from("<I", data, 4)[0]
        if len(data) == length + 12 and data[-4:] == b"bvx$":
            return data[8:-4]
    # Native output uses compressed LZFSE blocks. These checks run on macOS.
    library = ctypes.CDLL("/usr/lib/libcompression.dylib")
    decode = library.compression_decode_buffer
    decode.restype = ctypes.c_size_t
    decode.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p,
                       ctypes.c_size_t, ctypes.c_void_p, ctypes.c_int]
    output = ctypes.create_string_buffer(16 * 1024 * 1024)
    count = decode(output, len(output), data, len(data), None, 0x801)
    if not count:
        raise ValueError(f"invalid shortcut LZFSE model: {path}")
    return output.raw[:count]


class Model:
    def __init__(self, data):
        self.data = data

    def number(self, format, at):
        if at is None or at < 0 or at + struct.calcsize(format) > len(self.data):
            raise ValueError("invalid shortcut model offset")
        return struct.unpack_from(format, self.data, at)[0]

    def field(self, at, index):
        table = at - self.number("<i", at)
        size = self.number("<H", table)
        if 4 + index * 2 >= size:
            return None
        offset = self.number("<H", table + 4 + index * 2)
        return at + offset if offset else None

    def indirect(self, at):
        return None if at is None else at + self.number("<I", at)

    def reference(self, at, index):
        return self.indirect(self.field(at, index))

    def string(self, at):
        count = self.number("<I", at)
        if at + 4 + count > len(self.data):
            raise ValueError("invalid shortcut model string")
        return self.data[at + 4:at + 4 + count].decode("utf-8")

    def vector(self, at):
        if at is None:
            return []
        count = self.number("<I", at)
        if at + 4 + 4 * count > len(self.data):
            raise ValueError("invalid shortcut model vector")
        return [self.indirect(at + 4 + 4 * i) for i in range(count)]

    def content(self):
        root = self.number("<I", 0)
        if self.number("<H", self.field(root, 0)) != 1:
            raise ValueError("unsupported shortcut model schema")
        group, = self.vector(self.reference(root, 3))
        application, = self.vector(self.reference(group, 1))
        def phrases(at):
            return sorted(self.string(self.reference(self.reference(t, 1), 0)) for t in self.vector(at))
        intents = {}
        for intent in self.vector(self.reference(application, 1)):
            name = self.string(self.reference(intent, 0))
            if name in intents:
                raise ValueError("duplicate shortcut model intent")
            intents[name] = phrases(self.reference(intent, 2))
        version = self.reference(root, 1)
        return {"locale": self.string(self.reference(root, 2)),
                "bundle": self.string(self.reference(application, 0)),
                "formatVersion": self.string(self.reference(version, 3)),
                "intents": intents, "negatives": phrases(self.reference(application, 2))}, self.string(self.reference(version, 0))


def read(path, allow_stale=False):
    content, fingerprint = Model(decompress(path)).content()
    versions = list(Path(path).parent.glob("*.version"))
    marker = Path(path).parent / (fingerprint + ".version")
    if (not allow_stale and len(versions) != 1) or not marker.is_file() or marker.read_bytes() != b"\0":
        raise ValueError(f"shortcut model version marker does not match its header: {path}")
    return content, marker
