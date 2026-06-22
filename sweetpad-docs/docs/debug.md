---
sidebar_position: 4
---

# Debugging

SweetPad integrates with the [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb)
extension — powered by [LLDB](https://lldb.llvm.org/) — so you can debug your iOS app directly from VSCode.

## Tutorial

1. **Create `launch.json`.**  
   In the `.vscode` folder of your project, create a `launch.json` file with the following content:

   ```json title=".vscode/launch.json"
   {
     "version": "0.2.0",
     "configurations": [
       {
         "type": "sweetpad-lldb",
         "request": "attach",
         "name": "Attach to running app (SweetPad)",
         "preLaunchTask": "sweetpad: launch"
       }
     ]
   }
   ```

   You can also generate this file by clicking **Create a launch.json file** in the _Run and Debug_ panel.

   ![Create launch.json](/images/debug-create-launch-json.png)  
   ![Select SweetPad LLDB](/images/debug-select-sweetpad-lldb.png)  
   ![Update launch.json](/images/debug-update-launch-json.png)

2. **Configure the LLDB backend.** If you haven't done so already, point CodeLLDB at Xcode's bundled LLDB by adding
   the following to your `settings.json`:

   ```json title="settings.json"
   {
     "lldb.library": "/Applications/Xcode.app/Contents/SharedFrameworks/LLDB.framework/Versions/A/LLDB"
   }
   ```

   That's the default path for a stock Xcode install — adjust it if your Xcode lives somewhere else.

   Alternatively, run **LLDB: Use Alternate Backend** from the command palette and type "lldb" to let CodeLLDB locate
   the library for you.

3. **Start debugging (`F5`).**  
   Press **F5**. SweetPad builds the app, launches it in the Simulator, and attaches LLDB to the running process.

   ![Launch debugger](/images/debug-launch-debugger.png)

4. **Set breakpoints and iterate.**  
   Place breakpoints and debug as usual. On subsequent runs, just press **F5** again — SweetPad rebuilds, relaunches,
   and reattaches.

   ![Breakpoints](/images/debug-breakpoints.png)

## Customize `preLaunchTask`

If you need more control, you can point the `preLaunchTask` property to a custom task defined in `.vscode/tasks.json`.  
For example, the task below builds the app with the **Release** scheme before launching the debugger:

```json title=".vscode/tasks.json"
{
  "version": "2.0.0",
  "tasks": [
    {
      "type": "sweetpad",
      "action": "launch",
      "label": "sweetpad: launch release",
      "detail": "Build and launch the app (Release)",
      "scheme": "Release",
      "configuration": "Release",
      "isBackground": true, // Important: lets VSCode know when the task is ready
      "problemMatcher": ["$sweetpad-watch"]
    }
  ]
}
```

Then reference that task from `launch.json`:

```json title=".vscode/launch.json"
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "sweetpad-lldb",
      "request": "attach",
      "name": "Attach to running app (SweetPad – Release)",
      "preLaunchTask": "sweetpad: launch release"
    }
  ]
}
```

## Passing CodeLLDB parameters

To pass additional parameters to CodeLLDB, use the `codelldbAttributes` property in your `launch.json` file. For
example, if you want to execute LLDB commands before the debugger starts, you can do it like this:

```json title=".vscode/launch.json"
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "sweetpad-lldb",
      "request": "attach",
      "name": "Attach to running app (SweetPad)",
      "preLaunchTask": "sweetpad: launch",
      "codelldbAttributes": {
        "initCommands": [
          // This command will be executed before the debugger starts
          "script print('Hello from LLDB!')"
        ]
      }
    }
  ]
}
```

The full list of available parameters for `codelldbAttributes` can be found in the
[CodeLLDB documentation](https://github.com/vadimcn/codelldb/blob/master/MANUAL.md#starting-a-new-debug-session).

## Debugging on a physical device

This section is only relevant if you're debugging an app running on a physical device. Debugging on a device should
generally work out of the box, but there are some differences compared to the simulator that you should be aware of.
On iOS 17+ the device launch goes through a developer tunnel managed by `pymobiledevice3`; see
[Devices → iOS 17+: the developer tunnel](./devices.md#ios-17-the-developer-tunnel) for the one-time setup.

### Merging `codelldbAttributes`

When attaching to an app running **on a physical device**, SweetPad injects its own LLDB commands into
`initCommands`, `preRunCommands`, and `processCreateCommands`. If you supply your own commands through
`codelldbAttributes`, SweetPad merges them in this order:

```json
{
  "codelldbAttributes": {
    "initCommands": [...yourCommands, ...sweetpadCommands],
    "preRunCommands": [...yourCommands, ...sweetpadCommands],
    "processCreateCommands": [...yourCommands, ...sweetpadCommands]
  }
}
```

For the exact commands SweetPad injects, see
[resolveDeviceDebugConfiguration](https://github.com/sweetpad-dev/sweetpad/blob/main/src/debugger/provider.ts) in the
extension source.

### Stop on attach

By default, SweetPad tells the debugger to continue running immediately after attaching, so you don't end up paused
on an arbitrary instruction with no breakpoints set. If you'd rather have the debugger **stop on attach**, add
`"continueOnAttach": false` to your configuration:

```json
{
  "type": "sweetpad-lldb",
  "request": "attach",
  "name": "Attach to running app (SweetPad)",
  "preLaunchTask": "sweetpad: launch",
  "continueOnAttach": false,
  "codelldbAttributes": {}
}
```

Note that `continueOnAttach` is a SweetPad-specific attribute, not part of the CodeLLDB configuration.

## Old tutorial (deprecated)

:::warning

The following method is retained for backwards compatibility. It still works, but the flow above is the recommended
one.

:::

1. **Install CodeLLDB.** Install the [CodeLLDB](https://marketplace.visualstudio.com/items?itemName=vadimcn.vscode-lldb)
   extension from the VSCode Marketplace.

   ![Install CodeLLDB](/images/debug-old-install-codelldb.png)

2. **Create `launch.json`.** Add the configuration below:

   ```json title=".vscode/launch.json"
   {
     "version": "0.2.0",
     "configurations": [
       {
         "type": "lldb",
         "request": "attach",
         "name": "Attach to iOS Simulator",
         "waitFor": true,
         "program": "${command:sweetpad.debugger.getAppPath}"
       }
     ]
   }
   ```

![Create launch.json](/images/debug-old-create-launch-json.png)
![Update launch.json](/images/debug-old-update-launch-json.png)

The `${command:sweetpad.debugger.getAppPath}` variable resolves at runtime to the path of the app most recently built
by SweetPad — CodeLLDB needs it to attach to the simulator. See the
[CodeLLDB manual](https://github.com/vadimcn/codelldb/blob/master/MANUAL.md) for the full set of options.

3. **Launch the app.** Start the iOS Simulator and run **SweetPad › Launch** from the _Build_ panel.

   ![Launch](/images/debug-old-launch-app.png)

4. **Attach the debugger.** In the _Run and Debug_ panel, select **Attach to iOS Simulator**. When the _Call Stack_
   appears, the debugger is successfully attached.

   ![Attach](/images/debug-old-attach-ios-simulator.png)

5. **Debug.** Set breakpoints and debug as usual. For subsequent sessions, skip straight to step 4.

   ![Breakpoints](/images/debug-old-breakpoints.png)
