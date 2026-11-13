---
sidebar_label: Download Xcode on Linux
---

# Download Xcode from Apple

CLI 0.1.23 can authenticate with Apple and download Xcode directly on Linux.
The login and download commands also work on macOS.
These commands require Python 3 and CA certificates.
Use Linux on Apple hardware under [Apple's Xcode agreement](https://www.apple.com/legal/sla/docs/xcode.pdf).
Keep the archive and authenticated session private.

## Sign in from a terminal

Run this command in an interactive terminal:

```bash
sweetpad auth login --apple-id you@example.com
```

Enter your Apple account password at the hidden prompt.
Then enter the verification code from your trusted device or SMS.
Sweetpad does not store your password or verification code.
There is no password argument or password environment variable.

Sweetpad saves session cookies in `$XDG_STATE_HOME/sweetpad/apple-session.json`.
The default path is `~/.local/state/sweetpad/apple-session.json`.
The file has mode `600`. Treat it as a credential.
An existing valid session lets subsequent commands run without another password prompt.

The terminal flow supports ordinary Apple accounts with trusted-device or SMS two-factor codes.
Terminal login does not support federated accounts, physical security keys, or legacy two-step authentication.
Sweetpad reports these authentication requirements before it saves a new session.
If Apple requires new agreements, accept them in a browser before another login attempt.

## Download an archive

Select an exact release version:

```bash
sweetpad sdk download --xcode 27.2
```

For a prerelease, include its beta or release-candidate name:

```bash
sweetpad sdk download --xcode '27.3 beta'
```

A stable version selector does not silently select a beta.
Sweetpad resolves the archive through Apple's authenticated download catalog and current prerelease page.
It supports `.xip` and `.aea` archives and prefers `.aea` when both formats describe the same release.

The default destination is `$XDG_CACHE_HOME/sweetpad/downloads/` or `~/.cache/sweetpad/downloads/`.
Use `--output-file /private/path/Xcode.aea` to select another location.
After an interruption, repeat the same command to resume the download.
Resume uses Apple's entity validator and verifies the returned byte range.

Sweetpad checks the archive size and format, then records its local SHA-256 digest.
Repeated commands verify this digest before they reuse a completed download.
This receipt detects local changes. It is not an Apple-published checksum or publisher certificate check.
The SDK importer performs the separate integrity and authentication checks described in [SDK import](sdk-transfer.md).

## Download and configure Linux

After login, download and import with one command:

```bash
sweetpad setup --xcode '27.3 beta'
source "$HOME/.config/sweetpad/linux-env.sh"
sweetpad doctor --target ios --json --non-interactive
```

Setup installs the pinned Linux tools and builds the packaged iPhone sample.
It retains the downloaded archive in the cache.
Allow disk space for that archive, extracted inputs, toolchains, and the new SDK.
Use `--replace-sdk` to replace a different SDK or an older import recipe explicitly.
The existing `setup --sdk ARCHIVE` flow also remains available.
The `--sdk` and `--xcode` options are mutually exclusive.

## Reuse a private session in CI

Create a dedicated session file in an interactive terminal:

```bash
sweetpad auth login --apple-id you@example.com \
  --session-file "$HOME/private/apple-session.json"
```

Transfer this file through your CI secret store.
Restore the file with mode `600` and ownership for the user that runs Sweetpad.
Then supply its path:

```bash
sweetpad sdk download --xcode 27.2 \
  --session-file "$RUNNER_TEMP/apple-session.json" --json --non-interactive

sweetpad setup --xcode 27.2 \
  --session-file "$RUNNER_TEMP/apple-session.json" --json --non-interactive
```

You can also set `SWEETPAD_APPLE_SESSION_FILE` to the same path.
Download and setup accept a read-only session file.
The file uses Sweetpad's versioned JSON format. Fastlane YAML and browser cookie databases are not accepted.
JSON output and non-interactive mode never request a password or verification code.

Apple controls session expiry and can restrict a session by network or region.
If a CI session expires, create a fresh file interactively and replace the CI secret.
Do not publish the session, archive, or SDK as a public artifact or container layer.
Keep signing identities and provisioning profiles in separate private files.

## Remove a saved session

```bash
sweetpad auth logout
```

For a custom file, add `--session-file PATH`.
Logout removes only that local Sweetpad session file.
It does not sign out your browser or revoke other copies of the session.

## Protocol and Linux compatibility

Sweetpad uses [XcodesLoginKit](https://github.com/XcodesOrg/XcodesLoginKit) as a reference for Apple's SRP and two-factor protocol.
The referenced Swift library imports CommonCrypto, CryptoKit, and Apple's `os` module.
Those imports prevent direct use on Linux.
Sweetpad's implementation uses Python's standard library and requires no Mac authentication helper.

Apple's sign-in and download endpoints can change independently of Sweetpad.
An unsupported response produces an explicit error.
