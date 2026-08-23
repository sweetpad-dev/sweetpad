---
sidebar_position: 6
sidebar_label: Hot reload
---

# Hot reload

Hot reload replaces the build-and-relaunch loop with something much shorter: save a Swift file, and
that file alone is recompiled and injected into the app that's already running. The app doesn't
restart, so it keeps its current screen, its navigation stack, and whatever state you'd built up
getting there.

```bash
sweetpad run --hot
```

Everything else about the run is unchanged. `r` still forces a full rebuild and relaunch, and `q`
quits.

## Where it works

| Target             | Hot reload                                                    |
| ------------------ | ------------------------------------------------------------- |
| iOS Simulator      | Yes.                                                          |
| Native macOS apps  | Yes, with `--mac --hot`. See the macOS notes below.           |
| Physical devices   | No. iOS strips the launch environment injection depends on.   |

There's nothing to install. The injection client ships inside the `sweetpad` binary, so `--hot` works
on a fresh machine with only Homebrew and Xcode. (If you happen to have InjectionNext installed,
SweetPad falls back to its client when the bundled one can't be used.)

## Setting up SwiftUI

UIKit and AppKit apps need no changes at all. Method bodies are swapped in the running process and
the next redraw picks them up.

SwiftUI needs two lines, because SwiftUI caches the result of `body` and only recomputes it when an
observed input changes. Injection rebinds the code, but without a nudge the view has no reason to
redraw.

### 1. Add the Inject package

Add [Inject](https://github.com/krzysztofzablocki/Inject) to your app target, in Xcode via _File →
Add Package Dependencies…_, or in `Package.swift` alongside your other dependencies.

### 2. Annotate the views you're editing

```swift title="ContentView.swift"
import SwiftUI
import Inject

struct ContentView: View {
  // highlight-next-line
  @ObserveInjection var inject

  var body: some View {
    Text("Hello, hot reload!")
      // highlight-next-line
      .enableInjection()
  }
}
```

`@ObserveInjection` subscribes the view to injection events, which is what makes SwiftUI treat it as
invalidated. `.enableInjection()` loads the injection runtime and type-erases the view, so a
structural change between the old and new `body` doesn't trip SwiftUI's diffing.

:::tip

You don't need this on every view. Put it on your root view and SwiftUI invalidates from there down,
so any edit in the tree gets picked up, at the cost of re-rendering everything on each save. For a
large app, annotate just the views you're actively iterating on.

:::

## Turning it on by default

If a project is one you always work on this way, set it in `sweetpad.toml` and drop the flag:

```toml
# sweetpad.toml
[run]
hot = true
```

`--no-hot` overrides that for a single run, which is what you want when you're checking real
cold-start behavior.

## Choosing a recompiler

Each save has to be turned back into compiled code, and there are two ways to work out the compiler
arguments for one file. `--hot-recompiler` picks between them:

- **`resolver`** (the default) resolves the build settings itself and does a whole-module compile.
  Slower per save, but it doesn't depend on anything left over from an earlier build.
- **`buildlog`** recovers the single-file compile from the build transcript. Noticeably faster, and
  the right choice once a project is building cleanly and you're iterating hard.

```bash
sweetpad run --hot --hot-recompiler buildlog
```

## macOS notes

A macOS app has to be injectable, and a release-shaped one isn't. A `--mac --hot` build turns off the
hardened runtime and the App Sandbox for that Debug product, so the system honors the injected code
and accepts the recompiled libraries.

Two consequences are worth knowing before you're surprised by them.

**An App Sandbox set in an explicit entitlements file can't be overridden.** SweetPad's preflight
detects this and tells you what to change; the fix is to turn the sandbox off for your Debug
configuration. Two flags exist for the cases where you want to steer this yourself:
`--hot-entitlements <file>` signs with entitlements you supply instead of the automatically derived
ones, and `--keep-sandbox` skips the un-sandboxing entirely. A sandboxed product then fails the
injection preflight, which prints the manual fix.

**An unsandboxed run doesn't use the sandbox container.** Preferences and files the app writes land in
your home Library rather than in the container it normally uses, so a hot session won't see data a
sandboxed run wrote, and vice versa.

## When the port is stuck

One hot-reload session owns the injection port at a time. A session that died without cleaning up, whether a killed terminal or a crash, can leave the listener bound, and the next `--hot` run then fails with
"Address already in use".

```bash
sweetpad hot status   # is the port free, and if not, who holds it?
sweetpad hot reset    # end a leftover sweetpad listener
```

`hot reset` refuses to kill a process that isn't SweetPad's, since that's usually something you
started on purpose. `--force` overrides it, for example when InjectionNext is running and holding the
port.

## Troubleshooting

**Saves do nothing in a SwiftUI view.** The view is missing `@ObserveInjection` or
`.enableInjection()`, or the file you saved defines something below an annotated view but nothing
subscribes. Start by annotating the root view.

**It works, then stops after an unrelated change.** Some edits can't be injected. Changing a type's
stored properties, for instance, changes its layout. Press `r` for a full rebuild and carry on.

**Nothing injects on a physical device.** That's a platform limit rather than a setup problem: iOS strips
the launch environment injection relies on. Use a simulator for the fast loop.

`sweetpad help hot-reload` has the same material available offline, without leaving the terminal.
