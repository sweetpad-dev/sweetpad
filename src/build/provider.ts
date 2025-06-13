import * as vscode from "vscode";
import { type XcodeBuildSettings, getBuildSettingsToAskDestination } from "../common/cli/scripts";
import { type ExtensionContext, TaskExecutionScope } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { errorReporting } from "../common/error-reporting";
import {
  type TaskTerminal,
  TaskTerminalV1,
  TaskTerminalV1Parent,
  TaskTerminalV2,
  getTaskExecutorName,
  setTaskPresentationOptions,
} from "../common/tasks";
import { assertUnreachable } from "../common/types";
import type { Destination } from "../destination/types";
import {
  buildApp,
  getXcodeBuildDestinationString,
  resolveDependencies,
  runOnMac,
  runOniOSDevice,
  runOniOSSimulator,
} from "./commands";
import { DEFAULT_BUILD_PROBLEM_MATCHERS } from "./constants";
import {
  askConfiguration,
  askDestinationToRunOn,
  askSchemeForBuild,
  askXcodeWorkspacePath,
  getDestinationById,
} from "./utils";

interface TaskDefinition extends vscode.TaskDefinition {
  type: string;
  action: string;
  scheme?: string;
  configuration?: string;
  workspace?: string;
  simulator?: string; // deprecated, use "destinationId" or "destinationRaw"
  destinationId?: string; // ex: "00000000-0000-0000-0000-000000000000"
  destination?: string; // ex: "platform=iOS Simulator,id=00000000-0000-0000-0000-000000000000"
  launchArgs?: string[]; // ex: ["-arg1", "-arg2"]
  launchEnv?: { [key: string]: string }; // ex: { "MY_ENV": "value" }
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
      case "run":
        await this.runCallback(terminal, definition);
        break;
      // ===> Debugger actions
      case "debugging-launch":
        await this.debuggerLaunchCallback(terminal, definition);
        break;
      case "debugging-build":
        await this.debuggerBuildCallback(terminal, definition);
        break;
      case "debugging-run":
        await this.debuggerRunCallback(terminal, definition);
        break;
      // <===
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

  private async getDestination(options: {
    definition: TaskDefinition;
    buildSettings: XcodeBuildSettings | null;
  }): Promise<Destination> {
    // First try to get destination from task definition
    const destinationId: string | undefined =
      // ex: "00000000-0000-0000-0000-000000000000"
      options.definition.destinationId ??
      // ex: "00000000-0000-0000-0000-000000000000"
      options.definition.simulator ??
      // ex: "platform=iOS Simulator,id=00000000-0000-0000-0000-000000000000"
      options.definition.destination?.match(/id=(.+)/)?.[1];

    // If user has provided the ID of the destination, then use it directly
    if (destinationId) {
      return await getDestinationById(this.context, { destinationId: destinationId });
    }

    // If not in task definition, try to get from workspace state
    const selectedDestination = this.context.destinationsManager.getSelectedXcodeDestinationForBuild();
    if (selectedDestination) {
      return await getDestinationById(this.context, { destinationId: selectedDestination.id });
    }

    // If no destination found in either place, and no build settings, throw error
    if (options.buildSettings === null) {
      throw new Error("Build settings are required to determine the destination");
    }

    // Otherwise, ask the user to select the destination (it will be cached for the next time)
    const destination = await askDestinationToRunOn(this.context, options.buildSettings);
    return destination;
  }

  private async getDestinationFromDefinition(options: {
    definition: TaskDefinition;
    scheme: string;
    configuration: string;
    xcworkspace: string;
  }): Promise<Destination> {
    // Try to get destination from task definition first
    try {
      return await this.getDestination({
        definition: options.definition,
        buildSettings: null,
      });
    } catch (error) {
      // If we can't get destination from definition, we need to get build settings
      this.context.updateProgressStatus("Extracting build settings");
      const buildSettings = await getBuildSettingsToAskDestination({
        scheme: options.scheme,
        configuration: options.configuration,
        sdk: undefined,
        xcworkspace: options.xcworkspace,
      });

      this.context.updateProgressStatus("Searching for destination");
      return await this.getDestination({
        definition: options.definition,
        buildSettings: buildSettings,
      });
    }
  }

  private async launchCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    await this.commonLaunchCallback(terminal, definition, {
      debug: false,
    });
  }

  private async debuggerLaunchCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    await this.commonLaunchCallback(terminal, definition, {
      debug: true,
    });
  }

  private async commonLaunchCallback(terminal: TaskTerminal, definition: TaskDefinition, options: { debug: boolean }) {
    this.context.updateProgressStatus("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.context);

    this.context.updateProgressStatus("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.context, {
        title: options.debug ? "Select scheme to debug" : "Select scheme to run",
        xcworkspace: xcworkspace,
      }));

    this.context.updateProgressStatus("Searching for configuration");
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.context, {
        xcworkspace: xcworkspace,
      }));

    const destination = await this.getDestinationFromDefinition({
      definition,
      scheme,
      configuration,
      xcworkspace,
    });

    const destinationRaw = definition.destination ?? getXcodeBuildDestinationString({ destination: destination });

    const sdk = destination.platform;

    const launchArgs: string[] = definition.launchArgs ?? getWorkspaceConfig("build.launchArgs") ?? [];
    const launchEnv: { [key: string]: string } = definition.launchEnv ?? getWorkspaceConfig("build.launchEnv") ?? {};

    await buildApp(this.context, terminal, {
      scheme: scheme,
      sdk: sdk,
      configuration: configuration,
      shouldBuild: true,
      shouldClean: false,
      shouldTest: false,
      xcworkspace: xcworkspace,
      destinationRaw: destinationRaw,
      debug: options.debug,
    });

    if (destination.type === "macOS") {
      await runOnMac(this.context, terminal, {
        scheme: scheme,
        configuration: configuration,
        xcworkspace: xcworkspace,
        watchMarker: true,
        launchArgs: launchArgs,
        launchEnv: launchEnv,
      });
    } else if (
      destination.type === "iOSSimulator" ||
      destination.type === "watchOSSimulator" ||
      destination.type === "visionOSSimulator" ||
      destination.type === "tvOSSimulator"
    ) {
      await runOniOSSimulator(this.context, terminal, {
        scheme: scheme,
        destination: destination,
        sdk: sdk,
        configuration: configuration,
        xcworkspace: xcworkspace,
        watchMarker: true,
        launchArgs: launchArgs,
        launchEnv: launchEnv,
        debug: options.debug,
      });
    } else if (
      destination.type === "iOSDevice" ||
      destination.type === "watchOSDevice" ||
      destination.type === "tvOSDevice" ||
      destination.type === "visionOSDevice"
    ) {
      await runOniOSDevice(this.context, terminal, {
        scheme: scheme,
        destination: destination,
        sdk: sdk,
        configuration: configuration,
        xcworkspace: xcworkspace,
        watchMarker: true,
        launchArgs: launchArgs,
        launchEnv: launchEnv,
      });
    } else {
      assertUnreachable(destination);
    }
  }

  private async buildCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    await this.commonBuildCallback(terminal, definition, {
      debug: false,
    });
  }

  private async debuggerBuildCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    await this.commonBuildCallback(terminal, definition, {
      debug: true,
    });
  }

  private async commonBuildCallback(terminal: TaskTerminal, definition: TaskDefinition, options: { debug: boolean }) {
    this.context.updateProgressStatus("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.context);

    this.context.updateProgressStatus("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.context, {
        xcworkspace: xcworkspace,
      }));

    this.context.updateProgressStatus("Searching for configuration");
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.context, {
        xcworkspace: xcworkspace,
      }));

    const destination = await this.getDestinationFromDefinition({
      definition,
      scheme,
      configuration,
      xcworkspace,
    });

    const destinationRaw = definition.destination ?? getXcodeBuildDestinationString({ destination: destination });

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
      debug: options.debug,
    });
  }

  private async runCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    await this.commonRunCallback(terminal, definition, {
      debug: false,
    });
  }

  private async debuggerRunCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    await this.commonRunCallback(terminal, definition, {
      debug: true,
    });
  }

  private async commonRunCallback(terminal: TaskTerminal, definition: TaskDefinition, options: { debug: boolean }) {
    this.context.updateProgressStatus("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.context);

    this.context.updateProgressStatus("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.context, {
        xcworkspace: xcworkspace,
      }));

    this.context.updateProgressStatus("Searching for configuration");
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.context, {
        xcworkspace: xcworkspace,
      }));

    const destination = await this.getDestinationFromDefinition({
      definition,
      scheme,
      configuration,
      xcworkspace,
    });

    const sdk = destination.platform;

    // Launch arguments and envs have higher priority than the workspace configuration
    const launchArgs: string[] = definition.launchArgs ?? getWorkspaceConfig("build.launchArgs") ?? [];
    const launchEnv: { [key: string]: string } = definition.launchEnv ?? getWorkspaceConfig("build.launchEnv") ?? {};

    if (destination.type === "macOS") {
      await runOnMac(this.context, terminal, {
        scheme: scheme,
        configuration: configuration,
        xcworkspace: xcworkspace,
        watchMarker: false,
        launchArgs: launchArgs,
        launchEnv: launchEnv,
      });
    } else if (
      destination.type === "iOSSimulator" ||
      destination.type === "watchOSSimulator" ||
      destination.type === "visionOSSimulator" ||
      destination.type === "tvOSSimulator"
    ) {
      await runOniOSSimulator(this.context, terminal, {
        scheme: scheme,
        destination: destination,
        sdk: sdk,
        configuration: configuration,
        xcworkspace: xcworkspace,
        watchMarker: false,
        launchArgs: launchArgs,
        launchEnv: launchEnv,
        debug: options.debug,
      });
    } else if (
      destination.type === "iOSDevice" ||
      destination.type === "watchOSDevice" ||
      destination.type === "tvOSDevice" ||
      destination.type === "visionOSDevice"
    ) {
      await runOniOSDevice(this.context, terminal, {
        scheme: scheme,
        destination: destination,
        sdk: sdk,
        configuration: configuration,
        xcworkspace: xcworkspace,
        watchMarker: true,
        launchArgs: launchArgs,
        launchEnv: launchEnv,
      });
    } else {
      assertUnreachable(destination);
    }
  }

  private async cleanCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    this.context.updateProgressStatus("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.context);

    this.context.updateProgressStatus("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.context, {
        xcworkspace: xcworkspace,
      }));

    this.context.updateProgressStatus("Searching for configuration");
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.context, {
        xcworkspace: xcworkspace,
      }));

    const destination = await this.getDestinationFromDefinition({
      definition,
      scheme,
      configuration,
      xcworkspace,
    });

    const destinationRaw = definition.destination ?? getXcodeBuildDestinationString({ destination: destination });

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
      debug: false,
    });
  }

  private async testCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    this.context.updateProgressStatus("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.context);

    this.context.updateProgressStatus("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.context, {
        xcworkspace: xcworkspace,
      }));
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.context, {
        xcworkspace: xcworkspace,
      }));

    const destination = await this.getDestinationFromDefinition({
      definition,
      scheme,
      configuration,
      xcworkspace,
    });

    const destinationRaw = definition.destination ?? getXcodeBuildDestinationString({ destination: destination });

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
      debug: false,
    });
  }

  private async resolveDependenciesCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    this.context.updateProgressStatus("Searching for workspace");
    const xcworkspacePath = definition.workspace ?? (await askXcodeWorkspacePath(this.context));

    this.context.updateProgressStatus("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.context, {
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
        isBackground: true,
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
        name: "run",
        details: "Run the app (without building)",
        defintion: {
          type: this.type,
          action: "run",
        },
        isBackground: true,
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
      this.getTask({
        name: "debugging-launch",
        details: "Build and Launch the app (for debugging)",
        defintion: {
          type: this.type,
          action: "debugging-launch",
        },
        isBackground: true,
      }),
      this.getTask({
        name: "debugging-build",
        details: "Build the app (for debugging)",
        defintion: {
          type: this.type,
          action: "debugging-build",
        },
      }),
      this.getTask({
        name: "debugging-run",
        details: "Run the app (for debugging)",
        defintion: {
          type: this.type,
          action: "debugging-run",
        },
        isBackground: true,
      }),
    ];
  }

  async dispatchTask(terminal: TaskTerminal, definition: TaskDefinition): Promise<void> {
    const taskScope = new TaskExecutionScope({ action: definition.action });
    return await errorReporting.withScope(async () => {
      return await this.context.startExecutionScope(taskScope, async () => {
        await this.dispathcer.do(terminal, definition);
      });
    });
  }

  private getTask(options: {
    name: string;
    details?: string;
    defintion: TaskDefinition;
    isBackground?: boolean;
  }): vscode.Task {
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
            await this.dispatchTask(terminal, _defition);

            // create a dummy terminal to show the task in the terminal panel
            return new TaskTerminalV1Parent();
          }
          case "v2": {
            // In the V2 executor, one terminal is created for all tasks.
            // The callback should call terminal.execute(command) to run the script
            // in the current terminal.
            return new TaskTerminalV2(this.context, {
              callback: async (terminal) => {
                await this.dispatchTask(terminal, _defition);
              },
            });
          }
          default:
            throw new Error(`Task executor ${executorName} is not supported`);
        }
      }),
      DEFAULT_BUILD_PROBLEM_MATCHERS, // problemMatchers
    );
    setTaskPresentationOptions(task);

    if (options.isBackground) {
      task.isBackground = true;
    }

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
