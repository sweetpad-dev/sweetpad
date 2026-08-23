# SweetPad <img valign="middle" alt="SweetPad logo" width="38" src="./sweetpad-docs/static/images/logo.png" />

**xcodebuild for humans.** Build, run, debug, and test iOS, macOS, tvOS, watchOS, and visionOS apps
from your terminal, without opening Xcode. No Xcode window, no editor required. Works with Xcode
projects and workspaces, Tuist, XcodeGen, and Swift Packages.

Full documentation lives at **[sweetpad.hyzyla.dev](https://sweetpad.hyzyla.dev/docs/cli/getting-started)**.
If it saves you time, star the repo ⭐️ or become a sponsor 💰

[![GitHub Sponsors](https://img.shields.io/badge/Github%20Sponsors-%E2%9D%A4-red?style=flat&logo=github)](https://github.com/sponsors/sweetpad-dev)
[![Buy Me A Coffee](https://img.shields.io/badge/Buy%20Me%20A%20Coffee-%E2%9D%A4-red?style=flat&logo=buy-me-a-coffee)](https://www.buymeacoffee.com/hyzyla)

## Try it

```bash
brew install sweetpad-dev/tap/sweetpad
```

A Mac with Xcode is the whole dependency list. Then:

```bash
sweetpad project new MyApp   # or just cd into a project you already have
cd MyApp

sweetpad run
```

`run` builds, installs, launches, and streams the logs. Press `r` to rebuild and relaunch without
leaving, `q` to quit.

The first run asks which scheme to build and where to run it, then **remembers**, so you never answer
again. To skip the question outright, say where you want it:

```bash
sweetpad run   --on "iPhone 16 Pro"   # closest matching simulator or device
sweetpad build --on mac               # your Mac
sweetpad test  --on booted            # whichever simulator is already open
```

Full walkthrough: **[Get started with the CLI](https://sweetpad.hyzyla.dev/docs/cli/getting-started)**.

## Why you'd stay

- **[Hot reload](https://sweetpad.hyzyla.dev/docs/cli/hot-reload)**: `sweetpad run --hot` patches
  each Swift file you save into the live process, so the app keeps its screen and its state.
- **[Autocomplete anywhere](https://sweetpad.hyzyla.dev/docs/cli/autocomplete)**: `sweetpad bsp init`
  points SourceKit-LSP at SweetPad's build server, so Neovim, Zed, Helix, and Emacs get completions
  and diagnostics on a real Xcode project.
- **[Debug without the IDE](https://sweetpad.hyzyla.dev/docs/cli/app-lifecycle)**: run under lldb,
  script a session with `--batch --cmd`, or let `app diagnose` catch the first crash and print a
  structured report.
- **[Scripts and CI](https://sweetpad.hyzyla.dev/docs/cli/scripts-and-ci)**: every command speaks
  JSON, exit codes are specific enough to branch on, and `--gh-annotations` puts errors inline on a
  pull request.
- **[Agent skills](https://sweetpad.hyzyla.dev/docs/cli/agent-skills)**: vendor-neutral files that
  teach Claude Code, Cursor, Codex, Copilot, or Gemini to drive the CLI properly:
  `npx skills add sweetpad-dev/sweetpad`

## The rest of the surface

| Command             | What it does                                                                            |
| ------------------- | --------------------------------------------------------------------------------------- |
| `sweetpad test`     | Run tests, with `--only-testing`, `--failed`, `--retry-flaky`, `--coverage`, `--junit`.  |
| `sweetpad format`   | Format Swift sources, or lint them with `--tool swiftlint`.                              |
| `sweetpad devices`  | Everything runnable (Mac, simulators, connected devices), each with a copy-paste specifier. |
| `sweetpad simulator`| Boot, clone, erase, screenshot, record mp4, set location and permissions, deliver a push. |
| `sweetpad project`  | Inspect the project, resolve a build setting, add or update Swift Package dependencies.   |
| `sweetpad archive`  | Archive and export an `.ipa`.                                                             |
| `sweetpad merge`    | Git merge drivers that resolve `project.pbxproj` conflicts semantically, not line by line. |
| `sweetpad doctor`   | Diagnose the local Xcode and Swift toolchain when something is off.                       |

Configuration is optional and layered: answer the prompts once, or commit a `sweetpad.toml` for the
team and keep your own preferences in `~/.config/sweetpad/config.toml`. `sweetpad status` prints which
layer won. And SweetPad never tries to wrap all of `xcodebuild`. Anything after `--` is handed over
untouched, so one unusual flag doesn't send you back to the raw tool:

```bash
sweetpad build -- SWIFT_ACTIVE_COMPILATION_CONDITIONS="DEBUG STAGING"
```

The tool documents itself offline too: `sweetpad --help`, `sweetpad <command> --help`, and
`sweetpad help <topic>` for config, environment, exit-codes, destinations, and hot-reload.

## Prefer to work in VS Code?

The same builds, runs, and tests in the VS Code sidebar, plus breakpoints via CodeLLDB, a native
Testing panel, format-on-save, and autocomplete. It works in [Cursor](https://www.cursor.com/) too,
and has over 61,000 installs on the Marketplace.

[![Install from the Marketplace](https://img.shields.io/badge/VS%20Code-install%20extension-007ACC?logo=visualstudiocode)](https://marketplace.visualstudio.com/items?itemName=sweetpad.sweetpad)

**You do not need both.** The CLI needs no editor, and the extension builds, runs, debugs, and tests
on its own. They meet in exactly one place: the extension's default autocomplete runs the build server
that ships inside the CLI binary, so that one feature asks for the CLI as well.
[Which one do I need?](https://sweetpad.hyzyla.dev/docs)

## License

[MIT](./LICENSE.md)
