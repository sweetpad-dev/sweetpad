---
sidebar_position: 12
---

# Tuist

[Tuist](https://tuist.io) lets you define your Xcode project declaratively instead of editing it through Xcode's UI.
SweetPad surfaces the most common Tuist commands directly in the VSCode command palette.

## Commands

- **SweetPad: Generate an Xcode project using Tuist** — runs `tuist generate` from the workspace root.
- **SweetPad: Install Swift Package using Tuist** — runs `tuist install`.
- **SweetPad: Clean Tuist project** — removes generated files.
- **SweetPad: Edit Tuist project (Open project in Xcode)** — opens the manifest project in Xcode for editing.
- **SweetPad: Test Generated project using Tuist** — runs `tuist test`, building and testing every target Tuist
  knows about. Useful as a one-shot "did I break anything" check without picking a scheme.

## Auto-regenerate on `.swift` file changes

If you frequently add or remove `.swift` files, let SweetPad re-run `tuist generate` automatically when those files
change so new files show up in the project without a manual regeneration:

```json title=".vscode/settings.json"
{
  "sweetpad.tuist.autogenerate": true
}
```

Then restart VSCode to apply the change.

## Dynamic Tuist configuration

If you use [Tuist's dynamic configuration](https://docs.tuist.dev/en/guides/develop/projects/dynamic-configuration)
to switch app name, bundle ID, or feature flags per environment, pass the variables through
`sweetpad.tuist.generate.env`:

```json title=".vscode/settings.json"
{
  "sweetpad.tuist.generate.env": {
    "TUIST_APP_NAME": "Diia",
    "TUIST_TARGET_COUNTRY": "Ukraine"
  }
}
```

Every call SweetPad makes to `tuist generate` (including the auto-regeneration above) receives these variables, so
the project loaded into VSCode matches the variant Xcode would produce with the same env.
