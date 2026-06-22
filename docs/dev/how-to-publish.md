# How to publish

SweetPad ships two artifacts, versioned and released independently:

- the **VSCode extension** — `v*` tags → VS Marketplace / Open VSX
- the **`sweetpad` CLI** — `cli-v*` tags → Homebrew

## VSCode extension

### Links

- https://code.visualstudio.com/api/working-with-extensions/publishing-extension#publish-an-extension
- https://marketplace.visualstudio.com/manage/publishers/sweetpad/extensions/sweetpad/hub
- https://open-vsx.org/extension/sweetpad/sweetpad

### Steps

1. Update & commit CHANGELOG.md

2. Update the version number and publish to Github:

```shell
npm run publish-patch
```

3. Check publishing status on github actions page:

- https://github.com/sweetpad-dev/sweetpad/actions

4. Verify the GitHub release page includes the VSIX asset and the changelog section for the tag:

- https://github.com/sweetpad-dev/sweetpad/releases

The `v*` tag triggers `.github/workflows/ci.yaml`, which builds the VSIX (Developer ID-signing + notarizing the bundled `.node` addon) and publishes to the VS Marketplace, Open VSX, and a GitHub release.

## CLI (Homebrew)

The `sweetpad` CLI is distributed through the Homebrew tap
[`sweetpad-dev/homebrew-tap`](https://github.com/sweetpad-dev/homebrew-tap)
(`brew install sweetpad-dev/tap/sweetpad`). It is **not** bundled in the extension.

### Steps

1. Bump the version in `sweetpad-lib/Cargo.toml` (`version = "X.Y.Z"`) and commit it.
   This is what `sweetpad --version` reports and **must match the tag below** — the
   formula's `brew test` asserts `sweetpad X.Y.Z`, so a mismatch fails the formula.

2. Tag and push (the tag is the release version):

```shell
git tag -a cli-vX.Y.Z -m "sweetpad CLI X.Y.Z"
git push origin cli-vX.Y.Z
```

3. The push triggers `.github/workflows/cli-release.yaml`, which:
   - builds the universal CLI (including the bundled injection client),
   - Developer ID-signs + notarizes it,
   - publishes `sweetpad-cli-X.Y.Z-macos-universal.tar.gz` to a GitHub release `cli-vX.Y.Z`, and
   - regenerates `Formula/sweetpad.rb` in the tap (from `.github/homebrew/sweetpad.rb.tmpl`) and pushes it via the `HOMEBREW_TAP_DEPLOY_KEY` deploy key.

4. Verify:
   - the release has the tarball — https://github.com/sweetpad-dev/sweetpad/releases
   - the tap has the bumped formula — https://github.com/sweetpad-dev/homebrew-tap/blob/main/Formula/sweetpad.rb
   - `brew update && brew install sweetpad-dev/tap/sweetpad && sweetpad --version`

### Dry run

To exercise build + sign + notarize without publishing (the release and tap-bump steps are gated on a tag), run the workflow manually — from the Actions tab or:

```shell
gh workflow run cli-release.yaml --ref main
```

### Required secrets

Already configured on the repo; listed for reference:

- `MACOS_CERTIFICATE_P12`, `MACOS_CERTIFICATE_PASSWORD` — Developer ID Application certificate (`.p12`, base64) and its export password
- `MACOS_NOTARY_KEY_P8`, `MACOS_NOTARY_KEY_ID`, `MACOS_NOTARY_ISSUER_ID` — App Store Connect API key for `notarytool`
- `HOMEBREW_TAP_DEPLOY_KEY` — private SSH key of the `ci-formula-bump` write deploy key on the tap
