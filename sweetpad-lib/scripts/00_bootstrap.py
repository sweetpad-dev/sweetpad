#!/usr/bin/env python3
"""Phase-1 bootstrap.

Verifies / installs the host-side prerequisites declared in DOCS.md:

  - Homebrew is available.
  - CLI tools: `xcodes`, `tuist`, `xclogparser`.
  - At least 3 Xcode majors installed under /Applications/Xcode-<version>.app
    (current + prev-major + prev-major-2). The script does NOT initiate Xcode
    downloads automatically because they require Apple-ID login; instead it
    prints a clear `xcodes install <ver>` invocation for any missing slot.
  - Verifies each Xcode is runnable (`xcodebuild -version`).
  - Probes `xcrun xcresulttool get --legacy` support on each Xcode (Xcode 16+
    requires `--legacy`; earlier rejects it). The probe table is written to
    `.cache/xcresulttool_probe.json` for the build script to consume.

Flags:
  --force             redo probes / re-write cache files even if present
  --skip-install      do not attempt `brew install`; report only

Exit codes:
  0   all prerequisites satisfied
  1   at least one prerequisite missing (see stderr for what)
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


# Brew formulae always required.
REQUIRED_BREW_FORMULAE: list[str] = [
    "tuist",
    "xclogparser",
]
# Only required when installing additional Xcode majors.
OPTIONAL_BREW_FORMULAE_FOR_MULTI_XCODE: list[str] = [
    "xcodes",
]

# Min number of installed Xcode majors required for phase-1 capture.
MIN_XCODES = 3

CACHE_DIR = common.REPO_ROOT / ".cache"
PROBE_CACHE = CACHE_DIR / "xcresulttool_probe.json"


def check_disk_space(min_free_gb: int = 50) -> tuple[bool, float]:
    """Returns (ok, free_gb)."""
    st = os.statvfs(common.REPO_ROOT)
    free_gb = st.f_bavail * st.f_frsize / (1024**3)
    return free_gb >= min_free_gb, free_gb


def detect_brew() -> Path | None:
    return common.brew_path()


def brew_has_formula(brew: Path, formula: str) -> bool:
    cp = subprocess.run(
        [str(brew), "list", "--versions", formula],
        capture_output=True, text=True,
    )
    return cp.returncode == 0 and cp.stdout.strip() != ""


def brew_install(brew: Path, formula: str) -> bool:
    common.log(f"brew install {formula}")
    cp = subprocess.run([str(brew), "install", formula])
    return cp.returncode == 0


def tool_status(name: str) -> str | None:
    return shutil.which(name)


def probe_xcresulttool(xcode: common.XcodeInstall) -> dict:
    """Detect whether `xcrun xcresulttool get object` needs `--legacy`.

    Xcode 16+ ships the new top-level `xcresulttool get <subcommand>` API and
    keeps the legacy data-export path behind `get object --legacy`. The
    `object` subcommand is listed in the top-level help.

    Returns {"requires_legacy": bool, "has_object_subcommand": bool,
             "raw": <help-text>}.
    """
    env = dict(os.environ)
    env["DEVELOPER_DIR"] = str(xcode.developer_dir)
    try:
        # Inspect `xcresulttool get object --help` directly — it lists `--legacy`
        # in OPTIONS for Xcode 16+ and either errors out or omits it on older
        # Xcodes.
        cp = subprocess.run(
            ["/usr/bin/xcrun", "xcresulttool", "get", "object", "--help"],
            env=env, capture_output=True, text=True, timeout=30,
        )
        raw = (cp.stdout + "\n" + cp.stderr).strip()
        # The Xcode 16+ help text contains "USAGE: xcresulttool get object" and
        # "--legacy" in its OPTIONS section. Older Xcodes don't have the
        # `object` subcommand at all (cp.returncode != 0).
        has_object = cp.returncode == 0 and "xcresulttool get object" in raw
        requires_legacy = has_object and "--legacy" in raw
        return {
            "requires_legacy": requires_legacy,
            "has_object_subcommand": has_object,
            "raw": raw[:4000],
        }
    except Exception as e:
        return {"error": str(e)}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--force", action="store_true",
                    help="redo probes / re-write cache files even if present")
    ap.add_argument("--skip-install", action="store_true",
                    help="report only; do not attempt brew install")
    ap.add_argument("--xcodes-required", type=int, default=MIN_XCODES,
                    help=f"minimum installed Xcode majors to consider ready "
                         f"(default: {MIN_XCODES}; use 1 for single-Xcode pilot)")
    args = ap.parse_args()
    min_xcodes = args.xcodes_required

    issues: list[str] = []
    instructions: list[str] = []

    common.log(f"repo root: {common.REPO_ROOT}")

    # --- Disk -------------------------------------------------------------
    # Budget scales with the number of Xcodes we're going to install. ~50 GB
    # per Xcode + some headroom for build artifacts.
    min_gb = max(10, (min_xcodes - 1) * 50 + 10)
    ok, free_gb = check_disk_space(min_free_gb=min_gb)
    common.log(f"free disk: {free_gb:.1f} GB (need ~{min_gb} GB for {min_xcodes}-Xcode plan)")
    if not ok:
        issues.append(
            f"only {free_gb:.1f} GB free on / — plan needs ~{min_gb} GB "
            f"for {min_xcodes}-Xcode coverage plus build artifacts"
        )

    # --- macOS / current Xcode --------------------------------------------
    common.log(f"macOS: {common.host_macos_version()}")
    try:
        common.log("xcodebuild:\n" + common.xcodebuild_version())
    except subprocess.CalledProcessError as e:
        issues.append(f"`xcodebuild -version` failed: {e}")

    # --- Homebrew + CLI tools --------------------------------------------
    brew = detect_brew()
    if brew is None:
        issues.append(
            "Homebrew not found at /opt/homebrew/bin/brew or "
            "/usr/local/bin/brew. Install from https://brew.sh."
        )
    else:
        common.log(f"brew: {brew}")
        formulae = list(REQUIRED_BREW_FORMULAE)
        if min_xcodes > 1:
            formulae.extend(OPTIONAL_BREW_FORMULAE_FOR_MULTI_XCODE)
        for formula in formulae:
            on_path = tool_status(formula)
            if on_path:
                common.log(f"  {formula}: {on_path}")
                continue
            installed = brew_has_formula(brew, formula)
            if installed:
                common.log(f"  {formula}: installed via brew (not on PATH?)")
                continue
            if args.skip_install:
                issues.append(f"{formula!r} not installed (run: brew install {formula})")
                continue
            ok = brew_install(brew, formula)
            if not ok:
                instructions.append(
                    f"brew install {formula} failed — try manually:\n"
                    f"  {brew} install {formula}\n"
                    "If brew refuses due to ownership, run as the brew owner."
                )
                issues.append(f"could not install {formula}")

    # --- Installed Xcodes -------------------------------------------------
    installs = common.discover_installed_xcodes()
    common.log(f"found {len(installs)} Xcode major slots:")
    for x in installs:
        common.log(f"  [{x.slot}] {x.app_path} ({x.version})")
    if len(installs) < min_xcodes:
        installed_majors = sorted(
            {int(x.version.split('.')[0]) for x in installs}, reverse=True
        )
        # Apple's Xcode majors are not contiguous integers (16 -> 26 jump for
        # macOS Tahoe alignment). The right "previous major" depends on what
        # Apple actually shipped — fall back to a known table.
        KNOWN_MAJORS = [26, 16, 15, 14, 13, 12]
        current_major = installed_majors[0] if installed_majors else 0
        prior_majors = [m for m in KNOWN_MAJORS
                        if m < current_major and m not in installed_majors]
        wanted = prior_majors[:min_xcodes - len(installs)]
        issues.append(
            f"only {len(installs)} Xcode major(s) installed — need {min_xcodes}."
        )
        instructions.append(
            "Install the missing Xcode majors with `xcodes`. Run:\n"
            "  xcodes list                # see what Apple offers for this host\n"
            + "\n".join(
                f"  xcodes install <{m}.x>     # latest minor of the {m}.x series"
                for m in wanted
            )
            + "\n\nNote: `xcodes install` prompts for Apple ID credentials and "
              "downloads ~15-20 GB per Xcode (~50 GB installed). Each one ends "
              "up in /Applications/Xcode-<version>.app.\n"
              "Older Xcode majors may not be installable on this macOS; if "
              "xcodes refuses, accept partial coverage (the plan calls older "
              "Xcodes 'best-effort')."
        )

    # --- Per-Xcode probes -------------------------------------------------
    probes: dict[str, dict] = {}
    for x in installs:
        with common.with_xcode(x):
            try:
                xb = common.run_capture(["xcodebuild", "-version"], quiet=True)
            except subprocess.CalledProcessError as e:
                issues.append(f"xcodebuild failed for {x.version}: {e}")
                continue
            common.log(f"  {x.version}: {xb.splitlines()[0]}")
            probes[x.version] = probe_xcresulttool(x)

    if probes:
        CACHE_DIR.mkdir(parents=True, exist_ok=True)
        if args.force or not PROBE_CACHE.exists():
            with PROBE_CACHE.open("w") as f:
                json.dump(probes, f, indent=2, sort_keys=True)
                f.write("\n")
            common.log(f"wrote {PROBE_CACHE}")

    # --- Report -----------------------------------------------------------
    print("")
    if issues:
        print("=== bootstrap: NOT READY ===", file=sys.stderr)
        for i, msg in enumerate(issues, 1):
            print(f"  [{i}] {msg}", file=sys.stderr)
        for note in instructions:
            print("", file=sys.stderr)
            print(note, file=sys.stderr)
        return 1

    print("=== bootstrap: READY ===")
    print(f"  Xcodes (slot -> version): " +
          ", ".join(f"{x.slot}={x.version}" for x in installs))
    return 0


if __name__ == "__main__":
    sys.exit(main())
