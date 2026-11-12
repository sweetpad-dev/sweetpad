"""Bounded Apple archive block decoding; native codecs are built in an owned cache."""
import ctypes
import hashlib
import lzma
import os
from pathlib import Path
import platform
import shlex
import shutil
import stat
import struct
import subprocess
import tempfile
import zlib

MAX_BLOCK = 64 * 1024**2
SOURCES = Path(__file__).with_name("archive-codecs")
_library = None


def library():
    global _library
    if _library is not None:
        return _library
    sources = sorted(p for p in SOURCES.rglob("*") if p.suffix in (".c", ".h"))
    identity = repr((platform.system(), platform.machine(), platform.libc_ver(), platform.mac_ver()[0]))
    digest = hashlib.sha256(identity.encode())
    # Changes to the builder and its permission/ABI rules invalidate old libraries.
    digest.update(Path(__file__).read_bytes())
    for path in sources:
        digest.update(path.relative_to(SOURCES).as_posix().encode())
        digest.update(path.read_bytes())
    cache_base = Path(os.environ.get("XDG_CACHE_HOME", str(Path.home() / ".cache")))
    if not cache_base.is_absolute():
        cache_base = Path.home() / ".cache"
    cache = cache_base / "sweetpad/archive-codecs"
    cache.mkdir(parents=True, exist_ok=True, mode=0o700)
    metadata = cache.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o022:
        raise ValueError("Archive codec cache must be an owned directory without group or world write access")
    destination = cache / (digest.hexdigest() + (".dylib" if platform.system() == "Darwin" else ".so"))
    if not destination.exists():
        compiler = shlex.split(os.environ.get("SWEETPAD_ARCHIVE_CC", "cc"))
        if not compiler or not shutil.which(compiler[0]):
            raise ValueError("Install a C compiler before importing this archive; sweetpad setup installs the host build tools")
        with tempfile.TemporaryDirectory(prefix=".build-", dir=cache) as work:
            output = Path(work) / destination.name
            command = compiler + ["-std=c11", "-O2", "-fPIC", "-shared", "-DNDEBUG",
                "-I" + str(SOURCES / "lzfse/src"), "-I" + str(SOURCES / "lzraven/include"),
                "-I" + str(SOURCES / "lzraven/src"), "-o", str(output)]
            command += [str(p) for p in sources if p.suffix == ".c"]
            environment = dict(os.environ)
            environment.pop("SDKROOT", None)
            result = subprocess.run(command, capture_output=True, text=True, env=environment)
            if result.returncode:
                raise ValueError("Build archive codecs failed: " + result.stderr[-4096:])
            # Compilers can create mode 0777 under a permissive container umask.
            # Publish a private artifact without relaxing the cache validation.
            output.chmod(0o700)
            os.replace(output, destination)
    metadata = destination.lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o022:
        raise ValueError("Archive codec library must be an owned regular file without group or world write access")
    result = ctypes.CDLL(str(destination))
    result.lzfse_decode_buffer.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p]
    result.lzfse_decode_buffer.restype = ctypes.c_size_t
    result.lzraven_decode_buffer_ex.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_int)]
    result.lzraven_decode_buffer_ex.restype = ctypes.c_size_t
    result.sweetpad_murmur64.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    result.sweetpad_murmur64.restype = ctypes.c_uint64
    _library = result
    return result


def lz4_block(data, expected, dictionary=b""):
    output = bytearray(dictionary)
    start = len(output)
    pos = 0

    def length(value):
        nonlocal pos
        if value == 15:
            while True:
                if pos >= len(data):
                    raise ValueError("Truncated LZ4 length")
                extra = data[pos]; pos += 1; value += extra
                if value > expected:
                    raise ValueError("LZ4 block exceeds declared size")
                if extra != 255:
                    break
        return value

    while pos < len(data):
        token = data[pos]; pos += 1
        literal = length(token >> 4)
        if pos + literal > len(data) or len(output) + literal > start + expected:
            raise ValueError("Invalid LZ4 literal range")
        output.extend(data[pos:pos + literal]); pos += literal
        if pos == len(data):
            break
        if pos + 2 > len(data):
            raise ValueError("Truncated LZ4 match")
        distance = int.from_bytes(data[pos:pos + 2], "little"); pos += 2
        count = length(token & 15) + 4
        if not distance or distance > len(output) or len(output) + count > start + expected:
            raise ValueError("Invalid LZ4 match range")
        pattern = bytes(output[-distance:])
        whole, tail = divmod(count, distance)
        output.extend(pattern * whole + pattern[:tail])
    if len(output) != start + expected:
        raise ValueError("LZ4 output size differs from its header")
    return bytes(output[start:])


def apple_lz4(data, expected):
    output = bytearray()
    pos = 0
    while pos + 4 <= len(data):
        magic = data[pos:pos + 4]; pos += 4
        if magic == b"bv4$":
            if pos != len(data) or len(output) != expected:
                raise ValueError("Invalid LZ4 stream length")
            return bytes(output)
        if pos + 4 > len(data):
            break
        size = int.from_bytes(data[pos:pos + 4], "little"); pos += 4
        if size > expected - len(output):
            raise ValueError("LZ4 stream exceeds declared size")
        if magic == b"bv4-":
            packed = size
        elif magic == b"bv41" and pos + 4 <= len(data):
            packed = int.from_bytes(data[pos:pos + 4], "little"); pos += 4
        else:
            raise ValueError("Unsupported LZ4 stream block")
        if packed > len(data) - pos:
            raise ValueError("Truncated LZ4 stream block")
        block = data[pos:pos + packed]; pos += packed
        output.extend(block if magic == b"bv4-" else lz4_block(block, size, output[-65536:]))
    raise ValueError("Missing LZ4 stream terminator")


def decompress(algorithm, data, expected):
    if not 0 <= expected <= MAX_BLOCK or len(data) > MAX_BLOCK:
        raise ValueError("Archive compression block exceeds the memory limit")
    if algorithm in ("-", "raw"):
        result = data
    elif algorithm == "x":
        decoder = lzma.LZMADecompressor(memlimit=256 * 1024**2)
        try:
            result = decoder.decompress(data, max_length=expected + 1)
        except lzma.LZMAError:
            raise ValueError("Invalid LZMA archive block") from None
        if not decoder.eof or decoder.unused_data:
            raise ValueError("Invalid LZMA stream boundary")
    elif algorithm == "z":
        decoder = zlib.decompressobj()
        try:
            result = decoder.decompress(data, expected + 1)
        except zlib.error:
            raise ValueError("Invalid zlib archive block") from None
        if not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
            raise ValueError("Invalid zlib stream boundary")
    elif algorithm == "4":
        result = apple_lz4(data, expected)
    elif algorithm in ("e", "f", "m"):
        source = data
        if algorithm == "f":
            # LZFSE's decoder also handles its framed LZVN block representation.
            source = b"bvxn" + struct.pack("<II", expected, len(data)) + data + b"bvx$"
        target = ctypes.create_string_buffer(expected + 1)
        codec = library()
        if algorithm == "m":
            status = ctypes.c_int()
            size = codec.lzraven_decode_buffer_ex(target, expected + 1, source, len(source), ctypes.byref(status))
            if status.value:
                raise ValueError("Invalid LZRAVEN stream (status " + str(status.value) + ")")
        else:
            size = codec.lzfse_decode_buffer(target, expected + 1, source, len(source), None)
        result = target.raw[:size]
    else:
        raise ValueError("Unsupported Apple archive compression: " + repr(algorithm))
    if len(result) != expected:
        raise ValueError("Archive block output differs from its declared size")
    return result
