#!/usr/bin/env bash
#
# Cut a SweetPad extension patch release: bump the version, tag, and push.
#
#   sweetpad-vscode/scripts/publish-patch.sh [--yes]
#   npm run publish-patch          (from sweetpad-vscode/)
#
# Pushing the v* tag is what publishes: '.github/workflows/ci.yaml' builds the
# VSIX and deploys it to the marketplace, which reaches everyone on their next
# extension update and cannot be cleanly retracted, so the push is confirmed
# interactively unless --yes is passed.
#
# The CHANGELOG entry is written by hand before running this — the script
# refuses to release a version the changelog does not describe, because the
# entry is the release note users actually read.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

PKG="sweetpad-vscode/package.json"
LOCK="package-lock.json"
CHANGELOG="sweetpad-vscode/CHANGELOG.md"

ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
    --yes|-y) ASSUME_YES=1 ;;
    *) echo "usage: sweetpad-vscode/scripts/publish-patch.sh [--yes]" >&2; exit 2 ;;
  esac
done

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[ "$BRANCH" = "main" ] || { echo "releases are cut from main, on: $BRANCH" >&2; exit 1; }

# A dirty tree would leave uncommitted work out of the tagged commit while the
# deploy silently succeeds from whatever is committed. The changelog is the one
# exception: the entry is written for the version this script is about to cut,
# so it belongs in that release commit rather than a separate one before it.
while read -r changed; do
  [ -z "$changed" ] && continue
  [ "$changed" = "$CHANGELOG" ] || {
    echo "working tree is dirty ($changed); commit or stash first" >&2
    echo "only $CHANGELOG may be uncommitted — it goes into the release commit." >&2
    exit 1; }
done <<<"$(git status --porcelain --untracked-files=no | awk '{print $NF}')"

git fetch --quiet origin main --tags
# Tagging an unpushed commit deploys from a commit that is not on main, and the
# tag would outlive a later rebase.
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || {
  echo "main is not in sync with origin/main; pull or push first" >&2; exit 1; }

CURRENT="$(node -p "require('./$PKG').version")"
# Derive the next patch ourselves so the changelog can be checked before
# anything is written. 'npm version' prints install chatter rather than the new
# version, so its stdout is never the source of truth here.
IFS='.' read -r MAJOR MINOR PATCH <<<"$CURRENT"
[[ "$MAJOR" =~ ^[0-9]+$ && "$MINOR" =~ ^[0-9]+$ && "$PATCH" =~ ^[0-9]+$ ]] || {
  echo "cannot parse current version: $CURRENT" >&2; exit 1; }
VERSION="$MAJOR.$MINOR.$((PATCH + 1))"
TAG="v$VERSION"

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && {
  echo "tag $TAG already exists locally" >&2; exit 1; }
git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1 && {
  echo "tag $TAG already exists on origin" >&2; exit 1; }

grep -qF "## [$VERSION]" "$CHANGELOG" || {
  echo "no '## [$VERSION]' entry in $CHANGELOG" >&2
  echo "add the release note first — it is what users read on the marketplace." >&2
  exit 1; }

echo "Releasing SweetPad extension $CURRENT -> $VERSION"

echo "==> Fonts"
npm run --silent verify-font --workspace sweetpad

echo "==> Types"
npm run --silent check:types --workspace sweetpad

echo "==> Tests"
npm run --silent test --workspace sweetpad

echo "==> Bumping version"
# Writes the extension's package.json and the root lockfile, with no git side
# effects of its own: in a workspace member 'npm version' does not commit or
# tag, which is why the commit and tag below are made explicitly.
(cd sweetpad-vscode && npm version "$VERSION" --no-git-tag-version >/dev/null)

BUMPED="$(node -p "require('./$PKG').version")"
[ "$BUMPED" = "$VERSION" ] || {
  echo "$PKG reports $BUMPED after the bump, expected $VERSION" >&2; exit 1; }
git diff --quiet -- "$LOCK" && {
  echo "$LOCK did not pick up the bump; run 'npm install' and retry" >&2; exit 1; }

git add "$PKG" "$LOCK" "$CHANGELOG"
git commit -q -m "Release $VERSION"
git tag -a "$TAG" -m "SweetPad $VERSION"

echo
echo "Ready to publish $TAG ($(git rev-parse --short HEAD))."
echo "Pushing builds the VSIX and deploys it to the marketplace."
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
echo "Pushed. Watch: gh run watch \$(gh run list --workflow=ci.yaml --limit 1 --json databaseId -q '.[0].databaseId')"
