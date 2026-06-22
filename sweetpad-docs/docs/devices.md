---
sidebar_position: 6
---

# iOS Devices

SweetPad runs and debugs your app on physical iPhones and iPads, with on-device log streaming and full LLDB
debugging — the same flows you get for the Simulator.

## What works today

- 🚀 Build, install, and launch on a connected device.
- 🐞 Debug with LLDB (breakpoints, step, watch, the lot — see [Debugging](./debug.md)).
- 📋 Stream `os_log` / `Logger` / `print` / `NSLog` output from the device into the SweetPad terminal.
- 🔌 Wireless devices: as long as the device shows up in Xcode's **Devices and Simulators** window, SweetPad will use
  it.

## Requirements

1. **Xcode 15+** and a device paired through Xcode at least once (so the device trusts your Mac).
2. **iOS 17+** to use the modern `xcrun devicectl` flow. Older iOS versions are still detected, but not all features
   are supported.
3. For on-device log streaming and iOS 17+ launches you'll want
   [pymobiledevice3](https://github.com/doronz88/pymobiledevice3) installed (see below).

## Run on a device

1. Connect your iPhone/iPad over USB or have it on the same Wi-Fi network with **Connect via network** enabled in
   Xcode.
2. In the **Destinations** panel (or the status bar destination picker), pick the device.
3. Click ▶️ next to a scheme in the Build view, or press F5 to launch under the debugger.
4. Unlock the device — iOS shows the install prompt during the first launch, then the app starts.

   ![Devices Terminal](/images/devices-terminal.png)

The first launch after a long break, or after rebooting the device, may take a few seconds while iOS re-establishes
the developer tunnel.

## Pair a new device

Pairing only needs to happen once per device/Mac combination:

1. Open Xcode → **Window** → **Devices and Simulators**.
2. Click `+` in the lower-left.
3. Follow the prompts (USB recommended for the first pairing; tick **Connect via network** once paired to use it
   wirelessly afterwards).
4. Back in VSCode, click ↻ on the Destinations panel or run `> SweetPad: Refresh devices list` to pick up the new
   device.

## Stream `os_log` and `print` output from the device

By default, when you launch an app on a physical device, SweetPad streams the device's syslog into the build terminal
and filters it down to your app — `os_log`, `Logger`, `print`, and `NSLog` output all surface there, alongside the
build output. This makes the device feel like the Simulator for everyday debugging.

The stream uses [`pymobiledevice3`](https://github.com/doronz88/pymobiledevice3). Install it with the built-in
helper:

- Run `> SweetPad: Install pymobiledevice3` from the command palette and pick `uv`, `pipx`, or `pip` (`uv` is fastest
  if you already use it; otherwise `pipx` keeps the install isolated).

If you'd rather install it yourself:

```bash
uv tool install pymobiledevice3
# or
pipx install pymobiledevice3
# or
pip install --user pymobiledevice3
```

### Turn the stream off

If you don't care about device logs (or you're streaming them yourself via another tool), disable it globally:

```json title=".vscode/settings.json"
{
  "sweetpad.build.logStreamEnabled": false
}
```

### Filter what reaches the terminal

The device stream comes from `pymobiledevice3 syslog live` (not Apple's `log` tool), so the filtering knobs are
different from those on the Simulator. By default SweetPad keeps only your app's executable and drops Apple
subsystems (`com.apple.*`). Adjust with subsystem allow/deny lists (Apple-style glob patterns):

```json title=".vscode/settings.json"
{
  "sweetpad.build.pymobiledevice3SubsystemAllowList": ["com.myapp.*"],
  "sweetpad.build.pymobiledevice3SubsystemDenyList": ["com.apple.*"]
}
```

`allowList` keeps only matching entries; `denyList` drops matching entries. Use one or both.

:::note

`sweetpad.build.logStreamPredicate` is **not** applied to the device stream — it controls Apple's `log stream` tool,
which only runs for simulators and macOS. See
[Simulators → Customize the predicate](./simulators.md#customize-the-predicate) for that flow.

:::

### Pass extra args to `pymobiledevice3`

If you need flags that aren't covered above, append them with:

```json title=".vscode/settings.json"
{
  "sweetpad.build.pymobiledevice3ExtraArgs": ["--color", "always"]
}
```

### Use a non-default `pymobiledevice3`

If the binary isn't on `PATH`, point at it explicitly:

```json title=".vscode/settings.json"
{
  "sweetpad.build.pymobiledevice3Path": "/Users/me/.local/bin/pymobiledevice3"
}
```

## iOS 17+: the developer tunnel

iOS 17 moved on-device debugging behind a developer tunnel. Xcode normally manages this for you, but launching from
outside Xcode requires `pymobiledevice3 remote tunneld` to be running (and it needs `sudo` for the privileged
network bits).

SweetPad can start it for you:

```json title=".vscode/settings.json"
{
  "sweetpad.build.deviceTunnelAutoStart": true
}
```

With this enabled, the first device launch in a session opens a terminal, runs
`sudo pymobiledevice3 remote tunneld`, and reuses the running tunnel for subsequent launches. You'll be prompted for
your password the first time. Leave it off if you prefer to manage `tunneld` yourself.

## Debug on a device

Debugging on a physical device works the same way as on the Simulator — `F5` with a `sweetpad-lldb` configuration in
`launch.json`. There are a few device-specific knobs (LLDB command merging, "stop on attach") covered in
[Debugging → Debugging on a physical device](./debug.md#debugging-on-a-physical-device).

## Troubleshooting

- **Device missing from the panel.** Click ↻ on the Destinations panel or run `> SweetPad: Refresh devices list`. If
  it still doesn't show up, open **Devices and Simulators** in Xcode and check that the device is paired and trusted.
- **"Could not establish a connection to the device."** Usually means the developer tunnel isn't running. Enable
  `sweetpad.build.deviceTunnelAutoStart`, or run `sudo pymobiledevice3 remote tunneld` yourself.
- **Device launches but no logs appear.** Confirm `pymobiledevice3` is installed and on `PATH`
  (`which pymobiledevice3` should print a path). If you customized the subsystem allow/deny lists, try removing them
  first to make sure they aren't filtering everything out.
