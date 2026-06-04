#!/usr/bin/env python3
"""Capture the per-tool compiler/linker argument vectors of a real build.

This is the oracle for the compiler-argument resolver (see
`PLAN_COMPILER_ARGS.md`). We run one real `xcodebuild build` and extract, per
target, the literal command lines xcodebuild executed — the `swiftc` module
invocation, every `clang`/`clang++` compile, and the `ld`/`libtool`/`clang`
link — with their response files expanded. That argv is the most correct
ground truth possible, and the only source that uniformly covers every tool.

Source of truth: **xcodebuild's stdout.** Xcode 26's command-line builds no
longer persist a `.xcactivitylog` (the DerivedData `Logs/Build` manifest comes
back empty), but `xcodebuild` echoes every command it runs verbatim — one
command per line under a `<Phase> … (in target 'T' from project 'P')` header,
the body indented with `cd` / `export` / the tool invocation. We parse those
blocks, shell-tokenize each tool command, and expand:
  - `@<file>` response files (Swift `*.SwiftFileList`, `*-linker-args.resp`),
  - leaving `-filelist <path>` (object lists) and `-output-file-map <path>` as
    raw geometry — recorded, scored out (see the comparator).

The build runs into a dedicated, gitignored `-derivedDataPath` under
`corpus/<slug>/.work/` (the whole `corpus/` tree is gitignored). Only the
extracted argv JSON is committed, under
`fixtures/<slug>/xcode-<ver>/compiler-args/<scheme>__<config>__<dest>.json`,
with **raw, un-canonicalized values** — exactly like the committed
`-showBuildSettings` JSON; the Rust oracle test canonicalizes both sides.

The build is always full (the `-derivedDataPath` is removed first) so the log
holds every compile command rather than "up-to-date" skips.

Flags:
  --slug <name>          corpus project slug (default: alamofire)
  --xcode <ver|slot>     which installed Xcode (default: newest)
  --scheme <name>        scheme to build (required)
  --config <name>        configuration (default: Debug)
  --destination <str>    xcodebuild -destination (default: platform=macOS)
  --dest-slug <name>     filename component for the destination (default:
                         derived from --destination's platform)
  --project <rel>        .xcodeproj path relative to corpus/<slug> (else the
                         lone .xcodeproj is auto-discovered)
  --workspace <rel>      .xcworkspace path relative to corpus/<slug>
  --keep-derived         don't remove the -derivedDataPath before building
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


# Tools whose invocations we capture, by basename of argv[0].
SWIFT_TOOLS = frozenset({"swiftc", "swift-frontend", "swift"})
CLANG_TOOLS = frozenset({"clang", "clang++"})
LINK_TOOLS = frozenset({"ld", "ld64.lld", "libtool"})
# Source extensions that mark a clang invocation as a per-file compile.
CLANG_SOURCE_EXTS = (".c", ".m", ".mm", ".cc", ".cpp", ".cxx", ".C")

# A phase header line: `<Phase> … (in target 'T' from project 'P')`, column 0.
_TARGET_RE = re.compile(r"\(in target '(?P<target>.+?)' from project '(?P<project>.+?)'\)")


@dataclass
class ToolCommand:
    """One tool invocation extracted from the build log."""

    target: str
    phase: str
    tokens: list[str]  # full argv, argv[0] = the tool executable


@dataclass
class TargetArgs:
    """Per-target captured argv, grouped by tool."""

    target: str
    swift: dict | None = None
    clang: dict | None = None
    link: dict | None = None
    # Names of clang sources seen, to keep the per-file list deterministic.
    _clang_files: list[dict] = field(default_factory=list)


# --- build -----------------------------------------------------------------


def discover_project_args(
    corpus_root: Path, project: str | None, workspace: str | None
) -> list[str]:
    """xcodebuild -project/-workspace selector args, resolved under corpus."""
    if workspace:
        return ["-workspace", str(corpus_root / workspace)]
    if project:
        return ["-project", str(corpus_root / project)]
    projs = sorted(corpus_root.glob("*.xcodeproj"))
    if len(projs) == 1:
        return ["-project", str(projs[0])]
    raise SystemExit(
        f"{corpus_root}: pass --project/--workspace "
        f"(found {len(projs)} .xcodeproj at the root)"
    )


def run_build(
    corpus_root: Path,
    xcode: common.XcodeInstall,
    *,
    scheme: str,
    config: str,
    destination: str,
    project: str | None,
    workspace: str | None,
    keep_derived: bool,
) -> tuple[str, Path]:
    """Full `xcodebuild build`; return (stdout, derived-data dir)."""
    work = corpus_root / ".work"
    dd = work / "dd"
    common.ensure_dir(work)
    if not keep_derived and dd.exists():
        shutil.rmtree(dd)

    args = [
        "xcodebuild",
        "build",
        *discover_project_args(corpus_root, project, workspace),
        "-scheme",
        scheme,
        "-configuration",
        config,
        "-destination",
        destination,
        "-derivedDataPath",
        str(dd),
        "CODE_SIGNING_ALLOWED=NO",
    ]
    env = dict(os.environ)
    env["DEVELOPER_DIR"] = str(xcode.developer_dir)
    common.log("$ " + " ".join(shlex.quote(a) for a in args))
    cp = subprocess.run(args, env=env, capture_output=True, text=True, timeout=3600)
    (work / "build.stdout.txt").write_text(cp.stdout)
    (work / "build.stderr.txt").write_text(cp.stderr)
    if cp.returncode != 0:
        tail = "\n".join(cp.stderr.splitlines()[-20:])
        raise SystemExit(f"build failed (exit {cp.returncode}):\n{tail}")
    return cp.stdout, dd


# --- log parsing -----------------------------------------------------------


def parse_tool_commands(stdout: str) -> list[ToolCommand]:
    """Split xcodebuild stdout into per-target tool invocations.

    Each `<Phase> … (in target 'T' …)` header opens a block; its 4-space
    indented body lines are `cd` / `export` / the tool command. We pull the
    command lines, strip the `builtin-… -- ` driver wrapper, shell-tokenize,
    and keep those whose argv[0] is a compiler/linker we recognise.
    """
    lines = stdout.splitlines()
    out: list[ToolCommand] = []
    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]
        m = _TARGET_RE.search(line)
        if not line or line[0].isspace() or not m:
            i += 1
            continue
        target = m.group("target")
        phase = line.split(None, 1)[0].rstrip("\\")
        i += 1
        # Consume the indented body.
        while i < n and (not lines[i] or lines[i][0].isspace()):
            body = lines[i].strip()
            i += 1
            if not body or body.startswith(("cd ", "export ", "cd\t")):
                continue
            cmd = _strip_builtin_wrapper(body)
            if cmd is None:
                continue
            try:
                tokens = shlex.split(cmd)
            except ValueError:
                continue
            if not tokens:
                continue
            tool = Path(tokens[0]).name
            if tool in SWIFT_TOOLS or tool in CLANG_TOOLS or tool in LINK_TOOLS:
                out.append(ToolCommand(target=target, phase=phase, tokens=tokens))
    return out


def _strip_builtin_wrapper(body: str) -> str | None:
    """`builtin-SwiftDriver -- /path/swiftc …` → `/path/swiftc …`.

    Returns the real command for a `builtin-… -- <cmd>` wrapper, the body
    itself when it already starts with an absolute tool path, or None for the
    many non-compiler builtins (`builtin-copy`, `builtin-create-build-directory`,
    `builtin-infoPlistUtility`, …) and shell noise.
    """
    if body.startswith("builtin-"):
        marker = body.find(" -- ")
        if marker == -1:
            return None
        return body[marker + len(" -- ") :]
    if body.startswith("/"):
        return body
    return None


# --- expansion + classification --------------------------------------------


def read_response_file(path: Path) -> list[str] | None:
    """Read an `@response`/`-filelist`/`SwiftFileList` file.

    `*.SwiftFileList` / `*.LinkFileList` are newline-delimited paths; `*.resp`
    is shell-tokenized. Returns None if the file is gone (a stale reference).
    """
    if not path.exists():
        return None
    text = path.read_text()
    if path.suffix in (".SwiftFileList", ".LinkFileList"):
        return [ln.strip() for ln in text.splitlines() if ln.strip()]
    return shlex.split(text)


def expand_swift(tokens: list[str], dd: Path) -> tuple[list[str], list[str]]:
    """Split a swiftc argv into (flag arguments, input .swift files).

    Drops argv[0] (the toolchain path), pulls the `@*.SwiftFileList` (and any
    bare `.swift` tokens) out into `inputFiles`, and splices any other
    `@*.resp` response file inline so `arguments` is the literal flag vector.
    """
    args: list[str] = []
    inputs: list[str] = []
    for tok in tokens[1:]:
        if tok.startswith("@"):
            expanded = read_response_file(Path(tok[1:]))
            if expanded is None:
                args.append(tok)
                continue
            if tok.endswith(".SwiftFileList") or all(
                e.endswith(".swift") for e in expanded
            ):
                inputs.extend(expanded)
            else:
                args.extend(expanded)
        elif tok.endswith(".swift"):
            inputs.append(tok)
        else:
            args.append(tok)
    return args, inputs


def expand_link(tokens: list[str]) -> list[str]:
    """A linker argv (minus argv[0]) with `@*.resp` response files spliced in.

    `-filelist <path>` (the object-file list) is left as-is — it's pure build
    geometry that the comparator scores out.
    """
    out: list[str] = []
    for tok in tokens[1:]:
        if tok.startswith("@"):
            expanded = read_response_file(Path(tok[1:]))
            if expanded is not None:
                out.extend(expanded)
                continue
        out.append(tok)
    return out


def clang_source(tokens: list[str]) -> str | None:
    """The lone compiled source of a clang `-c` invocation, if any."""
    for tok in tokens:
        if tok.endswith(CLANG_SOURCE_EXTS) and not tok.startswith("-"):
            return tok
    return None


def classify(cmd: ToolCommand, dd: Path, by_target: dict[str, TargetArgs]) -> None:
    """Fold one tool command into its target's grouped argv."""
    ta = by_target.setdefault(cmd.target, TargetArgs(target=cmd.target))
    tool = Path(cmd.tokens[0]).name

    if tool in SWIFT_TOOLS:
        if ta.swift is not None:
            return  # one canonical module invocation per target (the first)
        args, inputs = expand_swift(cmd.tokens, dd)
        ta.swift = {"arguments": args, "inputFiles": inputs}
        return

    if tool in CLANG_TOOLS and "-c" in cmd.tokens and clang_source(cmd.tokens):
        src = clang_source(cmd.tokens)
        ta._clang_files.append({"file": src, "arguments": cmd.tokens[1:]})
        return

    # Anything else from clang/clang++/ld/libtool is a link step.
    if tool in CLANG_TOOLS or tool in LINK_TOOLS:
        if ta.link is not None:
            return
        ta.link = {"tool": tool, "arguments": expand_link(cmd.tokens)}


def fold_clang_common(ta: TargetArgs) -> None:
    """Collapse per-file clang argv to `{commonArguments, files:[{file, extra}]}`.

    The shared flag set (everything identical across every file, minus the
    per-file `-c <src>` / `-o <obj>`) is stored once; each file keeps only its
    own delta. Keeps app targets with hundreds of `.m` files compact.
    """
    files = ta._clang_files
    if not files:
        return
    common_set = set(files[0]["arguments"])
    for f in files[1:]:
        common_set &= set(f["arguments"])
    common_args = [a for a in files[0]["arguments"] if a in common_set]
    per_file = []
    for f in files:
        extra = [a for a in f["arguments"] if a not in common_set]
        per_file.append({"file": f["file"], "extraArguments": extra})
    ta.clang = {"commonArguments": common_args, "files": per_file}


# --- output ----------------------------------------------------------------


def arch_of(by_target: dict[str, TargetArgs]) -> str:
    """Pull the build arch out of a swiftc `-target <arch>-apple-…` triple."""
    for ta in by_target.values():
        if ta.swift:
            args = ta.swift["arguments"]
            if "-target" in args:
                triple = args[args.index("-target") + 1]
                return triple.split("-", 1)[0]
    return ""


def build_oracle(
    by_target: dict[str, TargetArgs],
    *,
    slug: str,
    xcode_version: str,
    scheme: str,
    config: str,
    destination: str,
    sdk: str,
) -> dict:
    targets = []
    for name in sorted(by_target):
        ta = by_target[name]
        fold_clang_common(ta)
        entry: dict = {"target": name}
        if ta.swift is not None:
            entry["swift"] = ta.swift
        if ta.clang is not None:
            entry["clang"] = ta.clang
        if ta.link is not None:
            entry["link"] = ta.link
        # Skip targets that produced no compiler/linker commands at all.
        if len(entry) > 1:
            targets.append(entry)
    return {
        "slug": slug,
        "xcode_version": xcode_version,
        "scheme": scheme,
        "configuration": config,
        "destination": destination,
        "sdk": sdk,
        "arch": arch_of(by_target),
        "targets": targets,
    }


def sdk_from_destination(destination: str) -> str:
    """Map `platform=macOS` → `macosx`, `platform=iOS Simulator` → `iphonesimulator`, …."""
    m = re.search(r"platform=([^,]+)", destination)
    plat = (m.group(1) if m else "macOS").strip().lower()
    table = {
        "macos": "macosx",
        "ios": "iphoneos",
        "ios simulator": "iphonesimulator",
        "tvos": "appletvos",
        "tvos simulator": "appletvsimulator",
        "watchos": "watchos",
        "watchos simulator": "watchsimulator",
        "visionos": "xros",
        "visionos simulator": "xrsimulator",
    }
    return table.get(plat, plat.replace(" ", ""))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--slug", default="alamofire")
    ap.add_argument("--xcode", help="version or slot (default: newest installed)")
    ap.add_argument("--scheme", required=True)
    ap.add_argument("--config", default="Debug")
    ap.add_argument("--destination", default="platform=macOS")
    ap.add_argument("--dest-slug", help="filename component for the destination")
    ap.add_argument("--project", help=".xcodeproj path relative to corpus/<slug>")
    ap.add_argument("--workspace", help=".xcworkspace path relative to corpus/<slug>")
    ap.add_argument("--keep-derived", action="store_true")
    args = ap.parse_args()

    installs = common.discover_installed_xcodes()
    xcodes = common.selected_xcodes(installs, args.xcode)
    xcode = xcodes[0]

    corpus_root = common.CORPUS_DIR / args.slug
    if not corpus_root.exists():
        raise SystemExit(
            f"{corpus_root} not found — clone it first "
            f"(git clone … corpus/{args.slug} at the manifest SHA)"
        )

    with common.with_xcode(xcode):
        stdout, dd = run_build(
            corpus_root,
            xcode,
            scheme=args.scheme,
            config=args.config,
            destination=args.destination,
            project=args.project,
            workspace=args.workspace,
            keep_derived=args.keep_derived,
        )

    commands = parse_tool_commands(stdout)
    common.log(f"parsed {len(commands)} tool invocations from the build log")
    by_target: dict[str, TargetArgs] = {}
    for cmd in commands:
        classify(cmd, dd, by_target)

    sdk = sdk_from_destination(args.destination)
    oracle = build_oracle(
        by_target,
        slug=args.slug,
        xcode_version=xcode.version,
        scheme=args.scheme,
        config=args.config,
        destination=args.destination,
        sdk=sdk,
    )

    dest_slug = args.dest_slug or sdk_from_destination(args.destination)
    out_dir = common.fixture_dir(args.slug, xcode.version) / "compiler-args"
    common.ensure_dir(out_dir)
    fname = f"{common.slug(args.scheme)}__{common.slug(args.config)}__{common.slug(dest_slug)}.json"
    out_path = out_dir / fname
    with out_path.open("w") as f:
        json.dump(oracle, f, indent=2)
        f.write("\n")

    n_swift = sum(1 for t in oracle["targets"] if "swift" in t)
    n_clang = sum(len(t["clang"]["files"]) for t in oracle["targets"] if "clang" in t)
    n_link = sum(1 for t in oracle["targets"] if "link" in t)
    common.log(
        f"WROTE {out_path} — {len(oracle['targets'])} target(s), "
        f"{n_swift} swift, {n_clang} clang file(s), {n_link} link"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
