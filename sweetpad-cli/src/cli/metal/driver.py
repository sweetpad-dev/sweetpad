#!/usr/bin/env python3
"""Experimental native Linux metal and metallib drivers. Requires stock LLVM 14."""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from library import Module, TARGET, library, wrapped
from msl import ATTR, attributes, parse, stdlib
from stages import GPU_TYPES, LAYOUT, Metadata, Stage, integer, string


MODES = [(2, 4), (3, 0), (3, 1), (3, 2), (4, 0), (4, 1)]


def tool(name):
    path = shutil.which(name + "-14")
    if not path:
        raise ValueError(f"{name}-14 is required (install clang-14 and llvm-14)")
    return path


def run(command, **options):
    result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **options)
    if result.returncode:
        raise ValueError(result.stderr.decode(errors="replace").strip() or f"{command[0]} failed")
    return result.stdout


def target_info(target):
    match = TARGET.fullmatch(target)
    if not match:
        raise ValueError(f"unsupported Metal target '{target}'")
    osname, major, minor, sim = match.groups()
    version = int(major)
    floors = [12, 13, 14, 15, 26, 27] if osname == "macosx" else [15, 16, 17, 18, 26, 27]
    if version not in floors:
        raise ValueError(f"unsupported Metal deployment target {target}")
    level = sum(version >= v for v in floors) + 3
    triple = f"air64_v2{level}-apple-{osname}{major}.{minor or '0'}.0{sim or ''}"
    return triple, level


def compile_air(args, work):
    triple, level = target_info(args.target)
    if args.std is None:
        default = MODES[level - 4]
        args.std = f"metal{default[0]}.{default[1]}"
    std = re.fullmatch(r"(?:(ios|macos)-)?metal(\d+)\.(\d+)", args.std)
    if not std:
        raise ValueError(f"unsupported Metal language mode '{args.std}'")
    language = int(std[2]), int(std[3])
    if language not in MODES:
        raise ValueError(f"unsupported Metal language mode '{args.std}'")
    if std[1] and (std[1] == "macos") != ("macosx" in triple):
        raise ValueError("Metal standard and deployment platform disagree")
    source = args.inputs[0]
    for name in ("metal_stdlib", "metal_raytracing", "simd/simd.h"):
        header = work / name
        header.parent.mkdir(exist_ok=True)
        header.touch()
    version = language[0] * 100 + language[1] * 10
    clang = tool("clang++")
    cpp_version = "-std=c++17" if language >= (4, 0) else "-std=c++14"
    base = [clang, "-x", "c++", "-target", "spir64", cpp_version, "-nostdinc"]
    defines = [f"-D{d}" for d in args.D] + [f"-I{i}" for i in args.I] + [f"-F{f}" for f in args.F]
    dependencies = []
    if args.MMD:
        if not args.MF:
            raise ValueError("-MMD requires -MF")
        dependencies = ["-MMD", "-MT", args.MT or str(args.o), "-MF", work / "deps"]
    pre = run(
        [
            *base,
            "-E",
            "-P",
            "-isystem",
            work,
            f"-D__METAL_VERSION__={version}",
            *defines,
            *dependencies,
            source,
        ]
    ).decode()
    if re.search(r"\bdouble\b", pre):
        raise ValueError("double is not a supported Metal value type")
    for expr in re.finditer(r"\bnew\b", pre):
        if language < (4, 1):
            raise ValueError("placement new requires MSL 4.1")
        if not pre[expr.end() :].lstrip().startswith("(") or not pre[: expr.start()].rstrip().endswith(
            (";", "{", "}")
        ):
            raise ValueError(
                "only placement construction in thread storage with a discarded result is implemented"
            )
    structs, aliases, entries, source_text = parse(pre, language)
    md = Metadata()
    stages = [Stage(e, structs, aliases, md) for e in entries]
    src = work / "shader.cpp"
    # All attributed interfaces must have been lowered above. Reject stray semantic annotations.
    supported = {
        "vertex_id",
        "instance_id",
        "base_vertex",
        "base_instance",
        "primitive_id",
        "thread_position_in_grid",
        "thread_position_in_threadgroup",
        "thread_index_in_threadgroup",
        "threadgroup_position_in_grid",
        "threads_per_threadgroup",
        "threadgroups_per_grid",
        "barycentric_coord",
        "position",
        "user",
        "flat",
        "center_perspective",
        "center_no_perspective",
        "color",
        "raster_order_group",
        "buffer",
        "payload",
        "stage_in",
        "attribute",
    }
    for group in ATTR.finditer(source_text):
        if not attributes(group[0]).keys() <= supported | {"maybe_unused"}:
            raise ValueError(f"unsupported Metal annotation {group[0]}")
    src.write_text(
        stdlib(language)
        + GPU_TYPES
        + "\n"
        + "\n".join(dict.fromkeys(f for s in stages for f in s.forwards))
        + "\n"
        + ATTR.sub("", source_text)
        + "\n"
        + "\n".join(dict.fromkeys(h for s in stages for h in s.helpers))
        + "\n"
        + "\n".join(s.cpp() for s in stages)
    )
    math = ["-ffast-math"] if args.math_mode == "fast" else ["-ffp-contract=off"]
    diag = ["--serialize-diagnostics", args.diagnostics] if args.diagnostics else []
    ir = run(
        [
            *base,
            "-S",
            "-emit-llvm",
            "-O1",
            "-fno-exceptions",
            "-fno-rtti",
            "-fno-strict-aliasing",
            *math,
            *diag,
            "-fno-ident",
            "-Werror=unknown-attributes",
            "-Werror=undefined-inline",
            "-ferror-limit=2",
            src,
            "-o",
            "-",
        ]
    ).decode()
    ir = ir.replace("spir_func ", "")
    ir = re.sub(r"^define (.*?)void @__sp_impl_", r"define internal \1void @__sp_impl_", ir, flags=re.M)
    if any('%"struct.metal::_imageblock_base"' in t for s in stages for t, p in s.parameters):
        ir += '\n%struct._imageblock_t = type opaque\n%"struct.metal::_imageblock_base" = type { %struct._imageblock_t addrspace(4)* }\n'
    ir += "\n" + "\n".join(s.llvm() for s in stages)
    md.add("version", integer(2), integer(level), integer(0))
    md.add("language_version", string("Metal"), integer(language[0]), integer(language[1]), integer(0))
    md.add("compile_options", string("air.compile.framebuffer_fetch_enable"))
    if args.math_mode == "fast":
        md.add("compile_options", string("air.compile.fast_math_enable"))
    merged = work / "merged.ll"
    merged.write_text(ir + "\n" + md.emit() + "\n")
    optimized = run([tool("opt"), "-S", "-O2", merged, "-o", "-"]).decode()
    optimized = re.sub(r'^target triple = ".*"', f'target triple = "{triple}"', optimized, flags=re.M)
    optimized = re.sub(
        r'^target datalayout = ".*"', f'target datalayout = "{LAYOUT}"', optimized, flags=re.M
    )
    optimized = re.sub(r'^source_filename = ".*"', 'source_filename = "metal"', optimized, flags=re.M)
    for decl in re.findall(r"^declare [^\n]*?@([^ (]+)", optimized, re.M):
        if not decl.startswith(("llvm.", "air.")):
            raise ValueError(f"unsupported external Metal function '{decl}'")
    return wrapped(run([tool("llvm-as"), "-o", "-", "-"], input=optimized.encode()))


def main(argv):
    if not argv:
        raise ValueError("usage: driver.py {metal|metallib} [options]")
    parser = argparse.ArgumentParser(prog=argv[0], allow_abbrev=False)
    parser.add_argument(
        "--version", "-v", action="version", version="SweetPad experimental Metal compiler (LLVM 14)"
    )
    parser.add_argument("-o", required=True, type=Path)
    parser.add_argument("inputs", nargs="+", type=Path)
    if argv[0] == "metal":
        parser.add_argument("-c", action="store_true")
        parser.add_argument("-target", required=True)
        parser.add_argument("-std")
        parser.add_argument("-D", action="append", default=[])
        parser.add_argument("-I", action="append", default=[])
        parser.add_argument("-F", action="append", default=[])
        parser.add_argument("-isysroot")
        parser.add_argument("-MMD", action="store_true")
        parser.add_argument("-MT")
        parser.add_argument("-MF", type=Path)
        parser.add_argument("-serialize-diagnostics", dest="diagnostics")
        parser.add_argument(
            "-fmetal-math-mode", dest="math_mode", choices=["safe", "fast"], default="safe"
        )
        parser.add_argument("-fmetal-math-fp32-functions", choices=["precise", "fast"])
        parser.add_argument("-gline-tables-only", "-frecord-sources", dest="debug", action="store_true")
    elif argv[0] != "metallib":
        raise ValueError("select metal or metallib")
    args = parser.parse_args(argv[1:])
    with tempfile.TemporaryDirectory(prefix="sweetpad-metal-") as tmp:
        if argv[0] == "metal" and args.c:
            if len(args.inputs) != 1:
                raise ValueError("metal -c expects one source file")
            data = compile_air(args, Path(tmp))
            if args.debug:
                print(
                    "sweetpad metal: source-level Metal debugging is unavailable in this compiler",
                    file=sys.stderr,
                )
            if args.MMD:
                atomic_write(args.MF, (Path(tmp) / "deps").read_bytes())
        else:
            if any(p.suffix != ".air" for p in args.inputs):
                raise ValueError("linking requires .air inputs; compile .metal sources with -c")
            modules = [Module.read(p, tool("llvm-dis")) for p in args.inputs]
            if argv[0] == "metal" and target_info(args.target)[0] != modules[0].target:
                raise ValueError("Metal link target disagrees with AIR input")
            data = library(modules)
        atomic_write(args.o, data)


def atomic_write(path, data):
    try:
        mode = path.stat().st_mode & 0o777
    except FileNotFoundError:
        mask = os.umask(0)
        os.umask(mask)
        mode = 0o666 & ~mask
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as out:
        temp = out.name
        try:
            out.write(data)
            os.fchmod(out.fileno(), mode)
            out.close()
            os.replace(temp, path)
        finally:
            if os.path.exists(temp):
                os.unlink(temp)


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"sweetpad metal: {error}")
