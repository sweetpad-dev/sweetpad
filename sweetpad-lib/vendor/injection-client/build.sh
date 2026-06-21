#!/usr/bin/env bash
#
# Build the bundled InjectionNext hot-reload client (CLI_DESIGN §9d) and vendor it
# at prebuilt/SweetpadInjectionClient.dylib, where the `sweetpad` crate embeds it
# via include_bytes!.
#
# It builds the SPM wrapper in this directory (see Package.swift) for the iOS
# simulator, fat (arm64 + x86_64). The wrapper re-exposes the *upstream*
# InjectionNext SPM product as a dynamic library — no fork, no source patches —
# which is XCTest-free and therefore portable across Xcode versions. So the
# *build* needs macOS + Xcode + network (SPM fetches InjectionNext); the sweetpad
# *runtime* needs none of that.
#
# Re-run (and commit the result) after bumping the `revision` pin in Package.swift.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
DD="$HERE/.build-dd"
OUT="$HERE/prebuilt/SweetpadInjectionClient.dylib"

echo "==> building injection client for iphonesimulator (arm64 + x86_64)…"
rm -rf "$DD"
xcodebuild -scheme SweetpadInjectionClient \
    -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath "$DD" \
    ARCHS="arm64 x86_64" ONLY_ACTIVE_ARCH=NO \
    -quiet build

BIN="$DD/Build/Products/Debug-iphonesimulator/PackageFrameworks/SweetpadInjectionClient.framework/SweetpadInjectionClient"
test -f "$BIN" || { echo "✗ build produced no dylib at $BIN" >&2; exit 1; }

mkdir -p "$HERE/prebuilt"
cp "$BIN" "$OUT"
# The dylib was signed inside its .framework (against the bundle's Info.plist), so
# the lone extracted Mach-O fails signature validation — and the simulator refuses
# to dlopen an inserted library with a broken signature. Re-sign it ad-hoc as a
# standalone dylib so the byte-for-byte copy sweetpad caches loads cleanly. (We
# deliberately don't run install_name_tool: it would also break the signature, and
# the install name is irrelevant under DYLD_INSERT_LIBRARIES.)
codesign --force --sign - "$OUT"

echo "==> verifying XCTest-free + only ABI-stable OS dependencies…"
# Capture once into variables: piping a long producer into `grep -q` trips
# SIGPIPE under `set -o pipefail` (grep closes early on a match → producer dies
# 141 → pipeline "fails" even on success). Grepping the variables avoids that.
loads="$(otool -arch arm64 -L "$OUT")"
syms="$(nm -arch arm64 "$OUT")"

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
for sym in connectToInjection SimpleSocket; do
    # Pure-bash substring match: no pipe to `grep -q`, which would SIGPIPE the
    # producer of the 9k-line symbol list under `set -o pipefail`.
    case "$syms" in
        *"$sym"*) ;;
        *) echo "✗ client symbol '$sym' missing — not a working client" >&2; exit 1 ;;
    esac
done
codesign -v "$OUT" 2>/dev/null || { echo "✗ dylib signature invalid — the simulator won't load it" >&2; exit 1; }

echo "✓ $(basename "$OUT") — $(lipo -archs "$OUT"), $(du -h "$OUT" | cut -f1), 0 XCTest deps"
echo "  staged at: ${OUT#"$HERE"/} (gitignored; build.rs embeds it via include_bytes!)"
