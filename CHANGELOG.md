# Change Log

All notable changes to the "sweetpad" extension will be documented in this file.

Check [Keep a Changelog](http://keepachangelog.com/) for recommendations on how to structure this file.

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
