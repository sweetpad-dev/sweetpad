import * as vscode from "vscode";
import { askConfiguration, askScheme, askSimulatorToRunOn, askXcodeWorkspacePath } from "./utils";
import { DEFAULT_SDK, buildApp, runOnDevice, resolveDependencies } from "./commands";
import { ExtensionContext } from "../common/commands";
import { getSimulatorByUdid } from "../common/cli/scripts";

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

  async do(definition: TaskDefinition) {
    const action = definition.action;
    switch (action) {
      case "launch":
        await this.launchCallback(definition);
        break;
      case "build":
        await this.buildCallback(definition);
        break;
      case "clean":
        await this.cleanCallback(definition);
        break;
      case "resolve-dependencies":
        await this.resolveDependenciesCallback(definition);
        break;
      default:
        throw new Error(`Action ${action} is not supported`);
    }
  }

  private async launchCallback(definition: TaskDefinition) {
    const scheme = definition.scheme ?? (await askScheme());
    const configuration = definition.configuration ?? (await askConfiguration(this.context));
    const simulator = definition.simulator
      ? await getSimulatorByUdid(definition.simulator)
      : await askSimulatorToRunOn(this.context);

    await buildApp(this.context, {
      scheme: scheme,
      sdk: DEFAULT_SDK,
      configuration: configuration,
      shouldBuild: true,
      shouldClean: false,
    });

    await runOnDevice(this.context, {
      scheme: scheme,
      simulator: simulator,
      sdk: DEFAULT_SDK,
      configuration: configuration,
    });
  }

  private async buildCallback(definition: TaskDefinition) {
    const scheme = definition.scheme ?? (await askScheme());
    const configuration = definition.configuration ?? (await askConfiguration(this.context));

    await buildApp(this.context, {
      scheme: scheme,
      sdk: DEFAULT_SDK,
      configuration: configuration,
      shouldBuild: true,
      shouldClean: false,
    });
  }

  private async cleanCallback(definition: TaskDefinition) {
    const scheme = definition.scheme ?? (await askScheme());
    const configuration = definition.configuration ?? (await askConfiguration(this.context));

    await buildApp(this.context, {
      scheme: scheme,
      sdk: DEFAULT_SDK,
      configuration: configuration,
      shouldBuild: false,
      shouldClean: true,
    });
  }

  private async resolveDependenciesCallback(definition: TaskDefinition) {
    const scheme = definition.scheme ?? (await askScheme());
    const xcworkspacePath = definition.workspace ?? (await askXcodeWorkspacePath(this.context));

    await resolveDependencies({
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

  private getExecution() {
    return new vscode.CustomExecution(async (defition: vscode.TaskDefinition) => {
      const _defition = defition as TaskDefinition;
      await this.dispathcer.do(_defition);
      return new XcodeBuildTerminal();
    });
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
      this.getExecution(),
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
