# Change Log

New features, improvements and bug fixes for SweetPad are documented in this file.

## [0.2.16] - 2026-09-19

- Fix launching apps on the simulator with Xcode 27 ([#337](https://github.com/sweetpad-dev/sweetpad/issues/337), thanks [@richardgroves](https://github.com/richardgroves))

## [0.2.15] - 2026-08-26

- Fix missing schemes from Swift packages a project references ([#327](https://github.com/sweetpad-dev/sweetpad/issues/327), thanks [@rssole](https://github.com/rssole))
- Fix wrong scheme names for a standalone Swift package

## [0.2.14] - 2026-08-26

- Fix apps not launching when the workspace name has a space ([#329](https://github.com/sweetpad-dev/sweetpad/issues/329), thanks [@richardgroves](https://github.com/richardgroves))

## [0.2.13] - 2026-08-26

- Add Xcode's "Any iOS Device" and the other build-only destinations ([#71](https://github.com/sweetpad-dev/sweetpad/issues/71), thanks [@NSExceptional](https://github.com/NSExceptional))
- Autocomplete now works in every target from the start
- "SweetPad: Diagnose BSP" now reports the last background build's error
- Fix Objective-C imports reporting `'Header.h' file not found` on code that builds ([#238](https://github.com/sweetpad-dev/sweetpad/issues/238))
- Fix autocomplete coming up empty on a first BSP setup ([#326](https://github.com/sweetpad-dev/sweetpad/issues/326))

## [0.2.12] - 2026-08-24

- Fix the Marketplace listing's broken screenshots and changelog link

## [0.2.11] - 2026-08-24

- Fix Swift package schemes missing from the Build and Testing panels ([#327](https://github.com/sweetpad-dev/sweetpad/issues/327), thanks [@rssole](https://github.com/rssole))

## [0.2.10] - 2026-08-16

- Add multi-root workspace support ([#314](https://github.com/sweetpad-dev/sweetpad/pull/314), thanks [@dvkellerman](https://github.com/dvkellerman))
- Swift autocomplete now works out of the box ([#317](https://github.com/sweetpad-dev/sweetpad/pull/317))
- Run the build server through the `sweetpad` CLI ([#319](https://github.com/sweetpad-dev/sweetpad/pull/319))
- Add `sweetpad.testing.baseClasses` for tests with a shared base class ([#324](https://github.com/sweetpad-dev/sweetpad/pull/324))
- "SweetPad: Diagnose BSP" now warns when the `sweetpad` CLI is outdated ([#322](https://github.com/sweetpad-dev/sweetpad/pull/322))
- Fix wrapped build errors missing from the Problems panel ([#316](https://github.com/sweetpad-dev/sweetpad/pull/316))
- Fix autocomplete for `.h` headers and symlinked paths ([#312](https://github.com/sweetpad-dev/sweetpad/pull/312))

## [0.2.9] - 2026-07-28

- Add debugging on physical devices running iOS 16 and below ([#309](https://github.com/sweetpad-dev/sweetpad/issues/309), thanks [@zHElEARN](https://github.com/zHElEARN))
- Stream `Logger` output from devices running iOS 16 and below

## [0.2.8] - 2026-07-26

- Add `sweetpad.build.scheme` and `sweetpad.build.destination` settings ([#307](https://github.com/sweetpad-dev/sweetpad/pull/307), thanks [@czuria1](https://github.com/czuria1))
- Fix the built app not being found in custom Derived Data ([#306](https://github.com/sweetpad-dev/sweetpad/issues/306))

## [0.2.7] - 2026-07-05

- Fix device launches failing on arguments that start with `-` ([#296](https://github.com/sweetpad-dev/sweetpad/issues/296), thanks [@rodrigosoldi](https://github.com/rodrigosoldi))

## [0.2.6] - 2026-06-22

- The `sweetpad` CLI now ships through Homebrew, not the extension
- Remove the "Install CLI on PATH" command

## [0.2.5] - 2026-06-22

- Sign and notarize the native resolver and the `sweetpad` CLI

## [0.2.4] - 2026-06-19

- Add Swift Package Manager support ([#290](https://github.com/sweetpad-dev/sweetpad/pull/290))
- Fix the built app not being found under a custom `SYMROOT` ([#292](https://github.com/sweetpad-dev/sweetpad/issues/292), thanks [@foltri](https://github.com/foltri))
- Fix the built app not being found beside an unrelated `.xcworkspace`
- Fix the built app not being found under a non-ASCII path ([#288](https://github.com/sweetpad-dev/sweetpad/pull/288))

## [0.2.3] - 2026-06-15

- Fix the built app not being found through the `project.xcworkspace` stub ([#285](https://github.com/sweetpad-dev/sweetpad/issues/285), thanks [@zHElEARN](https://github.com/zHElEARN))

## [0.2.2] - 2026-06-13

- Fix the built app not being found in some workspaces since 0.2.1 ([#265](https://github.com/sweetpad-dev/sweetpad/issues/265))
- Fix native macOS targets misdetected as Mac Catalyst ([#264](https://github.com/sweetpad-dev/sweetpad/pull/264), thanks [@zHElEARN](https://github.com/zHElEARN))

## [0.2.1] - 2026-06-11

- Match `xcodebuild -list` exactly when listing schemes
- List autocreated schemes for projects with no `.xcscheme` files
- Discover per-user schemes stored under `xcuserdata`
- Resolve schemes stored in the workspace bundle itself
- Resolve build settings more accurately across Xcode 15, 16 and 26
- Feed sourcekit-lsp more faithful linker arguments
- Honour a `DEVELOPER_DIR` exported from your shell profile
- "SweetPad: Refresh shell environment" now re-detects the active Xcode
- Route `-showBuildSettings` through `sweetpad.build.xcodebuildCommand` again
- Add opt-in `sweetpad.system.xcodebuildFallback` for projects the resolver cannot read

## [0.2.0] - 2026-06-10

- Add built-in Swift code intelligence through a bundled BSP server
- Route read-only Xcode operations through the bundled Rust resolver
- Remove the opt-in `sweetpad.system.useSweetpadLib` flag
- Rename the `sweetpad.server.*` settings and commands to `sweetpad.cliServer.*`
- Remove the `scheme.write` agent RPC method
- Fix a crash during workspace auto-detection on older Node runtimes ([#255](https://github.com/sweetpad-dev/sweetpad/issues/255))

## [0.1.94] - 2026-05-24

- Add opt-in `sweetpad.system.useSweetpadLib` to read projects without `xcodebuild`

## [0.1.93] - 2026-05-23

- Add opt-in hot reload for simulator and macOS apps with InjectionNext

## [0.1.92] - 2026-05-21

- Fix the extension failing to load on older VS Code versions ([#252](https://github.com/sweetpad-dev/sweetpad/issues/252))

## [0.1.91] - 2026-05-19

- Add an opt-in agent CLI and JSON-RPC server

## [0.1.90] - 2026-05-17

- Apply the scheme's launch arguments, environment, language and region ([#246](https://github.com/sweetpad-dev/sweetpad/issues/246))
- Don't load the SweetPad UI in non-Swift workspaces ([#247](https://github.com/sweetpad-dev/sweetpad/issues/247))
- Keep `.xcscheme` files intact when SweetPad edits them

## [0.1.89] - 2026-05-12

- Re-release of 0.1.88 with a working deploy

## [0.1.88] - 2026-05-12

- Re-release of 0.1.87 with a working deploy

## [0.1.87] - 2026-05-11

- Re-release of 0.1.85 and 0.1.86, which failed to deploy

## [0.1.86] - 2026-05-11

- Fix builds hanging on scheme pre-action scripts ([#240](https://github.com/sweetpad-dev/sweetpad/issues/240))

## [0.1.85] - 2026-05-11

- Add a search button to the Build and Destinations views ([microsoft/vscode#173742](https://github.com/microsoft/vscode/issues/173742))

## [0.1.84] - 2026-05-10

- Filter Build view schemes with include and exclude patterns ([#236](https://github.com/sweetpad-dev/sweetpad/pull/236))

## [0.1.83] - 2026-04-29

- Find tools on the PATH from your login shell ([#241](https://github.com/sweetpad-dev/sweetpad/issues/241))

## [0.1.82] - 2026-04-27

- Run tasks in a real terminal by default (`sweetpad.system.taskExecutor`)
- Give tasks your login shell's PATH and toolchains
- Add "SweetPad: Refresh shell environment"
- Add opt-in `pymobiledevice3` tunnel auto-start for iOS 17+ devices
- Add "SweetPad: Install pymobiledevice3"
- Show app logs in the task terminal ([#233](https://github.com/sweetpad-dev/sweetpad/pull/233))
- Filter app logs by process, not subsystem ([#235](https://github.com/sweetpad-dev/sweetpad/pull/235))
- Replace `deviceLogStreamBackend` with `sweetpad.build.logStreamEnabled`
- Filter device logs by the `ENABLE_DEBUG_DYLIB` build setting ([#232](https://github.com/sweetpad-dev/sweetpad/pull/232))
- List connected devices above stale paired ones ([#234](https://github.com/sweetpad-dev/sweetpad/issues/234))

## [0.1.81] - 2026-04-18

- Cut noise from device logs and keep custom `Logger` subsystems ([#231](https://github.com/sweetpad-dev/sweetpad/pull/231))

## [0.1.80] - 2026-04-15

- Add opt-in `os_log` streaming from physical iOS devices ([#230](https://github.com/sweetpad-dev/sweetpad/pull/230))

## [0.1.79] - 2026-04-13

- Add opt-outs for `buildServer.json` regeneration and LSP restarts ([#228](https://github.com/sweetpad-dev/sweetpad/pull/228))
- Emit watch markers when debugging on macOS and simulators ([#225](https://github.com/sweetpad-dev/sweetpad/pull/225))
- Fix `sweetpad.testing.configuration` being ignored ([#227](https://github.com/sweetpad-dev/sweetpad/pull/227))
- Fix the status bar sticking on "Extracting Xcode version" ([#226](https://github.com/sweetpad-dev/sweetpad/pull/226))

## [0.1.78] - 2026-04-12

- Fix detection of devices on iOS 16 and earlier ([#224](https://github.com/sweetpad-dev/sweetpad/pull/224))

## [0.1.77] - 2026-03-15

- Add a "Switch Git Worktree" command ([#218](https://github.com/sweetpad-dev/sweetpad/pull/218))

## [0.1.76] - 2026-02-24

- Add a release page

## [0.1.75] - 2026-02-22

- Handle legacy devices and incomplete `devicectl` data ([#209](https://github.com/sweetpad-dev/sweetpad/pull/209))
- Fix visionOS devices detected as the wrong type ([#214](https://github.com/sweetpad-dev/sweetpad/pull/214))

## [0.1.74] - 2026-02-08

- Add Swift Package Manager support ([#213](https://github.com/sweetpad-dev/sweetpad/pull/213), first proposed by [@maatheusgois-dd](https://github.com/maatheusgois-dd) in [#152](https://github.com/sweetpad-dev/sweetpad/pull/152))
- Show app logs from `os_log`, `print` and `NSLog` ([#212](https://github.com/sweetpad-dev/sweetpad/pull/212), thanks [@squad-person](https://github.com/squad-person))
- Allow overriding the `xcodebuild` command ([#210](https://github.com/sweetpad-dev/sweetpad/pull/210), thanks [@asevko](https://github.com/asevko))

## [0.1.73] - 2026-02-01

- Add a command to stop a running scheme action ([#198](https://github.com/sweetpad-dev/sweetpad/issues/198))
- Improve parsing of JSON output with a top-level array ([#206](https://github.com/sweetpad-dev/sweetpad/issues/206))

## [0.1.72] - 2025-12-26

- Add `sweetpad.build.bringSimulatorToForeground` ([#202](https://github.com/sweetpad-dev/sweetpad/pull/202))

## [0.1.71] - 2025-10-19

- Ignore warnings when parsing JSON output ([#195](https://github.com/sweetpad-dev/sweetpad/issues/195))

## [0.1.70] - 2025-09-07

- Reduce font size ([#188](https://github.com/sweetpad-dev/sweetpad/pull/188))

## [0.1.69] - 2025-09-07

- Add a Tuist Test command ([#187](https://github.com/sweetpad-dev/sweetpad/pull/187))

## [0.1.68] - 2025-07-13

- Add a configurable `xcode-build-server` path ([#176](https://github.com/sweetpad-dev/sweetpad/pull/176))

## [0.1.67] - 2025-07-12

- Refresh schemes automatically, with loading states ([#151](https://github.com/sweetpad-dev/sweetpad/pull/151))
- Run on Rosetta simulators ([#156](https://github.com/sweetpad-dev/sweetpad/pull/156))
- Expand environment variables in settings ([#163](https://github.com/sweetpad-dev/sweetpad/pull/163))
- Regenerate `buildServer.json` during build and run ([#164](https://github.com/sweetpad-dev/sweetpad/pull/164))
- Load build configurations from the workspace ([#168](https://github.com/sweetpad-dev/sweetpad/pull/168))
- Open Simulator after booting a device
- Speed up destination listing ([#162](https://github.com/sweetpad-dev/sweetpad/pull/162))
- Speed up workspace parsing ([#161](https://github.com/sweetpad-dev/sweetpad/pull/161))
- Fix a typo in the debugging-launch task name ([#158](https://github.com/sweetpad-dev/sweetpad/pull/158))
- Fix builds running on after their terminal is killed ([#160](https://github.com/sweetpad-dev/sweetpad/pull/160))

## [0.1.66] - 2025-05-17

- Add range formatting ([#149](https://github.com/sweetpad-dev/sweetpad/pull/149))
- Show a status bar item while commands run ([#147](https://github.com/sweetpad-dev/sweetpad/pull/147))
- Stop bringing Simulator to the front after builds ([#150](https://github.com/sweetpad-dev/sweetpad/pull/150))
- Make the simulator debugger more stable ([#138](https://github.com/sweetpad-dev/sweetpad/pull/138))

## [0.1.65] - 2025-04-27

- Use swift-format from the Xcode toolchain by default ([#136](https://github.com/sweetpad-dev/sweetpad/issues/136))

## [0.1.63-0.1.64] - 2025-04-27

- Resolve the debug configuration more reliably

## [0.1.61-0.1.62] - 2025-04-24

- Technical release

## [0.1.60] - 2025-04-23

- Improve build configuration selection

## [0.1.57] - 2025-03-30

- Improve recent devices management ([#130](https://github.com/sweetpad-dev/sweetpad/issues/130))
- Select destinations inline ([#128](https://github.com/sweetpad-dev/sweetpad/issues/128))

## [0.1.56] - 2025-03-09

- Fix the wrong build product being picked to run ([#118](https://github.com/sweetpad-dev/sweetpad/issues/118))

## [0.1.55] - 2025-03-08

- Add Tuist dynamic configuration (thanks [@ladislas](https://github.com/ladislas))
- Enable Tuist for workspace projects (thanks [@ladislas](https://github.com/ladislas))

## [0.1.54-0.1.56] - 2025-03-02

- Technical release

## [0.1.53] - 2025-02-23

- Add `sweetpad.build.env`
- Fix the `xcodebuild` problem matcher

## [0.1.52] - 2025-02-22

- Reorganize the build context menu
- Build packages without build settings

## [0.1.51] - 2025-02-05

- Fix Apple TV devices shown as visionOS ([#106](https://github.com/sweetpad-dev/sweetpad/issues/106), thanks [@MACwayne](https://github.com/MACwayne))

## [0.1.50] - 2025-01-19

- Show all destinations, not only supported ones

## [0.1.49] - 2024-12-28

- Add debugging on physical devices

## [0.1.48] - 2024-12-22

- Add launch arguments and environment variables to the configuration

## [0.1.47] - 2024-12-19

- Add a diagnose command for troubleshooting
- Improve logging

## [0.1.46] - 2024-12-07

- Stop the previous build task before starting a new one

## [0.1.45] - 2024-12-01

- Regenerate `buildServer.json` when the scheme changes
- Add visionOS device support
- Add tvOS device support
- Update the destination panel icons

## [0.1.44] - 2024-12-01

- Remember the app path for iOS devices (thanks [@kvaster](https://github.com/kvaster))

## [0.1.43] - 2024-12-01

- Disable `setvbuf` by default

## [0.1.42] - 2024-11-17

- Choose the build configuration
- Allow turning off `-allowProvisioningUpdates`
- Handle duplicated `sweetpad.build.args` parameters

## [0.1.41] - 2024-11-03

- Add tvOS simulator support
- Improve testing for SPM packages

## [0.1.40] - 2024-10-23

- Add separate pickers for testing and launching

## [0.1.39] - 2024-10-23

- Technical release

## [0.1.38] - 2024-10-23

- Add visionOS simulator support

## [0.1.37] - 2024-10-05

- Fix build configurations without a name

## [0.1.36] - 2024-09-22

- Reduce the number of problem matchers

## [0.1.35] - 2024-09-22

- Add problem matchers for build output (thanks [@dankinsoid](https://github.com/dankinsoid))
- Fix workspace parsing ([#30](https://github.com/sweetpad-dev/sweetpad/issues/30))

## [0.1.34] - 2024-09-21

- Take macOS support out of experimental
- Add `--console` for iOS devices

## [0.1.33] - 2024-09-18

- Fix running on iOS devices

## [0.1.32] - 2024-09-15

- Add watchOS simulator support

## [0.1.31] - 2024-09-15

- Fix imports

## [0.1.30] - 2024-09-14

- Add Sentry error reporting
- Try to fix a `.startsWith` error

## [0.1.29] - 2024-09-11

- Add a fallback for the Xcode project parser

## [0.1.28] - 2024-09-08

- Add experimental macOS app support
- Add `sweetpad.build.args` for extra `xcodebuild` arguments
- Move the docs to [sweetpad.hyzyla.dev](https://sweetpad.hyzyla.dev)

## [0.1.27] - 2024-08-03

- Add `codelldbAttributes` to customize the debugger in `launch.json`

## [0.1.26] - 2024-07-28

- Add the `sweetpad-lldb` debug type
- Start debugging with `F5` and no setup
- Add `preLaunchTask` to the debug configuration
- Depend on the CodeLLDB extension

## [0.1.25] - 2024-07-22

- Add a status bar item to pick the scheme
- Remember the default scheme per workspace

## [0.1.24] - 2024-07-21

- Add a destinations panel for devices and simulators
- Add a destination status bar item (thanks [@aelam](https://github.com/aelam))
- Add `destination` to `tasks.json` for a raw `xcodebuild` destination
- Add `sweetpad.build.derivedDataPath`
- Update icons across the extension (from [tabler-icons](https://github.com/tabler/tabler-icons))

## [0.1.23] - 2024-07-06

- Add `sweetpad.build.derivedDataPath`

## [0.1.22] - 2024-07-04

- Add [Tuist](https://tuist.io) integration (thanks [@N-Joy-Shadow](https://github.com/N-Joy-Shadow))

## [0.1.21] - 2024-06-29

- Run tests on simulators and devices

## [0.1.20] - 2024-06-23

- Run apps on physical devices (iOS 15+)

## [0.1.17-0.1.19] - 2024-06-05

- Show the iOS version in the simulator list

## [0.1.16] - 2024-06-01

- Add a `debug` log level
- Add `sweetpad.build.xcodeWorkspacePath`

## [0.1.15] - 2024-05-25

- Support workspaces with multiple projects

## [0.1.14] - 2024-05-15

- Support other formatters, like [swiftformat](https://github.com/nicklockwood/SwiftFormat) ([#8](https://github.com/sweetpad-dev/sweetpad/pull/8), thanks [@rafaelpedretti-toast](https://github.com/rafaelpedretti-toast))

## [0.1.13] - 2024-05-12

- Update the README

## [0.1.12] - 2024-05-11

- Attach the debugger to a running app ([docs](https://github.com/sweetpad-dev/sweetpad/blob/main/docs/wiki/debug.md))

## [0.1.11] - 2024-04-12

- Regenerate the XcodeGen project when `.swift` files are added

## [0.1.10] - 2024-04-14

- Add basic XcodeGen integration
- Add a new command executor

## [0.1.9] - 2024-03-21

- Leave the docs out of the extension package

## [0.1.8] - 2024-03-21

- Show an empty state when no Xcode project is found
- Add commands to open a GitHub issue

## [0.1.7] - 2024-03-20

- Build and run from the Command Palette
- Fix a typo in the launch command

## [0.1.6] - 2024-03-10

- Update the extension categories

## [0.1.5] - 2024-03-10

- Update the README

## [0.1.4] - 2024-03-10

- Update the README

## [0.1.3] - 2024-03-10

- Add a task provider to build and run on the simulator
- Configure tasks in `tasks.json`

## [0.1.2] - 2024-03-06

- Generate `buildServer.json` from the workspace, not the project

## [0.1.1] - 2024-03-03

- Move to 0.1.x

## [0.0.12] - 2024-03-03

- Show app logs when running on the simulator

## [0.0.11] - 2024-03-03

- Bundle the extension with esbuild

## [0.0.10] - 2024-03-03

- Add a command to reset the cache (`sweetpad.system.resetSweetPadCache`)
- Add a command to open the project in Xcode
- Allow turning off xcbeautify
- Cache schemes and configurations better
- Improve workspace and scheme selection
- Show `xcodebuild` errors when xcbeautify is on

## [0.0.9] - 2024-02-28

- Detect schemes from workspace files instead of `xcodebuild`
- Read the configuration name from the `.xcscheme` file

## [0.0.8] - 2024-02-27

- Add a command to set the active workspace
- Ask which workspace to use when there are several

## [0.0.7] - 2024-02-26

- Fix a broken build

## [0.0.6] - 2024-02-25

- Add a command to generate `buildServer.json` for SourceKit-LSP
- Add a clean command
- Add a command to resolve package dependencies
- Build without cleaning first
- Improve error handling and logging
- Improve shell and task execution

## [0.0.5] - 2024-02-17

- Add basic build support

## [0.0.4] - 2024-02-03

- Fix the panel icon not showing

## [0.0.3] - 2024-01-28

- First public release
- Format Swift files with swift-format
- Run and stop the iOS simulator from a panel
- Install iOS tools with Homebrew from a panel
