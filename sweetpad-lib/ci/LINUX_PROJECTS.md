# Linux project coverage

The follow-up suite compares pinned open-source projects with Xcode builds.
`linux-projects.json` contains the source revisions, schemes, and fixture patches.
These projects supplement the original 129-scheme corpus; they are not part of its count.
With explicit compatibility patches, 127 of 128 current corpus schemes build.
The only failure is intentional. One obsolete extension scheme merged into its watch app.

## Environment

The November 7, 2026 run used an arm64 Ubuntu 24.04 OrbStack machine.
It used Swift 6.4.2, patched Swift Build 6.4.0, and SDKs extracted from Xcode 27.2.
The native control used Apple Swift 6.4.2 and the same SDK version.
Simulator checks ran on iOS 26.0 and 27.0.
Later physical-device checks used an iPhone 13 with iOS 27.0.1.

Install the Linux tools with `linux-setup.sh` and build the CLI on both hosts.
The projects need their own dependencies before the suite can build them.
The suite checks source revisions and applies the listed fixture patches.
The separate `linux-project-prepare.py` recipe installs frozen hybrid-framework dependencies on the Mac for both build hosts.

## Results

| Project | Xcode build | Linux build | Simulator comparison |
| --- | --- | --- | --- |
| Wikipedia | Pass | Pass | Both launch after restoring their declared app-group entitlements in simulator copies |
| Mastodon | Pass | Pass | Both render the onboarding screen |
| Swiftfin | Pass | Pass | Both render the server connection screen |
| Flutter samples: platform_design | Pass | Pass | Both render the Songs screen |
| React Native community template | Pass | Pass | Both render the React Native/Hermes welcome screen after the shared scene patch |
| Ionic conference app: Capacitor | Pass | Pass | Both render the welcome screen |
| React Native MMKV example | Pass | Pass with explicit fixture patches | Both pass seven storage checks twice, including persistence after relaunch |

The MMKV fixture uses an explicit support-header shim for Linux Swift 6.4.2.
The shim suppresses forced emission of unused generic debug thunks, which causes recursive `Optional` instantiation.
It preserves all generated declarations and definitions, and restores `DEBUG` before application headers load.
The original native build passes without this workaround. Both final controls use the same patched source tree.
Both also use a scene delegate because SDK 27 traps on the original application lifecycle.

The first follow-up corpus run built 88 of 129 schemes. The action and platform
follow-up built 94 of the same 129 schemes with the updated toolchain.
Test-only schemes use `sweetpad test build`; the current sweep records the action.
The local `AccountTests` scheme passes that action.

Wikipedia's full build and all 70 XIB/storyboard compiles pass.
Its 1,040 common localization files match the native bundle.
Mastodon's final comparison matches all 4,505 common localization files.
Wikipedia's unsigned startup failure also occurred in the native control.
Its declared app groups now receive ad-hoc signatures in separate simulator copies. Both copies remain running.

The bundle checker compares inventories, all bundle Info.plists, localization values,
loose text resources, App Intents registration, and Mach-O load commands.
It normalizes the host build stamp and explicitly supplied products roots.
It preserves runtime search-path order, platform versions, and library load kinds.
It does not claim that executable code or asset archives are byte-identical.
The runtime checker installs each product, launches it, checks its process, and saves a screenshot.
Each comparison removes the existing simulator installation of that bundle. Use a test simulator.
A successful launch command alone does not count as a passing runtime check.

## Test execution and incremental coverage

The November 7 follow-up executed Linux-built products on the iOS simulator using
`sweetpad test run --xctestrun`. Executable hashes remained unchanged during each run.
The manifests were generated from explicitly selected bundles with `prebuilt_tests.py`.
They do not require a native build or a native manifest template.

| Linux-built suite | Result | Evidence |
| --- | --- | --- |
| ToolsAppTests, app-hosted XCTest | 1 pass | Linux app and test executable hashes retained |
| RSWebTests, logic XCTest | 5 pass | Independent logic-test manifest |
| TimelineTests, Swift Testing | 8 pass | Independent manifest; Swift Testing executed |
| AccountTests, app-hosted XCTest | 92 pass | Both controls use signed copies with an isolated app group |

The original unhosted AccountTests products had the same 38 keychain failures on both hosts.
An app host with an explicit app-group entitlement provides a valid simulator keychain context.
That exposed a shared `SecItemDelete` query error, status `-50`.
The fixture patch removes unsupported return and match-limit options from that query.
All 92 tests now pass on both hosts. Test assertions remain unchanged.
`linux-keychain-tests.py` preserves source products and signs separate copies.
It checks source hashes and verifies that signed executable hashes remain unchanged during testing.
Selecting a nonexistent test now fails explicitly, even when Xcode exits zero.

`fixture-linux-parity` adds a local package, package resources, app resources,
localized plurals, named image/color assets, App Intents, and a storyboard.
Its tests verify the compiled origin, values, resource contents, locale lookup,
asset loading, color metadata, intent registration, and initial scene loading.
Linux products carry a compilation marker that the tests check at runtime.
The current fixture adds SwiftUI/CoreLocation/Darwin/SQLite checks and localized shortcut model loading.

Run the comparison on a Mac with a booted simulator:

```bash
python3 -B sweetpad-lib/ci/linux-parity.py \
  --out sweetpad-lib/corpus/_linux-followups/validation/parity-run \
  --simulator BOOTED_SIMULATOR_UDID
```

Use a fresh output directory for a new run. Use `--resume` to continue its recorded phases.
The runner copies the fixture into a path with spaces and Unicode characters.
It mutates that copy and keeps the checked-in sources intact.
Each output directory selects a separate Linux DerivedData location.

The 13 phases cover a baseline, no-op rebuild, app/package source edits,
app/package resource edits, resource rename/deletion, asset color edit,
localization edit, asset deletion, deleted generated-output recovery, and a clean control.
Every phase builds on both hosts, compares bundles, and executes the prebuilt tests.
No-op executable hashes must match. Final incremental bundles must match fresh products
from the same host after products-root normalization.

The checks found and fixed stale export files, missing primary-icon validation,
canonical locale directories (`kmr`/`iw`/`in`), and accent/widget color metadata.
Explicit language filters preserve the requested spelling and resolve canonical aliases.
Mastodon's 12 catalogs now produce 14 Kurdish tables matching native values under `ku-TR.lproj`.
The storyboard comparison also found an initial scene omitted because it was a child controller.
That controller now has its own archive and entry-point registration.
A native golden fixture and a Linux-built UIKit test cover the case.

Fresh clean builds now include SwiftNIO privacy resources, static Icon Composer appearances,
sticker packs, custom-font metadata, and Swift compatibility runtimes.
Swiftfin has zero findings in the bundle checker. Both builds launch and remain running.
Mastodon has seven findings: six optional optimized `.omo` companions and one framework load-command comparison.
Wikipedia has twelve findings: seven `.omo` companions and five load-command comparisons.
MMKV has one load-command comparison finding. Its resource inventory and bundle metadata match.
The checker keeps these differences visible and does not equate them with verified runtime failures.

iOS Icon Composer support includes static light, dark, and tinted appearances, primary PNGs, and icon metadata.
Native captures verify layer order, appearance sizes, original-color fills, and shared icon/image names.
All 15 corpus `.icon` sources compile, including system backgrounds and group blend modes.
The renderer uses static system-background gradients.
It does not reproduce interactive glass lighting, native bevels, or other material effects.
Sticker support includes ordered PNG/GIF/JPEG members, accessibility labels, pack metadata, and message-extension icons.
Animated sticker sequences and localized sticker-pack strings remain unsupported.
These checks establish the recorded resource contracts, not pixel-identical artwork or full bundle equivalence.
A final Swiftfin no-op rebuild preserves all 505 bundle files after plist dictionary-order normalization.
Executable, icon, privacy-resource, and runtime-library hashes remain unchanged.

Local evidence is under `validation/`: `tests/independent-manifests.json`,
`parity-final/summary.json`, `initial-child-runtime-fixed/summary.json`,
`mastodon-locales-final/comparison.json`, and `wikipedia-storyboard/`.

## Physical-device validation

The Linux-built `ParityProbe` app passes seven tests on an iPhone 13 with iOS 27.0.1.
The tests verify the Linux compilation marker, app and package resources, assets,
storyboard loading, SwiftUI and system bindings, SQLite, and localized shortcut models.
Apple `codesign --verify --deep --strict` accepts the app and its nested code.
All four exported executable hashes remain unchanged during testing.
The app also launches after testing and appears in the device process list.

The build uses an Apple Development identity and a profile for the registered device.
The `.p12` file and profile stay outside the repository.
The identity does not require access to the Mac login keychain.
The device fixture copy removes the simulator fixture's `CODE_SIGNING_ALLOWED=NO` setting.
SDK defaults then control signing for each target.

The device build exposed two gaps that the simulator checks did not exercise.
The Linux backend now disables obsolete bitcode stripping when it copies embedded frameworks.
The signer also supports resource-only `BNDL` bundles with detached signatures.
Native reference files cover the public test identity, ad-hoc signatures, large Info.plists, and ignored executable entitlements.
Apple verification accepts the signatures and rejects modified resources.

After the signed Linux fixture build, run this command on the Mac:

```bash
python3 -B sweetpad-lib/ci/linux-device-checks.py \
  --device CONNECTED_IPHONE_UDID \
  --linux-products /home/USER/sweetpad-device-parity/Build/Products \
  --project /path/to/device-fixture/ParityProbe.xcodeproj \
  --out /path/to/fresh-device-evidence
```

The runner exports only the signed app, including its test bundle and runtime frameworks.
It installs the app with `devicectl` and runs an independent device manifest with `sweetpad test run --xctestrun`.
The Mac does not recompile these products.
The runner preserves signature results, executable hashes, test results, and process evidence.

The matched-source bundle comparison retains two debug-symbol differences.
The test dSYM has a different bundle identifier and an additional XCTest Swift interface.
The signed Linux products also contain two provisioning profiles that the unsigned native control does not contain.
App resources and decoded shortcut models match.

This run uses Mac device transport. Direct Linux device transport remains unverified.
Physical-device checks for extensions, widgets, and the larger corpus apps remain unverified.
Voice invocation and interactive Icon Composer effects also remain unverified.

## Run the suite

From the repository root on the Mac:

```bash
python3 sweetpad-lib/ci/linux-projects.py --prepare --only wikipedia
python3 sweetpad-lib/ci/linux-projects.py --only flutter --host both
python3 sweetpad-lib/ci/linux-project-checks.py --only flutter --simulator BOOTED_SIMULATOR_UDID
python3 sweetpad-lib/ci/linux-projects.py --summarize
```

Omit `--only` to select every manifest entry.
Use `--host linux` or `--host xcode` to rebuild one control.
Use `--clean` after changing a compiler replacement, because Swift Build can reuse old resources.
`--summarize` reads existing results without rebuilding.

Results stay in the ignored `corpus/_linux-followups` directory:

- `*-{linux,xcode}.json` and `.stderr`: complete CLI results.
- `*.command.json`: commands, source revisions, timing, and exit status.
- `real-projects.json`: build results and available bundle/runtime comparisons.
- `project-checks.json` and `*.png`: bundle comparisons and simulator evidence.

Source pins alone do not make setup hermetic. The fixture recipes also capture missing npm, Ruby, Pod, and Dart locks.
Receipts record tool versions, applied source patches, and final lock hashes.

## Dependency preparation

Use the same prepared source tree for both hosts. The Linux build reads generated
Pods and framework configuration from the Mac's shared filesystem.
This tests Linux compilation of those projects; it does not test CocoaPods on Linux.

### Wikipedia, Mastodon, and Swiftfin

The runner invokes Wikipedia's `bash scripts/setup_bundle_id ci` once.
SwiftPM resolves each project's checked-in package pins during the build.

Swiftfin needs SwiftFormat 0.64.6 on both hosts.
Place the Mac tool in `_linux-followups/tools/bin` and the Linux tool in `~/bin`.
The native build accepts the inspected package macros with
`-skipMacroValidation -skipPackagePluginValidation`.

Swiftfin's pinned StatefulMacros dependency lacks a Foundation import on Linux.
Apply `patches/statefulmacros-linux-foundation.patch` to its package checkout after resolution.
The affected revision is `afb27fb` from Swiftfin's package resolution.
Apply it to both controls to keep their package sources equal.
This patch belongs to the coverage fixture; sweetpad does not edit users' packages.

SwiftNIO's pinned manifests select privacy resources with the host's `canImport(Darwin)` result.
The explicit dependency patch also accepts `SWEETPAD_APPLE_TARGET=1` during destination manifest evaluation.
Host macro builds do not receive that variable.
After package resolution, prepare the separate package checkouts on both hosts:

```bash
python3 -B sweetpad-lib/ci/linux-dependency-patches.py
orb -m sweetpad-linux python3 -B "$PWD/sweetpad-lib/ci/linux-dependency-patches.py"
```

Use `--only mastodon` or `--only swiftfin` to select one project.
Use `--derived-data PATH` for another package cache and `--check` to validate without writing.
The helper checks the manifest's exact dependency revisions before applying patches.

The SDK importer now includes versioned Swift back-deployment libraries from the user's Xcode.
For an existing SDK installation, run this command from the repository root inside the Linux VM:

```bash
python3 -B sweetpad-lib/ci/linux-sdk-platforms.py \
  /Applications/Xcode.app "$HOME/.swiftpm/swift-sdks/darwin.artifactbundle" --runtime-only
```

Swift Build scans the app's Mach-O dependencies and embeds the needed Apple libraries.
The CLI selects the extracted Apple toolchain and disables obsolete bitcode stripping.

### Flutter

Use Flutter 3.47.9, revision `c197ca5199`, on both hosts.
The tested app is `flutter-samples/platform_design`.
On the Mac, run:

```bash
flutter pub get
cd platform_design
flutter build ios --simulator --config-only --no-codesign
```

Install the same Flutter revision at `~/src/flutter` in the VM and run
`flutter precache --ios` there. The manifest overrides `FLUTTER_ROOT` for Linux.
The build then uses the Linux Flutter SDK with the prepared iOS project.

### React Native

Use Node 24.22.2 and the template's Ruby Gemfile.
The tested community template is React Native 0.85.3.
Inside `react-native-template/template`, install npm dependencies and run `bundle install`.
Inside its `ios` directory, run `bundle exec pod install`.
The tested Gemfile resolved CocoaPods 1.15.2 and xcodeproj 1.25.1.

Set the generated `ios/.xcode.env.local` to:

```bash
export NODE_BINARY=$(command -v node)
```

The fixture patch moves window creation to a scene delegate and adds the scene manifest.
The unmodified template traps on both hosts under SDK 27 with
`UIKitNoSceneLifecycleAdoption`. Both controls receive the same patch.
Start Metro on port 8081 from the template directory before the simulator comparison.

For the MMKV stress case, use Bun 1.3.14 and `bun install --frozen-lockfile`
at its repository root. Run its postinstall build, then install the example's Ruby/Pod dependencies.
The pin uses React Native 0.85.3 and NitroModules 0.35.9.
Apply the MMKV fixture patches before `bundle exec pod install`.
For an existing checkout, `linux-projects.py --only react-native-mmkv --prepare-only` applies the patches without building.
Then run `bundle exec pod install` again inside `example/ios` to regenerate the Pod header-search paths.
The `SwiftCxxDebugThunks` shim affects only NitroModules and NitroMmkv.
It leaves Apple's compiler behavior intact and suppresses Linux's debug-only forced thunk emission.
No generated Swift header rewrite or higher template-depth limit is necessary.

After successful builds and `linux-project-checks.py --only react-native-mmkv`, run:

```bash
python3 -B sweetpad-lib/ci/mmkv-parity-smoke.py --simulator BOOTED_SIMULATOR_UDID
```

The probe starts its own Metro server on port 8081 and removes it afterward.
It exercises storage APIs twice per product, with process termination between launches.
It retains the existing binaries and records their hashes, runtime results, and screenshots.
The temporary Metro entry renders the original app after the checks.
It removes each existing MMKV simulator installation. Use a test simulator.

### Capacitor

Use Node 24.22.2. From `ionic-conference-app`, run:

```bash
PUPPETEER_SKIP_DOWNLOAD=1 npm ci
npm run build
npx cap sync ios
```

The iOS project uses `CapApp-SPM` and seven Capacitor plugins.

## Gaps closed by these projects

The follow-up adds native captures for relationship mapping models, legacy Core Data
compound indexes, named String Catalog formats, rich intent definitions, and localization extraction.
Wikipedia also exercises gesture recognizers, layout guides, page controllers, stack variations,
and calibrated colors in Interface Builder archives.

Hybrid projects exercise Bash script phases, UTF-16 resources, SDK C++ headers,
Darwin Clang targets, linker selection, and portable `ditto` behavior.
Swiftfin exercises optional `@Entry` values and package macros built for the Linux host.

The separate `patches/netnewswire-linux-script.patch` replaces a Darwin-only import with
a Darwin/Glibc condition in the corpus fixture. Its macOS app builds on Linux.
With the newer SDK/toolchain, its iOS targets expose a Swift weak-self capture diagnostic.
A fresh native build fails at the same source line with the same diagnostic.
Earlier SDK 27.0 results must not be presented as an unchanged control for this run.

Release packaging work remains deferred. The macOS dependency-size cleanup and
Linux release CI are outside this follow-up.

## Additional validation

The final workspace suite passes with `--test-threads=1`, and Clippy passes with warnings denied.
One parallel temporary-directory cleanup test passed on its isolated retry and in the serial suite.
The follow-up reports 798 passing macOS CLI tests and 791 passing Linux CLI tests.
Linux skips six native-only checks. Workspace Clippy passes with warnings denied.
The Python preparation, bundle, and SDK regression suite passes 13 tests.

The final `@Entry` fixture passes its default-value and assignment preconditions on both hosts.
A fresh App Intents probe matches native registration metadata after sorting input-type sets
and omitting the generator identity. Unit fixtures cover the shortcut language model.

The development archive test uses the repository's public self-signed test identity.
Apple `codesign --verify --deep --strict` accepts the archive and exported IPA.
App and widget dSYM UUIDs match their executables. This does not establish App Store eligibility
or physical-device provisioning with an Apple-issued identity.

## Test-action and platform follow-up before fixture patches

The sweep now retries schemes with no running build targets using `sweetpad test build`.
Its JSON and Markdown summaries record the action used.
All six test-only schemes were checked with this action:

| Project | Scheme | Result |
| --- | --- | --- |
| NetNewsWire | AccountTests | Pass on Linux |
| NetNewsWire | RSWebTests | Pass on Linux |
| Ice Cubes | AccountTests | Shared source error in both controls |
| Ice Cubes | ModelsTests | Shared source error in both controls |
| Ice Cubes | StatusKitTests | Shared source error in both controls |
| Ice Cubes | TimelineTests | Shared source error in both controls |

Resolved Git package checkouts now contribute their explicit scheme files.
This exposes `RevenueCatUI` in both CLI backends. It reaches compilation and fails in
its source code, where the native control also fails.
Discovery reads the selected DerivedData graph, including a build's custom path or project configuration.
It does not fetch packages during scheme listing. A cold checkout needs dependency resolution
through an existing project build before its remote scheme files are available.
Package-root PIF reads use separate scratch directories, so they preserve the app's resolved graph.
A local Git fixture verifies remote alias builds and custom DerivedData lookup.
Action configurations also come from the selected graph; both CLI backends
honor a remote scheme's Release default in a custom DerivedData directory.

`linux-sdk-platforms.py` imports tvOS, watchOS, and visionOS SDKs from the user's Xcode.
It copies platform metadata, SDK data, testing frameworks, and companion stubs.
It also extends Swift SDK target-triple metadata. The CLI selects Apple compiler resources
and the LLD driver for these platforms, scoped to the destination SDK.

The following corpus schemes now build on Linux:

- Alamofire tvOS, watchOS, and visionOS.
- Kingfisher-tvOS-Demo.

The six remaining former platform failures use deprecated WatchKit extension products.
Current Xcode rejects those products too.
The checked-in `fixture-platforms` apps compile and link on both hosts for all three new simulator platforms.
The local machine has only iOS simulator runtimes, so tvOS/watchOS/visionOS runtime and device-signing checks remain unverified.

The updated same-scheme results are in `corpus/_linux-sweep-platforms/summary.json` and `summary.md`.
Standalone probes supplement that count. The intentionally broken dependency fixture remains a failure.

That same-scheme run recorded 94 successful Linux builds. Every remaining
failure also fails in a fresh Xcode control: 24 Ice Cubes schemes blocked by
`Document` ambiguity, two NetNewsWire schemes blocked by weak-self capture,
six deprecated WatchKit schemes, one Tuist fixture missing module `C`,
one RevenueCat initializer incompatibility, and the intentionally broken dependency fixture.
These are the errors stopping the builds; they can hide further compatibility issues.

## Compatibility patches for the remaining corpus failures

The original pins remain unchanged. Explicit fixture patches repair their compatibility problems.
`linux-corpus-patches.json` records the revisions, patch order, and scheme changes.
`linux-corpus-patches.py` validates the pins and applies patches idempotently.
Run it on the Mac, then run it with `--resolved-only` in the VM.
The shared corpus needs one patch application; RevenueCat has separate resolved checkouts on each host.
See `patches/README.md` for commands and patch details.

The small repairs qualify `SwiftSoup.Document`, suppress RevenueCat's conflicting synthesized initializer,
make NetNewsWire's existing closure capture explicit, and expose the prebuilt C module to Tuist's tests.
Compilation exposed inherited main-actor isolation in two value helpers and an immutable report response.
Their explicit `nonisolated` annotations retain isolation for UI code and permit decoding in the concurrent network request.
Its Timeline test mock now follows its protocol's main-actor isolation.
Its tag warning becomes a computed view property instead of a local declaration after its use.
EnvTests also gains an explicit test-only scheme and uses `test build`.

The three legacy watch applications migrate to single app targets.
Their source code, assets, package dependencies, and framework embedding move into the app target.
Their storyboards retain their controllers and use the new app modules.
`WKApplicationDelegate` and `WKApplicationMain` replace the extension lifecycle.
All three standalone app schemes compile on both hosts.
The Tuist iOS companion builds also include their embedded watch app on Linux.
Cross-tool overrides now cover other Apple SDKs in the same request, without leaking into host macros.

The obsolete Tuist `WatchAppExtension` scheme is retired into `WatchApp`.
The original 129-row baseline therefore has 128 current schemes after migration.
The `_spm-broken` package URL remains an intentional negative fixture.
The final comparable sweep is written to `corpus/_linux-sweep-patched-final`.
To repeat its selection, pass `--baseline corpus/_linux-sweep-platforms/summary.json`
to `linux-sweep.py`. Add `--xcode-all` to check positive native controls too.
It records 127 successful Linux builds out of 128 current schemes.
All 33 repaired original failures also pass their native controls.
The original failures now comprise 33 repaired schemes, one retired scheme, and one expected failure.

The complete app build exposed additional App Intents processor gaps.
Dynamic widget metadata now remains registered in its own component, as in Xcode.
Static dependency registration remains unsupported and fails explicitly.
Native captures now cover image file inputs, content types, descriptions, input connections, and dialog/snippet capabilities.
Enum case metadata follows declaration order, independently of display-dictionary order.
The clean Linux Ice Cubes build passes.
Its app and widget registration metadata match Xcode after normalizing generator identity and resolver-set order.
Platform setup also imports `XCTRunner.app`; the migrated watch workspace builds its UI-test runner.
An existing failed build needs one clean after adding missing SDK files.

The patched Timeline mock passes all eight tests in the iPhone simulator.

NetNewsWire's patched native and Linux iOS products both render the Feeds screen in the simulator.
Their application identity and four localization files match.
Ice Cubes' clean native and Linux products both render the Add Account screen.
Their application identity and 126 common localization files match.
Startup evidence is in `corpus/_linux-followups/corpus-patch-checks`.
This does not validate account synchronization or background notifications.
The modern-icon and localized-shortcut follow-ups rebuild Ice Cubes on both hosts.
The final bundle checker has zero findings, down from 42.
Its 19 localized shortcut models match native locale, bundle, intent phrase, and negative-utterance contents.
Both products render the Add Account screen. This does not verify voice invocation through Siri.
Watch runtime and device checks remain unverified.

The MMKV reproduction now covers both Swift module import and Objective-C++ object emission.
Disabling forced debug-thunk emission resolves both stages with the pinned compiler.
The final clean application builds pass on both hosts. The compiler workaround remains an explicit fixture patch.


## Linker, preparation, runtime, and localized-shortcut follow-up

The linker wrapper now keeps SwiftUI references through the public umbrella when
object autolink commands list SwiftUICore first. It prepends an empty object with
an implicit SwiftUI autolink command. Unused frameworks still disappear, and
explicit weak-framework policy remains intact. The object stays in the user's
state directory so linker dependency files remain valid.

`linux-linking-checks.py` compares native linking, raw LLD, and the wrapper in
24 cases. They cover deployment targets 15.0, 17.6, 18.0, and 27.0, both framework
orders, unused imports, and explicit weak imports. All wrapper cases match Xcode.
Six prebuilt framework checks pass on iOS 26.0 and 27.0 on both hosts. They exercise
SwiftUI layout, CoreLocation lookup, Darwin constants, and SQLite queries.
Apple's download catalog did not provide iOS 17.5, 18.0, or 18.5 for this Xcode.
Pre-iOS-18 execution remains unverified.

A Mastodon no-op preserves all 4,794 bundle files after plist dictionary-order
normalization. Executable and resource bytes remain unchanged.
Wikipedia, Mastodon, and the hybrid fixtures retain reported optional `.omo` or
library load-command differences. The checker does not suppress those fields.

### Frozen hybrid preparation

Install the pinned Node and Bun tools in a working tool directory:

```bash
mkdir -p sweetpad-lib/corpus/_linux-followups/tools/js
cp sweetpad-lib/ci/framework-tools/package*.json sweetpad-lib/corpus/_linux-followups/tools/js/
npm ci --prefix sweetpad-lib/corpus/_linux-followups/tools/js
```

Use Ruby compatible with each project's Gemfile and Bundler 4.0.20.
Install Flutter revision `c197ca5199e9ee5661e865596c9c07294dfab51c` separately.
Then prepare a fresh tree, with no existing Pods or generated framework files:

```bash
python3 -B sweetpad-lib/ci/linux-project-prepare.py \
  --corpus-root sweetpad-lib/corpus/_linux-followups/fresh-sources
python3 -B sweetpad-lib/ci/linux-projects.py --frameworks-only \
  --corpus-root sweetpad-lib/corpus/_linux-followups/fresh-sources \
  --linux-dd-suffix=-fresh \
  --out sweetpad-lib/corpus/_linux-followups/validation/fresh-builds \
  --developer-dir /Applications/Xcode.app/Contents/Developer
```

The recipe uses `npm ci`, frozen Bun and Dart resolution, frozen Bundler resolution,
and CocoaPods deployment mode. It applies source patches before dependency generation.
React Native embeds absolute paths in generated Pod specifications. Captured
specifications replace only the checkout root before computing those checksums.
Deployment mode still validates the full Pod graph. Changed dependency versions
or unrelated lock changes fail preparation.

All four fresh hybrid fixtures build on both hosts. Their bundle resources and
metadata match; one Mach-O comparison remains per fixture. This validates Mac
preparation followed by Linux compilation. CocoaPods execution on Linux remains untested.

`linux-projects.json` also records Swiftfin's pinned StatefulMacro Foundation patch.
`linux-dependency-patches.py --check` validates it alongside the NIO patches on each host.

### Localized App Shortcuts

Mac execution of `linux-setup.sh` exports phrase resources from the user's Xcode
before importing them into the private Darwin SDK. The export stays outside source control.
For an existing SDK, refresh only these resources:

```bash
python3 -B sweetpad-lib/ci/linux-appintents-resources.py \
  --out sweetpad-lib/corpus/_linux-followups/tools/appintents-phrases.json
orb -m sweetpad-linux bash -lc \
  'cp "$1" "$HOME/.swiftpm/swift-sdks/darwin.artifactbundle/appintents-phrases.json"' \
  -- "$PWD/sweetpad-lib/corpus/_linux-followups/tools/appintents-phrases.json"
```

Add `--locale LANGUAGE` to
export a project locale beyond the default set. The current export covers 81 locales.
A missing localized resource export produces an explicit preparation error.

The trainer reads localized app names and compiled AppShortcuts string tables.
It emits `LOCALE.lproj/nlu.appintents` models and the declared temporary SSU source.
Fingerprints change with model content, remain stable across no-op training, and
match their adjacent version markers. Removed shortcuts remove their old models.
The catalog validator supports AppShortcuts string tables and catalogs. It preserves
Xcode's structured diagnostics for the eight tested acceptance and rejection cases.
AssistantIntents catalogs and catalog variations remain explicit unsupported inputs.

`linux-appintents-checks.py` compares all decoded model contents with native controls
in five locales. It checks translations, app-name changes, no-op hashes, and removal.
The checker validates model markers before normalizing producer fingerprints.
It then compares every locale, bundle identifier, intent phrase, and negative example.
Generation timestamps and phrase order do not establish compatibility.

Native retraining can leave obsolete version markers or models after removal.
The oracle retains that evidence and also checks a fresh native removal control.
Xcode's package-only incremental build can also lose the archived SSU source while
keeping correct runtime models. The incremental runner preserves that discrepancy
and requires a fresh native control before accepting the Linux product.

The project fixture includes a real localized AppShortcuts catalog and seven
simulator tests. All 13 phases pass their Linux checks. Linux incremental products
match a fresh Linux build and the fresh native control. The report preserves
the native incremental archived-source discrepancy. Both hosts also pass all
seven final tests on iOS 26 with unchanged executable hashes. Evidence is under `validation/localized-build`,
`localized-shortcuts-final`, `keychain-tests2`, `linking-matrix`,
`linking-runtime2`, `fresh-*-preparation.json`, and `ice-cubes-localized-*`.
The physical-device parity checks are described in the physical-device validation section.
Voice-invocation and interactive Icon Composer material checks remain unverified.

Fresh MMKV products also pass seven storage checks twice per host, including persistence after relaunch.
Use `mmkv-parity-smoke.py --corpus-root` with the fresh source root and `--out` with its build evidence directory.
