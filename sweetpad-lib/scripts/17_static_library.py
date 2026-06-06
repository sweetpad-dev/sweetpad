#!/usr/bin/env python3
"""Synthetic static-library fixture for the compiler-args oracle.

No corpus project ships a static-library target, so the resolver's `libtool`
link path (`MACH_O_TYPE = staticlib` / `product-type.library.static`) has no
oracle. This generates a tiny static library (one Swift + one C source, so the
archive holds both a swiftc and a clang object) and captures the real per-tool
commands via `16_capture_compiler_args.py` — the `libtool -static` link plus the
swiftc/clang compiles.

Output (committed):
  fixtures/_synthetic-staticlib/xcode-<ver>/raw/Scratch.xcodeproj   project to resolve
  fixtures/_synthetic-staticlib/xcode-<ver>/compiler-args/Scratch__Debug__macOS.json

The project is generated into the gitignored `corpus/_synthetic-staticlib/` (the
build sandbox `16_*` expects), then copied into the committed `raw/` so the
oracle test can open it. Object identifiers are deterministic (uuid5-derived).

Flags:
  --xcode <ver|slot>    pick a specific Xcode (default: every installed)
  --force               re-capture even if outputs exist
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import textwrap
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

SLUG = "_synthetic-staticlib"
TARGET = "Scratch"
SCHEME = "Scratch"
CONFIG = "Debug"

MAIN_SWIFT = """import Foundation

public func scratchGreeting() -> String { "scratch" }
"""

# A second Swift file so the Debug build compiles incrementally in batches (a
# lone source makes swiftc pick whole-module, which a real library never is).
HELPER_SWIFT = """public func scratchAnswer() -> Int { 42 }
"""

# An ObjC++ source — the archive then holds a swiftc and a clang `.mm` object,
# exercising the `sourcecode.cpp.objcpp` language gate (C++ and ObjC flags, no
# C-only `-std`). Single-language so the per-file flags fold cleanly.
UTIL_MM = """#import <Foundation/Foundation.h>
#include <string>

NSString *scratchDescribe(int n) {
    std::string s = std::to_string(n);
    return [NSString stringWithUTF8String:s.c_str()];
}
"""

# Project-level settings shared by both configs. (PRODUCT_NAME is set per target
# below — a `$(...)` value must be quoted in pbxproj, so it stays out of here.)
PROJECT_SETTINGS = {
    "ALWAYS_SEARCH_USER_PATHS": "NO",
    "MACOSX_DEPLOYMENT_TARGET": "12.0",
    "ONLY_ACTIVE_ARCH": "YES",
    "SDKROOT": "macosx",
    # A real Debug config always pins this; without it the resolver falls to a
    # default that reads as whole-module while the build compiles incrementally.
    "SWIFT_OPTIMIZATION_LEVEL": '"-Onone"',
    "SWIFT_VERSION": "5.0",
}


def _uuid(seed: str) -> str:
    return uuid.uuid5(uuid.NAMESPACE_URL, f"sweetpad/staticlib/{seed}").hex[:24].upper()


def render_pbxproj(obj: dict[str, str]) -> str:
    settings = "".join(f"                    {k} = {v};\n" for k, v in PROJECT_SETTINGS.items())
    config_objects = ""
    for cfg in ("Debug", "Release"):
        config_objects += textwrap.dedent(f"""\
            {obj[f"cfg_{cfg}_proj"]} = {{
                isa = XCBuildConfiguration;
                buildSettings = {{
{settings}                }};
                name = {cfg};
            }};
            {obj[f"cfg_{cfg}_target"]} = {{
                isa = XCBuildConfiguration;
                buildSettings = {{
                    PRODUCT_NAME = "$(TARGET_NAME)";
                }};
                name = {cfg};
            }};
        """)
    config_objects = textwrap.indent(config_objects, " " * 16)
    proj_cfgs = "\n".join(f'                    {obj[f"cfg_{c}_proj"]},' for c in ("Debug", "Release"))
    target_cfgs = "\n".join(f'                    {obj[f"cfg_{c}_target"]},' for c in ("Debug", "Release"))

    return textwrap.dedent(f"""\
        // !$*UTF8*$!
        {{
            archiveVersion = 1;
            classes = {{}};
            objectVersion = 60;
            objects = {{
                {obj["swift_build"]} = {{isa = PBXBuildFile; fileRef = {obj["swift_ref"]}; }};
                {obj["swift2_build"]} = {{isa = PBXBuildFile; fileRef = {obj["swift2_ref"]}; }};
                {obj["c_build"]} = {{isa = PBXBuildFile; fileRef = {obj["c_ref"]}; }};
                {obj["swift_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = main.swift; sourceTree = "<group>"; }};
                {obj["swift2_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = helper.swift; sourceTree = "<group>"; }};
                {obj["c_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.cpp.objcpp; path = util.mm; sourceTree = "<group>"; }};
                {obj["product_ref"]} = {{isa = PBXFileReference; explicitFileType = archive.ar; includeInIndex = 0; path = libScratch.a; sourceTree = BUILT_PRODUCTS_DIR; }};
                {obj["main_group"]} = {{
                    isa = PBXGroup;
                    children = (
                        {obj["swift_ref"]},
                        {obj["swift2_ref"]},
                        {obj["c_ref"]},
                        {obj["products_group"]},
                    );
                    sourceTree = "<group>";
                }};
                {obj["products_group"]} = {{
                    isa = PBXGroup;
                    children = ({obj["product_ref"]});
                    name = Products;
                    sourceTree = "<group>";
                }};
                {obj["target"]} = {{
                    isa = PBXNativeTarget;
                    buildConfigurationList = {obj["bcl_target"]};
                    buildPhases = ({obj["sources_phase"]});
                    buildRules = ();
                    dependencies = ();
                    name = Scratch;
                    productName = Scratch;
                    productReference = {obj["product_ref"]};
                    productType = "com.apple.product-type.library.static";
                }};
                {obj["project"]} = {{
                    isa = PBXProject;
                    attributes = {{ LastUpgradeCheck = 1500; }};
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
                    files = (
                        {obj["swift_build"]},
                        {obj["swift2_build"]},
                        {obj["c_build"]},
                    );
                    runOnlyForDeploymentPostprocessing = 0;
                }};
{config_objects}                {obj["bcl_proj"]} = {{
                    isa = XCConfigurationList;
                    buildConfigurations = (
{proj_cfgs}
                    );
                    defaultConfigurationIsVisible = 0;
                    defaultConfigurationName = Release;
                }};
                {obj["bcl_target"]} = {{
                    isa = XCConfigurationList;
                    buildConfigurations = (
{target_cfgs}
                    );
                    defaultConfigurationIsVisible = 0;
                    defaultConfigurationName = Release;
                }};
            }};
            rootObject = {obj["project"]};
        }}
        """)


def render_scheme(target_uuid: str) -> str:
    return textwrap.dedent(f"""\
        <?xml version="1.0" encoding="UTF-8"?>
        <Scheme LastUpgradeVersion="1500" version="1.7">
           <BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES">
              <BuildActionEntries>
                 <BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES">
                    <BuildableReference
                       BuildableIdentifier="primary"
                       BlueprintIdentifier="{target_uuid}"
                       BuildableName="libScratch.a"
                       BlueprintName="Scratch"
                       ReferencedContainer="container:Scratch.xcodeproj">
                    </BuildableReference>
                 </BuildActionEntry>
              </BuildActionEntries>
           </BuildAction>
        </Scheme>
        """)


def materialize(root: Path) -> Path:
    """Create the static-library project + shared scheme under <root>."""
    root.mkdir(parents=True, exist_ok=True)
    (root / "main.swift").write_text(MAIN_SWIFT)
    (root / "helper.swift").write_text(HELPER_SWIFT)
    (root / "util.mm").write_text(UTIL_MM)
    keys = [
        "project", "main_group", "products_group", "target", "bcl_proj", "bcl_target",
        "swift_ref", "swift_build", "swift2_ref", "swift2_build",
        "c_ref", "c_build", "product_ref", "sources_phase",
    ]
    obj = {k: _uuid(k) for k in keys}
    for cfg in ("Debug", "Release"):
        obj[f"cfg_{cfg}_proj"] = _uuid(f"cfg_{cfg}_proj")
        obj[f"cfg_{cfg}_target"] = _uuid(f"cfg_{cfg}_target")
    xcodeproj = root / "Scratch.xcodeproj"
    xcodeproj.mkdir(parents=True, exist_ok=True)
    (xcodeproj / "project.pbxproj").write_text(render_pbxproj(obj))
    schemes = xcodeproj / "xcshareddata" / "xcschemes"
    schemes.mkdir(parents=True, exist_ok=True)
    (schemes / "Scratch.xcscheme").write_text(render_scheme(obj["target"]))
    return xcodeproj


def process(xcode: common.XcodeInstall, *, force: bool) -> int:
    build_root = common.CORPUS_DIR / SLUG
    xcodeproj = materialize(build_root)

    out_path = (
        common.fixture_dir(SLUG, xcode.version)
        / "compiler-args"
        / f"{common.slug(SCHEME)}__{common.slug(CONFIG)}__macOS.json"
    )
    if out_path.exists() and not force:
        common.log(f"  exists, skip (use --force): {out_path}")
    else:
        cmd = [
            sys.executable, str(Path(__file__).resolve().parent / "16_capture_compiler_args.py"),
            "--slug", SLUG, "--xcode", xcode.version, "--scheme", SCHEME,
            "--config", CONFIG, "--destination", "platform=macOS", "--dest-slug", "macOS",
            "--project", "Scratch.xcodeproj",
        ]
        cp = subprocess.run(cmd, capture_output=True, text=True)
        sys.stdout.write(cp.stdout)
        sys.stderr.write(cp.stderr)
        if cp.returncode != 0:
            return cp.returncode

    # Commit the project so the oracle test can resolve the target.
    raw = common.fixture_dir(SLUG, xcode.version) / "raw" / "Scratch.xcodeproj"
    if raw.exists():
        shutil.rmtree(raw)
    raw.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(xcodeproj, raw)
    common.log(f"  copied project -> {raw}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--xcode", help="restrict to one Xcode (version or slot)")
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()

    installs = common.discover_installed_xcodes()
    for x in common.selected_xcodes(installs, args.xcode):
        common.log(f"\n========= {SLUG} :: xcode {x.version} =========")
        rc = process(x, force=args.force)
        if rc != 0:
            return rc
    return 0


if __name__ == "__main__":
    sys.exit(main())
