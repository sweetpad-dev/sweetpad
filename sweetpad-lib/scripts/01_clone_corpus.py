#!/usr/bin/env python3
"""Clone the 5 corpus projects at pinned refs and write corpus/manifest.json.

Pin strategy:
  - "latest-release": pick the highest semver-looking tag returned by
    `git ls-remote --tags`. Tags like "v3.2.1" / "3.2.1" / "release/3.2.1"
    are all accepted; release-candidate / beta / preview tags are filtered out.
  - "default-branch": use HEAD of the upstream default branch.

For `tuist-fixtures`, this script clones the Tuist repo at its latest release
tag and selects 5-8 representative fixtures from `fixtures/` (covering: basic
app, framework dep, multi-platform, static linkage, dynamic linkage,
resources). It then runs `tuist install && tuist generate` per selected
fixture so subsequent steps see real `.xcodeproj` files.

Idempotent: a project whose `corpus/<slug>/` already exists at the recorded
SHA is skipped. Pass `--force` to re-clone.

Flags:
  --project <slug>    only operate on one project
  --force             re-clone even if the directory exists at the right SHA
  --skip-tuist        skip the Tuist-fixtures generate step (useful when
                      iterating)
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


# Tuist's repo layout: fixtures live under examples/xcode/ (as of 4.195.x).
# Earlier scaffolding's `fixtures/` directory no longer exists. Patterns are
# case-insensitive substring matches against the fixture directory name; order
# matters — earlier entries claim a slot first.
TUIST_FIXTURE_ROOTS: tuple[str, ...] = ("examples/xcode", "fixtures")
TUIST_FIXTURE_PATTERNS: list[tuple[str, list[str]]] = [
    ("basic-app", ["buildable_folders", "organization_name_project"]),
    ("frameworks", ["framework_and_tests", "ios_app_with_framework_linking"]),
    ("multiplatform", ["ios_app_with_watchapp2", "macos_app_with_extensions",
                       "ios_app_with_extensions"]),
    ("static-linkage", ["ios_app_with_static_frameworks"]),
    ("dynamic-linkage", ["dynamic_frameworks_linking_static_frameworks",
                          "command_line_tool_with_dynamic_framework"]),
    ("resources", ["ios_app_with_static_framework_with_xcstrings",
                    "static_library_with_string_resources"]),
    ("spm-deps", ["ios_app_with_spm_dependencies",
                   "local_package_with_traits"]),
    ("schemes", ["custom_scheme", "test_plan"]),
    # Plan-B additions covering audit gaps:
    # `custom_configuration` (singular) defines lowercase config names
    # ("debug", "release") — flips the "Non-Debug/Release configuration"
    # probe. `custom_default_configuration` (plural) only tweaks default
    # settings under standard config names; doesn't help here.
    ("non-default-config", ["ios_app_with_custom_configuration",
                              "custom_default_configuration"]),
    ("static-library", ["ios_app_with_static_libraries"]),
    ("dynamic-library", ["command_line_tool_with_dynamic_library"]),
    ("coredata", ["ios_app_with_coredata"]),
]
TUIST_FIXTURE_MIN = 5
TUIST_FIXTURE_MAX = 14


_SEMVER_RE = re.compile(
    r"(?:^|/)v?(\d+)\.(\d+)(?:\.(\d+))?(?:-([A-Za-z0-9.-]+))?$"
)


def ls_remote_tags(repo: str) -> list[str]:
    cp = common.run(
        ["git", "ls-remote", "--tags", "--refs", repo],
        capture=True, quiet=True,
    )
    tags: list[str] = []
    for line in cp.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) != 2:
            continue
        ref = parts[1]
        if not ref.startswith("refs/tags/"):
            continue
        tags.append(ref[len("refs/tags/"):])
    return tags


def pick_latest_release(tags: list[str]) -> str | None:
    """Return the highest-semver, non-pre-release tag, or None."""
    best: tuple[int, int, int] | None = None
    best_tag: str | None = None
    for t in tags:
        m = _SEMVER_RE.search(t)
        if not m:
            continue
        pre = m.group(4) or ""
        if pre and re.search(r"(rc|beta|alpha|preview|dev|nightly|snapshot)", pre, re.I):
            continue
        major = int(m.group(1))
        minor = int(m.group(2))
        patch = int(m.group(3) or 0)
        key = (major, minor, patch)
        if best is None or key > best:
            best = key
            best_tag = t
    return best_tag


def default_branch(repo_dir: Path) -> str:
    cp = common.run(
        ["git", "-C", str(repo_dir), "symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        capture=True, quiet=True, check=False,
    )
    if cp.returncode == 0:
        # e.g. "origin/main"
        return cp.stdout.strip().split("/", 1)[1]
    # fallback
    return "main"


def head_sha(repo_dir: Path) -> str:
    return common.run_capture(["git", "-C", str(repo_dir), "rev-parse", "HEAD"])


def shallow_clone(repo: str, dest: Path, ref: str) -> None:
    """Clone `repo` into `dest` at exactly `ref` (tag or branch). Re-clones if
    `dest` already exists — caller controls that decision.
    """
    if dest.exists():
        shutil.rmtree(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    # Use full clone — shallow is fragile for tag resolution across hosts.
    common.run(["git", "clone", "--filter=blob:none", repo, str(dest)])
    common.run(["git", "-C", str(dest), "checkout", "--detach", ref])


def pick_tuist_fixtures(repo_root: Path) -> list[tuple[str, Path]]:
    """Pick 5-8 fixtures covering distinct shapes. Returns list of (category, path).

    Scans the first existing root in TUIST_FIXTURE_ROOTS for child directories.
    """
    fixtures_root: Path | None = None
    for rel in TUIST_FIXTURE_ROOTS:
        cand = repo_root / rel
        if cand.exists():
            fixtures_root = cand
            break
    if fixtures_root is None:
        return []
    all_dirs = sorted(d for d in fixtures_root.iterdir() if d.is_dir())
    picks: list[tuple[str, Path]] = []
    claimed: set[str] = set()

    for category, patterns in TUIST_FIXTURE_PATTERNS:
        # Walk patterns in priority order. For each pattern, scan dirs and
        # claim the first match. Only if pattern[i] yields nothing do we try
        # pattern[i+1]. This makes the "preferred" pattern win even if an
        # alphabetically-earlier dir matches a fallback pattern.
        chosen: Path | None = None
        for pattern in patterns:
            for d in all_dirs:
                if d.name in claimed:
                    continue
                if pattern in d.name.lower():
                    chosen = d
                    break
            if chosen is not None:
                break
        if chosen is not None:
            picks.append((category, chosen))
            claimed.add(chosen.name)
        if len(picks) >= TUIST_FIXTURE_MAX:
            break

    # If we under-claimed, pad with anything that looks app-shaped
    if len(picks) < TUIST_FIXTURE_MIN:
        for d in all_dirs:
            if d.name in claimed:
                continue
            if (d / "Project.swift").exists() or (d / "Workspace.swift").exists():
                picks.append(("other", d))
                claimed.add(d.name)
                if len(picks) >= TUIST_FIXTURE_MIN:
                    break
    return picks


def run_tuist_generate(fixture_dir: Path) -> tuple[bool, str]:
    tuist = shutil.which("tuist")
    if not tuist:
        return False, "tuist not on PATH"
    try:
        # Some fixtures use Package.swift only (no tuist), and `tuist install`
        # is a no-op then; ignore install failures, fail loudly on generate.
        common.run([tuist, "install"], cwd=fixture_dir, check=False, quiet=False)
        cp = common.run([tuist, "generate", "--no-open"], cwd=fixture_dir,
                        check=False, capture=True, quiet=False)
        if cp.returncode != 0:
            return False, (cp.stdout + cp.stderr)[-2000:]
        return True, ""
    except Exception as e:
        return False, str(e)


def tool_version(name: str) -> str:
    found = shutil.which(name)
    if not found:
        return ""
    try:
        cp = subprocess.run([found, "--version"],
                            capture_output=True, text=True, timeout=10)
        if cp.returncode == 0 and cp.stdout.strip():
            return cp.stdout.strip().splitlines()[0]
        cp = subprocess.run([found, "version"],
                            capture_output=True, text=True, timeout=10)
        return cp.stdout.strip().splitlines()[0] if cp.stdout.strip() else ""
    except Exception:
        return ""


def now_iso() -> str:
    return dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def process_project(
    project: common.CorpusProject,
    manifest: dict,
    *,
    force: bool,
    skip_tuist: bool,
) -> dict:
    """Returns the manifest entry for this project."""
    dest = common.CORPUS_DIR / project.slug

    entry: dict = dict(manifest.get("projects", {}).get(project.slug, {}))

    # Determine target ref
    if project.pin == "latest-release":
        tags = ls_remote_tags(project.repo)
        chosen = pick_latest_release(tags)
        if chosen is None:
            raise RuntimeError(
                f"{project.slug}: no semver-looking release tag in {len(tags)} candidates"
            )
        common.log(f"{project.slug}: latest release tag = {chosen}")
        target_ref = chosen
    elif project.pin == "default-branch":
        # Probe default branch via HEAD ref
        cp = common.run(["git", "ls-remote", "--symref", project.repo, "HEAD"],
                        capture=True, quiet=True)
        first = cp.stdout.splitlines()[0]
        m = re.match(r"ref:\s+refs/heads/(\S+)\s+HEAD", first)
        branch = m.group(1) if m else "main"
        common.log(f"{project.slug}: default branch = {branch}")
        target_ref = branch
    else:
        raise ValueError(f"unknown pin strategy: {project.pin}")

    # Skip the clone step if already present at the right SHA (unless --force).
    # Per-project setup (e.g. tuist generate) still runs below so we can fix
    # selection mistakes without re-downloading the whole repo.
    existing_sha = entry.get("sha")
    needs_clone = True
    if not force and dest.exists() and existing_sha:
        actual_sha = head_sha(dest)
        if actual_sha == existing_sha:
            common.log(f"{project.slug}: up-to-date at {actual_sha[:12]} (skip clone)")
            needs_clone = False

    if needs_clone:
        shallow_clone(project.repo, dest, target_ref)
        sha = head_sha(dest)
        common.log(f"{project.slug}: cloned at {sha}")
        entry.update({
            "slug": project.slug,
            "repo": project.repo,
            "pin_strategy": project.pin,
            "ref": target_ref,
            "sha": sha,
            "captured_at": now_iso(),
        })
    else:
        # Refresh non-volatile fields
        entry.setdefault("slug", project.slug)
        entry.setdefault("repo", project.repo)
        entry.setdefault("pin_strategy", project.pin)
        entry.setdefault("ref", target_ref)

    # Per-project setup. Re-runs even when the clone was skipped — that way
    # editing the fixture-pick patterns doesn't require a re-download.
    already_generated = {
        fx["path"]: fx for fx in entry.get("fixtures_selected", [])
        if fx.get("generated")
    } if project.slug == "tuist-fixtures" else {}
    if project.slug == "tuist-fixtures" and not skip_tuist:
        picks = pick_tuist_fixtures(dest)
        common.log(f"tuist-fixtures: selected {len(picks)} fixtures")
        for cat, path in picks:
            common.log(f"  [{cat}] {path.relative_to(dest)}")
        tuist_ver = tool_version("tuist")
        selected: list[dict] = []
        for cat, path in picks:
            rel = str(path.relative_to(dest))
            # Skip regeneration if we previously succeeded for this path
            # (and not --force).
            prev = already_generated.get(rel)
            if prev and not force:
                selected.append(prev)
                common.log(f"  {rel}: reuse previous generate")
                continue
            ok, err = run_tuist_generate(path)
            selected.append({
                "category": cat,
                "path": rel,
                "generated": ok,
                "error": err if not ok else "",
            })
        entry["tuist_version"] = tuist_ver
        entry["fixtures_selected"] = selected

    return entry


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--project", help="only operate on one corpus slug")
    ap.add_argument("--force", action="store_true",
                    help="re-clone even if up-to-date")
    ap.add_argument("--skip-tuist", action="store_true",
                    help="don't run `tuist install/generate` for tuist-fixtures")
    args = ap.parse_args()

    common.ensure_dir(common.CORPUS_DIR)
    manifest = common.load_manifest()
    manifest.setdefault("projects", {})
    manifest["manifest_version"] = 1
    manifest["captured_at"] = now_iso()
    manifest["host_macos"] = common.host_macos_version()

    projects = common.selected_projects(args.project)
    had_error = False
    for project in projects:
        try:
            entry = process_project(
                project, manifest,
                force=args.force, skip_tuist=args.skip_tuist,
            )
            manifest["projects"][project.slug] = entry
            common.save_manifest(manifest)
        except Exception as e:
            had_error = True
            common.log(f"ERROR {project.slug}: {e}")
            manifest["projects"].setdefault(project.slug, {})["error"] = str(e)
            common.save_manifest(manifest)

    return 1 if had_error else 0


if __name__ == "__main__":
    sys.exit(main())
