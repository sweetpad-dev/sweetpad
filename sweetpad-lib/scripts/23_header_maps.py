#!/usr/bin/env python3
"""Synthetic header-map fixture for the BSP harness (see DOCS.md §8 (BSP server)).

`_synthetic-objc-headers` covers the one ObjC import that resolves without any
header map: a header in a directory `HEADER_SEARCH_PATHS` names. Real projects
mostly don't look like that, and the imports they do use — a sibling directory's
header, the `-Swift.h` a mixed target generates, another target's public header —
resolve through Xcode's header maps and generated-sources dirs. This fixture is
those three imports and nothing else, with no `HEADER_SEARCH_PATHS` anywhere, so
the editor arguments have to carry the real search paths or `Widget.m` fails to
parse (GitHub #238).

Three targets:
  HeaderMapsCore    a framework installing `CoreThing.h` as a public header
  HeaderMaps        a mixed ObjC/Swift static library whose `Widget.m` imports
                    `"DeepThing.h"` (a sibling dir), `"HeaderMaps-Swift.h"`
                    (generated) and `<HeaderMapsCore/CoreThing.h>` (the framework)
  HeaderMapsOrphan  the same cross-directory import from a target **no scheme
                    builds** — the shape `buildTarget/prepare` has to reach with
                    a `-target` build, since there is no scheme to name

Output (committed):
  fixtures/_synthetic-headermaps/project/HeaderMaps.xcodeproj
  fixtures/_synthetic-headermaps/project/{Core,Deep,Top}/…

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

SLUG = "_synthetic-headermaps"

SOURCES = {
    "Core/CoreThing.h": """\
#import <Foundation/Foundation.h>

@interface CoreThing : NSObject
+ (NSString *)coreName;
@end
""",
    "Core/CoreThing.m": """\
#import "CoreThing.h"

@implementation CoreThing
+ (NSString *)coreName { return @"core"; }
@end
""",
    "Deep/DeepThing.h": """\
#import <Foundation/Foundation.h>

@interface DeepThing : NSObject
+ (NSString *)deepName;
@end
""",
    "Deep/DeepThing.m": """\
#import "DeepThing.h"

@implementation DeepThing
+ (NSString *)deepName { return @"deep"; }
@end
""",
    "Top/Greeter.swift": """\
import Foundation

@objc public class Greeter: NSObject {
    @objc public func greeting() -> String { "hello" }
}
""",
    "Orphan/OrphanThing.m": """\
// This target is in no scheme, so only a `-target` build prepares it.
#import "DeepThing.h"

@interface OrphanThing : NSObject
@end

@implementation OrphanThing
+ (NSString *)name { return [DeepThing deepName]; }
@end
""",
    "Top/Widget.h": """\
#import <Foundation/Foundation.h>

@interface Widget : NSObject
+ (NSString *)describe;
@end
""",
    "Top/Widget.m": """\
#import "Widget.h"
// A sibling directory's header, named by no HEADER_SEARCH_PATHS entry: only the
// project header map resolves it.
#import "DeepThing.h"
// This target's Swift half, which exists only once a build has generated it.
#import "HeaderMaps-Swift.h"
// Another target's public header, reached through the framework it installs.
#import <HeaderMapsCore/CoreThing.h>

@implementation Widget
+ (NSString *)describe {
  return [NSString stringWithFormat:@"%@ %@ %@", [DeepThing deepName],
                                    [CoreThing coreName],
                                    [[Greeter new] greeting]];
}
@end
""",
}

# No HEADER_SEARCH_PATHS: every import in Widget.m has to resolve some other way.
PROJECT_SETTINGS = {
    "ALWAYS_SEARCH_USER_PATHS": "NO",
    "MACOSX_DEPLOYMENT_TARGET": "12.0",
    "ONLY_ACTIVE_ARCH": "YES",
    "SDKROOT": "macosx",
}

LIB_SETTINGS = {
    "PRODUCT_NAME": "$(TARGET_NAME)",
    "SWIFT_VERSION": "5.0",
    # Named rather than defaulted so the generated header's path is the same
    # whatever a product type's spec says it should be.
    "SWIFT_OBJC_INTERFACE_HEADER_NAME": "HeaderMaps-Swift.h",
}

ORPHAN_SETTINGS = {
    "PRODUCT_NAME": "$(TARGET_NAME)",
}

FRAMEWORK_SETTINGS = {
    "PRODUCT_NAME": "$(TARGET_NAME)",
    "GENERATE_INFOPLIST_FILE": "YES",
}


def _uuid(seed: str) -> str:
    return uuid.uuid5(uuid.NAMESPACE_URL, f"sweetpad/headermaps/{seed}").hex[:24].upper()


def _quote(value: str) -> str:
    """pbxproj leaves bare identifiers unquoted and quotes everything else — a
    bare `$(TARGET_NAME)` is a parse error, not a variable reference."""
    if value and all(c.isalnum() or c in "_." for c in value):
        return value
    return f'"{value}"'


def _config_list(name: str, settings: dict[str, str]) -> str:
    """The two XCBuildConfigurations for `name` plus the list that holds them."""
    body = ""
    rendered = "".join(f"                {k} = {_quote(v)};\n" for k, v in settings.items())
    for cfg in ("Debug", "Release"):
        body += (
            f'        {_uuid(f"cfg_{name}_{cfg}")} = {{\n'
            f"            isa = XCBuildConfiguration;\n"
            f"            buildSettings = {{\n"
            f"{rendered}"
            f"            }};\n"
            f"            name = {cfg};\n"
            f"        }};\n"
        )
    entries = "\n".join(
        f'                {_uuid(f"cfg_{name}_{c}")},' for c in ("Debug", "Release")
    )
    body += (
        f'        {_uuid(f"bcl_{name}")} = {{\n'
        f"            isa = XCConfigurationList;\n"
        f"            buildConfigurations = (\n"
        f"{entries}\n"
        f"            );\n"
        f"            defaultConfigurationIsVisible = 0;\n"
        f"            defaultConfigurationName = Release;\n"
        f"        }};\n"
    )
    return body


def render_pbxproj() -> str:
    u = _uuid
    refs = {path: u(f"ref_{path}") for path in SOURCES}
    builds = {path: u(f"build_{path}") for path in SOURCES}

    objects = ""
    for path in SOURCES:
        kind = {
            ".h": "sourcecode.c.h",
            ".m": "sourcecode.c.objc",
            ".swift": "sourcecode.swift",
        }[Path(path).suffix]
        objects += (
            f'        {refs[path]} = {{isa = PBXFileReference; '
            f"lastKnownFileType = {kind}; path = {Path(path).name}; "
            f'sourceTree = "<group>"; }};\n'
        )
    # The framework's public header is the only one in a Headers build phase;
    # that attribute is what installs it into HeaderMapsCore.framework/Headers.
    objects += (
        f'        {builds["Core/CoreThing.h"]} = {{isa = PBXBuildFile; '
        f'fileRef = {refs["Core/CoreThing.h"]}; settings = {{ATTRIBUTES = (Public, ); }}; }};\n'
    )
    for path in SOURCES:
        if Path(path).suffix in (".m", ".swift"):
            objects += (
                f"        {builds[path]} = {{isa = PBXBuildFile; "
                f"fileRef = {refs[path]}; }};\n"
            )

    groups = {"Core": ["Core/CoreThing.h", "Core/CoreThing.m"],
              "Deep": ["Deep/DeepThing.h", "Deep/DeepThing.m"],
              "Orphan": ["Orphan/OrphanThing.m"],
              "Top": ["Top/Greeter.swift", "Top/Widget.h", "Top/Widget.m"]}
    for name, members in groups.items():
        children = ", ".join(refs[m] for m in members)
        objects += (
            f'        {u(f"group_{name}")} = {{ isa = PBXGroup; children = ({children}); '
            f'path = {name}; sourceTree = "<group>"; }};\n'
        )

    objects += (
        f'        {u("product_lib")} = {{isa = PBXFileReference; explicitFileType = archive.ar; '
        f"includeInIndex = 0; path = libHeaderMaps.a; sourceTree = BUILT_PRODUCTS_DIR; }};\n"
        f'        {u("product_fw")} = {{isa = PBXFileReference; explicitFileType = wrapper.framework; '
        f"includeInIndex = 0; path = HeaderMapsCore.framework; sourceTree = BUILT_PRODUCTS_DIR; }};\n"
        f'        {u("product_orphan")} = {{isa = PBXFileReference; explicitFileType = archive.ar; '
        f"includeInIndex = 0; path = libHeaderMapsOrphan.a; sourceTree = BUILT_PRODUCTS_DIR; }};\n"
        f'        {u("products_group")} = {{ isa = PBXGroup; '
        f'children = ({u("product_fw")}, {u("product_lib")}, {u("product_orphan")}); name = Products; sourceTree = "<group>"; }};\n'
        f'        {u("main_group")} = {{ isa = PBXGroup; children = ({u("group_Core")}, '
        f'{u("group_Deep")}, {u("group_Orphan")}, {u("group_Top")}, {u("products_group")}); sourceTree = "<group>"; }};\n'
    )

    lib_sources = ", ".join(
        builds[p] for p in ("Top/Widget.m", "Top/Greeter.swift", "Deep/DeepThing.m")
    )
    objects += textwrap.indent(textwrap.dedent(f"""\
        {u("phase_fw_sources")} = {{
            isa = PBXSourcesBuildPhase;
            buildActionMask = 2147483647;
            files = ({builds["Core/CoreThing.m"]});
            runOnlyForDeploymentPostprocessing = 0;
        }};
        {u("phase_fw_headers")} = {{
            isa = PBXHeadersBuildPhase;
            buildActionMask = 2147483647;
            files = ({builds["Core/CoreThing.h"]});
            runOnlyForDeploymentPostprocessing = 0;
        }};
        {u("phase_lib_sources")} = {{
            isa = PBXSourcesBuildPhase;
            buildActionMask = 2147483647;
            files = ({lib_sources});
            runOnlyForDeploymentPostprocessing = 0;
        }};
        {u("target_fw")} = {{
            isa = PBXNativeTarget;
            buildConfigurationList = {u("bcl_fw")};
            buildPhases = ({u("phase_fw_headers")}, {u("phase_fw_sources")});
            buildRules = ();
            dependencies = ();
            name = HeaderMapsCore;
            productName = HeaderMapsCore;
            productReference = {u("product_fw")};
            productType = "com.apple.product-type.framework";
        }};
        {u("target_lib")} = {{
            isa = PBXNativeTarget;
            buildConfigurationList = {u("bcl_lib")};
            buildPhases = ({u("phase_lib_sources")});
            buildRules = ();
            dependencies = ({u("dep_lib_on_fw")});
            name = HeaderMaps;
            productName = HeaderMaps;
            productReference = {u("product_lib")};
            productType = "com.apple.product-type.library.static";
        }};
        {u("phase_orphan_sources")} = {{
            isa = PBXSourcesBuildPhase;
            buildActionMask = 2147483647;
            files = ({builds["Orphan/OrphanThing.m"]});
            runOnlyForDeploymentPostprocessing = 0;
        }};
        {u("target_orphan")} = {{
            isa = PBXNativeTarget;
            buildConfigurationList = {u("bcl_orphan")};
            buildPhases = ({u("phase_orphan_sources")});
            buildRules = ();
            dependencies = ();
            name = HeaderMapsOrphan;
            productName = HeaderMapsOrphan;
            productReference = {u("product_orphan")};
            productType = "com.apple.product-type.library.static";
        }};
        {u("proxy_fw")} = {{
            isa = PBXContainerItemProxy;
            containerPortal = {u("project")};
            proxyType = 1;
            remoteGlobalIDString = {u("target_fw")};
            remoteInfo = HeaderMapsCore;
        }};
        {u("dep_lib_on_fw")} = {{
            isa = PBXTargetDependency;
            target = {u("target_fw")};
            targetProxy = {u("proxy_fw")};
        }};
        {u("project")} = {{
            isa = PBXProject;
            attributes = {{ LastUpgradeCheck = 1500; }};
            buildConfigurationList = {u("bcl_proj")};
            compatibilityVersion = "Xcode 14.0";
            developmentRegion = en;
            hasScannedForEncodings = 0;
            knownRegions = (en, Base);
            mainGroup = {u("main_group")};
            productRefGroup = {u("products_group")};
            projectDirPath = "";
            projectRoot = "";
            targets = ({u("target_fw")}, {u("target_lib")}, {u("target_orphan")});
        }};
    """), " " * 8)

    for name, settings in (
        ("proj", PROJECT_SETTINGS),
        ("fw", FRAMEWORK_SETTINGS),
        ("lib", LIB_SETTINGS),
        ("orphan", ORPHAN_SETTINGS),
    ):
        objects += _config_list(name, settings)

    return (
        "// !$*UTF8*$!\n{\n"
        "    archiveVersion = 1;\n"
        "    classes = {};\n"
        "    objectVersion = 60;\n"
        "    objects = {\n"
        f"{objects}"
        "    };\n"
        f'    rootObject = {u("project")};\n'
        "}\n"
    )


def render_scheme() -> str:
    """Both targets, built in dependency order — the library's PBXTargetDependency
    on the framework is what puts `CoreThing.h` in the products dir before
    `Widget.m` imports it."""
    entries = ""
    for target, key, product in (
        ("HeaderMapsCore", "target_fw", "HeaderMapsCore.framework"),
        ("HeaderMaps", "target_lib", "libHeaderMaps.a"),
    ):
        entries += (
            '         <BuildActionEntry buildForTesting="YES" buildForRunning="YES"'
            ' buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES">\n'
            "            <BuildableReference\n"
            '               BuildableIdentifier="primary"\n'
            f'               BlueprintIdentifier="{_uuid(key)}"\n'
            f'               BuildableName="{product}"\n'
            f'               BlueprintName="{target}"\n'
            '               ReferencedContainer="container:HeaderMaps.xcodeproj">\n'
            "            </BuildableReference>\n"
            "         </BuildActionEntry>\n"
        )
    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<Scheme LastUpgradeVersion="1500" version="1.7">\n'
        '   <BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES">\n'
        "      <BuildActionEntries>\n"
        f"{entries}"
        "      </BuildActionEntries>\n"
        "   </BuildAction>\n"
        "</Scheme>\n"
    )


def materialize(root: Path) -> Path:
    for path, body in SOURCES.items():
        target = root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(body)
    xcodeproj = root / "HeaderMaps.xcodeproj"
    xcodeproj.mkdir(parents=True, exist_ok=True)
    (xcodeproj / "project.pbxproj").write_text(render_pbxproj())
    schemes = xcodeproj / "xcshareddata" / "xcschemes"
    schemes.mkdir(parents=True, exist_ok=True)
    (schemes / "HeaderMaps.xcscheme").write_text(render_scheme())
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
