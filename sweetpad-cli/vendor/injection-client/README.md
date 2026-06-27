# Bundled hot-reload injection client

`sweetpad app run --hot` injects an InjectionNext client into the simulator app,
then drives it as a server over `:8887` (CLI_DESIGN §9d). This directory builds
that client **once** and vendors it so the `sweetpad` binary can embed it — no
git clone, no per-Xcode `xcodebuild`, works offline.

## How it stays simple (no fork, no strip)

InjectionNext's *Xcode* `InjectionBundle` target links XCTest + Quick + Nimble for
its test-reload feature — the only Xcode-versioned dependencies in the client. Its
*SPM* product carries none of them, and SPM defines `SWIFT_PACKAGE`, which the
engine's `canImport(Nimble)` build sentinel keys on. So building the SPM product
yields an **XCTest-free** client that depends only on ABI-stable OS/runtime libs,
which makes a single prebuilt **portable across Xcode versions**.

`Package.swift` here is a thin wrapper that depends on pinned upstream
InjectionNext and re-exposes its product as a `.dynamic` library (upstream ships
only static ones), loadable via `DYLD_INSERT_LIBRARIES`. Nothing in upstream is
patched.

## Rebuilding

```sh
./build.sh        # macOS + Xcode + network; verifies the result is XCTest-free
```

This produces `prebuilt/SweetpadInjectionClient.dylib` (fat arm64 + x86_64), which
`build.rs` embeds into the `sweetpad` binary via `include_bytes!`. The dylib is
**not committed** (it's gitignored): CI and the release CLI scripts run `build.sh`
before `cargo build`, and any build without it falls back to `InjectionNext.app`
at runtime. After bumping the `revision` pin, just re-run `build.sh`.

CI validates the embedded client end-to-end (real injection on a simulator,
Xcode 16 + 26) via `ci/hot-reload-e2e.sh`.
