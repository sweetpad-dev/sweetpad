#!/usr/bin/env python3
"""Audit which Xcode-feature probes are covered by the captured corpus.

Indexes captured build-settings JSON, xcconfig files, xcscheme XML, and
raw/-tree file presence, then runs a fixed set of probes against the index.
Each probe answers a single question from `coverage.md`. The output is a
markdown table per category that's safe to paste into `coverage.md`.

Inputs (read-only):
  - fixtures/<slug>/xcode-<ver>/metadata/**/build-settings/*.json
  - fixtures/<slug>/xcode-<ver>/raw/**

Outputs:
  - fixtures/AUDIT.md       human-readable per-probe results
  - fixtures/AUDIT.json     machine-readable probe results

Not destructive. Adds nothing to git. Safe to re-run.

Flags:
  --xcode <ver>     only audit one Xcode (default: every xcode-* dir found)
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import os
import re
import sys
from pathlib import Path
from typing import Callable, Iterable

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


@dataclasses.dataclass
class FixtureIndex:
    slug: str
    xcode_version: str
    root: Path
    # Indexed data:
    settings_keys_by_target: dict[tuple[str, str], dict[str, str]] = dataclasses.field(default_factory=dict)
    settings_paths: list[Path] = dataclasses.field(default_factory=list)
    xcconfig_contents: dict[Path, str] = dataclasses.field(default_factory=dict)
    scheme_contents: dict[Path, str] = dataclasses.field(default_factory=dict)
    pbxproj_contents: dict[Path, str] = dataclasses.field(default_factory=dict)
    entitlements_contents: dict[Path, str] = dataclasses.field(default_factory=dict)
    raw_files_by_suffix: dict[str, list[Path]] = dataclasses.field(default_factory=dict)
    raw_files_by_basename: dict[str, list[Path]] = dataclasses.field(default_factory=dict)
    # Corpus tree (the original cloned project) — broader file-presence probes
    corpus_files_by_suffix: dict[str, list[Path]] = dataclasses.field(default_factory=dict)
    corpus_files_by_basename: dict[str, list[Path]] = dataclasses.field(default_factory=dict)
    # Apple-style bundle directories (.xcdatamodeld, .xcassets, .framework, ...) —
    # they are directories, not files. Track separately so probes can target them.
    corpus_dirs_by_suffix: dict[str, list[Path]] = dataclasses.field(default_factory=dict)


def index_fixture(slug: str, xcode_dir: Path) -> FixtureIndex:
    idx = FixtureIndex(slug=slug, xcode_version=xcode_dir.name.removeprefix("xcode-"), root=xcode_dir)

    # Build settings JSON
    for bs in xcode_dir.glob("metadata/**/build-settings/*.json"):
        if bs.stat().st_size < 10:
            continue
        idx.settings_paths.append(bs)
        try:
            data = json.loads(bs.read_text())
        except Exception:
            continue
        for entry in data:
            target = entry.get("target", "")
            settings = entry.get("buildSettings", {})
            key = (target, str(bs.relative_to(xcode_dir)))
            idx.settings_keys_by_target[key] = settings

    # raw/ tree plus auxiliary content dirs (xcconfigs/ for synthetic fixtures,
    # pif/ for PIF cache copies, etc.).
    EXTRA_CONTENT_DIRS = ("raw", "xcconfigs", "pif")
    for top in EXTRA_CONTENT_DIRS:
        d = xcode_dir / top
        if not d.exists():
            continue
        for dirpath, _, filenames in os.walk(d):
            for fn in filenames:
                p = Path(dirpath) / fn
                suffix = p.suffix
                idx.raw_files_by_suffix.setdefault(suffix, []).append(p)
                idx.raw_files_by_basename.setdefault(fn, []).append(p)
                try:
                    if fn.endswith(".xcconfig"):
                        idx.xcconfig_contents[p] = p.read_text(errors="replace")
                    elif fn.endswith(".xcscheme"):
                        idx.scheme_contents[p] = p.read_text(errors="replace")
                    elif fn.endswith(".pbxproj"):
                        idx.pbxproj_contents[p] = p.read_text(errors="replace")
                    elif fn.endswith(".entitlements"):
                        idx.entitlements_contents[p] = p.read_text(errors="replace")
                except Exception:
                    pass

    # Walk the cloned corpus tree too — raw/ is a narrow copy, but for
    # file-presence checks like "does this project have an .xcdatamodel?",
    # walking corpus/<slug>/ gives the full picture.
    corpus_root = common.CORPUS_DIR / slug
    if corpus_root.exists():
        SKIP_DIRS = {".git", ".svn", "DerivedData", ".derived", "build",
                     ".build", "node_modules", "Pods", "Carthage",
                     ".swiftpm", "tmp", ".tuist-cache",
                     "examples"  # for tuist-fixtures, the fixtures themselves
                                 # are nested under examples/xcode/; their
                                 # individual project trees are walked via
                                 # other slugs (or, here, included if you want
                                 # — we keep them in).
                     }
        # For tuist-fixtures we DO want examples/xcode walked.
        if slug == "tuist-fixtures":
            SKIP_DIRS.discard("examples")
        for dirpath, dirnames, filenames in os.walk(corpus_root, followlinks=False):
            dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
            for dn in dirnames:
                p = Path(dirpath) / dn
                suffix = p.suffix
                if suffix:
                    idx.corpus_dirs_by_suffix.setdefault(suffix, []).append(p)
            for fn in filenames:
                p = Path(dirpath) / fn
                suffix = p.suffix
                idx.corpus_files_by_suffix.setdefault(suffix, []).append(p)
                idx.corpus_files_by_basename.setdefault(fn, []).append(p)
    return idx


def all_setting_values(idx: FixtureIndex, key: str) -> list[str]:
    out = []
    for settings in idx.settings_keys_by_target.values():
        if key in settings:
            out.append(settings[key])
    return out


def any_setting_matches(idx: FixtureIndex, key: str,
                         pred: Callable[[str], bool] = lambda v: bool(v)) -> bool:
    return any(pred(v) for v in all_setting_values(idx, key))


def grep_xcconfigs(idx: FixtureIndex, pattern: str) -> list[Path]:
    p = re.compile(pattern)
    return [path for path, content in idx.xcconfig_contents.items() if p.search(content)]


def grep_pbxprojs(idx: FixtureIndex, pattern: str) -> list[Path]:
    p = re.compile(pattern)
    return [path for path, content in idx.pbxproj_contents.items() if p.search(content)]


def grep_schemes(idx: FixtureIndex, pattern: str) -> list[Path]:
    p = re.compile(pattern)
    return [path for path, content in idx.scheme_contents.items() if p.search(content)]


def has_suffix(idx: FixtureIndex, suffix: str) -> bool:
    return bool(idx.raw_files_by_suffix.get(suffix))


def has_basename(idx: FixtureIndex, name: str) -> bool:
    return bool(idx.raw_files_by_basename.get(name))


# --- Probes ----------------------------------------------------------------

@dataclasses.dataclass
class Probe:
    category: str
    name: str
    check: Callable[[FixtureIndex], tuple[bool, str]]


def probe_build_setting(key: str, pred: Callable[[str], bool] | None = None,
                         label: str | None = None) -> Probe:
    label = label or key
    def check(idx: FixtureIndex) -> tuple[bool, str]:
        values = all_setting_values(idx, key)
        if pred is not None:
            matches = [v for v in values if pred(v)]
        else:
            matches = [v for v in values if v]
        if not matches:
            return False, ""
        sample = matches[0][:80]
        return True, f"{key}={sample!r} ({len(matches)} hits)"
    return Probe("settings", label, check)


def probe_setting_key_present(key: str, label: str | None = None) -> Probe:
    label = label or key
    def check(idx: FixtureIndex) -> tuple[bool, str]:
        hits = sum(1 for s in idx.settings_keys_by_target.values() if key in s)
        return (hits > 0, f"present in {hits} target × config × dest combos")
    return Probe("settings", label, check)


def probe_xcconfig(pattern: str, label: str) -> Probe:
    def check(idx: FixtureIndex) -> tuple[bool, str]:
        hits = grep_xcconfigs(idx, pattern)
        if not hits:
            return False, ""
        sample = hits[0].relative_to(idx.root)
        return True, f"xcconfig matched: {sample} (+{len(hits)-1})"
    return Probe("xcconfig", label, check)


def probe_pbxproj(pattern: str, label: str) -> Probe:
    def check(idx: FixtureIndex) -> tuple[bool, str]:
        hits = grep_pbxprojs(idx, pattern)
        if not hits:
            return False, ""
        sample = hits[0].relative_to(idx.root)
        return True, f"pbxproj matched: {sample} (+{len(hits)-1})"
    return Probe("pbxproj", label, check)


def probe_scheme(pattern: str, label: str) -> Probe:
    def check(idx: FixtureIndex) -> tuple[bool, str]:
        hits = grep_schemes(idx, pattern)
        if not hits:
            return False, ""
        sample = hits[0].relative_to(idx.root)
        return True, f"xcscheme matched: {sample} (+{len(hits)-1})"
    return Probe("scheme", label, check)


def probe_raw_suffix(suffix: str, label: str) -> Probe:
    def check(idx: FixtureIndex) -> tuple[bool, str]:
        hits = idx.raw_files_by_suffix.get(suffix, [])
        if not hits:
            return False, ""
        return True, f"raw/ has {len(hits)} {suffix} files (e.g. {hits[0].name})"
    return Probe("files", label, check)


def probe_raw_basename(name: str, label: str) -> Probe:
    def check(idx: FixtureIndex) -> tuple[bool, str]:
        hits = idx.raw_files_by_basename.get(name, [])
        if not hits:
            return False, ""
        return True, f"raw/ has {len(hits)} {name} files"
    return Probe("files", label, check)


def probe_corpus_suffix(suffix: str, label: str) -> Probe:
    def check(idx: FixtureIndex) -> tuple[bool, str]:
        hits = idx.corpus_files_by_suffix.get(suffix, [])
        if not hits:
            return False, ""
        return True, f"corpus/ has {len(hits)} {suffix} files (e.g. {hits[0].name})"
    return Probe("files", label, check)


def probe_corpus_dir_suffix(suffix: str, label: str) -> Probe:
    """Apple-bundle directories (.xcdatamodeld, .xcassets, ...) are dirs, not files."""
    def check(idx: FixtureIndex) -> tuple[bool, str]:
        hits = idx.corpus_dirs_by_suffix.get(suffix, [])
        if not hits:
            return False, ""
        return True, f"corpus/ has {len(hits)} {suffix} bundle dirs (e.g. {hits[0].name})"
    return Probe("files", label, check)


def probe_corpus_basename(name: str, label: str) -> Probe:
    def check(idx: FixtureIndex) -> tuple[bool, str]:
        hits = idx.corpus_files_by_basename.get(name, [])
        if not hits:
            return False, ""
        return True, f"corpus/ has {len(hits)} {name} files"
    return Probe("files", label, check)


def probe_entitlement_key(key: str, label: str) -> Probe:
    pat = re.compile(rf"<key>{re.escape(key)}</key>")
    def check(idx: FixtureIndex) -> tuple[bool, str]:
        for path, content in idx.entitlements_contents.items():
            if pat.search(content):
                return True, f".entitlements matched: {path.name}"
        return False, ""
    return Probe("files", label, check)


PROBES: list[Probe] = [
    # Settings inheritance & substitution
    probe_setting_key_present("SRCROOT", "$(SRCROOT) resolved"),
    probe_setting_key_present("BUILT_PRODUCTS_DIR", "$(BUILT_PRODUCTS_DIR) resolved"),
    probe_setting_key_present("PROJECT_DIR", "$(PROJECT_DIR) resolved"),
    probe_setting_key_present("TARGET_NAME", "$(TARGET_NAME) resolved"),
    probe_setting_key_present("PRODUCT_NAME", "$(PRODUCT_NAME) resolved"),
    probe_setting_key_present("EFFECTIVE_PLATFORM_NAME", "$(EFFECTIVE_PLATFORM_NAME) resolved"),
    probe_xcconfig(r"\$\(inherited\)", "$(inherited) used in xcconfig"),
    probe_xcconfig(r"\$\([A-Z][A-Z0-9_]*\)[^/\s]*\$\(", "Recursive substitution in xcconfig (>=2 refs in one value)"),
    probe_xcconfig(r"\$\{[A-Z][A-Z0-9_]*:[a-z]+", "Modifier syntax ${VAR:lower}/${VAR:default=...} in xcconfig"),
    probe_xcconfig(r"\\\s*\n", "Multi-line continuation in xcconfig"),
    probe_xcconfig(r"\[sdk=", "Conditional [sdk=...] in xcconfig"),
    probe_xcconfig(r"\[arch=", "Conditional [arch=...] in xcconfig"),
    probe_xcconfig(r"\[config=", "Conditional [config=...] in xcconfig"),
    probe_pbxproj(r"\[sdk=", "Conditional [sdk=...] in pbxproj"),
    probe_pbxproj(r"\[arch=", "Conditional [arch=...] in pbxproj"),
    probe_pbxproj(r"baseConfigurationReference", ".xcconfig referenced from pbxproj"),
    probe_xcconfig(r"#include", "xcconfig #include directive"),

    # Configurations
    probe_build_setting("CONFIGURATION", lambda v: v not in ("Debug", "Release"),
                         label="Non-Debug/Release configuration"),

    # Architectures
    probe_setting_key_present("ARCHS_STANDARD", "ARCHS_STANDARD resolved"),
    probe_setting_key_present("VALID_ARCHS", "VALID_ARCHS present"),
    probe_build_setting("EXCLUDED_ARCHS", label="EXCLUDED_ARCHS set"),
    probe_build_setting("ARCHS", lambda v: "x86_64" in v, label="x86_64 architecture present"),
    probe_build_setting("ARCHS", lambda v: "arm64e" in v, label="arm64e architecture present"),

    # Linking
    probe_build_setting("MACH_O_TYPE", lambda v: v == "staticlib", label="Static library MACH_O_TYPE"),
    probe_build_setting("MACH_O_TYPE", lambda v: v == "mh_dylib", label="Dynamic library MACH_O_TYPE"),
    probe_build_setting("MACH_O_TYPE", lambda v: v == "mh_execute", label="Executable MACH_O_TYPE"),
    probe_build_setting("MACH_O_TYPE", lambda v: v == "mh_bundle", label="Bundle MACH_O_TYPE"),
    probe_setting_key_present("LD_RUNPATH_SEARCH_PATHS", "LD_RUNPATH_SEARCH_PATHS"),
    probe_setting_key_present("OTHER_LDFLAGS", "OTHER_LDFLAGS"),
    probe_build_setting("MERGEABLE_LIBRARY", lambda v: v == "YES", label="Mergeable libraries"),
    probe_build_setting("LLVM_LTO", lambda v: v not in ("", "NO"), label="Link-time optimization"),

    # Mac Catalyst
    probe_build_setting("SUPPORTS_MACCATALYST", lambda v: v == "YES", label="Mac Catalyst supported"),
    probe_build_setting("IS_MACCATALYST", lambda v: v == "YES", label="Built as Mac Catalyst"),
    probe_build_setting("SUPPORTS_MAC_DESIGNED_FOR_IPHONE_IPAD", lambda v: v == "YES",
                         label="Designed for iPad on Mac"),

    # SDKs / Platforms (case-insensitive — SDKROOT resolves to /.../iPhoneSimulator26.0.sdk)
    probe_build_setting("SDKROOT", lambda v: "iphonesimulator" in v.lower(), label="iphonesimulator SDK"),
    probe_build_setting("SDKROOT", lambda v: "macosx" in v.lower(), label="macosx SDK"),
    probe_build_setting("SDKROOT", lambda v: "watchsimulator" in v.lower(), label="watchsimulator SDK"),
    probe_build_setting("SDKROOT", lambda v: "appletvsimulator" in v.lower(), label="appletvsimulator SDK"),
    probe_build_setting("SDKROOT", lambda v: "xrsimulator" in v.lower(), label="xrsimulator SDK"),
    probe_build_setting("SDKROOT", lambda v: "driverkit" in v.lower(), label="driverkit SDK"),

    # Resources — probe the corpus tree (broader than raw/'s narrow allowlist)
    probe_setting_key_present("ASSETCATALOG_COMPILER_APPICON_NAME", "Asset catalog with AppIcon"),
    probe_corpus_suffix(".xcstrings", ".xcstrings file present"),
    probe_corpus_suffix(".strings", "Legacy .strings file present"),
    probe_corpus_suffix(".storyboard", ".storyboard file present"),
    probe_corpus_suffix(".xib", ".xib file present"),
    probe_corpus_basename("Contents.json", "Asset catalog Contents.json"),
    probe_corpus_dir_suffix(".xcdatamodeld", "Core Data .xcdatamodeld bundle"),
    probe_corpus_suffix(".mlmodel", "Core ML .mlmodel"),
    probe_corpus_suffix(".metal", "Metal .metal shader"),
    probe_corpus_suffix(".xcprivacy", "PrivacyInfo.xcprivacy"),
    probe_raw_suffix(".entitlements", ".entitlements file present"),
    probe_entitlement_key("com.apple.security.application-groups", "App Groups entitlement"),
    probe_entitlement_key("com.apple.developer.icloud-container-identifiers", "iCloud entitlement"),
    probe_entitlement_key("aps-environment", "Push notifications entitlement"),
    probe_entitlement_key("com.apple.developer.networking.wifi-info", "WiFi info entitlement"),

    # Info.plist + entitlements
    probe_build_setting("INFOPLIST_FILE", label="Info.plist explicitly listed"),
    probe_build_setting("GENERATE_INFOPLIST_FILE", lambda v: v == "YES",
                         label="Info.plist generated from build settings"),
    probe_build_setting("CODE_SIGN_ENTITLEMENTS", label="CODE_SIGN_ENTITLEMENTS set"),

    # Swift
    probe_setting_key_present("SWIFT_VERSION", "SWIFT_VERSION declared"),
    probe_build_setting("BUILD_LIBRARY_FOR_DISTRIBUTION", lambda v: v == "YES",
                         label="Library evolution (BUILD_LIBRARY_FOR_DISTRIBUTION=YES)"),
    probe_build_setting("SWIFT_STRICT_CONCURRENCY",
                         lambda v: v in ("complete", "targeted", "minimal"),
                         label="SWIFT_STRICT_CONCURRENCY explicit"),
    probe_build_setting("SWIFT_UPCOMING_FEATURE_STRICT_CONCURRENCY", lambda v: v == "YES",
                         label="Swift upcoming feature: strict concurrency"),
    probe_build_setting("OTHER_SWIFT_FLAGS", lambda v: v and v != "", label="OTHER_SWIFT_FLAGS non-empty"),
    probe_build_setting("SWIFT_OBJC_BRIDGING_HEADER", label="Obj-C bridging header"),

    # SwiftPM
    probe_corpus_basename("Package.swift", "Package.swift in corpus"),
    probe_corpus_basename("Package.resolved", "Package.resolved in corpus"),
    probe_pbxproj(r"XCRemoteSwiftPackageReference", "Remote SwiftPM dependency (pbxproj)"),
    probe_pbxproj(r"XCSwiftPackageProductDependency", "SwiftPM product dependency (pbxproj)"),

    # Build phases & dependencies
    probe_pbxproj(r"PBXShellScriptBuildPhase", "Run Script build phase"),
    probe_pbxproj(r"PBXHeadersBuildPhase", "Headers build phase (public/private)"),
    probe_pbxproj(r"PBXCopyFilesBuildPhase", "Copy Files build phase"),
    probe_pbxproj(r"PBXCopyFilesBuildPhase[^}]*Embed Frameworks", "Embed Frameworks copy phase"),
    probe_pbxproj(r"PBXTargetDependency", "PBXTargetDependency"),
    probe_pbxproj(r"PBXContainerItemProxy[^}]*containerPortal", "Cross-project container reference"),

    # ObjC / mixed
    probe_build_setting("DEFINES_MODULE", lambda v: v == "YES", label="DEFINES_MODULE=YES"),
    probe_build_setting("CLANG_ENABLE_OBJC_ARC", lambda v: v == "YES", label="ObjC ARC enabled"),
    probe_build_setting("CLANG_ENABLE_OBJC_WEAK", lambda v: v == "YES", label="ObjC weak refs enabled"),
    probe_corpus_suffix(".m", "Obj-C .m source present"),
    probe_corpus_suffix(".mm", "Obj-C++ .mm source present"),
    probe_corpus_suffix(".pch", "Pre-compiled header .pch"),

    # Schemes
    probe_scheme(r"<PreActions>\s*<", "Scheme pre-action defined"),
    probe_scheme(r"<PostActions>\s*<", "Scheme post-action defined"),
    probe_scheme(r"<EnvironmentVariables>\s*<EnvironmentVariable", "Scheme env vars"),
    probe_scheme(r"<CommandLineArguments>\s*<CommandLineArgument", "Scheme launch arguments"),
    probe_scheme(r"<TestPlans>\s*<TestPlanReference", "Scheme test plan reference"),
    probe_corpus_suffix(".xctestplan", ".xctestplan file present"),

    # Product types (extracted from PRODUCT_TYPE setting)
    probe_build_setting("PRODUCT_TYPE", lambda v: v == "com.apple.product-type.application",
                         label="Product: application"),
    probe_build_setting("PRODUCT_TYPE", lambda v: v == "com.apple.product-type.framework",
                         label="Product: framework"),
    probe_build_setting("PRODUCT_TYPE", lambda v: v == "com.apple.product-type.library.static",
                         label="Product: static library"),
    probe_build_setting("PRODUCT_TYPE", lambda v: v == "com.apple.product-type.library.dynamic",
                         label="Product: dynamic library"),
    probe_build_setting("PRODUCT_TYPE", lambda v: v == "com.apple.product-type.bundle",
                         label="Product: resource bundle"),
    probe_build_setting("PRODUCT_TYPE", lambda v: v == "com.apple.product-type.tool",
                         label="Product: command-line tool"),
    probe_build_setting("PRODUCT_TYPE", lambda v: v == "com.apple.product-type.xpc-service",
                         label="Product: XPC service"),
    probe_build_setting("PRODUCT_TYPE", lambda v: v == "com.apple.product-type.bundle.unit-test",
                         label="Product: unit-test bundle"),
    probe_build_setting("PRODUCT_TYPE", lambda v: v == "com.apple.product-type.bundle.ui-testing",
                         label="Product: UI-test bundle"),
    probe_build_setting("PRODUCT_TYPE", lambda v: v.startswith("com.apple.product-type.app-extension"),
                         label="Product: app-extension"),
    probe_build_setting("PRODUCT_TYPE", lambda v: v == "com.apple.product-type.driver-extension",
                         label="Product: DriverKit driver extension"),

    # Settings values with whitespace / quotes
    probe_build_setting("OTHER_LDFLAGS", lambda v: " " in v and '"' in v,
                         label="OTHER_LDFLAGS contains quoted whitespace"),
]


def render_markdown(per_probe: dict[str, dict[str, tuple[bool, str]]], slugs: list[str]) -> str:
    by_category: dict[str, list[str]] = {}
    for probe in PROBES:
        by_category.setdefault(probe.category, []).append(probe.name)

    out: list[str] = []
    out.append("# fixtures/AUDIT.md")
    out.append("")
    out.append("Generated by `scripts/06_audit_coverage.py`. ✅ = at least one fixture "
               "matches the probe; ❌ = no fixture matches. The **Where** column "
               "lists the first slug with a hit and the count of hits.")
    out.append("")

    by_probe = {p.name: p for p in PROBES}
    for category in ("settings", "xcconfig", "pbxproj", "scheme", "files"):
        names = by_category.get(category, [])
        if not names:
            continue
        out.append(f"## {category}")
        out.append("")
        out.append("| Probe | " + " | ".join(slugs) + " | Where |")
        out.append("|---" * (2 + len(slugs)) + "|")
        for name in names:
            row = [name]
            first_hit = ""
            details = ""
            for slug in slugs:
                ok, info = per_probe[name].get(slug, (False, ""))
                row.append("✅" if ok else "❌")
                if ok and not first_hit:
                    first_hit = slug
                    details = info
            row.append(f"{first_hit}: {details}" if first_hit else "—")
            out.append("| " + " | ".join(row) + " |")
        out.append("")
    return "\n".join(out) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--xcode", help="restrict to one Xcode version (e.g. 26.0.1)")
    args = ap.parse_args()

    fixtures_root = common.FIXTURES_DIR
    if not fixtures_root.exists():
        common.log(f"no fixtures/ at {fixtures_root}")
        return 1

    indices: list[FixtureIndex] = []
    slugs: list[str] = []
    for project_dir in sorted(fixtures_root.iterdir()):
        if not project_dir.is_dir():
            continue
        slug = project_dir.name
        for xcode_dir in sorted(project_dir.iterdir()):
            if not xcode_dir.is_dir() or not xcode_dir.name.startswith("xcode-"):
                continue
            if args.xcode and xcode_dir.name != f"xcode-{args.xcode}":
                continue
            common.log(f"indexing {slug}/{xcode_dir.name}")
            idx = index_fixture(slug, xcode_dir)
            indices.append(idx)
            if slug not in slugs:
                slugs.append(slug)

    per_probe: dict[str, dict[str, tuple[bool, str]]] = {p.name: {} for p in PROBES}
    for idx in indices:
        for probe in PROBES:
            try:
                ok, info = probe.check(idx)
            except Exception as e:
                ok, info = False, f"probe error: {e}"
            per_probe[probe.name][idx.slug] = (ok, info)

    md = render_markdown(per_probe, slugs)
    out_md = fixtures_root / "AUDIT.md"
    out_md.write_text(md)
    common.log(f"wrote {out_md}")

    # Machine-readable
    payload = {
        "slugs": slugs,
        "probes": [
            {
                "category": p.category,
                "name": p.name,
                "results": {
                    slug: {"ok": per_probe[p.name].get(slug, (False, ""))[0],
                            "info": per_probe[p.name].get(slug, (False, ""))[1]}
                    for slug in slugs
                }
            } for p in PROBES
        ],
    }
    out_json = fixtures_root / "AUDIT.json"
    out_json.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    common.log(f"wrote {out_json}")

    # Summary stats
    covered = sum(1 for p in PROBES if any(per_probe[p.name][s][0] for s in slugs))
    print(f"\nProbes covered by ≥1 fixture: {covered}/{len(PROBES)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
