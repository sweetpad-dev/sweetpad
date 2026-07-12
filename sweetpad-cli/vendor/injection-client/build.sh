#!/usr/bin/env bash
#
# Build the bundled InjectionNext hot-reload clients (CLI_DESIGN §9d) and vendor
# them at prebuilt/SweetpadInjectionClient.dylib (iOS simulator) and
# prebuilt/SweetpadInjectionClientMac.dylib (native macOS), where the `sweetpad`
# crate embeds them via include_bytes!.
#
# It builds the SPM wrapper in this directory (see Package.swift) once per
# platform, fat (arm64 + x86_64). The wrapper re-exposes the *upstream*
# InjectionNext SPM product as a dynamic library — no fork, no source patches —
# which is XCTest-free and therefore portable across Xcode versions. So the
# *build* needs macOS + Xcode + network (SPM fetches InjectionNext); the sweetpad
# *runtime* needs none of that.
#
# Re-run (and commit the result) after bumping the `revision` pin in Package.swift.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
DD="$HERE/.build-dd"

# xcodebuild resolves the SPM package from the working directory, and the
# build:cli npm scripts invoke this from sweetpad-lib/, so run from the package
# directory where Package.swift lives.
cd "$HERE"
mkdir -p "$HERE/prebuilt"

# build_one <destination> <products-dir> <output-dylib>
#
# Builds the wrapper for one platform and stages the framework binary as a
# standalone dylib. The binary was signed inside its .framework (against the
# bundle's Info.plist), so the lone extracted Mach-O fails signature validation —
# and dyld refuses to load an inserted library with a broken signature. Re-sign
# it ad-hoc as a standalone dylib so the byte-for-byte copy sweetpad caches loads
# cleanly. (We deliberately don't run install_name_tool: it would also break the
# signature, and the install name is irrelevant under DYLD_INSERT_LIBRARIES.)
build_one() {
    local dest="$1" products="$2" out="$3"

    echo "==> building injection client for $dest (arm64 + x86_64)…"
    rm -rf "$DD"
    xcodebuild -scheme SweetpadInjectionClient \
        -destination "$dest" \
        -derivedDataPath "$DD" \
        ARCHS="arm64 x86_64" ONLY_ACTIVE_ARCH=NO \
        -quiet build

    # The framework layout differs per platform: flat on iOS-family platforms,
    # versioned (Versions/A/) on macOS.
    local fw="$DD/Build/Products/$products/PackageFrameworks/SweetpadInjectionClient.framework"
    local bin="$fw/SweetpadInjectionClient"
    [ -f "$bin" ] || bin="$fw/Versions/A/SweetpadInjectionClient"
    test -f "$bin" || { echo "✗ build produced no dylib under $fw" >&2; exit 1; }

    cp "$bin" "$out"
    codesign --force --sign - "$out"
    verify_one "$out"
}

# Verify a staged dylib is XCTest-free, depends only on ABI-stable OS libraries,
# still carries the client's entry symbols, and has a valid signature.
verify_one() {
    local out="$1"
    echo "==> verifying $(basename "$out") — XCTest-free + only ABI-stable OS dependencies…"
    # Capture once into variables: piping a long producer into `grep -q` trips
    # SIGPIPE under `set -o pipefail` (grep closes early on a match → producer dies
    # 141 → pipeline "fails" even on success). Grepping the variables avoids that.
    local loads syms stray
    loads="$(otool -arch arm64 -L "$out")"
    syms="$(nm -arch arm64 "$out")"

    if printf '%s\n' "$loads" | grep -iE "xctest|quick|nimble"; then
        echo "✗ dylib links XCTest/Quick/Nimble — the SPM product regressed" >&2
        exit 1
    fi
    stray="$(printf '%s\n' "$loads" | tail -n +3 \
        | grep -vE "/usr/lib/|/System/Library/Frameworks/|@rpath/libswift|SweetpadInjectionClient" || true)"
    if [ -n "$stray" ]; then
        echo "✗ non-OS-stable load commands present (would break Xcode portability):" >&2
        echo "$stray" >&2
        exit 1
    fi
    local sym
    for sym in connectToInjection SimpleSocket; do
        # Pure-bash substring match: no pipe to `grep -q`, which would SIGPIPE the
        # producer of the 9k-line symbol list under `set -o pipefail`.
        case "$syms" in
            *"$sym"*) ;;
            *) echo "✗ client symbol '$sym' missing — not a working client" >&2; exit 1 ;;
        esac
    done
    codesign -v "$out" 2>/dev/null || { echo "✗ dylib signature invalid — dyld won't load it" >&2; exit 1; }

    echo "✓ $(basename "$out") — $(lipo -archs "$out"), $(du -h "$out" | cut -f1), 0 XCTest deps"
    echo "  staged at: ${out#"$HERE"/} (gitignored; build.rs embeds it via include_bytes!)"
}

build_one 'generic/platform=iOS Simulator' 'Debug-iphonesimulator' "$HERE/prebuilt/SweetpadInjectionClient.dylib"
build_one 'generic/platform=macOS' 'Debug' "$HERE/prebuilt/SweetpadInjectionClientMac.dylib"
