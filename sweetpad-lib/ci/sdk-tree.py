#!/usr/bin/env python3
"""Record a Darwin SDK bundle's tree, and compare two of them.

    sdk-tree.py record ROOT --out FILE.tsv.gz
    sdk-tree.py compare A B [--ignore GLOB]... [--examples N]

A record is one line per file, folder and link under ROOT, sorted by path:
its kind (f, d or l), permission bits in octal, size, the SHA-256 of a
file's contents or a link's target, and the path relative to ROOT. Owners
and times are left out, since they say nothing about what a build reads.

compare takes a record or a bundle on each side. It reports each path that
is only in A, only in B, or in both with a different kind, mode or content
(for a link, its target), as JSON: counts by kind of difference, then by
area (the path's first components, down to a platform or the toolchain),
with examples. --ignore leaves out the paths a glob matches, for the
differences already chosen.
"""
import argparse
from collections import Counter, defaultdict
import fnmatch
import gzip
import hashlib
import json
import os
from pathlib import Path
import stat
import sys

KINDS = {stat.S_IFREG: "f", stat.S_IFDIR: "d", stat.S_IFLNK: "l"}


def digest(path):
    hasher = hashlib.sha256()
    with open(path, "rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def entry(root, path):
    info = os.lstat(path)
    kind = KINDS.get(stat.S_IFMT(info.st_mode))
    if kind is None:
        raise ValueError(f"{path} is neither a file, a folder nor a link")
    relative = os.path.relpath(path, root)
    if "\t" in relative or "\n" in relative:
        raise ValueError(f"{relative!r} has a tab or a newline, which a record can't hold")
    if kind == "f":
        size, content = info.st_size, digest(path)
    elif kind == "l":
        target = os.readlink(path)
        size, content = len(target), target
    else:
        size, content = 0, "-"
    return relative, (kind, f"{stat.S_IMODE(info.st_mode):o}", str(size), content)


def walk(root):
    """Every entry under root but root itself, by path."""
    entries = {}
    for folder, folders, files in os.walk(root):
        for name in folders + files:
            path, value = entry(root, os.path.join(folder, name))
            entries[path] = value
    return dict(sorted(entries.items()))


def write(entries, out):
    with gzip.open(out, "wt", encoding="utf-8", newline="\n") as file:
        for path, (kind, mode, size, content) in entries.items():
            file.write(f"{kind}\t{mode}\t{size}\t{content}\t{path}\n")


def read(source):
    """A record, or the tree of a bundle folder."""
    if os.path.isdir(source):
        return walk(source)
    entries = {}
    with gzip.open(source, "rt", encoding="utf-8", newline="\n") as file:
        for number, line in enumerate(file, 1):
            fields = line.rstrip("\n").split("\t")
            if len(fields) != 5:
                raise ValueError(f"{source}:{number} isn't a record line")
            kind, mode, size, content, path = fields
            entries[path] = (kind, mode, size, content)
    return entries


def area(path):
    """The part of the bundle a path is in: down to a platform, an SDK or
    the toolchain's library, as far as the path goes."""
    parts = path.split("/")
    depth = 1
    if parts[0] == "Developer" and len(parts) > 1:
        depth = 3 if parts[1] in ("Platforms", "Toolchains") else 2
        if parts[1] == "Platforms" and parts[3:5] == ["Developer", "SDKs"]:
            depth = 6
    return "/".join(parts[:depth])


def compare(a, b, ignore=(), examples=5):
    def kept(path):
        return not any(fnmatch.fnmatchcase(path, glob) for glob in ignore)

    differences = []
    for path in sorted(set(a) | set(b)):
        if not kept(path):
            continue
        left, right = a.get(path), b.get(path)
        if left == right:
            continue
        if right is None:
            kind = "onlyInA"
        elif left is None:
            kind = "onlyInB"
        elif left[0] != right[0]:
            kind = "kind"
        elif left[3] != right[3]:
            kind = "link" if left[0] == "l" else "content"
        else:
            kind = "mode"
        differences.append((kind, path))
    by_kind = Counter(kind for kind, _ in differences)
    by_area = defaultdict(lambda: {"count": Counter(), "examples": []})
    for kind, path in differences:
        found = by_area[area(path)]
        found["count"][kind] += 1
        if len(found["examples"]) < examples:
            found["examples"].append(f"{kind}: {path}")
    return {
        "entries": {"a": len(a), "b": len(b)},
        "differences": len(differences),
        "byKind": dict(sorted(by_kind.items())),
        "byArea": {name: {"count": dict(sorted(v["count"].items())), "examples": v["examples"]}
                   for name, v in sorted(by_area.items())},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="action", required=True)
    record = commands.add_parser("record")
    record.add_argument("root", type=Path)
    record.add_argument("--out", type=Path, required=True)
    diff = commands.add_parser("compare")
    diff.add_argument("a")
    diff.add_argument("b")
    diff.add_argument("--ignore", action="append", default=[])
    diff.add_argument("--examples", type=int, default=5)
    args = parser.parse_args()
    if args.action == "record":
        entries = walk(args.root)
        write(entries, args.out)
        counts = Counter(kind for kind, *_ in entries.values())
        print(json.dumps({"out": str(args.out), "entries": len(entries), "byKind": dict(sorted(counts.items()))}))
        return 0
    report = compare(read(args.a), read(args.b), args.ignore, args.examples)
    print(json.dumps(report, indent=2))
    return 1 if report["differences"] else 0


if __name__ == "__main__":
    sys.exit(main())
