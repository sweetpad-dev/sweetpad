#!/usr/bin/env python3
"""Walk fixtures/ and write a coverage report.

Reports per (corpus project, Xcode-version):
  - metadata: list.json + showsdks.json present?
  - schemes counted from list.json
  - per-scheme: destinations + at least one build-settings JSON
  - raw/: file count
  - build/: count of combos with exit_code == 0, with the key artifacts
    present (xcactivitylog.parsed.json, xcresult.json, index-store.tgz)
  - errors/: lists each errors/*.txt found

Synthetic fixtures (fixtures/_*) are skipped — their layouts don't follow the
corpus capture rubric; scripts/06_audit_coverage.py probes them instead. The
retired dry-run/ captures are not scored (Xcode 26 removed -dry-run; the
surviving files are mostly the unsupported-option error).

Writes:
  fixtures/REPORT.json   machine-readable per-cell numbers
  fixtures/FIXTURES.md   the consolidated report (via common.render_fixtures_md,
                         merged with 06_audit_coverage.py's AUDIT.json)

Flags:
  --report  (no-op flag; kept for compat with the plan invocation)
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass, field, asdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


@dataclass
class CellReport:
    project: str
    xcode_version: str
    exists: bool = False
    list_json: bool = False
    showsdks_json: bool = False
    raw_files: int = 0
    schemes: list[str] = field(default_factory=list)
    schemes_with_destinations: int = 0
    schemes_with_buildsettings: int = 0
    builds_total: int = 0
    builds_ok: int = 0
    builds_with_all_artifacts: int = 0
    errors: list[str] = field(default_factory=list)

    def completeness_pct(self) -> int:
        """A rough 0-100 score for the matrix cell."""
        if not self.exists:
            return 0
        score = 0
        weight = 0
        # 20: list.json
        weight += 20
        if self.list_json:
            score += 20
        # 10: showsdks.json
        weight += 10
        if self.showsdks_json:
            score += 10
        # 30: per-scheme metadata
        scheme_n = max(1, len(self.schemes))
        per_scheme_max = 30 / scheme_n
        weight += 30
        # weight each scheme by 2 axes: destinations, build-settings
        axes = (
            self.schemes_with_destinations,
            self.schemes_with_buildsettings,
        )
        score += sum(a * per_scheme_max / 2 for a in axes)
        # 20: raw inputs
        weight += 20
        if self.raw_files > 0:
            score += 20
        # 20: at least one successful build with all artifacts
        weight += 20
        if self.builds_with_all_artifacts > 0:
            score += 20
        return int(round(100 * score / weight))


def files_in_dir(d: Path, pattern: str = "*") -> list[Path]:
    if not d.exists():
        return []
    return list(d.glob(pattern))


def has_nonempty(p: Path) -> bool:
    try:
        return p.exists() and p.stat().st_size > 0
    except OSError:
        return False


def gather_cell(project_slug: str, xcode_dir: Path) -> CellReport:
    cell = CellReport(project=project_slug, xcode_version=xcode_dir.name.removeprefix("xcode-"))
    if not xcode_dir.exists():
        return cell
    cell.exists = True

    meta_dir = xcode_dir / "metadata"
    cell.showsdks_json = has_nonempty(meta_dir / "showsdks.json")
    # list.json may live directly under metadata/ (single-subproject layout) or
    # under metadata/<subproject>/list.json (tuist-fixtures-style nested
    # layout). Accept either.
    if has_nonempty(meta_dir / "list.json"):
        cell.list_json = True
    elif meta_dir.exists() and any(
        has_nonempty(sub / "list.json")
        for sub in meta_dir.iterdir() if sub.is_dir()
    ):
        cell.list_json = True

    raw_dir = xcode_dir / "raw"
    if raw_dir.exists():
        for root, _, fns in os.walk(raw_dir):
            cell.raw_files += len(fns)

    # Schemes can live directly under metadata/schemes/ OR (tuist-fixtures)
    # under metadata/<fixture>/schemes/. We accept both layouts.
    candidate_schemes_dirs: list[Path] = []
    if (meta_dir / "schemes").exists():
        candidate_schemes_dirs.append(meta_dir / "schemes")
    else:
        # Look for nested layout.
        for sub in meta_dir.iterdir() if meta_dir.exists() else []:
            if sub.is_dir() and (sub / "schemes").exists():
                candidate_schemes_dirs.append(sub / "schemes")

    seen_schemes: list[str] = []
    for sd in candidate_schemes_dirs:
        for scheme_dir in sd.iterdir():
            if not scheme_dir.is_dir():
                continue
            scheme_name = scheme_dir.name
            seen_schemes.append(scheme_name)
            if has_nonempty(scheme_dir / "destinations.json"):
                cell.schemes_with_destinations += 1
            bs_dir = scheme_dir / "build-settings"
            if any(has_nonempty(p) for p in files_in_dir(bs_dir, "*.json")):
                cell.schemes_with_buildsettings += 1
    cell.schemes = seen_schemes

    # Builds
    build_root = xcode_dir / "build"
    if build_root.exists():
        for combo_dir in build_root.iterdir():
            if not combo_dir.is_dir():
                continue
            cell.builds_total += 1
            exit_code_file = combo_dir / "exit_code"
            try:
                ec = exit_code_file.read_text().strip() if exit_code_file.exists() else ""
            except OSError:
                ec = ""
            ok = ec == "0"
            if ok:
                cell.builds_ok += 1
            # `tool-invocations.jsonl` is best-effort: xcodebuild bypasses our
            # PATH shim by resolving toolchain binaries directly under
            # DEVELOPER_DIR, so the JSONL is typically empty for Apple-toolchain
            # builds. The authoritative command-line trace lives in
            # `stdout.txt` ("ExecuteExternalTool" lines) and the raw
            # `xcactivitylog.gz`. Don't penalize cells for that empty file.
            required = [
                combo_dir / "xcactivitylog.parsed.json",
                combo_dir / "xcresult.json",
                combo_dir / "index-store.tgz",
            ]
            if ok and all(has_nonempty(a) for a in required):
                cell.builds_with_all_artifacts += 1

    # Errors
    err_dir = xcode_dir / "errors"
    if err_dir.exists():
        for e in sorted(err_dir.glob("*.txt")):
            cell.errors.append(e.name)

    return cell


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--report", action="store_true",
                    help="(no-op; kept for plan compatibility)")
    _ = ap.parse_args()

    cells: list[CellReport] = []
    if not common.FIXTURES_DIR.exists():
        common.log(f"fixtures/ not found at {common.FIXTURES_DIR}")
        return 1

    for project_dir in sorted(common.FIXTURES_DIR.iterdir()):
        if not project_dir.is_dir() or project_dir.name.startswith("_"):
            continue
        for xcode_dir in sorted(project_dir.iterdir()):
            if not xcode_dir.is_dir() or not xcode_dir.name.startswith("xcode-"):
                continue
            cells.append(gather_cell(project_dir.name, xcode_dir))

    payload = {"cells": [asdict(c) | {"completeness_pct": c.completeness_pct()}
                         for c in cells]}
    (common.FIXTURES_DIR / "REPORT.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n"
    )
    common.log(f"wrote {common.FIXTURES_DIR / 'REPORT.json'}")
    common.render_fixtures_md()

    # Sanity: warn (but don't fail) if any cell is below 50%
    bad = [c for c in cells if c.exists and c.completeness_pct() < 50]
    if bad:
        common.log(f"WARN: {len(bad)} cells under 50% completeness")
        return 0  # still success — FIXTURES.md is the artifact
    return 0


if __name__ == "__main__":
    sys.exit(main())
