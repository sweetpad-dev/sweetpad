#!/usr/bin/env python3
"""Check the selected iOS SDK against the versioned registration checklist."""

import argparse
from collections import defaultdict
import difflib
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
BASELINE = "27.2"
CHECKLIST = HERE / f"sdk-{BASELINE}.md"
EXAMPLES = {"result.OpensIntent+ProvidesDialog", "result.OpensIntent+ShowsSnippetView", "summary.table"}


def checklist():
    rows = {}
    for line in CHECKLIST.read_text().splitlines():
        if not line.startswith("| `"):
            continue
        name, status, evidence = [c.strip() for c in line.strip("|").split("|")]
        name = name.strip("`")
        links = re.findall(r"\[[^\]]+\]\(([^)]+)\)", evidence)
        if (name in rows or status not in {"Match", "Diagnostic", "No effect"} or not links
                or not all((HERE / link.split("#")[0]).is_file() for link in links)):
            raise ValueError(f"Invalid or duplicate checklist entry: {line}")
        rows[name] = status
    return rows


def inventory(graphs):
    symbols = {}
    edges = []
    for graph in graphs:
        for symbol in graph["symbols"]:
            symbols[symbol["identifier"]["precise"]] = symbol
        edges.extend((r["source"], r["target"]) for r in graph["relationships"]
                     if r["kind"] in {"conformsTo", "inheritsFrom"})
    roots = {s["pathComponents"][0]: uid for uid, s in symbols.items()
             if len(s["pathComponents"]) == 1 and s["kind"]["identifier"] == "swift.protocol"}

    def descendants(name):
        found = {roots[name]}
        while True:
            more = {src for src, dst in edges if dst in found} - found
            if not more:
                return found
            found.update(more)

    result_types = descendants("IntentResult")
    summaries = descendants("ParameterSummary") - {roots["ParameterSummary"]}
    aliases = {}
    keys = set(EXAMPLES)
    declarations = defaultdict(set)

    def record(s):
        path = ".".join(s["pathComponents"])
        signature = "".join(f["spelling"] for f in s["declarationFragments"])
        declarations[path].add(s["identifier"]["precise"] + " " + signature)

    for s in symbols.values():
        p = s["pathComponents"]
        if p[:-1] != ["AppIntent"] or s["kind"]["identifier"] != "swift.typealias":
            continue
        # The first type after '=' is the alias target, before generic constraints.
        target = next((f for f in s["declarationFragments"] if f["kind"] == "typeIdentifier"), None)
        if target and (target.get("preciseIdentifier") in summaries or "ParameterSummary" in target["spelling"]):
            aliases[target["spelling"]] = p[-1]
            keys.add("summary." + p[-1])
            record(s)

    for uid, s in symbols.items():
        kind, p = s["kind"]["identifier"], s["pathComponents"]
        owner, name = p[:-1], p[-1]
        if kind == "swift.protocol" and uid in result_types:
            keys.add("result." + name)
        elif kind == "swift.init" and owner in (["IntentParameter"], ["String", "IntentInputOptions"]):
            family = "parameter" if owner == ["IntentParameter"] else "inputOptions"
            keys.update(f"{family}.{label}" for label in re.findall(r"(\w+):", name))
        elif kind == "swift.enum.case" and owner == ["InputConnectionBehavior"]:
            keys.add("connection." + name)
        elif kind == "swift.enum.case" and owner[:2] == ["String", "IntentInputOptions"]:
            keys.add(".".join(["inputOptions", *p[2:]]))
        elif len(owner) == 1 and owner[0] in aliases and kind == "swift.init":
            if aliases[owner[0]] == "Summary":
                keys.add("summary.Summary" + name.removeprefix("init"))
        elif uid in summaries and kind in {"swift.struct", "swift.enum", "swift.class", "swift.protocol"}:
            keys.add("summary." + aliases.get(name, name))
        else:
            continue
        record(s)
    return keys, declarations


def version_number(version):
    return tuple(map(int, version.split(".")))


def reviewed_baseline(version, suffix, directory):
    versions = {version_number(f.name.split("-")[1]): f for f in directory.glob(f"sdk-*-{suffix}")}
    current = version_number(version)
    candidates = [v for v in versions if v <= current]
    primary = version_number(BASELINE)
    if not candidates or (current >= primary and primary not in versions):
        raise ValueError(f"No reviewed {suffix} baseline for iOS {version}")
    return versions[max(candidates)]


def check(keys, declarations, version, update=False, directory=HERE, export=None):
    rows = checklist()
    newer = version_number(version) >= version_number(BASELINE)
    # Older SDKs can legitimately expose a subset of the most recent checklist.
    errors = [f"Unclassified SDK declaration: {k}" for k in sorted(keys - rows.keys())]
    if newer:
        errors += [f"Checklist declaration missing from SDK: {k}" for k in sorted(rows.keys() - keys)]
    lines = []
    for name, overloads in sorted(declarations.items()):
        digest = hashlib.sha256("\n".join(sorted(overloads)).encode()).hexdigest()[:20]
        lines.append(f"{name}\t{len(overloads)}\t{digest}\n")
    name = f"sdk-{version}-signatures.txt"
    if export:
        export.mkdir(parents=True, exist_ok=True)
        (export / name).write_text("".join(lines))
    if errors:
        raise ValueError("\n".join(errors))
    if update:
        (directory / name).write_text("".join(lines))
    else:
        baseline = reviewed_baseline(version, "signatures.txt", directory)
        previous = baseline.read_text().splitlines(keepends=True)
        if previous != lines:
            diff = difflib.unified_diff(previous, lines, fromfile=baseline.name, tofile=f"iOS {version} SDK")
            raise ValueError("SDK overloads changed. Review the named declarations and their evidence.\n" + "".join(diff))
    print(f"iOS {version}: {len(keys)} classified entries, {sum(map(len, declarations.values()))} declarations")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--update-signatures", action="store_true", help="record reviewed overloads for the selected SDK")
    parser.add_argument("--export-signatures", type=Path, help="export this SDK's declaration inventory for review")
    args = parser.parse_args()
    sdk = subprocess.check_output(["xcrun", "--sdk", "iphoneos", "--show-sdk-path"], text=True).strip()
    version = subprocess.check_output(["xcrun", "--sdk", "iphoneos", "--show-sdk-version"], text=True).strip()
    with tempfile.TemporaryDirectory(prefix="sweetpad-intents-sdk-") as tmp:
        path = Path(tmp)
        subprocess.run(["xcrun", "--sdk", "iphoneos", "swift-symbolgraph-extract", "-module-name", "AppIntents",
                        "-target", f"arm64-apple-ios{version}", "-sdk", sdk, "-minimum-access-level", "public",
                        "-output-dir", str(path)], check=True)
        keys, declarations = inventory(json.loads(f.read_text()) for f in sorted(path.glob("*.symbols.json")))
        check(keys, declarations, version, args.update_signatures, export=args.export_signatures)


if __name__ == "__main__":
    main()
