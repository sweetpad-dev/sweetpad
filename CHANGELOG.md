# Change Log

All notable changes to the "sweetpad" extension will be documented in this file.

Check [Keep a Changelog](http://keepachangelog.com/) for recommendations on how to structure this file.

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
