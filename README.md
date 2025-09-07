# SweetPad (iOS/Swift development) <img valign="middle" alt="SweetPad logo" width="40" src="./images/logo.png" />

📚 [Documentation](https://sweetpad.hyzyla.dev/) | 📦
[VSCode Marketplace](https://marketplace.visualstudio.com/items?itemName=sweetpad.sweetpad) | 🐞
[Github Issues](https://github.com/sweetpad-dev/sweetpad/issues) | 🏔️ [Roadmap](https://github.com/sweetpad-dev/sweetpad/blob/main/TODO.md)

<!-- [![Discord](https://img.shields.io/badge/SweetPad-Discord-blue?logo=discord&logoColor=white&link=https%3A%2F%2Fdiscord.gg%2FXZwRtQ5dew)](https://discord.gg/XZwRtQ5dew) -->

<hr/>
You can support this project by giving a star on GitHub ⭐️ or by becoming an official sponsor 💰

[![GitHub](https://img.shields.io/github/stars/sweetpad-dev/sweetpad?style=social)](https://github.com/sweetpad-dev/sweetpad)
[![Github Sponsors](https://img.shields.io/badge/Github%20Sponsors-%E2%9D%A4-red?style=flat&logo=github)](https://github.com/sponsors/sweetpad-dev)
[![Buy Me A Coffee](https://img.shields.io/badge/Buy%20Me%20A%20Coffee%20-%E2%9D%A4-red?style=flat&logo=buy-me-a-coffee&link=https%3A%2F%2Fgithub.com%2Fsponsors%2Fsweetpad-dev)](https://www.buymeacoffee.com/hyzyla)

<!-- [![Twitter](https://img.shields.io/twitter/follow/sweetpad_dev?style=social&logo=twitter)](https://twitter.com/sweetpad_dev) -->
<hr/>

Develop Swift/iOS projects using VSCode or Cursor.

The long-term goal is to make VSCode/Cursor a viable alternative to Xcode for iOS development by integrating open-source
tools such as **swift-format**, **swiftlint**, **xcodebuild**, **xcrun**, **xcode-build-server**, **sourcekit-lsp**.

![iOS simulator](./docs/images/build-demo.gif)

## Feature

- ✅ **[Autocomplete](https://sweetpad.hyzyla.dev/docs/autocomplete)** — setup autocomplete using
  [xcode-build-server](https://github.com/SolaWing/xcode-build-server)
  
- 🛠️ **[Build & Run](https://sweetpad.hyzyla.dev/docs/build)** — build and run application using
  [xcodebuild](https://developer.apple.com/library/archive/technotes/tn2339/_index.html)
  
- 💅🏼 **[Format](https://sweetpad.hyzyla.dev/docs/format)** — format files using
  [swift-format](https://github.com/apple/swift-format) or other formatter of your choice
  
- 📱 **[Simulator](https://sweetpad.hyzyla.dev/docs/simulators)** — manage iOS simulators
  
- 📱 **[Devices](https://sweetpad.hyzyla.dev/docs/devices)** — run iOS applications on iPhone or iPad
 
- 🛠️ **[Tools](https://sweetpad.hyzyla.dev/docs/tools)** — manage essential iOS development tools using
  [Homebrew](https://brew.sh/)
  
- 🪲 **[Debug](https://sweetpad.hyzyla.dev/docs/debug)** — debug iOS applications using
  [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb)
  
- ✅ **[Tests](https://sweetpad.hyzyla.dev/docs/tests)** — run tests on simulators and devices
  

> 💡 If you have any ideas, please open an issue or start a discussion on the
> [SweetPad](https://github.com/sweetpad-dev/sweetpad) GitHub repository.

## Requirements

1. 🍏 MacOS — other platforms are currently not supported
2. 📱 Xcode — required for building and running iOS apps via `xcodebuild`

## Local Development & Debugging

To develop and debug the extension locally:

1. Clone the repository:
   ```shell
   git clone git@github.com:sweetpad-dev/sweetpad.git
   cd sweetpad
   ```

2. Install dependencies:
   ```shell
   npm install
   ```

3. Open in VSCode:
   ```shell
   code .
   ```

4. Press **F5** to build and run the extension in a new window. You'll see two VSCode windows:
   - Source code window (for editing and debugging)
   - **[Extension Development Host]** window (where the extension runs)

5. Set breakpoints in the source code window where needed

6. In the extension window, press **Cmd+R** to restart the extension

7. When you trigger an action that hits a breakpoint, the debugger will pause and you can inspect variables and the call stack

For more detailed debugging instructions, see the [development guide](./docs/dev/debug.md).

## Changelog

The [CHANGELOG.md](./CHANGELOG.md) contains all notable changes to the "sweet pad" extension.

## License

This extension is licensed under the [MIT License](./LICENSE.md).