"""Streaming AEA profiles 0, 1 and 2, with per-block authentication.

Format references: https://github.com/kinnay/AEA/blob/main/FORMAT.md
and Apple's AppleArchive headers. Authentication does not establish Apple publisher trust.
"""
import base64
import hashlib
import hmac
import importlib.util
from pathlib import Path
import plistlib
import struct


def load_codecs():
    spec = importlib.util.spec_from_file_location("sweetpad_archive_codecs", Path(__file__).with_name("linux-archive-codecs.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


CODECS = load_codecs()
_spec = importlib.util.spec_from_file_location("sweetpad_apple_keys", Path(__file__).with_name("linux-apple-keys.py"))
APPLE_KEYS = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(APPLE_KEYS)
MAX_AUTH = 16 * 1024**2


def exact(source, size):
    if size < 0:
        raise ValueError("Invalid archive read size")
    value = source.read(size)
    if len(value) != size:
        raise ValueError("Truncated AEA archive")
    return value


def hkdf(key, info, size, salt=b""):
    extracted = hmac.digest(salt or bytes(32), key, "sha256")
    output = b""; previous = b""
    for index in range(1, (size + 31) // 32 + 1):
        previous = hmac.digest(extracted, previous + info + bytes([index]), "sha256")
        output += previous
    return output[:size]


def authenticated(key, data, associated, expected, encrypted):
    actual = hmac.digest(key[:32], associated + data + struct.pack("<Q", len(associated)), "sha256")
    if not hmac.compare_digest(actual, expected):
        raise ValueError("AEA authentication failed (wrong key or modified archive)")
    if not encrypted:
        return data
    try:
        from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
    except ImportError:
        raise ValueError("AEA import needs Python cryptography; run sweetpad setup or install your distribution's Python cryptography package") from None
    decryptor = Cipher(algorithms.AES(key[32:64]), modes.CTR(key[64:80])).decryptor()
    return decryptor.update(data) + decryptor.finalize()


def metadata_values(data):
    """Accept the documented key/value encoding and certificate plists."""
    values = []
    if data.startswith((b"bplist", b"<?xml")):
        try:
            pending = [(plistlib.loads(data), 0)]
            while pending and len(values) < 128:
                item, depth = pending.pop()
                if depth > 16:
                    raise ValueError("AEA metadata nesting limit exceeded")
                if isinstance(item, (bytes, str)):
                    values.append(item.encode() if isinstance(item, str) else item)
                elif isinstance(item, dict):
                    pending.extend((v, depth + 1) for v in item.values())
                elif isinstance(item, list):
                    pending.extend((v, depth + 1) for v in item)
            return values
        except (plistlib.InvalidFileException, ValueError):
            return []
    pos = 0
    while pos < len(data):
        if pos + 4 > len(data):
            return []
        length = int.from_bytes(data[pos:pos + 4], "little")
        if length < 5 or length > len(data) - pos:
            return []
        entry = data[pos + 4:pos + length]
        if b"\0" not in entry:
            return []
        values.append(entry.split(b"\0", 1)[1])
        if len(values) > 128:
            raise ValueError("Too many AEA authentication entries")
        pos += length
    return values


def key_bytes(value):
    if len(value) == 32:
        return value
    try:
        text = value.decode("ascii").strip()
        if text.startswith("hex:"):
            result = bytes.fromhex(text[4:])
        elif text.startswith("base64:"):
            result = base64.b64decode(text[7:], validate=True)
        elif len(text) == 64:
            result = bytes.fromhex(text)
        else:
            result = base64.b64decode(text, validate=True)
        return result if len(result) == 32 else None
    except (ValueError, UnicodeError):
        return None


def file_material(path):
    if path is None:
        return None
    path = Path(path)
    if path.stat().st_size > 128 * 1024:
        raise ValueError("AEA key file is too large")
    return path.read_bytes()


def public_keys(values):
    try:
        from cryptography.exceptions import UnsupportedAlgorithm
        from cryptography import x509
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric import ec
    except ImportError:
        raise ValueError("AEA import needs Python cryptography; run sweetpad setup or install your distribution's Python cryptography package") from None
    result = {}
    for value in values:
        candidates = [value]
        if value.startswith(b"\x04\x41\x04") and len(value) == 67:
            candidates.append(value[2:])
        for candidate in candidates:
            loaders = [serialization.load_pem_public_key, serialization.load_der_public_key,
                       lambda v: x509.load_der_x509_certificate(v).public_key(),
                       lambda v: x509.load_pem_x509_certificate(v).public_key(),
                       lambda v: ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), v)]
            for loader in loaders:
                try:
                    key = loader(candidate)
                    if isinstance(key, ec.EllipticCurvePublicKey) and key.curve.name == "secp256r1":
                        raw = key.public_bytes(serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint)
                        result[raw] = key
                        break
                except (ValueError, TypeError, UnsupportedAlgorithm):
                    pass
    return list(result.items())


def murmur64(data):
    return CODECS.library().sweetpad_murmur64(data, len(data)).to_bytes(8, "little")


def chunks(path, key_file=None, public_key_file=None):
    try:
        import cryptography  # noqa: F401
    except ImportError:
        raise ValueError("AEA import needs Python cryptography; run sweetpad setup or install your distribution's Python cryptography package") from None
    with Path(path).open("rb") as source:
        header = exact(source, 12)
        if header[:4] != b"AEA1":
            raise ValueError("Not an Apple Encrypted Archive")
        profile = int.from_bytes(header[4:7], "little")
        if profile not in (0, 1, 2):
            raise ValueError(f"AEA profile {profile} is unsupported; use a Mac SDK export for this archive")
        auth_size = int.from_bytes(header[8:12], "little")
        if auth_size > MAX_AUTH:
            raise ValueError("AEA authentication metadata exceeds the memory limit")
        auth = exact(source, auth_size)
        signature_start = source.tell()
        signature = exact(source, {0: 128, 1: 0, 2: 160}[profile])
        signature_end = source.tell()
        encryption_data = exact(source, 32) if profile == 0 else None
        salt = exact(source, 32); root_mac = exact(source, 32)
        encrypted_root = exact(source, 48); cluster_mac = exact(source, 32)
        prologue_end = source.tell()
        values = metadata_values(auth)
        supplied = file_material(key_file)
        keys = [encryption_data] if profile == 0 else [key_bytes(supplied)] if supplied is not None else [key_bytes(v) for v in values]
        keys = list(dict.fromkeys(k for k in keys if k is not None))[:16]
        apple_metadata = APPLE_KEYS.metadata(auth)
        if supplied is None and profile in (1,2) and "com.apple.wkms.fcs-response" in apple_metadata and "encryption_key" not in apple_metadata:
            keys = []
        if not keys and profile in (1,2):
            unwrapped = APPLE_KEYS.unwrap(auth, supplied)
            if unwrapped is not None:
                keys = [unwrapped]
        if not keys:
            raise ValueError("AEA decryption key is not embedded; supply --aea-key-file with a 32-byte key, hex: value, or base64: value")
        supplied_public = file_material(public_key_file)
        signing = public_keys([supplied_public] if supplied_public is not None else values)[:16] if profile != 1 else [(b"", None)]
        if not signing:
            raise ValueError("AEA signing public key is not embedded; supply --aea-public-key-file")
        found = None
        for material in keys:
            for raw_public, public in signing:
                main = hkdf(material, b"AEA_AMK" + header[4:8] + raw_public, 32, salt)
                try:
                    root = authenticated(hkdf(main, b"AEA_RHEK", 32 if profile == 0 else 80),
                                         encrypted_root, cluster_mac + auth, root_mac, profile != 0)
                    found = (main, public, root)
                    break
                except ValueError:
                    continue
            if found:
                break
        if not found:
            raise ValueError("AEA authentication failed (wrong key or modified archive)")
        main, public, root = found
        if profile != 1:
            from cryptography.exceptions import InvalidSignature
            from cryptography.hazmat.primitives import hashes
            from cryptography.hazmat.primitives.asymmetric import ec
            if profile == 2:
                signing_key = hkdf(hkdf(main, b"AEA_SEK", 32), b"AEA_SEK2", 80)
                signature = authenticated(signing_key, signature[:128], b"", signature[128:], True)
            if len(signature) < 2 or signature[0] != 0x30 or signature[1] >= 128 or signature[1] + 2 > 128:
                raise ValueError("Invalid AEA signature encoding")
            signature = signature[:signature[1] + 2]
            source.seek(0); prologue = bytearray(exact(source, prologue_end))
            prologue[signature_start:signature_end] = bytes(signature_end - signature_start)
            try:
                public.verify(signature, bytes(prologue), ec.ECDSA(hashes.SHA256()))
            except InvalidSignature:
                raise ValueError("AEA signature verification failed") from None
        source.seek(prologue_end)
        total, container_size, segment_size, per_cluster, algorithm, checksum = struct.unpack_from("<QQIIBB", root)
        if container_size != Path(path).stat().st_size or total > 64 * 1024**3:
            raise ValueError("Invalid AEA archive size")
        if not 0 < segment_size <= CODECS.MAX_BLOCK or not 0 < per_cluster <= 65536 or checksum not in (0, 1, 2):
            raise ValueError("Invalid AEA cluster parameters")
        checksum_size = {0: 0, 1: 8, 2: 32}[checksum]
        entry_size = 8 + checksum_size
        if per_cluster * (entry_size + 32) > CODECS.MAX_BLOCK:
            raise ValueError("AEA cluster metadata exceeds the memory limit")
        produced = 0; cluster_index = 0; encrypted = profile != 0; key_size = 80 if encrypted else 32
        while produced < total:
            cluster_key = hkdf(main, b"AEA_CK" + struct.pack("<I", cluster_index), 32)
            packed_headers = exact(source, entry_size * per_cluster)
            next_mac = exact(source, 32); segment_macs = exact(source, 32 * per_cluster)
            entries = authenticated(hkdf(cluster_key, b"AEA_CHEK", key_size), packed_headers,
                                    next_mac + segment_macs, cluster_mac, encrypted)
            for index in range(per_cluster):
                original, packed = struct.unpack_from("<II", entries, index * entry_size)
                expected_checksum = entries[index * entry_size + 8:(index + 1) * entry_size]
                if not original and not packed:
                    if produced != total:
                        raise ValueError("AEA cluster ended before the declared output size")
                    continue
                if original > segment_size or packed > segment_size or not original or produced + original > total:
                    raise ValueError("Invalid AEA segment range")
                data = exact(source, packed)
                key = hkdf(cluster_key, b"AEA_SK" + struct.pack("<I", index), key_size)
                data = authenticated(key, data, b"", segment_macs[index * 32:(index + 1) * 32], encrypted)
                data = CODECS.decompress(chr(algorithm), data, original) if packed != original else data
                actual_checksum = b"" if checksum == 0 else murmur64(data) if checksum == 1 else hashlib.sha256(data).digest()
                if not hmac.compare_digest(actual_checksum, expected_checksum):
                    raise ValueError("AEA segment checksum failed")
                produced += original
                yield data
            cluster_mac = next_mac; cluster_index += 1
        # With no padding Apple leaves the final next-cluster MAC unspecified.
        padding = hmac.new(hkdf(main, b"AEA_PAK", 32), digestmod="sha256")
        padding_size = 0
        while True:
            data = source.read(1024 * 1024)
            if not data:
                break
            padding_size += len(data)
            padding.update(data)
        if padding_size and not hmac.compare_digest(padding.digest(), cluster_mac):
            raise ValueError("AEA padding authentication failed")
