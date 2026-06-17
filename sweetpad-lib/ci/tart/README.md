# Reproducible oracle capture in Tart VMs

The committed oracles (`fixtures/<slug>/xcode-<ver>/`, `xcspec-cache/`) were
first captured on a personal Mac, so they carry that host's fingerprint —
today ~95k literal `/Users/hyzyla` path strings and a host-specific
DerivedData hash. The resolver's canonicalizer strips `$HOME` / the hash / SDK
versions, so the `structural` and `canonical` scoring tiers already pass
cross-machine ([`DOCS.md` §5.3](../../DOCS.md)); but a recapture on any other
machine still rewrites every path string (unreviewable churn) and the
path-derived hash (commits #287/#288/#289) is never **byte**-stable.

This directory makes capture happen in a **pinned, isolated macOS VM** so every
recapture is byte-reproducible, and so refreshing an Xcode version needs **no
Xcode download onto your working Mac** — Xcode lives in the image.

[Tart](https://tart.run) (Cirrus Labs) is the "container for macOS": OCI-
distributed macOS VM images on Apple Silicon. We use the Cirrus `*-xcode`
images, which ship Xcode preinstalled with a fixed user (`/Users/admin`).

## The identity invariant

Everything that would otherwise differ byte-for-byte across machines is pinned
in [`images.json`](images.json):

| Axis | Pinned to |
|---|---|
| `$HOME` / username (~95k path strings) | `/Users/admin` (Cirrus base user) |
| Checkout path (⇒ DerivedData hash) | `/Users/admin/sweetpad` |
| Xcode (build version ⇒ `CCHROOT`/`CACHE_ROOT`/object dirs) | the image's bundled Xcode |
| Simulator runtime | disposable — any runtime per platform (settings don't depend on it, §5.1) |

`capture-runner.sh` **refuses to run** off the canonical home/path (override
with `SWEETPAD_ALLOW_NONCANONICAL=1` only for throwaway experiments), so you
can't accidentally reintroduce host drift into a capture you mean to commit.

## Files

| File | Role |
|---|---|
| `images.json` | Single source of truth: per-version image ref, corpus subset, runtimes, canonical paths. |
| `capture.sh` | **Host driver** (Apple Silicon Mac + Tart). Clones the image, syncs this tree in, runs the capture, pulls the oracles back. |
| `capture-runner.sh` | **In-VM** worker. Enforces identity, exposes the bundled Xcode without re-downloading, provisions runtimes, runs the §10.4 capture. |
| `cirrus.yml.example` | **Cloud path** — copy to repo root as `.cirrus.yml` to capture on Cirrus-hosted Tart VMs (no local Mac). |

## How a version reuses the image's Xcode (no download)

`13_capture_version.py` reuses an Xcode already under
`/Applications/Xcode-<ver>.app` instead of running `xcodes install`
(`13_capture_version.py:468`). Cirrus images ship Xcode as the *selected*
default (often `/Applications/Xcode.app`), so `capture-runner.sh` symlinks the
versioned name to it. The orchestrator then finds it "already installed" and
captures with zero download.

## Run it — locally on a Mac

```sh
brew install cirruslabs/cli/tart sshpass rsync
cd sweetpad-lib
ci/tart/capture.sh 26.5.0           # clone image, capture, pull oracles back
git status                          # only real behaviour deltas — no /Users churn
```

Then triage + recalibrate floors + commit per [`DOCS.md` §10.5–10.8](../../DOCS.md).

## Run it — in the cloud (Cirrus, no local Mac)

```sh
cp ci/tart/cirrus.yml.example ../.cirrus.yml   # repo root
```

Connect the repo to [Cirrus CI](https://cirrus-ci.org), trigger the
`capture-<ver>` task manually, download the `fixtures`/`xcspec` artifacts,
commit them.

## Validating across versions needs NO Xcode

Scoring the resolver against the **already-captured** majors (15.4 / 16.4 /
26.5) reads only the committed `fixtures/` + `xcspec-cache/` — that's what
`.github/workflows/sweetpad-lib.yaml` does on a plain `macos-latest` with
nothing installed. Tart is only for *(re)capturing* a version; the multi-
version test matrix is committed data.

## Refreshing / adding a version

1. Find the image tag at
   [cirruslabs/macos-image-templates](https://github.com/cirruslabs/macos-image-templates)
   and add/update the version entry in `images.json`.
2. `ci/tart/capture.sh <ver>` (or the Cirrus task).
3. Follow the runbook: triage (§10.5), floors (§10.6), drop the old minor when
   refreshing a major (§10.7), embedded catalog (§10.7b), commit (§10.8).

### Baking an image (a minor Cirrus doesn't publish)

For an Xcode minor with no published `*-xcode` image (the §5.4 corpus-wall
case for old majors), bake one once from a base image and reference it in
`images.json`:

```sh
tart clone ghcr.io/cirruslabs/macos-<release>-base bake
tart run bake &                      # GUI; sign in once if needed
# inside: xcodes install <ver> --experimental-unxip --empty-trash
#         sudo xcodebuild -license accept && sudo xcodebuild -runFirstLaunch   (§10.2)
tart stop bake
# push to your registry, or keep it local and pass --image to capture.sh
```

The license + first-launch sudo steps (`DOCS.md` §10.2) happen **once, when
baking**, instead of on every capture run.
