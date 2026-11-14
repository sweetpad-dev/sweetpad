#!/bin/bash
# Regenerate what `productbuild` makes of the fixture Mac app, for the
# package tests in src/cli/pkg/tests.rs. Run on a Mac from any directory.
#
# The app is fixtures/codesign/expected/Mac.app with Info.plist here in
# place of its own (a version and a display name added), every time set to
# MTIME, folders at 0755, and no extended attributes. The tests copy the app
# the same way and compare the bill of materials, payload, PackageInfo and
# Distribution.
#
# This is a shell script because macOS can refuse to clear the
# com.apple.provenance attribute for a Python parent, and productbuild
# keeps every extended attribute as a ._ file.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
mtime=202701150800.00 # 1_800_000_000, 2027-01-15 08:00:00 UTC
work=$(mktemp -d "$here/work.XXXXXX")
trap 'rm -rf "$work"' EXIT

app="$work/Mac.app"
cp -R "$here/../codesign/expected/Mac.app" "$app"
cp "$here/../codesign/expected/Mac.app/Contents/Info.plist" "$here/Info.plist"
plutil -convert xml1 "$here/Info.plist"
plutil -insert CFBundleShortVersionString -string 1.0 "$here/Info.plist"
plutil -insert CFBundleDisplayName -string "Mac Fixture" "$here/Info.plist"
cp "$here/Info.plist" "$app/Contents/Info.plist"
find "$app" -type d -exec chmod 0755 {} +
TZ=UTC find "$app" -exec touch -h -t "$mtime" {} +
xattr -crs "$app"

productbuild --component "$app" /Applications "$work/Mac.pkg" >/dev/null
pkgutil --expand "$work/Mac.pkg" "$work/expanded"
for name in Bom Payload PackageInfo; do
  cp "$work/expanded/dev.sweetpad.macfixture.pkg/$name" "$here/$name"
done
cp "$work/expanded/Distribution" "$here/Distribution"
