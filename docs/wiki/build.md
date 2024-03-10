# SweetPad: Build & Run app on iOS Simulator

You can build and run your iOS app directly on the simulator from the VSCode sidebar. This functionality leverages
`xcodebuild`, a component of the Xcode command-line tools.

![iOS simulator](../images/build-demo.gif)

To build and run your app on the simulator, first open the folder containing your Xcode project. Then, open the SweetPad
tools panel on the left side of VSCode, navigate to the **"Build"** section, and click the **"Build & Run"** button next
to the schema name ▶️. The extension will prompt you to select a simulator, and then it will build and run your app on
the chosen simulator.

For better output, I highly recommend installing `xcbeautify` as well:

```bash
brew install xcbeautify
```

Alternatively, you can use the **"Tools"** section in the SweetPad panel to install `xcbeautify` and other essential
iOS.

## Main parts of the "Build" section:

[![iOS simulator](../images/build-preview.png)](../images/build-preview.png)

1. ▶️ **Build & Run** — Click the play `▶️` button next to the schema name to build and run the app on the simulator.
2. ⚙️ **Build** — Click the gear `⚙️` button next to the schema name just to build the app.
3. **SweetPad: Clean** — right-click on the schema name to see the "Clean" option. This option will clean the build
   folder and derived data.
4. **SweetPad: Resolve Dependencies** — right-click on the schema name to see the "Resolve Dependencies" option. This
   option will resolve the dependencies using Swift Package Manager.

   ![Context Menu](../images/build-context-menu.png)

> ⚠️ This feature is currently in alpha and may not perform as expected. Should you encounter any issues, please report
> them by opening an issue on the SweetPad GitHub repository.

## Tasks

SweetPad also provide TaskProvider that automatically provides tasks for building and running the app on the simulator.
You can run these tasks from the command palette by typing `Tasks: Run Task` and selecting the desired task.

[![Tasks](../images/tasks-preview.png)](../images/tasks-preview.png)

Or you can add tasks to the `tasks.json` file in the `.vscode` folder of your project:

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "Build & Run",
      "type": "shell",
      "command": "xcodebuild",
      "args": ["-scheme", "MyApp", "-destination", "platform=iOS Simulator,name=iPhone 11", "build", "test"],
      "group": {
        "kind": "build",
        "isDefault": true
      }
    }
  ]
}
```
