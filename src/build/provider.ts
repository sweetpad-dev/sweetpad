import * as vscode from "vscode";
import { askConfiguration, askScheme, askSimulatorToRunOn, askXcodeWorkspacePath } from "./utils";
import { DEFAULT_SDK, buildApp, runOnDevice, resolveDependencies } from "./commands";
import { ExtensionContext } from "../common/commands";
import { getSimulatorByUdid } from "../common/cli/scripts";
import { TaskTerminalV2, TaskTerminalV1, TaskTerminal, getExecutorVersion, runTaskV1 } from "../common/tasks";

interface TaskDefinition extends vscode.TaskDefinition {
  type: string;
  action: string;
  scheme?: string;
  configuration?: string;
  workspace?: string;
  simulator?: string;
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
      case "resolve-dependencies":
        await this.resolveDependenciesCallback(terminal, definition);
        break;
      default:
        throw new Error(`Action ${action} is not supported`);
    }
  }

  private async launchCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    const scheme = definition.scheme ?? (await askScheme());
    const configuration = definition.configuration ?? (await askConfiguration(this.context));
    const simulator = definition.simulator
      ? await getSimulatorByUdid(definition.simulator)
      : await askSimulatorToRunOn(this.context);

    await buildApp(this.context, terminal, {
      scheme: scheme,
      sdk: DEFAULT_SDK,
      configuration: configuration,
      shouldBuild: true,
      shouldClean: false,
    });

    await runOnDevice(this.context, terminal, {
      scheme: scheme,
      simulator: simulator,
      sdk: DEFAULT_SDK,
      configuration: configuration,
    });
  }

  private async buildCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    const scheme = definition.scheme ?? (await askScheme());
    const configuration = definition.configuration ?? (await askConfiguration(this.context));

    await buildApp(this.context, terminal, {
      scheme: scheme,
      sdk: DEFAULT_SDK,
      configuration: configuration,
      shouldBuild: true,
      shouldClean: false,
    });
  }

  private async cleanCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    const scheme = definition.scheme ?? (await askScheme());
    const configuration = definition.configuration ?? (await askConfiguration(this.context));

    await buildApp(this.context, terminal, {
      scheme: scheme,
      sdk: DEFAULT_SDK,
      configuration: configuration,
      shouldBuild: false,
      shouldClean: true,
    });
  }

  private async resolveDependenciesCallback(terminal: TaskTerminal, definition: TaskDefinition) {
    const scheme = definition.scheme ?? (await askScheme());
    const xcworkspacePath = definition.workspace ?? (await askXcodeWorkspacePath(this.context));

    await resolveDependencies(this.context, {
      scheme: scheme,
      xcodeWorkspacePath: xcworkspacePath,
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

        if (!getExecutorVersion()) {
          // Run all task before the custom terminal is created
          const terminal = new TaskTerminalV1(this.context, {
            name: options.name,
            source: "sweetpad",
          });
          await this.dispathcer.do(terminal, _defition);

          // create a dummy terminal to show the task in the terminal panel
          return new XcodeBuildTerminal();
        } else {
          return new TaskTerminalV2(this.context, {
            callback: async (terminal) => {
              await this.dispathcer.do(terminal, _defition);
            },
          });
        }
      }),
      []
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

class XcodeBuildTerminal implements vscode.Pseudoterminal {
  public writeEmitter = new vscode.EventEmitter<string>();
  public closeEmitter = new vscode.EventEmitter<number>();

  constructor() {}

  onDidWrite = this.writeEmitter.event;
  onDidClose = this.closeEmitter.event;

  writePlaceholderText(): void {
    this.writeEmitter.fire("====> It's parent task, just ignore it\r\n");
  }

  open(): void {
    this.writePlaceholderText();
    this.closeEmitter.fire(0);
  }

  close(): void {
    this.writeEmitter.dispose();
    this.closeEmitter.fire(0);
    this.closeEmitter.dispose();
  }
}
