# Building Xcode projects on Linux

Status on 2026-10-11. All the work is on branch `claude/linux-build-pif`, not
merged into `main` yet.

`sweetpad build` has a Swift Build engine, the default where there's no
Xcode. It translates the workspace into the PIF that Apple's open-source
Swift Build takes as input, then runs `swbuild` against a Darwin Swift SDK
that xtool extracts from the user's own Xcode.app. Xcode tools that Linux
doesn't have are replaced by the sweetpad binary itself, started under the
tool's name. lld links. Nothing calls Xcode or xcodebuild at build time.

Items marked *(optional)* can wait until a project needs them.

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
- [x] SwiftPM packages through `swift package dump-pif`, merged into the PIF
      (`42978410`)
- [x] Cross-import overlays, through a toolset flag (`4e2a25c7`)
- [ ] Apple's macro plugins (see the section below)
- [ ] SwiftPM: dynamic library products as frameworks, not dylibs
- [ ] SwiftPM: per-test-target products *(optional)*
- [ ] SwiftPM: package schemes *(optional)*

## Apple's macros

Xcode expands the SDK's macros through plugins that are macOS dylibs
(`iPhoneOS.platform/Developer/usr/lib/swift/host/plugins`). Each macro in
the SDK's `.swiftinterface` files names its plugin and type in
`#externalMacro(module:type:)`, which gives the full list. In the Xcode 27
iOS SDK:

| Plugin | Macros | On Linux |
|---|---|---|
| ObservationMacros, FoundationMacros, SwiftMacros | `@Observable`, `#Predicate`, `#Expression`, `#bundle`, `@TaskLocal`, `@DebugDescription`, `_SwiftifyImport` | Swift toolchain |
| FoundationModelsMacros | `@Generable`, `@Guide` | xtool |
| PreviewsMacros | `#Preview` | xtool for SwiftUI views; not UIKit views or view controllers (`KitViewMacro`, `PreviewCommonGroup`) |
| SwiftUIMacros | `@Entry`, `@Animatable`, `@State` (a macro in the 27 SDK) | xtool, except the helper types `EntryTypeCheckMacro`, `EntryTypeCheckClassMacro`, `SendableEntryDefaultMacro`, `UnsafeEntryDefaultMacro`, `StateTypeMacro` |
| SwiftDataMacros | `@Model`, `@Query`, `@ModelActor`, `@Attribute`, `@Relationship`, `@Transient`, `#Unique`, `#Index` | missing |
| AppIntentsMacros | the schema macros (`@AppIntent(schema:)` and the rest), `@ComputedProperty`, `@DeferredProperty`, `@UnionValue` | missing |
| TipKitMacros, StateReportingMacros | `#Rule`, `@Parameter`; `@ReportableMetadata` | missing |

xtool's server is `darwin.artifactbundle/OpenAppleMacrosServer`; the `.so`
files beside the plugins are empty placeholders. In the corpus, 5 files use
`@Model` and 13 use `@Query` (Ice Cubes probably hits this once RevenueCat
builds; not confirmed yet); none use the App Intents schema macros, TipKit
or StateReporting. Plain App Intents (the `@Parameter` property wrapper)
need no macros.

The oracle is Xcode's compiler printing each expansion
(`-Xfrontend -dump-macro-expansions`): reimplement the macro with
swift-syntax and diff the expanded Swift text.

- [ ] Confirm with a real swbuild build that `@Model` fails, and how
- [ ] Decide where our plugins live (beside xtool's server, or our own) and
      how swbuild points the compiler at them
- [ ] SwiftDataMacros: `@Model` (schema metadata, backing data, accessors
      per property) and `@Query` first, then `@ModelActor` and the attribute
      macros (1–2 weeks)
- [ ] Check whether xtool's gaps are reachable: `#Preview` of a UIKit view
      or view controller, the `@Entry` and `@State` helper types; fix
      upstream in xtool if so
- [ ] AppIntentsMacros, the schema macros *(optional)*
- [ ] TipKitMacros, StateReportingMacros *(optional)*

## Merging the branch

- [ ] Corpus sweep: build every corpus project on Linux (NetNewsWire,
      Kingfisher, the Tuist fixtures, Ice Cubes and the rest) and file each
      failure under the items below
- [ ] Document the engine, the setup script and the stand-ins in
      `CLI_DESIGN.md` and the user docs (run the humanizer)
- [ ] Decide whether hayro and resvg (7.5 MB, 66 crates) stay in the macOS
      binary, which never uses them
- [ ] Open the PR to `main` (show the message first)

## Toolchain tools

Swift Build names these in its xcspecs. The Swift toolchain has `llvm-ar`,
`llvm-objcopy`, `docc` and `plutil`; xtool's bundle has `dsymutil` and
`llvm-libtool-darwin` (as `llvm-lib`, the toolset's librarian) in
`toolset/bin`, which the engine puts on `PATH`.

- [x] `libtool -static`: xtool's toolset already names its
      `llvm-libtool-darwin` as the librarian
- [x] `dsymutil`: Release (`dwarf-with-dsym`) finds xtool's on `PATH`
- [x] `copypng`: copies, dropping text chunks for `-strip-PNG-text`. Xcode's
      writes Apple's CgBI PNGs, which UIKit doesn't need
- [x] `lipo`: `-create`, `-extract`, `-thin`, `-remove`, `-info`, `-archs`,
      `-verify_arch`; checked with an arm64 + x86_64 simulator build
- [x] `strip`: `llvm-objcopy` run as `llvm-strip`, which keeps the linker's
      ad-hoc signature valid; checked with `DEPLOYMENT_POSTPROCESSING=YES`
- [x] `ditto` for run scripts (XcodeGen gives a static library with Swift
      sources one), copies only
- [ ] `iconutil`: `.iconset` to `.icns` for macOS icons *(optional)*
- [ ] `tiffutil`: multi-resolution TIFFs, macOS *(optional)*
- [ ] `lex`/`yacc`, `unifdef`: apt's flex, bison and unifdef in the setup
      script *(optional)*
- [ ] `clang-stat-cache`, `tapi`: turn off with build settings *(optional)*

## Apple compilers for data files

- [ ] `ibtool`: XIBs and storyboards to `.nib`/`.storyboardc`. The biggest
      item; see the ibtool section below
- [ ] `momc`: `.xcdatamodeld` to `.momd`, plus the generated Swift classes
      the app compiles against
- [ ] `intentbuilderc`: `.intentdefinition` to generated intent classes and
      strings
- [ ] `appintentsmetadataprocessor`: `Metadata.appintents` from the
      compiler's const values. A no-op first (the app runs, intents don't
      register), real output later
- [ ] `coremlc`: generate the Swift class from the `.mlmodel`; the compiled
      `.mlmodelc` format is closed, so fail clearly or compile at runtime
      *(optional)*
- [ ] `mapc`: Core Data mapping models *(optional)*
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
are NIBArchive files; macOS nibs are NSKeyedArchiver plists of AppKit
classes, a separate project.

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

Coverage, measured on the corpus and on a few thousand public XIBs and
storyboards (the oracle compiles them in a batch on a Mac):
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

- [ ] NIBArchive writer, decoder and structural diff harness (2–3 days)
- [ ] `LaunchScreen.storyboard`: UIView, UILabel, UIImageView, constraints,
      safe area; unblocks Xcode's UIKit template (2–3 days)
- [ ] All 32 iOS corpus files matching structurally and passing runtime
      checks, seeded from `xib2nib`'s mappings (2–4 weeks)
- [ ] Public XIB set and the coverage report; stop around 95% of weighted
      input pairs *(optional)*
- [ ] AppKit nibs, NSKeyedArchiver (NetNewsWire's 26 files) *(optional)*
- [ ] watchKit and tvOS IB files *(optional)*

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
- [x] macOS app icons: `CFBundleIconName` for the icon in the car, and an
      `.icns` with the four sizes Xcode's actool stores beside it
- [ ] Icon Composer `.icon` files *(optional)*
- [x] Data sets (`NSDataAsset`), each file as it is with its type: the one
      its entry names, else its extension's, or the `dyn.` type macOS makes
      up for an extension it doesn't know. `_car-app`'s four data lookups
      match Xcode's build
- [ ] Fallback values for system colors (black today; UIKit resolves the
      name anyway) *(optional)*
- [ ] Sticker packs, AR resources, texture sets, sprite atlases, watch
      complications, launch images *(optional)*

## Build kinds to try

- [x] iOS Simulator, Debug: Xcode's app template, the Linux fixture, a
      SwiftUI app with Alamofire (project reference and package), the local
      checker apps; Ice Cubes compiles and fails only where Xcode 27 does too
- [x] Release configuration: dSYM, strip on install, a static library, a
      loose PNG; `corpus/_tools-app` (local) runs the same in the simulator
- [ ] iOS device builds
- [x] App extensions and widgets: an `embeddedBinaryValidationUtility`
      stand-in makes Xcode's checks (bundle ID prefix, matching versions);
      a widget built here registers in the simulator
- [ ] App Clips
- [x] Test targets: `test build` adds the default test plan's targets, or
      the scheme's testables, as `build-for-testing` does, and leaves the
      testing frameworks out of the host app like Xcode 27. The bundle passes
      under `xcodebuild test-without-building` with Xcode's `.xctestrun`
- [ ] An `.xctestrun` written by the build, so tests built here run on a Mac
      without a second build *(optional)*
- [x] macOS apps: `corpus/_mac-app` (local) matches Xcode's build in all 31
      asset lookups, the app icon among them. Swift
      Build needs `swift-build-linux.patch` (`linux-setup.sh` applies it)
      to skip registering the app with Launch Services; upstream it
- [ ] watchOS, tvOS and visionOS apps *(optional)*
- [ ] Mac Catalyst *(optional)*

## Past building

- [x] Info.plist keys: `DTXcode`, `DTXcodeBuild` and `DTPlatformBuild` from
      the `version.plist` files `linux-setup.sh` copies into the SDK bundle
      (xtool leaves them out); the same as Xcode's build
- [ ] `BuildMachineOSBuild`: left out, since no macOS built the app. Find
      out whether App Store validation needs it
- [ ] Signing with rcodesign (the apple-codesign crate) or xtool's signer:
      certificates, profiles, entitlements
- [ ] Device install through pymobiledevice3 or xtool's installer
- [ ] Archive and export an IPA *(optional)*
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
  `_alamofire-consumer`, `_tools-app` (XcodeGen: static libraries, a loose
  PNG, a widget, a unit test bundle), `_mac-app` (XcodeGen: a macOS app's
  asset lookups and icon). Each prints its results to stderr for a diff
  against Xcode's build.
- `sweetpad-lib/corpus/_car-tools/` (gitignored) keeps the scripts used to
  decode `Assets.car`: `cardump.py` dumps one, `surgery.py` edits and
  rebuilds one, `listing.py` and `corpus_compare2.py` compare `assetutil`
  listings with Xcode's, `glyphs.py` compares symbol records, and
  `icons.py <case>` compiles app icon sets of each shape with Xcode's actool,
  or with `OURS=<path to an actool symlink>` the stand-in. Their paths
  point at a deleted scratch folder; adjust them before use.
- Golden fixtures to regenerate after an Xcode update:
  `sweetpad-cli/fixtures/actool` and `fixtures/xcstringstool` (each has a
  `regen.py`), and the symbol templates' expected values in
  `src/cli/actool/glyph.rs`.
