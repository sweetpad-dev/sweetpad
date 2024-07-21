import * as vscode from "vscode";
import {
  askConfiguration,
  askDestinationToRunOn,
  askScheme,
  askXcodeWorkspacePath,
  getDestinationByUdid,
} from "./utils";
import { buildApp, runOniOSSimulator, resolveDependencies, runOniOSDevice, getDestinationRaw } from "./commands";
import { ExtensionContext } from "../common/commands";
import {
  TaskTerminalV2,
  TaskTerminalV1,
  TaskTerminal,
  getTaskExecutorName,
  TaskTerminalV1Parent,
} from "../common/tasks";
import { BuildSettingsOutput, getBuildSettings } from "../common/cli/scripts";

interface TaskDefinition extends vscode.TaskDefinition {
  type: string;
  action: string;
  scheme?: string;
  configuration?: string;
  workspace?: string;
  simulator?: string; // deprecated, use "destinationId" or "destinationRaw"
  destinationId?: string; // ex: "00000000-0000-0000-0000-000000000000"
  destination?: string; // ex: "platform=iOS Simulator,id=00000000-0000-0000-0000-000000000000"
}

class ActionDispatcher {
  context: ExtensionContext;
  constructor(context: ExtensionContext) {
    this.context = context;
  }

  async do(terminal: TaskTerminal, definition: TaskDefinition) {
    const action = definition.action;
    switch (action) {
      case "launch":
        await this.launchCallback(terminal, definition);
        break;
      case "build":
        await this.buildCallback(terminal, definition);
        break;
      case "clean":
        await this.cleanCallback(terminal, definition);
        break;
      case "test":
        await this.testCallback(terminal, definition);
        break;
      case "resolve-dependencies":
        await this.resolveDependenciesCallback(terminal, definition);
        break;
      default:
        throw new Error(`Action ${action} is not supported`);
    }
  }

  private async getDestination(options: { definition: TaskDefinition; buildSettings: BuildSettingsOutput }) {
    const destinationUdid: string | undefined =
      // ex: "00000000-0000-0000-0000-000000000000"
      options.definition.destinationId ??
      // ex: "00000000-0000-0000-0000-000000000000"
      options.definition.simulator ??
      // ex: "platform=iOS Simulator,id=00000000-0000-0000-0000-000000000000"
      options.definition.destination?.match(/id=(.+)/)?.[1];

    // If user has provided the ID of the destination, then use it directly
    if (destinationUdid) {
      return await getDestinationByUdid(this.context, { udid: destinationUdid });
    }

    // Otherwise, ask the user to select the destination (it will be cached for the next time)
    const destination = await askDestinationToRunOn(this.context, options.buildSettings);
    return destination;
  }

  private async launchCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    const xcworkspace = await askXcodeWorkspacePath(this.context);
    const scheme =
      definition.scheme ??
      (await askScheme({
        xcworkspace: xcworkspace,
      }));

    const configuration =
      definition.configuration ??
      (await askConfiguration(this.context, {
        xcworkspace: xcworkspace,
      }));

    const buildSettings = await getBuildSettings({
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    const destination = await this.getDestination({
      definition: definition,
      buildSettings: buildSettings,
    });
    const destinationRaw =
      definition.destination ??
      getDestinationRaw({
        platform: destination.platform,
        id: destination.udid,
      });

    const sdk = destination.platform;

    await buildApp(this.context, terminal, {
      scheme: scheme,
      sdk: sdk,
      configuration: configuration,
      shouldBuild: true,
      shouldClean: false,
      shouldTest: false,
      xcworkspace: xcworkspace,
      destinationRaw: destinationRaw,
      // destinationType: sdk,
      // destinationId: definition.destinationId,
    });

    if (destination.type == "iOSSimulator") {
      await runOniOSSimulator(this.context, terminal, {
        scheme: scheme,
        simulatorId: destination.udid,
        sdk: sdk,
        configuration: configuration,
        xcworkspace: xcworkspace,
      });
    } else {
      await runOniOSDevice(this.context, terminal, {
        scheme: scheme,
        deviceId: destination.udid ?? "",
        sdk: sdk,
        configuration: configuration,
        xcworkspace: xcworkspace,
      });
    }
  }

  private async buildCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    const xcworkspace = await askXcodeWorkspacePath(this.context);
    const scheme =
      definition.scheme ??
      (await askScheme({
        xcworkspace: xcworkspace,
      }));
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.context, {
        xcworkspace: xcworkspace,
      }));

    const buildSettings = await getBuildSettings({
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    const destination = await this.getDestination({
      definition: definition,
      buildSettings: buildSettings,
    });
    const destinationRaw =
      definition.destination ??
      getDestinationRaw({
        platform: destination.platform,
        id: destination.udid,
      });

    const sdk = destination.platform;

    await buildApp(this.context, terminal, {
      scheme: scheme,
      sdk: sdk,
      configuration: configuration,
      shouldBuild: true,
      shouldClean: false,
      shouldTest: false,
      xcworkspace: xcworkspace,
      destinationRaw: destinationRaw,
      // destinationType: sdk,
      // destinationId: definition.destinationId ?? null,
    });
  }

  private async cleanCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    const xcworkspace = await askXcodeWorkspacePath(this.context);

    const scheme =
      definition.scheme ??
      (await askScheme({
        xcworkspace: xcworkspace,
      }));
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.context, {
        xcworkspace: xcworkspace,
      }));

    const buildSettings = await getBuildSettings({
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    const destination = await this.getDestination({
      definition: definition,
      buildSettings: buildSettings,
    });
    const destinationRaw =
      definition.destination ??
      getDestinationRaw({
        platform: destination.platform,
        id: destination.udid,
      });

    const sdk = destination.platform;

    await buildApp(this.context, terminal, {
      scheme: scheme,
      sdk: sdk,
      configuration: configuration,
      shouldBuild: false,
      shouldClean: true,
      shouldTest: false,
      xcworkspace: xcworkspace,
      destinationRaw: destinationRaw,
      // destinationType: sdk,
      // destinationId: definition.destinationId,
    });
  }

  private async testCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    const xcworkspace = await askXcodeWorkspacePath(this.context);
    const scheme =
      definition.scheme ??
      (await askScheme({
        xcworkspace: xcworkspace,
      }));
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.context, {
        xcworkspace: xcworkspace,
      }));

    const buildSettings = await getBuildSettings({
      scheme: scheme,
      configuration: configuration,
      sdk: undefined,
      xcworkspace: xcworkspace,
    });

    const destination = await this.getDestination({
      definition: definition,
      buildSettings: buildSettings,
    });
    const destinationRaw =
      definition.destination ??
      getDestinationRaw({
        platform: destination.platform,
        id: destination.udid,
      });

    const sdk = destination.platform;

    await buildApp(this.context, terminal, {
      scheme: scheme,
      sdk: sdk,
      configuration: configuration,
      shouldBuild: false,
      shouldClean: false,
      shouldTest: true,
      xcworkspace: xcworkspace,
      destinationRaw: destinationRaw,
    });
  }

  private async resolveDependenciesCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    const xcworkspacePath = definition.workspace ?? (await askXcodeWorkspacePath(this.context));
    const scheme =
      definition.scheme ??
      (await askScheme({
        xcworkspace: xcworkspacePath,
      }));

    await resolveDependencies(this.context, {
      scheme: scheme,
      xcworkspace: xcworkspacePath,
    });
  }
}

export class XcodeBuildTaskProvider implements vscode.TaskProvider {
  public type = "sweetpad";
  context: ExtensionContext;
  dispathcer: ActionDispatcher;

  constructor(context: ExtensionContext) {
    this.context = context;
    this.dispathcer = new ActionDispatcher(context);
  }

  async provideTasks(token: vscode.CancellationToken): Promise<vscode.Task[]> {
    return [
      this.getTask({
        name: "launch",
        details: "Build and Launch the app",
        defintion: {
          type: this.type,
          action: "launch",
        },
      }),
      this.getTask({
        name: "build",
        details: "Build the app",
        defintion: {
          type: this.type,
          action: "build",
        },
      }),
      this.getTask({
        name: "clean",
        details: "Clean the app",
        defintion: {
          type: this.type,
          action: "clean",
        },
      }),
      this.getTask({
        name: "resolve-dependencies",
        details: "Resolve dependencies",
        defintion: {
          type: this.type,
          action: "resolve-dependencies",
        },
      }),
    ];
  }

  private getTask(options: { name: string; details?: string; defintion: TaskDefinition }): vscode.Task {
    // Task looks like this:
    // -------
    // sweetpad: ${options.name}
    // ${options.details}
    // -------
    const task = new vscode.Task(
      options.defintion,
      vscode.TaskScope.Workspace,
      options.name, // name, after source
      "sweetpad", // source, before name`
      new vscode.CustomExecution(async (defition: vscode.TaskDefinition) => {
        const _defition = defition as TaskDefinition;
        const executorName = getTaskExecutorName();
        switch (executorName) {
          case "v1": {
            // Each task will create a new vscode.Task for each script
            // and one parent Terminal is used to show all tasks
            const terminal = new TaskTerminalV1(this.context, {
              name: options.name,
              source: "sweetpad",
            });
            await this.dispathcer.do(terminal, _defition);

            // create a dummy terminal to show the task in the terminal panel
            return new TaskTerminalV1Parent();
          }
          case "v2": {
            // In the V2 executor, one terminal is created for all tasks.
            // The callback should call terminal.execute(command) to run the script
            // in the current terminal.
            return new TaskTerminalV2(this.context, {
              callback: async (terminal) => {
                await this.dispathcer.do(terminal, _defition);
              },
            });
          }
          default:
            throw new Error(`Task executor ${executorName} is not supported`);
        }
      }),
      [],
    );

    if (options.details) {
      task.detail = options.details;
    }

    return task;
  }

  async resolveTask(_task: vscode.Task): Promise<vscode.Task | undefined> {
    // ResolveTask requires that the same definition object be used.
    // Otherwise, the VSCode show an error that the task is not found.
    const definition: TaskDefinition = <any>_task.definition;

    // Create new task with the same definition, otherwise it doesn't work and don't know why
    return this.getTask({
      name: `Custom "${definition.action}" task`, // name, after source
      defintion: definition,
    });
  }
}
