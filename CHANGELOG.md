# Change Log

All notable changes to the "sweetpad" extension will be documented in this file.

Check [Keep a Changelog](http://keepachangelog.com/) for recommendations on how to structure this file.

## [0.1.36] - 2024-09-22

- Reduce the number of problem matchers

## [0.1.35] - 2024-09-22

- Update xcworkspace parsing to address issue [#30](https://github.com/sweetpad-dev/sweetpad/issues/30)
- Add problem matchers for build output. Thanks @dankinsoid for the contribution.

## [0.1.34] - 2024-09-21

- Remove macOS from experimental features
- Add support for "--console" option for ios physical devices

## [0.1.33] - 2024-09-18

- Fix run on iOS device

## [0.1.32] - 2024-09-15

- Add watchOS simalators support

## [0.1.31] - 2024-09-15

- Fix imports

## [0.1.30] - 2024-09-14

- Add sentry integration
- Try to fix issue with .startsWith

## [0.1.29] - 2024-09-11

- Add fallback for xcode parser

## [0.1.28] - 2024-09-08

- Add experimental support for macOS apps
- Add "sweetpad.build.args" settings to pass additional arguments to xcodebuild
- New home for documentation: [sweetpad.hyzyla.dev](https://sweetpad.hyzyla.dev) 🎉

## [0.1.27] - 2024-08-03

- Add "codelldbAttributes" to the "launch.json" configuration to allow to customize the debugger attributes.

## [0.1.26] - 2024-07-28

- Imrpove debugger setup
  - new debug type "sweetpad-lldb"
  - add "preLaunchTask" to debug configuration
  - add "CodeLLDB" extension to the list of dependencies
  - add zero-setup `F5` configuration for quick starting with debugging

## [0.1.25] - 2024-07-22

- Save default scheme in workspace state
- Add status bar item to select default scheme

## [0.1.24] - 2024-07-21

- Add the "sweetpad.build.derivedDataPath" setting to set the path of the DerivedData folder.
- Add the "destination" parameter to the ".vscode/tasks.json" file to specify the raw destination for xcodebuild.
- Add a "destination" status bar item at the bottom of the VSCode window to show the current destination. Thanks to
  [@aelam](https://github.com/aelam) for the contribution.
- Add a new destination sidebar panel to show the list of devices and simulators in one place.
- Update icons across the extension. Acknowledgements to [tabler-icons](https://github.com/tabler/tabler-icons).

## [0.1.23] - 2024-07-06

- Add "sweetpad.build.derivedDataPath" setting to set the path of the DerivedData folder

## [0.1.22] - 2024-07-04

- Integration with [Tuist](https://tuist.io). Thanks to [N-Joy-Shadow](https://github.com/N-Joy-Shadow) 💜

## [0.1.21] - 2024-06-29

- Add basic support for running tests on simulators and devices

## [0.1.20] - 2024-06-23

- Add basic support for running app on physical devices (iOS 15+)

## [0.1.17 ... 0.1.19] - 2024-06-05

- Add iOS version to simulators list

## [0.1.16] - 2024-06-01

- Add "debug" level to show debug messages in the output panel
- Add "sweetpad.build.xcodeWorkspacePath" setting to set the path of the current Xcode workspace

## [0.1.15] - 2024-05-25

- Add workspace with multiple projects support

## [0.1.14] - 2024-05-15

- Add settings options for alternative formatter, like [swiftformat](https://github.com/nicklockwood/SwiftFormat).
  Thanks to [Rafael Pedretti](https://github.com/rafaelpedretti-toast) for the adding this feature.
  [#8](https://github.com/sweetpad-dev/sweetpad/pull/8)

## [0.1.13] - 2024-05-12

- README.md updates

## [0.1.12] - 2024-05-11

- Add ability to attach debugger to running app. See
  [documentation](https://github.com/sweetpad-dev/sweetpad/blob/main/docs/wiki/debug.md) for more details.

## [0.1.11] - 2024-04-12

- Automatically regenerate Xcode project on new .swift files using XcodeGen

## [0.1.10] - 2024-04-14

- Add new command executor
- Add basic XcodeGen integration

## [0.1.9] - 2024-03-21

- Remove ./docs folder from the extension package

## [0.1.8] - 2024-03-21

- Add empty screen when Xcode project is not found
- Add commands to create issue on GitHub

## [0.1.7] - 2024-03-20

- Fix typo with launch command
- Add ability to build and run from command palette

## [0.1.6] - 2024-03-10

- Update extension categories

## [0.1.5] - 2024-03-10

- Update README.md

## [0.1.4] - 2024-03-10

- Update README.md

## [0.1.3] - 2024-03-10

- Add task provider for building and running app on simulator
- Allow to configure task in tasks.json

## [0.1.2] — 2024-03-06

- Use xcodeworkspace instead of xcodeproj for generating buildServer.json

## [0.1.1] - 2024-03-03

- move to 0.1.x version

## [0.0.12] - 2024-03-03

- Show logs from app when running on simulator

## [0.0.11] - 2024-03-03

- Bundle extension using esbuild

## [0.0.10] — 2024-03-03

- Improve caching of schemes and configurations
- Add command to reset cache "sweetpad.system.resetSweetpadCache"
- Add command to open project in Xcode
- Improve selection of workspaces and schemes
- Propage errors when xcodebuild used with xcbeautify
- Allow to disable xcbeautify in settings

## [0.0.9] — 2024-02-28

- Detect schemes by reading files in workspace folder instead of using xcodebuild
- Extract configuration name from .xcscheme file

## [0.0.8] - 2024-02-27

- Add command to set active workspace
- Ask user to choose workspace if there are multiple workspaces in the folder

## [0.0.7] - 2024-02-26

- Fix broken build

## [0.0.6] - 2024-02-25

- Add command to generate buildServer.json file for SourceKit-LSP integration
- Add command to execute "xcodebuild clean" command
- Execute "Build" command without "clean" option
- Add command to resolve dependencies using xcodebuild -resolvePackageDependencies
- Improve error handling and logging
- Imrpove shell and task execution

## [0.0.5] - 2024-02-17

Add basic build functionality

## [0.0.4] - 2024-02-03

Fixed problem with panel icon not showing up

## [0.0.3] - 2024-01-28

Public release of SweetPad:

- Integration of swift-format with VSCode for formatting Swift files.
- iOS simulator panel for running and stopping the iOS simulator.
- Panel for installing iOS tools using Homebrew.
