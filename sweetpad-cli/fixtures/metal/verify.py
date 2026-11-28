#!/usr/bin/env python3
"""Two independent builds and a GPU comparison. See README.md for invocations."""

import argparse
import concurrent.futures
import hashlib
import json
from pathlib import Path, PurePosixPath
import platform
import struct
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
DRIVER = HERE.parents[1] / "src/cli/metal/driver.py"
REVISION = "78b75e2d99ccf038fd525ba1effad3ebe7b6aeb4"
VERSIONS = ("2.4", "3.0", "3.1", "3.2", "4.0", "4.1")
STAGES = (
    "vertex",
    "fragment",
    "compute",
    "visible",
    "stitchable",
    "intersection",
    "object",
    "mesh",
    "tile",
)
ENTRIES = {
    "vertex": ("origin", 0),
    "fragment": ("shade", 1),
    "compute": ("increment", 2),
    "visible": ("twice", 4),
    "stitchable": ("addFive", 4),
    "intersection": ("acceptTriangle", 6),
    "object": ("makePayload", 8),
    "mesh": ("emitTriangle", 7),
    "tile": ("fillTile", 2),
    "language": ("language", 2),
}
EXPECTED = {
    "vertex": [128, 0, 0, 255, 256],
    "fragment": [128, 0, 0, 255, 256],
    "compute": [3, 4, 10, 22],
    "visible": [0, 2, 14, 38],
    "stitchable": [10, 11, 17, 29],
    "intersection": [1, 0],
    "object": [191, 0, 0, 255, 256],
    "mesh": [191, 0, 0, 255, 256],
    "tile": [0.25, 256],
}
KHRONOS_RUNTIME = {
    "asm/vert/copy-memory-interface.asm.vert": "khronos-vertex",
    "frag/false-loop-init.frag": "khronos-fragment",
    "asm/comp/bitcast_sar.asm.comp": "khronos-compute",
}
EXPECTED.update(
    {
        "khronos-vertex": [0, 255, 255, 255, 256],
        "khronos-fragment": [64, 128, 191, 255, 256],
        "khronos-compute": [0, 0, 0, 0, 3, 4, 3, 3],
    }
)


def check(*args):
    return subprocess.check_output(args, text=True).strip()


def exports(path):
    data = path.read_bytes()
    if data[:4] != b"MTLB":
        raise ValueError("missing Metal library header")
    start, size = struct.unpack_from("<QQ", data, 24)
    (count,) = struct.unpack_from("<I", data, start)
    pos, end = start + 4, start + 4 + size
    entries = {}
    for _ in range(count):
        (length,) = struct.unpack_from("<I", data, pos)
        stop = pos + length
        if stop > end or data[stop - 4 : stop] != b"ENDT":
            raise ValueError("invalid function record bounds")
        pos += 4
        tags = {}
        while pos < stop - 4:
            name, length = struct.unpack_from("<4sH", data, pos)
            tags[name] = data[pos + 6 : pos + 6 + length]
            pos += 6 + length
        if pos != stop - 4:
            raise ValueError("invalid function tag length")
        name = tags[b"NAME"][:-1].decode()
        if name in entries:
            raise ValueError("duplicate named entry")
        entries[name] = {"type": tags[b"TYPE"][0], "version": list(struct.unpack("<4H", tags[b"VERS"]))}
        pos = stop
    if pos != end:
        raise ValueError("unexpected function record count")
    return entries


def corpus(archive, dest):
    rows = {}
    prefix = ("SPIRV-Cross-" + REVISION, "reference", "shaders-msl")
    with tarfile.open(archive, "r:gz") as tar:
        for f in tar:
            name = PurePosixPath(f.name)
            if (
                not f.isfile()
                or name.parts[:3] != prefix
                or name.suffix not in {".vert", ".frag", ".comp"}
            ):
                continue
            data = tar.extractfile(f).read()
            key = hashlib.sha256(data).hexdigest()
            if key not in rows:
                (dest / (key + ".metal")).write_bytes(data)
            rows.setdefault(key, []).append(str(PurePosixPath(*name.parts[3:])))
    if len(rows) != 443 or sum(map(len, rows.values())) != 451:
        raise ValueError("unexpected pinned Khronos corpus inventory")
    return rows


def result_of(cmd):
    return subprocess.run(cmd, text=True, capture_output=True)


def check_baseline(report):
    baseline = json.loads((HERE / "coverage.json").read_text())
    if any(report[key] != baseline[key] for key in ("revision", "target")):
        raise ValueError("no Khronos baseline for this revision and deployment target")
    groups = report["corpus"]
    inventory = json.dumps(groups, sort_keys=True, separators=(",", ":")).encode()
    if hashlib.sha256(inventory).hexdigest() != baseline["inventorySha256"]:
        raise ValueError("the pinned Khronos source inventory changed")
    missing = []
    for v in report["versions"]:
        supported = set(baseline["matchingSources"])
        reference_failures = set(baseline["xcodeRejected"][v])
        for digest, paths in groups.items():
            key = v + "-" + digest
            built = report["cases"][key]["status"] == "built"
            name = min(paths)
            if report["compiler"] == "xcode":
                if built == (name in reference_failures):
                    missing.append(f"Xcode {v} {name}")
            elif name in supported and not built:
                missing.append(f"Linux {v} {name}")
    if missing:
        raise ValueError("Khronos coverage changed: " + ", ".join(missing))


def check_stages(report):
    for version in report["versions"]:
        for kind, (name, tag) in ENTRIES.items():
            key = version + "-" + kind
            row = report["cases"][key]
            if version == "2.4" and kind in {"mesh", "object"}:
                ok = row["status"] == "compile-rejected"
            else:
                major, minor = map(int, version.split("."))
                ok = row.get("exports") == {name: {"type": tag, "version": [2, 9, major, minor]}}
            if not ok:
                raise ValueError(f"{report['compiler']} {key}: {row}")


def build(args):
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=True)
    src = root / "sources"
    src.mkdir(exist_ok=True)
    groups = corpus(args.corpus, src) if args.corpus else {}
    modes = args.versions.split(",")
    if not modes or any(v not in VERSIONS for v in modes):
        raise ValueError("unknown MSL version")
    if args.compiler == "xcode":
        if platform.system() != "Darwin":
            raise ValueError("build the Xcode control on macOS")
        sdk = check("xcrun", "--sdk", "macosx", "--show-sdk-path")
        metal = check("xcrun", "--sdk", "iphoneos", "--find", "metal")
        prefix = [metal, "-isysroot", sdk, "-fmodules-cache-path=" + str(root / "modules")]
        linker = [metal, "-isysroot", sdk, "-target", args.target]
        compiler_version = check("xcodebuild", "-version")
    else:
        if platform.system() != "Linux":
            raise ValueError("build the candidate on Linux")
        compiler_version = check("clang++-14", "--version").splitlines()[0]
        prefix = (
            [sys.executable, str(DRIVER), "metal"]
            if not args.tools
            else [str(args.tools.resolve() / "metal")]
        )
        linker = (
            [sys.executable, str(DRIVER), "metallib"]
            if not args.tools
            else [str(args.tools.resolve() / "metallib")]
        )

    def job(item):
        v, key, source = item
        work = root / ("khronos" if key in groups else "stages")
        work.mkdir(exist_ok=True)
        stem = v + "-" + key
        air, lib = work / (stem + ".air"), work / (stem + ".metallib")
        language = ("macos-" if v == "2.4" else "") + "metal" + v
        opts = ["-D" + key.upper()] if key in STAGES else []
        compile_run = result_of(
            [*prefix, "-target", args.target, "-std=" + language, *opts, "-c", source, "-o", air]
        )
        if compile_run.returncode:
            return stem, {"status": "compile-rejected", "diagnostic": compile_run.stderr.strip()[:1800]}
        link_run = result_of([*linker, air, "-o", lib])
        if link_run.returncode:
            return stem, {"status": "link-rejected", "diagnostic": link_run.stderr.strip()[:1800]}
        return stem, {"status": "built", "exports": exports(lib)}

    items = [(v, k, HERE / "Kinds.metal") for v in modes for k in STAGES]
    items += [(v, "language", HERE / "Language.metal") for v in modes]
    items += [(v, k, src / (k + ".metal")) for v in modes for k in sorted(groups)]
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        rows = dict(pool.map(job, items))
    report = {
        "compiler": args.compiler,
        "toolchain": compiler_version,
        "target": args.target,
        "versions": modes,
        "revision": REVISION,
        "corpus": groups,
        "cases": rows,
    }
    (root / "results.json").write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
    check_stages(report)
    if args.corpus:
        check_baseline(report)
    print(f"{args.compiler}: {sum(r['status'] == 'built' for r in rows.values())}/{len(rows)} built")


def compare(args):
    linux, xcode = args.linux.resolve(), args.xcode.resolve()
    a, b = [json.loads((p / "results.json").read_text()) for p in (linux, xcode)]
    for k in ("target", "versions", "revision", "corpus"):
        if a[k] != b[k]:
            raise ValueError(f"build reports disagree on {k}")
    if a["compiler"] != "linux" or b["compiler"] != "xcode" or a["cases"].keys() != b["cases"].keys():
        raise ValueError("mismatched compiler inventories")
    if args.require_full_corpus and not a["corpus"]:
        raise ValueError("--require-full-corpus requires both builds to include the Khronos archive")
    for build_report in (a, b):
        check_stages(build_report)
        if build_report["corpus"]:
            check_baseline(build_report)
    dest = args.output.resolve()
    dest.mkdir(parents=True, exist_ok=True)
    runner, helpers = dest / "Runtime", dest / "Consumer.metallib"
    sdk = check("xcrun", "--sdk", "macosx", "--show-sdk-path")
    metal = check("xcrun", "--sdk", "iphoneos", "--find", "metal")
    subprocess.run(
        [
            "xcrun",
            "--sdk",
            "macosx",
            "clang",
            "-fobjc-arc",
            "-framework",
            "Foundation",
            "-framework",
            "Metal",
            HERE / "Runtime.m",
            "-o",
            runner,
        ],
        check=True,
    )
    subprocess.run(
        [
            metal,
            "-isysroot",
            sdk,
            "-target",
            a["target"],
            "-std=metal4.1",
            "-fmodules-cache-path=" + str(xcode / "modules"),
            HERE / "Consumer.metal",
            "-o",
            helpers,
        ],
        check=True,
    )
    runtime, counts, discrepancies, failures = {}, {}, {}, []
    common = [
        k
        for k, row in a["cases"].items()
        if k.split("-", 1)[1] in a["corpus"] and row["status"] == b["cases"][k]["status"] == "built"
    ]
    reflection = {}
    for name, root in (("linux", linux), ("xcode", xcode)):
        reflection[name] = {}
        for start in range(0, len(common), 64):
            files = [root / "khronos" / (k + ".metallib") for k in common[start : start + 64]]
            reflection[name].update(json.loads(check(runner, "reflect", *files)))

    def observe(shader, key, v, folder, left, right):
        row = {}
        for name, root, status in (("xcode", xcode, right), ("linux", linux, left)):
            if status != "built":
                row[name] = {"error": status}
                continue
            p = result_of(
                [runner, shader, root / folder / (key + ".metallib"), xcode / "stages", v, helpers]
            )
            row[name] = json.loads(p.stdout) if p.returncode == 0 else {"error": p.stderr.strip()}
        return row

    for key, source in b["cases"].items():
        v, shader = key.split("-", 1)
        candidate = a["cases"][key]
        left, right = candidate["status"], source["status"]
        equal = left == right == "built" and candidate["exports"] == source["exports"]
        if shader in STAGES or shader == "language":
            if v == "2.4" and shader in {"mesh", "object"}:
                ok = left == right == "compile-rejected" and "3.0" in candidate["diagnostic"]
                runtime[key] = {"version-gated": ok}
            else:
                row = runtime[key] = observe(shader, key, v, "stages", left, right)
                if shader == "language":
                    if v == "4.1":
                        expected = [65, 64, 70, 82]
                    else:
                        add = 40 if v == "4.0" else int(v.replace(".", "")) * 10
                        expected = [x + add for x in (0, 1, 7, 19)]
                else:
                    expected = EXPECTED[shader]
                ok = equal and all(row[n].get("values") == expected for n in ("xcode", "linux"))
                row["match"] = ok
            if not ok:
                failures.append(key)
        else:
            if equal:
                x = reflection["xcode"][key]
                y = reflection["linux"][key]
                equal = "error" not in x and x == y
            for path in b["corpus"][shader]:
                if path in KHRONOS_RUNTIME:
                    probe = KHRONOS_RUNTIME[path]
                    row = runtime[v + "-" + probe] = observe(probe, key, v, "khronos", left, right)
                    row["path"] = path
                    row["match"] = equal and all(
                        row[n].get("values") == EXPECTED[probe] for n in ("xcode", "linux")
                    )
                    if not row["match"]:
                        failures.append(v + "-" + probe)
            status = (
                "matching-interface"
                if equal
                else "different-interface"
                if left == right == "built"
                else "candidate-rejected"
                if right == "built"
                else "candidate-only"
                if left == "built"
                else "both-rejected"
            )
            version_counts = counts.setdefault(v, {})
            version_counts[status] = version_counts.get(status, 0) + 1
            if status not in {"matching-interface", "both-rejected"}:
                discrepancies[key] = {"status": status, "paths": b["corpus"][shader]}
                if status != "candidate-rejected" or args.require_full_corpus:
                    failures.append(key)
    report = {
        "target": a["target"],
        "xcode": b["toolchain"],
        "linux": a["toolchain"],
        "khronosRevision": REVISION,
        "runtime": runtime,
        "corpusCounts": counts,
        "corpusGaps": discrepancies,
        "failures": failures,
        "gpuReflection": reflection,
    }
    (dest / "comparison.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(counts, indent=2))
    print(
        f"{len(runtime)} stage cases, {len(failures)} failures; full evidence in {dest / 'comparison.json'}"
    )
    return bool(failures)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    b = commands.add_parser("build")
    b.add_argument("--compiler", choices=["linux", "xcode"], required=True)
    b.add_argument("--output", type=Path, required=True)
    b.add_argument("--versions", default=",".join(VERSIONS))
    b.add_argument("--corpus", type=Path, help="pinned Khronos/SPIRV-Cross source archive")
    b.add_argument("--jobs", type=int, default=4)
    b.add_argument("--target", default="air64-apple-macosx27.0", choices=["air64-apple-macosx27.0"])
    b.add_argument(
        "--tools", type=Path, help="directory containing SweetPad metal and metallib executables"
    )
    c = commands.add_parser("compare")
    for name in ("linux", "xcode", "output"):
        c.add_argument("--" + name, type=Path, required=True)
    c.add_argument("--require-full-corpus", action="store_true")
    args = parser.parse_args()
    return build(args) if args.command == "build" else compare(args)


if __name__ == "__main__":
    sys.exit(main())
