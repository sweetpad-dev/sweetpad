#!/usr/bin/env python3
"""Capture the resolution view of each .xcconfig in the corpus.

`-xcconfig FILE` layers an xcconfig on top of the normal resolution chain at
the top-most precedence. Running `xcodebuild -xcconfig FILE -showBuildSettings`
against the same project/scheme/configuration/destination — once with and
once without — reveals exactly what that xcconfig contributes after
xcodebuild has interpreted its conditionals, includes, and modifier syntax.

For each captured .xcconfig (under `corpus/<slug>/`), we find the nearest
ancestor `.xcodeproj` and run xcodebuild with that project + a representative
scheme/config/destination derived from the existing baseline captures in
`metadata/schemes/.../build-settings/`.

Output:
  fixtures/<slug>/xcode-<ver>/metadata/<sub>/_xcconfig_resolution/
      <slugified-relative-path>.json   captured with -xcconfig FILE applied
      <slugified-relative-path>.meta.json   base command used + xcconfig path

We don't capture the "without" view here — that's already in the existing
`metadata/schemes/.../build-settings/` files. The resolver can diff.

Skips xcconfigs that have no buildable ancestor project. Idempotent (skips
existing outputs unless --force). Best-effort: per-xcconfig failures are
logged but don't abort the run.

Flags:
  --project <slug>   restrict to one corpus slug
  --xcode <ver>      restrict to one Xcode version
  --force            re-capture even if outputs exist
  --max <N>          cap total xcconfig captures per fixture (default: 50)
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


def find_xcconfigs(slug: str) -> list[Path]:
    """Find all .xcconfig files under the corpus tree for a slug."""
    root = common.CORPUS_DIR / slug
    if not root.exists():
        return []
    SKIP = {".git", "DerivedData", ".derived", ".build", "node_modules",
            "Pods", "Carthage", ".swiftpm", "tmp", ".tuist-cache"}
    found: list[Path] = []
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        dirnames[:] = [d for d in dirnames if d not in SKIP]
        for fn in filenames:
            if fn.endswith(".xcconfig"):
                found.append(Path(dirpath) / fn)
    return sorted(found)


def find_nearest_project(xcconfig: Path, slug_root: Path) -> Path | None:
    """Walk up from the xcconfig looking for a .xcodeproj sibling.

    Returns the first .xcodeproj found at or above the xcconfig's directory
    (without crossing out of `slug_root`).
    """
    current = xcconfig.parent.resolve()
    slug_root = slug_root.resolve()
    while True:
        if not str(current).startswith(str(slug_root)):
            break
        candidates = sorted(p for p in current.iterdir()
                            if p.is_dir() and p.suffix == ".xcodeproj")
        if candidates:
            return candidates[0]
        if current == slug_root:
            break
        current = current.parent
    return None


def find_baseline_capture(fixture_root: Path, project: Path) -> dict | None:
    """Find an existing build-settings*.json that used the same project.

    We use its target name as a hint to derive (scheme, configuration,
    destination). Returns the parsed JSON entry's metadata (we synthesize
    a command from it).
    """
    # The existing per-scheme captures live under metadata/schemes/<S>/build-settings/.
    # We don't strictly need to parse them; we just want to find a scheme name
    # and a destination that worked.
    schemes_dir = fixture_root / "metadata" / "schemes"
    if not schemes_dir.exists():
        # Try nested (tuist-fixtures)
        nested = list((fixture_root / "metadata").glob("*/schemes"))
        if not nested:
            return None
        schemes_dir = nested[0]
    for scheme_dir in sorted(schemes_dir.iterdir()):
        if not scheme_dir.is_dir():
            continue
        bs_dir = scheme_dir / "build-settings"
        if not bs_dir.exists():
            continue
        for bs in sorted(bs_dir.glob("*.json")):
            # Filename like "Debug__platform-iOS-Simulator_OS26.0_iPhone-16.json"
            # We need the scheme name + a destination.
            try:
                data = json.loads(bs.read_text())
            except Exception:
                continue
            if not isinstance(data, list) or not data:
                continue
            # Read scheme from path; config and dest from filename
            scheme = scheme_dir.name
            fname = bs.stem  # e.g. "Debug__platform-iOS-Simulator_..."
            if "__" not in fname:
                continue
            config, _, dest_slug = fname.partition("__")
            return {"scheme": scheme, "config": config, "dest_slug": dest_slug, "bs_path": bs}
    return None


def reconstruct_destination(dest_slug: str, fixture_root: Path, scheme: str) -> str | None:
    """Re-derive a `-destination` string from the slugged filename.

    The slug is `<key>-<value>_<key>-<value>_...`. We need a real id-based
    destination, which we get from
    `metadata/schemes/<S>/destinations.json`.
    """
    # Try root-level then nested layouts (tuist-fixtures nests one level deep).
    candidates: list[Path] = [fixture_root / "metadata" / "schemes" / scheme / "destinations.json"]
    for nested in (fixture_root / "metadata").glob(f"*/schemes/{scheme}"):
        candidates.append(nested / "destinations.json")
    for candidate in candidates:
        if not candidate.exists():
            continue
        try:
            dests = json.loads(candidate.read_text())
        except Exception:
            continue
        # Pick the first non-placeholder simulator or macOS dest.
        for d in dests:
            plat = d.get("platform", "")
            if plat == "macOS":
                return "platform=macOS"
            if "id" in d and "Placeholder" not in d.get("id", "") \
                and "placeholder" not in d.get("id", "") \
                and not d.get("name", "").startswith("Any "):
                return f"platform={plat},id={d['id']}"
    return None


def xb_env(xcode: common.XcodeInstall) -> dict[str, str]:
    e = dict(os.environ)
    e["DEVELOPER_DIR"] = str(xcode.developer_dir)
    return e


def capture_xcconfig_resolution(
    xcconfig: Path, project: Path, scheme: str, config: str, dest: str,
    out_path: Path, *, xcode: common.XcodeInstall,
) -> tuple[bool, str]:
    cmd = [
        "xcodebuild", "-showBuildSettings", "-json",
        "-xcconfig", str(xcconfig),
        "-project", str(project),
        "-scheme", scheme,
        "-configuration", config,
        "-destination", dest,
    ]
    cp = subprocess.run(cmd, env=xb_env(xcode), capture_output=True, text=True,
                        timeout=180)
    if cp.returncode != 0 or not cp.stdout.strip():
        return False, (cp.stderr or cp.stdout)[-400:]
    try:
        parsed = json.loads(cp.stdout)
    except json.JSONDecodeError as e:
        return False, f"non-JSON: {e}"
    with out_path.open("w") as f:
        json.dump(parsed, f, indent=2, sort_keys=True)
        f.write("\n")
    return True, str(out_path)


def process_fixture(slug: str, fixture_root: Path, xcode: common.XcodeInstall,
                     *, force: bool, max_per_fixture: int) -> None:
    xcconfigs = find_xcconfigs(slug)
    if not xcconfigs:
        common.log(f"  no .xcconfig files in corpus/{slug}/")
        return
    slug_root = common.CORPUS_DIR / slug
    common.log(f"  {len(xcconfigs)} xcconfig files found")

    captured = 0
    skipped_exists = 0
    skipped_no_proj = 0
    failed = 0
    for xcconfig in xcconfigs:
        if captured >= max_per_fixture:
            break

        rel = xcconfig.relative_to(slug_root)
        out_dir = fixture_root / "metadata" / "_xcconfig_resolution"
        out_dir.mkdir(parents=True, exist_ok=True)
        slug_name = common.slug(str(rel).replace("/", "__"))
        out_path = out_dir / f"{slug_name}.json"
        meta_path = out_dir / f"{slug_name}.meta.json"

        if out_path.exists() and not force:
            skipped_exists += 1
            continue

        project = find_nearest_project(xcconfig, slug_root)
        if not project:
            # xcconfig lives in a part of the corpus we didn't generate/select
            # (e.g. tuist fixtures we didn't pick) — there's no ancestor
            # .xcodeproj to test against. Skip silently.
            skipped_no_proj += 1
            continue

        baseline = find_baseline_capture(fixture_root, project)
        if not baseline:
            skipped_no_proj += 1
            continue

        dest = reconstruct_destination(baseline["dest_slug"], fixture_root, baseline["scheme"])
        if not dest:
            failed += 1
            continue

        ok, info = capture_xcconfig_resolution(
            xcconfig, project, baseline["scheme"], baseline["config"], dest,
            out_path, xcode=xcode,
        )
        if ok:
            captured += 1
            with meta_path.open("w") as f:
                json.dump({
                    "xcconfig": str(rel),
                    "project": str(project.relative_to(slug_root)),
                    "scheme": baseline["scheme"],
                    "configuration": baseline["config"],
                    "destination": dest,
                }, f, indent=2, sort_keys=True)
                f.write("\n")
        else:
            failed += 1
            common.log(f"    failed for {rel}: {info[-200:]}")
    common.log(f"  captured={captured} exists={skipped_exists} no-project={skipped_no_proj} failed={failed}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--project", help="restrict to one corpus slug")
    ap.add_argument("--xcode", help="restrict to one Xcode version (e.g. 26.0.1)")
    ap.add_argument("--force", action="store_true")
    ap.add_argument("--max", type=int, default=50,
                    help="cap total xcconfig captures per fixture (default: 50)")
    args = ap.parse_args()

    installs = common.discover_installed_xcodes()
    xcodes = common.selected_xcodes(installs, args.xcode)

    fixtures_root = common.FIXTURES_DIR
    if not fixtures_root.exists():
        common.log("no fixtures/")
        return 1

    for project_dir in sorted(fixtures_root.iterdir()):
        if not project_dir.is_dir() or project_dir.name.startswith("_"):
            continue
        slug = project_dir.name
        if args.project and slug != args.project:
            continue
        for x in xcodes:
            xcode_dir = project_dir / f"xcode-{x.version}"
            if not xcode_dir.exists():
                continue
            common.log(f"\n==> {slug} / xcode {x.version}")
            try:
                with common.with_xcode(x):
                    process_fixture(slug, xcode_dir, x,
                                    force=args.force, max_per_fixture=args.max)
            except Exception as e:
                common.log(f"ERROR {slug}/{x.version}: {e}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
