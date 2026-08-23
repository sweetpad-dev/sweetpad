---
sidebar_position: 17
sidebar_label: Commands reference
---

# Commands reference

Everything SweetPad can do is exposed as a command. Open the command palette (`⌘⇧P`), type "sweetpad",
and every command below shows up. Most are also reachable from the sidebar, where buttons and right-click
menus in the Build, Destinations, and Tools views run the same commands.

:::tip

To put a command on a keyboard shortcut, open **Keyboard Shortcuts** (`⌘K ⌘S`) and search for
"sweetpad". Every command here can be bound to a key.

:::

## Build and run

| Command                                    | What it does                                                                 |
| ------------------------------------------- | ----------------------------------------------------------------------------- |
| SweetPad: Build & Run (Launch)              | Build the active scheme and launch it on the active destination.             |
| SweetPad: Build (without Run)               | Build only.                                                                   |
| SweetPad: Run (without Build)               | Launch the already-built app without rebuilding.                              |
| SweetPad: Test                              | Run the scheme's tests.                                                       |
| SweetPad: Clean                             | Clean the build folder and derived data.                                      |
| SweetPad: Stop build / running app          | Kill the running build or launched app and free the terminal.                 |
| SweetPad: Resolve dependencies              | Resolve Swift Package Manager dependencies.                                   |
| SweetPad: Remove bundle directory           | Delete SweetPad's intermediate app-bundle directory.                          |
| SweetPad: Set scheme                        | Pick the scheme used for builds.                                              |
| SweetPad: Select build configuration        | Pick the configuration (Debug/Release/…).                                     |
| SweetPad: Select Xcode workspace            | Pick which workspace or project SweetPad builds, and save it to settings.     |
| SweetPad: Refresh schemes                   | Re-read the scheme list from the project.                                     |
| SweetPad: Search schemes (Build view)       | Filter the Build view by name.                                                |
| SweetPad: Apply scheme filter (Build view)  | Re-apply the scheme include/exclude filter from settings.                     |
| SweetPad: Pause scheme filter (Build view)  | Temporarily show all schemes, ignoring the filter.                            |
| SweetPad: Diagnose build setup              | Walk the workspace-detection logic and print what was (or wasn't) found.      |
| SweetPad: Switch Git Worktree               | Point the build at another [git worktree](./worktree.md) of the same repo.    |
| SweetPad: Open Xcode                        | Open the current project in Xcode.                                            |

## Autocomplete and code intelligence

| Command                                            | What it does                                                        |
| --------------------------------------------------- | --------------------------------------------------------------------- |
| SweetPad: Generate Build Server Config (buildServer.json) | Write `buildServer.json` for SourceKit-LSP and restart the LSP. |
| SweetPad: Set up Swift code intelligence (BSP)      | Switch to the built-in build server and write its config.            |
| SweetPad: Diagnose BSP (Doctor)                     | Check the autocomplete chain and print a ✓/✗ report with fix hints.   |
| SweetPad: Show BSP logs                             | Open the build server's log stream (built-in provider).              |
| SweetPad: Enable LSP Diagnostics                    | Turn build-log squiggles in the editor back on.                       |
| SweetPad: Disable LSP Diagnostics                   | Turn build-log squiggles off (e.g. to avoid duplicates).              |

## Testing

| Command                                                       | What it does                                          |
| -------------------------------------------------------------- | ------------------------------------------------------- |
| SweetPad.Testing: Set scheme for testing                        | Pick the scheme used when running tests.               |
| SweetPad.Testing: Select testing target                         | Pick the test target when a project has several.       |
| SweetPad.Testing: Select configuration for testing              | Pick the configuration used for test builds.           |
| SweetPad.Testing: Select destination for testing                | Pick a separate destination just for tests.            |
| SweetPad.Testing: Build for testing (without running tests)     | Compile the test bundle only.                          |
| SweetPad.Testing: Test without building                         | Run already-built tests without recompiling.           |

## Destinations, simulators, and devices

| Command                                | What it does                                                        |
| --------------------------------------- | --------------------------------------------------------------------- |
| SweetPad: Select destination            | Pick where the app runs: simulator, device, or macOS.                 |
| SweetPad: Search destinations (Destinations view) | Filter the Destinations view by name.                       |
| SweetPad: Remove recent destination     | Drop an entry from the Recent group.                                 |
| SweetPad: Start simulator               | Boot a simulator.                                                     |
| SweetPad: Stop simulator                | Shut a simulator down.                                                |
| SweetPad: Open simulator                | Open the Simulator.app window.                                        |
| SweetPad: Remove simulator cache        | Clear the simulator cache (fixes some boot failures).                 |
| SweetPad: Refresh simulators list       | Re-read installed simulators.                                         |
| SweetPad: Refresh devices list          | Re-scan for connected devices.                                        |
| SweetPad: Install pymobiledevice3       | Install the tool used for device log streaming and the iOS 17+ tunnel. |

## Formatting

| Command                     | What it does                                       |
| ---------------------------- | ---------------------------------------------------- |
| SweetPad: Format             | Format the current file.                             |
| SweetPad: Show format logs   | Open the formatter's output channel.                 |

## Tuist and XcodeGen

| Command                                                  | What it does                                    |
| --------------------------------------------------------- | ------------------------------------------------- |
| SweetPad: Generate an Xcode project using Tuist            | Run Tuist generate at the workspace root.        |
| SweetPad: Install Swift Package using Tuist                | Run Tuist install.                               |
| SweetPad: Clean Tuist project                              | Remove Tuist's generated files.                  |
| SweetPad: Edit Tuist project (Open project in Xcode)       | Open the Tuist manifests in Xcode.               |
| SweetPad: Test Generated project using Tuist               | Run Tuist test across every target.              |
| SweetPad: Generate an Xcode project using XcodeGen         | Run XcodeGen generate at the workspace root.     |

## Debugging

These back the `sweetpad-lldb` debug configuration, and you rarely run them by hand. See
[Debugging](./debug.md).

| Command                              | What it does                                                       |
| ------------------------------------- | -------------------------------------------------------------------- |
| SweetPad: Build & Run (for debugging) | Build and launch, signalling the debugger when the app is ready.    |
| SweetPad: Build (for debugging)       | Build only, as a debug pre-launch step.                             |
| SweetPad: Run (for debugging)         | Launch without building, as a debug pre-launch step.                |
| SweetPad: Get app path for debugging  | Resolve the last-built app's path (used as a launch.json variable). |

## Tools

| Command                            | What it does                                             |
| ----------------------------------- | ---------------------------------------------------------- |
| SweetPad: Install tool              | Install one of the integrated tools via Homebrew.         |
| SweetPad: Open tool documentation   | Open a tool's docs in the browser.                        |
| SweetPad: Refresh tools list        | Re-check which tools are installed.                       |

## Agent CLI

See [Agent CLI & RPC server](../cli/agent-cli.md).

| Command                             | What it does                                    |
| ------------------------------------ | ------------------------------------------------- |
| SweetPad: Show RPC server status     | Print the server name, socket path, and process info. |
| SweetPad: Copy RPC server name       | Copy the server name to the clipboard.           |
| SweetPad: Restart RPC server         | Restart the server after settings changes or a stuck state. |

## Housekeeping and troubleshooting

See [Troubleshooting](./troubleshooting.md).

| Command                                        | What it does                                                    |
| ----------------------------------------------- | ----------------------------------------------------------------- |
| SweetPad: Reset Extension Cache                 | Forget remembered choices (workspace, scheme, destination, …).   |
| SweetPad: Refresh shell environment             | Re-read `PATH` and friends from your login shell.                |
| SweetPad: Open Terminal Panel                   | Reveal SweetPad's terminal panel.                                |
| SweetPad: Create Issue on GitHub                | Open a pre-filled bug report.                                    |
| SweetPad: Create Issue on GitHub (No Schemes)   | Open a bug report pre-filled with scheme-detection diagnostics.  |
| SweetPad: Test Error Reporting                  | Fire a test error to verify Sentry reporting is wired up.        |
