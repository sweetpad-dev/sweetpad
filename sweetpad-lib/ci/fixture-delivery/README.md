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
