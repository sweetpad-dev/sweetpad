# Updating Xcode versions in the corpus

Runbook for the agent asked to **refresh a major to its latest minor** (e.g.
`26.0.1` → `26.5`) or **add a new major**. Policy: keep the **latest non-beta
minor of each major** — never a beta. A refresh = capture the new minor, then
drop the old one entirely. This was last done for the 26.x refresh (26.0.1 → 26.5,
commit `ea02daf`); read that diff + PLAN.md "26.x refresh" for a worked example.

Most of this is autonomous. The **only** steps a human must do are flagged
**[HUMAN]** — surface them early and wait.

---

## 0. Decide the target version

- Latest non-beta of the major: `xcodes list | grep '^<major>\.' | grep -viE 'beta|rc'`
  and take the highest. (As of 2026-05 the majors that run on macOS 26 are 26, 16,
  15; Apple jumped 16 → 26, no 17–25.)
- The canonical version string is the **`/Applications/Xcode-<ver>.app` folder
  name** (e.g. `26.5.0`). The numbered scripts resolve `--xcode <ver>` against it.

## 1. Install — into `/Applications`, NOT `.xcodes/`

```
xcodes install <ver> --no-superuser --experimental-unxip --empty-trash
```
- **[HUMAN, one-time] `xcodes` Apple-ID sign-in** if the session isn't cached:
  `xcodes signin` (one 2FA code). Probe first: `scripts/13_capture_version.py
  --check-auth`. The install streams ~12 GB and unxips in place (the final
  `Xcode-<ver>.app` is only ~3.5 GB — SDKs are bundled, platforms download later).
- **Install to `/Applications`** (the default). `discover_installed_xcodes()` —
  used by every numbered script and the orchestrator — only scans `/Applications`.
  If an app lands elsewhere (e.g. the orchestrator's `--xcodes-dir .xcodes`),
  `mv` it to `/Applications` (same volume → instant; the license is version-keyed,
  not path-keyed, so moving is safe).
- Free disk first if needed (a capture wants ~50–60 GB with runtimes). Reclaim:
  delete old runtimes (`xcrun simctl runtime delete all`, ~8 GB each) and the
  Xcode app you're about to drop. Build settings **don't depend on runtime
  version**, so old runtimes are disposable.

## 2. License + first launch — **[HUMAN, sudo]** *only if newer than the system Xcode*

If `<ver>` is **newer** than the system-licensed Xcode (`defaults read
/Library/Preferences/com.apple.dt.Xcode IDEXcodeVersionForAgreedToGMLicense`),
the `--no-superuser` install skipped license + first-launch, and `xcodebuild`
will refuse everything (first a license error, then a
`CoreSimulator`/`IDESimulatorFoundation` plugin-load error). These two writes go
to root-owned `/Library`, so they need sudo — **you can't; ask the user to run**:

```
sudo DEVELOPER_DIR="/Applications/Xcode-<ver>.app/Contents/Developer" xcodebuild -license accept
sudo DEVELOPER_DIR="/Applications/Xcode-<ver>.app/Contents/Developer" xcodebuild -runFirstLaunch
```

Verify after: `DEVELOPER_DIR=.../Developer xcodebuild -showsdks` lists SDKs with no
error. If `<ver>` is equal-or-older than the system Xcode, **skip this** — its
license covers them and the capture is fully sudo-free.

## 3. Provision simulator runtimes (sudo-free)

Per platform the corpus uses (iOS, tvOS, watchOS, visionOS):
```
DEVELOPER_DIR=/Applications/Xcode-<ver>.app/Contents/Developer xcodebuild -downloadPlatform iOS
# repeat for tvOS watchOS visionOS  (each ~8 GB)
```
Cycle them (download → capture → `xcrun simctl runtime delete`) if disk is tight;
otherwise hold all four. **Boot one device to warm CoreSimulator** before
capturing, else `-showdestinations` races (see Gotchas):
```
DEVELOPER_DIR=.../Developer xcrun simctl boot "iPad (A16)"   # any device for the runtime
```

## 4. Capture the full corpus

The orchestrator drives steps 04/02/03/07–12 under the right `DEVELOPER_DIR`:
```
python3 scripts/13_capture_version.py --versions <ver> \
  --subset alamofire,kingfisher,ice-cubes,netnewswire,tuist-fixtures \
  --no-runtime --keep --force --min-disk-gb 5
```
- `--no-runtime` skips the smoke *builds* (03) — they aren't scored; metadata (02)
  still runs and captures whatever destinations the installed runtimes offer.
- `--keep` (don't auto-teardown), `--force` (re-capture; **this now forwards to the
  sub-scripts** — it didn't before `ea02daf`).
- **Verify simulator destinations actually landed:**
  `find fixtures -path '*xcode-<ver>*build-settings*' -name '*Simulator*' | wc -l`
  should be large. If it's 0 (CoreSimulator race), re-run 02 directly with the
  runtimes warm — this is the reliable fallback:
  ```
  for s in alamofire kingfisher ice-cubes netnewswire tuist-fixtures; do
    python3 scripts/02_capture_metadata.py --xcode <ver> --project $s --force
  done
  ```
  (02 self-sets `DEVELOPER_DIR` via `--xcode`.)
- If synthetic-override (`07`) produced nothing, run it directly **with
  `DEVELOPER_DIR` exported** (07 does NOT self-set it):
  ```
  DEVELOPER_DIR=/Applications/Xcode-<ver>.app/Contents/Developer \
    python3 scripts/07_synthetic_overrides.py --base alamofire --xcode <ver>
  ```

Confirm every oracle source exists for `<ver>` before dropping the old one:
per-target, project-defaults, scheme build-settings (incl. simulators), `_synthetic/`
(07), `_synthetic-xcconfigs/` (11), `_global/` (08), `_xcconfig_resolution` (10),
and `xcspec-cache/xcode-<ver>/`.

## 5. Triage & fix

```
cargo test                                    # new version gets a structural>=98 safety guard
ORACLE_ONLY_VERSION=<ver> cargo test --test per_target_oracle -- --nocapture   # systematic tally
DEBUG_DIFF_KEY=<KEY> ORACLE_ONLY_VERSION=<ver> cargo test --test per_target_oracle -- --nocapture
python3 scripts/14_compare_versions.py <old> <ver>    # what changed across the bump
```
Per-target is the cleanest oracle. Ground every fix xcspec → corpus → web
(CLAUDE.md). Expect mostly version-echo deltas (deployment targets, new setting
keys) that need no resolver work. Version-specific **hardcoded** values that may
need refreshing:
- `src/project.rs` `SWIFT_EMIT_CONST_VALUE_PROTOCOLS` — AppIntents list is
  SDK-injected (not in any xcspec); update the snapshot to `<ver>`'s value if it
  appears as a systematic mismatch (it grows + re-sorts per SDK; 26.x-only key).

## 6. Codify per-version floors

Each oracle has a `version_floor(version)` (corpus uses `CORPUS_FLOOR_<n>` consts).
Add/rename the arm for `<ver>` from the observed numbers minus ~1 pt (structural
floored at 98; lower only with a documented irreducible, like 15.x arm64e arch
reporting). Run `cargo test` and read the printed `[<oracle> <ver>] exact=.. canon=..
struct=..` lines to harvest the numbers.

## 7. Drop the old version (when refreshing a major)

```
git rm -r fixtures/*/xcode-<old> xcspec-cache/xcode-<old>
rm -rf fixtures/*/xcode-<old> xcspec-cache/xcode-<old>     # untracked build artifacts
```
Then **repoint every hardcoded `<old>` reference** — `cargo test` failures pinpoint
each one. Known spots (repoint `xcode-<old>` → `xcode-<new>`):
- Unit tests: `src/{project,build_context,xcspec,workspace,bplist,scheme}.rs`.
- Integration tests: `tests/*.rs` (NOT `tests/common/mod.rs` — its `26.0.1` refs are
  canonicalizer *test data* / comments, version-agnostic; leave them).
- Floor tables: rename the `version_floor` arm in all five oracles (step 6).
And these **version-specific assertions / slugs**:
- `src/bplist.rs`: `CanonicalName == "macosx<NN.N>"` → new macOS SDK version.
- `tests/xcspec.rs`: `macosx<NN.N>` (twice) + the "holds N xcspec files" comment count.
- `tests/scheme_planner.rs`: the oracle filename's destination slug
  `OS<NN.N>_iPad-A16` → the new simulator OS (`find fixtures -path
  '*xcode-<new>*Simulator*' -name '*.json' | head` shows the real slug).

`grep -rn 'xcode-<old>\|macosx<old SDK>\|OS<old sim OS>' src/ tests/` should come
back clean except `tests/common/mod.rs` test data.

## 7b. Refresh the embedded defaults catalog (when the latest version changes)

`build-settings` with no `--xcspec-root` resolves against a catalog baked into
the binary (`src/catalog_embedded.bin`), which tracks the **newest** captured
Xcode. When you add a new major or bump the latest minor, regenerate it:

```
# point DEFAULT_VERSION in examples/gen_embedded_catalog.rs at the new version, then:
cargo run --release --example gen_embedded_catalog
git add src/catalog_embedded.bin
```
It prints the assignment/product-type counts; commit the regenerated blob. (No
need to touch it when only refreshing an *older* major.)

## 8. Green, docs, commit

- `cargo test` (all versions green), `cargo fmt`, `cargo clippy --tests`.
- Update **COVERAGE.md** "Xcode versions captured" table and **PLAN.md** (version
  status + a short outcomes note). Judge correctness by **structural %** + the
  systematic-mismatch tally, not the geometry-capped exact %.
- **[HUMAN approval]** Show the commit message and wait (per the user's commit
  rules: terse, no co-author, no test plan, don't push unless asked).

## 9. Cleanup (per the user's "remove after usage")

```
xcrun simctl shutdown all; xcrun simctl runtime delete all   # remove this run's runtimes
rm -rf /Applications/Xcode-<old>.app                          # drop the replaced Xcode
```
Keep the new Xcode app (current representative) + the other majors (15.4/16.4).

---

## Adding a NEW major (vs refreshing) — the corpus wall

Latest-release projects only open in recent Xcode. For an **older** major you must
pin **era-appropriate refs**: a project's `objectVersion` / Swift-tools must be
≤ what that Xcode supports (alamofire@77 needs Xcode 16+, netnewswire 76,
ice-cubes Swift-tools 6.2 needs Xcode 26; kingfisher@54 + tuist's objectVersion-55
generated projects open in 15.4). Re-checkout the corpus project at an era-suitable
tag before capturing (the shared single-clone model breaks here). An older major
is also equal-or-older than the system Xcode → **no sudo** (step 2 skipped).

## Gotchas (all hit during the 26.5 refresh; fixes are in the repo)

- **Newer-than-system Xcode ⇒ two sudo steps** (license + runFirstLaunch). Only
  blocker that isn't autonomous. Equal-or-older ⇒ fully sudo-free.
- **Install to `/Applications`** — sub-scripts don't see `.xcodes/`.
- **`-showdestinations` simulator race** — under the orchestrated run it
  intermittently omits concrete simulators (lazy CoreSimulator device creation),
  yielding macOS-only captures. Fixed: `02_capture_metadata.py`
  `augment_with_simulators()` derives a representative simulator per supported
  platform from `simctl` (reliable). Still: boot a device first to warm it, and
  run 02 with `--force`.
- **Orchestrator `--force`** now forwards to the numbered sub-scripts (was a
  silent no-op before `ea02daf`, so re-captures kept stale outputs).
- **`07` doesn't self-set `DEVELOPER_DIR`** — export it when running 07 directly,
  else it uses the `xcode-select`ed Xcode and targets a missing/old runtime.
- **`--no-runtime`** only skips the *builds* (03); 02 still captures simulator
  destinations if their runtimes are installed.
