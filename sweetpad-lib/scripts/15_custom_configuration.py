#!/usr/bin/env python3
"""Synthetic custom-configuration fixture.

No corpus project defines a build configuration *named* anything other than
`Debug`/`Release`, so the resolver's config-name-driven selection (pick the
`XCBuildConfiguration` whose `name` matches an arbitrary config, apply its
`buildSettings`, fire `[config=<name>]` xcconfig conditionals) is never
exercised end-to-end. This generates a tiny scratch macOS tool with a THIRD
configuration `Profile`, then captures the resolved view per configuration.

`Profile` is distinguished two ways, one per resolution layer:
  - a per-config pbxproj setting       `PBXPROJ_MARKER = profile`   (debug/release/profile)
  - a `[config=Profile]` xcconfig entry `XCCONFIG_MARKER = profile`  (base elsewhere)
The xcconfig is wired as the `baseConfigurationReference` of every config, so
the same file resolves differently depending only on the selected config name.

Captures are no-destination single-target `-showBuildSettings`, mirroring
`09_per_project_settings.py` so the oracle is a bare
`ResolveQuery::new(target, config, sdk, arch)` (no scheme, no destination).

Output:
  fixtures/_synthetic-custom-config/xcode-<ver>/
      project/Scratch.xcodeproj/...     generated scratch project (3 configs)
      project/main.swift                generated source file
      project/Shared.xcconfig           base xcconfig with a [config=Profile] entry
      captures/Scratch__<config>.json   build settings per configuration
      captures/meta.json                description + the captured configs

Idempotent: existing captures are kept unless --force.

Flags:
  --xcode <ver|slot>    pick a specific Xcode (default: current)
  --force               re-capture even if outputs exist
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import textwrap
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


SYNTH_DIR = common.FIXTURES_DIR / "_synthetic-custom-config"

# The three configurations the scratch project defines. `Profile` is the one
# that doesn't exist in any real corpus project — its per-config pbxproj marker
# and the `[config=Profile]` xcconfig entry are what this fixture proves resolve.
CONFIGS = ["Debug", "Release", "Profile"]

MAIN_SWIFT = """import Foundation
print("scratch")
"""

# Base xcconfig wired as every config's baseConfigurationReference. `[config=...]`
# selects per the *selected configuration name*, so the custom `Profile` arm only
# fires when the project is resolved under `-configuration Profile`.
SHARED_XCCONFIG = """\
// baseConfigurationReference for every configuration. The [config=Profile] arm
// only resolves when the custom `Profile` configuration is selected.
XCCONFIG_MARKER = base
XCCONFIG_MARKER[config=Profile] = profile
"""

# Per-config target buildSettings marker — distinct from the xcconfig marker so
# the two resolution layers (pbxproj inline vs baseConfiguration xcconfig) are
# tested independently. `Profile` mirrors a Release-style optimized build.
_TARGET_CONFIG_SETTINGS = {
    "Debug": {
        "PBXPROJ_MARKER": "debug",
        "PRODUCT_NAME": "Scratch",
        "SWIFT_OPTIMIZATION_LEVEL": '"-Onone"',
    },
    "Release": {
        "PBXPROJ_MARKER": "release",
        "PRODUCT_NAME": "Scratch",
        "SWIFT_OPTIMIZATION_LEVEL": '"-O"',
    },
    "Profile": {
        "PBXPROJ_MARKER": "profile",
        "PRODUCT_NAME": "Scratch",
        "SWIFT_OPTIMIZATION_LEVEL": '"-O"',
        "SWIFT_COMPILATION_MODE": "wholemodule",
    },
}

_PROJECT_CONFIG_SETTINGS = {
    "ALWAYS_SEARCH_USER_PATHS": "NO",
    "MACOSX_DEPLOYMENT_TARGET": "12.0",
    "SDKROOT": "macosx",
    "SWIFT_VERSION": "5.0",
}


def _stable_uuid(seed: str) -> str:
    """24-char hex string — pbxproj uses 96-bit identifiers."""
    return uuid.uuid5(uuid.NAMESPACE_URL, f"sweetpad/custom-config/{seed}").hex[:24].upper()


def _render_settings(settings: dict[str, str], indent: str) -> str:
    return "".join(f"{indent}{k} = {v};\n" for k, v in settings.items())


def render_pbxproj() -> str:
    """Render a minimal one-target macOS tool with Debug/Release/Profile configs.

    Every project-level config references `Shared.xcconfig` as its
    baseConfigurationReference. Object IDs are deterministic (uuid5-derived).
    """
    obj = {
        "project": _stable_uuid("project"),
        "main_group": _stable_uuid("main_group"),
        "products_group": _stable_uuid("products_group"),
        "target": _stable_uuid("target"),
        "bcl_proj": _stable_uuid("bcl_proj"),
        "bcl_target": _stable_uuid("bcl_target"),
        "main_source_ref": _stable_uuid("main_source_ref"),
        "main_build_file": _stable_uuid("main_build_file"),
        "sources_phase": _stable_uuid("sources_phase"),
        "product_ref": _stable_uuid("product_ref"),
        "xcconfig_ref": _stable_uuid("xcconfig_ref"),
    }
    for cfg in CONFIGS:
        obj[f"cfg_{cfg}_proj"] = _stable_uuid(f"cfg_{cfg}_proj")
        obj[f"cfg_{cfg}_target"] = _stable_uuid(f"cfg_{cfg}_target")

    # XCBuildConfiguration objects (project- and target-level) for each config.
    config_objects = ""
    for cfg in CONFIGS:
        config_objects += textwrap.dedent(f"""\
            {obj[f"cfg_{cfg}_proj"]} = {{
                isa = XCBuildConfiguration;
                baseConfigurationReference = {obj["xcconfig_ref"]};
                buildSettings = {{
{_render_settings(_PROJECT_CONFIG_SETTINGS, " " * 20)}                }};
                name = {cfg};
            }};
            {obj[f"cfg_{cfg}_target"]} = {{
                isa = XCBuildConfiguration;
                buildSettings = {{
{_render_settings(_TARGET_CONFIG_SETTINGS[cfg], " " * 20)}                }};
                name = {cfg};
            }};
        """)

    proj_cfg_list = "\n".join(
        f'                    {obj[f"cfg_{cfg}_proj"]},' for cfg in CONFIGS
    )
    target_cfg_list = "\n".join(
        f'                    {obj[f"cfg_{cfg}_target"]},' for cfg in CONFIGS
    )
    config_objects = textwrap.indent(config_objects, " " * 16)

    return textwrap.dedent(f"""\
        // !$*UTF8*$!
        {{
            archiveVersion = 1;
            classes = {{}};
            objectVersion = 60;
            objects = {{
                {obj["main_build_file"]} = {{isa = PBXBuildFile; fileRef = {obj["main_source_ref"]}; }};
                {obj["main_source_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = main.swift; sourceTree = "<group>"; }};
                {obj["xcconfig_ref"]} = {{isa = PBXFileReference; lastKnownFileType = text.xcconfig; path = Shared.xcconfig; sourceTree = "<group>"; }};
                {obj["product_ref"]} = {{isa = PBXFileReference; explicitFileType = "compiled.mach-o.executable"; includeInIndex = 0; path = Scratch; sourceTree = BUILT_PRODUCTS_DIR; }};
                {obj["main_group"]} = {{
                    isa = PBXGroup;
                    children = (
                        {obj["main_source_ref"]},
                        {obj["xcconfig_ref"]},
                        {obj["products_group"]},
                    );
                    sourceTree = "<group>";
                }};
                {obj["products_group"]} = {{
                    isa = PBXGroup;
                    children = (
                        {obj["product_ref"]},
                    );
                    name = Products;
                    sourceTree = "<group>";
                }};
                {obj["target"]} = {{
                    isa = PBXNativeTarget;
                    buildConfigurationList = {obj["bcl_target"]};
                    buildPhases = (
                        {obj["sources_phase"]},
                    );
                    buildRules = ();
                    dependencies = ();
                    name = Scratch;
                    productName = Scratch;
                    productReference = {obj["product_ref"]};
                    productType = "com.apple.product-type.tool";
                }};
                {obj["project"]} = {{
                    isa = PBXProject;
                    attributes = {{
                        LastUpgradeCheck = 1500;
                    }};
                    buildConfigurationList = {obj["bcl_proj"]};
                    compatibilityVersion = "Xcode 14.0";
                    developmentRegion = en;
                    hasScannedForEncodings = 0;
                    knownRegions = (en, Base);
                    mainGroup = {obj["main_group"]};
                    productRefGroup = {obj["products_group"]};
                    projectDirPath = "";
                    projectRoot = "";
                    targets = ({obj["target"]});
                }};
                {obj["sources_phase"]} = {{
                    isa = PBXSourcesBuildPhase;
                    buildActionMask = 2147483647;
                    files = ({obj["main_build_file"]});
                    runOnlyForDeploymentPostprocessing = 0;
                }};
{config_objects}                {obj["bcl_proj"]} = {{
                    isa = XCConfigurationList;
                    buildConfigurations = (
{proj_cfg_list}
                    );
                    defaultConfigurationIsVisible = 0;
                    defaultConfigurationName = Release;
                }};
                {obj["bcl_target"]} = {{
                    isa = XCConfigurationList;
                    buildConfigurations = (
{target_cfg_list}
                    );
                    defaultConfigurationIsVisible = 0;
                    defaultConfigurationName = Release;
                }};
            }};
            rootObject = {obj["project"]};
        }}
        """)


def materialize_scratch(out_root: Path) -> Path:
    """Create the scratch xcodeproj + xcconfig under <out_root>/project/."""
    proj_root = out_root / "project"
    proj_root.mkdir(parents=True, exist_ok=True)
    (proj_root / "main.swift").write_text(MAIN_SWIFT)
    (proj_root / "Shared.xcconfig").write_text(SHARED_XCCONFIG)
    xcodeproj = proj_root / "Scratch.xcodeproj"
    xcodeproj.mkdir(parents=True, exist_ok=True)
    (xcodeproj / "project.pbxproj").write_text(render_pbxproj())
    return xcodeproj


def xb_env(xcode: common.XcodeInstall) -> dict[str, str]:
    e = dict(os.environ)
    e["DEVELOPER_DIR"] = str(xcode.developer_dir)
    return e


def capture_one(xcodeproj: Path, target: str, config: str, out_path: Path,
                *, xcode: common.XcodeInstall) -> tuple[bool, str]:
    """No-destination single-target `-showBuildSettings` (per-target style)."""
    cmd = ["xcodebuild", "-showBuildSettings", "-json",
           "-project", str(xcodeproj),
           "-target", target, "-configuration", config]
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

    target = "Scratch"
    captures_root = out_root / "captures"
    captures_root.mkdir(parents=True, exist_ok=True)

    captured: list[str] = []
    for config in CONFIGS:
        out_path = captures_root / f"{target}__{config}.json"
        if not force and out_path.exists():
            captured.append(config)
            continue
        common.log(f"capture: {config}")
        ok, info = capture_one(xcodeproj, target, config, out_path, xcode=xcode)
        if ok:
            captured.append(config)
        else:
            common.log(f"  WARN {config} capture failed: {info[-200:]}")

    with (captures_root / "meta.json").open("w") as f:
        json.dump({
            "description": "Custom-configuration resolution (Debug/Release/Profile)",
            "project": str(xcodeproj.relative_to(out_root)),
            "target": target,
            "configs": CONFIGS,
            "captured": captured,
            "markers": {
                "PBXPROJ_MARKER": "per-config pbxproj target setting",
                "XCCONFIG_MARKER": "[config=Profile] entry in Shared.xcconfig",
            },
        }, f, indent=2, sort_keys=True)
        f.write("\n")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--xcode", help="restrict to one Xcode (version or slot)")
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()

    installs = common.discover_installed_xcodes()
    xcodes = common.selected_xcodes(installs, args.xcode)

    for x in xcodes:
        common.log(f"\n========= _synthetic-custom-config :: xcode {x.version} =========")
        try:
            with common.with_xcode(x):
                process(x, force=args.force)
        except Exception as e:
            common.log(f"ERROR: {e}")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
