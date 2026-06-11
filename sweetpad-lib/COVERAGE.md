# Test-case coverage plan

Tracks which Xcode build-system features have a real example in the corpus
that our future Rust resolver will be snapshot-tested against. Items marked
"❌" are gaps — we either need to expand a corpus project to expose them,
add a new project in phase 2, or accept that they're out of scope.

This file is hand-maintained. Re-check against `fixtures/REPORT.md` after
each capture run to update the "Where" column.

**Reconciled 2026-05-30** against the captured corpus (754 scheme + 240
per-target build-settings JSONs + the 5 full source clones): every row was
verified with concrete evidence, so the table now reflects what the fixtures
actually exercise rather than unconfirmed guesses. Result (after the 2026-05-30
gap audit): **113 ✅ / 19 ❌** (no ⏳). The audit flipped two rows to ✅ —
`arm64e` (already covered by the `_synthetic/archs-arm64e` override) and a
genuinely custom-named configuration (the new `_synthetic-custom-config`
fixture). The remaining 19 ❌ are genuine corpus gaps — all niche/edge constructs
(XPC, DriverKit, Core ML / Metal / `.bundle` resources, custom build rules,
headers phase, scheme post-actions/launch-args, weak-linking, Swift-package-root)
— and most are pbxproj/scheme *structure* that doesn't change the resolved
build-settings dictionary, not core resolution. Re-run the verification when
fixtures change.

**Updated 2026-06-10**: the scheme-discovery work (user schemes under
`xcuserdata/`, autocreated per-target schemes) closed the "User scheme under
`xcuserdata/`" row and added an autocreated-schemes row, both covered by
hermetic tests rather than corpus captures (no corpus project ships either
layout) — see the Schemes table. Result: **115 ✅ / 18 ❌**.

**Updated 2026-06-11**: `tests/discovery_oracle.rs` now scores the listing
surface (`targets` / `configurations` / `schemes` per project, merged
workspace `schemes`) against the captured `xcodebuild -list -json`
(`metadata/**/list.json`, 30 captures) — every container matches exactly,
sets AND case-insensitive ordering. Deriving the rules fixed per-target
scheme autocreation (it fires even when other scheme files exist; tests,
WatchKit extensions, watchapp2 containers, and Safari legacy extensions are
excluded). Schemes `xcodebuild` synthesizes from Swift *package* manifests
(41 across ice-cubes/netnewswire) are out of the pbxproj surface and tallied,
not failed. The same pass drove the compiler-args **link** oracle to 100%
structural+precision on 25 of 27 dynamic links (Swift-runtime rpath via the
Concurrency/Span back-deployment gates, version-gated `-dead_strip` /
`-export_dynamic` spellings, `LD_DEBUG_VARIANT`, dependency
`-add_ast_path` + `-l<lib>` registration, test-bundle `-iframework` /
XCTestSwiftSupport); the residuals are the visionOS scheme-coverage capture
gap and a multi-arch Release capture artifact.

**Updated 2026-06-11 (roadmap):** a fresh full-oracle run found the settings
resolver at structural 100% on five of six oracles and 99.999% on the corpus
oracle — exactly 2 of ~431k scored keys fail structurally, both
`CLANG_COVERAGE_MAPPING` from one missing `.xctestplan` capture. The remaining
work to 100% is now itemized in **“Correctness roadmap to 100%”** at the end of
this file: the prioritized tracks (last-2-keys fix, geometry closure, the
itemized compiler-args gaps, ranked corpus expansion, harness hardening) with
acceptance criteria per step.

## Legend

- ✅ — at least one fixture exercises this; pointer in **Where**. Rows marked
  *hermetic, not corpus* are exercised by tests that build the layout in temp
  dirs instead of a captured fixture — covered, but with no oracle capture
  behind them.
- ⏳ — the corpus *probably* contains it (e.g. SwiftPM dep), but we haven't
  manually verified the captured `build-settings/*.json` actually reflects
  the resolved value. Confirm before claiming coverage.
- ❌ — known gap. Worth a phase-2 follow-up.
- 🚫 — explicitly out of scope per `PLAN.md` (e.g. signing, CocoaPods).

## Corpus quick reference

| Slug | What it brings to the corpus |
|---|---|
| `alamofire` | Pure-Swift library framework × iOS / macOS / tvOS / watchOS / visionOS variants; one iOS example app; xcworkspace |
| `ice-cubes` | Real-world iOS app: many SPM deps, app extensions (share / widget / action / notifications), multi-target |
| `netnewswire` | Multi-platform (macOS + iOS), Objective-C interop, Core Data, many internal Swift frameworks, share/widget extensions |
| `kingfisher` | Image library with iOS/macOS/tvOS/watchOS demo apps; xcworkspace |
| `tuist-fixtures` | 8 generated Tuist projects — buildable folders, framework+tests, ios+extensions, static frameworks, command-line tool with dynamic framework, xcstrings resources, local package with traits, custom schemes |

## Xcode versions captured

Each version is a directory under `fixtures/<slug>/xcode-<ver>/` + a matching
`xcspec-cache/xcode-<ver>/`, committed so it's validated forever. The oracle
tests score every captured version; `ORACLE_ONLY_VERSION=<ver>` isolates one
version's systematic-mismatch tally. Capturing a *new major* is the highest-ROI
coverage move — version-conditional keystone bugs (e.g. `XCODE_VERSION_MAJOR`
nested expansion) surface there and fix all versions at once (see PLAN.md
"Resolution-quality strategy").

Floors are **per Xcode version** (`assert_version_floors`), data-driven from the
first clean run; `structural` is the geometry-independent correctness signal,
`exact`/`canonical` are per-version geometry-capped. A freshly captured version
with no codified floor gets only a `structural ≥ 98` safety guard until calibrated.

| Xcode | Captured | Notes |
|---|---|---|
| `26.5.0` | full corpus (all 5 projects) | latest non-beta 26.x — refreshed from 26.0.1 (now dropped); per-target + project-defaults + iOS/tvOS/watchOS/visionOS-simulator + macOS schemes + synthetic + xcconfig; all oracle sources |
| `16.4.0` | alamofire, kingfisher (per-target + project-defaults + macOS scheme) | second major; ice-cubes incompatible (Swift-tools 6.2 manifests); iOS scheme/simulator needs the user-gated `xcodebuild -downloadPlatform iOS` |
| `15.4.0` | kingfisher, tuist-fixtures (per-target + project-defaults + macOS scheme) | third major; exposed two undomained-xcspec parser bugs (`PACKAGE_TYPE`/`BUNDLE_FORMAT` clobber, now fixed) and a family of 16+-calibrated built-in rules that misfired on 15.x (device bitcode strip, `STRIP_INSTALLED_PRODUCT`, `SUPPORTED_PLATFORMS` pair order, synthesized `$(BUILT_PRODUCTS_DIR)` search paths, no-destination ARCHS collapse, `ENABLE_PREVIEWS`, swift-testing plugin path — now version-gated, see `tests/version_and_optimization_gates.rs`); alamofire/netnewswire/ice-cubes walled off (objectVersion 76/77, Swift-tools 6.2); the 15.x host/arch reporting family is now modelled as version-gated rules too (arm64e `NATIVE_ARCH`/`HOST_ARCH` on Apple Silicon, concrete `CURRENT_ARCH`/`arch` = last of resolved `ARCHS`, the legacy per-SDK `VALID_ARCHS` lists, empty `LOCROOT`/`LOCSYMROOT`, no `BUILD_ACTIVE_RESOURCES_ONLY` flip, no Catalyst `SUPPORTED_PLATFORMS` append / 13.1 deployment floor, and the watch UI-test XCTRunner wrapping — see the `legacy_xcode15` gates in `src/project.rs`) |

## Project shapes

| Test case | Status | Where |
|---|---|---|
| Single `.xcodeproj`, no workspace | ✅ | fixtures/ice-cubes/xcode-26.0.1/metadata/list.json and fixtures/netnewswire/xcode-26.0.1/metadata/list.json |
| `.xcworkspace` wrapping one project | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/list.json (workspace=Alamofire), fixtures/kingfisher/.../list.json… |
| `.xcworkspace` wrapping multiple projects | ✅ | fixtures/alamofire/xcode-26.0.1/raw/Alamofire.xcworkspace/contents.xcworkspacedata |
| Nested sub-`.xcodeproj` referenced from a parent project | ✅ | fixtures/alamofire/xcode-26.0.1/raw/Example/iOS Example.xcodeproj/project.pbxproj (lines 102, 241-242,… |
| Swift package as root project (Package.swift only, no xcodeproj) | ❌ | — |
| Buildable Folders (Xcode 16+ groupless folders) | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/raw/examples_xcode_generated_app_with_buildable_folders/App.xcodeproj/project.pb… |

## Target / product types

| Test case | Status | Where |
|---|---|---|
| iOS app | ✅ | fixtures/ice-cubes/.../schemes/IceCubesApp/build-settings/*iOS-Simulator*.json |
| macOS app | ✅ | fixtures/netnewswire/xcode-26.0.1/metadata/schemes/NetNewsWire/build-settings/Release__macOS.json |
| watchOS WatchKit app | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_watchapp2/schemes/WatchApp/build-settings/Release__… |
| tvOS app | ✅ | fixtures/kingfisher/xcode-26.0.1/metadata/schemes/Kingfisher-tvOS-Demo/build-settings/Release__tvOS-Simulator_OS26.0_… |
| visionOS app | ✅ | fixtures/kingfisher/xcode-26.0.1/metadata/schemes/Kingfisher-Demo/build-settings/Release__visionOS-Simulator_OS26.0_A… |
| Dynamic framework | ✅ | fixtures/alamofire/.../schemes/Alamofire iOS/build-settings/*.json |
| Static framework | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_static_frameworks/schemes/AppTestsSupport/build-set… |
| Static library (`.a`) | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_static_libraries/schemes/iOSAppWithTransistiveStati… |
| Dynamic library (`.dylib`) | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_command_line_tool_with_dynamic_library/schemes/DynamicLib/build-… |
| Resource bundle | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_extensions/schemes/Bundle/build-settings/Release__m… |
| Command-line tool (macOS) | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_command_line_tool_with_dynamic_framework/schemes/CommandLineTool… |
| Unit test target | ✅ | fixtures/alamofire/.../schemes/Alamofire iOS Tests/... |
| UI test target | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_watchapp2/schemes/App-Workspace/build-settings/Rele… |
| App extension — share | ✅ | fixtures/ice-cubes/.../IceCubesShareExtension build-settings |
| App extension — widget | ✅ | fixtures/ice-cubes/.../IceCubesAppWidgetsExtensionExtension build-settings |
| App extension — action | ✅ | fixtures/ice-cubes/.../IceCubesActionExtension build-settings |
| App extension — notification service | ✅ | fixtures/ice-cubes/.../IceCubesNotifications build-settings |
| App extension — intent | ✅ | fixtures/netnewswire/.../schemes/NetNewsWire iOS Intents Extension/build-settings/*.json |
| XPC service | ❌ | — |
| DriverKit driver | ❌ | — |
| Mac Catalyst (iOS app on macOS) | ✅ | fixtures/ice-cubes/xcode-26.0.1/metadata/schemes/IceCubesApp/build-settings/Release__macOS.json |

## Configurations & xcconfig

| Test case | Status | Where |
|---|---|---|
| `Debug` configuration | ✅ | fixtures/*/xcode-26.0.1/metadata/schemes/*/build-settings/Debug__*.json (250 files) |
| `Release` configuration | ✅ | fixtures/*/xcode-26.0.1/metadata/schemes/*/build-settings/Release__*.json (250 files) |
| Custom configuration (e.g. `Profile`) | ✅ | fixtures/_synthetic-custom-config/xcode-*/captures/Scratch__Profile.json — a synthetic project with a third config `Profile` carrying a per-config pbxproj marker + a `[config=Profile]` xcconfig override (`tests/custom_configuration_oracle.rs`) |
| `.xcconfig` referenced from project | ✅ | fixtures/netnewswire/xcode-26.0.1/raw/NetNewsWire.xcodeproj/project.pbxproj +… |
| `.xcconfig` includes another `.xcconfig` (`#include`) | ✅ | fixtures/netnewswire/xcode-26.0.1/raw/xcconfig/*.xcconfig +… |
| Per-target overrides on top of shared xcconfig | ✅ | fixtures/netnewswire/xcode-26.0.1/metadata/schemes/NetNewsWire-iOS/build-settings/Debug__iOS-Simulator_OS26.0.1_iPad-… |
| Conditional override `setting[sdk=*] = ...` | ✅ | fixtures/netnewswire/xcode-26.0.1/raw/xcconfig/common/NetNewsWire_codesigning_common.xcconfig +… |
| Conditional override `setting[arch=arm64] = ...` | ✅ | fixtures/_synthetic-xcconfigs/xcode-26.0.1/xcconfigs/conditional-arch.xcconfig +… |
| Per-configuration override on a single target | ✅ | fixtures/netnewswire/xcode-26.0.1/metadata/schemes/NetNewsWire/build-settings/{Debug,Release}__macOS.json |

## Settings inheritance & substitution

| Test case | Status | Where |
|---|---|---|
| `$(inherited)` propagation | ✅ | fixtures/netnewswire/xcode-26.0.1/metadata/schemes/NetNewsWire/build-settings/Debug__macOS.json |
| `$(SRCROOT)` / `$(PROJECT_DIR)` substitution | ✅ | fixtures/netnewswire/xcode-26.0.1/metadata/schemes/NetNewsWire/build-settings/Debug__macOS.json |
| `$(TARGET_NAME)`, `$(PRODUCT_NAME)` substitution | ✅ | fixtures/netnewswire/xcode-26.0.1/metadata/schemes/NetNewsWire/build-settings/Debug__macOS.json |
| `$(BUILT_PRODUCTS_DIR)` cross-target reference | ✅ | fixtures/*/xcode-26.0.1/metadata/.../*.json (BUILT_PRODUCTS_DIR present in 620 files) |
| Recursive substitution (variable referencing variable) | ✅ | fixtures/netnewswire/xcode-26.0.1/metadata/schemes/NetNewsWire/build-settings/{Debug,Release}__macOS.json |
| `${VAR:default=…}` modifier syntax | ✅ | fixtures/_synthetic-xcconfigs/xcode-26.0.1/{xcconfigs/modifier-syntax.xcconfig,captures/modifier-syntax/with.json} +… |
| Lower/upper-case `${VAR:lower}` modifiers | ✅ | fixtures/_synthetic-xcconfigs/xcode-26.0.1/{xcconfigs/modifier-syntax.xcconfig,captures/modifier-syntax/with.json} +… |
| Settings with whitespace in values | ✅ | fixtures/netnewswire/xcode-26.0.1/metadata/schemes/NetNewsWire/build-settings/Debug__macOS.json |
| Settings with quotes | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/schemes/Alamofire iOS/build-settings/Release__macOS.json (and other… |
| Multi-line settings (e.g. xcconfig backslash continuation) | ✅ | fixtures/_synthetic-xcconfigs/xcode-26.0.1/{xcconfigs/multi-line-continuation.xcconfig,captures/multi-line-continuati… |

## Schemes

| Test case | Status | Where |
|---|---|---|
| Shared scheme under `xcshareddata/xcschemes/` | ✅ | fixtures/alamofire/xcode-26.0.1/raw/Alamofire.xcodeproj/xcshareddata/xcschemes/Alamofire iOS.xcscheme (and every… |
| User scheme under `xcuserdata/` | ✅ | hermetic, not corpus: tests/build_settings.rs (`user_scheme_in_xcuserdata_resolves`) + src/scheme.rs discovery tests build the `xcuserdata/<user>.xcuserdatad/xcschemes` layout in temp dirs |
| Autocreated per-target schemes (no `.xcscheme` on disk) | ✅ | hermetic, not corpus: src/workspace.rs (`merged_schemes_autocreates_per_target_when_no_scheme_files`) + src/project.rs schemeless-project tests (no corpus project ships schemeless) |
| Scheme with multiple build entries | ✅ | fixtures/alamofire/xcode-26.0.1/raw/Alamofire.xcodeproj/xcshareddata/xcschemes/Alamofire iOS.xcscheme |
| Scheme with pre-action script | ✅ | fixtures/netnewswire/xcode-26.0.1/raw/NetNewsWire.xcodeproj/xcshareddata/xcschemes/NetNewsWire-iOS.xcscheme |
| Scheme with post-action script | ❌ | — |
| Scheme with environment variables | ✅ | fixtures/alamofire/xcode-26.0.1/raw/Example/iOS Example.xcodeproj/xcshareddata/xcschemes/iOS Example.xcscheme |
| Scheme with launch arguments | ❌ | — |
| Scheme with custom test plan (`.xctestplan`) | ✅ | fixtures/alamofire/xcode-26.0.1/raw/Alamofire.xcodeproj/xcshareddata/xcschemes/Alamofire iOS.xcscheme +… |
| Scheme using parallel testing config | ❌ | — |
| Scheme overriding `buildImplicitDependencies` | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/raw/examples_xcode_generated_app_with_custom_scheme/App.xcodeproj/xcshareddata/x… |

## SDKs / Platforms

| Test case | Status | Where |
|---|---|---|
| `iphoneos` | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/schemes/iOS Example/build-settings/Release__macOS.json |
| `iphonesimulator` | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/schemes/Alamofire… |
| `macosx` | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/schemes/Alamofire macOS/build-settings/Release__macOS.json |
| `watchos` / `watchsimulator` | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/schemes/Alamofire… |
| `appletvos` / `appletvsimulator` | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/schemes/Alamofire… |
| `xros` / `xrsimulator` (visionOS) | ✅ | fixtures/ice-cubes/xcode-26.0.1/metadata/schemes/IceCubesApp/build-settings/Debug__visionOS-Simulator_OS26.0_Apple-Vi… |
| Mac Catalyst variant | ✅ | fixtures/ice-cubes/xcode-26.0.1/metadata/schemes/IceCubesApp/build-settings/Debug__macOS.json |
| DriverKit | ❌ | — |

## Architectures

| Test case | Status | Where |
|---|---|---|
| `arm64` (iOS device / Apple Silicon Mac) | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/schemes/iOS Example/build-settings/Release__macOS.json |
| `x86_64` (older sims) | ✅ | fixtures/ice-cubes/xcode-26.0.1/metadata/schemes/IceCubesApp/build-settings/Release__iOS-Simulator_OS26.0.1_iPad-A16.… |
| `ARCHS_STANDARD` resolution | ✅ | fixtures/ice-cubes/xcode-26.0.1/metadata/schemes/IceCubesApp/build-settings/{Debug,Release}__iOS-Simulator_OS26.0.1_i… |
| `EXCLUDED_ARCHS` per platform | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/schemes/watchOS Example WatchKit… |
| `arm64e` (production iPhones) | ✅ | fixtures/alamofire/xcode-*/metadata/_synthetic/archs-arm64e/build-settings/*.json (ARCHS=arm64e override; `tests/synthetic_override_oracle.rs`) |
| Universal binary (macOS arm64 + x86_64) | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/schemes/Alamofire macOS/build-settings/Release__macOS.json |

## Linking

| Test case | Status | Where |
|---|---|---|
| Embed dynamic framework into app | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/raw/examples_xcode_generated_ios_app_with_static_frameworks/App.xcodeproj/projec… |
| Link static framework (no embed) | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/metadata/examples_xcode_generated_ios_app_with_static_frameworks/schemes/A/build… |
| Dynamic library link (`.dylib`) | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/raw/examples_xcode_generated_command_line_tool_with_dynamic_library/CommandLineT… |
| Static library link (`.a`) | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/raw/examples_xcode_generated_ios_app_with_static_libraries/iOSAppWithTransistive… |
| `OTHER_LDFLAGS` extra args | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/schemes/Alamofire iOS/build-settings/*.json (and Tuist static_libraries… |
| `LD_RUNPATH_SEARCH_PATHS` defaults | ✅ | fixtures/*/xcode-26.0.1/metadata/**/build-settings/*.json (under corpus_oracle) |
| Mergeable libraries (`MERGEABLE_LIBRARY=YES`) | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/_synthetic/mergeable-library/build-settings/*.json… |
| Link-time optimization (`LLVM_LTO`) | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/_synthetic/llvm-lto/build-settings/*.json and… |
| Optional / weak framework link | ❌ | — |
| `-framework` vs `-l` flag styles | ❌ | — |

## Resources

| Test case | Status | Where |
|---|---|---|
| Asset catalog (`.xcassets`) | ✅ | fixtures/netnewswire/xcode-26.0.1/raw/NetNewsWire.xcodeproj/project.pbxproj (Resources/Assets.xcassets) |
| AppIcon set | ✅ | fixtures/ice-cubes/.../metadata/_per_target/*.json and many build-settings JSONs (ASSETCATALOG_COMPILER_APPICON_NAME) |
| `xcstrings` localization (Xcode 15+) | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_static_framework_with_xcstrings/StaticFramework/Sta… |
| Legacy `.strings` files | ✅ | fixtures/netnewswire/xcode-26.0.1/raw/NetNewsWire.xcodeproj/project.pbxproj (Intents.strings) |
| Storyboard | ✅ | fixtures/netnewswire/xcode-26.0.1/raw/NetNewsWire.xcodeproj/project.pbxproj |
| XIB | ✅ | fixtures/netnewswire/xcode-26.0.1/raw/NetNewsWire.xcodeproj/project.pbxproj |
| Core Data `.xcdatamodeld` | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/raw/examples_xcode_generated_ios_app_with_coredata/*.xcodeproj/project.pbxproj… |
| CloudKit schema in `.xcdatamodeld` | ❌ | — |
| Core ML `.mlmodel` | ❌ | — |
| Metal shader `.metal` | ❌ | — |
| Embedded `.bundle` resource | ❌ | — |
| Privacy manifest `PrivacyInfo.xcprivacy` | ✅ | fixtures/alamofire/xcode-26.0.1/raw/Alamofire.xcodeproj/project.pbxproj (PrivacyInfo.xcprivacy in Resources) and… |
| Loose files copied via Build Phase | ✅ | fixtures/kingfisher/.../Kingfisher.xcodeproj/project.pbxproj (PBXCopyFilesBuildPhase copying PrivacyInfo.xcprivacy) |

## Swift specifics

| Test case | Status | Where |
|---|---|---|
| `SWIFT_VERSION` declaration | ✅ | fixtures/ice-cubes/.../IceCubesApp/build-settings (and 754 captures) |
| Mixed Swift + Objective-C target | ✅ | fixtures/netnewswire/xcode-26.0.1/metadata/_per_target/NetNewsWire/NetNewsWire-iOS__Debug.json + corpus/netnewswire |
| Objective-C bridging header | ✅ | fixtures/netnewswire/.../_per_target/NetNewsWire/NetNewsWire-iOS__Debug.json |
| `BUILD_LIBRARY_FOR_DISTRIBUTION = YES` | ✅ | fixtures/alamofire/xcode-26.0.1/metadata/_synthetic/library-evolution/build-settings/Release__platform-iOS-Simulator_… |
| Strict concurrency (`SWIFT_STRICT_CONCURRENCY=complete`) | ✅ | fixtures/ice-cubes/.../IceCubesApp/build-settings/Debug__visionOS-Simulator_OS26.0_Apple-Vision-Pro.json |
| Swift macros (`.swift` macro target) | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_dynamic_frameworks_linking_static_frameworks/scheme… |
| Swift package traits (4.16+) | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_app_with_local_package_with_traits/schemes/App |
| `@testable import` | ✅ | fixtures/ice-cubes/.../schemes/ModelsTests/build-settings |
| Custom Swift compiler flags | ✅ | fixtures/netnewswire/.../NetNewsWire Share Extension/build-settings/*.json |

## Build phases / scripts

| Test case | Status | Where |
|---|---|---|
| Compile Sources phase | ✅ | fixtures/alamofire/xcode-26.0.1/raw/Alamofire.xcodeproj/project.pbxproj |
| Copy Bundle Resources phase | ✅ | fixtures/alamofire/xcode-26.0.1/raw/Example/iOS Example.xcodeproj/project.pbxproj |
| Embed Frameworks phase | ✅ | fixtures/kingfisher/.../Demo/Kingfisher-Demo.xcodeproj/project.pbxproj and fixtures/alamofire/.../watchOS… |
| Headers phase (public/project/private) | ❌ | — |
| Run Script phase | ✅ | fixtures/netnewswire/xcode-26.0.1/raw/NetNewsWire.xcodeproj/project.pbxproj |
| Run Script with input/output files declared | ❌ | — |
| Build rule (custom file extension → command) | ❌ | — |
| Generated source files (`.intentdefinition`, etc.) | ✅ | fixtures/netnewswire/xcode-26.0.1/raw/NetNewsWire.xcodeproj/project.pbxproj (Intents.intentdefinition) |

## Dependencies

| Test case | Status | Where |
|---|---|---|
| Same-target dependency | ✅ | fixtures/ice-cubes/xcode-26.0.1/raw/IceCubesApp.xcodeproj/project.pbxproj |
| Cross-project dependency (sub-xcodeproj) | ✅ | fixtures/alamofire/xcode-26.0.1/raw/Example/iOS Example.xcodeproj/project.pbxproj + metadata/schemes/iOS… |
| Workspace cross-project dependency | ✅ | fixtures/alamofire/xcode-26.0.1/raw/Alamofire.xcworkspace/contents.xcworkspacedata + .../Example/iOS… |
| SPM remote dependency | ✅ | fixtures/ice-cubes/xcode-26.0.1/raw/IceCubesApp.xcodeproj/project.pbxproj +… |
| SPM local dependency | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/raw/examples_xcode_generated_app_with_local_package_with_traits/App.xcworkspace/… |
| SPM target with `static` linkage | ✅ | corpus/netnewswire/Modules/RSCore/Package.swift +… |
| SPM target with `dynamic` linkage | ✅ | corpus/netnewswire/Modules/RSCore/Package.swift +… |
| SPM target with `auto` linkage | ✅ | corpus/ice-cubes/Packages/Env/Package.swift + fixtures/ice-cubes/xcode-26.0.1/raw/IceCubesApp.xcodeproj/project.pbxproj |
| Binary XCFramework dependency | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/raw/examples_xcode_generated_ios_app_with_dynamic_frameworks_linking_static_fram… |
| System framework (e.g. `UIKit.framework`) | ✅ | fixtures/ice-cubes/xcode-26.0.1/raw/IceCubesApp.xcodeproj/project.pbxproj (and corpus/ice-cubes mirror) |
| Optional framework (weak link) | ❌ | — |

## Info.plist & entitlements

| Test case | Status | Where |
|---|---|---|
| Info.plist explicitly listed (`INFOPLIST_FILE`) | ✅ | fixtures/netnewswire/.../build-settings (INFOPLIST_FILE=iOS/Resources/Info.plist) |
| Info.plist generated from build settings (Xcode 13+ "Generate from build settings") | ✅ | fixtures/ice-cubes/.../schemes/IceCubesApp/build-settings |
| `.entitlements` file referenced | ✅ | fixtures/netnewswire/.../build-settings (CODE_SIGN_ENTITLEMENTS=iOS/Resources/NetNewsWire.entitlements) |
| App Groups entitlement | ✅ | corpus/ice-cubes/IceCubesApp/App/IceCubesApp.entitlements |
| iCloud / CloudKit entitlement | ✅ | corpus/netnewswire/iOS/Resources/NetNewsWire.entitlements |
| Push notifications entitlement | ✅ | corpus/ice-cubes/IceCubesApp/App/IceCubesApp.entitlements |
| Keychain Sharing entitlement | ✅ | corpus/ice-cubes/IceCubesApp/App/IceCubesApp.entitlements |

## Tuist-specific resolution shapes

| Test case | Status | Where |
|---|---|---|
| `Project.swift` → `.xcodeproj` generation parity | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/raw/examples_xcode_generated_*/ (16 fixtures, each with a Tuist-generated… |
| `Workspace.swift` workspace generation | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/.../examples_xcode_generated_ios_app_with_custom_configuration… |
| Tuist plugins | ❌ | — |
| Tuist + remote SPM | ✅ | fixtures/tuist-fixtures/xcode-26.0.1/.../examples_xcode_generated_ios_app_with_spm_dependencies (build-settings… |

## Out of scope (per PLAN.md phase 1)

- 🚫 Archive / Release-signed builds
- 🚫 Code signing identities, provisioning profiles
- 🚫 CocoaPods integration
- 🚫 Carthage integration
- 🚫 Multi-host CI capture
- 🚫 Public API or schema design for the resolver

## Phase-2 corpus expansion (queued)

Reconciled against the captured corpus — several queued items are already
satisfied by existing fixtures (their resolution behaviour is what we score, so a
dedicated whole-project clone would add no resolver coverage):

- 🚫 A CocoaPods-using project — CocoaPods is out of scope (see below), so this is
  not a gap; left here only to retire the contradictory ❌.
- ✅ An app using mergeable libraries — `_synthetic/mergeable-library`
  (MERGEABLE_LIBRARY=YES, `tests/synthetic_override_oracle.rs`).
- ✅ A project using `PrivacyInfo.xcprivacy` — alamofire + kingfisher ship one
  (see the Resources table).
- ✅ A project with SPM trait combinations — tuist `app_with_local_package_with_traits`
  (see the Swift specifics table).
- ✅ A standalone command-line tool — tuist `command_line_tool_with_*` variants
  cover the product type (a *non-generated* CLI would only re-exercise it).
- ◐ A large app with many extensions — every extension product type (share /
  widget / action / notification / intent) is already ✅ via ice-cubes +
  netnewswire; another large app would be incremental, not new coverage.
- ❌ A project with a custom build rule — structural pbxproj feature; doesn't
  change the resolved build-settings dictionary (low resolver value).
- ❌ A project with a `.metal` / `.mlmodel` resource — adds a compile rule, not
  settings (`MTL_*` defaults already appear corpus-wide regardless).

## Verification workflow

For each ⏳ row above:

1. Open the relevant `fixtures/<slug>/xcode-26.0.1/metadata/schemes/<S>/build-settings/<C>__<D>.json`.
2. Grep for the variable / setting name claimed by the row (e.g. `BUILD_LIBRARY_FOR_DISTRIBUTION`).
3. If present with a non-default value, flip ⏳ → ✅ and record the exact JSON path in **Where**.
4. If absent, either pick a different fixture or downgrade to ❌.

This file is the source of truth for "do we have a snapshot of <thing>?" —
update it whenever we add or drop a fixture.

## Additional data sources

Beyond the per-scheme build-settings captures in
`metadata/schemes/<S>/build-settings/`, the corpus also contains:

| Source | Path | Purpose | Test status |
|---|---|---|---|
| Global per-SDK metadata | `fixtures/_global/xcode-<ver>/sdks/<sdk>.json` | xcrun `--show-sdk-{path,version,build-version,platform-path}` per SDK + `xcodebuild -version -sdk`. | dormant |
| xcodebuild version | `fixtures/_global/xcode-<ver>/defaults/xcodebuild-version.txt` | Full version banner. | dormant |
| Project-default settings | `metadata/<sub>/_project_defaults/<proj>/project-only*.json` | `xcodebuild -showBuildSettings -json -project P` with NO scheme/target. The "what does the project contribute before any target is selected" layer. | ✅ `tests/project_defaults_oracle.rs` — 83 files, 87% exact / 88% canonical / 99% structural. |
| Per-target settings | `metadata/<sub>/_per_target/<proj>/<target>__<config>.json` | `xcodebuild -showBuildSettings -json -project P -target T -configuration C`. Isolated target view — no scheme aggregation. | ✅ `tests/per_target_oracle.rs` — 150 files, 87% exact / 88% canonical / 99% structural. |
| Synthetic build-setting overrides | `metadata/_synthetic/<override>/build-settings/*.json` | `KEY=VALUE` xcodebuild overrides for flags no real project enables (library evolution, LTO, arm64e, Swift 6, …). See `scripts/07_synthetic_overrides.py`. | ✅ `tests/synthetic_override_oracle.rs` — 26 files, 87% exact / 98% canonical / 99% structural. |
| xcconfig resolution view | `metadata/_xcconfig_resolution/*.json` (+ `.meta.json`) | `xcodebuild -xcconfig FILE -showBuildSettings` per `.xcconfig`. Reveals what each xcconfig actually contributes after conditionals/includes/modifiers resolve. | ✅ `tests/xcconfig_resolution_oracle.rs` — 23 files, 88% exact / 99% canonical / 99% structural. |
| Synthetic xcconfig probes | `fixtures/_synthetic-xcconfigs/xcode-<ver>/{xcconfigs,captures}/` | Hand-crafted xcconfigs exercising `[arch=…]`, `[config=…]`, modifier syntax, multi-line continuation, `#include`. Captured with and without the xcconfig layered on. | ✅ `tests/resolver.rs` (synthetic xcconfig cases). |
| Custom-configuration probe | `fixtures/_synthetic-custom-config/xcode-<ver>/` | A synthetic macOS tool defining a third config `Profile`; per-config no-destination `-showBuildSettings` captures. Exercises config-name-driven selection + a `[config=Profile]` xcconfig override (a path no real corpus project has). See `scripts/15_custom_configuration.py`. | ✅ `tests/custom_configuration_oracle.rs` — 9 files (Debug/Release/Profile × 15.4/16.4/26.5), explicit marker assertions + per-version structural floors (96% on 15.4, 99% on 16.4/26.5). |
| PIF cache dumps | `fixtures/<slug>/xcode-<ver>/pif/{workspace,project,target}/` | Xcode's normalized JSON intermediate representation, copied from DerivedData. Cleaner to parse than pbxproj. | dormant |
| xcspec snapshots | `xcspec-cache/xcode-<ver>/` | The static spec layer — documented defaults and inheritance rules for every build setting Xcode knows about. From `scripts/04_snapshot_xcspecs.py`. | ✅ `tests/xcspec.rs` (catalog + scratch oracle). |

### Oracle sources under test

So future sessions can see at a glance which captured sources actually drive a
snapshot test versus which are checked into the repo but not yet consumed:

| Oracle source | Test | What it exercises |
|---|---|---|
| Per-scheme build-settings | `tests/corpus_oracle.rs` | Full scheme-aggregated resolution — the headline oracle (exact ≥87, canonical ≥96, structural ≥99). |
| Per-target | `tests/per_target_oracle.rs` | Isolated single-target layer stack, no scheme aggregation / destination — the cleanest analog to `corpus_oracle`, surfaces gaps scheme aggregation masks. |
| Project-defaults | `tests/project_defaults_oracle.rs` | `-project P` with no target — xcodebuild's default-target view. Path-root drift (project-relative `build/` vs DerivedData) pushes the `BUILD_DIR`/`*_SEARCH_PATHS` family into the structural tier, hence canonical 88% < corpus's 96% (documented gap, not a bug). |
| Real-xcconfig | `tests/xcconfig_resolution_oracle.rs` | `resolver::flatten_xcconfig` on real project `.xcconfig`s via `BuildContext::with_extra_xcconfig` — `#include`, conditionals, `$(inherited)`, modifiers. |
| Synthetic-override | `tests/synthetic_override_oracle.rs` | `ResolveQuery::with_override(KEY,VALUE)` for flags no real project sets (library evolution, LTO, arm64e, Swift 6, …). |
| Synthetic xcconfig probes | `tests/resolver.rs`, `tests/xcconfig.rs` | Hand-crafted xcconfig edge cases (`[arch=…]`, `[config=…]`, modifiers, continuation, `#include`). |
| Custom-configuration | `tests/custom_configuration_oracle.rs` | A project defining a config named neither Debug nor Release (`Profile`); proves config-name selection of the right `XCBuildConfiguration` (per-config pbxproj marker) + a `[config=Profile]` xcconfig conditional firing under a non-stock name. |
| xcspec catalog | `tests/xcspec.rs` | Static spec-layer defaults + inheritance against a captured scratch oracle. |

**Still dormant** (captured in the repo, no test consumes them yet): PIF cache
dumps, dry-run captures, tool-invocation captures, the global per-SDK metadata,
and the xcodebuild version banner.

Each oracle test shares `tests/common/mod.rs` (the JSON reader, `canonicalize_value`
+ `canon_*` helpers, corpus walk + `find_xcodeproj_between`, `Stats`, `compare`,
`print_summary`) via `mod common;`, prints a per-key systematic-mismatch tally plus
a canonical-only (path-root drift) tally so each is a diagnostic and not just
pass/fail, and asserts floors set just under its observed pass rate. The lone
documented per-source skips are fixture-capture gaps (e.g. ice-cubes references an
`IceCubesApp.xcconfig` baseConfiguration that was never captured into `raw/`), not
resolver gaps — never fabricated as a pass.

## Correctness roadmap to 100% (added 2026-06-11)

This section is the prioritized work queue for closing the remaining gap between
the resolver and `xcodebuild`. It is grounded in a fresh full-oracle run on this
date — re-measure (`cargo test --release --test <oracle> -- --nocapture`) before
acting, and re-rank if the numbers moved.

### Measured baseline (2026-06-11, all tests green)

Settings-resolution oracles — shared keys scored exact / canonical / structural:

| Oracle | Files | Shared keys | Exact | Canon | Struct | Systematic mismatches |
|---|---|---|---|---|---|---|
| corpus (per-scheme) | 390 | 190,193 | 88% | 96% | 99.999% | `CLANG_COVERAGE_MAPPING` ×2 — the **only** one |
| per-target | 294 | 136,194 | 88% | 91% | **100%** | none |
| project-defaults | 165 | 75,744 | 88% | 91% | **100%** | none |
| synthetic-override | 26 | 12,561 | 88% | 99% | **100%** | none |
| real-xcconfig | 25 | 12,843 | 88% | 99% | **100%** | none |
| custom-config | 9 | 3,291 | 85% | 89% | **100%** | none |
| discovery (`-list`) | 30 | 30 containers | — | — | **100%** exact (sets + ordering) | none |

**Across all six settings oracles, exactly 2 of ~431,000 scored keys fail
structurally** — both `CLANG_COVERAGE_MAPPING` on the Alamofire visionOS scheme,
a known *capture* gap (its `TestAction` points at a `.xctestplan` never copied
into `raw/`), not a resolver bug. The exact/canonical deficits are dominated by
two documented geometry families (see Track B), not value errors.

Compiler-args oracle (structural, per Xcode-version × platform): swift 97–100 %
everywhere except `xros` 94 %; clang 95–98 %; link 99–100 % except `xros` 92 %.
Precision 94–100 % per cell. The per-flag tallies behind those numbers are short
and specific — every one is itemized in Track C.

### Definition of "100 % correct"

- **Hard target:** structural 100 % + an empty systematic-mismatch tally on every
  oracle, every version. Structural is geometry-independent, so 100 % here means
  "every value xcodebuild computes, we compute" — this is achievable and Tracks
  A + C get there.
- **Exact/canonical are geometry-capped** (we resolve against `fixtures/raw/`,
  the oracle was captured at the original checkout; `CCHROOT` embeds the capture
  host's Darwin cache dir). They can be pushed into the high 90s by closing the
  two known drift families (Track B) but byte-for-byte 100 % exact is not a
  meaningful goal across machines — canonical is the cross-machine ceiling.
- Two documented irreducibles stay documented unless Track E1 disproves them:
  `ENABLE_DEBUG_DYLIB` (opaque xcodebuild Release heuristic) and the 15.x
  host/arch reporting family (already version-gated).

### Track A — settings resolver: the last 2 keys (highest value, small)

1. **Close `CLANG_COVERAGE_MAPPING` ×2.** Extend the raw-input copy list in
   `scripts/02_capture_metadata.py` to include scheme-referenced `.xctestplan`
   files (the 5 plans already exist under `corpus/alamofire/`, they were just
   never copied to `raw/`), re-capture alamofire 26.5 raw inputs, then derive
   `CLANG_COVERAGE_MAPPING` from the plan's `codeCoverage` flag during scheme
   resolution (`src/scheme.rs` / `src/build_context.rs`). _Acceptance:_ corpus
   systematic tally prints empty; corpus 26.5 structural floor raised to 100
   (`CORPUS_FLOOR_2650` in `tests/corpus_oracle.rs`).
2. **Audit the 34 skipped corpus captures** ("target/project lookup failed" in
   `tests/corpus_oracle.rs`) + 2 per-target skips. Every silent skip is unscored
   surface where a bug can hide. For each: fix the lookup, or move it to an
   explicit documented-skip list with the reason printed. _Acceptance:_ skipped
   count is 0 or every skip is named + justified in the test output.
3. **Capture `IceCubesApp.xcconfig`** — the baseConfiguration ice-cubes
   references that was never copied into `raw/` (the lone documented per-source
   skip in `tests/xcconfig_resolution_oracle.rs` / `project_defaults_oracle.rs`).
   Same capture-script fix family as A1. _Acceptance:_ those skips removed; the
   xcconfig scores like any other.

### Track B — geometry closure: raise canonical to its real ceiling (optional)

4. **`CCHROOT` (fails canonical in every file of every oracle — ×294 per-target
   alone).** The value embeds the capture host's `/var/folders/...` Darwin user
   cache dir. `canonicalize_value` already has a darwin-user-cache rule
   (`canonicalize_darwin_user_cache` passes), yet CCHROOT still misses — find the
   form mismatch and close it. Single highest-count canonical win, pure test-
   harness work, zero resolver risk.
5. **tuist capture-root drift** (`PROJECT_DIR`/`SRCROOT`/`SOURCE_ROOT`/
   `PROJECT_FILE_PATH` ×270 each + the `BUILD_DIR`-derived family): the tuist
   oracles were captured at per-example generated checkouts whose roots the
   canonicalizer doesn't recognize (tuist canon=95 % vs ≥99 % for every other
   project). Teach it the tuist fixture layout (or record `capture_root` in each
   `meta.json` and rebase). Moves corpus canonical ~96→~99 and per-target
   canonical ~88→mid-90s, since tuist is ~76 % of all keys.
6. After B1+B2, re-measure and ratchet every canonical floor; document whatever
   residue remains (it should be only true per-machine paths).

### Track C — compiler-args: itemized structural gaps to 100 %

Every gap below is visible today in `tests/compiler_args_oracle.rs -- --nocapture`;
fix → re-run all versions → raise that `(version, platform)` floor.

7. **visionOS coverage family** (the largest cell gap: xros swift 94 / clang 95 /
   link 92): missing `-profile-generate`, `-profile-coverage-mapping`,
   `-fcoverage-mapping`, `-fprofile-instr-generate`, `-Xlinker`. Same root cause
   as A1 — the scheme's test plan enables code coverage and we never see it.
   Landing A1's `.xctestplan` derivation fixes this cell too (or re-capture
   without coverage; prefer modelling it — it's real behaviour).
8. **`.mm` sources dropped from clang compiles** (`util.mm` missing in
   `_synthetic-rich` and `_synthetic-staticlib`): a real enumeration or
   language-gating bug — ObjC++ files must reach `clang_arguments`. Investigate
   `project::target_source_files` / the `FileTypes` gate in
   `src/compiler_args.rs`.
9. **`<Product>_vers.o` missing from links** (~8 cells, one per framework
   target): `VERSIONING_SYSTEM = apple-generic` generates a `<Product>_vers.c`
   compile + link object. Model it (it's a pure function of
   `CURRENT_PROJECT_VERSION`/product name) or classify as geometry with a
   comment — decide once, apply everywhere.
10. **Test-bundle clang `-iframework <Platform>/Developer/Library/Frameworks`**
    (×1 per version): the swift generator already emits the platform test
    frameworks path; port the same rule to `clang_arguments`.
11. **Version gates:** `-explicit-module-build` emitted on 15.4/16.4 where the
    oracle has none (gate to ≥26); 15.4 clang missing `-g`/`-gmodules`
    (debug-info gating on the 15.x driver); 15.4 swift missing `-F` ×4.
12. **Known confident-wrong extras** (precision tail): `-fexceptions`
    (`GCC_ENABLE_EXCEPTIONS`), `-fvisibility=hidden`
    (`GCC_SYMBOLS_PRIVATE_EXTERN`), the one-target `-W(no-)shorten-64-to-32`
    flip. These come from resolver values `-showBuildSettings` doesn't surface —
    ground the true default in the xcspec; if genuinely underivable, document
    per-flag and exclude from precision instead of carrying a silent tail.
13. **`-target` triple mismatches** (26.5 macosx link ×1, 26.5 watchos clang ×1,
    likely `armv7k`/`arm64_32` or deployment-suffix drift): diff the literal
    triples and fix derivation.
14. **Autolinked `-framework` recall** (the one tracked link gap): frameworks
    pulled in by `import` are encoded in object files, not the project graph.
    Decide formally: out-of-scope (recommended — BSP never links, and the build
    oracle's precision is unaffected) and document it, or model an import-scan
    heuristic. Stop carrying it as an open item either way.

### Track D — corpus expansion for the remaining 18 ❌ rows (ranked)

Only rows that change resolved settings or argv earn a fixture; the rest get
hermetic parse tests or stay ❌ by design.

15. **Swift package as root** (`Package.swift`, no xcodeproj) — the biggest
    genuinely uncovered real-world surface: the discovery oracle already tallies
    41 xcodebuild-synthesized package schemes as out-of-scope. This is a *scope
    decision* first: if the BSP server should serve SPM-root repos, add a
    synthetic SPM-root fixture, capture `-list` + `-showBuildSettings` for a
    package scheme, and extend discovery/resolution to manifests. If not, record
    the 🚫 with rationale and retire the ❌.
16. **Weak/optional framework link + `-framework` vs `-l` styles** (3 ❌ rows,
    one fixture): a synthetic project with a `Weak` ATTRIBUTES framework entry +
    a `-l`-style library link, captured like `_synthetic-staticlib`
    (`scripts/17_static_library.py` is the template). Feeds the link oracle.
17. **XPC service** (and DriverKit if cheap): distinct product-type xcspec
    domains never exercised — a synthetic macOS XPC target needs no runtime and
    closes a whole product-type family. DriverKit SDK JSONs already sit in
    `_global`; capture only if a per-target capture is low-effort.
18. **Scheme post-actions / launch args / parallel testing + headers phase +
    run-script I/O files** — structure-only (don't change the settings dict):
    cover hermetically in `src/scheme.rs`/`tests/xcscheme.rs` parse tests, same
    pattern as the user-scheme rows; flip to ✅ *(hermetic, not corpus)*.
19. **Core ML / Metal / `.bundle` / custom build rules / CloudKit / Tuist
    plugins** — stay ❌/🚫; documented low resolver value (compile rules, not
    settings).

### Track E — harness hardening (keeps 100 % honest once reached)

20. **Interrogate PIF dumps for `ENABLE_DEBUG_DYLIB`** before accepting it as
    irreducible forever: the dormant PIF captures are Xcode's own normalized
    target representation — if the flag (or its real input) appears there, the
    heuristic becomes derivable. One bounded investigation, then either model it
    or close the question permanently.
21. **Wake cheap dormant sources:** `_global/.../sdks/*.json` → a small SDK
    discovery test; the version banner → trivial assert. Retire the dead
    `dry-run/` captures (632 of 654 are just the Xcode-26 "no longer supported"
    error) — superseded by the compiler-args oracle.
22. **Extend the mutation audit** (`scripts/21_mutation_audit.py`) with one row
    per rule added in Tracks A/C (coverage-mapping derivation, `.mm` enumeration,
    `-iframework`, version gates) so every new rule has a net that goes red.
23. **Ratchet floors after every fix** — that's what makes progress permanent:
    corpus 26.5 structural → 100 after A1; each compiler-args cell after its C
    item; canonical floors after Track B.
24. **Refresh the generated reports** — `fixtures/REPORT.md` and
    `fixtures/AUDIT.md` still reference the dropped `xcode-26.0.1`; re-run
    `scripts/05_validate.py` / `06_audit_coverage.py` at the next capture so the
    generated views match the corpus again.

### Suggested execution order

A1 (+C7, same root cause) → A2/A3 → C8–C13 → B4/B5 → D16 → D15 scope decision →
E20–E24 interleaved. Tracks A+C reach the hard target (structural 100 %, empty
tallies); B raises the cross-machine ceiling; D/E widen and lock what "100 %"
covers. Mac-host capture steps (A1, A3, D16, D17) need a macOS machine with the
corpus Xcodes; everything else runs on any host against committed fixtures.
