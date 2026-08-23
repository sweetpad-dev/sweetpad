---
sidebar_position: 8
sidebar_label: Simulators
---

# Simulators

`sweetpad simulator`, or `sim` for short, manages the simulator pool and drives a booted one. Most of
these wrap `simctl` with names you can remember and defaults that guess right.

```console
$ sweetpad simulator list
* iOS iPhone 15 (26.5)  F92801F8-9EE7-4AF6-8D9E-8D8D8F5A06A3 [booted]
  iOS iPhone 15 Plus (26.5)  3AE2296B-7BEF-45F6-AFDF-90D2AD082738
  iOS iPhone 15 Pro (26.5)  B6044142-DD54-4E6C-B973-1B628E418976
…
```

Almost every verb takes an optional target, a name or a UDID, and **defaults to the booted
simulator** when you leave it out. So the common case is short:

```bash
sweetpad sim screenshot                    # the booted one
sweetpad sim screenshot "iPhone 16 Pro"    # a specific one
```

## Booting and shutting down

```bash
sweetpad sim boot                    # prompts if you don't say which
sweetpad sim boot "iPhone 16 Pro"
sweetpad sim boot "iPhone 16 Pro" --wait   # block until it's fully ready
sweetpad sim shutdown
sweetpad sim open                    # open the Simulator.app window
```

`--wait` is the one that matters in a script. Without it, `boot` returns as soon as the boot has
started, and the next command can arrive before the simulator can answer.

You usually don't need to boot anything by hand, since `sweetpad run` boots what it needs.

## Screenshots and video

```bash
sweetpad sim screenshot
sweetpad sim screenshot --output-file ./docs/home.png
sweetpad sim screenshot --clipboard          # also copy it
```

Without `--output-file`, images land in `./sweetpad-shots/` named for the device and the time.

Video works the same way, and Ctrl-C is how you stop it. The recording is finalized on the way out,
so the file is playable:

```bash
sweetpad sim record
sweetpad sim record --output-file ./demo.mp4
```

### Screenshots worth shipping

App Store screenshots want a clean status bar and a predictable appearance. Two commands set that up:

```bash
sweetpad sim status-bar                # 9:41, full signal, full battery
sweetpad sim appearance dark
sweetpad sim screenshot --output-file ./shots/home-dark.png
sweetpad sim status-bar --clear        # back to the real status bar
```

The status-bar override sticks until you clear it or the simulator is erased, so a batch of
screenshots only needs it set once.

## Push notifications

`sim push` delivers an APNs payload from a JSON file, so you can test notification handling without a
server:

```json title="push.json"
{
  "Simulator Target Bundle": "com.example.MyApp",
  "aps": {
    "alert": { "title": "New message", "body": "Tap to read it" },
    "badge": 1,
    "sound": "default"
  }
}
```

```bash
sweetpad sim push com.example.MyApp ./push.json
```

The bundle identifier goes on the command line as well as in the payload. If you don't know it,
`sweetpad settings show --key PRODUCT_BUNDLE_IDENTIFIER` prints it.

## Permissions

`sim privacy` grants, revokes, or resets a permission for an app, which is how you test both the
happy path and the one where the user said no, without tapping through the alert each time:

```bash
sweetpad sim privacy grant photos com.example.MyApp
sweetpad sim privacy revoke camera com.example.MyApp
sweetpad sim privacy reset location com.example.MyApp
```

Services include `photos`, `camera`, `microphone`, `location`, `contacts`, and `calendar`, among
others.

## Location

```bash
sweetpad sim location 50.4501 30.5234
```

Latitude then longitude. Useful for anything map- or geofence-shaped, and for making a demo look the
same every time.

## Media

Seed the photo library so an image picker has something to pick:

```bash
sweetpad sim media-add ./fixtures/photo1.jpg ./fixtures/clip.mov
sweetpad sim media-add ./fixtures/*.jpg --target "iPhone 16 Pro"
```

Note that `media-add` takes its target as a `--target` flag rather than a trailing argument, since the
file list is positional.

## Deep links

Opening a URL belongs to the app, not the simulator, so it lives under `app`:

```bash
sweetpad app open-url "myapp://profile/42"
sweetpad app open-url "https://example.com/invite/abc"
```

See [App lifecycle and debugging](./app-lifecycle.md) for the rest of that group.

## Managing the pool

```bash
sweetpad sim create "CI iPhone" "iPhone 16 Pro"
sweetpad sim create "CI iPhone" "iPhone 16 Pro" --runtime "iOS 18.0"
sweetpad sim clone "iPhone 16 Pro" "iPhone 16 Pro (clean)"
sweetpad sim erase "CI iPhone"
sweetpad sim delete "CI iPhone" --yes
```

`create` takes the newest available runtime unless you name one. `clone` needs the source shut down,
and so does `erase`. A clean baseline you clone before each test run is a common CI pattern.

:::warning

Both are destructive, and they guard themselves differently. `delete` requires you to name the target
and refuses to run unconfirmed: `error: refusing to delete a simulator without confirmation; pass
--yes`. `erase` has no such guard: it doesn't prompt, and it *does* default to the booted simulator,
so a bare `sweetpad sim erase` wipes whatever you happen to have running. Name the target when you're
not certain what that is.

:::
