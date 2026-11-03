#!/usr/bin/env python3
"""Compare the local IB corpus with Xcode. Requires a built sweetpad binary.

    python3 sweetpad-cli/fixtures/ibtool/sweep.py --out /tmp/ib-sweep

Each run uses a new output directory. report.json records every input, command,
exit code and diagnostic. Successful documents also use the Rust tests'
structural comparator; compilation alone does not count as a match.
"""

import argparse
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import difflib
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import xml.etree.ElementTree as ET

REPO = Path(__file__).resolve().parents[3]
PRUNED = {".git", ".build", "node_modules", "linux-products", "Pods",
          "Carthage", "DerivedData", "build", "target"}


def inputs(corpus, runtime):
    for base, dirs, files in os.walk(corpus):
        dirs[:] = sorted(d for d in dirs if d not in PRUNED
                         and not (Path(base) == corpus and d.startswith("_")))
        for name in sorted(files):
            if name.endswith((".storyboard", ".xib")):
                path = Path(base) / name
                document = ET.parse(path).getroot()
                if runtime == "all" or document.get("targetRuntime") == runtime:
                    yield path, document


def run(command, log, env=None):
    try:
        result = subprocess.run(command, cwd=REPO, env=env,
                                capture_output=True, text=True, timeout=180)
        code, stdout, stderr = result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired as error:
        code = 124
        stdout = (error.stdout or b"")
        stderr = (error.stderr or b"")
        stdout = stdout.decode(errors="replace") if isinstance(stdout, bytes) else stdout
        stderr = stderr.decode(errors="replace") if isinstance(stderr, bytes) else stderr
        stderr += "\nTimed out after 180 seconds"
    log.with_suffix(".stdout").write_text(stdout)
    log.with_suffix(".stderr").write_text(stderr)
    return {"command": command, "exit_code": code,
            "stdout": stdout, "stderr": stderr}


def inventory(document):
    """Feature presence, including features hidden behind the first error.

    These counts are not independent compiler failures or coverage scores.
    """
    features = Counter()
    if document.get("type") == "com.apple.InterfaceBuilder3.CocoaTouch.XIB":
        features["XIB documents"] += 1
    for element in document.iter():
        if element.tag.endswith("Controller") and element.tag != "viewController":
            features[f"controller: {element.tag}"] += 1
        if element.get("customClass") and element.tag != "placeholder":
            features[f"custom classes: {element.tag}"] += 1
        if element.tag in {"outlet", "outletCollection", "action", "segue",
                           "variation", "userDefinedRuntimeAttribute"}:
            features[element.tag] += 1
        if element.tag == "subviews":
            for view in element:
                if view.tag not in {"view", "label", "imageView"}:
                    features[f"view: {view.tag}"] += 1
        if element.tag == "fontDescription":
            kind = element.get("type", "system")
            weight = element.get("weight")
            if not ((kind == "system" and weight in {None, "regular", "light"})
                    or (kind == "boldSystem" and weight is None)):
                features[f"font: {kind} {weight or ''}".strip()] += 1
            if element.get("name") or element.get("style"):
                features["custom or text-style fonts"] += 1
        if element.tag == "color" and element.get("name"):
            features["named colors"] += 1
        if element.tag == "imageView" and element.get("catalog"):
            features["symbol images"] += 1
    return dict(sorted(features.items()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", default="iOS.CocoaTouch", choices=["all", "iOS.CocoaTouch", "MacOSX.Cocoa", "AppleTV", "watchKit"])
    parser.add_argument("--corpus", type=Path, default=REPO / "sweetpad-lib/corpus")
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/sweetpad")
    parser.add_argument("--linux-vm", help="Run the stand-in in this OrbStack VM; --binary is its Linux path")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    corpus, out = args.corpus.resolve(), args.out.resolve()
    binary = args.binary if args.linux_vm else args.binary.resolve()
    if not binary.is_absolute():
        parser.error("--binary must be an absolute path on the Linux VM")
    prefix = ["orb", "-m", args.linux_vm] if args.linux_vm else []
    if args.linux_vm:
        subprocess.run([*prefix, "test", "-x", str(binary)], check=True)
    elif not binary.is_file():
        parser.error(f"build sweetpad first: missing {binary}")
    out.mkdir(parents=True, exist_ok=False)
    (out / "ibtool").symlink_to(binary)
    comparison = out / "comparison"
    (comparison / "expected").mkdir(parents=True)
    (comparison / "actual").mkdir()
    dump = out / "canonical"
    dump.mkdir()
    rows = []
    for path, document in inputs(corpus, args.runtime):
        relative = str(path.relative_to(corpus))
        name = hashlib.sha256(relative.encode()).hexdigest()[:12] + "_" + path.stem
        directory = out / "cases" / name
        (directory / "ours").mkdir(parents=True)
        (directory / "xcode").mkdir()
        rows.append({"path": relative, "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                     "name": name, "kind": path.suffix[1:], "runtime": document.get("targetRuntime"),
                     "features": inventory(document)})

    def compile_case(row):
        source = corpus / row["path"]
        directory = out / "cases" / row["name"]
        platform_flags = {"iOS.CocoaTouch": ["--target-device", "iphone", "--target-device", "ipad", "--minimum-deployment-target", "18.0"],
                          "MacOSX.Cocoa": ["--minimum-deployment-target", "12.0"],
                          "AppleTV": ["--target-device", "tv", "--minimum-deployment-target", "18.0"],
                          "watchKit": ["--target-device", "watch", "--minimum-deployment-target", "10.0"]}[row["runtime"]]
        flags = ["--errors", "--warnings", "--notices", "--module", "App", *platform_flags,
                 "--output-format", "human-readable-text", "--auto-activate-custom-fonts"]
        row["ours"] = run([*prefix, str(out / "ibtool"), *flags, "--compilation-directory",
                           str(directory / "ours"), str(source)], directory / "ours-log")
        compiled = source.stem + (".storyboardc" if row["kind"] == "storyboard" else ".nib")
        row["compiled_name"] = compiled
        row["xcode"] = run(["xcrun", "ibtool", *flags, "--compile",
                            str(directory / "xcode" / compiled), str(source)],
                           directory / "xcode-log")
        if row["ours"]["exit_code"] == row["xcode"]["exit_code"] == 0:
            shutil.copyfile(source, comparison / (row["name"] + source.suffix))
            suffix = ".storyboardc" if row["kind"] == "storyboard" else ".nib"
            copy = shutil.copytree if row["kind"] == "storyboard" else shutil.copyfile
            copy(directory / "xcode" / compiled, comparison / "expected" / (row["name"] + suffix))
            copy(directory / "ours" / compiled, comparison / "actual" / (row["name"] + suffix))
        print(f"{row['path']}: sweetpad={row['ours']['exit_code']} "
              f"xcode={row['xcode']['exit_code']}", flush=True)
        return row

    # ibtool uses a service shared by its processes. Keep the batch small.
    with ThreadPoolExecutor(max_workers=2) as pool:
        rows = list(pool.map(compile_case, rows))
    env = dict(os.environ, IBTOOL_FIXTURES=str(comparison), IBTOOL_DUMP=str(dump),
               IBTOOL_ACTUAL=str(comparison / "actual"))
    harness = run(["cargo", "test", "-p", "sweetpad-cli", "--lib",
                   "cli::ibtool::tests::each_document_compiles_as_xcode_compiles_it",
                   "--", "--exact", "--nocapture"], out / "comparison-log", env)
    for row in rows:
        row["structural_match"] = None
        if row["ours"]["exit_code"] or row["xcode"]["exit_code"]:
            continue
        directory = out / "cases" / row["name"]
        ours = directory / "ours" / row["compiled_name"]
        xcode = directory / "xcode" / row["compiled_name"]
        differences = []
        if row["kind"] == "xib":
            got = (dump / f"{row['name']}-document.nib.got").read_text()
            want = (dump / f"{row['name']}-document.nib.want").read_text()
            differences.extend(difflib.unified_diff(want.splitlines(True), got.splitlines(True),
                                                   fromfile="Xcode/document.nib", tofile="Sweetpad/document.nib"))
        elif sorted(p.name for p in ours.iterdir()) != sorted(p.name for p in xcode.iterdir()):
            differences.append("Compiled file names differ")
        else:
            for file in sorted(ours.iterdir()):
                if file.suffix == ".plist":
                    # Use the same UUID-aware metadata comparison as the Rust
                    # harness. Controller UUIDs change on every native compile.
                    got = (dump / f"{row['name']}-{file.name}.got").read_text()
                    want = (dump / f"{row['name']}-{file.name}.want").read_text()
                    if got != want:
                        differences.append(f"{file.name}: canonical plist values differ")
                else:
                    got = (dump / f"{row['name']}-{file.name}.got").read_text()
                    want = (dump / f"{row['name']}-{file.name}.want").read_text()
                    differences.extend(difflib.unified_diff(want.splitlines(True), got.splitlines(True),
                                                          fromfile=f"Xcode/{file.name}",
                                                          tofile=f"Sweetpad/{file.name}"))
        row["structural_match"] = not differences
        (directory / "structural.diff").write_text("".join(differences))
    feature_files, feature_occurrences = defaultdict(list), Counter()
    errors = Counter()
    for row in rows:
        for feature, count in row["features"].items():
            feature_files[feature].append(row["path"])
            feature_occurrences[feature] += count
        if row["ours"]["exit_code"]:
            error = (row["ours"]["stdout"] + row["ours"]["stderr"]).strip()
            error = re.split(r": error: ", error, maxsplit=1)[-1]
            error = re.sub(r"[A-Za-z0-9]{3}-[A-Za-z0-9]{2}-[A-Za-z0-9]{3}", "<id>", error)
            errors[error] += 1
    report = {"recorded_at": datetime.now(timezone.utc).isoformat(),
              "corpus": str(corpus), "binary": str(binary), "linux_vm": args.linux_vm,
              "summary": {"total": len(rows), "kinds": dict(Counter(r["kind"] for r in rows)),
                          "compiled": sum(r["ours"]["exit_code"] == 0 for r in rows),
                          "xcode_compiled": sum(r["xcode"]["exit_code"] == 0 for r in rows),
                          "structural_matches": sum(r["structural_match"] is True for r in rows)},
              "platforms": {runtime: {"total": len(group), "compiled": sum(r["ours"]["exit_code"] == 0 for r in group), "structural_matches": sum(r["structural_match"] is True for r in group)} for runtime in sorted({r["runtime"] for r in rows}) for group in [[r for r in rows if r["runtime"] == runtime]]},
              "first_errors": dict(errors.most_common()),
              "feature_presence": {f: {"file_count": len(paths), "occurrences": feature_occurrences[f],
                                          "files": paths}
                                   for f, paths in sorted(feature_files.items(), key=lambda item: -len(item[1]))},
              "comparison_harness": harness, "files": rows}
    (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["summary"], indent=2))
    print(json.dumps(report["first_errors"], indent=2))
    print(f"Report: {out / 'report.json'}")
    return 0 if rows and harness["exit_code"] == 0 and all(row["structural_match"] is True for row in rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
