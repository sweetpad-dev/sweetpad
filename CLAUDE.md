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
from the tag, while the binary's version is stamped from `Cargo.toml`. The
`Resolve version` step fails the run when they differ, ahead of the signing and
notarization legs, so a mismatch costs seconds instead of publishing a binary
that contradicts its own formula.

**Only a build made at the `cli-v<version>` tag reports the bare version.**
Anywhere else `sweetpad --version` stamps `<version>-dev+<sha>` (`build.rs`):
the crate version alone cannot distinguish a build off `main` from the release
sharing its number, so a bare version read off a local build is not evidence
that a fix has shipped. `release.sh` compares only the part before the `-`,
since it checks the binary before tagging.

Release notes are generated from commit subjects over the tag range, scoped to
the paths the CLI ships from so unrelated monorepo work stays out. There is no
CHANGELOG: commit titles *are* the release notes, which is the practical reason
each one should be a single short self-contained sentence.

## Options objects: `?` versus `| undefined`

**Do not mark an options property optional when the call sites all hold the
value.** Declare it `foo: T | undefined` instead, so a caller with nothing to
pass has to say so.

Both read the same at the use site. They differ for the next call somebody
adds: `foo?: T` lets it compile while quietly taking the default, while
`foo: T | undefined` is a type error until it is answered. `refreshBuildServer`
carries `configuration` this way, which is what makes each of its six call
sites name one instead of falling through to Debug, and
`repairStaleBuildServerConfig` carries `workspaceState` the same way.

Keep `?` for what a caller really can have no opinion on: a flag with a
default, a rarely-used override. A spec that would otherwise need a stub is not
one of those — write the stub. Loosening a signature to keep a test call short
means the tests stop exercising the shape production actually uses.

## Writing the extension CHANGELOG

`sweetpad-vscode/CHANGELOG.md` is what users read on the Marketplace, and
`publish-patch.sh` refuses to cut a release without a `## [<version>]` entry for
the version it is about to tag.

**Write each entry as a title, not a sentence.** Ten words is a good ceiling:
one clause, no colon splicing two halves together, no subordinate clause
explaining the mechanism. The CLI's notes are commit subjects, so they stay
terse by construction; the extension's are hand-written, and the failure mode is
a 60-word entry that packs the symptom, the root cause and the fix into one
breath. Say what changed from the user's side and stop. Anyone who wants the
diagnosis has the linked issue.

    too long  - Fix schemes and targets from a workspace's local Swift packages
                never reaching the Build and Testing panels: a package joins an
                `.xcworkspace` as a reference to its directory, and SweetPad
                read only the references ending in `.xcodeproj`
    a title   - Fix Swift package schemes missing from the Build and Testing
                panels

Attribution stays as it is: link the issue or PR, and thank the reporter.

    - Fix Swift package schemes missing from the Build and Testing panels
      ([#327](https://github.com/sweetpad-dev/sweetpad/issues/327),
      thanks [@rssole](https://github.com/rssole))

Entries are written one per user-visible change, newest version on top, and the
version's own entries ordered features first, then fixes.
