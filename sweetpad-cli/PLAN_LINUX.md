# Building Xcode projects on Linux

Status on 2026-11-10. The initial Linux engine merged in PR #346.
SDK export, Ubuntu setup/import, and Linux doctor are committed in `880b692d` on `codex/linux-sdk-setup`.
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

The release archive targets Ubuntu 24.04 ARM64 on Apple hardware.
It contains the CLI, DAP alias, checksums, an installer, a sample app, licenses, and public setup sources.
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
Distribution export and Linux device debugging remain outside this release.
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
SDK data was reused read-only with x64 host tools; a complete fresh x64 SDK import
and compiler download/bootstrap was not repeated. No physical device was used.

The existing Flutter AOT and full hybrid-app validation remains ARM64-specific.
The x64 private CI path covers package installation, core app builds, test manifests,
and package linking; it does not imply full hybrid-framework runtime coverage.

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
- [ ] S03: Check the build service, Darwin librarian, zip, extra platforms, device tools, and pinned Swift Build provenance where available (`commands/doctor.rs`). Build-service and librarian checks are required; zip/usbmuxd are optional checks. Extra platforms and patch provenance remain open.
- [x] S04: Report `--replace-sdk` for setup migration, retain useful bootstrap errors in JSON, and handle root without sudo (`linux-sdk.py`, `commands/setup.rs`, `ci/linux-setup.sh`).
- [x] S05: Test setup source discovery and Linux doctor independently of the developer machine (`commands/setup.rs`, `commands/doctor.rs`).
- [x] S06: Consolidate tool version requirements across bootstrap, export, doctor, and packaging (`commands/setup.rs`, `linux-sdk.py`, `ci/linux-setup.sh`).
- [x] S07: Document Python before setup and replace raw JSON in SDK human output (`ci/linux-release-README.md`, `commands/sdk.rs`).
- [x] S08: Upgrade an existing pinned Swift Build checkout without losing local edits; pin its package dependencies (`ci/linux-setup.sh`, Swift Build `Package.swift`).
- [ ] S09: Validate the complete setup on a fresh host, separately from reusing preinstalled compiler tools (`ci/check-linux-sdk-install.py`). Isolated package compilation passes with an empty home and read-only toolchain/SDK mounts. A full first bootstrap with no preinstalled compiler remains unverified.
- [ ] S10: Validate `doctor --target` behavior on macOS, signing credentials, and provisioning contents (`commands/doctor.rs`). Explicit --target on macOS now fails with a usage error; credential and profile contents remain open.

### Build engine and command routing

- [x] E01: Route run and app install builds through the selected engine (`commands/app.rs::build_and_install`, `xcodebuild.rs::BuildPlan`).
- [x] E02: Clean Linux products without invoking xcodebuild, including purge (`commands/clean.rs`, `swbuild.rs::run`).
- [x] E03: Distinguish configuration-sourced xcodebuild flags from passthrough errors (`cli/mod.rs::xcodebuild_args`, `swbuild.rs::refuse_xcodebuild_flags`).
- [x] E04: Reproduce script sandboxing on Linux and disable unavailable sandbox-exec tasks there (`swbuild.rs::DarwinSdk::overrides`, Swift Build `TaskProducerSandboxing.swift`).
- [ ] E05: Parse and run scheme pre/post actions with the selected build context (`sweetpad-lib/src/scheme.rs`). Build and Archive shell actions now run in Xcode-validated order, with selected project-target settings, generated-source support, and failure propagation. Test runtime hooks, package-target environments, and non-shell actions remain open.
- [ ] E06: Preserve Xcode target build-tool plugin dependencies and verify package-target plugins (`sweetpad-lib/src/workspace_pif.rs`).
- [ ] E07: Use the Darwin SDK for resolved settings, product paths, and BSP preparation (`commands/settings.rs`, `sweetpad-core/src/build_settings.rs`, `bsp/mod.rs`).
- [x] E08: Offer runnable Linux device destinations without defaulting runtime commands to My Mac (`resolve.rs`, `simctl.rs`).
- [x] E09: Surface Linux device-discovery failures in destination listings (`commands/destination.rs::gather`).
- [x] E10: Resolve `build --on device` and device names on Linux (`resolve.rs::build_target_without_devices`).
- [x] E11: Select the platform of tvOS/watchOS/visionOS-only schemes for generic builds (`resolve.rs::build_target_without_devices`).
- [ ] E12: Resolve and update packages without xcodebuild on Linux (`commands/dependency.rs`).
- [ ] E13: Select a Linux opener for config and build directories; report unsupported Xcode opening (`commands/open.rs`).
- [ ] E14: Match scheme and test-plan targets by identifier and container when names collide (`swbuild.rs::selection`).
- [ ] E15: Isolate concurrent invocation files and update stand-in links atomically (`swbuild.rs::Invocation`, `swbuild.rs::link_tools`).
- [ ] E16: Invalidate resource tasks when stand-ins or SDK inputs change (`swbuild.rs::toolset`).
- [ ] E17: Preserve useful failure output, log paths, exit status, and child-process cancellation (`swbuild.rs`, `process.rs`). Failed build logs are now linked in human output; bubblewrap dies with its parent. Exit-status preservation and descendant-process cancellation remain open.
- [ ] E18: Correct classic intent-definition codegen visibility, no-codegen, and synchronized-folder metadata (`sweetpad-lib/src/workspace_pif.rs`).
- [ ] E19: Verify macOS cross-architecture builds and Info.plist preprocessing on Linux (`swbuild.rs::DarwinSdk::overrides`).
- [ ] E20: Preserve required linker settings when projects supply OTHER_LDFLAGS on additional platforms (`swbuild.rs::Invocation`).
- [ ] E21: Warn for missing workspace members, explain path case mismatches, and map ASSET_TAGS (`sweetpad-lib/src/workspace_pif.rs`, `swbuild.rs`).
- [ ] E22: Define verbose passthrough and structured blocker reporting for Swift Build (`swbuild.rs`).

### Signing, tests, and devices

- [ ] D01: Mount the developer disk image after a phone reboot or first connection before opening developer services. Needs hardware (`device_network.py::DvtProvider`).
- [ ] D02: Preserve simulator entitlements through ad-hoc signing and validate Keychain/App Groups on a Mac (`swbuild.rs::DarwinSdk::overrides`, `codesign/provisioning.rs`).
- [ ] D03: Match provisioning devices, manual profiles, entitlement values, and certificate validity (`codesign/provisioning.rs`, `codesign/identity.rs`).
- [ ] D04: Verify watchOS profile platform matching (`codesign/provisioning.rs`).
- [x] D05: Isolate failures for individual discovered devices and retain actionable resolution errors (`device_network.py`, `resolve.rs`).
- [x] D06: Report network-action timeout context and support bounded action-specific limits (`device_network.py`).
- [ ] D07: Validate the legacy device adapter version and scope log streams to the selected UDID (`pymobiledevice3.rs`). Legacy syslog now receives the selected UDID; adapter-version validation remains open.
- [ ] D08: Keep generated xctestrun paths portable across Linux and Mac hosts (`swbuild/test_manifest.rs`).
- [ ] D09: Accept harmless test-plan settings such as disabled coverage, and report unsupported coverage explicitly (`swbuild/test_manifest/selection.rs`).
- [ ] D10: Check zip availability and reject distribution-profiled development exports (`commands/archive/portable.rs`).
- [ ] D11: Verify signing-secret inheritance into build scripts and plugins (`swbuild.rs`, `codesign/identity.rs`).
- [ ] D12: Handle CocoaPods absolute codesign calls without requiring fixture rewrites, or document the limitation (`ci/linux-project-prepare.py`, `docs/cli/linux/react-native.md`).
- [ ] D13: Verify certificate chains beyond the bundled WWDR G3 intermediate (`codesign/identity.rs`).

### Packaging, CI, and docs

- [ ] P01: Run ARM64 packaging on main pushes and define private SDK validation for tag releases (`linux-arm64-package.yaml`, `cli-release.yaml`). Main pushes now run package checks; private SDK tag-release gating remains open.
- [x] P02: Give Linux self-update instructions that do not require the macOS-only Homebrew formula (`commands/self_update.rs`).
- [x] P03: Replace installed setup resources without stale files; describe PATH setup and removal (`ci/install-linux.sh`).
- [x] P04: Reject development-stamped binaries in actual tag-release packages (`ci/package-linux.py`).
- [x] T01: Correct archive --output-file in the main reference and add setup/sdk/device/doctor commands (`docs/cli/reference.md`).
- [x] T02: Mark setup convenience commands as newer than 0.1.18 and preserve the released manual setup route (`docs/cli/linux/sdk-transfer.md`, `docs/cli/linux/ubuntu-usb.md`, `880b692d`).
- [x] T03: Prevent generated shell configuration from shadowing a CI-pinned CLI (`commands/setup.rs`, `docs/cli/linux/github-actions.md`).
- [x] T04: Replace the setup script's reference to unshipped linux-build.sh (`ci/linux-setup.sh`).
- [x] T05: Align the Linux device help with the packaged venv layout and remove corpus-only paths (`LINUX_DEVICES.md`).
- [ ] T06: Document engine selection and setup environment controls, test-manifest platform limits, and app-host requirements (`CLI_DESIGN.md`, CLI help, Linux guides). Engine selection and setup controls are documented. Remaining manifest limits and app-host requirements still need a full review.
- [ ] T07: Reconcile SDK setup design, current branch/status, MMKV notes, and reference counts (`CLI_DESIGN.md`, `ci/LINUX_PROJECTS.md`, Linux reference). SDK command design, current branch, and MMKV shim notes are updated. Historical status/reference counts still need a full reconciliation.

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
- [ ] `metal`: the compiler is closed; fail clearly, maybe try the Windows
      build under Wine *(optional)*
- [ ] `realitytool`, `referenceobjectc`, Skybox, App Playground catalogs:
      fail clearly *(optional)*
- [ ] macOS-only legacy tools (`osacompile`, `Rez`, `ResMerger`, `mig`,
      `iig`, `dtrace`, `exutil`, `lsregister`): skip or fail clearly
      *(optional)*

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
- [ ] Icon Composer on other Apple platforms *(optional)*
- [x] Data sets (`NSDataAsset`), each file as it is with its type: the one
      its entry names, else its extension's, or the `dyn.` type macOS makes
      up for an extension it doesn't know. `_car-app`'s four data lookups
      match Xcode's build
- [ ] Fallback values for system colors (black today; UIKit resolves the
      name anyway) *(optional)*
- [x] Ordered PNG/GIF/JPEG sticker packs, accessibility labels, pack metadata,
      and message-extension icon files and metadata
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
- [ ] `BuildMachineOSBuild`: left out, since no macOS built the app. Find
      out whether App Store validation needs it
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
      export, simulator archives, and distribution export are rejected.
      App, extension, and embedded-framework dSYMs are collected.
- [ ] Upload with Transporter, which has a Linux build *(optional)*

Running in the simulator stays on a Mac.

## Resuming

- The VM is `sweetpad-linux` (provisioned by `ci/linux-setup.sh`). Inside it,
  OrbStack's view of host files lags edits: `touch` only the changed files
  before `cargo build` there. A `find … -exec touch` over the shared folder
  once hung OrbStack. Set `CARGO_TARGET_DIR=$HOME/sweetpad-target`.
- Swift Build's sources are at `~/src/swift-build` in the VM; the xcspecs
  are under `Sources/*/Specs`.
- Swift Build reuses an unchanged catalog's `Assets.car` even after the
  stand-in changes; clear the app's DerivedData before checking a new one.
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
