#!/usr/bin/env python3
"""Regenerate synthetic archive vectors on macOS; contains no Apple SDK data."""
import ctypes
import hashlib
from pathlib import Path
import lzma
import os
import plistlib
import struct
import subprocess
import tempfile
import xml.etree.ElementTree as ET
import zlib

DATA = b"Sweetpad archive fixture header\n" * 10000
VERSION = plistlib.dumps({"CFBundleShortVersionString":"27.2", "ProductBuildVersion":"fixture"})
HEADER = "Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/include/swift/bridging"


def generate_clusters(output):
    """Use Apple's API flag that omits duplicate DAT blobs from clusters."""
    import shutil
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="sweetpad-archive-clusters-") as temporary:
        root = Path(temporary)
        tree = root / "input"
        outside = tree / "Xcode.app/Contents/AAData/shared.dat"
        outside.parent.mkdir(parents=True)
        outside.write_bytes(DATA)
        header = tree / HEADER
        header.parent.mkdir(parents=True)
        os.link(outside, header)
        copy = header.with_name("copy")
        shutil.copyfile(header, copy)
        copy.chmod(0o755)
        (header.parent / "link").symlink_to("bridging")
        (header.parent / "empty").touch()
        (header.parent / "empty-copy").touch()
        (tree / "Xcode.app/Contents/version.plist").write_bytes(VERSION)
        program = root / "generate-clusters"
        environment = dict(os.environ)
        environment.pop("SDKROOT", None)
        subprocess.run(["cc", "-std=c11", "-O2", str(output / "generate-clusters.c"),
                        "-lAppleArchive", "-o", str(program)], check=True, env=environment)
        for deduplicated in [False, True]:
            name = "clusters-deduplicated" if deduplicated else "clusters-full"
            target = output / (name + ".aar")
            target.unlink(missing_ok=True)
            subprocess.run([str(program), str(tree), str(target), str(int(deduplicated))], check=True)
        (root / "key").write_bytes(bytes(range(32)))
        subprocess.run(["/usr/bin/aea", "encrypt", "-profile", "1", "-i", str(output / "clusters-deduplicated.aar"),
                        "-o", str(output / "profile1-clusters.aea"), "-a", "lzraven", "-checksum", "sha256",
                        "-padding", "none", "-key", str(root / "key"), "-auth-data-key", "encryption_key",
                        "-auth-data", str(root / "key")], check=True)


def write_checksums(output):
    import json
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in output.iterdir()
              if p.suffix in [".aar", ".aea", ".xip", ".pem"]}
    (output / "checksums.json").write_text(json.dumps(hashes, indent=2, sort_keys=True) + "\n")


def cpio(items, newc=False):
    result = bytearray()
    for index, (name, data, mode, inode, links) in enumerate(items + [("TRAILER!!!", b"", 0, 0, 1)]):
        name = name.encode() + b"\0"
        if newc:
            fields = [inode,mode,0,0,links,0,len(data),1,0,0,0,len(name),0]
            header = b"070701" + b"".join(f"{v:08x}".encode() for v in fields)
        else:
            fields = [1,inode,mode,0,0,links,0]
            header = b"070707" + b"".join(f"{v:06o}".encode() for v in fields)
            header += f"{0:011o}{len(name):06o}{len(data):011o}".encode()
        result.extend(header + name)
        if newc: result.extend(bytes((-len(result)) % 4))
        result.extend(data)
        if newc: result.extend(bytes((-len(result)) % 4))
    return bytes(result)


def xip(payload, algorithm="x"):
    if algorithm == "m":
        lib = ctypes.CDLL("/usr/lib/libcompression.dylib")
        lib.compression_encode_buffer.argtypes = [ctypes.c_void_p,ctypes.c_size_t,ctypes.c_void_p,ctypes.c_size_t,ctypes.c_void_p,ctypes.c_int]
        lib.compression_encode_buffer.restype = ctypes.c_size_t
        output = ctypes.create_string_buffer(len(payload) + 65536)
        size = lib.compression_encode_buffer(output,len(output),payload,len(payload),None,0xD05)
        if not size: raise RuntimeError("Apple LZRAVEN fixture encoding failed")
        packed = output.raw[:size]
    else:
        packed = lzma.compress(payload)
    content = ("pbz" + algorithm).encode() + struct.pack(">QQQ",1<<24,len(payload),len(packed)) + packed
    toc = ET.Element("xar"); table = ET.SubElement(toc,"toc")
    checksum = ET.SubElement(table,"checksum",style="sha256")
    ET.SubElement(checksum,"offset").text="0"; ET.SubElement(checksum,"size").text="32"
    entry = ET.SubElement(table,"file",id="1")
    ET.SubElement(entry,"name").text="Content"; ET.SubElement(entry,"type").text="file"
    data = ET.SubElement(entry,"data")
    for name,value in [("offset",32),("length",len(content)),("size",len(content))]: ET.SubElement(data,name).text=str(value)
    ET.SubElement(data,"encoding",style="application/octet-stream")
    for name in ["archived-checksum","extracted-checksum"]:
        ET.SubElement(data,name,style="sha256").text=hashlib.sha256(content).hexdigest()
    raw=ET.tostring(toc,encoding="utf-8",xml_declaration=True); compressed=zlib.compress(raw)
    return struct.pack(">4sHHQQI",b"xar!",28,1,len(compressed),len(raw),3)+compressed+hashlib.sha256(compressed).digest()+content


def generate(output):
    output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="sweetpad-archive-vectors-") as temporary:
        root=Path(temporary); tree=root/"input"; tree.mkdir()
        header=tree/HEADER; header.parent.mkdir(parents=True)
        ignored=tree/"Xcode.app/Contents/Resources/shared.dat"; ignored.parent.mkdir(parents=True)
        ignored.write_bytes(DATA); os.link(ignored,header)
        (header.parent/"link").symlink_to("bridging");(header.parent/"empty").touch()
        (tree/"Xcode.app/Contents/version.plist").write_bytes(VERSION)
        for algorithm in ["raw","lzma","zlib","lzfse","lz4","lzraven"]:
            subprocess.run(["/usr/bin/aa","archive","-d",str(tree),"-exclude-field","attr","-a",algorithm,"-o",str(output/(algorithm+".aar"))],check=True)
        (root/"key").write_bytes(bytes(range(32)))
        subprocess.run(["openssl","ecparam","-name","prime256v1","-genkey","-noout","-out",str(root/"private.pem")],check=True)
        subprocess.run(["openssl","ec","-in",str(root/"private.pem"),"-pubout","-out",str(root/"public.pem")],check=True,capture_output=True)
        for profile in [0,1,2]:
            command=["/usr/bin/aea","encrypt","-profile",str(profile),"-i",str(output/"raw.aar"),"-o",str(output/f"profile{profile}.aea"),"-a","lzraven","-checksum","sha256","-padding","none"]
            if profile in [1,2]: command += ["-key",str(root/"key"),"-auth-data-key","encryption_key","-auth-data",str(root/"key")]
            if profile in [0,2]: command += ["-sign-priv",str(root/"private.pem"),"-auth-data-key","signing-public-key","-auth-data",str(root/"public.pem")]
            subprocess.run(command,check=True)
        for profile in [0,1]:
            command=["/usr/bin/aea","encrypt","-profile",str(profile),"-i",str(output/"raw.aar"),"-o",str(output/f"profile{profile}-external.aea"),"-a","lzma","-checksum","murmur64","-padding","4096","-b","16k"]
            command += ["-key",str(root/"key")] if profile == 1 else ["-sign-priv",str(root/"private.pem")]
            subprocess.run(command,check=True)
        (output/"signing-public.pem").write_bytes((root/"public.pem").read_bytes())
        with (root/"fcs.json").open("wb") as response:
            subprocess.run(["go","run",str(output/"wrap-fcs.go"),str(output/"rfc9180-p256.json")],
                check=True,stdout=response,env={**os.environ,"GOCACHE":str(root/"go-cache")})
        subprocess.run(["/usr/bin/aea","encrypt","-profile","1","-i",str(output/"raw.aar"),
            "-o",str(output/"profile1-fcs.aea"),"-a","lzraven","-checksum","sha256","-padding","none",
            "-key",str(root/"key"),"-auth-data-key","com.apple.wkms.fcs-response","-auth-data",str(root/"fcs.json"),
            "-auth-data-key","com.apple.wkms.fcs-key-url","-auth-data-value","hex:"+b"https://swdist.apple.com/sweetpad-fixture.p8".hex()],check=True)
        # The first link is deliberately outside the retained SDK selection.
        items=[("Xcode.app/Contents/Resources/shared.dat",DATA,0o100644,99,2),
               ("Xcode.app/Contents/version.plist",VERSION,0o100644,100,1),
               (HEADER,b"",0o100644,99,2),
               (str(Path(HEADER).parent/"link"),b"bridging",0o120777,101,1),
               (str(Path(HEADER).parent/"empty"),b"",0o100644,102,1)]
        for newc in [False,True]:
            for algorithm in ["x","m"]:
                (output/f"{'newc' if newc else 'odc'}-pbz{algorithm}.xip").write_bytes(xip(cpio(items,newc),algorithm))
    generate_clusters(output)
    write_checksums(output)


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--clusters-only", action="store_true")
    args = parser.parse_args()
    output = Path(__file__).with_name("fixtures") / "xcode-archives"
    if args.clusters_only:
        generate_clusters(output)
        write_checksums(output)
    else:
        generate(output)
