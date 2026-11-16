#!/usr/bin/env python3
"""Compare the Mach-O products of two sweeps that differ only in their
Darwin SDK bundle, such as one linker against another.

    ci/linux-sweep-compare.py --a VM_DIR --b VM_DIR [--out FILE]

Runs in the VM, over two --derived-data folders of ci/linux-sweep.py. Every
Mach-O file under Build/Products in A is matched by path in B, and each
architecture is compared on what the link decides: the libraries it loads,
in order and with their weak and reexport flags, the symbols it exports, the
library each undefined symbol binds to, and the order of its load commands.
The linker's version in LC_BUILD_VERSION, UUIDs, addresses and sizes are
left out. The report lists each difference by kind, then by file.
"""
import argparse
import collections
import json
import os
import re
import subprocess
from pathlib import Path

TOOLS = Path(os.environ.get("SWIFT_BIN") or next(Path.home().glob("swift/swift-*-RELEASE/usr/bin")))
MAGICS = {b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"}


def is_macho(path):
    try:
        with open(path, "rb") as f:
            return f.read(4) in MAGICS
    except OSError:
        return False


def tool(*args):
    run = subprocess.run([str(TOOLS / args[0]), *map(str, args[1:])], capture_output=True, text=True)
    return run.stdout


def per_arch(text):
    """Split llvm-objdump or llvm-nm output into its architectures."""
    arches, current = collections.defaultdict(list), "single"
    for line in text.splitlines():
        match = re.search(r"\(architecture ([\w.]+)\):?$", line) or re.search(r"\(for architecture ([\w.]+)\):$", line)
        if match:
            current = match.group(1)
            continue
        if line.startswith("/") and line.endswith(":"):
            continue
        arches[current].append(line)
    return arches


def fingerprint(path):
    dylibs = {arch: [re.sub(r" \(compatibility version [^,]*, current version [^,)]*", "", l.strip()).rstrip(")")
                     for l in lines if l.strip()]
              for arch, lines in per_arch(tool("llvm-objdump", "--macho", "--dylibs-used", path)).items()}
    commands = {arch: [l.split()[1] for l in lines if re.match(r"\s+cmd LC_", l)]
                for arch, lines in per_arch(tool("llvm-objdump", "--macho", "--private-headers", path)).items()}
    symbols = {}
    for arch, lines in per_arch(tool("llvm-nm", "-m", "--arch=all", path)).items():
        exports, imports = set(), set()
        for line in lines:
            line = re.sub(r"^[0-9a-f]+ ", "", line.strip())
            if "(undefined)" in line:
                imports.add(re.sub(r"^\(undefined\) ", "", line))
            elif " external " in line and "non-external" not in line:
                exports.add(re.sub(r"^\([^)]*\) ", "", line))
        symbols[arch] = (exports, imports)
    return dylibs, commands, symbols


def compare(a, b):
    da, ca, sa = fingerprint(a)
    db, cb, sb = fingerprint(b)
    found = []
    for arch in sorted(set(da) | set(db)):
        la, lb = da.get(arch, []), db.get(arch, [])
        if la != lb:
            if sorted(la) == sorted(lb):
                found.append(("dylib order", arch, []))
            else:
                found.append(("dylib set", arch, [f"- {x}" for x in sorted(set(la) - set(lb))]
                                                 + [f"+ {x}" for x in sorted(set(lb) - set(la))]))
        if ca.get(arch) != cb.get(arch):
            found.append(("load commands", arch, [" ".join(ca.get(arch, [])), " ".join(cb.get(arch, []))]))
        (ea, ia), (eb, ib) = sa.get(arch, (set(), set())), sb.get(arch, (set(), set()))
        if ea != eb:
            found.append(("exports", arch, [f"- {x}" for x in sorted(ea - eb)] + [f"+ {x}" for x in sorted(eb - ea)]))
        if ia != ib:
            found.append(("imports", arch, [f"- {x}" for x in sorted(ia - ib)] + [f"+ {x}" for x in sorted(ib - ia)]))
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--a", type=Path, required=True)
    parser.add_argument("--b", type=Path, required=True)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    report = {"a": str(args.a), "b": str(args.b), "compared": 0, "missingInB": [], "differences": []}
    for path in sorted(args.a.rglob("*")):
        rel = path.relative_to(args.a)
        if "Build/Products" not in str(rel) or path.is_symlink() or not path.is_file() or not is_macho(path):
            continue
        if rel.suffix in (".a", ".o"):
            continue
        other = args.b / rel
        if not other.is_file():
            report["missingInB"].append(str(rel))
            continue
        report["compared"] += 1
        for kind, arch, detail in compare(path, other):
            report["differences"].append({"file": str(rel), "arch": arch, "kind": kind, "detail": detail[:40]})
    kinds = collections.Counter(d["kind"] for d in report["differences"])
    report["byKind"] = dict(kinds)
    text = json.dumps(report, indent=2) + "\n"
    if args.out:
        args.out.write_text(text)
    print(json.dumps({k: report[k] for k in ("compared", "byKind")} | {"missingInB": len(report["missingInB"])}, indent=2))


if __name__ == "__main__":
    main()
