#!/usr/bin/env python3
"""Synthetic ObjC-headers fixture for the BSP harness (see DOCS.md §8 (BSP server)).

The multi-module fixture is pure Swift, so the **clang** search-path surface goes
unexercised. This materializes a tiny ObjC static library whose source
`#import`s a header that lives in a different directory, reachable only via
`HEADER_SEARCH_PATHS`. When the editor opens `widget.m`, the BSP server's clang
arguments must carry `-I <include>` or the header won't resolve — which is the
gap Layer 0 catches.

Like the multi-module fixture, the whole project (sources + header) is committed
so the harness is hermetic.

Output (committed):
  fixtures/_synthetic-objc-headers/project/ObjCHeaders.xcodeproj
  fixtures/_synthetic-objc-headers/project/{widget.m, include/widget.h}

Flags:
  --force   overwrite an existing project
"""

from __future__ import annotations

import argparse
import sys
import textwrap
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

SLUG = "_synthetic-objc-headers"

# The header lives in include/; widget.m reaches it only via HEADER_SEARCH_PATHS.
WIDGET_H = """int widget_value(void);
"""

WIDGET_M = """#import "widget.h"

int widget_value(void) { return 42; }
"""

PROJECT_SETTINGS = {
    "ALWAYS_SEARCH_USER_PATHS": "NO",
    "MACOSX_DEPLOYMENT_TARGET": "12.0",
    "ONLY_ACTIVE_ARCH": "YES",
    "SDKROOT": "macosx",
    # The whole point: the header is found only through this search path.
    "HEADER_SEARCH_PATHS": '"$(SRCROOT)/include"',
}


def _uuid(seed: str) -> str:
    return uuid.uuid5(uuid.NAMESPACE_URL, f"sweetpad/objc-headers/{seed}").hex[:24].upper()


def render_pbxproj(o: dict[str, str]) -> str:
    settings = "".join(f"                    {k} = {v};\n" for k, v in PROJECT_SETTINGS.items())
    cfgs = ""
    for cfg in ("Debug", "Release"):
        cfgs += textwrap.dedent(f"""\
            {o[f"cfg_proj_{cfg}"]} = {{
                isa = XCBuildConfiguration;
                buildSettings = {{
{settings}                }};
                name = {cfg};
            }};
            {o[f"cfg_tgt_{cfg}"]} = {{
                isa = XCBuildConfiguration;
                buildSettings = {{ PRODUCT_NAME = "$(TARGET_NAME)"; }};
                name = {cfg};
            }};
        """)
    cfgs = textwrap.indent(cfgs, " " * 16)
    proj_cfgs = "\n".join(f'                    {o[f"cfg_proj_{c}"]},' for c in ("Debug", "Release"))
    tgt_cfgs = "\n".join(f'                    {o[f"cfg_tgt_{c}"]},' for c in ("Debug", "Release"))

    return textwrap.dedent(f"""\
        // !$*UTF8*$!
        {{
            archiveVersion = 1;
            classes = {{}};
            objectVersion = 60;
            objects = {{
                {o["m_build"]} = {{isa = PBXBuildFile; fileRef = {o["m_ref"]}; }};
                {o["m_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.c.objc; path = widget.m; sourceTree = "<group>"; }};
                {o["h_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.c.h; path = widget.h; sourceTree = "<group>"; }};
                {o["product"]} = {{isa = PBXFileReference; explicitFileType = archive.ar; includeInIndex = 0; path = libObjCHeaders.a; sourceTree = BUILT_PRODUCTS_DIR; }};
                {o["inc_group"]} = {{ isa = PBXGroup; children = ({o["h_ref"]}); path = include; sourceTree = "<group>"; }};
                {o["main_group"]} = {{
                    isa = PBXGroup;
                    children = ({o["m_ref"]}, {o["inc_group"]}, {o["products_group"]});
                    sourceTree = "<group>";
                }};
                {o["products_group"]} = {{ isa = PBXGroup; children = ({o["product"]}); name = Products; sourceTree = "<group>"; }};
                {o["target"]} = {{
                    isa = PBXNativeTarget;
                    buildConfigurationList = {o["bcl_tgt"]};
                    buildPhases = ({o["sources"]});
                    buildRules = ();
                    dependencies = ();
                    name = ObjCHeaders;
                    productName = ObjCHeaders;
                    productReference = {o["product"]};
                    productType = "com.apple.product-type.library.static";
                }};
                {o["project"]} = {{
                    isa = PBXProject;
                    attributes = {{ LastUpgradeCheck = 1500; }};
                    buildConfigurationList = {o["bcl_proj"]};
                    compatibilityVersion = "Xcode 14.0";
                    developmentRegion = en;
                    hasScannedForEncodings = 0;
                    knownRegions = (en, Base);
                    mainGroup = {o["main_group"]};
                    productRefGroup = {o["products_group"]};
                    projectDirPath = "";
                    projectRoot = "";
                    targets = ({o["target"]});
                }};
                {o["sources"]} = {{
                    isa = PBXSourcesBuildPhase;
                    buildActionMask = 2147483647;
                    files = ({o["m_build"]});
                    runOnlyForDeploymentPostprocessing = 0;
                }};
{cfgs}                {o["bcl_proj"]} = {{
                    isa = XCConfigurationList;
                    buildConfigurations = (
{proj_cfgs}
                    );
                    defaultConfigurationIsVisible = 0;
                    defaultConfigurationName = Release;
                }};
                {o["bcl_tgt"]} = {{
                    isa = XCConfigurationList;
                    buildConfigurations = (
{tgt_cfgs}
                    );
                    defaultConfigurationIsVisible = 0;
                    defaultConfigurationName = Release;
                }};
            }};
            rootObject = {o["project"]};
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
                       BuildableName="libObjCHeaders.a"
                       BlueprintName="ObjCHeaders"
                       ReferencedContainer="container:ObjCHeaders.xcodeproj">
                    </BuildableReference>
                 </BuildActionEntry>
              </BuildActionEntries>
           </BuildAction>
        </Scheme>
        """)


def materialize(root: Path) -> Path:
    keys = [
        "project", "main_group", "products_group", "inc_group", "target",
        "bcl_proj", "bcl_tgt", "m_ref", "m_build", "h_ref", "product", "sources",
    ]
    o = {k: _uuid(k) for k in keys}
    for cfg in ("Debug", "Release"):
        o[f"cfg_proj_{cfg}"] = _uuid(f"cfg_proj_{cfg}")
        o[f"cfg_tgt_{cfg}"] = _uuid(f"cfg_tgt_{cfg}")

    (root / "include").mkdir(parents=True, exist_ok=True)
    (root / "widget.m").write_text(WIDGET_M)
    (root / "include" / "widget.h").write_text(WIDGET_H)
    xcodeproj = root / "ObjCHeaders.xcodeproj"
    xcodeproj.mkdir(parents=True, exist_ok=True)
    (xcodeproj / "project.pbxproj").write_text(render_pbxproj(o))
    schemes = xcodeproj / "xcshareddata" / "xcschemes"
    schemes.mkdir(parents=True, exist_ok=True)
    (schemes / "ObjCHeaders.xcscheme").write_text(render_scheme(o["target"]))
    return xcodeproj


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()
    root = common.FIXTURES_DIR / SLUG / "project"
    if root.exists() and not args.force:
        common.log(f"exists, skip (use --force): {root}")
        return 0
    xcodeproj = materialize(root)
    common.log(f"wrote {xcodeproj}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
