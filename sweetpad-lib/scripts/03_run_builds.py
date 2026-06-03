#!/usr/bin/env python3
"""Run one representative build per (scheme, config, destination) and collect
the artifacts needed for snapshot validation later.

Inputs (must already exist):
  - corpus/<slug>/      (from 01_clone_corpus.py)
  - fixtures/<slug>/xcode-<ver>/metadata/  (from 02_capture_metadata.py)
  - .cache/xcresulttool_probe.json         (from 00_bootstrap.py)

Outputs per (project × xcode × scheme × config × destination), under
`fixtures/<slug>/xcode-<ver>/build/<scheme>__<config>__<dest>/`:

  xcactivitylog.gz              gzipped raw .xcactivitylog from DerivedData
  xcactivitylog.parsed.json     XCLogParser dump of the above
  xcresult.json                 xcresulttool get --format json [--legacy]
  result.xcresult/              raw bundle (kept for re-extraction)
  index-store.tgz               tar+gzip of DerivedData/.../Index.noindex/DataStore/v5/
  tool-invocations.jsonl        appended by the PATH-wrapped toolchain shim
                                — NOTE: typically empty for the main Apple
                                toolchain, because xcodebuild calls clang /
                                swiftc / ld via absolute Xcode toolchain
                                paths (not PATH). It still catches user
                                build phases and SwiftPM subprocesses.
                                The authoritative command-line trace is in
                                `stdout.txt` (look for `ExecuteExternalTool`
                                lines) and in the raw `xcactivitylog.gz`.
  stdout.txt, stderr.txt, exit_code, command.txt
  meta.json                     dest dict, scheme, config, durations

Destination policy (per scheme):
  - If the scheme supports macOS, prefer macOS.
  - Else prefer iOS Simulator (latest OS, name=iPhone-something).
  - Else watchOS / tvOS / visionOS Simulator (latest OS).

PATH-wrapped toolchain shim:
  - `scripts/toolshim/` is prepended to PATH.
  - `SWEETPAD_SHIM_LOG` is set to the build's `tool-invocations.jsonl`.

Idempotent: a build whose output dir already has `exit_code` is skipped
unless `--force`.

Flags:
  --project <slug>
  --xcode <ver|slot>
  --scheme <name>      restrict to one scheme
  --config <name>      restrict to one configuration
  --force              re-run even if outputs exist
"""

from __future__ import annotations

import argparse
import datetime as dt
import gzip
import json
import os
import shutil
import subprocess
import sys
import tarfile
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402
import importlib

# Reuse subproject discovery + destination helpers from 02
_cap = importlib.import_module("02_capture_metadata")


XCRESULT_PROBE_CACHE = common.REPO_ROOT / ".cache" / "xcresulttool_probe.json"


def load_xcresulttool_probes() -> dict[str, dict]:
    if not XCRESULT_PROBE_CACHE.exists():
        return {}
    with XCRESULT_PROBE_CACHE.open() as f:
        return json.load(f)


def requires_legacy(probes: dict[str, dict], xcode_version: str) -> bool:
    info = probes.get(xcode_version)
    if isinstance(info, dict) and "requires_legacy" in info:
        return bool(info["requires_legacy"])
    # Heuristic fallback: Xcode 16+ requires --legacy.
    try:
        major = int(xcode_version.split(".")[0])
    except ValueError:
        return False
    return major >= 16


def pick_representative_destination(
    dests: list[dict[str, str]],
) -> dict[str, str] | None:
    """Return the single 'best' destination for one build.

    Preference order — pick the most reproducible simulator before macOS,
    since iOS+Catalyst schemes have macOS variants we don't want to pick
    by accident:
      1. iOS Simulator (highest OS, prefer iPhone name)
      2. visionOS / watchOS / tvOS Simulator (highest OS)
      3. macOS (only if no simulator destination exists at all)
    Placeholder destinations and entries with `error` are filtered upstream.
    """
    def os_key(d: dict[str, str]) -> tuple[int, ...]:
        parts = (d.get("OS", "0").split(".") + ["0", "0"])[:3]
        return tuple(int(x) for x in parts if x.isdigit()) or (0,)

    # 1. iOS Simulator
    candidates = [
        d for d in dests
        if d.get("platform") == "iOS Simulator"
        and "id" in d and "OS" in d
        and not _cap.is_placeholder(d)
    ]
    if candidates:
        def ios_key(d: dict[str, str]) -> tuple:
            return (os_key(d), "iPhone" in d.get("name", ""))
        candidates.sort(key=ios_key, reverse=True)
        return candidates[0]

    # 2. Other simulators
    for plat in ("visionOS Simulator", "watchOS Simulator", "tvOS Simulator"):
        candidates = [
            d for d in dests
            if d.get("platform") == plat
            and "id" in d and "OS" in d
            and not _cap.is_placeholder(d)
        ]
        if candidates:
            candidates.sort(key=os_key, reverse=True)
            return candidates[0]

    # 3. macOS (fallback)
    for d in dests:
        if d.get("platform") == _cap.MAC_PLATFORM and not _cap.is_placeholder(d):
            return d
    return None


def find_xcactivitylog(derived: Path, scheme: str, config: str) -> Path | None:
    """Return the most recent .xcactivitylog under derived/Logs/Build/."""
    log_dir = derived / "Logs" / "Build"
    if not log_dir.exists():
        return None
    candidates = sorted(
        log_dir.glob("*.xcactivitylog"),
        key=lambda p: p.stat().st_mtime, reverse=True,
    )
    return candidates[0] if candidates else None


def gzip_copy(src: Path, dst: Path) -> None:
    with src.open("rb") as fin, gzip.open(dst, "wb") as fout:
        shutil.copyfileobj(fin, fout)


def parse_xcactivitylog(src: Path, dst_json: Path) -> tuple[bool, str]:
    tool = shutil.which("xclogparser")
    if not tool:
        return False, "xclogparser not on PATH"
    cp = subprocess.run(
        [tool, "parse", "--file", str(src), "--reporter", "json", "--output", str(dst_json)],
        capture_output=True, text=True, timeout=600,
    )
    if cp.returncode != 0:
        return False, (cp.stderr or cp.stdout)[-1000:]
    return True, ""


def extract_xcresult(
    bundle: Path, dst: Path, *, use_legacy: bool, xcode: common.XcodeInstall,
) -> tuple[bool, str]:
    # Xcode 16+: `xcresulttool get object --legacy --path …`.
    # Older:     `xcresulttool get --path …`.
    if use_legacy:
        args = ["/usr/bin/xcrun", "xcresulttool", "get", "object", "--legacy",
                "--format", "json", "--path", str(bundle)]
    else:
        args = ["/usr/bin/xcrun", "xcresulttool", "get",
                "--format", "json", "--path", str(bundle)]
    env = dict(os.environ)
    env["DEVELOPER_DIR"] = str(xcode.developer_dir)
    cp = subprocess.run(args, env=env, capture_output=True, text=True, timeout=300)
    if cp.returncode != 0:
        return False, (cp.stderr or cp.stdout)[-1000:]
    dst.write_text(cp.stdout)
    return True, ""


def snapshot_index_store(derived: Path, dst_tgz: Path) -> tuple[bool, str]:
    src = derived / "Index.noindex" / "DataStore" / "v5"
    if not src.exists():
        # Try older layout
        src = derived / "Index" / "DataStore" / "v5"
    if not src.exists():
        return False, f"no Index.noindex DataStore under {derived}"
    with tarfile.open(dst_tgz, "w:gz") as tar:
        tar.add(src, arcname="v5")
    return True, ""


@dataclass
class BuildPlan:
    project: common.CorpusProject
    sub: "_cap.Subproject"
    scheme: str
    config: str
    dest: dict[str, str]
    out_dir: Path  # fixtures/.../build/<combo>/


def plans_for_subproject(
    project: common.CorpusProject, xcode: common.XcodeInstall, sub,
    *, scheme_filter: str | None, config_filter: str | None,
    max_schemes: int | None = None,
) -> list[BuildPlan]:
    meta_dir_root = common.metadata_dir(project.slug, xcode.version)
    if sub.label:
        per_sub_meta = meta_dir_root / sub.label
    else:
        per_sub_meta = meta_dir_root
    list_path = per_sub_meta / "list.json"
    if not list_path.exists():
        common.log(f"no metadata/list.json at {list_path} — skip")
        return []

    with list_path.open() as f:
        data = json.load(f)
    node = data.get("workspace") or data.get("project") or {}
    schemes: list[str] = list(node.get("schemes") or [])
    # Workspace `-list -json` does not expose configurations (only project
    # listings do). Apple's default Xcode template ships Debug + Release;
    # fall back to those when the listing doesn't tell us otherwise.
    configs: list[str] = list(node.get("configurations") or ["Debug", "Release"])
    if scheme_filter:
        schemes = [s for s in schemes if s == scheme_filter]
    if config_filter:
        configs = [c for c in configs if c == config_filter]

    out: list[BuildPlan] = []
    schemes_built = 0
    for scheme in schemes:
        if max_schemes is not None and schemes_built >= max_schemes:
            common.log(f"reached --max-schemes-per-subproject={max_schemes}; "
                       f"skipping remaining schemes for {sub.label or 'root'}")
            break
        scheme_meta_dir = per_sub_meta / "schemes" / scheme
        dest_path = scheme_meta_dir / "destinations.json"
        if not dest_path.exists():
            common.log(f"no destinations.json for scheme {scheme} — skip")
            continue
        # Skip schemes that produced zero successful build-settings during
        # metadata capture — they're orphan test targets or otherwise
        # un-buildable in isolation. -showBuildSettings is strictly cheaper
        # than `xcodebuild build`; if the former failed, the latter will too.
        bs_dir = scheme_meta_dir / "build-settings"
        if bs_dir.exists() and not any(bs_dir.glob("*.json")):
            common.log(f"scheme {scheme}: no successful build-settings in metadata — skip")
            continue
        with dest_path.open() as f:
            dests = json.load(f)
        rep = pick_representative_destination(dests)
        if not rep:
            common.log(f"no representative destination for {scheme} — skip")
            continue
        schemes_built += 1
        for config in configs:
            dest_slug = _cap.destination_filename_slug(rep)
            combo = f"{common.slug(scheme)}__{common.slug(config)}__{dest_slug}"
            if sub.label:
                combo = f"{common.slug(sub.label)}__{combo}"
            out_dir = common.build_dir(project.slug, xcode.version) / combo
            out.append(BuildPlan(
                project=project, sub=sub, scheme=scheme, config=config,
                dest=rep, out_dir=out_dir,
            ))
    return out


def already_done(plan: BuildPlan) -> bool:
    return (plan.out_dir / "exit_code").exists()


def run_build(
    plan: BuildPlan, xcode: common.XcodeInstall, probes: dict,
) -> bool:
    out = plan.out_dir
    common.ensure_dir(out)

    derived = common.fixture_dir(plan.project.slug, xcode.version) / ".derived"
    common.ensure_dir(derived)

    invocations_log = out / "tool-invocations.jsonl"
    # Truncate so the file contains only this build's invocations.
    invocations_log.write_text("")

    result_bundle = out / "result.xcresult"
    if result_bundle.exists():
        # xcodebuild fails if the bundle already exists.
        shutil.rmtree(result_bundle)

    dest_value = _cap.destination_string(plan.dest)

    common_args = [
        "-scheme", plan.scheme,
        "-configuration", plan.config,
        "-destination", dest_value,
        "-derivedDataPath", str(derived),
        *_cap.xb_args(plan.sub),
    ]
    clean_args = ["xcodebuild", "clean", *common_args]
    build_args = [
        "xcodebuild", "build",
        *common_args,
        "-resultBundlePath", str(result_bundle),
        "CODE_SIGNING_ALLOWED=NO",
    ]

    env = _cap.xb_env(xcode)
    env["PATH"] = f"{common.TOOLSHIM_DIR}{os.pathsep}{env.get('PATH', '')}"
    env["SWEETPAD_SHIM_LOG"] = str(invocations_log)

    (out / "command.txt").write_text(
        " ".join(clean_args) + "\n" + " ".join(build_args) + "\n"
    )

    # Clean (best-effort; not fatal)
    common.log(f"clean: scheme={plan.scheme} config={plan.config}")
    subprocess.run(clean_args, env=env, capture_output=True, text=True, timeout=600)

    common.log(f"build: scheme={plan.scheme} config={plan.config} dest={dest_value}")
    t0 = time.time()
    cp = subprocess.run(
        build_args, env=env, capture_output=True, text=True, timeout=3600,
    )
    duration = time.time() - t0
    (out / "stdout.txt").write_text(cp.stdout)
    (out / "stderr.txt").write_text(cp.stderr)
    (out / "exit_code").write_text(str(cp.returncode) + "\n")
    common.log(f"  -> exit={cp.returncode} duration={duration:.1f}s")

    success = (cp.returncode == 0)

    # Always try to collect artifacts (even on failure — partial logs are
    # interesting). Each step is best-effort and writes errors/ entries on
    # failure.
    log = find_xcactivitylog(derived, plan.scheme, plan.config)
    if log:
        try:
            gzip_copy(log, out / "xcactivitylog.gz")
        except Exception as e:
            common.log(f"WARN: gzip xcactivitylog failed: {e}")
        ok, msg = parse_xcactivitylog(log, out / "xcactivitylog.parsed.json")
        if not ok:
            common.write_error(
                plan.project.slug, xcode.version,
                f"03_xclogparser__{out.name}", msg,
            )

    if result_bundle.exists():
        ok, msg = extract_xcresult(
            result_bundle, out / "xcresult.json",
            use_legacy=requires_legacy(probes, xcode.version),
            xcode=xcode,
        )
        if not ok:
            common.write_error(
                plan.project.slug, xcode.version,
                f"03_xcresulttool__{out.name}", msg,
            )

    ok, msg = snapshot_index_store(derived, out / "index-store.tgz")
    if not ok:
        # Only an error if the build actually succeeded — otherwise no index
        # store is expected.
        if success:
            common.write_error(
                plan.project.slug, xcode.version,
                f"03_indexstore__{out.name}", msg,
            )

    # meta.json for the build
    with (out / "meta.json").open("w") as f:
        json.dump({
            "project": plan.project.slug,
            "xcode_version": xcode.version,
            "scheme": plan.scheme,
            "configuration": plan.config,
            "destination": plan.dest,
            "destination_value": dest_value,
            "subproject": plan.sub.label,
            "duration_sec": round(duration, 2),
            "exit_code": cp.returncode,
            "captured_at": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        }, f, indent=2, sort_keys=True)
        f.write("\n")

    return success


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--project", help="restrict to one corpus slug")
    ap.add_argument("--xcode", help="restrict to one Xcode (version or slot)")
    ap.add_argument("--scheme", help="restrict to one scheme")
    ap.add_argument("--config", help="restrict to one configuration")
    ap.add_argument("--force", action="store_true",
                    help="re-run even if outputs exist")
    ap.add_argument("--max-schemes-per-subproject", type=int, default=None,
                    help="cap how many schemes to build per (sub)project. "
                         "Useful for phase-1 pilots; unset to build all "
                         "buildable schemes.")
    args = ap.parse_args()

    installs = common.discover_installed_xcodes()
    xcodes = common.selected_xcodes(installs, args.xcode)
    projects = common.selected_projects(args.project)
    probes = load_xcresulttool_probes()

    had_error = False
    for project in projects:
        for x in xcodes:
            common.log(f"\n========= {project.slug} :: xcode {x.version} =========")
            try:
                with common.with_xcode(x):
                    subs = _cap.find_subprojects(project)
                    plans: list[BuildPlan] = []
                    for sub in subs:
                        plans.extend(plans_for_subproject(
                            project, x, sub,
                            scheme_filter=args.scheme,
                            config_filter=args.config,
                            max_schemes=args.max_schemes_per_subproject,
                        ))
                    if not plans:
                        common.log(f"no build plans for {project.slug}/{x.version}")
                        continue

                    for plan in plans:
                        if not args.force and already_done(plan):
                            common.log(f"skip (done): {plan.out_dir}")
                            continue
                        try:
                            ok = run_build(plan, x, probes)
                            if not ok:
                                common.write_error(
                                    plan.project.slug, x.version,
                                    f"03_build__{plan.out_dir.name}",
                                    f"build failed (see {plan.out_dir}/stderr.txt)",
                                )
                        except Exception as e:
                            had_error = True
                            common.log(f"ERROR plan {plan.scheme}/{plan.config}: {e}")
                            common.write_error(
                                plan.project.slug, x.version,
                                f"03_build_exception__{plan.out_dir.name}",
                                repr(e),
                            )
            except Exception as e:
                had_error = True
                common.log(f"ERROR {project.slug}/{x.version}: {e}")
                common.write_error(project.slug, x.version, "03_run", repr(e))

    return 1 if had_error else 0


if __name__ == "__main__":
    sys.exit(main())
