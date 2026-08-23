---
sidebar_position: 16
sidebar_label: Editor autocomplete
---

# Editor autocomplete

Completion, jump-to-definition, and hover docs for Swift all come from sourcekit-lsp, which ships with
your Xcode toolchain. It works out of the box for Swift packages, and not at all for Xcode projects,
because it has no way to know how your project compiles a given file. Which SDK? Which deployment
target? Which of your five build configurations?

SweetPad answers that question. The CLI contains a build server that sourcekit-lsp can ask, and one
command wires the two together:

```bash
sweetpad bsp init
```

That works in any editor with sourcekit-lsp support: Neovim, Zed, Helix, Emacs, or anything else
speaking LSP. Xcode isn't involved beyond supplying the toolchain.

:::note

Using VS Code? The extension sets this up for you, with a Tools panel button and an auto-regenerate
option. See the [extension's autocomplete page](../vscode/autocomplete.md) instead, since you don't need to
run anything from a terminal.

:::

## Setting it up

Run `bsp init` once from inside your project:

```console
$ sweetpad bsp init
wrote /path/to/MyApp/buildServer.json
```

It writes a `buildServer.json` next to your project container, which is the file sourcekit-lsp looks
for in a workspace root:

```json
{
  "argv": [
    "/Users/you/.local/bin/sweetpad",
    "bsp",
    "serve",
    "--project",
    "/path/to/MyApp/MyApp.xcodeproj"
  ],
  "bspVersion": "2.2.0",
  "languages": ["swift", "objective-c", "objective-cpp", "c", "cpp"],
  "name": "sweetpad-lib",
  "version": "0.1.1"
}
```

When sourcekit-lsp opens the workspace it finds that file, runs the command in `argv`, and asks it for
each file's compiler arguments. You never run `bsp serve` yourself. It exists for sourcekit-lsp to
exec.

`--output-file` puts the file somewhere else, for a layout where the container isn't where your editor
opens the workspace:

```bash
sweetpad bsp init --output-file ./buildServer.json
```

## Pointing your editor at it

There's no SweetPad-specific editor configuration. What each editor needs is the same two things:

1. **sourcekit-lsp running for Swift files.** Most editors' LSP setups already know about it, since it's
   the standard Swift language server, found in your Xcode toolchain.
2. **The workspace root set to the directory holding `buildServer.json`.** This is the part that
   actually breaks. If you open a subdirectory, or your editor picks the git root while the file sits
   in `ios/`, sourcekit-lsp won't find it. Either open the right directory or use `--output-file` to
   put the file where your editor will look.

Nothing else is SweetPad-aware, which is the point: the build server is a standard BSP server and
every LSP editor treats it the same way.

## Checking the wiring

`bsp doctor` verifies each thing that has to be true, and says which one isn't:

```console
$ sweetpad bsp doctor
buildServer.json: /path/to/MyApp/buildServer.json
  ✓ file exists
  ✓ `name` present
  ✓ `version` present
  ✓ `bspVersion` present
  ✓ `languages` present
  ✓ `argv` present
  ✓ server binary /Users/you/.local/bin/sweetpad exists
  ✓ argv starts a BSP server
```

The last check actually launches the server and speaks to it, so a green line there means the wiring
works, rather than that the file merely looks plausible.

:::warning

All five fields have to be present. sourcekit-lsp skips an incomplete `buildServer.json` **silently**:
no error, no log line, just no completions. If autocomplete stopped working after someone hand-edited
that file, this is the first thing to check.

:::

## When completions are missing or stale

**Jump-to-definition works, but project-wide search doesn't.** Completions and hovers come from the
compiler arguments the server hands over, and it builds whatever dependency modules it needs on
demand, so those work in a fresh clone with no build. Project-wide navigation is different: it reads
the index your builds write, so it only lights up once you've built the project at least once. Run
`sweetpad build` and reopen the workspace.

**Regenerate after the project moves.** The `argv` in `buildServer.json` contains absolute paths, to
the `sweetpad` binary and to your project. Moving the checkout, or switching how you installed
SweetPad, invalidates it. Re-run `sweetpad bsp init`.

**A new file has no completions.** The build server answers per file, so a file the project doesn't
know about yet, added on disk but not in the project, has no arguments to hand over. Add it to the
target, then rebuild.

**Get a log of what the server is doing.** Set `SWEETPAD_BSP_LOG` to a file path and restart your
editor; every request and response is written there. This is the fastest way to tell "sourcekit-lsp
never started the server" apart from "the server started and answered with nothing":

```bash
SWEETPAD_BSP_LOG=/tmp/bsp.log nvim Sources/App/ContentView.swift
```

## Keeping it out of git

`buildServer.json` holds absolute paths specific to your machine, so it isn't shareable. Add it to
`.gitignore` and let each person run `sweetpad bsp init` once:

```gitignore
buildServer.json
```
