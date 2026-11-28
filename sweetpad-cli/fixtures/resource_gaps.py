"""Find resource combinations in a checkout that lack a checked-in source fixture.

Compare keyed Interface Builder children on their owning elements, and the
selector combinations of populated asset slots. Counts describe fixture
inputs. Confirm candidate gaps with Xcode and the runtime before treating
them as compiler bugs. Inline Rust fixtures and generated catalogs are outside
this inventory.
"""
import argparse
from collections import defaultdict
import json
import os
from pathlib import Path

from ibtool.audit import PRUNED, documents

HERE = Path(__file__).resolve().parent


def interface_features(root):
    found = defaultdict(set)
    for path, document in documents(root):
        runtime = document.get("targetRuntime", "unspecified")
        for parent in document.iter():
            for child in parent:
                if child.get("key"):
                    feature = (runtime, parent.tag, child.tag, child.attrib["key"])
                    found[feature].add(path.relative_to(root).as_posix())
    return found


def asset_features(root):
    found = defaultdict(set)
    for base, dirs, files in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in PRUNED)
        folder = Path(base)
        if "Contents.json" not in files or not any(p.suffix == ".xcassets" for p in folder.parents):
            continue
        contents = json.loads((folder / "Contents.json").read_text())
        for group in ("images", "colors", "symbols", "data"):
            for slot in contents.get(group, []):
                if not slot.get("filename") and not slot.get("color"):
                    continue
                selectors = []
                for key, value in sorted(slot.items()):
                    if key in {"filename", "color"}:
                        continue
                    if key == "appearances":
                        value = sorted((a["appearance"], a["value"]) for a in value)
                    value = value if isinstance(value, str) else json.dumps(value, sort_keys=True, separators=(',', ':'))
                    selectors.append(f"{key}={value}")
                kind = Path(slot.get("filename", "")).suffix.lower() or "color"
                feature = (folder.suffix, group, kind, *selectors)
                found[feature].add(folder.relative_to(root).as_posix())
    return found


def compare(observed, fixtures, limit):
    missing = sorted(observed.keys() - fixtures.keys(), key=lambda f: (-len(observed[f]), f))
    return {"observed": len(observed), "present_in_fixtures": len(observed) - len(missing),
            "missing": len(missing),
            "examples": [{"feature": feature, "files": len(observed[feature]), "example": min(observed[feature])}
                         for feature in missing[:limit]]}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkout", type=Path)
    parser.add_argument("--limit", type=int, default=10, help="maximum examples per tool")
    parser.add_argument("--require-covered", action="store_true", help="fail if an observed combination has no fixture")
    args = parser.parse_args()
    root = args.checkout.resolve()
    if not root.is_dir() or args.limit < 0:
        parser.error("provide an existing checkout and a nonnegative example limit")
    report = {
        "ibtool": compare(interface_features(root), interface_features(HERE / "ibtool"), args.limit),
        "actool": compare(asset_features(root), asset_features(HERE / "actool/resources"), args.limit),
    }
    print(json.dumps(report, indent=2))
    if args.require_covered and any(tool["missing"] for tool in report.values()):
        raise SystemExit(1)
