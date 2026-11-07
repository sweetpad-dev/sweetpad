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
Simulator checks ran on iOS 27.0. No physical device was used.

Install the Linux tools with `linux-setup.sh` and build the CLI on both hosts.
The projects need their own dependencies before the suite can build them.
The suite checks source revisions and applies the listed fixture patches.
It does not install every framework dependency.

## Results

| Project | Xcode build | Linux build | Simulator comparison |
| --- | --- | --- | --- |
| Wikipedia | Pass | Pass | Both exit in `MWKDataStore.setupLegacyDataStore` with a nil file-URL path |
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
Wikipedia's startup failure also occurs in the native control, so its UI check fails on both hosts.

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
| AccountTests, logic XCTest | 54 pass, 38 fail | Same 92 tests and exact same 38 failing identifiers |

AccountTests failures report keychain status `-34018` in both builds.
They remain failures; the suite does not remove the assertions.
Selecting a nonexistent test now fails explicitly, even when Xcode exits zero.

`fixture-linux-parity` adds a local package, package resources, app resources,
localized plurals, named image/color assets, App Intents, and a storyboard.
Its tests verify the compiled origin, values, resource contents, locale lookup,
asset loading, color metadata, intent registration, and initial scene loading.
Linux products carry a compilation marker that the tests check at runtime.

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

The framework source pins do not make dependency setup hermetic.
Keep the generated CocoaPods, npm, and Dart lockfiles with a captured run.

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
The resource follow-up reports 792 passing macOS CLI tests and 785 passing Linux CLI tests.
Linux skips six native-only checks. Clippy passes with warnings denied.
The bundle comparison suite passes five tests. The SDK importer passes three tests.

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
The modern-icon follow-up rebuilds Ice Cubes on both hosts.
Its icon file inventory and bundle metadata now match the native control.
The strict comparison retains 42 findings: 38 localized NLU files, two module-level NLU files, and two Mach-O comparisons.
Xcode emits localized `nlu.appintents` files. Linux currently emits a module-level shortcut language model.
Matching registration metadata therefore does not establish localized Siri parity.
Watch runtime and device checks remain unverified.

The MMKV reproduction now covers both Swift module import and Objective-C++ object emission.
Disabling forced debug-thunk emission resolves both stages with the pinned compiler.
The final clean application builds pass on both hosts. The compiler workaround remains an explicit fixture patch.
