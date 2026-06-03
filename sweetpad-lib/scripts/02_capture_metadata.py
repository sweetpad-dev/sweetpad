#!/usr/bin/env python3
"""Capture xcodebuild metadata + raw inputs per (project, Xcode).

For each (project × Xcode) it writes, under
`fixtures/<slug>/xcode-<ver>/`:

  metadata/list.json                                xcodebuild -list -json
  metadata/showsdks.json                            xcodebuild -showsdks -json
  metadata/schemes/<S>/destinations.json            parsed -showdestinations
  metadata/schemes/<S>/build-settings/<C>__<D>.json -showBuildSettings -json
  metadata/schemes/<S>/dry-run/<C>__<D>.txt         -dry-run (stdout+stderr)

  raw/<preserved-relative-path>                     pbxproj, xcworkspacedata,
                                                    xcscheme, xcconfig,
                                                    Info.plist, entitlements

  meta.json                                         Xcode + host info, status

Destination dedup: `-showdestinations` typically lists 30+ device variants per
scheme with identical (platform, OS) — they yield identical build settings.
We dedupe to one representative device per (platform, OS), keeping `id=<UUID>`
for precise destination targeting downstream.

Project discovery: for non-Tuist projects we scan `corpus/<slug>/` for the
top-level workspace/project. For `tuist-fixtures` we iterate over the
`fixtures_selected` array in `corpus/manifest.json` and treat each fixture
sub-directory as an independent "subproject" (output goes under
`fixtures/tuist-fixtures/xcode-<ver>/<fixture-name>/`).

Idempotent: any output file that already exists is preserved unless
`--force` is given. Failures for one scheme/config/destination don't abort
the run — they are recorded under `errors/`.

Flags:
  --project <slug>          restrict to one project
  --xcode <ver|slot>        restrict to one Xcode
  --force                   redo outputs even if they exist
  --include-sim-ids         keep every simulator id instead of deduping by
                            (platform, OS) — produces many more captures
"""

from __future__ import annotations

import argparse
import datetime as dt
import functools
import json
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


# Platforms we capture. macOS is treated specially (no Simulator suffix).
SIM_PLATFORMS: frozenset[str] = frozenset({
    "iOS Simulator", "watchOS Simulator", "tvOS Simulator", "visionOS Simulator",
})
MAC_PLATFORM = "macOS"

# File patterns to copy under raw/.
RAW_PATTERNS_FILE: tuple[str, ...] = (
    "Info.plist",
)
RAW_PATTERNS_SUFFIX: tuple[str, ...] = (
    ".pbxproj",
    ".xcworkspacedata",
    ".xcscheme",
    ".xcconfig",
    ".entitlements",
)
# Directories to skip during raw/ scan.
SKIP_DIRS: frozenset[str] = frozenset({
    ".git", ".svn", ".hg",
    "DerivedData", "build", ".build",
    "node_modules", "Pods", "Carthage",
    ".swiftpm", "tmp", ".tuist-cache",
})


@dataclass(frozen=True)
class Subproject:
    """A workspace or project to operate on."""
    label: str                    # used in output paths (e.g. "" or fixture name)
    project_kind: str             # "workspace" | "project"
    project_path: Path            # absolute path to .xcworkspace / .xcodeproj
    root: Path                    # the source tree root for raw/ collection


def find_subprojects(project: common.CorpusProject) -> list[Subproject]:
    """Locate workspace(s) / project(s) inside `corpus/<slug>/`."""
    root = common.CORPUS_DIR / project.slug
    if not root.exists():
        raise FileNotFoundError(
            f"{root} does not exist — run 01_clone_corpus.py first"
        )

    if project.slug == "tuist-fixtures":
        manifest = common.load_manifest()
        entry = manifest.get("projects", {}).get(project.slug, {})
        out: list[Subproject] = []
        for fx in entry.get("fixtures_selected", []):
            if not fx.get("generated"):
                continue
            fx_root = root / fx["path"]
            sub = _pick_workspace_or_project(fx_root, label=fx["path"].replace("/", "_"))
            if sub:
                out.append(sub)
        if not out:
            raise RuntimeError(
                "tuist-fixtures has no generated subprojects — "
                "check corpus/manifest.json fixtures_selected.*.generated"
            )
        return out

    sub = _pick_workspace_or_project(root, label="")
    if not sub:
        raise RuntimeError(f"no .xcworkspace or .xcodeproj found in {root}")
    return [sub]


def _pick_workspace_or_project(root: Path, *, label: str) -> Subproject | None:
    """Prefer a workspace over a project; pick the first match by name order."""
    if not root.exists():
        return None
    workspaces = sorted(p for p in root.iterdir()
                        if p.is_dir() and p.suffix == ".xcworkspace"
                        and p.name != "project.xcworkspace")
    if workspaces:
        return Subproject(label=label, project_kind="workspace",
                          project_path=workspaces[0], root=root)
    projects = sorted(p for p in root.iterdir()
                      if p.is_dir() and p.suffix == ".xcodeproj")
    if projects:
        return Subproject(label=label, project_kind="project",
                          project_path=projects[0], root=root)
    return None


# --- xcodebuild wrappers ---------------------------------------------------

def xb_env(xcode: common.XcodeInstall) -> dict[str, str]:
    e = dict(os.environ)
    e["DEVELOPER_DIR"] = str(xcode.developer_dir)
    return e


def xb_args(sub: Subproject) -> list[str]:
    flag = "-workspace" if sub.project_kind == "workspace" else "-project"
    return [flag, str(sub.project_path)]


def run_xb(
    args: list[str],
    *,
    xcode: common.XcodeInstall,
    timeout: float = 180,
) -> subprocess.CompletedProcess[str]:
    cmd = ["xcodebuild", *args]
    return subprocess.run(
        cmd, env=xb_env(xcode), capture_output=True, text=True,
        timeout=timeout,
    )


def capture_list(sub: Subproject, *, xcode: common.XcodeInstall) -> dict | None:
    cp = run_xb(["-list", "-json", *xb_args(sub)], xcode=xcode)
    if cp.returncode != 0:
        common.log(f"-list failed for {sub.project_path}: {cp.stderr[-500:]}")
        return None
    try:
        return json.loads(cp.stdout)
    except json.JSONDecodeError as e:
        common.log(f"-list output not JSON: {e}")
        return None


def discover_configurations(sub: Subproject, *, xcode: common.XcodeInstall) -> list[str]:
    """Discover all build configurations for a (sub)project.

    Workspace `-list -json` does NOT expose `configurations` (only project
    `-list -json` does). When the listing we have is a workspace, this
    function locates each underlying `.xcodeproj` and unions their
    configurations. Falls back to ["Debug", "Release"] if discovery yields
    nothing.
    """
    targets: list[Path]
    if sub.project_kind == "project":
        targets = [sub.project_path]
    else:
        # Find sibling/descendant .xcodeproj directories under the subproject
        # root, excluding the workspace's own embedded
        # `project.xcworkspace/` (which isn't a real project).
        targets = sorted({
            p for p in sub.root.rglob("*.xcodeproj")
            if "project.xcworkspace" not in p.parts
            and ".derived" not in p.parts
            and ".build" not in p.parts
        })
    configs: set[str] = set()
    for proj in targets:
        cp = run_xb(["-list", "-json", "-project", str(proj)],
                    xcode=xcode, timeout=60)
        if cp.returncode != 0:
            continue
        try:
            data = json.loads(cp.stdout)
        except json.JSONDecodeError:
            continue
        proj_node = data.get("project") or {}
        for c in (proj_node.get("configurations") or []):
            configs.add(c)
    return sorted(configs) if configs else ["Debug", "Release"]


def capture_showsdks(*, xcode: common.XcodeInstall) -> dict | list | None:
    cp = run_xb(["-showsdks", "-json"], xcode=xcode)
    if cp.returncode != 0:
        return None
    try:
        return json.loads(cp.stdout)
    except json.JSONDecodeError:
        return None


_DEST_LINE_RE = re.compile(r"\{\s*([^{}]*)\s*\}")


def parse_destinations(text: str) -> list[dict[str, str]]:
    """Parse `xcodebuild -showdestinations` text output into dicts.

    Each `{ platform:..., id:..., OS:..., name:... }` line becomes one dict.
    """
    dests: list[dict[str, str]] = []
    for m in _DEST_LINE_RE.finditer(text):
        inner = m.group(1)
        d: dict[str, str] = {}
        # k:v, k:v — careful: values may contain spaces but not commas.
        for kv in inner.split(","):
            kv = kv.strip()
            if not kv or ":" not in kv:
                continue
            k, v = kv.split(":", 1)
            d[k.strip()] = v.strip()
        if d.get("platform"):
            dests.append(d)
    return dests


def capture_destinations(
    sub: Subproject, scheme: str, *, xcode: common.XcodeInstall,
) -> list[dict[str, str]] | None:
    cp = run_xb([
        "-showdestinations", "-scheme", scheme, *xb_args(sub),
    ], xcode=xcode, timeout=300)
    if cp.returncode != 0:
        common.log(f"-showdestinations {scheme}: failed")
        return None
    return parse_destinations(cp.stdout + "\n" + cp.stderr)


def is_placeholder(d: dict[str, str]) -> bool:
    """Detect `xcodebuild`'s placeholder destinations (e.g. 'Any iOS Device').

    These are listed by -showdestinations but can't be used for
    -showBuildSettings or builds — they exist as scheme defaults / archive
    targets. They have id values containing 'Placeholder' / 'placeholder',
    or no OS for a simulator platform, or names starting with 'Any '.
    """
    pid = d.get("id", "")
    if "Placeholder" in pid or "placeholder" in pid:
        return True
    name = d.get("name", "")
    if name.startswith("Any "):
        return True
    return False


# `-showdestinations` reliably reports the *device* platforms a scheme supports
# (iOS, tvOS, watchOS, visionOS appear even with no runtime), but under an
# orchestrated run it intermittently omits the concrete *Simulator* devices. So
# we derive a representative simulator per supported platform from `simctl`
# (which lists devices reliably) and inject it. Maps a device-platform label to
# the xcodebuild `-destination platform=` label.
_DEVICE_TO_SIM = {
    "iOS": "iOS Simulator",
    "tvOS": "tvOS Simulator",
    "watchOS": "watchOS Simulator",
    "visionOS": "visionOS Simulator",
}


@functools.lru_cache(maxsize=1)
def available_simulators() -> dict[str, tuple[str, str, str]]:
    """One available simulator per `-destination` platform label, from `simctl`.

    Returns `{platform_label: (udid, os_version, device_name)}`. The visionOS
    runtime is `xrOS` in `simctl` but `visionOS Simulator` to xcodebuild.
    """
    runtime_to_label = {
        "iOS": "iOS Simulator", "tvOS": "tvOS Simulator",
        "watchOS": "watchOS Simulator", "xrOS": "visionOS Simulator",
    }
    out: dict[str, tuple[str, str, str]] = {}
    cp = subprocess.run(
        ["xcrun", "simctl", "list", "devices", "available", "-j"],
        capture_output=True, text=True,
    )
    if cp.returncode != 0:
        return out
    try:
        data = json.loads(cp.stdout)
    except json.JSONDecodeError:
        return out
    for runtime, devices in data.get("devices", {}).items():
        tail = runtime.split("SimRuntime.")[-1]   # e.g. iOS-26-5
        label = runtime_to_label.get(tail.split("-")[0])
        if not label or label in out:
            continue
        os_ver = ".".join(tail.split("-")[1:])    # 26-5 -> 26.5
        for dev in devices:
            if dev.get("isAvailable"):
                out[label] = (dev["udid"], os_ver, dev["name"])
                break
    return out


def augment_with_simulators(dests: list[dict[str, str]]) -> list[dict[str, str]]:
    """Add a representative simulator destination for each device platform the
    scheme supports but for which `-showdestinations` surfaced no concrete one."""
    supported = {d.get("platform", "") for d in dests}
    have_sim = {p for p in supported if p.endswith("Simulator")}
    sims = available_simulators()
    for dev_plat, sim_label in _DEVICE_TO_SIM.items():
        if dev_plat in supported and sim_label not in have_sim and sim_label in sims:
            udid, os_ver, name = sims[sim_label]
            dests.append({
                "platform": sim_label, "id": udid, "OS": os_ver,
                "name": name, "arch": "arm64",
            })
    return dests


def representative_destinations(
    dests: list[dict[str, str]], *, include_all_sims: bool,
) -> list[dict[str, str]]:
    """Filter to (platform, OS) representatives.

    - Only platforms in SIM_PLATFORMS or MAC_PLATFORM.
    - Drop placeholder entries (see `is_placeholder`).
    - For MAC_PLATFORM keep one entry (prefer non-variant).
    - For simulator platforms: one per (platform, OS) unless include_all_sims.
      Simulator platforms must have an `OS` key — placeholder/generic ones
      that lack it are skipped.
    """
    kept: list[dict[str, str]] = []
    seen_keys: set[tuple[str, str, str]] = set()
    saw_macos = False
    for d in dests:
        if "error" in d or is_placeholder(d):
            continue
        plat = d.get("platform", "")
        if plat == MAC_PLATFORM:
            if saw_macos:
                continue
            # Prefer a destination without a Catalyst/iPad variant.
            if "variant" in d and saw_macos is False:
                # Hold off — look for a plain macOS entry first
                pass
            kept.append(d)
            saw_macos = True
            continue
        if plat not in SIM_PLATFORMS:
            continue
        if "id" not in d or "OS" not in d:
            continue
        os_ver = d.get("OS", "")
        name = d.get("name", "")
        key = (plat, os_ver, name if include_all_sims else "")
        if key in seen_keys:
            continue
        seen_keys.add(key)
        kept.append(d)
    return kept


def destination_string(d: dict[str, str]) -> str:
    """Build a `-destination` value for xcodebuild from a parsed dest dict."""
    plat = d.get("platform", "")
    if plat == MAC_PLATFORM:
        # `platform=macOS` (no arch lock for now; xcodebuild picks)
        return "platform=macOS"
    parts = [f"platform={plat}"]
    if "id" in d:
        parts.append(f"id={d['id']}")
    elif "name" in d:
        parts.append(f"name={d['name']}")
        if "OS" in d:
            parts.append(f"OS={d['OS']}")
    return ",".join(parts)


def destination_filename_slug(d: dict[str, str]) -> str:
    plat = d.get("platform", "unknown")
    os_ver = d.get("OS", "")
    name = d.get("name", "")
    if plat == MAC_PLATFORM:
        return common.slug("macOS")
    parts = [plat]
    if os_ver:
        parts.append(f"OS{os_ver}")
    if name:
        parts.append(name)
    return common.slug("_".join(parts))


def capture_build_settings(
    sub: Subproject, scheme: str, config: str, dest_value: str, *,
    xcode: common.XcodeInstall,
) -> tuple[bool, str, str]:
    """Returns (ok, stdout, stderr)."""
    cp = run_xb([
        "-showBuildSettings", "-json",
        "-scheme", scheme,
        "-configuration", config,
        "-destination", dest_value,
        *xb_args(sub),
    ], xcode=xcode, timeout=180)
    return (cp.returncode == 0 and bool(cp.stdout.strip()), cp.stdout, cp.stderr)


def capture_dry_run(
    sub: Subproject, scheme: str, config: str, dest_value: str, *,
    xcode: common.XcodeInstall,
) -> tuple[bool, str, str]:
    cp = run_xb([
        "-dry-run",
        "-scheme", scheme,
        "-configuration", config,
        "-destination", dest_value,
        *xb_args(sub),
    ], xcode=xcode, timeout=300)
    combined = cp.stdout + cp.stderr
    return (cp.returncode == 0, combined, cp.stderr)


# --- Raw input collection --------------------------------------------------

def collect_raw_inputs(src_root: Path, out: Path) -> int:
    """Copy raw input files preserving paths under `src_root`.

    Returns the number of files copied.
    """
    count = 0
    for dirpath, dirnames, filenames in os.walk(src_root, followlinks=False):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for fn in filenames:
            if fn in RAW_PATTERNS_FILE or any(fn.endswith(s) for s in RAW_PATTERNS_SUFFIX):
                src = Path(dirpath) / fn
                rel = src.relative_to(src_root)
                dst = out / rel
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dst)
                count += 1
    return count


# --- Main per-(project, xcode) flow ---------------------------------------

def process_one(
    project: common.CorpusProject,
    xcode: common.XcodeInstall,
    *,
    force: bool,
    include_all_sims: bool,
) -> None:
    fixture_root = common.fixture_dir(project.slug, xcode.version)
    common.ensure_dir(fixture_root)

    try:
        subs = find_subprojects(project)
    except Exception as e:
        common.write_error(project.slug, xcode.version, "02_discover", str(e))
        return

    # meta.json (per-fixture, written even when partial)
    meta = {
        "slug": project.slug,
        "xcode_version": xcode.version,
        "xcode_slot": xcode.slot,
        "host_macos": common.host_macos_version(),
        "captured_at": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "subprojects": [],
    }

    # showsdks once per (project, xcode)
    meta_dir_root = common.metadata_dir(project.slug, xcode.version)
    common.ensure_dir(meta_dir_root)
    showsdks_path = meta_dir_root / "showsdks.json"
    if force or not showsdks_path.exists():
        sdks = capture_showsdks(xcode=xcode)
        if sdks is not None:
            with showsdks_path.open("w") as f:
                json.dump(sdks, f, indent=2, sort_keys=True)
                f.write("\n")

    for sub in subs:
        common.log(f"==> {project.slug} | {xcode.version} | "
                   f"{sub.label or sub.project_path.name}")
        sub_label_dir = sub.label or ""
        # For non-tuist projects, sub_label_dir == "" and we write under metadata/.
        # For tuist-fixtures, we nest under metadata/<fixture>/.
        per_sub_meta = meta_dir_root / sub_label_dir if sub_label_dir else meta_dir_root
        per_sub_raw = (
            common.raw_dir(project.slug, xcode.version) / sub_label_dir
            if sub_label_dir else common.raw_dir(project.slug, xcode.version)
        )
        common.ensure_dir(per_sub_meta)
        common.ensure_dir(per_sub_raw)

        sub_meta: dict = {
            "label": sub.label,
            "project_kind": sub.project_kind,
            "project_path": str(sub.project_path.relative_to(common.CORPUS_DIR)),
            "schemes": [],
        }
        meta["subprojects"].append(sub_meta)

        # raw/
        if force or not any(per_sub_raw.iterdir() if per_sub_raw.exists() else []):
            n = collect_raw_inputs(sub.root, per_sub_raw)
            sub_meta["raw_files"] = n
            common.log(f"  raw: copied {n} files")

        # -list
        list_path = per_sub_meta / "list.json"
        if force or not list_path.exists():
            data = capture_list(sub, xcode=xcode)
            if data is None:
                common.write_error(project.slug, xcode.version,
                                   f"02_list__{sub.label or 'root'}",
                                   f"-list failed for {sub.project_path}")
                continue
            with list_path.open("w") as f:
                json.dump(data, f, indent=2, sort_keys=True)
                f.write("\n")
        else:
            with list_path.open() as f:
                data = json.load(f)

        node = data.get("workspace") or data.get("project") or {}
        schemes: list[str] = list(node.get("schemes") or [])
        # Workspace listings don't include configurations. Discover them by
        # querying each underlying .xcodeproj. discover_configurations()
        # falls back to Debug+Release when discovery fails.
        listed_configs = node.get("configurations")
        if listed_configs:
            configs: list[str] = list(listed_configs)
        else:
            configs = discover_configurations(sub, xcode=xcode)
        sub_meta["scheme_count"] = len(schemes)
        sub_meta["config_count"] = len(configs)
        sub_meta["configs"] = configs

        for scheme in schemes:
            scheme_dir = per_sub_meta / "schemes" / scheme
            dest_path = scheme_dir / "destinations.json"
            common.ensure_dir(scheme_dir)

            # destinations
            if force or not dest_path.exists():
                dests = capture_destinations(sub, scheme, xcode=xcode)
                if dests is None:
                    common.write_error(
                        project.slug, xcode.version,
                        f"02_destinations__{common.slug(scheme)}",
                        f"-showdestinations failed for scheme {scheme!r}",
                    )
                    continue
                dests = augment_with_simulators(dests)
                with dest_path.open("w") as f:
                    json.dump(dests, f, indent=2, sort_keys=True)
                    f.write("\n")
            else:
                with dest_path.open() as f:
                    dests = json.load(f)

            reps = representative_destinations(
                dests, include_all_sims=include_all_sims,
            )
            sub_meta["schemes"].append({
                "scheme": scheme,
                "destinations_raw": len(dests),
                "destinations_used": len(reps),
            })

            for config in configs:
                bs_dir = scheme_dir / "build-settings"
                dr_dir = scheme_dir / "dry-run"
                common.ensure_dir(bs_dir)
                common.ensure_dir(dr_dir)

                for d in reps:
                    dest_value = destination_string(d)
                    dest_slug = destination_filename_slug(d)
                    fname_stem = f"{common.slug(config)}__{dest_slug}"

                    bs_path = bs_dir / f"{fname_stem}.json"
                    if force or not bs_path.exists():
                        ok, out, err = capture_build_settings(
                            sub, scheme, config, dest_value, xcode=xcode,
                        )
                        if ok:
                            try:
                                parsed = json.loads(out)
                                with bs_path.open("w") as f:
                                    json.dump(parsed, f, indent=2, sort_keys=True)
                                    f.write("\n")
                            except json.JSONDecodeError:
                                # Save raw if not parseable
                                bs_path.with_suffix(".txt").write_text(out)
                        else:
                            common.write_error(
                                project.slug, xcode.version,
                                f"02_buildsettings__{common.slug(scheme)}__{fname_stem}",
                                err[-1000:] or out[-1000:] or "no output",
                            )

                    dr_path = dr_dir / f"{fname_stem}.txt"
                    if force or not dr_path.exists():
                        ok, out, err = capture_dry_run(
                            sub, scheme, config, dest_value, xcode=xcode,
                        )
                        # Always save -dry-run output (even if non-zero) — it's
                        # useful for debugging why a config/destination is
                        # rejected.
                        dr_path.write_text(out)

    # Persist meta.json
    meta_path = fixture_root / "meta.json"
    with meta_path.open("w") as f:
        json.dump(meta, f, indent=2, sort_keys=True)
        f.write("\n")
    common.log(f"wrote {meta_path}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--project", help="restrict to one corpus slug")
    ap.add_argument("--xcode", help="restrict to one Xcode (version or slot)")
    ap.add_argument("--force", action="store_true")
    ap.add_argument("--include-sim-ids", action="store_true",
                    help="don't dedupe simulators to one per (platform, OS)")
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
                    process_one(
                        project, x,
                        force=args.force,
                        include_all_sims=args.include_sim_ids,
                    )
            except Exception as e:
                had_error = True
                common.log(f"ERROR {project.slug}/{x.version}: {e}")
                common.write_error(project.slug, x.version, "02_run", repr(e))
    return 1 if had_error else 0


if __name__ == "__main__":
    sys.exit(main())
