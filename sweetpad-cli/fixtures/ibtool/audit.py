#!/usr/bin/env python3
"""Inventory IB documents, command options and Xcode's installed IB metadata.

Source mentions are evidence for investigation, not implementation coverage.
Use the oracle fixtures and runtime tests to establish actual compatibility.
The report includes every corpus platform, not just documents we can compile.
"""
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import xml.etree.ElementTree as ET

REPO = Path(__file__).resolve().parents[3]
PRUNED = {".git", ".build", "node_modules", "Pods", "Carthage", "DerivedData",
          "build", "target", "linux-products"}


def documents(corpus):
    for base, dirs, files in os.walk(corpus):
        dirs[:] = sorted(d for d in dirs if d not in PRUNED
                         and not (Path(base) == corpus and d.startswith("_")))
        for name in sorted(files):
            if name.endswith((".storyboard", ".xib")):
                path = Path(base) / name
                yield path, ET.parse(path).getroot()


def strings(source):
    return set(re.findall(r'"([^"\n]*)"', source))


def inventory(corpus, source):
    literals = strings(source)
    records = []
    attributes = defaultdict(lambda: {"occurrences": 0, "files": set(), "values": Counter()})
    children = Counter()
    for path, document in documents(corpus):
        relative = str(path.relative_to(corpus))
        runtime = document.get("targetRuntime", "unspecified")
        elements = Counter(e.tag for e in document.iter())
        records.append({"path": relative, "runtime": runtime,
                        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                        "elements": dict(elements)})
        for element in document.iter():
            for child in element:
                children[(runtime, element.tag, child.tag)] += 1
            for key, value in element.attrib.items():
                entry = attributes[(runtime, element.tag, key)]
                entry["occurrences"] += 1
                entry["files"].add(relative)
                entry["values"][value] += 1
    pairs = []
    for (runtime, element, attribute), record in sorted(attributes.items()):
        pairs.append({"runtime": runtime, "element": element, "attribute": attribute,
                      "occurrences": record["occurrences"], "files": sorted(record["files"]),
                      "values": dict(record["values"]),
                      "attribute_name_mentioned_in_source": attribute in literals,
                      "status": "requires oracle evidence; source mention is not coverage"})
    return {"documents": records, "attributes": pairs,
            "child_relationships": [{"runtime": r, "parent": p, "child": c, "occurrences": n}
                                    for (r, p, c), n in sorted(children.items())]}


def command_options(contents, source):
    manual = subprocess.run(["man", "ibtool"], capture_output=True, check=True).stdout
    manual = subprocess.run(["col", "-b"], input=manual, capture_output=True, check=True).stdout.decode()
    options = set(re.findall(r"--[a-z][a-z-]*", manual))
    xcspec = contents / "SharedFrameworks/SwiftBuild.framework/Versions/A/PlugIns/SWBBuildService.bundle/Contents/PlugIns/SWBApplePlatformPlugin.bundle/Contents/Frameworks/SWBApplePlatform.framework/Versions/A/Resources"
    specs = {}
    for name in ["IBCompiler.xcspec", "IBStoryboardCompiler.xcspec", "IBStoryboardLinker.xcspec"]:
        path = xcspec / name
        if path.exists():
            specs[name] = sorted(set(re.findall(r"--[a-z][a-z-]*", path.read_text())))
            options.update(specs[name])
    module = (REPO / "sweetpad-cli/src/cli/ibtool/mod.rs").read_text()
    accepted = set(re.findall(r'"(--[a-z][a-z-]*)"', module))
    return {"manual": manual, "xcspec_options": specs,
            "options": [{"option": option,
                         "parser_mentions_option": option in accepted,
                         "status": "requires semantic probe" if option in accepted else "unimplemented"}
                        for option in sorted(options | accepted)]}


def xcode_metadata(contents, source):
    literals = strings(source)
    classes, properties = [], []
    for framework in sorted((contents / "PlugIns").glob("IDEInterfaceBuilder*Integration.framework")):
        resources = framework / "Versions/A/Resources"
        for path in sorted(resources.glob("*.classdescriptions")):
            data = plistlib.loads(path.read_bytes())
            if isinstance(data, list):
                for entry in data:
                    if isinstance(entry, dict) and "ClassName" in entry:
                        classes.append({"class": entry["ClassName"], "superclass": entry.get("SuperClass"),
                                        "source": str(path), "archive_class_name_mentioned_in_source": entry["ClassName"] in literals,
                                        "status": "requires minimal fixture; a class name is not implementation evidence"})
        for path in sorted(resources.glob("*.inspector")):
            try:
                root = ET.parse(path).getroot()
            except ET.ParseError:
                continue
            for element in root.iter("property"):
                key = element.get("configurableProperty") or element.get("keyPath")
                if key:
                    properties.append({"inspector": path.name, "framework": framework.name,
                                       "property": key, "type": element.get("type"),
                                       "source": str(path),
                                       "status": "inspector property; XML spelling and archive encoding require probes"})
    return {"classes": classes, "inspector_properties": properties}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, default=REPO / "sweetpad-lib/corpus")
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    source_paths = sorted((REPO / "sweetpad-cli/src/cli/ibtool").rglob("*.rs"))
    source = "\n".join(p.read_text() for p in source_paths if "tests" not in p.parts)
    developer = Path(subprocess.check_output(["xcode-select", "-p"], text=True).strip())
    contents = developer.parent
    report = inventory(args.corpus.resolve(), source)
    commands = command_options(contents, source)
    (args.out / "ibtool-man.txt").write_text(commands.pop("manual"))
    report.update({"recorded_at": datetime.now(timezone.utc).isoformat(),
                   "source_hashes": {str(p.relative_to(REPO)): hashlib.sha256(p.read_bytes()).hexdigest()
                                     for p in source_paths},
                   "commands": commands, "xcode_metadata": xcode_metadata(contents, source),
                   "scope": "Complete inventory of the current corpus, installed IB inspectors/class descriptions, man page and build xcspec options. Not proof of all possible ibtool behavior."})
    report["summary"] = {"documents": dict(Counter(d["runtime"] for d in report["documents"])),
                         "element_attribute_pairs": len(report["attributes"]),
                         "command_options": len(commands["options"]),
                         "class_descriptions": len(report["xcode_metadata"]["classes"]),
                         "inspector_properties": len(report["xcode_metadata"]["inspector_properties"])}
    (args.out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["summary"], indent=2))


if __name__ == "__main__":
    main()
