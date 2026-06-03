#!/usr/bin/env python3
"""Capture project-default + per-target build settings.

The scheme-level capture in `02_capture_metadata.py` shows settings as they
resolve through a scheme's selected target(s) and dependencies. This script
captures two complementary views the resolver needs:

  1. **Project defaults**: `xcodebuild -showBuildSettings -json` against each
     `.xcodeproj` with NO `-scheme`/`-target` and NO `-configuration`. This
     is the layer between SDK defaults (xcspecs) and target overrides — what
     the project file alone contributes.

     Note: xcodebuild still demands a configuration in some cases, so we run
     it twice: once with no args, once with `-configuration <default>` so the
     resolver can compare.

  2. **Per-target settings**: for each named target in the project,
     `xcodebuild -showBuildSettings -json -target <T> -configuration <C>` per
     (target, configuration). This is the "isolated target" view — no
     dependency resolution, no scheme aggregation. It's the cleanest oracle
     for "what settings does this single target have, after merging xcconfigs
     and pbxproj entries but before xcodebuild aggregates anything?"

Outputs (per fixture, per Xcode):

  fixtures/<slug>/xcode-<ver>/metadata/<sub>/_project_defaults/
      project-only.json                       no -scheme/-target/-configuration
      project-only__<config>.json             no -scheme/-target, only -configuration
  fixtures/<slug>/xcode-<ver>/metadata/<sub>/_per_target/
      <target>__<config>.json                 -target T -configuration C

`<sub>` is "" for non-Tuist projects (one subproject == fixture root) and
the fixture name for tuist-fixtures (e.g. `examples_xcode_generated_ios_app`).

Idempotent: skips outputs that already exist unless `--force`.
Best-effort: failures per target/config don't abort the run.

Flags:
  --project <slug>    restrict to one corpus slug
  --xcode <ver|slot>  restrict to one Xcode
  --force             redo outputs even if they exist
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

# Reuse helpers from 02_capture_metadata via importlib (sibling file with a
# numeric prefix — can't `import 02_capture_metadata` directly).
import importlib.util
_meta_path = Path(__file__).resolve().parent / "02_capture_metadata.py"
_spec = importlib.util.spec_from_file_location("capture_metadata", _meta_path)
_meta = importlib.util.module_from_spec(_spec)
sys.modules["capture_metadata"] = _meta
_spec.loader.exec_module(_meta)
Subproject = _meta.Subproject
find_subprojects = _meta.find_subprojects
xb_env = _meta.xb_env
run_xb = _meta.run_xb
capture_list = _meta.capture_list
discover_configurations = _meta.discover_configurations


def find_xcodeprojs(sub) -> list[Path]:
    """Return every `.xcodeproj` actually inside this subproject tree.

    For workspace-based subprojects this can be multiple projects. For
    project-based subprojects it's just the project itself.
    """
    if sub.project_kind == "project":
        return [sub.project_path]
    return sorted({
        p for p in sub.root.rglob("*.xcodeproj")
        if "project.xcworkspace" not in p.parts
        and ".derived" not in p.parts
        and ".build" not in p.parts
        and ".swiftpm" not in p.parts
    })


def list_targets(proj: Path, *, xcode) -> list[str]:
    """Use `xcodebuild -list -json -project P` to read target names."""
    cp = run_xb(["-list", "-json", "-project", str(proj)], xcode=xcode, timeout=60)
    if cp.returncode != 0:
        return []
    try:
        data = json.loads(cp.stdout)
    except json.JSONDecodeError:
        return []
    return list((data.get("project") or {}).get("targets") or [])


def capture_project_defaults(proj: Path, configs: list[str], out_dir: Path,
                              *, xcode, force: bool) -> int:
    out_dir.mkdir(parents=True, exist_ok=True)
    wrote = 0

    # 1) -showBuildSettings with no scheme, no target, no configuration.
    bs_path = out_dir / "project-only.json"
    if force or not bs_path.exists():
        cp = run_xb(["-showBuildSettings", "-json", "-project", str(proj)],
                    xcode=xcode, timeout=120)
        if cp.returncode == 0 and cp.stdout.strip():
            try:
                parsed = json.loads(cp.stdout)
                with bs_path.open("w") as f:
                    json.dump(parsed, f, indent=2, sort_keys=True)
                    f.write("\n")
                wrote += 1
            except json.JSONDecodeError:
                bs_path.with_suffix(".txt").write_text(cp.stdout)

    # 2) Same per configuration (no target, no scheme).
    for config in configs:
        path = out_dir / f"project-only__{common.slug(config)}.json"
        if not force and path.exists():
            continue
        cp = run_xb(["-showBuildSettings", "-json", "-project", str(proj),
                     "-configuration", config], xcode=xcode, timeout=120)
        if cp.returncode != 0 or not cp.stdout.strip():
            continue
        try:
            parsed = json.loads(cp.stdout)
            with path.open("w") as f:
                json.dump(parsed, f, indent=2, sort_keys=True)
                f.write("\n")
            wrote += 1
        except json.JSONDecodeError:
            pass
    return wrote


def capture_per_target(proj: Path, targets: list[str], configs: list[str],
                        out_dir: Path, *, xcode, force: bool) -> int:
    out_dir.mkdir(parents=True, exist_ok=True)
    wrote = 0
    for target in targets:
        for config in configs:
            path = out_dir / f"{common.slug(target)}__{common.slug(config)}.json"
            if not force and path.exists():
                continue
            cp = run_xb(["-showBuildSettings", "-json", "-project", str(proj),
                         "-target", target, "-configuration", config],
                        xcode=xcode, timeout=120)
            if cp.returncode != 0 or not cp.stdout.strip():
                continue
            try:
                parsed = json.loads(cp.stdout)
                with path.open("w") as f:
                    json.dump(parsed, f, indent=2, sort_keys=True)
                    f.write("\n")
                wrote += 1
            except json.JSONDecodeError:
                pass
    return wrote


def process_subproject(project, sub, *, xcode, force: bool) -> None:
    sub_label_dir = sub.label or ""
    meta_root = common.metadata_dir(project.slug, xcode.version)
    per_sub_meta = meta_root / sub_label_dir if sub_label_dir else meta_root
    per_sub_meta.mkdir(parents=True, exist_ok=True)

    projects = find_xcodeprojs(sub)
    if not projects:
        common.log(f"  no .xcodeproj under {sub.root}")
        return

    configs = discover_configurations(sub, xcode=xcode)

    for proj in projects:
        rel = proj.relative_to(sub.root)
        proj_slug = common.slug(str(rel).replace("/", "_").removesuffix(".xcodeproj"))
        common.log(f"  proj: {rel} ({proj_slug})")

        # Project defaults
        defaults_dir = per_sub_meta / "_project_defaults" / proj_slug
        wrote_def = capture_project_defaults(
            proj, configs, defaults_dir, xcode=xcode, force=force,
        )
        common.log(f"    project-defaults: wrote {wrote_def}")

        # Per-target
        targets = list_targets(proj, xcode=xcode)
        if not targets:
            common.log(f"    no targets in {rel}")
            continue
        per_target_dir = per_sub_meta / "_per_target" / proj_slug
        wrote_pt = capture_per_target(
            proj, targets, configs, per_target_dir, xcode=xcode, force=force,
        )
        common.log(f"    per-target: {len(targets)} targets × {len(configs)} configs, wrote {wrote_pt}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--project", help="restrict to one corpus slug")
    ap.add_argument("--xcode", help="restrict to one Xcode (version or slot)")
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()

    installs = common.discover_installed_xcodes()
    xcodes = common.selected_xcodes(installs, args.xcode)
    projects = common.selected_projects(args.project)

    had_error = False
    for project in projects:
        for x in xcodes:
            common.log(f"\n========= {project.slug} :: xcode {x.version} =========")
            try:
                with common.with_xcode(x):
                    try:
                        subs = find_subprojects(project)
                    except Exception as e:
                        common.log(f"  discover failed: {e}")
                        continue
                    for sub in subs:
                        common.log(f"==> {project.slug} | {x.version} | "
                                   f"{sub.label or sub.project_path.name}")
                        process_subproject(project, sub, xcode=x, force=args.force)
            except Exception as e:
                had_error = True
                common.log(f"ERROR {project.slug}/{x.version}: {e}")
    return 1 if had_error else 0


if __name__ == "__main__":
    sys.exit(main())
