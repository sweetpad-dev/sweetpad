#!/usr/bin/env python3
"""Synthetic multi-module fixture for the BSP harness (see DOCS.md §8 (BSP server)).

The compiler-args corpus is single-module, so cross-module `import` resolution —
the crux of editor intelligence — goes unexercised. This materializes a tiny
two-module Xcode project where **ModuleB imports ModuleA**, with the dependency
declared so `xcodebuild` builds A first and drops `ModuleA.swiftmodule` where B's
compile (and the editor) can find it.

Unlike the compiler-args fixtures (sources live in the gitignored `corpus/`, only
`raw/` is committed), this commits the **whole project including sources**, so the
BSP harness is hermetic: it builds the fixture with a throwaway `-derivedDataPath`
and type-checks our generated args against it without needing the corpus
materialized.

Two static-library targets keep it minimal (a module is a module to SourceKit;
no framework bundle / signing). Expand later to framework + app targets if a real
multi-target-kind case is needed.

Output (committed):
  fixtures/_synthetic-multimodule/project/MultiModule.xcodeproj
  fixtures/_synthetic-multimodule/project/{ModuleA,ModuleB}/*.swift

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

SLUG = "_synthetic-multimodule"

A_SWIFT = """public struct Greeter {
    public init() {}
    public func greet() -> String { "hello from ModuleA" }
}
"""

# ModuleB imports ModuleA — the cross-module edge the BSP harness measures.
B_SWIFT = """import ModuleA

public func greetViaA() -> String {
    Greeter().greet()
}
"""

PROJECT_SETTINGS = {
    "ALWAYS_SEARCH_USER_PATHS": "NO",
    "MACOSX_DEPLOYMENT_TARGET": "12.0",
    "ONLY_ACTIVE_ARCH": "YES",
    "SDKROOT": "macosx",
    "SWIFT_OPTIMIZATION_LEVEL": '"-Onone"',
    "SWIFT_VERSION": "5.0",
    # B must find A's generated module in the products dir.
    "SWIFT_INCLUDE_PATHS": '"$(BUILT_PRODUCTS_DIR)"',
}


def _uuid(seed: str) -> str:
    return uuid.uuid5(uuid.NAMESPACE_URL, f"sweetpad/multimodule/{seed}").hex[:24].upper()


def render_pbxproj(o: dict[str, str]) -> str:
    settings = "".join(f"                    {k} = {v};\n" for k, v in PROJECT_SETTINGS.items())

    def config_block(prefix: str, name: str) -> str:
        return textwrap.dedent(f"""\
            {o[f"cfg_{prefix}_{name}"]} = {{
                isa = XCBuildConfiguration;
                buildSettings = {{
                    PRODUCT_NAME = "$(TARGET_NAME)";
                }};
                name = {name};
            }};
        """)

    # Project-level config carries the shared settings; each target config just
    # names the product.
    proj_cfg_objs = ""
    for name in ("Debug", "Release"):
        proj_cfg_objs += textwrap.dedent(f"""\
            {o[f"cfg_proj_{name}"]} = {{
                isa = XCBuildConfiguration;
                buildSettings = {{
{settings}                }};
                name = {name};
            }};
        """)
    target_cfg_objs = "".join(
        config_block(t, name) for t in ("a", "b") for name in ("Debug", "Release")
    )
    config_objects = textwrap.indent(proj_cfg_objs + target_cfg_objs, " " * 16)

    def cfg_list(ids: list[str]) -> str:
        return "\n".join(f"                    {i}," for i in ids)

    return textwrap.dedent(f"""\
        // !$*UTF8*$!
        {{
            archiveVersion = 1;
            classes = {{}};
            objectVersion = 60;
            objects = {{
                {o["a_build"]} = {{isa = PBXBuildFile; fileRef = {o["a_ref"]}; }};
                {o["b_build"]} = {{isa = PBXBuildFile; fileRef = {o["b_ref"]}; }};
                {o["a_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = a.swift; sourceTree = "<group>"; }};
                {o["b_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = b.swift; sourceTree = "<group>"; }};
                {o["a_product"]} = {{isa = PBXFileReference; explicitFileType = archive.ar; includeInIndex = 0; path = libModuleA.a; sourceTree = BUILT_PRODUCTS_DIR; }};
                {o["b_product"]} = {{isa = PBXFileReference; explicitFileType = archive.ar; includeInIndex = 0; path = libModuleB.a; sourceTree = BUILT_PRODUCTS_DIR; }};
                {o["group_a"]} = {{ isa = PBXGroup; children = ({o["a_ref"]}); path = ModuleA; sourceTree = "<group>"; }};
                {o["group_b"]} = {{ isa = PBXGroup; children = ({o["b_ref"]}); path = ModuleB; sourceTree = "<group>"; }};
                {o["main_group"]} = {{
                    isa = PBXGroup;
                    children = ({o["group_a"]}, {o["group_b"]}, {o["products_group"]});
                    sourceTree = "<group>";
                }};
                {o["products_group"]} = {{
                    isa = PBXGroup;
                    children = ({o["a_product"]}, {o["b_product"]});
                    name = Products;
                    sourceTree = "<group>";
                }};
                {o["a_target"]} = {{
                    isa = PBXNativeTarget;
                    buildConfigurationList = {o["bcl_a"]};
                    buildPhases = ({o["a_sources"]});
                    buildRules = ();
                    dependencies = ();
                    name = ModuleA;
                    productName = ModuleA;
                    productReference = {o["a_product"]};
                    productType = "com.apple.product-type.library.static";
                }};
                {o["b_target"]} = {{
                    isa = PBXNativeTarget;
                    buildConfigurationList = {o["bcl_b"]};
                    buildPhases = ({o["b_sources"]});
                    buildRules = ();
                    dependencies = ({o["b_dep"]});
                    name = ModuleB;
                    productName = ModuleB;
                    productReference = {o["b_product"]};
                    productType = "com.apple.product-type.library.static";
                }};
                {o["b_dep"]} = {{
                    isa = PBXTargetDependency;
                    target = {o["a_target"]};
                    targetProxy = {o["b_dep_proxy"]};
                }};
                {o["b_dep_proxy"]} = {{
                    isa = PBXContainerItemProxy;
                    containerPortal = {o["project"]};
                    proxyType = 1;
                    remoteGlobalIDString = {o["a_target"]};
                    remoteInfo = ModuleA;
                }};
                {o["a_sources"]} = {{
                    isa = PBXSourcesBuildPhase;
                    buildActionMask = 2147483647;
                    files = ({o["a_build"]});
                    runOnlyForDeploymentPostprocessing = 0;
                }};
                {o["b_sources"]} = {{
                    isa = PBXSourcesBuildPhase;
                    buildActionMask = 2147483647;
                    files = ({o["b_build"]});
                    runOnlyForDeploymentPostprocessing = 0;
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
                    targets = ({o["a_target"]}, {o["b_target"]});
                }};
{config_objects}                {o["bcl_proj"]} = {{
                    isa = XCConfigurationList;
                    buildConfigurations = (
{cfg_list([o["cfg_proj_Debug"], o["cfg_proj_Release"]])}
                    );
                    defaultConfigurationIsVisible = 0;
                    defaultConfigurationName = Release;
                }};
                {o["bcl_a"]} = {{
                    isa = XCConfigurationList;
                    buildConfigurations = (
{cfg_list([o["cfg_a_Debug"], o["cfg_a_Release"]])}
                    );
                    defaultConfigurationIsVisible = 0;
                    defaultConfigurationName = Release;
                }};
                {o["bcl_b"]} = {{
                    isa = XCConfigurationList;
                    buildConfigurations = (
{cfg_list([o["cfg_b_Debug"], o["cfg_b_Release"]])}
                    );
                    defaultConfigurationIsVisible = 0;
                    defaultConfigurationName = Release;
                }};
            }};
            rootObject = {o["project"]};
        }}
        """)


def render_scheme(o: dict[str, str]) -> str:
    # Builds ModuleB; buildImplicitDependencies builds ModuleA first.
    def entry(target_id: str, lib: str, name: str) -> str:
        return textwrap.dedent(f"""\
            <BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES">
               <BuildableReference
                  BuildableIdentifier="primary"
                  BlueprintIdentifier="{target_id}"
                  BuildableName="{lib}"
                  BlueprintName="{name}"
                  ReferencedContainer="container:MultiModule.xcodeproj">
               </BuildableReference>
            </BuildActionEntry>
        """)

    entries = textwrap.indent(
        entry(o["a_target"], "libModuleA.a", "ModuleA") + entry(o["b_target"], "libModuleB.a", "ModuleB"),
        " " * 9,
    )
    return textwrap.dedent(f"""\
        <?xml version="1.0" encoding="UTF-8"?>
        <Scheme LastUpgradeVersion="1500" version="1.7">
           <BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES">
              <BuildActionEntries>
{entries}         </BuildActionEntries>
           </BuildAction>
        </Scheme>
        """)


def materialize(root: Path) -> Path:
    keys = [
        "project", "main_group", "products_group", "group_a", "group_b",
        "a_target", "b_target", "bcl_proj", "bcl_a", "bcl_b",
        "a_ref", "a_build", "a_product", "a_sources",
        "b_ref", "b_build", "b_product", "b_sources",
        "b_dep", "b_dep_proxy",
    ]
    o = {k: _uuid(k) for k in keys}
    for t in ("proj", "a", "b"):
        for name in ("Debug", "Release"):
            o[f"cfg_{t}_{name}"] = _uuid(f"cfg_{t}_{name}")

    (root / "ModuleA").mkdir(parents=True, exist_ok=True)
    (root / "ModuleB").mkdir(parents=True, exist_ok=True)
    (root / "ModuleA" / "a.swift").write_text(A_SWIFT)
    (root / "ModuleB" / "b.swift").write_text(B_SWIFT)

    xcodeproj = root / "MultiModule.xcodeproj"
    xcodeproj.mkdir(parents=True, exist_ok=True)
    (xcodeproj / "project.pbxproj").write_text(render_pbxproj(o))
    schemes = xcodeproj / "xcshareddata" / "xcschemes"
    schemes.mkdir(parents=True, exist_ok=True)
    (schemes / "ModuleB.xcscheme").write_text(render_scheme(o))
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
