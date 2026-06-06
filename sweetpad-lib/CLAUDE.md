# sweetpad-lib

A Rust resolver for Xcode `pbxproj` / `xcconfig` build settings, snapshot-tested
against real `xcodebuild -showBuildSettings` captures. See `PLAN.md` for phases
and `COVERAGE.md` for the test-case matrix.

## Dependencies

Keep them minimal — but *minimal*, not *zero for its own sake*. Hand-roll the
**project-domain** formats Apple invented and no crate handles well (OpenStep
`pbxproj`, `xcconfig`, the DerivedData path hash, the binary catalog cache): that
parsing *is* the library's value, and owning it keeps us exact. Do **not**
reinvent well-known, standardized formats — JSON, XML, and the like — where a
mature crate is effectively part of the ecosystem's std (e.g. `serde_json` for
the BSP server's JSON-RPC). The Node runtime is feature-gated (`node`) because
it's heavy and platform-specific; a small pure-Rust utility crate for a standard
format does not need that ceremony.

The corpus tracks the **latest non-beta minor of each Xcode major**. When asked
to update/refresh a version (e.g. bump 26.x to the newest 26.minor, or add a
major), follow **`UPDATING_XCODE_VERSIONS.md`** — a step-by-step runbook covering
install, the sudo-only steps for a newer-than-system Xcode, the per-platform
simulator capture, dropping the old version + repointing its hardcoded paths, and
the gotchas. Surface the human-required steps (the `xcodes` 2FA sign-in, and the
two `sudo` commands when the new Xcode is newer than the system one) up front.

## Investigating how a build setting behaves

When you need to understand how a particular build setting resolves — its
default, what gates it, how it couples to other settings, or why `xcodebuild`
emits a value we don't — use every source available, in roughly this order:

1. **Apple's cached xcspecs** under `xcspec-cache/xcode-<ver>/` — the
   authoritative local source for documented defaults and per-product-type
   rules (e.g. `DarwinProductTypes.xcspec`, `macOSProductTypes.xcspec`).
2. **The corpus oracles** — `fixtures/<slug>/.../build-settings/*.json` are real
   captured outputs; correlate values across product types, configs, and
   platforms to derive a rule empirically.
3. **The internet.** It is fine — and encouraged — to search the web (Apple
   developer documentation, the Apple Developer Forums, release notes, WWDC
   notes) to find out how a setting works when the local sources don't fully
   explain it. Confirm whatever you read back against the xcspecs and the
   corpus before encoding it.

Prefer a rule grounded in the xcspec + verified against the corpus over a guess.
When a value genuinely depends on a build-system heuristic that isn't a function
of any input we can see, document that in the code rather than over-fitting.

## Validating resolver changes

`cargo test` runs the unit tests plus `tests/corpus_oracle.rs`, which scores the
full pipeline against every capture. Watch the per-key "systematic mismatches"
tally — it isolates genuine value bugs from path-geometry noise (the
exact→canonical→structural tiers already absorb `$HOME`/DerivedData-hash/
project-root drift, so those are not resolver defects).
