---
sidebar_position: 7
sidebar_label: Destinations & devices
---

# Destinations and devices

Every command that builds needs to know three things: which scheme, which configuration, and where
the result should run. That last one is the **destination**, and it's the one you'll change most
often, between a simulator, your Mac, and a phone on your desk.

## Seeing what's available

`sweetpad devices` lists everything runnable in one place, each with the specifier xcodebuild wants:

```console
$ sweetpad devices
* simulator · iPhone 15 (iOS 26.5) [booted]
    platform=iOS Simulator,id=F92801F8-9EE7-4AF6-8D9E-8D8D8F5A06A3
macOS · My Mac (macOS)
    platform=macOS
simulator · iPhone 15 Plus (iOS 26.5)
    platform=iOS Simulator,id=3AE2296B-7BEF-45F6-AFDF-90D2AD082738
…
```

The list is ordered most-used-first for the current project, so the destination you actually work with
sits at the top. A `*` marks the one this project remembers; `[booted]` marks simulators that are
already running.

Each pool has its own narrower list when that's what you want:

```bash
sweetpad simulator list   # simulators only, with UDIDs
sweetpad device list      # connected physical devices, with their connection state
```

[Simulators](./simulators.md) covers the rest of that group: booting, screenshots, push payloads,
permissions, and managing the pool.

## Saying where to run

`--on` takes a human description and resolves it against that live list:

| You write                   | You get                                             |
| --------------------------- | --------------------------------------------------- |
| `--on "iPhone 16 Pro"`      | The simulator or device with the closest name.      |
| `--on booted`               | Whatever simulator is already running.              |
| `--on mac`                  | Your Mac, for macOS schemes.                        |
| `--on device`               | Your connected physical device.                     |
| `--on ios` / `--on watchos` | Any destination of that platform.                   |
| `--on work-phone`           | An alias you created yourself (see below).          |
| `--on <UDID>`               | That exact simulator or device.                     |

It works on every command that builds or runs:

```bash
sweetpad run --on "iPhone 16 Pro"
sweetpad build --on mac
sweetpad test --on booted
```

Matching is fuzzy, so you don't have to reproduce Apple's exact naming. When a description fits more
than one thing, SweetPad names the candidates instead of picking for you:

```console
$ sweetpad build --on "17 pro"
error: --on "17 pro" is ambiguous (iPhone 17 Pro (26.5), iPhone 17 Pro Max (26.5), iPad mini (A17 Pro) (26.5)) — be more specific
```

An exact name always wins over a longer one that contains it, so `--on "iPhone 15"` picks the iPhone
15 rather than complaining about the Plus and the Pro.

## The choice SweetPad remembers

Most of the time you won't pass `--on` at all. The first build in a project with no destination
settled gets an interactive picker, ordered most-used-first with booted simulators marked, and the
answer is **remembered for that project**. Every later command reuses it.

`sweetpad status` shows what's currently in effect and where each value came from:

```console
$ sweetpad status
/path/to/SweetpadCIApp.xcodeproj (project)
  scheme        SweetpadCIApp  (remembered)
  configuration Debug  (remembered)
  destination   platform=iOS Simulator,id=F92801F8-…  (remembered)
run `sweetpad app run` to build and launch
```

That parenthetical is the useful part. When a build does something you didn't ask for, it names the
layer responsible: a flag, an environment variable, a config file, a remembered answer, or
auto-detection.

:::note

One-off destinations aren't remembered. A `--destination` you typed, and the `--mac` and `--device`
shortcuts, apply to that command only, so a quick check on your Mac doesn't quietly become the
default for the next week.

:::

## Changing what's remembered

`sweetpad context` is how you edit the remembered values. Don't hand-edit the state file; this is the
supported way in.

```bash
sweetpad context show               # everything currently saved for this project
sweetpad context select             # re-answer scheme, configuration, and destination
sweetpad context select destination # re-answer just one, interactively
sweetpad context set scheme MyApp   # set a value with no prompt (scripts and CI)
sweetpad context remove destination # forget one value
sweetpad context remove --all       # forget the whole project context
```

`context set` accepts `scheme`, `configuration`, `sdk`, `destination`, and `target`.

### A separate context for tests

Testing keeps its own remembered destination, so you can develop against one simulator and run the
suite on another. Add `--testing` to any of the commands above to act on it:

```bash
sweetpad context select --testing
sweetpad context set destination 'platform=iOS Simulator,name=iPhone SE (3rd generation)' --testing
```

### Naming a destination

A UDID is not something anyone wants to type twice. Give one a name, then use the name anywhere `--on`
is accepted:

```bash
sweetpad context alias work-phone 00008110-000559182E90401E
sweetpad run --on work-phone
sweetpad context alias work-phone --remove
```

## Physical devices

A connected iPhone or iPad shows up in `sweetpad devices` and in `sweetpad device list`, which also
reports whether it's currently reachable:

```console
$ sweetpad device list
Iphone 13 (iPhone 13, iOS 26.6)  [disconnected]
    00008110-000559182E90401E
```

Target it by name, by UDID, or with the `device` shorthand when there's only one:

```bash
sweetpad run --on device
sweetpad run --on "Iphone 13"
sweetpad run --device-id 00008110-000559182E90401E
```

Device builds have to be signed, which is the one place xcodebuild usually needs more from you than
SweetPad asks for. Pass the signing settings through:

```bash
sweetpad app install --on device -- -allowProvisioningUpdates DEVELOPMENT_TEAM=ABCDE12345
```

If your project always needs them, put them in `sweetpad.toml` once instead. See
[Extra xcodebuild arguments](./reference.md#extra-xcodebuild-arguments).

:::tip

A device that's plugged in isn't necessarily ready. It also has to be unlocked, trusted, and in
Developer Mode before xcodebuild can reach it. When a device build stalls looking for a destination,
that's the first thing to check.

:::

## macOS

macOS schemes run natively, with no simulator involved:

```bash
sweetpad run --on mac
sweetpad run --mac      # the same thing
```

The macOS destination is also where the CLI's Mac-only verbs apply: `app screenshot` captures the
app's window, and `app ui` reads and drives it through accessibility.

## The raw escape hatch

`--destination` takes xcodebuild's exact specifier, with no fuzzy matching:

```bash
sweetpad build --destination 'platform=iOS Simulator,name=iPhone 16 Pro'
sweetpad build --destination 'platform=iOS,id=00008110-000559182E90401E'
sweetpad build --destination 'platform=macOS'
```

`--on` and `--destination` are mutually exclusive, so pick one per command.

Prefer `--destination` in CI. Fuzzy matching against whatever simulators a runner happens to have
installed is a liability, and a pinned specifier fails loudly instead of quietly building for the
wrong thing. `sweetpad devices` prints a copy-paste-ready specifier for everything it lists.

Every targeting flag also has an environment-variable twin, so a pipeline can set the context once
instead of on every command:

```bash
export SWEETPAD_SCHEME=MyApp
export SWEETPAD_DESTINATION='platform=iOS Simulator,name=iPhone 16 Pro'
sweetpad build && sweetpad test
```

## Which setting wins

When the same value is set in more than one place, the most explicit one wins:

```
flag  >  environment variable  >  config.toml  >  sweetpad.toml  >  remembered answer  >  auto-detect
```

`sweetpad status` always reports which of those a value came from, and `sweetpad help destinations`
has the same material available offline.
