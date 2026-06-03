#!/usr/bin/env python3
"""Synthetic xcconfig fixtures exercising tricky syntax.

No corpus project we have uses these constructs end-to-end, so to flip the
remaining xcconfig probes green (and to give the Rust resolver oracles for
edge cases) we generate a tiny scratch xcodeproj and a battery of .xcconfig
files exercising each construct, then capture the resolved view per xcconfig.

Constructs covered:
  - `[sdk=iphone*]`  conditional (already covered by netnewswire, included
                     here as a control)
  - `[arch=arm64]`   architecture conditional
  - `[config=Debug]` configuration conditional
  - `$(VAR:lower)` / `$(VAR:upper)` / `$(VAR:default=...)`  modifier syntax
  - multi-line continuation (backslash at EOL)
  - `#include`

For each .xcconfig, two captures are run:
  - WITH the xcconfig as `-xcconfig FILE` (top-level precedence override)
  - WITHOUT the xcconfig (the same base command); used as a diff baseline

Output:
  fixtures/_synthetic-xcconfigs/xcode-<ver>/
      project/Scratch.xcodeproj/...     generated scratch project
      project/main.swift                generated source file
      xcconfigs/<name>.xcconfig         each xcconfig fixture
      captures/<name>/with.json         build settings WITH xcconfig
      captures/<name>/without.json      build settings WITHOUT xcconfig
      captures/<name>/meta.json         description + base command

Idempotent: existing captures are kept unless --force.

Flags:
  --xcode <ver|slot>    pick a specific Xcode (default: current)
  --force               re-capture even if outputs exist
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import textwrap
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


SYNTH_DIR = common.FIXTURES_DIR / "_synthetic-xcconfigs"


# A minimal but real Swift binary project. We hand-roll a project.pbxproj so
# we don't need Tuist/XcodeGen. Settings are intentionally sparse.
MAIN_SWIFT = """import Foundation
print("scratch")
"""


def _stable_uuid(seed: str) -> str:
    """24-char hex string — pbxproj uses 96-bit identifiers."""
    return uuid.uuid5(uuid.NAMESPACE_URL, f"sweetpad/{seed}").hex[:24].upper()


def render_pbxproj() -> str:
    """Render a minimal project.pbxproj with one Swift command-line target.

    Tested against Xcode 26.0.1. Object IDs are deterministic (uuid5-derived).
    """
    OBJ = {
        "project": _stable_uuid("project"),
        "main_group": _stable_uuid("main_group"),
        "products_group": _stable_uuid("products_group"),
        "target": _stable_uuid("target"),
        "build_config_list_proj": _stable_uuid("bcl_proj"),
        "build_config_list_target": _stable_uuid("bcl_target"),
        "config_debug_proj": _stable_uuid("cfg_debug_proj"),
        "config_release_proj": _stable_uuid("cfg_release_proj"),
        "config_debug_target": _stable_uuid("cfg_debug_target"),
        "config_release_target": _stable_uuid("cfg_release_target"),
        "main_source_ref": _stable_uuid("main_source_ref"),
        "main_build_file": _stable_uuid("main_build_file"),
        "sources_phase": _stable_uuid("sources_phase"),
        "product_ref": _stable_uuid("product_ref"),
    }

    return textwrap.dedent(f"""\
        // !$*UTF8*$!
        {{
            archiveVersion = 1;
            classes = {{}};
            objectVersion = 60;
            objects = {{
                {OBJ["main_build_file"]} = {{isa = PBXBuildFile; fileRef = {OBJ["main_source_ref"]}; }};
                {OBJ["main_source_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = main.swift; sourceTree = "<group>"; }};
                {OBJ["product_ref"]} = {{isa = PBXFileReference; explicitFileType = "compiled.mach-o.executable"; includeInIndex = 0; path = Scratch; sourceTree = BUILT_PRODUCTS_DIR; }};
                {OBJ["main_group"]} = {{
                    isa = PBXGroup;
                    children = (
                        {OBJ["main_source_ref"]},
                        {OBJ["products_group"]},
                    );
                    sourceTree = "<group>";
                }};
                {OBJ["products_group"]} = {{
                    isa = PBXGroup;
                    children = (
                        {OBJ["product_ref"]},
                    );
                    name = Products;
                    sourceTree = "<group>";
                }};
                {OBJ["target"]} = {{
                    isa = PBXNativeTarget;
                    buildConfigurationList = {OBJ["build_config_list_target"]};
                    buildPhases = (
                        {OBJ["sources_phase"]},
                    );
                    buildRules = ();
                    dependencies = ();
                    name = Scratch;
                    productName = Scratch;
                    productReference = {OBJ["product_ref"]};
                    productType = "com.apple.product-type.tool";
                }};
                {OBJ["project"]} = {{
                    isa = PBXProject;
                    attributes = {{
                        LastUpgradeCheck = 1500;
                    }};
                    buildConfigurationList = {OBJ["build_config_list_proj"]};
                    compatibilityVersion = "Xcode 14.0";
                    developmentRegion = en;
                    hasScannedForEncodings = 0;
                    knownRegions = (en, Base);
                    mainGroup = {OBJ["main_group"]};
                    productRefGroup = {OBJ["products_group"]};
                    projectDirPath = "";
                    projectRoot = "";
                    targets = ({OBJ["target"]});
                }};
                {OBJ["sources_phase"]} = {{
                    isa = PBXSourcesBuildPhase;
                    buildActionMask = 2147483647;
                    files = ({OBJ["main_build_file"]});
                    runOnlyForDeploymentPostprocessing = 0;
                }};
                {OBJ["config_debug_proj"]} = {{
                    isa = XCBuildConfiguration;
                    buildSettings = {{
                        ALWAYS_SEARCH_USER_PATHS = NO;
                        MACOSX_DEPLOYMENT_TARGET = 12.0;
                        SDKROOT = macosx;
                        SWIFT_VERSION = 5.0;
                    }};
                    name = Debug;
                }};
                {OBJ["config_release_proj"]} = {{
                    isa = XCBuildConfiguration;
                    buildSettings = {{
                        ALWAYS_SEARCH_USER_PATHS = NO;
                        MACOSX_DEPLOYMENT_TARGET = 12.0;
                        SDKROOT = macosx;
                        SWIFT_VERSION = 5.0;
                    }};
                    name = Release;
                }};
                {OBJ["config_debug_target"]} = {{
                    isa = XCBuildConfiguration;
                    buildSettings = {{
                        PRODUCT_NAME = Scratch;
                    }};
                    name = Debug;
                }};
                {OBJ["config_release_target"]} = {{
                    isa = XCBuildConfiguration;
                    buildSettings = {{
                        PRODUCT_NAME = Scratch;
                    }};
                    name = Release;
                }};
                {OBJ["build_config_list_proj"]} = {{
                    isa = XCConfigurationList;
                    buildConfigurations = (
                        {OBJ["config_debug_proj"]},
                        {OBJ["config_release_proj"]},
                    );
                    defaultConfigurationIsVisible = 0;
                    defaultConfigurationName = Release;
                }};
                {OBJ["build_config_list_target"]} = {{
                    isa = XCConfigurationList;
                    buildConfigurations = (
                        {OBJ["config_debug_target"]},
                        {OBJ["config_release_target"]},
                    );
                    defaultConfigurationIsVisible = 0;
                    defaultConfigurationName = Release;
                }};
            }};
            rootObject = {OBJ["project"]};
        }}
        """)


# Each xcconfig is a (name, body, description) tuple. The body is written to
# `xcconfigs/<name>.xcconfig` verbatim.
XCCONFIGS: list[tuple[str, str, str]] = [
    ("conditional-sdk",
     textwrap.dedent("""\
        // Tests [sdk=...] conditional resolution.
        FOO = base
        FOO[sdk=iphoneos*] = ios_device
        FOO[sdk=iphonesimulator*] = ios_sim
        FOO[sdk=macosx*] = macos
        """),
     "Conditional [sdk=...] in xcconfig"),

    ("conditional-arch",
     textwrap.dedent("""\
        // Tests [arch=...] conditional resolution.
        BAR = base
        BAR[arch=arm64] = arm64_val
        BAR[arch=arm64e] = arm64e_val
        BAR[arch=x86_64] = x86_64_val
        """),
     "Conditional [arch=...] in xcconfig"),

    ("conditional-config",
     textwrap.dedent("""\
        // Tests [config=...] conditional resolution.
        BAZ = base
        BAZ[config=Debug] = debug_val
        BAZ[config=Release] = release_val
        """),
     "Conditional [config=...] in xcconfig"),

    ("multi-line-continuation",
     textwrap.dedent("""\
        // Tests multi-line continuation in xcconfig (backslash + newline).
        QUUX = first_part \\
            second_part \\
            third_part
        """),
     "Multi-line continuation in xcconfig"),

    ("modifier-syntax",
     textwrap.dedent("""\
        // Tests modifier syntax: ${VAR:lower}, ${VAR:upper}, ${VAR:default=...}.
        BASE_NAME = HelloWorld
        LOWER_NAME = ${BASE_NAME:lower}
        UPPER_NAME = ${BASE_NAME:upper}
        DEFAULTED = ${UNSET_VAR:default=fallback}
        """),
     "Modifier syntax ${VAR:lower}/${VAR:default=...} in xcconfig"),

    ("include-directive",
     textwrap.dedent("""\
        // Tests #include directive.
        #include "conditional-sdk.xcconfig"
        EXTRA = layered
        """),
     "xcconfig #include directive"),

    ("inherited",
     textwrap.dedent("""\
        // Tests $(inherited) recursive layering.
        OTHER_SWIFT_FLAGS = $(inherited) -DMY_FLAG
        OTHER_LDFLAGS = $(inherited) -framework Foundation
        """),
     "$(inherited) used in xcconfig"),
]


def materialize_scratch(out_root: Path) -> Path:
    """Create the scratch xcodeproj under <out_root>/project/. Returns project path."""
    proj_root = out_root / "project"
    proj_root.mkdir(parents=True, exist_ok=True)
    (proj_root / "main.swift").write_text(MAIN_SWIFT)
    xcodeproj = proj_root / "Scratch.xcodeproj"
    xcodeproj.mkdir(parents=True, exist_ok=True)
    pbxproj_path = xcodeproj / "project.pbxproj"
    pbxproj_path.write_text(render_pbxproj())
    return xcodeproj


def materialize_xcconfigs(out_root: Path) -> Path:
    cfg_dir = out_root / "xcconfigs"
    cfg_dir.mkdir(parents=True, exist_ok=True)
    for name, body, _desc in XCCONFIGS:
        (cfg_dir / f"{name}.xcconfig").write_text(body)
    return cfg_dir


def xb_env(xcode: common.XcodeInstall) -> dict[str, str]:
    e = dict(os.environ)
    e["DEVELOPER_DIR"] = str(xcode.developer_dir)
    return e


def capture_one(xcconfig_path: Path | None, xcodeproj: Path, target: str, config: str,
                dest: str, out_path: Path, *, xcode: common.XcodeInstall,
                ) -> tuple[bool, str]:
    cmd = ["xcodebuild", "-showBuildSettings", "-json",
           "-project", str(xcodeproj),
           "-target", target, "-configuration", config,
           "-destination", dest]
    if xcconfig_path is not None:
        cmd += ["-xcconfig", str(xcconfig_path)]
    cp = subprocess.run(cmd, env=xb_env(xcode), capture_output=True, text=True,
                        timeout=120)
    if cp.returncode != 0 or not cp.stdout.strip():
        return False, (cp.stderr or cp.stdout)[-400:]
    try:
        parsed = json.loads(cp.stdout)
    except json.JSONDecodeError as e:
        return False, f"non-JSON: {e}"
    with out_path.open("w") as f:
        json.dump(parsed, f, indent=2, sort_keys=True)
        f.write("\n")
    return True, ""


def process(xcode: common.XcodeInstall, *, force: bool) -> None:
    out_root = SYNTH_DIR / f"xcode-{xcode.version}"
    out_root.mkdir(parents=True, exist_ok=True)
    xcodeproj = materialize_scratch(out_root)
    xcconfigs_dir = materialize_xcconfigs(out_root)

    target = "Scratch"
    config = "Debug"
    dest = "platform=macOS"
    captures_root = out_root / "captures"
    captures_root.mkdir(parents=True, exist_ok=True)

    for name, _body, desc in XCCONFIGS:
        xcc = xcconfigs_dir / f"{name}.xcconfig"
        case_dir = captures_root / name
        case_dir.mkdir(parents=True, exist_ok=True)

        with_path = case_dir / "with.json"
        without_path = case_dir / "without.json"
        meta_path = case_dir / "meta.json"

        if not force and with_path.exists() and without_path.exists():
            continue

        common.log(f"capture: {name}")
        ok_with, info_with = capture_one(
            xcc, xcodeproj, target, config, dest, with_path, xcode=xcode,
        )
        ok_without, info_without = capture_one(
            None, xcodeproj, target, config, dest, without_path, xcode=xcode,
        )
        with meta_path.open("w") as f:
            json.dump({
                "name": name,
                "description": desc,
                "xcconfig": str(xcc.relative_to(out_root)),
                "project": str(xcodeproj.relative_to(out_root)),
                "target": target,
                "configuration": config,
                "destination": dest,
                "ok_with": ok_with,
                "ok_without": ok_without,
            }, f, indent=2, sort_keys=True)
            f.write("\n")
        if not ok_with:
            common.log(f"  WARN with-xcconfig failed: {info_with[-200:]}")
        if not ok_without:
            common.log(f"  WARN without-xcconfig failed: {info_without[-200:]}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--xcode", help="restrict to one Xcode (version or slot)")
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()

    installs = common.discover_installed_xcodes()
    xcodes = common.selected_xcodes(installs, args.xcode)

    for x in xcodes:
        common.log(f"\n========= _synthetic-xcconfigs :: xcode {x.version} =========")
        try:
            with common.with_xcode(x):
                process(x, force=args.force)
        except Exception as e:
            common.log(f"ERROR: {e}")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
