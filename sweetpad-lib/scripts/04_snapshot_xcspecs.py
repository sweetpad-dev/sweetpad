#!/usr/bin/env python3
"""Snapshot Apple-side spec data for each installed Xcode.

Per Xcode version, copies into `xcspec-cache/xcode-<ver>/`:

  - Every `*.xcspec` file found anywhere under the Xcode app bundle's
    `Contents/` directory (there are hundreds; mostly small). Relative paths
    under `Contents/` are preserved so later analysis can locate each spec
    back to its origin.
  - Every `SDKSettings.plist` under
    `Contents/Developer/Platforms/*/Developer/SDKs/*.sdk/`, mirrored under
    `sdksettings/`.
  - `sdksettings/sdk-paths.json`: a mapping from canonical SDK name (as
    reported by `xcodebuild -showsdks -json`) to the absolute SDK path
    reported by `xcrun --show-sdk-path --sdk <name>`.
  - `meta.json`: Xcode version, ProductBuildVersion (from the app bundle's
    `Contents/version.plist`; feeds `XCODE_PRODUCT_BUILD_VERSION` and the
    `<short>-<build>` segment of `CCHROOT` / `CACHE_ROOT` in the resolver),
    app path, host macOS, capture timestamp, file counts.

Idempotent: skips any (xcode_version) whose `meta.json` already exists
unless `--force` is given.

Flags:
  --xcode <ver|slot>   only operate on one Xcode (matches by version string
                       like "26.0.1" or by slot like "current")
  --force              re-snapshot even if meta.json exists
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


def find_files(root: Path, pattern_suffix: str) -> list[Path]:
    """Return all files under `root` whose name ends with `pattern_suffix`."""
    matches: list[Path] = []
    if not root.exists():
        return matches
    for dirpath, _, filenames in os.walk(root, followlinks=False):
        for fn in filenames:
            if fn.endswith(pattern_suffix):
                matches.append(Path(dirpath) / fn)
    return matches


def copy_preserving_rel(src: Path, base: Path, out: Path) -> Path:
    """Copy `src` into `out`, preserving its path relative to `base`. Returns
    the destination path."""
    rel = src.relative_to(base)
    dst = out / rel
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)
    return dst


def list_sdks(xcode: common.XcodeInstall) -> list[dict]:
    env = dict(os.environ)
    env["DEVELOPER_DIR"] = str(xcode.developer_dir)
    try:
        cp = subprocess.run(
            ["xcodebuild", "-showsdks", "-json"],
            env=env, capture_output=True, text=True, timeout=120, check=True,
        )
        return json.loads(cp.stdout)
    except (subprocess.CalledProcessError, json.JSONDecodeError) as e:
        common.log(f"WARN: -showsdks failed for {xcode.version}: {e}")
        return []


def show_sdk_path(xcode: common.XcodeInstall, sdk_name: str) -> str:
    env = dict(os.environ)
    env["DEVELOPER_DIR"] = str(xcode.developer_dir)
    try:
        cp = subprocess.run(
            ["/usr/bin/xcrun", "--show-sdk-path", "--sdk", sdk_name],
            env=env, capture_output=True, text=True, timeout=30, check=True,
        )
        return cp.stdout.strip()
    except subprocess.CalledProcessError as e:
        common.log(f"WARN: --show-sdk-path --sdk {sdk_name} failed: {e}")
        return ""


def product_build_version(xcode: common.XcodeInstall) -> str:
    """`ProductBuildVersion` (e.g. "17F42") from the app bundle's
    `Contents/version.plist` — the build number xcodebuild embeds in
    `XCODE_PRODUCT_BUILD_VERSION` and the CCHROOT/CACHE_ROOT version
    segment. Empty string if unreadable (the resolver then falls back to
    the host install)."""
    import plistlib

    plist_path = xcode.app_path / "Contents" / "version.plist"
    try:
        with plist_path.open("rb") as f:
            return str(plistlib.load(f).get("ProductBuildVersion", ""))
    except (OSError, plistlib.InvalidFileException):
        common.log(f"WARN: could not read {plist_path}")
        return ""


def snapshot_one(xcode: common.XcodeInstall, *, force: bool) -> None:
    out_dir = common.XCSPEC_CACHE_DIR / f"xcode-{xcode.version}"
    meta_path = out_dir / "meta.json"
    if meta_path.exists() and not force:
        common.log(f"xcspec snapshot for {xcode.version}: already present, skipping")
        return

    contents_dir = xcode.app_path / "Contents"
    common.log(f"scanning {contents_dir} for .xcspec files (this may take 30-60s)…")
    xcspecs = find_files(contents_dir, ".xcspec")
    common.log(f"  found {len(xcspecs)} .xcspec files")
    if not xcspecs:
        raise RuntimeError(f"no .xcspec files under {contents_dir}; "
                           f"is the Xcode install corrupt?")

    if out_dir.exists() and force:
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    for src in xcspecs:
        copy_preserving_rel(src, contents_dir, out_dir)

    # SDKSettings.plist
    sdks_root = xcode.developer_dir / "Platforms"
    sdksettings = find_files(sdks_root, "SDKSettings.plist")
    common.log(f"  found {len(sdksettings)} SDKSettings.plist files")
    sdksettings_out = out_dir / "sdksettings"
    sdksettings_out.mkdir(parents=True, exist_ok=True)
    for src in sdksettings:
        # Mirror under sdksettings/ to keep the layout sane
        rel = src.relative_to(xcode.developer_dir)
        dst = sdksettings_out / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)

    # Resolve SDK paths via xcrun
    sdks = list_sdks(xcode)
    sdk_paths: dict[str, str] = {}
    seen: set[str] = set()
    for s in sdks:
        # canonicalName is e.g. "iphoneos18.0", "iphonesimulator18.0"
        name = s.get("canonicalName") or s.get("name")
        if not name or name in seen:
            continue
        seen.add(name)
        p = show_sdk_path(xcode, name)
        if p:
            sdk_paths[name] = p

    with (sdksettings_out / "sdk-paths.json").open("w") as f:
        json.dump(sdk_paths, f, indent=2, sort_keys=True)
        f.write("\n")

    meta = {
        "xcode_version": xcode.version,
        "product_build_version": product_build_version(xcode),
        "xcode_app": str(xcode.app_path),
        "developer_dir": str(xcode.developer_dir),
        "host_macos": common.host_macos_version(),
        "captured_at": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "xcspec_count": len(xcspecs),
        "sdksettings_count": len(sdksettings),
        "sdk_count": len(sdk_paths),
    }
    with meta_path.open("w") as f:
        json.dump(meta, f, indent=2, sort_keys=True)
        f.write("\n")
    common.log(f"wrote {meta_path}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--xcode", help="restrict to one Xcode (version like '26.0.1' or slot like 'current')")
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()

    installs = common.discover_installed_xcodes()
    if not installs:
        common.log("no installed Xcodes found; bail")
        return 1
    targets = common.selected_xcodes(installs, args.xcode)

    common.ensure_dir(common.XCSPEC_CACHE_DIR)
    had_error = False
    for x in targets:
        try:
            snapshot_one(x, force=args.force)
        except Exception as e:
            had_error = True
            common.log(f"ERROR {x.version}: {e}")
    return 1 if had_error else 0


if __name__ == "__main__":
    sys.exit(main())
