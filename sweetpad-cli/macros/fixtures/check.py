#!/usr/bin/env python3
"""Compare what sweetpad-macros expands the fixtures to with what Xcode's own
plugins expand them to, recorded in expected/. Needs Xcode (macOS).

    check.py            build the plugin, then compare
    check.py --record   record Xcode's expansions into expected/

Each fixture is its own module. An expansion is keyed by its mangled name,
which the compiler gives each one from its place in the source, so the
fixture and the expansions it's recorded with change together.
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PACKAGE = os.path.dirname(HERE)
# The modules sweetpad build hands the plugin (MACRO_MODULES in swbuild.rs).
MODULES = "FoundationMacros,PreviewsMacros,SwiftDataMacros"


def expansions(fixture, plugin):
    sdk = subprocess.run(["xcrun", "--sdk", "iphonesimulator", "--show-sdk-path"],
                         capture_output=True, text=True, check=True).stdout.strip()
    command = ["xcrun", "swiftc", "-typecheck", "-sdk", sdk,
               "-target", "arm64-apple-ios26.0-simulator", "-swift-version", "6",
               "-module-name", os.path.splitext(fixture)[0],
               "-Xfrontend", "-dump-macro-expansions", fixture]
    if plugin:
        command += ["-load-plugin-executable", f"{plugin}#{MODULES}"]
    log = subprocess.run(command, cwd=HERE, capture_output=True, text=True).stderr
    lines = log.replace(HERE + "/", "").splitlines()
    found, diagnostics, i = {}, set(), 0
    while i < len(lines):
        line = lines[i]
        if line.startswith("@__swiftmacro_") and i + 1 < len(lines) and lines[i + 1].startswith("-----"):
            j = i + 2
            while j < len(lines) and not lines[j].startswith("-" * 30):
                j += 1
            found[line[1:]] = "\n".join(lines[i + 2:j])
            i = j + 1
            continue
        if line.startswith(fixture + ":") and (": error: " in line or ": warning: " in line and "(from macro" in line):
            diagnostics.add(line)
        i += 1
    out = "".join(f"=== {name}\n{body}\n" for name, body in sorted(found.items()))
    return out + "".join(f"=== diagnostic\n{d}\n" for d in sorted(diagnostics))


def main():
    record = sys.argv[1:] == ["--record"]
    plugin = None
    if not record:
        subprocess.run(["swift", "build", "-c", "release", "--package-path", PACKAGE,
                        "--product", "sweetpad-macros"], check=True)
        bin_path = subprocess.run(["swift", "build", "-c", "release", "--package-path", PACKAGE,
                                   "--show-bin-path"], capture_output=True, text=True,
                                  check=True).stdout.strip()
        plugin = os.path.join(bin_path, "sweetpad-macros")
    failed = False
    for fixture in sorted(f for f in os.listdir(HERE) if f.endswith(".swift")):
        got = expansions(fixture, plugin)
        path = os.path.join(HERE, "expected", fixture.replace(".swift", ".txt"))
        if record:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            open(path, "w").write(got)
            print(f"{fixture}: {got.count('=== ')} recorded")
            continue
        want = open(path).read()
        if got == want:
            print(f"{fixture}: {want.count('=== ')} match")
            continue
        failed = True
        print(f"{fixture}: differs from Xcode's")
        for a, b in zip(got.splitlines(), want.splitlines()):
            if a != b:
                print(f"  got:      {a}\n  expected: {b}")
                break
    sys.exit(1 if failed else 0)


main()
