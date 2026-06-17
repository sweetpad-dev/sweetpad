# sweetpad-lib

A Rust resolver for Xcode `pbxproj` / `xcconfig` build settings, snapshot-tested
against real `xcodebuild` captures, plus a compiler-args generator and a BSP
server for sourcekit-lsp.

**All documentation lives in `DOCS.md`** — overview, corpus, oracles, coverage
matrix, roadmap, and the Xcode-version runbook. Key sections for an agent:

- **Principles (read first):** `DOCS.md` §3 — dependency policy (hand-roll
  Apple's project-domain formats, never reinvent standard ones like JSON),
  minimum abstraction, fixture-driven tests, and the grounding order for any
  build-setting investigation: **xcspec → corpus → web**, then document
  irreducible heuristics in code instead of over-fitting.
- **Validating changes:** `cargo test` scores the full pipeline against every
  capture. Judge correctness by the **structural %** and the per-key
  "systematic mismatches" tally — not the geometry-capped exact % (`DOCS.md`
  §5.3). Re-run all versions after every change; ratchet floors.
- **Updating/refreshing an Xcode version:** follow the runbook in `DOCS.md`
  §10. Surface the human-required steps up front (the `xcodes` 2FA sign-in,
  and the two `sudo` commands when the new Xcode is newer than the system
  one). Policy: latest non-beta minor of each major. For **byte-reproducible**
  captures in an isolated Tart VM (no Xcode on your Mac), use `ci/tart/`
  (`DOCS.md` §10.10) — it pins the host identity so recaptures show only real
  deltas, not `/Users` path churn.
- **Known irreducibles** (`ENABLE_DEBUG_DYLIB`, the 15.x arch-reporting
  family): `DOCS.md` §6.2 — do not re-investigate without new evidence.
- **Open work:** `DOCS.md` §11 (correctness roadmap + audit follow-ups).
