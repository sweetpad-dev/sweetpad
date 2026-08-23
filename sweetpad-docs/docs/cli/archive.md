---
sidebar_position: 13
sidebar_label: Archive & distribute
---

# Archive and distribute

`sweetpad archive` produces a distributable build: it runs `xcodebuild archive` to make an
`.xcarchive`, then exports it into an `.ipa` (or a signed `.app`, for macOS).

```bash
sweetpad archive
```

Both artifacts land in `./build` unless you say otherwise:

```bash
sweetpad archive --output-file ./dist
```

## Choosing how it's exported

The export method decides which provisioning profile and entitlements the export uses, and therefore
where the result can be installed:

| `--export-method`   | For                                                                |
| ------------------- | ------------------------------------------------------------------- |
| `debugging`         | Development builds, installable on registered devices. **Default.** |
| `app-store-connect` | Uploading to App Store Connect.                                    |
| `release-testing`   | TestFlight-external and ad-hoc style release testing.              |
| `enterprise`        | In-house enterprise distribution.                                  |
| `developer-id`      | macOS Developer ID: notarizable, distributed outside the App Store. |
| `mac-application`   | macOS: a signed `.app`, with no installer around it.                |

```bash
sweetpad archive --export-method app-store-connect
```

A release archive is normally a Release build, and SweetPad uses whatever configuration your context
resolves to, so say so explicitly rather than assuming:

```bash
sweetpad archive --configuration Release --export-method app-store-connect
```

`sweetpad status` tells you which configuration is in effect and why.

## Bringing your own export options

SweetPad generates an `ExportOptions.plist` from `--export-method`. When your distribution needs
options that flag can't express (a specific provisioning profile mapping, symbol stripping choices,
a manageAppVersionAndBuildNumber setting) supply the plist yourself:

```bash
sweetpad archive --export-options ./ExportOptions.plist
```

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

## Checking before you commit to a run

An archive is slow, and a signing mistake usually surfaces at the end of it. `--show-command` prints
the exact invocations without running them:

```bash
sweetpad archive --show-command
```

## In CI

The pieces from [Scripts and CI](./scripts-and-ci.md) apply here unchanged: pin the context, keep the
output machine-readable, and upload what you built:

```yaml
- name: Archive
  run: |
    sweetpad archive \
      --configuration Release \
      --export-method app-store-connect \
      --output-file ./dist \
      -- -allowProvisioningUpdates
  env:
    SWEETPAD_SCHEME: MyApp

- name: Upload IPA
  uses: actions/upload-artifact@v4
  with:
    name: ipa
    path: ./dist/*.ipa
```

A failed archive exits non-zero, so the job stops on its own without an explicit check. See
[exit codes](./scripts-and-ci.md#exit-codes) for telling a broken build apart from a misconfigured
one.
