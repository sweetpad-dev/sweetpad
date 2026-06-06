#!/usr/bin/env python3
"""Rich-settings synthetic fixture for the compiler-args oracle.

The corpus projects (Alamofire, Kingfisher, the sample apps) all build with
near-default settings, so most xcspec option encodings — warnings toggled on,
sanitizers, strict concurrency, exceptions — never fire and go unvalidated. This
generates a tiny static library whose build settings deliberately turn many of
those on, then captures the real per-tool commands via
`16_capture_compiler_args.py`.

It is the positive complement to the `Condition` gating: with
`CLANG_UNDEFINED_BEHAVIOR_SANITIZER = YES`, the `_INTEGER` / `_NULLABILITY`
sub-settings (which resolve `YES` corpus-wide but are gated off) now legitimately
emit `-fsanitize=integer` / `-fsanitize=nullability`, so the oracle confirms the
gate lets them through rather than only that it suppresses them.

Sources are Swift + a single ObjC++ (`.mm`) file: one clang language keeps the
comparison apples-to-apples (our shared per-target argv vs the oracle's common
clang arguments) so the score reflects the setting encodings, not the
union-vs-intersection noise a mixed-language target would inject. All sources are
warning-clean so the toggled-on warnings don't fail the build.

Output (committed):
  fixtures/_synthetic-rich/xcode-<ver>/raw/Scratch.xcodeproj   project to resolve
  fixtures/_synthetic-rich/xcode-<ver>/compiler-args/Scratch__Debug__macOS.json

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

SLUG = "_synthetic-rich"
TARGET = "Scratch"
SCHEME = "Scratch"
CONFIG = "Debug"

MAIN_SWIFT = """import Foundation

public func scratchGreeting() -> String { "scratch" }
"""

HELPER_SWIFT = """public func scratchAnswer() -> Int { 42 }
"""

# An ObjC++ source (`sourcecode.cpp.objcpp`) — exercises the C++/ObjC++ gates.
UTIL_MM = """#import <Foundation/Foundation.h>
#include <string>

NSString *scratchDescribe(int n) {
    std::string s = std::to_string(n);
    return [NSString stringWithUTF8String:s.c_str()];
}
"""

# Project-level settings: a broad spread of normally-off toggles so their xcspec
# option encodings are validated against a real build. UBSan is on (with its
# integer/nullability sub-checks) to confirm the `Condition` gate lets gated
# flags through when the parent gate holds. Warnings stay un-escalated
# (`GCC_TREAT_WARNINGS_AS_ERRORS = NO`) so a stray diagnostic can't fail the
# build. `PRODUCT_NAME` is set per target (a `$(...)` value must be quoted).
PROJECT_SETTINGS = {
    "ALWAYS_SEARCH_USER_PATHS": "NO",
    "MACOSX_DEPLOYMENT_TARGET": "12.0",
    "ONLY_ACTIVE_ARCH": "YES",
    "SDKROOT": "macosx",
    "SWIFT_OPTIMIZATION_LEVEL": '"-Onone"',
    "SWIFT_VERSION": "5.0",
    # Sanitizer: parent on + the two gated sub-checks on.
    "CLANG_UNDEFINED_BEHAVIOR_SANITIZER": "YES",
    "CLANG_UNDEFINED_BEHAVIOR_SANITIZER_INTEGER": "YES",
    "CLANG_UNDEFINED_BEHAVIOR_SANITIZER_NULLABILITY": "YES",
    # Codegen / language toggles with command-line encodings.
    "GCC_ENABLE_EXCEPTIONS": "YES",
    "GCC_SYMBOLS_PRIVATE_EXTERN": "YES",
    "GCC_TREAT_WARNINGS_AS_ERRORS": "NO",
    # Warning toggles (Boolean ByValue maps).
    "GCC_WARN_SHADOW": "YES",
    "GCC_WARN_64_TO_32_BIT_CONVERSION": "YES",
    "GCC_WARN_ABOUT_MISSING_NEWLINE": "YES",
    "CLANG_WARN_DOCUMENTATION_COMMENTS": "YES",
    "CLANG_WARN_ASSIGN_ENUM": "YES",
    "CLANG_WARN_UNGUARDED_AVAILABILITY": "YES_AGGRESSIVE",
    # Swift feature toggles.
    "SWIFT_STRICT_CONCURRENCY": "complete",
}


def _uuid(seed: str) -> str:
    return uuid.uuid5(uuid.NAMESPACE_URL, f"sweetpad/rich/{seed}").hex[:24].upper()


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
                {obj["mm_build"]} = {{isa = PBXBuildFile; fileRef = {obj["mm_ref"]}; }};
                {obj["swift_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = main.swift; sourceTree = "<group>"; }};
                {obj["swift2_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = helper.swift; sourceTree = "<group>"; }};
                {obj["mm_ref"]} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.cpp.objcpp; path = util.mm; sourceTree = "<group>"; }};
                {obj["product_ref"]} = {{isa = PBXFileReference; explicitFileType = archive.ar; includeInIndex = 0; path = libScratch.a; sourceTree = BUILT_PRODUCTS_DIR; }};
                {obj["main_group"]} = {{
                    isa = PBXGroup;
                    children = (
                        {obj["swift_ref"]},
                        {obj["swift2_ref"]},
                        {obj["mm_ref"]},
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
                        {obj["mm_build"]},
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
    """Create the rich-settings project + shared scheme under <root>."""
    root.mkdir(parents=True, exist_ok=True)
    (root / "main.swift").write_text(MAIN_SWIFT)
    (root / "helper.swift").write_text(HELPER_SWIFT)
    (root / "util.mm").write_text(UTIL_MM)
    keys = [
        "project", "main_group", "products_group", "target", "bcl_proj", "bcl_target",
        "swift_ref", "swift_build", "swift2_ref", "swift2_build",
        "mm_ref", "mm_build", "product_ref", "sources_phase",
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
