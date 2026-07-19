# sweetpad

A Rust workspace (`sweetpad-lib`, `sweetpad-core`, `sweetpad-cli`,
`sweetpad-vscode/native`) plus the VS Code extension. `sweetpad-cli/CLI_DESIGN.md`
is the CLI's design record — grammar, output model, and the per-version feature
sections (§9a onward), including directions that are written down but not
scheduled. `sweetpad-lib/CLAUDE.md` covers the project-file and build-settings
crate.

## Releasing the CLI

`sweetpad-lib/ci/release.sh 0.1.3` cuts a release: it bumps the
`[workspace.package]` version, refreshes `Cargo.lock`, runs tests and clippy,
commits `Release CLI <version>`, tags `cli-v<version>`, and pushes after an
interactive confirm (`--yes` skips it). Guards refuse a dirty tree, a branch
other than `main`, a `main` out of sync with `origin`, and a tag that already
exists.

**Pushing the tag is the publish step, and it is public and effectively
irreversible.** `.github/workflows/cli-release.yaml` builds a universal binary,
signs it with the Developer ID, notarizes it with Apple, publishes a GitHub
release, and pushes a formula bump to `sweetpad-dev/homebrew-tap`, which reaches
everyone on `brew upgrade`. A bad release is superseded by the next version
rather than retracted. The CLI ships through the tap on its own cadence; it is
not bundled into the extension's VSIX.

**The tag and the crate version must agree.** The workflow names the release
from the tag, while `sweetpad --version` reports `CARGO_PKG_VERSION` from
`Cargo.toml`. The `Resolve version` step fails the run when they differ, ahead
of the signing and notarization legs, so a mismatch costs seconds instead of
publishing a binary that contradicts its own formula.

Release notes are generated from commit subjects over the tag range, scoped to
the paths the CLI ships from so unrelated monorepo work stays out. There is no
CHANGELOG: commit titles *are* the release notes, which is the practical reason
each one should be a single short self-contained sentence.
