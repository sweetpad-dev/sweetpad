---
sidebar_position: 13
---

# Troubleshooting

## See logs

1. Bump SweetPad's log level to `debug`:

   ```json title=".vscode/settings.json"
   {
     "sweetpad.system.logLevel": "debug"
   }
   ```

2. Restart VSCode to apply the change.
3. Open the **Output** panel and select a SweetPad channel.

![debug](/images/troubleshooting-output-panel.png)

## Reset extension cache

The extension caches user choices such as the simulator to run on, configuration to use, or active workspace. If you
encounter any issues or just want to make another choice, you can reset the cache by running the
`> SweetPad: Reset Extension Cache` command from the command palette.

## Diagnose build setup

If the Build view says "No Xcode scheme was found", or builds fail before they reach `xcodebuild`, run
`> SweetPad: Diagnose build setup` from the command palette. It walks through the workspace detection logic and
prints what it found (or didn't find) so you can see whether the workspace path, derived data, or Xcode CLI tools
are the culprit.

## Refresh the shell environment

SweetPad reads `$PATH` and other variables from your **login shell** when it activates — so tools installed by
[mise](https://mise.jdx.dev/), [asdf](https://asdf-vm.com/), [direnv](https://direnv.net/), Homebrew, etc. are
visible to `xcodebuild`, `swift-format`, and other binaries it calls.

If you change your shell profile (`~/.zshrc`, `~/.zprofile`, etc.) and want SweetPad to see the new values without
restarting VSCode, run `> SweetPad: Refresh shell environment`.

If your shell startup files are slow and the initial activation times out, raise the timeout:

```json title=".vscode/settings.json"
{
  "sweetpad.shellEnv.timeout": 15000
}
```

Or pin the shell SweetPad uses (defaults to `$SHELL`):

```json title=".vscode/settings.json"
{
  "sweetpad.shellEnv.shell": "/bin/zsh"
}
```

## Choose the task executor

SweetPad runs `xcodebuild` (and other tasks) through a task executor. The default `v3` executor uses a real
pseudoterminal (via `node-pty`), so build output has ANSI colors, TUI commands work, and the resolved login-shell
environment is available to subprocesses.

If you hit an issue specific to v3 (rare — usually flaky terminals or unusual shells), fall back to the older
implementation:

```json title=".vscode/settings.json"
{
  "sweetpad.system.taskExecutor": "v2"
}
```

`v2` uses plain pipes — no PTY, no color, no TUI support — but a smaller surface for bugs.

## Install pymobiledevice3

On-device log streaming and the iOS 17+ developer tunnel both rely on `pymobiledevice3`. Install it without leaving
VSCode:

- Run `> SweetPad: Install pymobiledevice3` and pick `uv`, `pipx`, or `pip`.

If `pymobiledevice3` is installed but SweetPad can't find it, point at the binary explicitly:

```json title=".vscode/settings.json"
{
  "sweetpad.build.pymobiledevice3Path": "/Users/me/.local/bin/pymobiledevice3"
}
```

See [Devices](./devices.md) for filtering, the tunnel auto-start setting, and other on-device options.

## Enable Sentry

:::tip

Sending error reports to the SweetPad team is optional and is **disabled** by default. The error reports may contain
sensitive information about your project, so make sure you understand what you're doing.

:::

If you hit an issue, enable Sentry so SweetPad can send the error report to the maintainers. Set
`sweetpad.system.enableSentry` to `true`:

```json title=".vscode/settings.json"
{
  "sweetpad.system.enableSentry": true
}
```

Then restart VSCode and reproduce the issue. If Sentry is wired up correctly, you'll see this entry in the **Output**
panel (`Output` → `SweetPad: Common`). SweetPad's logger formats messages as YAML-like blocks, so look for:

```yaml
---
level: INFO
message: "Sentry setup"
context:
  sentryIsEnabled: true
```

![sentry](/images/troubleshooting-sentry.png)

## Report a bug

If a fix isn't obvious, two commands open a pre-filled GitHub issue with the right context:

- `> SweetPad: Create Issue on GitHub` — generic crash/bug report.
- `> SweetPad: Create Issue on GitHub (No Schemes)` — specifically for "No Xcode scheme was found" errors; includes
  diagnostic output.
