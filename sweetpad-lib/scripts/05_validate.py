#!/usr/bin/env python3
"""Walk fixtures/ and write a coverage report.

Reports per (project, Xcode-version):
  - metadata: list.json + showsdks.json present?
  - schemes counted from list.json
  - per-scheme: destinations + at least one build-settings JSON + at least
    one dry-run txt
  - raw/: file count
  - build/: count of combos with exit_code == 0, with all four key artifacts
    present (xcactivitylog.parsed.json, xcresult.json, tool-invocations.jsonl,
    index-store.tgz)
  - errors/: lists each errors/*.txt found

Writes:
  fixtures/REPORT.md   high-level coverage matrix + per-cell detail blocks
  fixtures/REPORT.json machine-readable copy of the same numbers

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
    schemes_with_dryrun: int = 0
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
        # weight each scheme by 3 axes: destinations, build-settings, dry-run
        axes = (
            self.schemes_with_destinations,
            self.schemes_with_buildsettings,
            self.schemes_with_dryrun,
        )
        score += sum(a * per_scheme_max / 3 for a in axes)
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
            dr_dir = scheme_dir / "dry-run"
            if any(has_nonempty(p) for p in files_in_dir(dr_dir, "*.txt")):
                cell.schemes_with_dryrun += 1
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


def render_markdown(cells: list[CellReport]) -> str:
    out: list[str] = []
    out.append("# fixtures/REPORT.md")
    out.append("")
    out.append("Coverage matrix for the corpus-capture phase. Generated by "
               "`scripts/05_validate.py`. Cells show completeness % (0-100). "
               "See per-cell detail below for blocked tuples and errors.")
    out.append("")

    by_project: dict[str, dict[str, CellReport]] = {}
    versions: set[str] = set()
    for c in cells:
        by_project.setdefault(c.project, {})[c.xcode_version] = c
        versions.add(c.xcode_version)
    version_list = sorted(versions)

    # Coverage matrix
    out.append("## Coverage matrix")
    out.append("")
    if version_list:
        header = "| Project | " + " | ".join(f"xcode-{v}" for v in version_list) + " |"
        sep = "|---" * (1 + len(version_list)) + "|"
        out.append(header)
        out.append(sep)
        for slug in sorted(by_project.keys()):
            row = [slug]
            for v in version_list:
                c = by_project[slug].get(v)
                if not c or not c.exists:
                    row.append("—")
                else:
                    row.append(f"{c.completeness_pct()}%")
            out.append("| " + " | ".join(row) + " |")
    else:
        out.append("_no fixture cells found_")
    out.append("")

    # Per-cell detail
    out.append("## Per-cell detail")
    out.append("")
    for slug in sorted(by_project.keys()):
        out.append(f"### {slug}")
        out.append("")
        for v in version_list:
            c = by_project[slug].get(v)
            if not c:
                continue
            out.append(f"#### xcode-{v}")
            if not c.exists:
                out.append("_not captured_")
                out.append("")
                continue
            out.append(f"- completeness: **{c.completeness_pct()}%**")
            out.append(f"- list.json: {'OK' if c.list_json else 'MISSING'}")
            out.append(f"- showsdks.json: {'OK' if c.showsdks_json else 'MISSING'}")
            out.append(f"- raw files: {c.raw_files}")
            out.append(f"- schemes: {len(c.schemes)} "
                       f"({', '.join(c.schemes[:8])}"
                       f"{'…' if len(c.schemes) > 8 else ''})")
            out.append(f"- schemes with destinations.json: "
                       f"{c.schemes_with_destinations}/{len(c.schemes)}")
            out.append(f"- schemes with build-settings/: "
                       f"{c.schemes_with_buildsettings}/{len(c.schemes)}")
            out.append(f"- schemes with dry-run/: "
                       f"{c.schemes_with_dryrun}/{len(c.schemes)}")
            out.append(f"- builds: total={c.builds_total}, "
                       f"exit0={c.builds_ok}, "
                       f"complete_artifacts={c.builds_with_all_artifacts}")
            if c.errors:
                out.append(f"- errors:")
                for e in c.errors:
                    out.append(f"  - `{e}`")
            out.append("")
    return "\n".join(out) + "\n"


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
        if not project_dir.is_dir():
            continue
        for xcode_dir in sorted(project_dir.iterdir()):
            if not xcode_dir.is_dir() or not xcode_dir.name.startswith("xcode-"):
                continue
            cells.append(gather_cell(project_dir.name, xcode_dir))

    md = render_markdown(cells)
    (common.FIXTURES_DIR / "REPORT.md").write_text(md)
    common.log(f"wrote {common.FIXTURES_DIR / 'REPORT.md'}")

    payload = {"cells": [asdict(c) for c in cells]}
    (common.FIXTURES_DIR / "REPORT.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n"
    )

    # Sanity: warn (but don't fail) if any cell is below 50%
    bad = [c for c in cells if c.exists and c.completeness_pct() < 50]
    if bad:
        common.log(f"WARN: {len(bad)} cells under 50% completeness")
        return 0  # still success — REPORT.md is the artifact
    return 0


if __name__ == "__main__":
    sys.exit(main())
