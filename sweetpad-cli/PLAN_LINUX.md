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
- [ ] SwiftPM: dynamic library products as frameworks, not dylibs
- [ ] SwiftPM: per-test-target products *(optional)*
- [ ] SwiftPM: package schemes *(optional)*

## Merging the branch

- [ ] Corpus sweep: build every corpus project on Linux (NetNewsWire,
      Kingfisher, the Tuist fixtures, Ice Cubes and the rest) and file each
      failure under the items below
- [ ] Document the engine, the setup script and the stand-ins in
      `CLI_DESIGN.md` and the user docs (run the humanizer)
- [ ] Decide whether hayro and resvg (7.5 MB, 66 crates) stay in the macOS
      binary, which never uses them
- [ ] Open the PR to `main` (show the message first)

## Toolchain tools (missing, but cheap)

Swift Build names these in its xcspecs. The Swift toolchain has `llvm-ar`,
`llvm-objcopy`, `docc` and `plutil`; xtool's bundle has `dsymutil` in
`toolset/bin`, off PATH.

- [ ] `copypng`: runs for every loose PNG in an iOS target
      (`COMPRESS_PNG_FILES` defaults to YES). Stand-in: copy, dropping text
      chunks for `-strip-PNG-text`
- [ ] `libtool -static`: every static library target. Stand-in over
      `llvm-ar --format=darwin`, handling `-filelist` and `-o`
- [ ] `dsymutil`: Release (`dwarf-with-dsym`). Put xtool's on PATH
- [ ] `lipo`: multi-arch builds and test bundles. Stand-in for `-create`,
      `-info`, `-thin`, `-extract` and `-remove` (fat headers are simple)
- [ ] `strip`: install and archive builds. `llvm-strip` (a symlink to
      `llvm-objcopy`) with a wrapper for Apple's flags
- [ ] `iconutil`: `.iconset` to `.icns` for macOS icons *(optional)*
- [ ] `tiffutil`: multi-resolution TIFFs, macOS *(optional)*
- [ ] `lex`/`yacc`, `unifdef`: apt's flex, bison and unifdef in the setup
      script *(optional)*
- [ ] `clang-stat-cache`, `tapi`: turn off with build settings *(optional)*

## Apple compilers for data files

- [ ] `ibtool`: XIBs and storyboards to `.nib`/`.storyboardc` (UIKit and
      AppKit keyed archives). Start with `LaunchScreen.storyboard` (Xcode's
      UIKit template has one) and simple views, measured against the corpus.
      The biggest item
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
- [ ] JPEG and HEIC images
- [ ] iOS app icon inside `Assets.car` (App Store builds need it)
- [ ] macOS app icons (`.icns`)
- [ ] Alternate app icons *(optional)*
- [ ] Icon Composer `.icon` files *(optional)*
- [ ] Data sets (`NSDataAsset`)
- [ ] Fallback values for system colors (black today; UIKit resolves the
      name anyway) *(optional)*
- [ ] Sticker packs, AR resources, texture sets, sprite atlases, watch
      complications, launch images *(optional)*

## Build kinds to try

- [x] iOS Simulator, Debug: Xcode's app template, the Linux fixture, a
      SwiftUI app with Alamofire (project reference and package), the local
      checker apps; Ice Cubes compiles and fails only where Xcode 27 does too
- [ ] Release configuration
- [ ] iOS device builds
- [ ] App extensions, widgets and App Clips
- [ ] Test targets (build the bundles)
- [ ] macOS apps, with a runtime check of the macOS `Assets.car`
- [ ] watchOS, tvOS and visionOS apps *(optional)*
- [ ] Mac Catalyst *(optional)*

## Past building

- [ ] Info.plist keys: real `DTXcode`, `DTXcodeBuild`, `DTPlatformBuild` and
      `BuildMachineOSBuild` from the user's Xcode.app (App Store validation
      reads them)
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
  `_car-app` (47 asset lookups), `_strings-app`, `_alamofire-spm`,
  `_alamofire-consumer`. Each prints its results to stderr for a diff
  against Xcode's build.
- `sweetpad-lib/corpus/_car-tools/` (gitignored) keeps the scripts used to
  decode `Assets.car`: `cardump.py` dumps one, `surgery.py` edits and
  rebuilds one, `listing.py` and `corpus_compare2.py` compare `assetutil`
  listings with Xcode's, `glyphs.py` compares symbol records. Their paths
  point at a deleted scratch folder; adjust them before use.
- Golden fixtures to regenerate after an Xcode update:
  `sweetpad-cli/fixtures/actool` and `fixtures/xcstringstool` (each has a
  `regen.py`), and the symbol templates' expected values in
  `src/cli/actool/glyph.rs`.
