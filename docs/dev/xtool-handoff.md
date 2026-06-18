# xtool / build-backends — handoff (continue on macOS)

Snapshot for picking this up on a Mac. Branched from the Linux work at the
commit that added the macOS-builds-the-SDK CI iteration.

## What's done (and verified on Linux)

- **Pluggable build backends** in the Rust CLI (`sweetpad-lib`):
  - `src/cli/backend.rs` — `BuildBackend` trait + registry + `select`
    (precedence: `--backend` / `SWEETPAD_BACKEND` > per-project config `backend`
    > auto by container). Backends: `xcodebuild`, `swiftpm` (native), `xtool`.
  - `src/cli/plan.rs` — normalized `BuildPlan` (identity from the in-process
    build-settings resolver; **source/dependency graph** from the pbxproj
    reader). Surfaced by `sweetpad build plan [--json]`.
  - `src/cli/xtool.rs` — the xtool backend: `generate` writes a `.library`
    `Package.swift` + minimal `xtool.yml`; `build` runs `xtool dev build`.
  - `sweetpad build generate --backend xtool` and `… build start --backend xtool`
    wired in `src/cli/commands/build.rs`.
- **358 lib tests pass**, `cargo clippy`/`fmt` clean.
- **CI**: `.github/workflows/xtool-linux-build.yaml` — generator runs green on
  Linux with no Mac (jobs verified repeatedly).
- Design docs: `docs/dev/build-backends.md`, `docs/dev/xtool-linux-build.md`.

## The one remaining blocker

`xtool dev build` on Linux needs a **Darwin Swift SDK** bundle in
`~/.swiftpm/swift-sdks`. xtool builds that **from an `Xcode.xip`**
(`xtool sdk build`). Hosted CI has `Xcode.app`, not a `.xip`, so the cross-build
can't go green there. The last CI iteration tries to build the SDK on the macOS
runner and pass the bundle to Linux — its log will show whether
`xtool sdk build` accepts an installed `Xcode.app` or strictly a `.xip`.

## Next steps on a Mac (where this gets easy)

1. **Validate the generated `Package.swift` template** against reality:
   ```sh
   xtool new Demo            # inspect the real Package.swift + xtool.yml
   ```
   Then diff against what `sweetpad build generate --backend xtool` emits
   (`src/cli/xtool.rs::render_package_swift` / `render_xtool_yml`) and adjust.
   Open question: xtool's template uses a `.library` target (no external dep) —
   confirm, and confirm whether `@main` App needs anything special.
2. **Confirm the SDK interface**: `xtool sdk build --help`, `xtool sdk --help`.
   If it builds from the local Xcode (no `.xip`), wire that into the macOS CI job
   (already scaffolded). If a `.xip` is required, decide how to supply one
   (secret/URL) or keep the cross-build as an informational CI job.
3. **Real end-to-end on the Mac**:
   ```sh
   cd sweetpad-lib && cargo build --bin sweetpad
   ./target/debug/sweetpad --project <App.xcodeproj> build generate --backend xtool
   ./target/debug/sweetpad --project <App.xcodeproj> build start --backend xtool
   ```
4. Then resume the phased plan in `docs/dev/build-backends.md`
   (Bazel generator next; `doctor` integration; run/install via
   `resolve_product`).

## Build / test

```sh
cd sweetpad-lib
cargo build --bin sweetpad
cargo test --lib
cargo clippy --all-targets && cargo fmt --check
```
