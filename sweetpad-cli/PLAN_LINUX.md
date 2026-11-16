# Building Xcode projects on Linux

Status on 2026-11-10. The initial Linux engine merged in PR #346.
SDK export, setup/import, and Linux doctor shipped in CLI 0.1.19.
The follow-up audit below tracks work after that checkpoint. See
[`ci/LINUX_PROJECTS.md`](../sweetpad-lib/ci/LINUX_PROJECTS.md) for current app coverage and limits.

`sweetpad build` has a Swift Build engine, the default where there's no
Xcode. It translates the workspace into the PIF that Apple's open-source
Swift Build takes as input, then runs `swbuild` against a Darwin Swift SDK
that xtool extracts from the user's own Xcode.app. Xcode tools that Linux
doesn't have are replaced by the sweetpad binary itself, started under the
tool's name. lld links. Nothing calls Xcode or xcodebuild at build time.

The patched corpus builds 127 of 128 current schemes with Swift 6.4.2 and SDK 27.2.
Of the original 35 failures, 33 now pass on both hosts. One obsolete watch extension
scheme merged into its app, and the deliberately missing package remains a negative test.
Test-only schemes use `test build`; the summary records each action.
Wikipedia, Mastodon, Swiftfin, Flutter platform_design, the React Native template,
and the Capacitor conference app build on both hosts. All render in the simulator.
Wikipedia requires its declared app-group entitlements in signed simulator copies.
The MMKV example builds on both hosts with its scene-lifecycle fixture patch.
The Linux header-merge stage corrects two recursive generic bridge helpers without the former fixture shim.
Both products pass seven storage checks twice, including persistence across process relaunch.
Release cleanup remains deferred. Physical-device checks now pass for the seven larger app fixtures.
Wikipedia and Mastodon use isolated app-local startup fixtures without extensions or unsupported capabilities.
All checks reuse an existing certificate and wildcard profile. No account resources were created.
Flutter Release uses an iOS AOT compiler built from pinned Dart sources on Linux.
The latest iOS 27.2 checks pass four automatic test-plan executions and fresh MMKV, Flutter, and React Native launches.
Direct Linux Wi-Fi installation, launch, process checks, and startup logs also pass through RemotePairing without a Mac relay.

## ARM64 release gate

The release ships static ARM64 and x86_64 archives, Debian packages and a container image.
An archive contains the CLI, DAP alias, a README, the device notes and licenses, with checksums beside it.
Apple SDKs, signing material, and build caches remain private.
A clean Ubuntu container validates installation without Rust, a developer home, or a source checkout.
A separate isolated SDK check builds the installed sample from an empty home and fresh DerivedData.
It exposes only the explicit read-only compiler, build engine, macro binary, and private SDK prerequisites.
The macro launcher restores Swift runtime lookup after the compiler strips PATH and LD_LIBRARY_PATH.

Pull requests run ARM64 release packaging and the clean-container check on a hosted runner.
Tag releases publish both macOS and Linux archives only after their packaging jobs pass.
Manual release runs can select only Linux and invoke the private SDK integration workflow.
That workflow requires a provisioned Apple-hardware runner and never uploads SDK or signing files.
A dry run does not publish a release or update Homebrew.
The release remains experimental. Apple test execution and simulator operation require macOS.
Linux device debugging remains unsupported.
Optional `.omo` work and dependency-size cleanup remain deferred.

Items marked *(optional)* can wait until a project needs them.

## Linux x64 host support, issue #349

- [x] Commit the previous audit fixes (`0f88efc5`).
- [x] Add `linux-x64` packaging and matching-host install checks alongside ARM64.
- [x] Accept x86_64 in setup, SDK import, and doctor; pin the x64 xtool checksum.
- [x] Record SDK host architecture and detect incompatible tools before reuse.
- [x] Add both host architectures to release packaging and private SDK CI selection.
- [x] Validate ARM64 iPhone and simulator output with the x64 compiler, driver, CLI, and macro plugin.
- [x] Finish the x64 clean-install and regression evidence; keep 0.1.18 documentation accurate until a new release.

Validation used x86_64 Ubuntu 24.04 under container emulation on Apple hardware,
Swift 6.4.2, Swift Build 6.4.0, and the pinned x64 Darwin tools. The packaged x64 CLI
built ARM64 iPhone and simulator apps with zero warnings/errors, including `@Entry`
macros. Mach-O headers confirm ARM64 and the correct iOS/iOS Simulator platforms.
The device build used the default target architecture, without an ARCHS override.
Both package architectures pass clean install and wrong-host rejection checks.
Evidence: `corpus/_linux-followups/validation/issue-349/` (gitignored).
A full fresh x64 bootstrap also passes in an isolated Ubuntu 24.04 machine.
Its initial home has no Swift, xtool, Swift Build, SDK, or source caches. Setup
PGP-verifies the compiler download, imports a new SDK (`reused: false`), and
builds the driver and macro plugin from source. Doctor reports zero required
problems. A setup rerun keeps the same driver and SDK receipt; a subsequent SDK
import reports reuse. Independent ARM64 iPhone and simulator builds pass without
changing Ubuntu's Dash shell. No signing material or physical device was used.
Fresh evidence: `corpus/_linux-followups/validation/fresh-x64-bootstrap-2026-11-11/`.

The existing Flutter AOT and full hybrid-app validation remains ARM64-specific.
The x64 private CI path covers package installation, core app builds, test manifests,
and package linking; it does not imply full hybrid-framework runtime coverage.

## Self-contained setup

Goal: make setup simple and SweetPad as self-contained as possible. Setup
installs only three things itself: system packages, the pinned Swift
toolchain, and the Apple SDK. Everything else ships inside the SweetPad
package. On Ubuntu or Debian the end state is:

```
sudo apt install sweetpad        # Swift 6.4.2 arrives as a dependency
sweetpad setup --xcode 27.2      # downloads Xcode and imports the SDK
```

| Piece | Today | Target |
| --- | --- | --- |
| Patched Swift Build (`swbuild`) | compiled on the host, about 14 minutes | in the SweetPad package |
| Macro plugin (`sweetpad-macros`) | compiled on the host | in the SweetPad package |
| Swift 6.4.2 toolchain | downloaded by `linux-setup.sh` | a dependency from our APT feed; setup downloads it elsewhere |
| Darwin SDK bundle | `xtool sdk build`, then Python fix-ups | written by the CLI |
| `.xip`, `.aea` and SDK export reading | Python, plus C codecs compiled at import | the CLI |
| Setup steps | `linux-setup.sh` behind a Python shim | the CLI |

Each phase ships in its own release. Setup no longer needs `git`, xtool,
Python or a sourced environment file at the end; device commands still use
`pymobiledevice3`. The other directions considered (linking Swift Build into
the binary, shipping Swift inside the SweetPad package) are dropped.

### Phase 1: Swift Build and the macro plugin in the package

Measured in the ARM64 VM: stripped `swbuild` is 26 MB,
`SWBBuildServiceBundle` 20 MB and `sweetpad-macros` 8.5 MB, 20 MB gzipped
together, against a 6 to 10 MB package today. `swbuild` loads 16 Swift
libraries from the toolchain, which setup installs anyway.

- [ ] SC01: Build the patched `swbuild`, its build service and `sweetpad-macros` per architecture in release CI, on Debian 12 for the oldest glibc, against the pinned toolchain's runtime.
- [ ] SC02: Ship them in the tarball, `.deb`, APT feed and container image, with Swift Build's license and NOTICE and a note of our changes.
- [ ] SC03: Find them beside the CLI binary; keep `SWEETPAD_SWBUILD` and `SWEETPAD_MACROS` as overrides. Confirm Swift Build's spec resources are embedded or ship them.
- [ ] SC04: Run the packaged binaries in the public install checks on Ubuntu 24.04, Debian 12 and 13, and Arch.
- [ ] SC05: Drop the clone, patch and build steps from setup, and the `~/src/swift-build-*` caches.

### Phase 2: replace xtool

xtool 1.21.0 (`SDKBuilder.swift`, MIT) does five things. It downloads a
Darwin linker toolset from `xtool-org/darwin-tools-linux-llvm` 1.1.0 and
wraps `ld64.lld` so `-r` becomes a `libtool` archive. It downloads the
OpenAppleMacros server and stub plugin libraries. It copies a trimmed
Developer folder, moving XCTest and Testing, linking the simulator plugin
folder and removing Clang's include folder. It patches each platform's
`Info.plist` (`SWIFT_RESOURCE_DIR`, `CLANG_RESOURCE_DIR`,
`ALTERNATE_LINKER=lld`, `CODESIGN=true`). And it writes `swift-sdk.json`,
`toolset.json`, `toolset-swb.json` and `info.json`.

- [ ] SC06: In the VM, check whether the pinned toolchain's own `lld`, run as `ld64.lld`, links the corpus as xtool's toolset does. If not, build that toolset in our release CI or pin its tarball by checksum.
- [ ] SC07: Check whether Swift Build with our patches still emits `ld64.lld -r`, and whether `sweetpad-macros` covers what the OpenAppleMacros stubs provide.
- [ ] SC08: Save today's `darwin.xtoolsdk` tree with file hashes as the reference.
- [ ] SC09: Write the bundle from an unpacked Xcode.app in the CLI: layout, JSON files, platform manifests for all nine platforms, Linux Clang headers, `version.plist`, `build-machine.plist` and App Intents phrases. This replaces `linux-sdk-platforms.py` and the fix-ups after xtool.
- [ ] SC10: Match the reference apart from chosen differences; pass the corpus, device install and hybrid-app checks; bump `IMPORTER_VERSION`.
- [ ] SC11: Remove xtool from setup, `linux-toolchain.json`, `doctor` and the docs.

### Phase 3: archive reading in the CLI

- [ ] SC12: Read SDK exports, xar (`.xip`), pbzx, cpio (odc and newc), Apple Archive and AEA in Rust, with the selected paths as a typed table.
- [ ] SC13: Codecs: `flate2`, an LZMA crate and `lzfse_rust`. For LZRAVEN, test the `lzraven` crate against our conformance archives, or port the 0BSD C decoder.
- [ ] SC14: AEA with RustCrypto HPKE, HKDF and AES and `p256`, keeping the key-host rules and `--aea-key-file` formats.
- [ ] SC15: Port the traversal, link, size and checksum tests over the existing fixtures, and keep `--replace-sdk` keeping the old SDK until the new one imports.
- [ ] SC16: Add a private CI job that imports a real `.xip`, the gap #363 shipped through.
- [ ] SC17: Move `sdk export` on the Mac to the same code, then delete `linux-sdk.py`, `linux-xcode-archive.py`, `linux-archive-codecs.py`, `archive-codecs/` and their Python tests.

### Phase 4: setup in the CLI

- [ ] SC18: Steps with a `status()` and `run()`, shared with `doctor`, with progress events under `--json` and typed errors.
- [ ] SC19: Host packages through `sudo apt-get` or `pacman`, with the package lists in `linux-toolchain.json`.
- [ ] SC20: Publish `sweetpad-swift-6.4.2` on our APT feed, repackaged from the swift.org release with its notices, and make `sweetpad` depend on it. Include it in the container image.
- [ ] SC21: Elsewhere, use a matching Swift on `PATH` or from swiftly, and otherwise download and verify the pinned release.
- [ ] SC22: Find the toolchain, SDK and helpers without `linux-env.sh`; keep writing it for running `swift` by hand.
- [ ] SC23: Delete `linux-setup.sh`, `linux-host-tools.sh` and the Python shim; update `CLI_DESIGN.md`, the Linux docs and `help environment`.

## Follow-up audit, 2026-11-10

The handoff identified these candidates by reading code. An unchecked item is not a reproduced runtime failure.
References name the audited implementation. Line numbers in the handoff refer to the code before these fixes.
Close an item only after implementation checks or a reproduction establishes the result.
Device disk-image mounting and entitlement behavior need separate hardware or Mac runtime checks.
The `.omo` work and optional dependency cleanup remain deferred.

Validation evidence for this batch is retained under
`corpus/_linux-followups/validation/gap-audit-2026-11-10/` (gitignored).
The public regression fixtures and checks live under `sweetpad-lib/ci/fixture-linux-script`
and `sweetpad-cli/ci/check-linux-gaps.py`. Device routing tests use a local transport fixture;
they do not establish physical-device execution for these changes.

### Setup and diagnostics

- [x] S01: Find public setup sources beside extracted archives and mise binaries, as well as installed prefixes (`commands/setup.rs`).
- [x] S02: Treat missing localized App Intents phrases as an optional capability, and share the engine's macro lookup (`commands/doctor.rs`, `swbuild.rs`).
- [x] S03: Check the build service, Darwin librarian, zip, extra platforms, device tools, and pinned Swift Build provenance where available (`commands/doctor.rs`). Build-service and librarian checks are required; zip/usbmuxd are optional checks. Setup records the Swift Build version and patch digest beside `swbuild`, and `doctor` warns when they differ from what this sweetpad embeds ("Swift Build patches"); "Extra platforms" lists the tvOS, watchOS and visionOS platforms the SDK has. Checked in the VM.
- [x] S04: Report `--replace-sdk` for setup migration, retain useful bootstrap errors in JSON, and handle root without sudo (`linux-sdk.py`, `commands/setup.rs`, `ci/linux-setup.sh`).
- [x] S05: Test setup source discovery and Linux doctor independently of the developer machine (`commands/setup.rs`, `commands/doctor.rs`).
- [x] S06: Consolidate tool version requirements across bootstrap, export, doctor, and packaging (`commands/setup.rs`, `linux-sdk.py`, `ci/linux-setup.sh`).
- [x] S07: Document Python before setup and replace raw JSON in SDK human output (`ci/linux-release-README.md`, `commands/sdk.rs`).
- [x] S08: Upgrade an existing pinned Swift Build checkout without losing local edits; pin its package dependencies (`ci/linux-setup.sh`, Swift Build `Package.swift`).
- [x] S09: Validate the complete setup on a fresh host, separately from reusing preinstalled compiler tools. Full fresh Ubuntu 24.04 x64 bootstrap passes, including compiler download/signature, new SDK import, cold driver/macro builds, doctor, setup reuse, and ARM64 app builds. See the fresh x64 evidence above; `ci/check-linux-sdk-install.py` retains the separate package-build isolation check.
- [x] S10: Validate `doctor --target` behavior on macOS, signing credentials, and provisioning contents (`commands/doctor.rs`). Explicit --target on macOS fails with a usage error. The identity check lists each identity's kinds and warns about an expired certificate or a missing issuer. The profile check counts profiles by kind and warns when no current profile lists a configured identity's certificate.

### Build engine and command routing

- [x] E01: Route run and app install builds through the selected engine (`commands/app.rs::build_and_install`, `xcodebuild.rs::BuildPlan`).
- [x] E02: Clean Linux products without invoking xcodebuild, including purge (`commands/clean.rs`, `swbuild.rs::run`).
- [x] E03: Distinguish configuration-sourced xcodebuild flags from passthrough errors (`cli/mod.rs::xcodebuild_args`, `swbuild.rs::refuse_xcodebuild_flags`).
- [x] E04: Reproduce script sandboxing on Linux and disable unavailable sandbox-exec tasks there (`swbuild.rs::DarwinSdk::overrides`, Swift Build `TaskProducerSandboxing.swift`).
- [x] E05: Parse and run scheme pre/post actions with the selected build context (`sweetpad-lib/src/scheme.rs`). Build and Archive shell actions run in Xcode-validated order, with selected project-target settings, generated-source support, and failure propagation. A script that selects a package target gets that target's settings, resolved for the build's destination (`PLATFORM_NAME`, `BUILT_PRODUCTS_DIR` and `SUPPORTED_PLATFORMS` match Xcode's on a generic simulator build). Send Email actions are skipped with a note, as xcodebuild skips them. Test action scripts don't run, matching xcodebuild's `build-for-testing` and `-xctestrun` runs. The package-target path also fixed `package_pif::load` on Linux, which cloned `SourcePackages` with macOS-only `cp -c`.
- [x] E06: Preserve Xcode target build-tool plugin dependencies and verify package-target plugins (`sweetpad-lib/src/workspace_pif.rs`). A project target's plugin is a dependency on the plugin product, as in Xcode's PIF, so its tool builds. Package-target plugins didn't run: SwiftPM's PIF named the tool in its own build folder, which a workspace build never fills. The tool path now points at `${BUILD_DIR}/${CONFIGURATION}-<platform>`, and `sweetpad-lib/ci/fixture-build-tool-plugin`'s app that uses a plugin-generated symbol from a package builds in Debug and Release in the VM. Linux can't evaluate a project target's plugin (no `XcodeProjectPlugin`), so the build warns for each one; E24 tracks running them.
- [x] E07: Use the Darwin SDK for resolved settings, product paths, and BSP preparation (`commands/settings.rs`, `sweetpad-core/src/build_settings.rs`, `bsp/mod.rs`). `settings show` named `/Applications/Xcode.app` for `SDKROOT`, `DEVELOPER_DIR` and the toolchain; the BSP's arguments couldn't load the SDK's `Swift` module (no `SwiftShims`), and prepare tried to start `xcodebuild`. The Darwin SDK's `Developer` folder is now the developer directory off macOS, with the built-in catalog's SDKs replaced by the bundle's; editor arguments add the toolchain's resource directories, cross-import overlays and `sweetpad-macros`; prepare runs `sweetpad build` on Swift Build. Product paths already matched the engine's DerivedData. In the VM, `PackagePluginApp`'s prepare builds `Greeting.swiftmodule` where the editor's `-I` points, and its arguments type-check `App.swift` (SwiftUI and the package module) with the Linux `swiftc`.
- [x] E08: Offer runnable Linux device destinations without defaulting runtime commands to My Mac (`resolve.rs`, `simctl.rs`).
- [x] E09: Surface Linux device-discovery failures in destination listings (`commands/destination.rs::gather`).
- [x] E10: Resolve `build --on device` and device names on Linux (`resolve.rs::build_target_without_devices`).
- [x] E11: Select the platform of tvOS/watchOS/visionOS-only schemes for generic builds (`resolve.rs::build_target_without_devices`).
- [x] E12: Resolve and update packages without xcodebuild on Linux (`commands/dependency.rs`). `dependency resolve`, `update` and `add` failed to spawn `xcodebuild`. They now resolve the engine's root manifest with SwiftPM, with the checkouts in `SourcePackages`, and write moved pins back to the workspace's `Package.resolved` in Xcode's format 3 with its `originHash`. In the VM, a project using swift-collections resolves, updates 1.1.0 → 1.7.6, adds swift-algorithms and builds; Xcode 27.2's `-resolvePackageDependencies` leaves the Linux-written file byte for byte as it is.
- [x] E13: Select a Linux opener for config and build directories; report unsupported Xcode opening (`commands/open.rs`). Off macOS, `open dd` uses `xdg-open`, `open config` uses `$VISUAL`/`$EDITOR` at a terminal and `xdg-open` otherwise, and `open xcode`/`open sim` exit 5 (`tool_missing`) saying they need macOS. Before, every target ran `open`, which Ubuntu 24.04 doesn't have.
- [x] E14: Match scheme and test-plan targets by identifier and container when names collide (`swbuild.rs::selection`). The build request picks a project target by its project and pbxproj ID (`workspace_pif::project_target_guid`), falling back to the name for package entries. `sweetpad-lib/ci/fixture-name-collision` has a `Core` framework in each of two projects; its `BCore` scheme built A's `Core` before and builds B's now, as Xcode does.
- [x] E15: Isolate concurrent invocation files and update stand-in links atomically (`swbuild.rs::Invocation`, `swbuild.rs::link_tools`). A build locks its DerivedData folder (`sweetpad-build.lock`) before writing its inputs, so a second build there waits with a note; input files are replaced by rename; stand-in links are made in a staging folder renamed into place. Two builds started together in one DerivedData both succeed in the VM, the second after waiting.
- [x] E16: Invalidate resource tasks when stand-ins or SDK inputs change (`swbuild.rs::toolset`). Stand-ins live in `SweetPad/Tools/<fingerprint>`, hashed from the binary and the SDK's identity files, so their tasks' command lines change. In the VM, swapping the binary regenerated `Assets.car` (it stayed stale before) without recompiling the app's Swift; a no-op build stays a no-op.
- [x] E17: Preserve useful failure output, log paths, exit status, and child-process cancellation (`swbuild.rs`, `process.rs`). Failed build logs are now linked in human output; bubblewrap dies with its parent. Swift Build runs each task in its own process group, so a cancelled build left its tasks running; sweetpad is now their subreaper during a build and stops them all after SIGINT, SIGTERM or SIGHUP, or when `swbuild` itself is killed, then exits 130/143/129. A signal death is named ("swbuild was killed by SIGKILL", with a `journalctl -k` tip), through `bwrap`'s 128+signal code too. VM checks: SIGTERM 143, Ctrl-C 130, SIGKILL to `swbuild` 3, no process left in each; a daemon started by a successful build keeps running.
- [x] E18: Correct classic intent-definition codegen visibility, no-codegen, and synchronized-folder metadata (`sweetpad-lib/src/workspace_pif.rs`). The emitter wrote `intentsCodegenVisibility = public` for every classic intent definition and nothing for a synchronized folder's. It now follows Xcode 27's dump for each `ATTRIBUTES` combination (`ci/fixture-intents`, 0 differences), and a synchronized file takes its attributes from the exception set (checked against Xcode's dump of the probe with intent definitions added).
- [x] E19: Verify macOS cross-architecture builds and Info.plist preprocessing on Linux (`swbuild.rs::DarwinSdk::overrides`). Archives build every `ARCHS_STANDARD` architecture: the delivery fixture's universal Mac app launches as arm64 and as x86_64, and is notarized. Info.plist preprocessing failed on a host whose `cc` is GCC, which refuses the `-target` Swift Build passes. A `cc` stand-in now sends a run for an Apple target to Clang and any other to the host's `cc`. With a prefix header, `-D` definitions and an `#if` block, the Info.plist built on Linux equals Xcode 27.2's.
- [x] E20: Preserve required linker settings when projects supply OTHER_LDFLAGS on additional platforms (`swbuild.rs::Invocation`). A tvOS build with `OTHER_LDFLAGS=-ObjC`, or with an `-xcconfig` file setting `OTHER_LDFLAGS = -lz`, lost `-fuse-ld=lld` and failed to link: the engine's `$(inherited) -fuse-ld=lld` sat among the command-line settings, which a user's value replaces and Swift Build places below the xcconfig. Settings the engine adds to now go in the request's environment-config level, the top one. In the VM, the tvOS probe links with `-ObjC -fuse-ld=lld`, `-lz -fuse-ld=lld`, and `-lc++ -fuse-ld=lld` from an xcconfig `[sdk=appletvsimulator*]` setting; watchOS, visionOS and iOS builds still succeed.
- [x] E21: Warn for missing workspace members, explain path case mismatches, and map ASSET_TAGS (`sweetpad-lib/src/workspace_pif.rs`, `swbuild.rs`). A missing member is named in a warning; a `Build input file cannot be found` whose path exists under another case puts the name on disk in the tip; `ASSET_TAGS` becomes the PIF's sorted `assetTags` (`ci/fixture-asset-tags`, 0 differences from Xcode's dump), and the VM build lays out the same asset packs as Xcode.
- [x] E22: Define verbose passthrough and structured blocker reporting for Swift Build (`swbuild.rs`). `-v` prints each task's command line and the output of tasks that report no diagnostic; `-- -verbose` and `-- -quiet` set the same levels as `-v` and `-q` instead of being refused. A task whose tool isn't on the machine (`unable to spawn process 'metal'`) makes the build blocked, not broken, naming the task and the tool, and the JSON error carries the hint as `error.blocker` on both engines. Checked in the VM with a `.metal` file.
- [x] E23: A process a script phase leaves running in the background holds Swift Build's service socket (it inherits fd 18 and the service's pipes), so `swbuild` never exits until that process does; a Gradle daemon would hold a build for hours. Found while checking E17; Xcode's spawns don't leak those descriptors. Close inherited descriptors in Swift Build's Linux task spawns (`swift-build-linux.patch`). The service's descriptors reach every task because Linux has no `POSIX_SPAWN_CLOEXEC_DEFAULT`; llbuild spawns the tasks, not Swift Build. `ci/swift-llbuild-spawn-descriptors.patch` adds `posix_spawn_file_actions_addclosefrom_np` (glibc 2.34+) after the standard three and the control pipe, which moves to descriptor 3 (`LLBUILD_CONTROL_FD` follows it). In the VM a script phase that starts `sleep 60 > /dev/null 2>&1 &` held the build for 61 s; now the build takes 1 s and the `sleep` has only descriptors 0 to 2. `fixture-linux` still builds.
- [ ] E24: Run build-tool plugins on Xcode project targets. Xcode evaluates them in the IDE through `XcodeProjectPlugin` and gives Swift Build a script phase per command, with outputs in `BuildToolPluginIntermediates/<Target>.output/<Target>/<Plugin>/`; the Linux toolchain has no `XcodeProjectPlugin`. One route is a generated package whose target stands in for the project target, evaluated by `swift package dump-pif` through the plugin's `BuildToolPlugin` entry, with its custom tasks moved onto the project target; most plugins (SwiftLint, SwiftGen, R.swift) implement both entries. Inputs that differ between the two contexts (the package directory, `XcodeTarget.inputFiles`) need checking per plugin.
- [x] E25: Build every standard arch for a generic destination. Xcode builds a Debug app for `generic/platform=iOS Simulator` with `ONLY_ACTIVE_ARCH=NO`, giving `arm64 x86_64`, and exports that to scripts; the Swift Build engine keeps the project's `ONLY_ACTIVE_ARCH=YES` and builds arm64 only. Pass `ONLY_ACTIVE_ARCH=NO` for generic destinations as xcodebuild does, after checking a capture for each platform. A generic destination without `arch=` now sets the run destination's `disableOnlyActiveArch`, which makes Swift Build export `ONLY_ACTIVE_ARCH=NO`, and drops the host's one-architecture overrides as an archive does. A static library per platform, built in Debug, has the slices Xcode 27's build has: x86_64 and arm64 for every simulator and macOS, arm64 and arm64_32 for watchOS, arm64 for iOS. xcodebuild refuses `generic/…,arch=arm64`; sweetpad keeps it to one architecture.
- [x] E26: Name On-Demand Resources asset packs as Xcode does. Off Apple platforms Swift Build's `String.md5()` (`InsecureHashContext`) falls back to SHA-256, so a pack is `<bundle>.<tags>-<64 hex>.assetpack` where Xcode writes 32 hex, and the pack identifiers in `OnDemandResources.plist` differ the same way. Patch `InsecureHashContext` to use swift-crypto's `Insecure.MD5` where CryptoKit isn't available; the same hash names other Swift Build files (`*-common-args.resp`), which nothing outside the build reads. `swift-build-linux.patch` now gives `InsecureHashContext` a vendored MD5 (RFC 1321, checked against Python's `hashlib` for lengths around each padding boundary) where CryptoKit is missing, rather than a swift-crypto dependency, which Swift Build doesn't have. `fixture-asset-tags` built in the VM has Xcode 27's names: `dev.sweetpad.check.TagApp.alpha+zeta-9b88897e8524b68dc8249f7b3628e37c.assetpack` and `…level1-43d3810c065f4bf3550fac648d605fcb.assetpack`, with the same identifiers in `OnDemandResources.plist`.

### Signing, tests, and devices

- [ ] D01: Mount the developer disk image after a phone reboot or first connection before opening developer services. Needs hardware (`device_network.py::DvtProvider`).
- [x] D02: Preserve simulator entitlements through ad-hoc signing and validate Keychain/App Groups on a Mac (`swbuild.rs::DarwinSdk::overrides`, `codesign/provisioning.rs`). A simulator build is signed ad hoc as Xcode signs it, with its entitlements in `__entitlements` and `__ents_der`. For a test app with an App Group and two keychain groups, both entitlement files and both sections match Xcode 27.2's byte for byte. In an iOS 27 simulator, the app reaches its App Group container and both keychain groups, and is refused an App Group and a keychain group it doesn't list, exactly as Xcode's build is. The delivery fixture's app, widget and watch extension carry their entitlements and pass `codesign --verify --deep --strict`.
- [x] D03: Match provisioning devices, manual profiles, entitlement values, and certificate validity (`codesign/provisioning.rs`, `codesign/identity.rs`). A device build takes only a development or ad hoc profile listing its device; a named profile is checked for expiry, app ID, certificate and device; entitlement values are checked against the profile; expired identities are skipped and named as expired.
- [x] D04: Verify watchOS profile platform matching (`codesign/provisioning.rs`). A watch target takes a profile naming iOS or watchOS. Apple's App Store profiles for a watch app and its WatchKit extension list `iOS`, `xrOS` and `visionOS`, as for any iOS app.
- [x] D05: Isolate failures for individual discovered devices and retain actionable resolution errors (`device_network.py`, `resolve.rs`).
- [x] D06: Report network-action timeout context and support bounded action-specific limits (`device_network.py`).
- [x] D07: Validate the legacy device adapter version and scope log streams to the selected UDID (`pymobiledevice3.rs`). Legacy syslog receives the selected UDID. Without the network transport, the first device command checks `pymobiledevice3 version` against 11.50.0, the release the Python transport pins, and exits 5 (`tool_missing`) naming the release it found, or that none is on PATH. `doctor`'s iPhone adapter check reads the command's version in that case instead of `python3`'s module. Checked in the VM with no `pymobiledevice3` and with a stand-in that reports 4.14.16.
- [x] D08: Keep generated xctestrun paths portable across Linux and Mac hosts (`swbuild/test_manifest.rs`). Product paths were already relative; values expanded from build settings (a test plan's `$(SRCROOT)/Fixtures`) held the Linux path. The manifest is now XML and records `SweetPadSourceRoot`/`SweetPadProductsRoot` in its metadata, and `test run --xctestrun` moves those prefixes to the local project and products folders in a temporary sibling copy. A test reading `$(SRCROOT)/Fixtures` failed on an iOS 27 simulator before and passes after.
- [x] D09: Accept harmless test-plan settings such as disabled coverage, and report unsupported coverage explicitly (`swbuild/test_manifest/selection.rs`). Disabled coverage, sanitizers, Main Thread Checker and runtime-issue detection are accepted (9 of the corpus's 22 plans set `codeCoverage: false`, which failed the manifest); enabled coverage and sanitizers say they need a supplied manifest.
- [x] D10: Export signs the app again for its method, so a profile of the wrong kind is refused by name (`codesign/resign.rs`, `commands/archive/export.rs`).
- [x] D11: Verify signing-secret inheritance into build scripts and plugins (`swbuild.rs`, `codesign/identity.rs`). A script phase saw `SWEETPAD_CODE_SIGN_P12_PASSWORD` and `SWEETPAD_ASC_KEY`, since Swift Build passes its environment to every task, as Xcode does. A build now removes both and passes the password to the codesign and provisioning stand-ins in a temporary file only the user can read, deleted when the build ends. Scheme actions get neither. The same probe now sees no secret, and the stand-ins still open the `.p12`.
- [x] D12: Handle CocoaPods absolute codesign calls without requiring fixture rewrites, or document the limitation (`ci/linux-project-prepare.py`, `docs/cli/linux/react-native.md`). Documented in the Linux signing guide: CocoaPods' `Pods-<Target>-frameworks.sh` runs `/usr/bin/codesign`, which Linux doesn't have, whenever `EXPANDED_CODE_SIGN_IDENTITY` is set and signing is allowed; a `sed` after each `pod install` points it at the `codesign` stand-in on PATH, as the preparation recipe does.
- [x] D13: Verify certificate chains beyond the bundled WWDR G3 intermediate (`codesign/identity.rs`). WWDR G3 to G6, Developer ID G1 and G2, and Apple's RSA and ECC roots are bundled, each checked against Apple's root and pinned by SHA-256. A certificate from any of them signs without its `.p12` carrying the intermediate. An identity whose chain stops short of a root is refused with the missing authority's name, in place of a signature without it.

### Packaging, CI, and docs

- [ ] P01: Run ARM64 packaging on main pushes and define private SDK validation for tag releases (`linux-arm64-package.yaml`, `cli-release.yaml`). Main pushes now run package checks; private SDK tag-release gating remains open.
- [x] P02: Give Linux self-update instructions that do not require the macOS-only Homebrew formula (`commands/self_update.rs`).
- [x] P03: Replace setup resources without stale files; embed them in the standalone CLI and describe native package upgrades, PATH setup, and removal.
- [x] P04: Reject development-stamped binaries in actual tag-release packages (`ci/package-linux.py`).
- [x] T01: Correct archive --output-file in the main reference and add setup/sdk/device/doctor commands (`docs/cli/reference.md`).
- [x] T02: Mark setup convenience commands as newer than 0.1.18 and preserve the released manual setup route (`docs/cli/linux/sdk-transfer.md`, `docs/cli/linux/ubuntu-usb.md`, `880b692d`).
- [x] T03: Prevent generated shell configuration from shadowing a CI-pinned CLI (`commands/setup.rs`, `docs/cli/linux/github-actions.md`).
- [x] T04: Replace the setup script's reference to unshipped linux-build.sh (`ci/linux-setup.sh`).
- [x] T05: Align the Linux device help with the packaged venv layout and remove corpus-only paths (`LINUX_DEVICES.md`).
- [x] T06: Document engine selection and setup environment controls, test-manifest platform limits, and app-host requirements (`CLI_DESIGN.md`, CLI help, Linux guides). Engine selection and setup controls are documented. The manifest's limits (iOS only, app host for device unit tests, `TEST_TARGET_NAME` for UI tests, plan version 1, accepted and refused options) and the path move are in `CLI_DESIGN.md` and the Linux signing and testing guide.
- [x] T07: Reconcile SDK setup design, current branch/status, MMKV notes, and reference counts (`CLI_DESIGN.md`, `ci/LINUX_PROJECTS.md`, Linux reference). SDK command design, current branch, and MMKV shim notes are updated. Historical status/reference counts still need a full reconciliation. A read of the Linux docs, LINUX_PROJECTS.md and CLI_DESIGN.md against the code and this plan corrected: arm64-only Mac archives (E19 made them universal), the Ubuntu-only host and ARM64-only archive descriptions, the archive's contents (no installer or sample; setup is embedded), the retired MMKV shim, `--clean` after a stand-in change (E16), cold-checkout scheme discovery (E12), sanitizer manifests (D09), the three Swift Build patches, the export refusals, and the `880b692d` branch pointer. Version pins in install commands and dated corpus counts stay as they are.

## Engine

- [x] PIF emitter: `sweetpad pbxproj pif`, GUIDs as `xcodebuild -dumpPIF`
      derives them, tested against Xcode 27 dumps of a probe fixture and 21
      corpus projects (`b6398036`, `7284e45b`)
- [x] The CLI builds and passes its unit tests on Linux; macOS-only FFI has
      stand-ins that fail clearly (`baedae73`, `180f5091`)
- [x] `ci/linux-setup.sh` (OrbStack machine or Linux host) and
      `ci/linux-build.sh` (end-to-end check) (`ca9ba41a`)
- [x] Swift Build engine: runs swbuild and turns its messages into the
      events, diagnostics and productPath an xcodebuild build gives
      (`17c64909`, `bfbbeeb5`)
- [x] Build requests preserve the scheme's implicit dependency and parallel
      target policies. The active destination selects the SDK without changing
      the platforms of implicit dependency candidates
- [x] Framework module verification uses Swift Build's portable builtin verifier
- [x] SwiftPM packages through `swift package dump-pif`, merged into the PIF
      (`42978410`)
- [x] Cross-import overlays, through a toolset flag (`4e2a25c7`)
- [x] Apple's macro plugins: SwiftData, `#Predicate` and `#Preview` (see
      the section below)
- [x] SwiftPM: dynamic library products as frameworks in
      `PackageFrameworks`, as Xcode builds them
      (`--experimental-build-dylibs-as-frameworks`). xtool's linker makes a
      static archive for `ld -r`, so a stand-in in front of it loads those
      whole (`-force_load`); otherwise a framework made from a package
      target alone links none of its code. `corpus/_alamofire-dynamic`
      (local) embeds `AlamofireDynamic.framework` and passes its checks
- [x] SwiftPM: per-test-target products, verified with NetNewsWire AccountTests through `sweetpad test build`
- [x] Resolved remote package scheme files, including RevenueCatUI. Custom
      DerivedData paths are honored. Package-root PIF reads preserve the app graph.
- [x] Local package schemes in `.swiftpm/xcode`: Account and Articles build.
      The scheme-owning package is a root so its test products are present.
- [x] SwiftPM: the scheme Xcode makes for a local package's product builds
      the product's target, `<name>-product` in SwiftPM's PIF, when no
      project target has the scheme's name. A product shares no name with a
      module when it's made only of binary targets, like `Foo` in
      `corpus/_xcframework-app`, which builds here as with Xcode
- [x] Mach-O on Linux: Swift Build reads Mach-O files only on Darwin
      (`#if canImport(Darwin)` in `SWBUtil/MachO.swift`), and fails every
      read elsewhere. It reads a package's binary XCFramework of a framework
      to decide whether to embed it, a macOS framework from an XCFramework
      (Sparkle, in NetNewsWire's Mac app) and a watch app's Messages stub.
      `swift-build-linux.patch` defines the structures it reads in a new
      `SWBUtil/MachOStructures.swift` and names the architectures, as
      `macho_arch_name_for_cpu_type` does. `corpus/_xcframework-app` (local,
      its C framework made by `Vendor/make-xcframework.sh`) embeds a
      package's binary framework as Xcode does, and NetNewsWire's Mac app
      embeds Sparkle
- [x] Text encodings: Swift Build finds an encoding by its name only
      through CoreFoundation, so it couldn't write a Mac app's `.strings`
      files, which are UTF-16. The patch names the encodings Foundation
      converts on every platform; NetNewsWire's 91 `.strings` files come out
      byte for byte as Xcode's
- [x] Macros Tuist builds as macOS tools: its Xcode-project integration of
      packages makes a macro target a `tool` with `SDKROOT = macosx`, which
      the targets using it load with `-load-plugin-executable`. That Mach-O
      can't run here, so the macro isn't found (`swift-case-paths` in a
      Tuist fixture). The engine builds reachable tools for the host and
      rewrites plugin paths. All four Tuist macro schemes build.

## Apple's macros

Xcode expands the SDK's macros through plugins that are macOS dylibs
(`iPhoneOS.platform/Developer/usr/lib/swift/host/plugins`). Each macro in
the SDK's `.swiftinterface` files names its plugin and type in
`#externalMacro(module:type:)`, which gives the full list. In the Xcode 27
iOS SDK:

| Plugin | Macros | On Linux |
|---|---|---|
| ObservationMacros, SwiftMacros | `@Observable`, `@TaskLocal`, `@DebugDescription`, `_SwiftifyImport` | Swift toolchain |
| FoundationMacros | `#Predicate`, `#Expression`, `#bundle` | `sweetpad-macros`; the toolchain's copy names `FoundationEssentials`, which the SDK lacks |
| FoundationModelsMacros | `@Generable`, `@Guide` | xtool |
| PreviewsMacros | `#Preview`, `@Previewable` | `sweetpad-macros`; xtool's has SwiftUI views only, not UIKit views or view controllers (`KitViewMacro`, `PreviewCommonGroup`) |
| SwiftUIMacros | `@Entry`, `@Animatable`, `@State` | `sweetpad-macros`; licensed OpenAppleMacros implementations with optional-default support |
| SwiftDataMacros | `@Model`, `@Query`, `@ModelActor`, `@Attribute`, `@Relationship`, `@Transient`, `#Unique`, `#Index` | `sweetpad-macros` |
| AppIntentsMacros | the schema macros (`@AppIntent(schema:)` and the rest), `@ComputedProperty`, `@DeferredProperty`, `@UnionValue` | missing |
| TipKitMacros | `#Rule`, `@Parameter` | `sweetpad-macros`, native expansion captures |
| StateReportingMacros | `@ReportableMetadata` | missing |

xtool's server is `darwin.artifactbundle/OpenAppleMacrosServer`; the `.so`
files beside the plugins are empty placeholders. In the corpus, 5 files use
`@Model` and 13 use `@Query` (Ice Cubes probably hits this once RevenueCat
builds; not confirmed yet); none use the App Intents schema macros, TipKit
or StateReporting. Plain App Intents (the `@Parameter` property wrapper)
need no macros.

The oracle is Xcode's compiler printing each expansion
(`-Xfrontend -dump-macro-expansions`): reimplement the macro with
swift-syntax and diff the expanded Swift text.

- [x] Confirm with a real swbuild build that `@Model` fails, and how: the
      compiler can't find `SwiftDataMacros` in xtool's plugin server and
      names the module in the error
- [x] Where our plugins live: `sweetpad-cli/macros` is a SwiftPM package
      building one executable, `sweetpad-macros`, which `linux-setup.sh`
      puts on `PATH` (`SWEETPAD_MACROS` overrides it). On a host without
      Xcode, `sweetpad build` adds `-load-plugin-executable
      <plugin>#FoundationMacros,PreviewsMacros,SwiftDataMacros,SwiftUIMacros,TipKitMacros` to the
      toolset, which the
      compiler takes before the SDK's plugin server and the toolchain's own
      plugins. Without the plugin, a build fails only where one of those
      macros is used
- [x] SwiftDataMacros: `@Model`, `@Query`, `@ModelActor` and the attribute
      macros, reimplemented with swift-syntax. Each expansion matches
      Xcode's byte for byte
- [x] FoundationMacros: swift-foundation's own source (swift-6.4.0-RELEASE,
      Apache 2.0), built with `FOUNDATION_FRAMEWORK` so its expansions name
      `Foundation`
- `macros/fixtures/check.py` compares the plugin's expansions of the
  fixtures against Xcode's, recorded in `fixtures/expected` (`--record`
  re-records them; needs Xcode). `corpus/_swiftdata-app` (local) builds on
  Linux, and its `SDCHECK` lines in the simulator are identical to the
  Xcode build's: schema, unique constraints, indices, fetches, predicates,
  `@Query` and `@ModelActor`. `corpus/_swiftui-app` does the same for
  `@Entry`, `@State` and `#Preview`
- [x] xtool's gaps: `#Preview` of a UIKit view or view controller failed
      the build, so `sweetpad-macros` takes over `PreviewsMacros` whole:
      SwiftUI views (with `@Previewable`), argument previews, UIKit views
      and view controllers, and widget, timeline and Live Activity previews,
      each expansion, error and warning as Xcode's. `@Entry` and `@State`
      build and run as with Xcode; the helper types are unreachable through
      xtool's expansions. xtool's `@State` expands as Xcode's does. Its
      `@Entry` leaves out Xcode's `#_SwiftUIEntryTypeCheck`, so a class or
      closure default value builds without Xcode's warning that it
      invalidates dependents on every access
- [x] SwiftUIMacros: OpenAppleMacros implementations, including optional `@Entry` defaults.
      Licenses and source revision are recorded beside the sources.
- [ ] AppIntentsMacros, the schema macros *(optional)*
- [x] TipKitMacros: typed `@Parameter` defaults and `#Rule`, verified against native expansions.
- [ ] StateReportingMacros *(optional)*

## Merging the branch

- [x] Corpus sweep: every scheme of every corpus project, on Linux and
      with Xcode 27. See the sweep below; each failure is filed under its
      item
- [x] Kingfisher demo, Alamofire iOS Example and NetNewsWire iOS: Linux and
      Xcode builds, bundle comparisons and simulator navigation tests.
      See the real-app integration result below
- [x] Document the engine, the setup script and the stand-ins in
      `CLI_DESIGN.md` and the user docs (run the humanizer)
- [ ] Decide whether hayro and resvg (7.5 MB, 66 crates) stay in the macOS
      binary, which never uses them
- [x] Initial PR #346 merged into `main`. Follow-up changes remain separate.

### Linux products, tests, and incremental checks, 2026-11-07

- [x] Execute Linux-built hosted XCTest, logic XCTest, and Swift Testing on a Mac
      without recompilation. `test run --xctestrun` preserves the built binaries
      and previous build diagnostics. Empty test selections fail explicitly.
- [x] Generate relocatable iOS logic, app-hosted unit-test, and UI-test manifests.
      Version-1 test plans preserve named configurations, locale overrides,
      selected/skipped identifiers, quoted arguments, and build-setting expressions.
      `-- -testPlan NAME` selects a referenced plan explicitly.
      Unsupported plan policies, including sanitizers, require a supplied manifest.
      Mixed UI/logic plans pass four executions per host on the simulator,
      with matching selected manifest fields and unchanged executable hashes.
- [x] Add app/package source and resource edits, asset/localization changes,
      renames, deletions, missing-output recovery, no-op hashes, and clean controls.
      `ci/linux-parity.py` runs both backends and prebuilt runtime checks.
- [x] Compare nested bundle Info.plists, localizations, App Intents, loose text
      resources, and Mach-O load commands. Product exports replace old bundles.
- [x] Fix missing requested-icon validation, canonical locale output names,
      accent/widget color metadata, and an initial storyboard child scene.

- [x] Include SwiftNIO privacy resources with an explicit pinned manifest patch.
      Destination manifest evaluation opts in without changing host macro builds.
- [x] Compile static Icon Composer light/dark/tinted appearances, loose primary icons,
      ordered sticker packs, message-extension icons, and custom-font bundle metadata.
- [x] Embed Swift back-deployment runtimes from the extracted Apple toolchain.
- [x] Build MMKV on both hosts without the former fixture support-header shim.
      The Linux header-merge stage corrects the recursive generic bridge helpers.
      Seven native storage checks pass twice per product, including persistence after relaunch.

AccountTests now passes all 92 tests on both hosts in signed simulator app hosts.
An explicit fixture patch removes unsupported SecItemDelete query options.
Fresh Swiftfin and Ice Cubes bundles have zero findings in the checker.
Ice Cubes matches all 19 localized shortcut models after validating producer fingerprints.
Mastodon, Wikipedia, and MMKV retain optimized-model or Mach-O load-command differences.
The SwiftUI autolink fix matches 24 native linker controls. Six framework runtime checks pass on iOS 26 and 27.
Frozen fresh-source preparation and both-host builds pass for Flutter, React Native, MMKV, and Capacitor.
Wikipedia now launches on both hosts after simulator copies receive their declared app groups.
Interactive Icon Composer material effects, pre-iOS-18 execution, and Siri voice invocation remain open.
See `ci/LINUX_PROJECTS.md` for the verified scope and evidence.

### Corpus sweep, 2026-11-04

`ci/linux-sweep.py` builds every scheme of every corpus project in its
Debug configuration on Linux, and with Xcode 27 wherever Linux fails. Of
127 schemes, 76 build on Linux, 41 fail with Xcode 27 too, and 10 fail
only on Linux. The results are in `corpus/_linux-sweep` (local): a JSON
file and a log per scheme, and `summary.md` over them.

Both hosts build a scheme the same way. It builds for the first platform
it supports, of iOS Simulator, macOS, tvOS, watchOS and visionOS
Simulator; a package's scheme builds for iOS Simulator. Xcode 27 refuses
deployment targets below iOS 15, macOS 12, tvOS 15 and watchOS 9, which
Alamofire, Kingfisher and five Tuist fixtures set, so those schemes build
again with the targets of iOS 18's generation. The Xcode builds don't
sign, since Linux signs only a device build.

The failures only on Linux are filed above:

| Schemes | Cause | Item |
|---|---|---|
| 4 in a Tuist fixture | `CasePathsMacros`, built as a macOS tool | Engine |
| 2 in Tuist's Core Data fixture | `mapc` | Apple compilers |
| NetNewsWire (Mac) | Sparkle's binary, through the Mach-O reader; now its `VerifyNoBS.swift` script, which imports `Darwin` | Engine |
| NetNewsWire's `Account`, `Articles` | a package's own schemes | Engine |
| Alamofire visionOS | no visionOS platform | Build kinds |

The failures with Xcode 27 too:

- Ice Cubes 2.1.3 doesn't build with Xcode 27: RevenueCat's `PaywallColor`
  redeclares a memberwise initializer, and `Document` is ambiguous in
  `HTMLString.swift` against the iOS 27 SDK. Its packages' own schemes
  fail here before that, as NetNewsWire's do
- NetNewsWire's iOS schemes treat warnings as errors. Xcode 27's compiler
  warns about a `weak` capture in `MainTimelineModernViewController.swift`,
  and Swift 6.4 doesn't; with `SWIFT_TREAT_WARNINGS_AS_ERRORS=NO` both
  build. Here they also passed the project's type-check limits (500 and
  650 ms) only when built one at a time
- WatchKit 1 apps (`watchapp2`, `watchkit2-extension`) in a Tuist fixture
  and Alamofire: Xcode 27 no longer builds them
- Alamofire's and Kingfisher's tvOS and watchOS schemes: no platform here,
  and Xcode stops on their deployment targets, which the sweep raises only
  after a Linux failure that names one
- A Tuist workspace scheme whose test target imports a module it doesn't
  depend on, `_spm-broken` (on purpose), test-only schemes, and
  `RevenueCatUI`, a remote package's scheme that sweetpad doesn't list on
  either host

During the sweep, sweetpad passed the scheme's targets to `swbuild` by name
as well as in the build request, and `swbuild` looks names up before the
request replaces them. A package whose product and module share a name
(NetNewsWire's `CloudKitSync`, `NewsBlur`, `RSCoreResources`) failed with
"Found multiple targets named"; the request alone names the targets now,
by GUID. Swift Build's service also crashed once on Kingfisher's macOS
demo while three builds ran at once; it builds on its own.

After the Mach-O reader and text encodings were ported, the sweep ran
again on Linux with `corpus/_xcframework-app` added: 78 of 129 schemes
build. Every scheme that built before still builds, the app in
`_xcframework-app` builds, and NetNewsWire's iOS widget passed its
type-check limits this time. Its Mac app now stops at its own script.
With the `xcrun` stand-in and package product schemes, 79 build: `Foo`
joins them, and NetNewsWire's Mac app gets through `xcrun` to the
script's `import Darwin`.

### Additional real projects

The first three projects now have pinned native and Linux builds. Their bundle
and simulator results are in `ci/LINUX_PROJECTS.md`. The last three remain planned additions.

| Priority | Project | Coverage to investigate |
|---|---|---|
| 1 | [Wikipedia iOS](https://github.com/wikimedia/wikipedia-ios) | Mixed Swift/Objective-C, framework targets, localization, widgets and multiple schemes |
| 2 | [Mastodon iOS](https://github.com/mastodon/mastodon-ios) | SwiftPM dependencies, app groups, share and notification extensions, widgets |
| 3 | [Swiftfin](https://github.com/jellyfin/Swiftfin) | SwiftUI, custom macros, shared code and separate iOS/tvOS targets |
| 4 | [Firefox iOS](https://github.com/mozilla-mobile/firefox-ios) | Bootstrap scripts, bundled JavaScript, Swift packages and macros |
| 5 | [CodeEdit](https://github.com/CodeEditApp/CodeEdit) | Native macOS packaging and dependencies across several repositories |
| 6 | [WordPress iOS](https://github.com/wordpress-mobile/WordPress-iOS) | Large workspace, modules, dependency setup and API credential configuration |

The first group is complete, with the shared Wikipedia startup limitation recorded.
Flutter, React Native, and Capacitor coverage was added in the same follow-up.
Firefox, CodeEdit, and WordPress remain future coverage.

For each project:

1. Pin a commit and record the setup requirements.
2. Establish a passing Xcode build.
3. Build the same target and configuration on Linux.
4. Compare the app bundles and runtime behavior on the target Apple platform.
5. Record setup failures separately from compiler failures.

## Toolchain tools

Swift Build names these in its xcspecs. The Swift toolchain has `llvm-ar`,
`llvm-objcopy`, `docc` and `plutil`; xtool's bundle has `dsymutil` and
`llvm-libtool-darwin` (as `llvm-lib`, the toolset's librarian) in
`toolset/bin`, which the engine puts on `PATH`.

- [x] `libtool -static`: setup installs LLVM Darwin libtool. The Swift Build
      patch recognizes its real version banner, including a `libtool` symlink.
      The static-framework aggregate workspace scheme passes.
- [x] `dsymutil`: Release (`dwarf-with-dsym`) finds xtool's on `PATH`
- [x] `copypng`: copies, dropping text chunks for `-strip-PNG-text`. Xcode's
      writes Apple's CgBI PNGs, which UIKit doesn't need
- [x] `lipo`: `-create`, `-extract`, `-thin`, `-remove`, `-info`, `-archs`,
      `-verify_arch`; checked with an arm64 + x86_64 simulator build
- [x] `strip`: `llvm-objcopy` run as `llvm-strip`, which keeps the linker's
      ad-hoc signature valid; checked with `DEPLOYMENT_POSTPROCESSING=YES`
- [x] `ditto` for run scripts (XcodeGen gives a static library with Swift
      sources one), copies only
- [x] `xcrun` for run scripts: runs a tool from `PATH`, which has the
      toolchain's and the stand-ins, and answers `--find` and the `--show-sdk-*` queries from the
      Darwin SDK. A Mac script runs a tool for the host with `-sdk macosx`,
      so that runs it without `SDKROOT`; another SDK becomes its `SDKROOT`.
      The script phase in `corpus/_xcframework-app` gets Xcode's answers.
      NetNewsWire's Mac app gets through it to `VerifyNoBS.swift`, which
      imports `Darwin` and can't run here; with that script skipped, the app
      builds
- [ ] `iconutil`: `.iconset` to `.icns` for macOS icons *(optional)*
- [ ] `tiffutil`: multi-resolution TIFFs, macOS *(optional)*
- [ ] `lex`/`yacc`, `unifdef`: apt's flex, bison and unifdef in the setup
      script *(optional)*
- [ ] `clang-stat-cache`, `tapi`: turn off with build settings *(optional)*

## Apple compilers for data files

- [x] `ibtool`: XIBs and storyboards to `.nib`/`.storyboardc` for all 63
      corpus documents: iOS, macOS, watchOS and tvOS. See the coverage and
      remaining scope below
- [x] `momc`: `.xcdatamodel` to `.mom` and `.xcdatamodeld` to `.momd` with
      `VersionInfo.plist`, plus the generated Swift or Objective-C classes.
      Golden tests against Xcode's in `fixtures/momc` (`regen.py`). Fetch
      request templates, fetched properties, derived and composite
      attributes and partial index predicates need an `NSPredicate` parser,
      and stop the build with an error naming them; so do date and decimal
      minimums and maximums. The CloudKit checks and the `.omo` file (only an
      optimization) are left out
- [x] `intentbuilderc`: Swift classes for NetNewsWire's legacy intent definition,
      including String and URL parameters, responses, dynamic options and
      resolution results. Generated declarations match Xcode 27's capture in
      `fixtures/intentbuilderc`. Unsupported model features fail explicitly
- [ ] `intentbuilderc`: Xcode's private runtime metadata upgrade, localization
      and Siri registration. Compilation currently preserves the source
      `.intentdefinition` and emits a warning about this limit
- [x] App Intents metadata and NLU processors emit `Metadata.appintents`
      from compiler const values. Synthetic native captures cover primitive
      parameters, image file inputs, dialogs/snippets, enums, entities, widget
      intents, and basic App Shortcuts. Dynamic components keep separate registrations.
      Advanced schemas and static dependency registration fail explicitly. Siri
      behavior and physical-device registration remain unverified.
- [ ] `coremlc`: generate the Swift class from the `.mlmodel`; the compiled
      `.mlmodelc` format is closed, so fail clearly or compile at runtime
      *(optional)*
- [x] `mapc`: transform mappings and direct source-property expressions.
      Native archive golden and both Tuist Core Data schemes pass. Custom
      expressions and unsupported mapping kinds fail explicitly.
- [ ] `TextureAtlas`: SpriteKit `.atlas` to `.atlasc` *(optional)*
- [ ] SceneKit tools (`scntool`, `copySceneKitAssets`,
      `compileSceneKitShaders`): copy first *(optional)*
- [x] `metal`: the compiler is closed; fail clearly, maybe try the Windows
      build under Wine *(optional)*. A build that needs it is blocked and
      names the task and the tool (E22, checked with a `.metal` file); Wine
      isn't tried.
- [ ] `realitytool`, `referenceobjectc`, Skybox, App Playground catalogs:
      fail clearly *(optional)*
- [x] macOS-only legacy tools (`osacompile`, `Rez`, `ResMerger`, `mig`,
      `iig`, `dtrace`, `exutil`, `lsregister`): skip or fail clearly
      *(optional)*. A macOS target with a `.defs` file, a `.r` file or an
      AppleScript is blocked in the VM, naming the task and `mig`, `Rez` or
      `/usr/bin/osacompile` (E22's list, which also names `ResMerger`, `iig`
      and `dtrace`). A failed `lsregister` start is a note (the Swift Build
      patch). `exutil` isn't on the list, so it gets the generic "couldn't
      find" blocker.

## ibtool

The corpus has 63 IB files: 32 iOS, 26 macOS (almost all NetNewsWire), 4
watchKit, 1 tvOS; 34 are storyboards, using 121 XML element kinds. iOS nibs
use NIBArchive. Xcode 27 also writes AppKit nibs in NIBArchive format.
AppKit uses an NSIBObjectData root and separate object encoders.

There's no open-source ibtool. References:
[WinObjC's `xib2nib`](https://github.com/microsoft/WinObjC/tree/develop/tools/vsimporter/xib2nib)
(MIT, archived, iOS 9–10 era XML, about 55 UIKit class converters and a
NIBArchive writer, never checked against Apple's UIKit), the
[`nibarchive`](https://docs.rs/nibarchive) crate (reads and writes
NIBArchive; doesn't understand trailing bytes some newer files have), and
the nibsqueeze and
[Mother's Ruin](https://www.mothersruin.com/software/Archaeology/reverse/uinib.html)
format notes.

Build it against ibtool as the oracle, the way `Assets.car` was built:
golden outputs from `ibtool --compile` with Xcode's flags (a `regen.py`),
compared structurally (decode both into a graph of class and key → value,
match objects by position in the graph rather than archive order). Tiny
XIBs that change one attribute at a time show the key and encoding each
attribute produces. A checker app dumps the loaded view tree (classes,
frames after layout, constraints, text, colors, images, outlets, actions)
for a diff against Xcode's build.

Measure coverage on the corpus first. A larger public XIB set remains open:
- input: the share of (element, attribute) pairs we translate, weighted
  by how often each appears; the most frequent missing pair goes next
- output: the share of (class, key) pairs in ibtool's output we emit with
  the same value
- files that match structurally, and files that compile with no
  unsupported-element error (an unknown element is an error, never a
  wrong nib)
- runtime checks that match

Beyond translating elements, ibtool also splits storyboards into a nib per
scene plus an `Info.plist` of scenes and segues, follows storyboard
references, writes variants per idiom and deployment target, resolves named
colors and images against the asset catalogs (with fallback values), encodes
fonts, overlays base localization `.strings`, and turns trait and size class
overrides into variations.

- [x] NIBArchive writer, decoder and structural diff harness. `nib.rs`
      writes the archive; `tests.rs` reads ours and Xcode's into a form that
      doesn't depend on object or key order (some of ibtool's arrays come
      from hash tables) and compares them. Golden storyboards in
      `fixtures/ibtool` (`regen.py`). `IBTOOL_FIXTURES=<dir>` points the
      test at another folder laid out the same way, and
      `IBTOOL_DUMP=<dir>` writes both canonical forms there for a diff
- [x] Version, compile and link operations, including single-file XIB output
      and localization directories. The version probe matches Xcode 27's XML
      plist (`com.apple.ibtool.version`, bundle 25098, short version 27.0)
- [x] System font weights, italic and custom fonts, Dynamic Type descriptors,
      system color fallbacks, named colors, images and SF Symbols
- [x] Swift and Objective-C custom classes, module names, outlets, actions,
      runtime attributes and Base.lproj localizable strings
- [x] Buttons and button configurations, text fields, switches, sliders,
      activity indicators, pickers, scroll views, stack views, visual effects
      and refresh controls
- [x] Navigation, table, collection and split controllers; static tables,
      cell and supplementary-view prototypes, self-sizing layouts, toolbar
      items, show/show-detail segues and connections across nib boundaries
- [x] All 32 iOS corpus files compile on Linux and match Xcode structurally
      at their original source paths: 24 storyboards and 8 XIBs
- [x] Focused UIKit checks against an Xcode build on the iOS 27 iPhone 17
      simulator. See the runtime result below
- [ ] Public XIB set and a broader weighted coverage report *(optional)*
- [x] AppKit encoders for all 26 macOS corpus files, containers and
      storyboards included. Each matches Xcode structurally; the full audit
      below lists what stays open beyond the corpus
- [x] All four WatchKit corpus files and the tvOS corpus file match Xcode
      structurally. This does not cover all WatchKit or tvOS controls.

### iOS corpus sweep, 2026-11-01

The completed Linux sweep accepts all 32 iOS documents and matches Xcode 27
on all 32. The baseline accepted 15 and matched 14.

| Result | Baseline | Completed |
|---|---:|---:|
| Storyboards compiled | 15 / 24 | 24 / 24 |
| XIBs compiled | 0 / 8 | 8 / 8 |
| Documents matching Xcode structurally | 14 / 32 | 32 / 32 |

The final local report is
`corpus/_ib-tools/sweep-linux-verified-2026-11-01/report.json`.
It records source hashes, commands, diagnostics, archives and comparisons.
The comparator reads the actual Linux output through `IBTOOL_ACTUAL`.
It preserves property values and connection destinations while normalizing
hash-table order, proxy numbering and equal NSString sharing. A regression
test checks that normalization still detects a changed connection target.

The first sweep exposed a path-dependent issue: Xcode wraps text from
`Base.lproj` in `NSLocalizableString`. The final sweep uses the original
paths, and a dedicated `Base.lproj/Localized.storyboard` fixture prevents
copied sources from hiding this behavior again. Empty strings stay plain;
controller and button configuration titles retain their localization keys.

The focused runtime checker is the local `corpus/_uikit-app` project.
It checks fonts, colors, custom classes, control properties, actions,
XIB outlets, containers, static tables, prototype cells and a show-detail
segue. Clean builds on Linux and Xcode both report zero errors and warnings.
All 316 `IBCHECK` lines match on the iOS 27 iPhone 17 simulator. The local
results are `corpus/_ib-tools/complete-{linux,xcode}-build.json`,
`complete-{linux,xcode}.txt` and the empty `complete-runtime.diff`.
Runtime checks supplement the full archive sweep; they do not launch each
of the 32 original applications.

Validation also passes on both hosts:

- macOS: workspace library tests (715 CLI, 125 core and 449 library tests),
  plus the new proxy-destination regression test
- Linux: CLI library tests (710 passed; 6 platform-specific tests ignored)
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all -- --check` and `git diff --check`

To repeat the full sweep, choose a new output directory:

```bash
python3 sweetpad-cli/fixtures/ibtool/sweep.py \
  --linux-vm sweetpad-linux \
  --binary /home/hyzyla_home/sweetpad-target/debug/sweetpad \
  --out sweetpad-lib/corpus/_ib-tools/sweep-linux-next
```

The script returns a nonzero status if compilation or structural comparison
fails. `fixtures/ibtool/regen.py` refreshes the checked-in Xcode captures.
`cargo test -p sweetpad-cli --lib cli::ibtool::tests` checks those captures
without requiring Xcode.

Coverage here means the current iOS corpus. External storyboard references,
trait/idiom variations, non-base localization overlays and UIKit object types
outside these fixtures still need separate oracle fixtures and validation.

### Session handoff, 2026-11-03

The working branch is `claude/linux-build-pif`. The compiler changes and new
fixtures are still in the local working tree. The corpus audit reached 63/63
structural matches. The AppKit runtime comparison passed 338 assertions and
matched all 44 observations.

1. Inspect `git status --short --branch` before you edit the compiler.
2. Rebuild `sweetpad-cli` in the `sweetpad-linux` OrbStack VM.
3. Run the corpus sweep from the repository root on the Mac.

```sh
python3 sweetpad-cli/fixtures/ibtool/sweep.py --runtime all \
  --linux-vm sweetpad-linux \
  --binary /home/hyzyla_home/sweetpad-target/debug/sweetpad \
  --out /tmp/ibtool-linux-next
```

Use a new output directory for each run. Expect 63 compilations and 63
structural matches. If a document differs, read its `cases/*/structural.diff`
in the output directory.

For the next coverage pass, select an unverified feature from the remaining
work in the full audit. Start with a small fixture that isolates the feature.
Compile it with native Xcode for the reference output. Compare the Linux output
with that reference. If the change affects AppKit behavior, run the runtime
checker as well. Run the full corpus sweep after the fix.

### Full audit, 2026-11-03

`fixtures/ibtool/audit.py` inventories all corpus platforms and installed Xcode
metadata. It found 1,036 element/attribute pairs, 57 command options, 607 class
descriptions and 1,828 inspector properties. Source-name matches are search
hints. They do not prove implementation coverage.

`fixtures/ibtool/sweep.py --runtime all` checks all 63 documents. The latest
Linux audit, on 2026-11-03, compiles all 63 files and matches all 63 native
outputs structurally: 32 iOS, 26 macOS, four watchKit and one tvOS document.
Xcode also compiles all 63 files. No corpus differences remain.
Results and commands are in
`corpus/_ib-tools/audit-2026-11-01/full-sweep-linux14/report.json`.

The expanded UIKit checker runs 56 assertions in English and French. The Linux
and Xcode products pass all 112 assertions with identical results. These include
accessibility, outlet collections, localized strings, trait changes, constraint
changes, tab bars, embedded controllers, and button and custom segue triggers.
The AppKit checker also validates Linux-generated resources in a native Mac
executable. Both resource compilers pass the same 338 assertions and produce
identical runtime observations. New checks cover accessibility descriptions,
adaptive sheet layout, keyboard modifiers, recessed popups, storyboard table
prototypes, control defaults, constraint precision and bindings without options.
The reference and Linux reports are
`corpus/_ib-tools/audit-2026-11-01/runtime-mac-xcode-finish63g/report.json` and
`corpus/_ib-tools/audit-2026-11-01/runtime-mac-linux-finish63/report.json`.
This checks resource compatibility; it does not prove a Linux build of that executable.
Both checkers live under `fixtures/ibtool/runtime*`.

`fixtures/ibtool/mutate.py` tests one mutation per selected iOS attribute pair.
The first run found 16 gaps in 224 cases. All 224 now match Xcode on Linux.
Fixes cover decimal integer spellings, unresolved cell labels, system color
fallbacks, runtime booleans, layout priorities, zero minimum scale, navigation
items and triple-column split controllers. Separate fixtures cover all four
button configuration styles and their five corner styles. The report is
`corpus/_ib-tools/audit-2026-11-01/property-mutations-linux-fixed/report.json`.

New AppKit encoders cover stacks, grids, secure fields, WebKit views, menus,
scroll containers, text views, tables, outline views, collection views and defaults controllers.
Collection fixtures cover flow layouts, selection, spacing, supplementary sizes and legacy grids.
Runtime checks load collection data through a data source and exercise item reuse.
Outline checks expand a data-source tree and retain parent identities. Table encoding
covers columns, header and data cells, sizing, scroll increments and selection
options. All 48 isolated table variants match Xcode. Scroll fixtures cover
three zoom levels. Thirty-three isolated text-view variants match Xcode.
Menu-only Mac storyboards, ordinary view-controller scenes and window-controller
scenes compile and load through AppKit. Controller fixtures cover named, unnamed and empty identifiers,
custom classes, separate view archives, outlets, actions and mixed menu scenes.
Window fixtures cover custom controllers, content-controller relationships,
identifiers, tabbing modes and size limits. Content-controller assignments run
after storyboard outlets because those assignments can load their views.
A native-fixture regression test preserves that connection order.
The runtime comparison also checks text getters that system preferences can
override.

Scene objects remain in controller archives and use external placeholders in view archives.
Fixtures cover bindings to scene models, show segues, embed segues and split-controller relationships.
Split checks cover child order, collapsed state, priorities, delegates and view frames.
Tables also archive reusable prototype views in nested NSNib resources.

Named fonts resolve through a captured system-font catalog, including family aliases.
Unavailable names use the system font and retain their requested point size.
Native fixtures cover private system names, weights and case-sensitive fallback.
Font and layout profiles come from the macOS 27 native oracle environment.
They do not establish custom-font activation or font availability on every OS version.

Views now use a Cassowary solver with measured control metrics.
The solver handles alignment rectangles, baselines, intrinsic sizes, priorities and constraint relations.
It also propagates constraints across nested containers and resizes scroll contents.
Separate fitting passes preserve native canvas geometry for partially sized containers.
Window content retains its minimum size. Native profiles supply checkbox, popup,
separator and system-image metrics.
Fixtures cover equal-width sheet buttons, multiline fields, adaptive sheets,
nested storyboard tables and centered bold labels.
Grid rows and columns retain automatic sizing when content provides intrinsic dimensions.
Other writing systems and custom fonts still need native metric profiles.
Progress fixtures also preserve omitted and explicit range values; an omitted maximum is zero.
Constraint encoding now includes relations, priorities, identifiers and ratio
multipliers. Placeholder constraints are omitted. View metadata covers alpha,
identifiers, tooltips and compression priorities. The structural comparator
keeps parallel ownership arrays paired and preserves invalid or duplicate IDs.
It resolves controller UUIDs through their package metadata and retains links
between archives. It normalizes two private table caches that change when
sandbox restrictions prevent access to window services. Table and scroller
visibility, geometry and other flags remain part of the comparison.
Accessibility object numbers are normalized together with ordinary object numbers;
connector destinations and duplicate-number errors remain checked.

Repeated native compilations can archive an Auto Layout child before or after
its parent resizes. The comparator normalizes only that exact vertical resize
offset, using required constraints anchored to the root. Sizes, fixed views,
other offsets and the constraints remain checked. Regression tests reject a
one-point error, a changed size, a changed constraint and an unanchored inequality.
The sweep also uses the Rust harness's canonical plist comparison. Raw controller
UUIDs change on every native compile and cannot serve as equality checks.

Final validation passed `cargo test --workspace`, CLI Clippy for all targets,
and Linux CLI tests: 729 passed, six ignored. Six new native fixtures cover
the final layout, popup, table, accessibility, precision and binding fixes.

The audit fixed an integer encoding error that structural comparisons missed.
NIBArchive's compact integers are unsigned. Negative values need the 64-bit
form. An AppKit image cell exposed the error at runtime. The new IntegerWidths
fixture checks signed values and each compact-width boundary.

Other additions include external storyboard references, companion strings,
editable NIB packaging, stripping, diagnostic output formats, validation without
output, and modal, custom, unwind and embed segue archives. Archive fixtures
cover unwind segues; a working unwind navigation flow remains unverified.

Remaining audit work includes:

- More AppKit properties and object variants, including rich menu titles,
  accessibility attributes beyond descriptions and additional data-cell classes.
- Layout and font cases beyond the corpus, including other writing systems,
  custom fonts and additional container combinations.
- Further scene-model variants and controller initialization order.
- More UIKit classes, rich text, gesture recognizers and property variants.
- AppKit rich text storage and general sRGB color conversion. Named-color
  fallbacks use seven captured native RGB samples. Other samples fail explicitly;
  these profiles do not establish a general ColorSync replacement.
- Additional trait conditions and relationship masks.
- Import, export and editing commands from the command-option inventory.
- SDK, target-device and deployment-target effects. Accepting these options
  does not prove that every platform-specific transformation is implemented.

Many unsupported properties still need isolated probes. The audit must not use
corpus success, accepted arguments or source mentions as a claim of full parity.

### Real-app integration, 2026-11-01

All three apps build on Linux and Xcode for the arm64 iOS Simulator.
The comparison uses Debug, deployment target 18.0 and disabled code signing.
NetNewsWire also uses `SWIFT_TREAT_WARNINGS_AS_ERRORS=NO` on both hosts.

| App | Pinned source revision | Linux build | Xcode build | Linked IB containers |
|---|---|---|---|---:|
| Kingfisher demo | `cf8be20d07654570554c8a8a4952bc8a5766a8b0` | Pass | Pass | 2 match |
| Alamofire iOS Example | `7595cbcf59809f9977c5f6378500de2ad73b7ddb` | Pass | Pass | 2 match |
| NetNewsWire iOS | `4d1cb6afae1aa7d52c51077f7f40d0d267e8cc4b` | Pass | Pass | 13 match |

Kingfisher uses its root workspace, which includes the demo and framework.
The legacy Kingfisher and Alamofire apps require scene delegates on iOS 27.
Both hosts use the same small compatibility patches, saved with the results.
NetNewsWire requires its existing script to generate `SecretKey.swift` before
SwiftPM discovers that source. No credentials were supplied for this check.

The simulator tests cover Kingfisher's Basic and Auto Sizing pages,
Alamofire's main table and GET detail, and NetNewsWire's Settings and Add Account.
They capture screenshots and accessibility trees on eight screens.
The assertions also require Alamofire's section headers and at least one
completed Kingfisher image request. All three tests pass on both hosts.

Five complete accessibility trees match after process metadata normalization.
Kingfisher's visible Auto Sizing cells also match. The remaining differences
in the trees come from offscreen image requests, HTTP timing and live feed
refreshes. The report retains all these differences.
Five screenshots match pixel for pixel after exclusion of the status bar.
NetNewsWire's Add Account screen has small service-icon rasterization
differences: 0.187% of app pixels differ, while labels and frames match.

The runtime comparison exposed an outlet-order defect that structural
normalization hid. The compiler now connects `staticDataSource` before the
table's `dataSource` outlet. This order prevents UIKit from caching empty
section headers. A regression test checks this order against Xcode's fixture.

All 17 linked storyboard/XIB containers match Xcode structurally.
All bundle resource paths match after the extension icon fix.
Xcode also includes debug/preview libraries and Alamofire's thread sanitizer
runtime. The Linux bundles do not include that instrumentation.
Binary bytes, sanitizer behavior and Siri registration are not parity claims.

The local results are in `corpus/_app-integration/2026-11-01/`:

- `build.py`, build envelopes and command records for both hosts
- `kingfisher-ios27-compatibility.patch` and `alamofire-ios27-compatibility.patch`
- `bundle-ib-comparison-final/report.json` and `bundle-inventory-final.json`
- `ui/Tests/IntegrationTests.swift`, UI test envelopes and exported attachments
- `ui-comparison-final/report.json` and the full accessibility tree differences
- `report.json`, which summarizes builds, runtime checks and known limits

Final regression checks pass: 722 CLI, 125 core and 449 library tests on macOS;
716 CLI tests on Linux, with six platform-specific skips.
Clippy with `-D warnings`, formatting and diff checks also pass.

## String Catalogs

- [x] `xcstringstool` stand-in: compile, `--dry-run`, generated symbols,
      byte for byte with Xcode on golden probes and every corpus catalog
      (`cf4a7de5`)

## Asset catalogs

- [x] `actool` stand-in: asset symbols, the iOS app icon as loose PNGs
      (`101c1443`, `f153d439`)
- [x] `Assets.car` with colors (every color space, appearances, system
      colors) and PNG images (scale, idiom, appearance, template rendering)
      (`11b3cbb7`)
- [x] PDF (hayro) and SVG (resvg) images drawn at the platform's scales, and
      custom symbols on every platform (`31a3fd2e`). 44 of 47 checker
      lookups match Xcode's build; 89 of 93 corpus catalogs list the same
- [x] JPEG and HEIF images, kept in the car as they are, with their size
      and EXIF orientation. `_car-app`'s six photo lookups match Xcode's
      build. Xcode also decodes a HEIF image into a bitmap for systems
      before iOS 11; that needs an HEVC decoder, and no app built with
      Xcode 27 runs there
- [x] App icons inside `Assets.car`, each size numbered as Xcode numbers
      them, with a list of sizes per idiom. Single-size, every-size, dark
      and tinted, and alternate sets (`--alternate-app-icon`,
      `--include-all-app-icons`) list the same in `assetutil` as Xcode's,
      except that Xcode packs the small sizes into atlases. The simulator
      shows the icon from the car alone
- [x] Extension icons: `--standalone-icon-behavior all` preserves the extra
      standalone sizes. NetNewsWire's share and widget extensions match Xcode's
      icon file names and dimensions
- [x] macOS app icons: `CFBundleIconName` for the icon in the car, and an
      `.icns` with the four sizes Xcode's actool stores beside it
- [x] iOS Icon Composer `.icon` static light/dark/tinted appearances, layer order,
      primary PNGs, icon metadata, and shared icon/image names
- [ ] Icon Composer interactive glass lighting and native material effects *(optional)*
- [x] watchOS app icons: the single-size set and the legacy set with an
      image for each size go in the car alone, named in `CFBundleIcons`.
      Each legacy image goes in by the slot it fills, under its watch's
      device class; as with actool, images that fill no slot and dark or
      tinted ones stay out. Car metadata names the platform as Xcode does
      (`watch`, `atv`, `vision`). Goldens in `fixtures/actool`
      (`regen_resources.py`). The delivery fixture's Linux archive carries
      the watch icon, as Xcode's does; an upload of a fully Linux-built
      watch app is unchecked
- [ ] Icon Composer on other Apple platforms *(optional)*
- [x] Data sets (`NSDataAsset`), each file as it is with its type: the one
      its entry names, else its extension's, or the `dyn.` type macOS makes
      up for an extension it doesn't know. `_car-app`'s four data lookups
      match Xcode's build
- [ ] Fallback values for system colors (black today; UIKit resolves the
      name anyway) *(optional)*
- [x] Ordered PNG/GIF/JPEG sticker packs, accessibility labels, pack metadata,
      and message-extension icon files and metadata. The icon's App Store
      image goes in the car as a plain image, the set's only rendition there,
      and the 29-point sizes stay out, as Xcode 27's actool does; App Store
      Connect refuses the extension otherwise (ITMS-90644)
- [x] Loose catalog outputs an earlier build put in the product, like an
      icon size the set no longer has, leave it on the next build.
      `swift-build-linux.patch` has the asset catalog link step remove what
      the previous compile's dependency info lists and this one doesn't
      produce; Swift Build declares only `Assets.car`
- [ ] Sticker sequences and localized sticker-pack strings *(optional)*
- [ ] AR resources, texture sets, sprite atlases, watch complications,
      launch images *(optional)*

## Build kinds to try

- [x] iOS Simulator, Debug: Xcode's app template, the Linux fixture, a
      SwiftUI app with Alamofire (project reference and package), the local
      checker apps; Ice Cubes compiles and fails only where Xcode 27 does too
- [x] Real iOS apps: Kingfisher demo, Alamofire iOS Example and NetNewsWire,
      including its three extensions. See the integration result above
- [x] Release configuration: dSYM, strip on install, a static library, a
      loose PNG; `corpus/_tools-app` (local) runs the same in the simulator
- [x] iOS device builds: `corpus/_tools-app` (local) for
      `generic/platform=iOS` matches Xcode's unsigned build, widget
      included; signing is under Past building
- [x] App extensions and widgets: an `embeddedBinaryValidationUtility`
      stand-in makes Xcode's checks (bundle ID prefix, matching versions);
      a widget built here registers in the simulator
- [ ] App Clips
- [x] Test targets: `test build` adds the selected test plan's targets, or
      the scheme's testables, as `build-for-testing` does, and leaves the
      testing frameworks out of the host app like Xcode 27. The bundle passes
      under `xcodebuild test-without-building` with Xcode's `.xctestrun`
- [x] An `.xctestrun` written by the build, so tests built here run on a Mac
      without a second build *(optional)*
- [x] macOS apps: `corpus/_mac-app` (local) matches Xcode's build in all 31
      asset lookups, the app icon among them. Swift
      Build needs `swift-build-linux.patch` (`linux-setup.sh` applies it)
      to skip registering the app with Launch Services; upstream it
- [x] watchOS, tvOS and visionOS simulator builds. Setup imports their SDKs
      and metadata, including UI-test runner bundles. Alamofire builds for all three;
      Kingfisher's tvOS demo and modern standalone app probes build. Fixture patches
      migrate the legacy watch apps; their app and companion workspace schemes build.
      Runtime and device-signing checks for these platforms remain unverified.
- [ ] Mac Catalyst *(optional)*

## Past building

- [x] Info.plist keys: `DTXcode`, `DTXcodeBuild` and `DTPlatformBuild` from
      the `version.plist` files `linux-setup.sh` copies into the SDK bundle
      (xtool leaves them out); the same as Xcode's build
- [x] `BuildMachineOSBuild`: the SDK bundle's `build-machine.plist` holds
      the macOS build of the Mac that exported the SDK (`sw_vers`), or, for
      an Xcode archive imported without a Mac, Xcode's own build machine's.
      sweetpad passes it as `MAC_OS_X_PRODUCT_BUILD_VERSION`, which Swift
      Build writes into the Info.plist. App Store Connect also accepted a
      build without it
- [x] Signing: sweetpad stands in for `codesign` and `derq`, and answers
      Swift Build's provisioning requests, which `swift-build-linux.patch`
      sends to the tool `SWEETPAD_PROVISIONING` names. The identity comes
      from a `.p12` file (`SWEETPAD_CODE_SIGN_P12`, its password in
      `SWEETPAD_CODE_SIGN_P12_PASSWORD`), as Keychain Access or OpenSSL 3
      exports it. Profiles come from Xcode's folders or
      `SWEETPAD_PROVISIONING_PROFILES`, picked the way automatic signing
      picks them. A device build of `corpus/_tools-app` (local), widget
      included, passes `codesign --verify --deep --strict`. Apple's
      `codesign` signs the same files to the same bytes, apart from the CMS
      signing time
- [x] Install and launch a Linux-built parity fixture on an iPhone 13 with
      iOS 27.0.1, signed with an Apple Development identity. All seven
      app-hosted tests pass with a Linux compilation marker. Apple signature
      verification passes. Exported executable hashes remain unchanged.
      This run uses Mac device transport. The seven larger app fixtures also
      pass two launches each; MMKV passes seven storage checks twice. Limited
      Wikipedia and Mastodon copies omit extensions and use app-local storage.
      App Groups, push, payments, widgets and extensions remain unverified on hardware.
- [x] Execute the unedited automatic Linux test manifest on iOS 27.2.
      Unit and UI tests pass in both selected test-plan configurations: four executions.
      Runner signatures pass Apple verification. Exported executables remain unchanged.
      Fresh unshimmed MMKV, Flutter AOT, and React Native/Hermes Release apps also
      pass two launches each with successful Linux execution audits.
- [x] Linux device list/install/uninstall/launch/log transport through
      pymobiledevice3. Tested argument construction against version 11.50.0.
      The direct Wi-Fi check now passes hostname resolution, authenticated device
      selection, installation, launch, process checks, and scoped startup logs.
      The CLI now supports both direct Wi-Fi and physical USB through the
      pinned Python adapter. Install, launch, scoped logs, stop, uninstall,
      and reinstall pass on the iPhone without a Mac relay or recompilation.
      `device pair-network` creates an isolated record through trusted USB.
      Automatic discovery works through multicast or system DNS-SD. The VM
      uses DNS-SD because its bridge does not carry raw LAN multicast.
      OrbStack USB passthrough exposes the phone to local Linux usbmuxd.
      `help linux-devices` documents setup and bounded logs. CLI readiness
      inspection and device debugging remain unsupported on Linux.
- [x] Swift Build archives and development IPA export on Linux. A signed
      fixture archive and IPA pass Apple codesign verification. Unsigned IPA
      export and simulator archives are rejected. App, extension, and
      embedded-framework dSYMs are collected.
- [x] Distribution export on Linux, as Xcode's: `archive export` and
      `archive --export-method` sign the archived app again with the
      identity and profile kind of the method (App Store Connect, ad hoc,
      enterprise, development), from the identity set and profiles above.
      A device archive is always signed, ad hoc with its entitlements when
      the identities can't sign it. The delivery fixture's App Store export
      matches Xcode's entitlements and files, uploads from Linux and
      processes as VALID; its ad hoc export installs and launches on an
      iPhone 13
- [x] Upload to App Store Connect from Linux and macOS through its REST
      API (`sweetpad upload`, `archive --upload`), IPAs and Mac App Store
      packages alike
- [x] macOS signing on Linux: the `codesign` stand-in signs macOS bundles,
      versioned frameworks, XPC services and universal binaries as Apple's
      does (16 KiB pages for arm64, 4 KiB for Intel, the runtime version
      with `-o runtime`, nested code sealed by its code directory hash and
      designated requirement), matching `codesign`'s output byte for byte
      for the fixtures. Developer ID signatures get a secure timestamp from
      Apple's server, as `codesign` gives them; `--timestamp[=URL]` asks
      for one. A macOS build is signed when there's an identity, and a
      macOS archive always, ad hoc when the identities can't sign it. The
      delivery fixture's App Store archive matches Xcode's entitlements,
      files and seals; packaged with `productbuild` it uploads from Linux
      and processes as VALID
- [x] Mac App Store packages on Linux: a macOS archive's
      `app-store-connect` export signs the app again with Apple
      Distribution and the Mac App Store profiles, refuses an app without
      the App Sandbox, and writes the installer package `productbuild
      --component … --sign` writes, signed with the Mac Installer
      Distribution identity. Its bill of materials, payload, PackageInfo
      and Distribution match `productbuild`'s for the fixture app, and the
      delivery fixture's package uploads from Linux and processes as VALID.
      `debugging` and `mac-application` exports write the `.app`
- [x] Developer ID on Linux: a macOS archive's `developer-id` export signs
      the app again with Developer ID Application, a timestamp and the
      hardened runtime (refusing a main executable without it). It embeds a
      Developer ID profile only for entitlements Apple must grant, and
      writes a package signed with Developer ID Installer and timestamped
      when that identity is in the set
- [x] Notarization from Linux and macOS through the notary REST API
      (`sweetpad notarize`, `archive --notarize`): the file goes to S3
      under the submission's credentials, and the ticket is stapled from
      Apple's ticket service, byte for byte as `stapler staple` staples it
      to an app, a package and a signed disk image. The delivery fixture's
      Linux-built app, zip and package are accepted and pass Gatekeeper as
      Notarized Developer ID; a Developer ID disk image stapled by sweetpad
      is identical to `stapler`'s and passes `spctl` as Notarized Developer
      ID. Files over 64 MiB go to S3 in parts; a 12 MB zip sent in 5 MiB
      parts is accepted
- [x] macOS archives are universal: Xcode's `ARCHS_STANDARD`, arm64 and
      x86_64, and arm64_32 beside arm64 for a watch app. `lipo -segalign`
      for `dsymutil`, `lipo`'s output permissions, and the string table
      alignment `llvm-strip` breaks for x86_64 are fixed on the way; see E19
- [x] Apple's content in archives and App Store IPAs: `SwiftSupport/<platform>`
      with the toolchain's own copies of the Swift libraries the app embeds
      (with Xcode 27, `libswiftCompatibilitySpan.dylib` for Span below OS
      26), `WatchKitSupport2/WK` and the Messages stubs, each identical to
      Xcode's archive. The delivery fixture's IPA with SwiftSupport and its
      universal Mac App Store package, embedding the Span library, upload
      from Linux and process as VALID, and so does the fixture with a watch
      app and a sticker pack, built and exported entirely on Linux, as well
      as Xcode's archive of it exported on Linux
- [x] The codesign stand-in matches Apple's per-platform rules: 4 KiB pages
      for Intel, tvOS, and iOS before 16 or watchOS before 9; a SHA-1 code
      directory beside the SHA-256 one for iOS and tvOS before 11 and
      arm64_32 before watchOS 12, whose bundles seal each resource with its
      SHA-1 too (ITMS-90034 otherwise, as for the Messages stub); and a bare
      Mach-O's identifier from its embedded Info.plist (ITMS-90334
      otherwise). Goldens in `fixtures/codesign` (`regen.py platforms`)

Running in the simulator stays on a Mac.

## Resuming

- The VM is `sweetpad-linux` (provisioned by `ci/linux-setup.sh`). Inside it,
  OrbStack's view of host files lags edits: `touch` only the changed files
  before `cargo build` there. A `find … -exec touch` over the shared folder
  once hung OrbStack. Set `CARGO_TARGET_DIR=$HOME/sweetpad-target`.
- Swift Build's sources are at `~/src/swift-build` in the VM; the xcspecs
  are under `Sources/*/Specs`.
- Local checker apps live in the gitignored `sweetpad-lib/corpus/`:
  `_car-app` (57 asset lookups), `_strings-app`, `_alamofire-spm`,
  `_alamofire-consumer`, `_alamofire-dynamic` (the dynamic product,
  embedded), `_tools-app` (XcodeGen: static libraries, a loose
  PNG, a widget, a unit test bundle), `_mac-app` (XcodeGen: a macOS app's
  asset lookups and icon), `_data-app` (XcodeGen: a Core Data model with
  inheritance, a fetch index and generated classes, and an App Intent;
  `linux-build.sh` builds it in the VM), `_swiftdata-app` (XcodeGen:
  `@Model` classes with unique constraints, indices, a `@ModelActor` and
  `@Query`; `expansions/` holds Xcode's macro expansions), `_swiftui-app`
  (XcodeGen: `@Entry` values of several kinds, `@State`, `@Observable`, and
  `#Preview` of SwiftUI and UIKit views), `_uikit-app` (XcodeGen: the
  template, Kingfisher, Tuist and Alamofire launch screens and the
  `Labels` and `Images` fixtures, loaded from storyboards and printed as
  `IBCHECK` view trees with constraints). Each prints its
  results to stderr for a diff against Xcode's build.
- The `linux-build.sh` scripts stop when the build leaves no products.
  An empty products path once turned their `cp -R "$D"/.` into a copy of
  the VM's root, Mac mounts included, which filled the disk.
- `sweetpad-lib/corpus/_ib-tools/` (gitignored): `nibdump.py` dumps a
  NIBArchive as it's laid out, `ibcheck.sh` installs an app in the booted
  simulator and prints its `IBCHECK` lines, `ib-xcode.txt` is what
  `_uikit-app` prints when Xcode builds it, and `vm.sh` runs a command in
  the VM after rebuilding sweetpad there (`FILES=` lists the changed files
  to touch first).
- To diff one source file's macro expansions with Xcode's, typecheck it
  twice with `-Xfrontend -dump-macro-expansions`, once with
  `-load-plugin-executable <sweetpad-macros>#FoundationMacros,PreviewsMacros,SwiftDataMacros`
  added, and diff the two (`macros/fixtures/check.py` does this for its
  fixtures).
- `sweetpad-lib/corpus/_momc-tools/` (gitignored): `keyed.py` dumps a keyed
  archive as it's laid out, `canon.py` dumps it independent of object
  order (to diff a `.mom` with Xcode's), and `hashes.py` computes version
  hashes from a model's XML. To see what Core Data hashes, run a small
  Swift tool that prints `NSEntityDescription.versionHash` under lldb with
  breakpoints on `CC_SHA256_Update` that dump each buffer.
- `sweetpad-lib/corpus/_car-tools/` (gitignored) keeps the scripts used to
  decode `Assets.car`: `cardump.py` dumps one, `surgery.py` edits and
  rebuilds one, `listing.py` and `corpus_compare2.py` compare `assetutil`
  listings with Xcode's, `glyphs.py` compares symbol records, and
  `icons.py <case>` compiles app icon sets of each shape with Xcode's actool,
  or with `OURS=<path to an actool symlink>` the stand-in. `carcheck.sh`
  and `afcheck.sh` install an app in the booted simulator and print its
  check lines; the Alamofire apps want `afserver.py` running. Their paths
  point at a deleted scratch folder; adjust them before use.
- Golden fixtures to regenerate after an Xcode update:
  `sweetpad-cli/fixtures/actool`, `fixtures/xcstringstool`,
  `fixtures/momc` and `fixtures/ibtool` (each has a `regen.py`), `macros/fixtures`
  (`check.py --record`), and the symbol templates' expected values in
  `src/cli/actool/glyph.rs`.
