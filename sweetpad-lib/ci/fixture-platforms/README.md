# Standalone Apple platform probes

These three SwiftUI apps test tvOS, watchOS, and visionOS simulator builds.
`WatchProbe` uses the current single-target watchOS application format.
The same sources compile with Xcode and the Linux Swift Build backend.

For example, from the repository root:

```bash
sweetpad build --project sweetpad-lib/ci/fixture-platforms/PlatformProbe.xcodeproj \
  --scheme TVProbe --destination 'generic/platform=tvOS Simulator' \
  --configuration Debug --json --non-interactive
```

Use `WatchProbe` with `watchOS Simulator` or `VisionProbe` with `visionOS Simulator`.
The generated project is checked in. Run XcodeGen with `project.yml` to regenerate it.
Linux setup imports SDK data from the user's Xcode; no Apple SDK files are distributed here.

These are compile/link probes. Runtime checks require the corresponding simulator runtimes on a Mac.
