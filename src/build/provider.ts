import * as vscode from "vscode";

import { TaskExecutionScope } from "../common/commands";
import { getWorkspaceConfig } from "../common/config";
import { errorReporting } from "../common/error-reporting";
import type { ExecutionScopeService } from "../common/execution-scope";
import { setTaskPresentationOptions } from "../common/tasks/presentation";
import { loadNodePty } from "../common/tasks/pty";
import { getTaskExecutorName } from "../common/tasks/run";
import type { TaskTerminal } from "../common/tasks/types";
import { TaskTerminalV2 } from "../common/tasks/v2";
import { TaskTerminalV3 } from "../common/tasks/v3";
import { assertUnreachable } from "../common/types";
import type { WorkspaceStateService } from "../common/workspace-state";
import type { DestinationsManager } from "../destination/manager";
import type { Destination } from "../destination/types";
import type { ProgressStatusBar } from "../system/status-bar";
import { DEFAULT_BUILD_PROBLEM_MATCHERS } from "./constants";
import type { BuildManager } from "./manager";
import {
  askConfiguration,
  askDestinationToRunOn,
  askSchemeForBuild,
  askXcodeWorkspacePath,
  getXcodeBuildDestinationString,
} from "./utils";

type DispatcherDeps = {
  buildManager: BuildManager;
  destinationsManager: DestinationsManager;
  workspace: WorkspaceStateService;
  progressStatusBar: ProgressStatusBar;
};

type ProviderDeps = DispatcherDeps & {
  execution: ExecutionScopeService;
};

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
  deps: DispatcherDeps;
  constructor(deps: DispatcherDeps) {
    this.deps = deps;
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

  private async getDestinationByUserInput(
    destinationsManager: DestinationsManager,
    options: { definition: TaskDefinition },
  ): Promise<Destination | undefined> {
    // For simulators and devices, we try to find destination by ID
    // ex: "00000000-0000-0000-0000-000000000000"
    // ex: "platform=iOS Simulator,id=00000000-0000-0000-0000-000000000000"
    const udidRaw: string | undefined =
      options.definition.destinationId ??
      options.definition.simulator ??
      options.definition.destination?.match(/id=(.+)/)?.[1];

    const udidLower = udidRaw?.trim()?.toLowerCase();

    // For macOS, we just check if the destination string contains "macos"
    const isMacOS = options.definition.destination?.toLowerCase().includes("macos") ?? false;

    const destinations = await destinationsManager.getDestinations();
    const destination = destinations.find((d) => {
      switch (d.type) {
        case "iOSSimulator":
        case "watchOSSimulator":
        case "visionOSSimulator":
        case "tvOSSimulator":
        case "iOSDevice":
        case "watchOSDevice":
        case "visionOSDevice":
        case "tvOSDevice":
          return d.udid.toLowerCase() === udidLower;
        case "macOS":
          return isMacOS;
        default:
          assertUnreachable(d);
      }
    });

    return destination;
  }

  private async getDestination(options: {
    definition: TaskDefinition;
    scheme: string;
    configuration: string;
    xcworkspace: string;
  }): Promise<Destination> {
    // If user has provided the ID of the destination, then use it directly
    const inputDestination = await this.getDestinationByUserInput(this.deps.destinationsManager, {
      definition: options.definition,
    });
    if (inputDestination) {
      return inputDestination;
    }

    // If not in task definition, then ask user to select destination (or get from cache)
    const destination = await askDestinationToRunOn(this.deps.progressStatusBar, this.deps.destinationsManager, {
      scheme: options.scheme,
      configuration: options.configuration,
      sdk: undefined,
      xcworkspace: options.xcworkspace,
    });
    return destination;
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
    this.deps.progressStatusBar.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.deps.workspace, this.deps.buildManager);

    this.deps.progressStatusBar.updateText("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.deps.progressStatusBar, this.deps.buildManager, {
        title: options.debug ? "Select scheme to debug" : "Select scheme to run",
        xcworkspace: xcworkspace,
      }));

    this.deps.progressStatusBar.updateText("Searching for configuration");
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.deps.progressStatusBar, this.deps.buildManager, {
        xcworkspace: xcworkspace,
      }));

    const destination = await this.getDestination({
      definition: definition,
      scheme: scheme,
      configuration: configuration,
      xcworkspace: xcworkspace,
    });

    const destinationRaw = definition.destination ?? getXcodeBuildDestinationString({ destination: destination });

    const sdk = destination.platform;

    const launchArgs: string[] = definition.launchArgs ?? getWorkspaceConfig("build.launchArgs") ?? [];
    const launchEnv: { [key: string]: string } = definition.launchEnv ?? getWorkspaceConfig("build.launchEnv") ?? {};

    await this.deps.buildManager.buildApp(terminal, {
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
      await this.deps.buildManager.runOnMac(terminal, {
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
      await this.deps.buildManager.runOniOSSimulator(terminal, {
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
      await this.deps.buildManager.runOniOSDevice(terminal, {
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
    this.deps.progressStatusBar.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.deps.workspace, this.deps.buildManager);

    this.deps.progressStatusBar.updateText("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.deps.progressStatusBar, this.deps.buildManager, {
        xcworkspace: xcworkspace,
      }));

    this.deps.progressStatusBar.updateText("Searching for configuration");
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.deps.progressStatusBar, this.deps.buildManager, {
        xcworkspace: xcworkspace,
      }));

    const destination = await this.getDestination({
      definition: definition,
      scheme: scheme,
      configuration: configuration,
      xcworkspace: xcworkspace,
    });

    const destinationRaw = definition.destination ?? getXcodeBuildDestinationString({ destination: destination });

    const sdk = destination.platform;

    await this.deps.buildManager.buildApp(terminal, {
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
    this.deps.progressStatusBar.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.deps.workspace, this.deps.buildManager);

    this.deps.progressStatusBar.updateText("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.deps.progressStatusBar, this.deps.buildManager, {
        xcworkspace: xcworkspace,
      }));

    this.deps.progressStatusBar.updateText("Searching for configuration");
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.deps.progressStatusBar, this.deps.buildManager, {
        xcworkspace: xcworkspace,
      }));

    const destination = await this.getDestination({
      definition: definition,
      scheme: scheme,
      configuration: configuration,
      xcworkspace: xcworkspace,
    });

    const sdk = destination.platform;

    // Launch arguments and envs have higher priority than the workspace configuration
    const launchArgs: string[] = definition.launchArgs ?? getWorkspaceConfig("build.launchArgs") ?? [];
    const launchEnv: { [key: string]: string } = definition.launchEnv ?? getWorkspaceConfig("build.launchEnv") ?? {};

    if (destination.type === "macOS") {
      await this.deps.buildManager.runOnMac(terminal, {
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
      await this.deps.buildManager.runOniOSSimulator(terminal, {
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
      await this.deps.buildManager.runOniOSDevice(terminal, {
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
    this.deps.progressStatusBar.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.deps.workspace, this.deps.buildManager);

    this.deps.progressStatusBar.updateText("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.deps.progressStatusBar, this.deps.buildManager, {
        xcworkspace: xcworkspace,
      }));

    this.deps.progressStatusBar.updateText("Searching for configuration");
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.deps.progressStatusBar, this.deps.buildManager, {
        xcworkspace: xcworkspace,
      }));

    const destination = await this.getDestination({
      definition: definition,
      scheme: scheme,
      configuration: configuration,
      xcworkspace: xcworkspace,
    });

    const destinationRaw = definition.destination ?? getXcodeBuildDestinationString({ destination: destination });

    const sdk = destination.platform;

    await this.deps.buildManager.buildApp(terminal, {
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
    this.deps.progressStatusBar.updateText("Searching for workspace");
    const xcworkspace = await askXcodeWorkspacePath(this.deps.workspace, this.deps.buildManager);

    this.deps.progressStatusBar.updateText("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.deps.progressStatusBar, this.deps.buildManager, {
        xcworkspace: xcworkspace,
      }));
    const configuration =
      definition.configuration ??
      (await askConfiguration(this.deps.progressStatusBar, this.deps.buildManager, {
        xcworkspace: xcworkspace,
      }));

    const destination = await this.getDestination({
      definition: definition,
      scheme: scheme,
      configuration: configuration,
      xcworkspace: xcworkspace,
    });

    const destinationRaw = definition.destination ?? getXcodeBuildDestinationString({ destination: destination });

    const sdk = destination.platform;

    await this.deps.buildManager.buildApp(terminal, {
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
    this.deps.progressStatusBar.updateText("Searching for workspace");
    const xcworkspacePath =
      definition.workspace ?? (await askXcodeWorkspacePath(this.deps.workspace, this.deps.buildManager));

    this.deps.progressStatusBar.updateText("Searching for scheme");
    const scheme =
      definition.scheme ??
      (await askSchemeForBuild(this.deps.progressStatusBar, this.deps.buildManager, {
        xcworkspace: xcworkspacePath,
      }));

    await this.deps.buildManager.resolveDependenciesCommand({
      scheme: scheme,
      xcworkspace: xcworkspacePath,
    });
  }
}

export class XcodeBuildTaskProvider implements vscode.TaskProvider {
  public type = "sweetpad";
  deps: ProviderDeps;
  dispathcer: ActionDispatcher;

  constructor(deps: ProviderDeps) {
    this.deps = deps;
    this.dispathcer = new ActionDispatcher({
      buildManager: deps.buildManager,
      destinationsManager: deps.destinationsManager,
      workspace: deps.workspace,
      progressStatusBar: deps.progressStatusBar,
    });
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
      return await this.deps.execution.startScope(taskScope, async () => {
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
        const taskDefinition = defition as TaskDefinition;
        let executorName = getTaskExecutorName();
        if (executorName === "v3" && loadNodePty() === null) {
          // Fall back to v2 when node-pty cannot be loaded from VS Code's app
          // root (e.g. on forks that don't bundle it). loadNodePty already logs.
          executorName = "v2";
        }
        switch (executorName) {
          case "v2": {
            // In the V2 executor, one terminal is created for all tasks.
            // The callback should call terminal.execute(command) to run the script
            // in the current terminal.
            return new TaskTerminalV2({
              callback: async (terminal) => {
                await this.dispatchTask(terminal, taskDefinition);
              },
            });
          }
          case "v3": {
            return new TaskTerminalV3({
              callback: async (terminal) => {
                await this.dispatchTask(terminal, taskDefinition);
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
