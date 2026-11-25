# Delivery fixture

An iOS and macOS app for testing distribution end to end: export, upload to
App Store Connect, and Developer ID notarization. It has what makes signing
hard: an embedded framework (`DeliveryKit`), a widget extension, push
notifications, and an App Group shared by the app and widget. The iOS app also
embeds a WatchKit 2 watch app with its extension and an iMessage sticker pack,
for which an App Store export adds Apple's `WatchKitSupport2` and Messages
stubs, and its framework uses `Span`, so the app embeds a Swift runtime library
that the export puts in `SwiftSupport`. The macOS app
is sandboxed and uses the hardened runtime, as both Mac App Store and
Developer ID builds need.

The project is generated from `project.yml` with XcodeGen. Run `xcodegen` here
after editing the spec, and commit the regenerated `Delivery.xcodeproj`.

## Account settings

Nothing account-specific is committed. The bundle ID, team and profiles come
from build-setting overrides:

| Setting | Value |
| --- | --- |
| `DEVELOPMENT_TEAM` | The team ID. |
| `DELIVERY_BUNDLE_ID` | The test app's bundle ID. The widget is `<id>.Widget`, the framework `<id>.kit`, the App Group `group.<id>`, the watch app `<id>.watchkitapp` with its extension `<id>.watchkitapp.watchkitextension`, and the sticker pack `<id>.Stickers`. |
| `DELIVERY_APS_ENVIRONMENT` | `development`, or `production` for a distribution archive. |
| `CODE_SIGN_STYLE`, `CODE_SIGN_IDENTITY` | `Manual` and the identity, to sign as a particular method. |
| `DELIVERY_APP_PROFILE`, `DELIVERY_WIDGET_PROFILE` | The profile names for manual signing. |
| `DELIVERY_CLIP_PROFILE` | The development profile for `<id>.Clip` when signing the App Clip manually. Its host uses `DELIVERY_APP_PROFILE`. |
| `DELIVERY_WATCH_PROFILE`, `DELIVERY_WATCH_EXTENSION_PROFILE`, `DELIVERY_STICKERS_PROFILE` | The same for the iOS app's watch app, its extension and the sticker pack. |
| `CURRENT_PROJECT_VERSION` | A new build number for each upload. |

The account material lives outside the repository, in `~/.sweetpad-e2e`.
`sweetpad-cli/ci/check-delivery-setup.py` describes its layout and reports
every missing or mismatched certificate, profile and key in one pass.

## Archiving

Pass the overrides after `--`, here for a Developer ID build of the macOS app:

```bash
sweetpad archive --project Delivery.xcodeproj --scheme Delivery-macOS \
  --destination generic/platform=macOS --export-method developer-id \
  -- DEVELOPMENT_TEAM="$TEAM_ID" DELIVERY_BUNDLE_ID="$BUNDLE_ID" \
  DELIVERY_APS_ENVIRONMENT=production CODE_SIGN_STYLE=Manual \
  CODE_SIGN_IDENTITY="Developer ID Application" \
  DELIVERY_APP_PROFILE="<app profile name>" \
  DELIVERY_WIDGET_PROFILE="<widget profile name>" \
  CURRENT_PROJECT_VERSION=1
```

`sweetpad-cli/PLAN_LINUX.md` records which exports, uploads and notarizations
were checked against Apple, and how.

## App Clips

The `DeliveryClipHost` scheme builds a host with `AppClips/DeliveryClip.app`.
The Clip displays the HTTPS URL it receives. Run the following commands from this
directory. Use a dedicated booted iPhone simulator on an Apple silicon Mac.

On the Mac, set `ENGINE=xcodebuild` and `DEST="platform=iOS Simulator,id=$SIMULATOR_UDID"`.
On Linux, set `ENGINE=swift-build` and `DEST="generic/platform=iOS Simulator,arch=arm64"`.
Build once on each machine using the fixture's default bundle IDs and the same
test team so their simulator entitlements can be compared.

```bash
sweetpad build --engine "$ENGINE" --project Delivery.xcodeproj --scheme DeliveryClipHost \
  --configuration Debug --destination "$DEST" -- \
  -derivedDataPath "$PWD/build/$ENGINE-sim" DEVELOPMENT_TEAM=CLIPTEST01
```

Make both host apps available on the Mac and set `XCODE_SIM_APP` and
`LINUX_SIM_APP` to their paths. The checker installs and launches each host,
removes it, then installs its embedded Clip on its own. A standalone UI test
opens a fresh URL in the installed Clip and reads it back from the screen.
The checker verifies the installed binaries against the originals and saves
both XCTest result bundles.

```bash
python3 ../check-app-clip.py simulator --simulator "$SIMULATOR_UDID" \
  --xcode "$XCODE_SIM_APP" --linux "$LINUX_SIM_APP" --out build/clip-invocation
```

For a device archive, use an Apple Development identity and development profiles
for the registered host and Clip. The host profile must name the Clip, and the
Clip profile must grant on-demand installation and name the host. Configure the
Linux signing environment as described in [the Linux reference](../../../sweetpad-docs/docs/cli/linux/reference.md#environment-variables).
The following command produces the Linux archive and IPA. On the Mac, change
`--export-method debugging` to `--no-export` to build the reference archive.

```bash
sweetpad archive --engine "$ENGINE" --project Delivery.xcodeproj --scheme DeliveryClipHost \
  --configuration Release --destination generic/platform=iOS --output-file "$PWD/build/$ENGINE-device" \
  --export-method debugging -- DEVELOPMENT_TEAM="$TEAM_ID" DELIVERY_BUNDLE_ID="$BUNDLE_ID"
```

On the Mac, point the checker at the two archives and the Linux IPA. It compares
the bundle metadata and signed entitlements, verifies each signature against its
embedded profile, and matches the host and Clip dSYMs to their executables.

```bash
python3 ../check-app-clip.py distribution --xcode "$XCODE_ARCHIVE" \
  --linux "$LINUX_ARCHIVE" --ipa "$LINUX_IPA" --out build/clip-signatures
```

To exercise the failure case, append `CODE_SIGN_ENTITLEMENTS=` after `--` on a
signed archive build. Both engines must fail and name the missing
`com.apple.developer.parent-application-identifiers` entitlement.
