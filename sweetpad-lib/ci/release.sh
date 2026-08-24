#!/usr/bin/env bash
#
# Cut a sweetpad CLI release: bump the workspace version, tag, and push.
#
#   sweetpad-lib/ci/release.sh 0.1.3
#
# Pushing the cli-v* tag is what publishes: CI builds a universal binary, signs
# it with the Developer ID, notarizes it with Apple, creates a public GitHub
# release, and pushes a formula bump to sweetpad-dev/homebrew-tap. That reaches
# everyone on 'brew upgrade' and cannot be cleanly retracted, so the push is
# confirmed interactively unless --yes is passed.
#
# The release version comes from the tag and the binary stamps its own from
# Cargo.toml, so the two must agree; the workflow enforces this as well. Only a
# build made at the tag reports a bare version — everywhere else 'sweetpad
# --version' carries a '-dev+<sha>' suffix (sweetpad-cli/build.rs).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

ASSUME_YES=0
VERSION=""
for arg in "$@"; do
  case "$arg" in
    --yes|-y) ASSUME_YES=1 ;;
    -*) echo "unknown flag: $arg" >&2; exit 2 ;;
    *) VERSION="$arg" ;;
  esac
done

if [ -z "$VERSION" ]; then
  echo "usage: sweetpad-lib/ci/release.sh <version> [--yes]   e.g. 0.1.3" >&2
  exit 2
fi
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "version must be MAJOR.MINOR.PATCH (no leading 'v'), got: $VERSION" >&2
  exit 2
fi

TAG="cli-v$VERSION"

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[ "$BRANCH" = "main" ] || { echo "releases are cut from main, on: $BRANCH" >&2; exit 1; }

# A dirty tree would leave uncommitted work out of the tagged commit while the
# build silently succeeds from whatever is committed.
git diff --quiet && git diff --cached --quiet || {
  echo "working tree is dirty; commit or stash first" >&2; exit 1; }

git fetch --quiet origin main --tags
# Tagging an unpushed commit builds the release from a commit that is not on
# main, and the tag would outlive a later rebase.
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || {
  echo "main is not in sync with origin/main; pull or push first" >&2; exit 1; }

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && {
  echo "tag $TAG already exists locally" >&2; exit 1; }
git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1 && {
  echo "tag $TAG already exists on origin" >&2; exit 1; }

CURRENT="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
echo "Releasing sweetpad CLI $CURRENT -> $VERSION"

echo "==> Tests"
cargo test --workspace --quiet
echo "==> Clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> Bumping workspace version"
# Only the [workspace.package] version; member crates inherit it via
# 'version.workspace = true'. Building refreshes Cargo.lock in the same commit.
perl -0pi -e "s/(\[workspace\.package\]\nversion = \")[^\"]+/\${1}$VERSION/" Cargo.toml
cargo build --quiet --bin sweetpad

# A build made away from the release tag stamps '<version>-dev+<sha>', and this
# one is: the bump is not even committed yet. Compare the release core only.
REPORTED="$(./target/debug/sweetpad --version | awk '{print $2}')"
[ "${REPORTED%%-*}" = "$VERSION" ] || {
  echo "binary reports $REPORTED after the bump, expected $VERSION" >&2; exit 1; }

git add Cargo.toml Cargo.lock
git commit -q -m "Release CLI $VERSION"
git tag -a "$TAG" -m "sweetpad CLI $VERSION"

echo
echo "Ready to publish $TAG ($(git rev-parse --short HEAD))."
echo "Pushing signs, notarizes, publishes a public release, and bumps the Homebrew tap."
if [ "$ASSUME_YES" -ne 1 ]; then
  # Without a terminal (CI, a pipeline, an agent) 'read' fails on EOF, which
  # under 'set -e' would abort here — after the commit and tag exist, and
  # before the line saying how to undo them. Treat EOF as declining.
  read -r -p "Push to origin? [y/N] " reply || reply=""
  case "$reply" in
    y|Y|yes) ;;
    *) echo "Stopped. Local commit and tag are in place; 'git tag -d $TAG' to undo."; exit 0 ;;
  esac
fi

git push origin main
git push origin "$TAG"
echo "Pushed. Watch: gh run watch \$(gh run list --workflow=cli-release.yaml --limit 1 --json databaseId -q '.[0].databaseId')"
