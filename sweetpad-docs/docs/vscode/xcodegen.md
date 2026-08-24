---
sidebar_position: 13
---

# XcodeGen

[XcodeGen](https://github.com/yonaskolb/XcodeGen) generates your `.xcodeproj` from a YAML manifest
(`project.yml`), so the project file never has to be edited, or merged, by hand. SweetPad works with
XcodeGen projects out of the box: generate the project, and the schemes show up in the Build view like
any other.

Install XcodeGen from the [Tools](./tools.md) panel in the SweetPad sidebar, or with Homebrew:

```bash
brew install xcodegen
```

## Generate the project

Run `> SweetPad: Generate an Xcode project using XcodeGen` from the command palette. SweetPad runs
`xcodegen generate` at the workspace root and refreshes the scheme list when it finishes.

## Auto-regenerate when Swift files change

XcodeGen projects list their source files in the generated project by default, so every added or
deleted `.swift` file needs a regeneration. Let SweetPad do that automatically:

```json title=".vscode/settings.json"
{
  "sweetpad.xcodegen.autogenerate": true
}
```

Then restart VSCode to apply the change. SweetPad watches for new or removed Swift files and re-runs
`xcodegen generate` in the background, so the project stays in sync while you work. The watcher only
activates when a `project.yml` exists at the workspace root.

:::tip

XcodeGen 2.44 and newer can make this unnecessary. Give a source `type: syncedFolder`, or set
`defaultSourceDirectoryType: syncedFolder` under `options`, and the generated project points at the
folder instead of listing the files inside it. Xcode then picks up new and deleted files from disk on
its own. Changes to `project.yml` itself still need a regenerate.

:::

:::tip

Prefer [Tuist](./tuist.md)? SweetPad has the same integration for it, including the matching
`sweetpad.tuist.autogenerate` setting.

:::

:::note

Working from a terminal too? The [SweetPad CLI](../cli/generated-projects.md) handles generated
projects differently. It doesn't regenerate for you, but it warns when the spec is newer than the
project and refuses edits that a regenerate would overwrite. It's a separate product with its own
install.

:::
