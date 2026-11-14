---
sidebar_position: 13
sidebar_label: Archive & distribute
---

# Archive and distribute

`sweetpad archive` builds an `.xcarchive` and exports it: an `.ipa` for iPhone and the other iOS
platforms, and an `.app` or installer `.pkg` for macOS. The export can then go straight to App Store
Connect, or to Apple's notary service for a Developer ID build.

```bash
sweetpad archive
```

On a Mac the archive and the export run through xcodebuild. On Linux, SweetPad builds with Swift Build
and signs the export itself, the way Xcode's export does. See
[signing on Linux](./linux/signing-and-testing.md#archive-and-distribute) for that setup.

Everything lands in `./build` unless you say otherwise:

```bash
sweetpad archive --output-file ./dist
```

## Choosing how it's exported

The export method decides which identity, provisioning profile and entitlements the export signs
with, and therefore where the result can go:

| `--export-method`   | For                                                    | Writes                         |
| ------------------- | ------------------------------------------------------ | ------------------------------ |
| `debugging`         | Development builds for registered devices. **Default.** | `.ipa`, or `.app` for macOS    |
| `app-store-connect` | Uploading to App Store Connect and TestFlight.          | `.ipa`, or `.pkg` for macOS    |
| `release-testing`   | Ad hoc builds for registered devices.                   | `.ipa`                         |
| `enterprise`        | In-house enterprise distribution.                       | `.ipa`                         |
| `developer-id`      | macOS apps distributed outside the App Store.           | `.app`, and a `.pkg`           |
| `mac-application`   | macOS: the `.app` as it was signed, with no installer.  | `.app`                         |

```bash
sweetpad archive --export-method app-store-connect
```

A Developer ID export writes the `.pkg` only when a Developer ID Installer identity is available.

The archive is built for every architecture Xcode builds for a generic destination, so a Mac app is
universal, arm64 and x86_64. An App Store `.ipa` carries the `SwiftSupport` and `WatchKitSupport2`
folders App Store Connect expects, with Apple's own copies of the Swift libraries the app embeds.

A release archive is normally a Release build, and SweetPad uses whatever configuration your context
resolves to, so say so explicitly rather than assuming:

```bash
sweetpad archive --configuration Release --export-method app-store-connect
```

`sweetpad status` tells you which configuration is in effect and why.

## Exporting an archive you already have

`archive export` exports an existing `.xcarchive` again, with another method or options file. It's
the quick way to make an ad hoc build and an App Store build from the same archive:

```bash
sweetpad archive --no-export --output-file ./dist
sweetpad archive export ./dist/MyApp.xcarchive --export-method release-testing --output-file ./adhoc
sweetpad archive export ./dist/MyApp.xcarchive --upload --output-file ./store
```

## Bringing your own export options

SweetPad generates an `ExportOptions.plist` from `--export-method`. When your distribution needs
options that flag can't express, such as a specific provisioning profile per bundle, supply the plist
yourself:

```bash
sweetpad archive --export-options ./ExportOptions.plist
```

xcodebuild reads every key. SweetPad's own export on Linux reads `method`, `teamID`, `signingStyle`,
`signingCertificate`, `installerSigningCertificate`, `provisioningProfiles` and `destination`, and
refuses, by name, any other key that would change the result. Either way, the export folder gets
the options it used.

To stop before the export entirely, and hand the `.xcarchive` to something else:

```bash
sweetpad archive --no-export
```

## Signing

Archiving is where signing stops being optional, and it's the one part xcodebuild usually wants more
from you than SweetPad asks for. Pass what it needs through the `--` tail:

```bash
sweetpad archive -- -allowProvisioningUpdates
sweetpad archive -- -allowProvisioningUpdates DEVELOPMENT_TEAM=ABCDE12345
```

If every archive in the project needs the same thing, write it down once in `sweetpad.toml` instead of
typing it each release:

```toml
# sweetpad.toml
[xcodebuild]
args = ["-allowProvisioningUpdates"]
```

See [Configuration](./configuration.md#every-key) for what belongs there and what doesn't.

## Uploading to App Store Connect

`--upload` sends the export to App Store Connect, and implies `--export-method app-store-connect`.
`sweetpad upload` does the same for an `.ipa` or `.pkg` you already have:

```bash
sweetpad archive --configuration Release --upload
sweetpad upload ./dist/export/MyApp.ipa
```

Both talk to App Store Connect's API directly, on a Mac and on Linux alike, so there's no Transporter
or altool to install.

### The API key

Uploads sign in with an App Store Connect API key, created under Users and Access in App Store
Connect. A team key needs the App Manager role or higher. Point SweetPad at it with flags or
environment variables:

| Flag             | Variable                 | Value                                                  |
| ---------------- | ------------------------ | ------------------------------------------------------ |
| `--api-key`      | `SWEETPAD_ASC_KEY_ID`    | The key ID.                                            |
| `--api-issuer`   | `SWEETPAD_ASC_ISSUER_ID` | The issuer ID of a team key. Leave it out for an individual key. |
| `--api-key-path` | `SWEETPAD_ASC_KEY_PATH`  | The `AuthKey_<id>.p8` file.                             |
|                  | `SWEETPAD_ASC_KEY`       | The key file's text, for a CI secret.                   |

Without a path, SweetPad looks for `AuthKey_<id>.p8` in the folders altool uses: `./private_keys`,
`~/private_keys`, `~/.private_keys`, `~/.appstoreconnect/private_keys` and `$API_PRIVATE_KEYS_DIR`.

### What an upload waits for

An upload returns once Apple accepts or rejects the file. A rejection exits 1 and prints Apple's
messages. Processing comes after that and often takes 5 to 30 minutes; `--wait` follows it too, and
`upload status` picks up an upload later:

```bash
sweetpad upload ./dist/export/MyApp.ipa --wait
sweetpad upload status UPLOAD_ID
```

App Store Connect needs a new build number for each upload. If it has seen yours, the error says so,
and the fix is to raise `CURRENT_PROJECT_VERSION`.

The app is found by its bundle identifier. Pass `--app-id` with the app's Apple ID to skip the lookup.

## Notarizing a Developer ID build

A macOS app distributed outside the App Store needs Apple's notarization before Gatekeeper lets it
open without a warning. `--notarize` implies `--export-method developer-id` and does the rest:

```bash
sweetpad archive --configuration Release --notarize
```

It sends the app to Apple, waits for the verdict, staples the ticket to the app and zips the stapled
app beside it. Then it does the same for the `.pkg`. The export folder ends up with the stapled `.app`,
its `.zip` and the stapled `.pkg`.

`sweetpad notarize` does the same for one file: an `.app`, `.zip`, `.pkg` or `.dmg`.

```bash
sweetpad notarize ./dist/export/MyApp.pkg
```

A disk image gets its ticket stapled only if it's signed with your Developer ID Application identity,
so sign it before you notarize it. An unsigned one is still notarized but can't hold a ticket, and
Gatekeeper then looks the app's ticket up online.

:::info
Notarization takes a team API key. An individual key can upload builds, but Apple's notary service
refuses it.
:::

The app must be signed with the hardened runtime. SweetPad refuses an export without it and names the
build setting to change, `ENABLE_HARDENED_RUNTIME`.

### Following a submission

`--no-wait` returns once Apple has the file, with the submission ID. Follow it from there:

```bash
sweetpad notarize ./MyApp.zip --no-wait
sweetpad notarize status SUBMISSION_ID --wait
sweetpad notarize log SUBMISSION_ID
sweetpad notarize staple ./MyApp.app
```

`notarize log` shows what Apple checked and every issue it found. When Apple refuses a file, the
`notarize` run already lists those issues and exits 1. `notarize staple` needs no API key.

## Checking before you commit to a run

An archive is slow, and a signing mistake usually surfaces at the end of it. `--show-command` prints
the exact invocations without running them:

```bash
sweetpad archive --show-command
```

`sweetpad doctor --network` checks that App Store Connect, the notary service and Apple's timestamp
server answer from where you are. Run it on a new CI machine or behind a proxy.

## In CI

The pieces from [Scripts and CI](./scripts-and-ci.md) apply here unchanged: pin the context, keep the
output machine-readable, and keep the API key in your secret store:

```yaml
- name: Archive and upload to TestFlight
  run: |
    sweetpad archive \
      --configuration Release \
      --upload \
      --output-file ./dist \
      --json --non-interactive \
      -- -allowProvisioningUpdates
  env:
    SWEETPAD_SCHEME: MyApp
    SWEETPAD_ASC_KEY_ID: ${{ secrets.ASC_KEY_ID }}
    SWEETPAD_ASC_ISSUER_ID: ${{ secrets.ASC_ISSUER_ID }}
    SWEETPAD_ASC_KEY: ${{ secrets.ASC_KEY }}

- name: Keep the IPA
  uses: actions/upload-artifact@v4
  with:
    name: ipa
    path: ./dist/export/*.ipa
```

A failed archive, a rejected upload and a refused notarization all exit non-zero, so the job stops on
its own without an explicit check. See [exit codes](./scripts-and-ci.md#exit-codes) for telling a
broken build apart from a misconfigured one.
